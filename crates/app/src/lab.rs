//! The evolution lab: a trainer on its own thread, and the generation the window draws.
//!
//! This is M3's watch mode hosted in M4's window. The terminal version (`train-cli
//! watch`) proves the shape — one generation, then the eight episodes the plan asks
//! for re-run tick by tick, then the next generation — and everything structural here
//! is that shape with the frame drawing removed, because the *rules* are the part that
//! has to survive the move:
//!
//! * **A run never trains on a timer.** The worker evaluates a generation, parks the
//!   result, and waits until the window has finished animating it before evaluating
//!   the next. That is what keeps the GA's RNG stream identical whether or not anyone
//!   is watching (§5.6, asserted in `evo`'s `watching_does_not_change_what_is_evolved`),
//!   and it is why the window can be paused mid-generation without the population
//!   running away from what is on screen.
//! * **A lane animates the episode that was scored**, driven by [`EpisodeRunner`] with
//!   the exact seed `Trainer::showcase` recorded — never a fresh episode on the same
//!   genome. A watch mode that animated a *different* episode would be lying about the
//!   fitness printed next to it. `a_lane_replays_the_episode_that_was_scored` checks
//!   the animation's final score against the number the packet advertises.
//! * **A recorded route is its own episode**, not a sample of the animation: a paused
//!   or skipped animation would truncate the path, and on a stalled run — the case the
//!   scrubber exists for — the viewer is the most likely to have walked away.
//!
//! Why a thread at all: a generation at pop 200 costs tens of milliseconds of wall
//! clock across every core, and a window that blocks on `step_verbose` drops frames
//! exactly while the user is trying to look at it. The worker owns the trainer and the
//! level; the window owns the drawing; a mutex, a condvar and one packet cross between
//! them per generation.
//!
//! The three ladders ([`SPEEDS`], [`MUTATION_RATES`]) and the death window are the same
//! values the terminal mode offers, deliberately: two front-ends for one game should
//! not disagree about what 8x means or which mutation rates are reachable.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use evo::{BestEver, EpisodeRunner, EvalConfig, GaConfig, Genome, LaneKind, Trainer};
use render::chart::HistoryPoint;
use render::trace::{ChampionTrace, TraceLog, TracePoint, TRACE_SAMPLE_FRAMES};
use sim::{Level, Outcome, Sim};

pub use evo::MUTATION_RATES;

/// Generations of death marks the canvas keeps, matching watch mode's `--death-window`:
/// a forty-generation-old corpse says nothing about where the population is stuck *now*.
pub const DEATH_WINDOW: u32 = 25;

/// Best lanes and random lanes drawn per generation (§10 Tier 1.5). Best-4 + random-4
/// rather than best-only on purpose: a population that has collapsed onto one strategy
/// is exactly the failure the view has to make visible, and a chart cannot show it.
pub const DEFAULT_BEST_LANES: usize = 4;
pub const DEFAULT_RANDOM_LANES: usize = 4;

/// The speed ladder, in physics ticks per 1/60 s. Same rungs as watch mode's `--speed`,
/// so the plan's 1x–50x range (§10 Tier 1.4) is a handful of steps wide here too.
pub const SPEEDS: [u32; 7] = [1, 2, 4, 8, 16, 32, 50];

/// Physics ticks per rendered frame at startup. 8x is the compromise the fun gate
/// needs: fast enough that a 5 000-tick episode is a ten-second generation, slow enough
/// to see a jump happen.
pub const DEFAULT_SPEED: u32 = 8;

/// Ticks per second at 1x, i.e. the sim's own step rate.
const TICKS_PER_SECOND: f32 = 60.0;

/// How much simulation one frame may catch up on. The playtest clamps for the same
/// reason: a stalled window must not skip a whole episode past the viewer.
const MAX_CATCHUP: f32 = 0.25;

/// Outline colours for the lanes, champion first.
///
/// A colour is how the panel's lane list and an agent on the canvas agree about which
/// agent is which. The terminal gives each lane a glyph, and eight overlapping glyphs
/// work because a terminal *is* a character grid; two sprite agents in one tile would
/// smear into each other instead, so the window distinguishes them by outline — the
/// same problem, answered in the medium that is actually on screen.
const LANE_COLOURS: [[u8; 3]; 8] = [
    [250, 214, 102], // the palette's own selection gold: the champion should look chosen
    [120, 216, 120],
    [110, 190, 240],
    [240, 150, 90],
    [200, 150, 240],
    [140, 220, 200],
    [240, 190, 120],
    [170, 200, 250],
];

/// One agent to animate, with the episode it replays.
#[derive(Clone, Debug)]
pub struct Lane {
    /// How the panel names this lane: `champion`, `best 2`, `random 1`.
    pub name: String,
    pub best: bool,
    pub rank: usize,
    pub genome_id: u32,
    /// Fitness of the episode this lane replays (trial 0) — the number on screen has to
    /// be the number being drawn.
    pub fitness: f32,
    /// Mean fitness over all trials, i.e. the metric selection ranked on.
    pub mean_fitness: f32,
    /// Outline colour, so a lane's row and its agent cannot be confused.
    pub colour: [u8; 3],
    /// The genome itself, so the lane can be animated without the population that
    /// produced it (the trainer has already moved on to the next generation by then).
    pub genome: Genome,
    /// The exact seed the evaluator used.
    pub seed: u64,
}

/// One generation, packaged for the window.
#[derive(Clone, Debug)]
pub struct Packet {
    pub generation: u32,
    pub max_fitness: f32,
    pub avg_fitness: f32,
    pub min_fitness: f32,
    /// Agents that touched the flag.
    pub wins: usize,
    /// `max > 1.5 × previous max` — the "genius agent" splash (§10 Tier 1.3).
    pub genius: bool,
    pub pop_size: usize,
    pub lanes: Vec<Lane>,
    /// The whole run's history, not just this generation: the chart is the run's shape,
    /// and re-deriving it in the window would let the two disagree.
    pub history: Vec<HistoryPoint>,
    /// Where the population has been dying, `(x, y, generation)`.
    pub deaths: Vec<(f32, f32, u32)>,
    /// The recorded improvements the scrubber walks.
    pub traces: Vec<ChampionTrace>,
    /// The best genome seen so far, with the episode that earned it — everything
    /// `ChampionFile` needs, so saving from the window is the same file `train` writes.
    pub best: Option<BestEver>,
    /// Generations this run was asked for, when it was given a cap.
    pub target: Option<u32>,
    pub level_name: String,
    pub level_hash: String,
    /// The run reached its cap and the worker has stopped.
    pub finished: bool,
}

/// A generation being animated: the packet, and one live episode per lane.
///
/// The runners own their genomes ([`EpisodeRunner`] takes one by value), which is what
/// lets this struct hold both — a borrowing runner would make the pool self-referential
/// and unrepresentable in a struct the window owns.
pub struct Lanes {
    packet: Packet,
    runners: Vec<EpisodeRunner>,
    /// Fractional ticks carried between frames, so a fast monitor does not animate
    /// further per second than a slow one.
    carry: f32,
    ticks: u32,
}

impl Lanes {
    /// Build the animation for one generation. The level and eval config are the ones
    /// the generation was evaluated with; a lane's episode is only the scored episode if
    /// both match.
    pub fn new(packet: Packet, level: &Level, eval: &EvalConfig) -> Lanes {
        let runners = packet
            .lanes
            .iter()
            .map(|lane| EpisodeRunner::new(level, lane.genome.clone(), eval, lane.seed))
            .collect();
        Lanes {
            packet,
            runners,
            carry: 0.0,
            ticks: 0,
        }
    }

    pub fn packet(&self) -> &Packet {
        &self.packet
    }

    pub fn lanes(&self) -> &[Lane] {
        &self.packet.lanes
    }

    pub fn len(&self) -> usize {
        self.runners.len()
    }

    pub fn is_finished(&self) -> bool {
        self.runners.iter().all(|r| r.is_finished())
    }

    /// Lanes still running, for the progress line.
    pub fn running(&self) -> usize {
        self.runners.iter().filter(|r| !r.is_finished()).count()
    }

    /// Seconds of animation, for the sprite clocks. A pure function of the tick count,
    /// so replaying the same lane draws the same frames.
    pub fn clock(&self) -> f32 {
        self.ticks as f32 * sim::DT
    }

    /// The live episode of one lane.
    pub fn sim(&self, i: usize) -> &Sim {
        self.runners[i].sim()
    }

    /// Advance by `dt` seconds of wall clock at `speed`.
    pub fn advance(&mut self, dt: f32, speed: u32) {
        self.carry += ticks_for(dt, speed);
        let whole = self.carry.floor();
        self.carry -= whole;
        let ticks = whole.max(0.0) as u32;
        for runner in &mut self.runners {
            for _ in 0..ticks {
                if runner.is_finished() {
                    break;
                }
                runner.step();
            }
        }
        self.ticks += ticks;
    }
}

/// The next rung of the speed ladder, up or down, stopping at the ends.
///
/// A ladder rather than a doubling so every keypress lands on a rung the viewer can get
/// back to, and no keypress can drive the run past the top or stall it at zero. The
/// *values* are shared with the terminal (`SPEEDS`), so 8x means the same thing in both
/// front-ends; only the stepping is local, because the terminal counts ticks per drawn
/// frame and this counts ticks per 1/60 s.
pub fn step_speed(current: u32, up: bool) -> u32 {
    let nearest = SPEEDS
        .iter()
        .enumerate()
        .min_by_key(|(_, rung)| rung.abs_diff(current))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let next = if up {
        (nearest + 1).min(SPEEDS.len() - 1)
    } else {
        nearest.saturating_sub(1)
    };
    SPEEDS[next]
}

/// The next rung of the mutation ladder, up or down, stopping at the ends.
pub fn step_mutation_rate(current: f32, up: bool) -> f32 {
    // `total_cmp` rather than a cast to an integer distance: truncating 0.07 and 0.05 to
    // 0 makes every rung equidistant, and the search then returns the first one — which
    // is how `up` from the top rate used to walk back down to the middle of the ladder.
    let nearest = MUTATION_RATES
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (**a - current).abs().total_cmp(&(**b - current).abs()))
        .map(|(i, _)| i)
        .unwrap_or(1);
    let next = if up {
        (nearest + 1).min(MUTATION_RATES.len() - 1)
    } else {
        nearest.saturating_sub(1)
    };
    MUTATION_RATES[next]
}

/// Physics ticks one rendered frame is worth.
///
/// Frame-rate independent on purpose: `speed` is "physics ticks per 1/60 s", so 8x is
/// eight ticks per 60 Hz frame whether the window runs at 30 Hz or 144 Hz. The dt clamp
/// is the playtest's: a hitch (a breakpoint, a slow redraw) must not teleport the
/// population past the part the viewer was watching.
pub fn ticks_for(dt: f32, speed: u32) -> f32 {
    dt.clamp(0.0, MAX_CATCHUP) * TICKS_PER_SECOND * speed.max(1) as f32
}

/// The thing that runs generations. Owns the trainer, so it is what the worker thread
/// owns; the window never touches it.
pub struct Generator {
    trainer: Trainer,
    level: Level,
    cfg: GaConfig,
    eval: EvalConfig,
    traces: TraceLog,
    history: Vec<HistoryPoint>,
    deaths: Vec<(f32, f32, u32)>,
    best_k: usize,
    random_k: usize,
    target: Option<u32>,
}

impl Generator {
    pub fn new(
        level: Level,
        cfg: GaConfig,
        best_k: usize,
        random_k: usize,
        target: Option<u32>,
    ) -> Generator {
        let cfg = cfg.sanitized(None);
        Generator {
            eval: cfg.eval_config(),
            trainer: Trainer::new(cfg.clone()),
            level,
            cfg,
            traces: TraceLog::new(),
            history: Vec::new(),
            deaths: Vec::new(),
            best_k,
            random_k,
            target,
        }
    }

    /// The level a generation is being evaluated on. The window reads the run's level
    /// from [`Lab`] instead; this exists for the tests that check a restart moved it.
    #[cfg(test)]
    pub fn level(&self) -> &Level {
        &self.level
    }

    /// Start over — on the same level after an edit, or on a different one entirely —
    /// with a fresh population, a fresh chart and an empty record.
    ///
    /// Everything the run accumulated is dropped on purpose: the chart is a claim about
    /// *this* population on *this* level, and continuing it across a level change would
    /// draw a cliff that no agent ever experienced.
    pub fn start_over(&mut self, level: Level, seed: u64, mutation_rate: f32) {
        let mut cfg = self.cfg.clone();
        cfg.master_seed = seed;
        cfg.mutation_rate = mutation_rate;
        self.eval = cfg.eval_config();
        self.trainer = Trainer::new(cfg.clone());
        self.cfg = cfg;
        self.traces = TraceLog::new();
        self.history.clear();
        self.deaths.clear();
        self.level = level;
    }

    /// The viewer's live mutation-rate slider: applied from the next generation on, so a
    /// rate change shows up in the very next offspring (the terminal's behaviour).
    pub fn set_mutation_rate(&mut self, rate: f32) {
        self.cfg.mutation_rate = rate;
    }

    /// Evaluate one generation and package everything the window needs to draw it.
    pub fn next_packet(&mut self) -> Packet {
        // The population snapshot is taken before breeding, so these are the exact
        // genomes (with the exact ids) the report describes.
        self.trainer.config.mutation_rate = self.cfg.mutation_rate;
        let (report, population) = self.trainer.step_verbose(&self.level);
        let showcase = self
            .trainer
            .showcase(&report, &population, self.best_k, self.random_k);

        // Labels count within each group rather than using `lane.rank`: the trainer
        // numbers random lanes `best_k + n`, so a rank would name the first outsider
        // "best 5" whenever four best lanes precede it.
        let (mut best, mut random) = (0usize, 0usize);
        let lanes: Vec<Lane> = showcase
            .iter()
            .enumerate()
            .map(|(i, lane)| {
                let ordinal = match lane.kind {
                    LaneKind::Best => {
                        best += 1;
                        best - 1
                    }
                    LaneKind::Random => {
                        random += 1;
                        random - 1
                    }
                };
                Lane {
                    name: lane_name(lane.kind, ordinal),
                    best: lane.kind == LaneKind::Best,
                    rank: lane.rank,
                    genome_id: lane.genome_id,
                    fitness: lane.fitness,
                    mean_fitness: lane.mean_fitness,
                    colour: lane_colour(i),
                    genome: population[lane.index].clone(),
                    seed: lane.seed,
                }
            })
            .collect();

        // Record the champion's route when it is an improvement (§10 Tier 2.7). Asked
        // *before* paying: the recording costs one extra episode, and a stalled run —
        // the case the scrubber exists for — improves on almost no generation at all.
        if self.traces.would_record(report.max_fitness) {
            if let Some(lane) = lanes.first() {
                let trace = record_trace(
                    &self.level,
                    &self.eval,
                    &lane.genome,
                    lane.seed,
                    report.generation,
                    report.max_fitness,
                );
                self.traces.observe(trace);
            }
        }

        // Deaths from the whole population, not only the lanes: `ShowcaseLane` is a
        // sample of eight, while the report already carries every agent's ending, so the
        // map of where the population is stuck costs nothing extra here.
        for result in &report.results {
            if let Some((x, y)) = result.death_pos {
                self.deaths.push((x, y, report.generation));
            }
        }
        self.deaths
            .retain(|(_, _, g)| report.generation.saturating_sub(*g) <= DEATH_WINDOW);

        self.history.push(HistoryPoint {
            generation: report.generation,
            max_fitness: report.max_fitness,
            avg_fitness: report.avg_fitness,
            wins: report.wins,
        });
        let run = self.history.len() as u32;

        Packet {
            generation: report.generation,
            max_fitness: report.max_fitness,
            avg_fitness: report.avg_fitness,
            min_fitness: report.min_fitness,
            wins: report.wins,
            genius: report.genius,
            pop_size: self.cfg.pop_size,
            lanes,
            history: self.history.clone(),
            deaths: self.deaths.clone(),
            traces: self.traces.entries().to_vec(),
            best: self.trainer.best_ever().cloned(),
            target: self.target,
            level_name: self.level.name.clone(),
            level_hash: self.level.content_hash_string(),
            finished: self.target.is_some_and(|t| run >= t),
        }
    }
}

/// Record one generation's champion route, in full.
///
/// Run as its own episode rather than sampled off the animation, for the reason given
/// on [`ChampionTrace`]: the record has to be a fact about the run, not about how long
/// the viewer happened to watch. The sampler is `render`'s
/// [`TRACE_SAMPLE_FRAMES`], so a route recorded here has the same resolution as one
/// recorded by the terminal mode.
fn record_trace(
    level: &Level,
    eval: &EvalConfig,
    genome: &Genome,
    seed: u64,
    generation: u32,
    max_fitness: f32,
) -> ChampionTrace {
    let mut runner = EpisodeRunner::new(level, genome.clone(), eval, seed);
    let mut points = Vec::new();
    while !runner.is_finished() {
        let frames = runner.sim().stats().frames;
        if frames % TRACE_SAMPLE_FRAMES == 0 {
            let p = runner.sim().player();
            points.push(TracePoint {
                frame: frames,
                x: p.x,
                y: p.y,
            });
        }
        runner.step();
    }

    // The final position is the one that matters most — the flag, or the grave — and an
    // episode rarely ends on a sample boundary, so it is recorded exactly rather than
    // rounded to the nearest sample.
    let p = runner.sim().player();
    let end = TracePoint {
        frame: runner.sim().stats().frames,
        x: p.x,
        y: p.y,
    };
    if points.last().map(|q| (q.x, q.y)) != Some((end.x, end.y)) {
        points.push(end);
    }

    ChampionTrace {
        generation,
        max_fitness,
        won: runner.sim().outcome() == Outcome::Won,
        points,
    }
}

/// How the panel names a lane.
fn lane_name(kind: LaneKind, ordinal: usize) -> String {
    match kind {
        LaneKind::Best if ordinal == 0 => "champion".to_string(),
        LaneKind::Best => format!("best {}", ordinal + 1),
        LaneKind::Random => format!("random {}", ordinal + 1),
    }
}

/// A lane's outline colour, by position in the showcase.
fn lane_colour(ordinal: usize) -> [u8; 3] {
    LANE_COLOURS[ordinal % LANE_COLOURS.len()]
}

// ---------------------------------------------------------------------------
// the worker
// ---------------------------------------------------------------------------

/// What the window and the worker say to each other.
struct Hub {
    state: Mutex<State>,
    cv: Condvar,
}

#[derive(Default)]
struct State {
    /// A finished generation the window has not taken yet.
    queued: Option<Packet>,
    /// The window has finished animating and is ready for the next generation. This is
    /// the whole reason the run cannot train ahead of the viewer.
    want_next: bool,
    quit: bool,
    /// A level to start a fresh run on, with its seed.
    restart: Option<(Level, u64)>,
    mutation_rate: Option<f32>,
    finished: bool,
}

/// The worker loop: one generation per request, parked in between.
fn work(hub: Arc<Hub>, mut generator: Generator, initial_rate: f32) {
    loop {
        let (restart, rate) = {
            let mut state = hub.state.lock().unwrap();
            while !state.quit && !state.want_next && state.restart.is_none() {
                state = hub.cv.wait(state).unwrap();
            }
            if state.quit {
                return;
            }
            state.want_next = false;
            (state.restart.take(), state.mutation_rate)
        };

        if let Some((level, seed)) = restart {
            generator.start_over(level, seed, rate.unwrap_or(initial_rate));
        } else if let Some(rate) = rate {
            generator.set_mutation_rate(rate);
        }

        let packet = generator.next_packet();
        let finished = packet.finished;

        let mut state = hub.state.lock().unwrap();
        while state.queued.is_some() && !state.quit {
            state = hub.cv.wait(state).unwrap();
        }
        if state.quit {
            return;
        }
        state.queued = Some(packet);
        if finished {
            state.finished = true;
        }
        hub.cv.notify_all();
        if finished {
            return;
        }
    }
}

/// The window's handle on a running evolution.
pub struct Lab {
    hub: Arc<Hub>,
    worker: Option<JoinHandle<()>>,
    eval: EvalConfig,
    /// The level this run was started on, as a copy rather than as a reference to the
    /// editor's document: a lane has to be animated on the terrain its generation was
    /// evaluated on, and an edit made mid-generation must not change the episode being
    /// replayed (which is why [`Lab::take`] discards any generation whose hash has moved).
    level: Level,
    level_hash: String,
    level_name: String,
    pop_size: usize,
    /// The window has already asked for the generation it is waiting for, so it does not
    /// ask again every frame.
    asked: bool,
    /// The run's configuration, mirrored here for the champion file's metadata.
    config: GaConfig,
}

impl Lab {
    /// Start a run on `level`. `target` caps the generations, for a scripted or
    /// screenshot run; `None` runs until the window stops it.
    pub fn start(
        level: &Level,
        cfg: GaConfig,
        best_k: usize,
        random_k: usize,
        target: Option<u32>,
    ) -> Result<Lab, String> {
        let cfg = cfg.sanitized(None);
        let generator = Generator::new(level.clone(), cfg.clone(), best_k, random_k, target);
        let rate = cfg.mutation_rate;
        let hub = Arc::new(Hub {
            state: Mutex::new(State {
                // A fresh run should produce its first generation immediately: waiting
                // for the first "next" would show an empty panel for no reason.
                want_next: true,
                ..State::default()
            }),
            cv: Condvar::new(),
        });
        let worker = {
            let hub = Arc::clone(&hub);
            thread::Builder::new()
                .name("evolution".into())
                .spawn(move || work(hub, generator, rate))
                .map_err(|e| format!("cannot start the evolution thread: {e}"))?
        };
        Ok(Lab {
            hub,
            worker: Some(worker),
            eval: cfg.eval_config(),
            level: level.clone(),
            level_hash: level.content_hash_string(),
            level_name: level.name.clone(),
            pop_size: cfg.pop_size,
            asked: true,
            config: cfg,
        })
    }

    /// The level the run is training on. Not the editor's document: a lane has to be
    /// drawn on the terrain its generation was evaluated on, and the panel warns when
    /// the two have diverged.
    pub fn level(&self) -> &Level {
        &self.level
    }

    /// The configuration the run is using, including the mutation rate the viewer has
    /// dialled in. A saved champion records it, so its metadata describes the run that
    /// produced it rather than the defaults it started from.
    pub fn config(&self) -> &GaConfig {
        &self.config
    }

    pub fn level_name(&self) -> &str {
        &self.level_name
    }

    pub fn pop_size(&self) -> usize {
        self.pop_size
    }

    /// Was this run started on the level as it stands now?
    ///
    /// The hash covers the grid, the spawn and every entity box, so this answers
    /// "the same level" rather than "the same file name": an edit that matters to the
    /// physics is an edit that makes the run's numbers a claim about a level nobody can
    /// see any more.
    pub fn trained_on(&self, level: &Level) -> bool {
        self.level_hash == level.content_hash_string()
    }

    /// Take the generation that is ready, if any, and start animating it.
    ///
    /// A generation evaluated before a restart belongs to the level this run has moved
    /// on from; animating it would draw one level's episode on another level's terrain,
    /// so it is discarded and the fresh run's generation — which the worker has already
    /// queued behind it — is taken instead.
    pub fn take(&mut self) -> Option<Lanes> {
        loop {
            let packet = {
                let mut state = self.hub.state.lock().unwrap();
                let packet = state.queued.take()?;
                self.hub.cv.notify_all();
                packet
            };
            if packet.level_hash != self.level_hash {
                continue;
            }
            self.asked = false;
            return Some(Lanes::new(packet, &self.level, &self.eval));
        }
    }

    /// Ask for the next generation. Called when the animation of the current one
    /// finishes — the run's own rate limit, and the reason a paused window trains
    /// nothing.
    pub fn request_next(&mut self) {
        let mut state = self.hub.state.lock().unwrap();
        if state.finished || state.quit {
            return;
        }
        state.want_next = true;
        self.asked = true;
        self.hub.cv.notify_all();
    }

    /// Has the window already asked for the generation it is waiting for?
    pub fn asked(&self) -> bool {
        self.asked
    }

    /// The viewer's live mutation-rate control.
    pub fn set_mutation_rate(&mut self, rate: f32) {
        self.config.mutation_rate = rate;
        let mut state = self.hub.state.lock().unwrap();
        state.mutation_rate = Some(rate);
        self.hub.cv.notify_all();
    }

    /// Start a fresh run — a new population — on `level`.
    pub fn restart(&mut self, level: &Level, seed: u64) {
        self.level = level.clone();
        self.level_hash = level.content_hash_string();
        self.level_name = level.name.clone();
        self.config.master_seed = seed;
        let mut state = self.hub.state.lock().unwrap();
        state.restart = Some((level.clone(), seed));
        // A restart is also a request: the window wants to see the new run's first
        // generation without having to press something else.
        state.want_next = true;
        self.asked = true;
        self.hub.cv.notify_all();
    }

    /// Wait up to `budget` for a generation, for the scripted screenshot path: a frame
    /// captured `cargo run`-style has to contain a real generation, or it verifies
    /// nothing.
    pub fn wait_for_first(&mut self, budget: Duration) -> Option<Lanes> {
        let deadline = Instant::now() + budget;
        loop {
            // Only ask when nothing is pending. Asking again while the worker is already
            // running a generation would queue a second one behind it, which is exactly
            // the "trains ahead of the viewer" behaviour this design forbids.
            if !self.asked {
                self.request_next();
            }
            if let Some(lanes) = self.take() {
                return Some(lanes);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Stop the worker and wait for it. A run that outlived its window would keep
    /// training with nobody watching, which is exactly what this design forbids.
    pub fn stop(&mut self) {
        {
            let mut state = self.hub.state.lock().unwrap();
            state.quit = true;
            self.hub.cv.notify_all();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evo::score;

    fn shipped(slug: &str) -> Level {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"));
        Level::from_json_file(&path).unwrap_or_else(|e| panic!("{slug}: {e}"))
    }

    /// A population small enough that a test can run several generations in
    /// milliseconds. The GA's own behaviour is `evo`'s business; what is tested here is
    /// the packaging and the animation.
    fn tiny_config() -> GaConfig {
        GaConfig {
            pop_size: 8,
            elite: 2,
            tournament: 3,
            ..GaConfig::default()
        }
    }

    fn new_generator(slug: &str, target: Option<u32>) -> Generator {
        Generator::new(
            shipped(slug),
            tiny_config(),
            DEFAULT_BEST_LANES,
            DEFAULT_RANDOM_LANES,
            target,
        )
    }

    fn wait_for(lab: &mut Lab, budget: Duration) -> Option<Lanes> {
        let deadline = Instant::now() + budget;
        loop {
            if let Some(lanes) = lab.take() {
                return Some(lanes);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_generation_carries_its_lanes_history_and_deaths() {
        let mut generator = new_generator("l2-first-gap", None);
        let first = generator.next_packet();
        assert_eq!(first.generation, 0);
        assert_eq!(first.pop_size, 8);
        assert_eq!(first.lanes.len(), 8, "4 best + 4 random");
        assert_eq!(first.lanes[0].name, "champion");
        assert!(first.lanes[0].best);
        assert_eq!(first.lanes[4].name, "random 1");
        assert!(!first.lanes[4].best);
        assert_eq!(first.history.len(), 1);
        assert!(!first.finished);

        // Every lane can be animated without the population it came from.
        let level = generator.level().clone();
        let eval = tiny_config().eval_config();
        let lanes = Lanes::new(first, &level, &eval);
        for i in 0..lanes.len() {
            assert_eq!(lanes.sim(i).stats().frames, 0, "a lane starts at frame 0");
        }

        // A second generation extends the history rather than replacing it, and its
        // lanes are drawn from the population that was just scored.
        let second = generator.next_packet();
        assert_eq!(second.history.len(), 2);
        assert_eq!(second.generation, 1);
        assert!(second.lanes.iter().all(|l| l.genome.n_genes() > 0));
    }

    #[test]
    fn deaths_are_tagged_with_their_generation_and_forgotten() {
        // L2 has a pit, so a random population falls in it almost immediately.
        let mut generator = new_generator("l2-first-gap", None);
        let mut seen = false;
        for _ in 0..4 {
            let packet = generator.next_packet();
            for (_, _, generation) in &packet.deaths {
                assert!(*generation <= packet.generation);
            }
            seen |= !packet.deaths.is_empty();
        }
        assert!(seen, "some agent should have died in a pit by now");

        // The window keeps a rolling window: with a cap of DEATH_WINDOW generations, a
        // mark older than that is dropped rather than drawn.
        let mut generator = new_generator("l2-first-gap", None);
        for _ in 0..(DEATH_WINDOW + 3) {
            let packet = generator.next_packet();
            let newest = packet.generation;
            assert!(
                packet
                    .deaths
                    .iter()
                    .all(|(_, _, g)| newest.saturating_sub(*g) <= DEATH_WINDOW),
                "an old corpse is still on the map"
            );
        }
    }

    #[test]
    fn a_run_stops_at_its_cap() {
        let mut generator = new_generator("l1-flat-run", Some(2));
        assert!(!generator.next_packet().finished);
        let last = generator.next_packet();
        assert!(last.finished, "the second of two generations is the last");
        assert_eq!(last.history.len(), 2);
    }

    #[test]
    fn starting_over_forgets_the_old_run() {
        let mut generator = new_generator("l1-flat-run", None);
        generator.next_packet();
        generator.next_packet();
        assert_eq!(generator.history.len(), 2);

        let other = shipped("l3-gauntlet");
        generator.start_over(other.clone(), 99, 0.10);
        assert_eq!(generator.level().name, other.name);
        let fresh = generator.next_packet();
        assert_eq!(fresh.generation, 0, "a fresh population starts at generation 0");
        assert_eq!(fresh.history.len(), 1, "the old chart is gone with the old run");
        // A first generation always records (there is nothing to beat), so the record is
        // not empty — but everything in it belongs to the fresh run, which is what
        // "the old record is gone" actually means.
        assert!(
            fresh.traces.iter().all(|t| t.generation == 0),
            "an old route survived the restart: {:?}",
            fresh.traces.iter().map(|t| t.generation).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_keyboard_ladders_stop_at_both_ends() {
        assert_eq!(step_speed(1, false), 1, "the bottom is a stop, not a wrap");
        assert_eq!(step_speed(1, true), 2);
        assert_eq!(step_speed(8, true), 16);
        assert_eq!(step_speed(50, true), 50);
        // An off-ladder value (a hand-edited config, a future rung) snaps in first.
        assert_eq!(step_speed(9, true), 16);
        assert_eq!(step_speed(9, false), 4, "9 snaps to 8, then steps down");

        assert_eq!(step_mutation_rate(0.03, false), 0.03);
        assert_eq!(step_mutation_rate(0.03, true), 0.05);
        assert_eq!(step_mutation_rate(0.10, true), 0.10);
        assert_eq!(step_mutation_rate(0.10, false), 0.08);
        assert_eq!(
            step_mutation_rate(0.07, true),
            0.10,
            "0.07 snaps to 0.08, then steps up"
        );
    }

    #[test]
    fn a_lane_replays_the_episode_that_was_scored() {
        // The load-bearing claim of the whole view: what is drawn next to a number is
        // the episode that earned the number. Animated to its end, a lane's own score
        // must be the fitness the packet advertises.
        let mut generator = new_generator("l2-first-gap", None);
        let packet = generator.next_packet();
        let level = generator.level().clone();
        let eval = tiny_config().eval_config();
        let advertised: Vec<f32> = packet.lanes.iter().map(|l| l.fitness).collect();
        let mut lanes = Lanes::new(packet, &level, &eval);

        assert!(!lanes.is_finished());
        let mut guard = 0;
        while !lanes.is_finished() {
            lanes.advance(1.0 / 60.0, 50);
            guard += 1;
            assert!(guard < 10_000, "the animation never finished");
        }
        assert_eq!(lanes.running(), 0);

        for (i, expected) in advertised.iter().enumerate() {
            let sim = lanes.sim(i);
            let got = score(sim.stats(), sim.outcome());
            assert!(
                (got - expected).abs() < 1e-3,
                "lane {i} ({}) animated an episode scoring {got}, but advertises {expected}",
                lanes.lanes()[i].name
            );
        }
    }

    #[test]
    fn a_recorded_route_ends_where_the_champion_ended() {
        let mut generator = new_generator("l2-first-gap", None);
        let packet = generator.next_packet();
        let level = generator.level().clone();
        let eval = tiny_config().eval_config();
        let lane = packet.lanes[0].clone();
        let trace = record_trace(
            &level,
            &eval,
            &lane.genome,
            lane.seed,
            packet.generation,
            packet.max_fitness,
        );

        assert_eq!(trace.generation, packet.generation);
        assert_eq!(trace.max_fitness, packet.max_fitness);
        assert!(!trace.points.is_empty());

        // Replaying the same genome and seed gives the same ending: the route is the
        // episode, not a summary of one.
        let mut replay = EpisodeRunner::new(&level, lane.genome.clone(), &eval, lane.seed);
        while !replay.is_finished() {
            replay.step();
        }
        let end = trace.end().expect("a recorded route has an end");
        let p = replay.sim().player();
        assert_eq!((end.x, end.y), (p.x, p.y));
        assert_eq!(end.frame, replay.sim().stats().frames);
        assert_eq!(trace.won, replay.sim().outcome() == Outcome::Won);
        // And the record is a claim about the number on screen.
        assert!((score(replay.sim().stats(), replay.sim().outcome()) - lane.fitness).abs() < 1e-3);
    }

    #[test]
    fn speed_is_frame_rate_independent_and_clamped() {
        // At 60 Hz, 8x is eight ticks a frame; at 120 Hz the same second of wall clock
        // is the same number of ticks, spread over twice the frames.
        assert!((ticks_for(1.0 / 60.0, 8) - 8.0).abs() < 1e-4);
        let sixty: f32 = (0..60).map(|_| ticks_for(1.0 / 60.0, 8)).sum();
        let hundred_twenty: f32 = (0..120).map(|_| ticks_for(1.0 / 120.0, 8)).sum();
        assert!((sixty - hundred_twenty).abs() < 1e-2);
        assert!((sixty - 480.0).abs() < 1e-2, "8x is 480 ticks a second");

        // A hitch is clamped, exactly as the playtest clamps it.
        assert!((ticks_for(5.0, 8) - ticks_for(MAX_CATCHUP, 8)).abs() < 1e-4);

        // And the ladder's ends are both sane.
        assert!(ticks_for(1.0 / 60.0, *SPEEDS.first().unwrap()) >= 1.0);
        assert!(ticks_for(1.0 / 60.0, *SPEEDS.last().unwrap()) < 100.0);
    }

    #[test]
    fn a_run_waits_for_the_viewer_between_generations() {
        let level = shipped("l1-flat-run");
        let mut lab = Lab::start(&level, tiny_config(), 4, 4, Some(2)).expect("thread starts");
        assert!(lab.trained_on(&level));
        assert!(!lab.trained_on(&shipped("l2-first-gap")));

        let first = lab
            .wait_for_first(Duration::from_secs(30))
            .expect("the first generation arrives");
        assert_eq!(first.packet().generation, 0);
        assert!(!first.packet().finished);

        // The worker is parked, not training on a timer: nothing is waiting for the
        // window, and nothing will be until it asks.
        thread::sleep(Duration::from_millis(200));
        assert!(
            lab.take().is_none(),
            "a parked run must not train ahead of the viewer"
        );

        lab.request_next();
        let second = wait_for(&mut lab, Duration::from_secs(30))
            .expect("the next generation arrives once it is asked for");
        assert_eq!(second.packet().generation, 1);
        assert!(second.packet().finished, "the cap was two");
    }

    #[test]
    fn a_fresh_run_starts_on_the_level_it_was_asked_for() {
        let level = shipped("l1-flat-run");
        let mut lab = Lab::start(&level, tiny_config(), 2, 2, None).expect("thread starts");
        // A restart does not wait for the animation to finish: the window may want to
        // abandon a run it has already seen the first generation of.
        lab.restart(&shipped("l3-gauntlet"), 7);
        assert!(lab.trained_on(&shipped("l3-gauntlet")));
        let first = wait_for(&mut lab, Duration::from_secs(30)).expect("the fresh run reports");
        assert_eq!(first.packet().generation, 0);
        assert_eq!(first.packet().level_name, shipped("l3-gauntlet").name);
    }

    #[test]
    fn dropping_the_lab_stops_the_worker() {
        let level = shipped("l1-flat-run");
        let lab = Lab::start(&level, tiny_config(), 4, 4, None).expect("thread starts");
        let started = Instant::now();
        drop(lab);
        // `stop` joins the thread, so returning here at all proves the quit handshake
        // woke a parked worker rather than leaving a thread training behind a window
        // that no longer exists.
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the worker must be joined, not abandoned"
        );
    }
}
