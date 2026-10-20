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

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use evo::{
    curriculum_step, from_certified_plan, practice_level_for_stall, practice_step,
    save_practice_level, warm_population, BestEver, ChampionFile, EpisodeResult, EpisodeRunner,
    EvalConfig, GaConfig, Genome, ImitateConfig, LaneKind, PopulationFile, Trainer,
};
use render::chart::{fitness_stall, HistoryPoint};
use render::trace::{ChampionTrace, TraceLog, TracePoint, TRACE_SAMPLE_FRAMES};
use sim::{Level, Outcome, Sim};

use crate::replay::Replay;
use crate::view::AgentAnim;

pub use evo::{DEFAULT_CURRICULUM_GENERATIONS, MUTATION_RATES};

/// Generations of death marks the canvas keeps, matching watch mode's `--death-window`:
/// a forty-generation-old corpse says nothing about where the population is stuck *now*.
pub const DEATH_WINDOW: u32 = 25;

/// Best lanes and random lanes drawn per generation (§10 Tier 1.5). Best-4 + random-4
/// rather than best-only on purpose: a population that has collapsed onto one strategy
/// is exactly the failure the view has to make visible, and a chart cannot show it.
pub const DEFAULT_BEST_LANES: usize = 4;
pub const DEFAULT_RANDOM_LANES: usize = 4;

/// Whether the worker prints the cost of each part of a generation to stderr.
///
/// Set `LAB_PROFILE=1` to profile a run without a profiler: the writes are stderr-only and only
/// on this env var's value, so a normal run pays one `env::var_os` per generation.
fn profiling() -> bool {
    std::env::var_os("LAB_PROFILE").is_some()
}

/// A file-name-safe version of a level name.
///
/// The level's *name*, not its file's: a level drawn in the editor has no file until it
/// is saved, and one level should not produce two champions depending on how the run was
/// launched. Everything that is not a letter or a digit collapses to a dash, so the
/// result is always a usable file name whatever the level is called.
pub fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '_' | '-') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}

/// The planner budget the window's warm start compiles with.
///
/// 64, not the planner's shipped default (32) and not the practice suite's certification depth
/// (48): `l11-the-elements` — the level the plan's own diagnosis turns on — needs 64 expansions
/// before its 11-leg route appears. A depth is a cap rather than a cost, so the levels that route
/// sooner are unaffected; the ones that need the extra rungs stop being silently unwarmed.
const WARM_PLAN_DEPTH: u32 = 64;

/// A fresh population's worth of genomes warmed from the level's **timed plan**, or no population
/// plus the reason when there is no certified plan.
///
/// The whole of the window's warm start, and the Phase 1 replacement for the old route walk: the
/// route compiler ([`sim::route_plan::compile`]) certifies the level twice (oracle, then a planner
/// route that must win a replay) and cuts it into legs; [`evo::from_certified_plan`] clones the
/// block-aligned demonstration and runs the DAgger rounds; a quarter of the population starts from
/// that clone and the rest are random.
///
/// **Why the plan and not the walk.** The oracle's route is indexed in ticks, while a policy decides
/// once per four ticks, so a clone of the walk walks the route and steps into the first pit at the
/// shipped cadence (`evo::imitate` documents the measurement). The planner's route is already
/// written in decision blocks, so a clone that reproduces it is playing the control grid it has.
///
/// **Nothing here can fail a run, and nothing here fails silently.** A level the oracle says is
/// unbeatable, or one the oracle certifies but this planner budget cannot route, returns no warm
/// population **and a note naming which of the two happened** — the population then starts random.
/// In particular this never falls back to the tick-indexed route walk: a clone of a route the
/// planner could not certify is a warm start from a weaker demonstration, and the caller would have
/// no way to tell which one it got.
///
/// The fit is the shipped default (300 gradient passes plus six DAgger rounds). It runs once, on the
/// worker thread, before the first generation; the profile line says what it cost.
fn warm_seed_from_plan(level: &Level, cfg: &GaConfig) -> (Option<Vec<Genome>>, String) {
    warm_seed_from_plan_with(
        level,
        cfg,
        sim::plan::PlanConfig {
            depth: WARM_PLAN_DEPTH,
            ..sim::plan::PlanConfig::default()
        },
        &ImitateConfig::default(),
    )
}

/// [`warm_seed_from_plan`] with the planner budget and the fit under the caller's control. The
/// no-route case is testable without a full-depth search, and a test that only needs to see the
/// clone and the note does not have to pay for the shipped epoch budget.
fn warm_seed_from_plan_with(
    level: &Level,
    cfg: &GaConfig,
    plan_cfg: sim::plan::PlanConfig,
    imit: &ImitateConfig,
) -> (Option<Vec<Genome>>, String) {
    let started = Instant::now();
    let plan = match sim::route_plan::compile(level, cfg.master_seed, plan_cfg) {
        Ok(plan) => plan,
        Err(err) => {
            return (
                None,
                format!(
                    "no warm start: {}. The population starts random rather than from a weaker \
                     route",
                    err.line()
                ),
            )
        }
    };
    let eval = cfg.eval_config();
    let route = plan.timed_route();
    let fit = match from_certified_plan(level, &route, &eval, cfg.shape.clone(), imit) {
        Ok((play, report)) => (play, report),
        Err(why) => {
            return (
                None,
                format!(
                    "no warm start: {why}. The population starts random rather than from a \
                     weaker route"
                ),
            )
        }
    };
    let (play, report) = fit;
    if profiling() {
        eprintln!(
            "lab warm plan+clone {:?} ({} decisions, {} legs, {:.0}% agreement)",
            started.elapsed(),
            play.decisions.len(),
            plan.legs.len(),
            report.agreement * 100.0
        );
    }
    let mut rng = sim::rng::rng_from_seed(cfg.master_seed ^ 0x2E5E_ED2E_5EED_2E5E);
    let warm = (cfg.pop_size / 4).max(1);
    let pop = warm_population(
        &report.genome,
        cfg.pop_size,
        warm - 1,
        cfg.mutation_sigma,
        cfg.mutation_rate,
        &mut rng,
    );
    let coverage = if plan.covered == plan.hops {
        format!("{} legs / {} rungs", plan.legs.len(), plan.hops)
    } else {
        format!(
            "{} legs, but the flag is reached after {} of {} route rungs",
            plan.legs.len(),
            plan.covered,
            plan.hops
        )
    };
    (
        Some(pop),
        format!(
            "warmed from this level's timed plan: {coverage} in {} blocks and {}; a genome was \
             cloned from the plan ({:.0}% of the demonstrated decisions reproduced) and {warm} of \
             {} genomes start from that clone, the rest are random; {}",
            plan.actions.len(),
            sim::format_time(plan.frames),
            report.agreement * 100.0,
            cfg.pop_size,
            plan.line()
        ),
    )
}

/// How a run was asked for: how long it may run, where it starts from, and what to do
/// when it stops moving.
///
/// One value rather than four arguments to [`Lab::start`], because these arrive together
/// (from the command line or from the window) and belong to the *run*, not to the level
/// it happens to be training on. A restart edits the resume field rather than rebuilding
/// the whole thing: the population is new, the settings are the same.
#[derive(Clone, Debug, Default)]
pub struct RunPlan {
    /// Stop after this many generations. `None` runs until the window stops it.
    pub cap: Option<u32>,
    /// Start from a population checkpoint instead of a fresh population, as the
    /// terminal's `--resume` does. The checkpoint's generation and genome ids continue,
    /// because the ids are part of every episode seed.
    pub resume: Option<PathBuf>,
    /// Seed a **new** population from one saved champion, as the terminal's
    /// `--from-champion` does: the champion in slot 0 and a shell of reheated copies of it
    /// behind, starting at generation 0 with its recorded episode standing as the run's
    /// record.
    ///
    /// The alternative to [`RunPlan::resume`], not a second step after it: a checkpoint is
    /// a population and this is one genome, and a run that was stopped without a checkpoint
    /// leaves only the champion. The level hash is deliberately not checked — the interesting
    /// seed is exactly a champion from a level that has changed since — and the run says
    /// which level it came from instead ([`ChampionFile::level_note`]).
    pub seed_from_champion: Option<PathBuf>,
    /// The easier level to fall back to when the run stalls, and for how long.
    pub curriculum: Option<Curriculum>,
    /// Warm a **fresh** population from the level's certified **timed plan** (Phase 1): compile a
    /// [`sim::route_plan::RoutePlan`], clone its block-aligned demonstration with DAgger, and start
    /// a quarter of generation 0 from that clone.
    ///
    /// It is a *default* for the window rather than something to ask for, because the level the
    /// window trains on is usually one somebody has just drawn. It replaces the old tick-indexed
    /// route walk, whose clone could not place a press on the policy's own cadence. A level the
    /// planner cannot route starts **random** and says why in the run's origin note; it is never
    /// warmed from the weaker route instead. Ignored when the run resumes a checkpoint, which
    /// already *has* a population.
    pub warm_from_plan: bool,
}

/// A run's fallback level: what a stalled population gets trained on instead (§12 M5).
#[derive(Clone, Debug)]
pub struct Curriculum {
    pub level: Level,
    /// Generations the detour lasts. At least one: a detour of zero generations would
    /// checkpoint the population and hand it straight back, which is not a step.
    pub generations: u32,
}

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

/// Write a run's best-ever genome out as a champion file, exactly as `train-cli train`
/// writes one.
///
/// This lives here rather than in the window because it is the one thing the view does
/// that touches the disk, and the window is the one part of the crate a test cannot
/// drive. It is the same `ChampionFile` the terminal writes, with the same provenance
/// copied from a real evaluation — so a champion saved while *watching* is a champion
/// `train-cli replay` can re-verify, and watching instead of training wastes nothing.
///
/// The file is named after the level, which is how a level's champion is always found in
/// the same place whatever produced it.
pub fn save_champion(
    best: &BestEver,
    level: &Level,
    config: &GaConfig,
    dir: &Path,
    stem: &str,
) -> Result<PathBuf, String> {
    let champion = ChampionFile::new(&best.genome, &best.result, level, best.generation, config);
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(format!("{stem}.json"));
    champion.save(&path)?;
    Ok(path)
}

/// One agent of a generation, as it travels to the window.
///
/// The genome *and* the episode it was scored on, because a genome alone cannot
/// reproduce an episode: the seed is the other half of it. The lanes carry this pair for
/// the eight agents the run draws; the packet carries it for all of them, because the
/// replay ghosts every other agent of the generation beside the star, and a ghost is only
/// honest if it is running the episode that was actually scored.
#[derive(Clone, Debug)]
pub struct Agent {
    pub genome: Genome,
    pub result: EpisodeResult,
}

/// One generation, packaged for the window.
#[derive(Clone, Debug)]
pub struct Packet {
    pub generation: u32,
    pub max_fitness: f32,
    pub avg_fitness: f32,
    pub min_fitness: f32,
    /// Agents that touched the flag on the episode they were judged on.
    pub wins: usize,
    /// Agents that won **every** episode they were judged on — the conservative reading of the same
    /// census, and the one that is not a coin flip where a patrol phase is drawn. Equal to `wins`
    /// on a level with nothing stochastic in it.
    pub solved: usize,
    /// `max > 1.5 × previous max` — the "genius agent" splash (§10 Tier 1.3).
    pub genius: bool,
    pub pop_size: usize,
    /// The mutation step the children bred after this generation were given.
    ///
    /// A reading rather than a setting: a run with a fixed sigma holds it at one value — the
    /// default — and a run with `adaptive_sigma` moves it between generations, down while its
    /// children stop beating their parents and up while they mostly do. The panel wants it
    /// because it is the one number that says how big a move the search is making, which no
    /// chart of the population's score can show.
    pub mutation_sigma: f32,
    /// Generations since the best fitness last improved.
    ///
    /// The trainer's own counter, carried rather than re-derived: the panel's stall rule
    /// reads the *history*, and this is the live process that produced it.
    pub stagnation: u32,
    /// Fresh random genomes bred in after that stagnation, which is non-zero only after a
    /// stall — so a non-zero value is also the statement that the run has stopped moving and
    /// done something about it.
    pub immigrants: usize,
    /// Children bred out of the quality-diversity archive: perturbations of the best genome
    /// in a behavioural niche, rather than of anything in the generation's ranking.
    pub archive_children: usize,
    /// Children bred from a **mistake** the generation's best genome made (Item 14): each is the
    /// best genome with its output layer moved just far enough to take the better action at a
    /// decision that lost. Zero when [`evo::GaConfig::repair_children`] is off, which is the
    /// terminal default and not the window's.
    pub repairs: usize,
    /// Behavioural niches the archive holds, of `evo::ARCHIVE_CELLS`. The run's *diversity*
    /// reading: a population that has collapsed onto one way of moving fills one or two
    /// niches and stays there, which no chart of its scores can show.
    pub archive_bits: usize,
    /// Finish bands holding a winner, of `evo::ARCHIVE_TIME_BINS`. The run's *speed* coverage,
    /// and a different reading from the one above: a run that won four times inside the same
    /// quarter of the level's budget holds one band, so the count only climbs when the
    /// population learns to solve the level *quicker* rather than solve it at all.
    pub archive_time_bits: usize,
    /// **The episode** the record above was made on: the genome, and the result carrying the
    /// seed that replays it.
    ///
    /// [`Packet::fastest`] is the time and this is the agent, and they are written together by
    /// the trainer, so `fastest.frames == fastest_agent.result.frames` holds. That is the whole
    /// reason both are on the packet: the window quotes the time *and* races the ghost, and a
    /// ghost drawn from any other genome would finish at a different moment from the number
    /// beside it — a race against two different records, which is not a race.
    pub fastest_agent: Option<evo::BestEver>,
    /// The quickest winning episode the run has recorded, and whose it was (see
    /// [`evo::FastWin`]).
    ///
    /// The clock a viewer races. A **run**-level record rather than a per-generation one:
    /// a generation that won nothing leaves the previous best time standing, so the number on
    /// screen is the same one from frame to frame — which is what makes it a time to beat
    /// rather than a number that flickers. It is also a different record from `best`: `best`
    /// is the genome with the highest fitness, and since fitness v8 that score includes the
    /// clock but is not the clock, so the two can be different genomes as soon as a run has
    /// two winners.
    pub fastest: Option<evo::FastWin>,
    pub lanes: Vec<Lane>,
    /// Every agent the generation scored, in population order, index-aligned with the
    /// report's own results.
    ///
    /// The lanes are a sample chosen for drawing eight agents legibly; this is the
    /// population itself, and it exists for the replay, which ghosts every other agent of
    /// the generation beside the champion. It is the one heavy field here — the copy
    /// `step_verbose` already makes for the lanes is the same ~1.2 MB at pop 200 — and it
    /// is paid once per generation rather than once per frame.
    pub agents: Vec<Agent>,
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
    /// The note from the run's one curriculum step, if it fell back to an easier level
    /// when it stalled (§12 M5). `None` for a run that never stalled, or one with no
    /// easier level to fall back to.
    pub curriculum: Option<String>,
    /// The run reached its cap and the worker has stopped.
    pub finished: bool,
    /// **A find this generation produced (WP16).** Present only when the record moved *and*
    /// detection named something the level's codex had not held before: a rediscovery is not
    /// news, and a generation that did not move the record cannot have found anything new.
    /// The window shows the newest one and the panel prints the line.
    pub discovery: Option<evo::Discovery>,
    /// How many finds the level's codex holds after this generation, so the panel can say
    /// "2 of 7" without reading a file per frame.
    pub codex_len: usize,
    /// **Phase 0: what this generation learned**, in the terms a fitness scalar cannot show —
    /// solved rate, the winning clock, and where on the level the population ended
    /// ([`evo::Telemetry`]). Reporting only; it reaches no score and no RNG.
    pub telemetry: evo::Telemetry,
}

/// A generation being animated: the packet, and one live episode per lane.
///
/// The runners own their genomes ([`EpisodeRunner`] takes one by value), which is what
/// lets this struct hold both — a borrowing runner would make the pool self-referential
/// and unrepresentable in a struct the window owns.
pub struct Lanes {
    packet: Packet,
    runners: Vec<EpisodeRunner>,
    /// One agent's animation per lane, advanced with the lane.
    anims: Vec<AgentAnim>,
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
        let anims = vec![AgentAnim::new(); packet.lanes.len()];
        Lanes {
            packet,
            runners,
            anims,
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

    /// One lane's animation: the run cycle, the landing pose and the dust.
    pub fn anim(&self, i: usize) -> &AgentAnim {
        &self.anims[i]
    }

    /// Advance by `dt` seconds of wall clock at `speed`.
    ///
    /// A lane's own episode is stepped, and the *only* thing that used to make this expensive was
    /// the route table: a lane's sim owns a copy of the level, and a copy of a cold route cache
    /// swept the whole move graph itself (24 ms on a 60x15 level) the first time it asked for a
    /// waypoint. The levels are shared now ([`sim::Level::routes`]), so a generation's eight lanes
    /// ask the question once and every one of them reads the answer: this loop is microseconds.
    pub fn advance(&mut self, dt: f32, speed: u32) {
        self.carry += ticks_for(dt, speed);
        let whole = self.carry.floor();
        self.carry -= whole;
        let ticks = whole.max(0.0) as u32;
        for (runner, anim) in self.runners.iter_mut().zip(self.anims.iter_mut()) {
            let mut stepped = 0u32;
            for _ in 0..ticks {
                if runner.is_finished() {
                    break;
                }
                runner.step();
                stepped += 1;
            }
            // Simulated time, not wall clock: a finished lane's dust settles instead of
            // drifting on for as long as the slowest lane in the generation keeps running.
            if stepped > 0 {
                anim.advance(runner.sim(), stepped as f32 * sim::DT);
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

/// The centre of the agent a follow camera should be on, in tile coordinates.
///
/// The **live lane furthest along**, counting a winner still standing on the flag: the
/// terminal watch mode's own rule, kept here rather than in the window for two reasons —
/// it can be tested without a window, and the two front-ends cannot disagree about which
/// agent "the leader" is. `None` when every lane is dead: there is nobody to follow, and
/// the camera should stay where the viewer put it rather than fly to a corpse.
pub fn leader_centre(lanes: &Lanes) -> Option<(f32, f32)> {
    let mut leader: Option<(f32, (f32, f32))> = None;
    for i in 0..lanes.len() {
        let sim = lanes.sim(i);
        let outcome = sim.outcome();
        let player = sim.player();
        let standing = !outcome.is_finished() || outcome == Outcome::Won;
        let centre = (player.x + player.w * 0.5, player.y + player.h * 0.5);
        if standing && leader.as_ref().is_none_or(|(x, _)| centre.0 > *x) {
            leader = Some((centre.0, centre));
        }
    }
    leader.map(|(_, centre)| centre)
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
    /// The name the run's artifacts wear ([`artifact_stem`]), carried here because the
    /// checkpoint a detour writes is named on this thread. The app's saved champion and this
    /// checkpoint have to agree with the shelf and the terminal, and a copy of a shipped level
    /// must not write over the shipped level's files.
    stem: String,
    cfg: GaConfig,
    eval: EvalConfig,
    /// How this run was asked for. A restart edits this rather than replacing it: a
    /// fresh population is not a fresh set of settings.
    plan: RunPlan,
    /// Where a curriculum step writes its checkpoint — `<stem>.population.json` in the run's
    /// output directory, which is the file `train-cli watch --resume` reads.
    out_dir: PathBuf,
    /// The checkpoint this run started from, as a note for the viewer.
    origin: Option<String>,
    /// The curriculum steps' notes, kept for the rest of the run: "this run changed
    /// shape" is not something the viewer should have to catch inside one generation.
    /// One line per step — a run that practised its way past three spots says so three
    /// times.
    curriculum: Option<String>,
    /// The run's standing best when the last derived practice step fired: the gate that
    /// allows one step per *stall spell* rather than one per run, and never two in the
    /// same spell.
    detour_best: f32,
    /// The practice detour currently running, if any. See [`Detour`].
    detour: Option<Detour>,
    traces: TraceLog,
    history: Vec<HistoryPoint>,
    deaths: Vec<(f32, f32, u32)>,
    best_k: usize,
    random_k: usize,
    /// The level's trick codex (WP16). A missing file is an empty codex; a file that belongs to
    /// another level is reported and set aside, because a stale journal must not stop a run.
    codex: evo::Codex,
    /// Where that codex is written. Resolved when the run starts, and again on a restart, so a
    /// run moved onto another level cannot append to the previous level's journal.
    codex_path: PathBuf,
    /// The `(generation, genome)` detection last ran on. A record is detected once rather than
    /// every generation, because finding its shortcut re-runs the episode.
    last_record: Option<(u32, u32)>,
}

/// One practice detour, running on a thread of its own.
///
/// **Why a thread.** A detour is forty generations on an easier level, plus the sweep that
/// derives that level, plus two file writes — measured at **1.73 s** on `scratch-tower` against
/// a 20-50 ms median generation. Run between two generations it was a visible freeze: the
/// window animates the generation it holds for 0.6-5 s, and a detour longer than the animation
/// leaves the viewer on a still frame with nothing arriving. The lookahead in `evolve_step`
/// covers one generation of slack; a detour is forty.
///
/// So the detour takes a clone of the trainer and works on it. Generations keep arriving from
/// the run's own population meanwhile, and the practised population is adopted the generation
/// after the thread finishes — through [`Trainer::adopt`], which exists for exactly this (a
/// curriculum step hands a population back into a run that owns the history and the ids).
///
/// The cost is that the detour's starting point is the stalled population as it was, not as it
/// would have been after forty more stuck generations — which is a *better* starting point for
/// the thing the detour is for.
struct Detour {
    handle: std::thread::JoinHandle<DetourDone>,
}

/// What a detour thread hands back.
struct DetourDone {
    /// The practised population, or `None` when there was nowhere to practise from.
    genomes: Option<Vec<Genome>>,
    /// The note the panel shows. One line per stall spell, exactly as the inline step's was.
    note: String,
}

/// A saved champion, loaded and validated as a **starting point** rather than as a claim
/// about the level.
///
/// Loaded on the window's thread before anything is dropped, so a file this build cannot
/// train on is a message in the status bar instead of a restart that silently became a fresh
/// population. The origin sentence travels with it: the worker cannot invent the "trained on
/// <other level>" note, and the window shows it once.
#[derive(Clone, Debug)]
pub struct ChampionSeed {
    genome: Genome,
    episode_seed: u64,
    origin: String,
}

/// Load `path` as a champion to seed a run on `level`.
///
/// [`ChampionFile::load_seed`] is the trainable half of the loader — format, versions,
/// sensors, activations and gene counts are all still refusals, the level hash alone is not.
/// That is the point: a champion saved before the level was edited is exactly the thing a
/// designer wants to continue from.
pub fn champion_seed(path: &Path, level: &Level) -> Result<ChampionSeed, String> {
    let champion = ChampionFile::load_seed(path)?;
    let genome = champion.to_genome()?;
    let origin = match champion.level_note(level) {
        Some(note) => format!("seeded from {}; {note}", champion.summary()),
        None => format!("seeded from {}", champion.summary()),
    };
    Ok(ChampionSeed {
        genome,
        episode_seed: champion.metadata.episode_seed,
        origin,
    })
}

/// Turn one loaded champion into the population a run starts from.
///
/// The champion file's own episode seed is replayed rather than a new one invented, so the
/// record this run starts with is the episode the file claims — which is what lets a seeded
/// run keep its winner and arm the mastery league from generation 0.
fn train_from_champion(
    cfg: &GaConfig,
    level: &Level,
    seed: &ChampionSeed,
) -> Result<Trainer, String> {
    let mut trainer = Trainer::from_champion(cfg.clone(), seed.genome.clone())?;
    trainer.seed_record(level, seed.genome.clone(), seed.episode_seed)?;
    Ok(trainer)
}

impl Generator {
    /// A fresh population, or — when the plan names a checkpoint — the population that
    /// checkpoint holds.
    ///
    /// A resumed run continues the ids and the generation it was handed, for the reason
    /// the terminal's `--resume` does: the ids are part of every episode seed, so
    /// renumbering them would silently train on different episodes than the ones the
    /// population was stalled on. A checkpoint's level hash is a note rather than a gate
    /// — resuming on a different level is exactly what a curriculum step is, and
    /// [`evo::PopulationFile::level_note`] is what says so out loud.
    pub fn new(
        level: Level,
        cfg: GaConfig,
        best_k: usize,
        random_k: usize,
        plan: RunPlan,
        out_dir: PathBuf,
        stem: impl Into<String>,
    ) -> Result<Generator, String> {
        let mut cfg = cfg.sanitized(None);
        let recommended = evo::GaConfig::recommended_trials_for(&level);
        if cfg.trials <= 1 && recommended > 1 {
            cfg.trials = recommended;
        }
        let stem = stem.into();
        let (trainer, origin) = match plan.resume.as_deref() {
            Some(path) => {
                let checkpoint = PopulationFile::load(path)?;
                let mut origin = format!("resumed {}", checkpoint.summary());
                if let Some(note) = checkpoint.level_note(&level) {
                    origin.push_str(&format!("; {note}"));
                }
                let mut trainer =
                    Trainer::resume(cfg.clone(), checkpoint.to_genomes()?, checkpoint.generation)?;
                // The cohort may not contain the genome that solved the level; the checkpoint's
                // record is what turns a resumed stall back into a run that can breed its winner.
                // Older checkpoints predate the field, and the likely survivor is the champion
                // beside the level: seed from that rather than resume a solved run with no winner.
                let record = match checkpoint.best_genome()? {
                    Some(record) => Some(record),
                    None => ChampionFile::load_for_level(
                        out_dir.join(format!("{stem}.json")),
                        &level,
                    )
                    .ok()
                    .and_then(|champion| champion.to_genome().ok()),
                };
                if let Some(record) = record {
                    trainer.seed_record_from_checkpoint(&level, record)?;
                }
                (trainer, Some(origin))
            }
            // A champion is the other artifact a run leaves behind. It is one genome rather
            // than a cohort, so [`Trainer::from_champion`] reheats a population around it —
            // and the window resolves the file before the first generation, not on this
            // thread, so a bad file is a startup message.
            None => match plan.seed_from_champion.as_deref() {
                Some(path) => {
                    let seed = champion_seed(path, &level)?;
                    let trainer = train_from_champion(&cfg, &level, &seed)?;
                    (trainer, Some(seed.origin))
                }
                // A fresh run warms itself from the level's certified timed plan when it has one.
                // See [`warm_seed_from_plan`] and [`RunPlan::warm_from_plan`].
                None if plan.warm_from_plan => {
                    let (pop, note) = warm_seed_from_plan(&level, &cfg);
                    match pop {
                        Some(pop) => (Trainer::resume(cfg.clone(), pop, 0)?, Some(note)),
                        None => (Trainer::new(cfg.clone()), Some(note)),
                    }
                }
                None => (Trainer::new(cfg.clone()), None),
            },
        };
        let codex_path = evo::Codex::path_for(&out_dir, &stem);
        let codex = match evo::Codex::load(&level, &codex_path) {
            Ok(codex) => codex,
            Err(err) => {
                eprintln!("codex: {err}; starting a fresh one");
                evo::Codex::empty(&level)
            }
        };
        Ok(Generator {
            eval: cfg.eval_config(),
            trainer,
            level,
            stem,
            cfg,
            plan,
            out_dir,
            origin,
            curriculum: None,
            detour_best: f32::NEG_INFINITY,
            detour: None,
            traces: TraceLog::new(),
            history: Vec::new(),
            deaths: Vec::new(),
            best_k,
            random_k,
            codex,
            codex_path,
            last_record: None,
        })
    }

    /// How this run was asked for, including the edits a restart makes to it.
    ///
    /// Only the tests read this: the window asks the [`Lab`] what a run is doing, never
    /// the generator, which belongs to the worker thread.
    #[cfg(test)]
    pub fn plan(&self) -> &RunPlan {
        &self.plan
    }

    /// The checkpoint this run was started from, as a note for the viewer.
    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    /// The level a generation is being evaluated on. The window reads the run's level
    /// from [`Lab`] instead; this exists for the tests that check a restart moved it.
    #[cfg(test)]
    pub fn level(&self) -> &Level {
        &self.level
    }

    /// The run's best-ever record, for the tests that check where a population started.
    #[cfg(test)]
    pub fn best(&self) -> Option<&BestEver> {
        self.trainer.best_ever()
    }

    /// Is a practice detour running on its own thread right now?
    #[cfg(test)]
    pub fn detour_in_flight(&self) -> bool {
        self.detour.is_some()
    }

    /// Start over — on the same level after an edit, or on a different one entirely —
    /// with a fresh population, a fresh chart and an empty record.
    ///
    /// "Fresh" is either random (with the level's route as a warm start) or, when the viewer
    /// asked to continue a saved champion, a reheated shell around that genome — which is
    /// still a new run, whose record is the champion's own recorded episode.
    ///
    /// Everything else the run accumulated is dropped on purpose: the chart is a claim about
    /// *this* population on *this* level, and continuing it across a level change would
    /// draw a cliff that no agent ever experienced.
    pub fn start_over(
        &mut self,
        level: Level,
        seed: u64,
        mutation_rate: f32,
        stem: impl Into<String>,
        champion: Option<ChampionSeed>,
    ) {
        self.stem = stem.into();
        let mut cfg = self.cfg.clone();
        cfg.master_seed = seed;
        cfg.mutation_rate = mutation_rate;
        // A restart is a new run on the level *as it stands now*, so the level gets to ask for a
        // different brain again (WP2): an edit that added a deck or a turret must not inherit the
        // memoryless shape of the run it replaced. The scan only raises memory and never clears an
        // operator's choice, so this is safe to re-run on the same level.
        let _ = evo::apply_auto_shape(&mut cfg, std::slice::from_ref(&level), false, false);
        cfg.trials = evo::GaConfig::recommended_trials_for(&level);
        self.eval = cfg.eval_config();
        // A restart is a fresh population on the level as it stands *now*, which is exactly the
        // case the route warm start is for: the edit may have moved the flag, the pit or the whole
        // level, and a walk of the current one is the best thing a new population can be handed.
        // A champion is the third answer and the more specific one: when the viewer asked to
        // continue one, its reheated shell *is* the fresh population and the route walk would be
        // a second, weaker idea layered on top.
        let (trainer, origin) = match champion {
            Some(champion) => match train_from_champion(&cfg, &level, &champion) {
                Ok(trainer) => (trainer, Some(champion.origin)),
                Err(err) => (
                    Trainer::new(cfg.clone()),
                    Some(format!(
                        "could not seed from the champion ({err}); a fresh population"
                    )),
                ),
            },
            None if self.plan.warm_from_plan => {
                let (pop, note) = warm_seed_from_plan(&level, &cfg);
                match pop {
                    Some(pop) => (
                        Trainer::resume(cfg.clone(), pop, 0)
                            .unwrap_or_else(|_| Trainer::new(cfg.clone())),
                        Some(note),
                    ),
                    None => (Trainer::new(cfg.clone()), Some(note)),
                }
            }
            None => (Trainer::new(cfg.clone()), None),
        };
        self.trainer = trainer;
        self.cfg = cfg;
        self.traces = TraceLog::new();
        self.history.clear();
        self.deaths.clear();
        self.level = level;
        // A restart means a *fresh population*, so a checkpoint this run started from is
        // dropped: reproducing it is the opposite of what Restart means. The fallback
        // level stays, because it is a setting of the run rather than of its population,
        // and a run restarted onto a hard level still wants somewhere to step back to.
        self.plan.resume = None;
        // A champion is a way *into* a run, not a setting of it: once the population exists,
        // Restart's own meaning — a fresh population — takes over again.
        self.plan.seed_from_champion = None;
        self.curriculum = None;
        self.origin = origin;
        // The codex follows the *level*, not the population: a restart keeps what the level has
        // already produced (it is a journal of the geometry), and a restart onto another level
        // opens that level's own journal instead of appending to the old one.
        self.codex_path = evo::Codex::path_for(&self.out_dir, &self.stem);
        self.codex = match evo::Codex::load(&self.level, &self.codex_path) {
            Ok(codex) => codex,
            Err(err) => {
                eprintln!("codex: {err}; starting a fresh one");
                evo::Codex::empty(&self.level)
            }
        };
        self.last_record = None;
        // A detour belongs to the run that was stuck, not to the fresh population. It is
        // detached rather than waited for on purpose: a restart that blocked a second and a
        // half on a thread nobody will read would be the freeze this thread exists to remove.
        // Its only side effects are the checkpoint and practice level it writes, which a
        // restart is about to overwrite anyway.
        self.detour = None;
    }

    /// The viewer's live mutation-rate slider: applied from the next generation on, so a
    /// rate change shows up in the very next offspring (the terminal's behaviour).
    pub fn set_mutation_rate(&mut self, rate: f32) {
        self.cfg.mutation_rate = rate;
    }

    /// Evaluate one generation and package everything the window needs to draw it.
    /// Append one line to the run's curriculum note. Every step keeps its line: a run that
    /// practised its way past three spots says so three times, not once.
    fn push_note(&mut self, note: String) {
        self.curriculum = Some(match self.curriculum.take() {
            Some(prev) => format!("{prev}\n{note}"),
            None => note,
        });
    }

    /// Start the stall's practice detour on its own thread. See [`Detour`].
    ///
    /// The population is snapshotted, and the arm closes (`detour_best`) before the thread
    /// starts, so a run that stalls again while the detour is in flight neither starts a second
    /// one nor re-reports the same spell.
    fn begin_detour(&mut self, label: String, best: f32) {
        self.detour_best = best;
        let mut trainer = self.trainer.clone();
        let main = self.level.clone();
        let authored = self.plan.curriculum.clone();
        let path = self.out_dir.join(format!("{}.population.json", self.stem));
        let rate = self.cfg.mutation_rate;
        let main_name = self.level.name.clone();
        let spawned = thread::Builder::new()
            .name("practice".into())
            .spawn(move || {
                let started = Instant::now();
                // The authored easier level is the run's first choice (§12 M5). Without one
                // the run practises *where it is stuck* instead (finding 50): its own level
                // from the furthest stand its best genome has made. Both are decided here
                // rather than on the worker: deriving the level is a sweep, and a sweep is
                // the cost this thread exists to keep off the packet path.
                let plan = match authored {
                    Some(c) => Some((c.level, c.generations, false)),
                    None => match practice_level_for_stall(&trainer, &main) {
                        Ok(level) => Some((level, DEFAULT_CURRICULUM_GENERATIONS, true)),
                        Err(reason) => {
                            return DetourDone {
                                genomes: None,
                                note: format!(
                                    "curriculum: {reason}; the run stays where it is ({label})"
                                ),
                            }
                        }
                    },
                };
                let Some((easier, generations, derived)) = plan else {
                    return DetourDone {
                        genomes: None,
                        note: format!("curriculum: nowhere to practise ({label})"),
                    };
                };
                // The practice detour keeps the main level in its judgement, so what it
                // learns comes back able to reach the spot it left.
                let step = if derived {
                    practice_step(
                        &mut trainer,
                        &main,
                        &easier,
                        generations,
                        &path,
                        rate,
                        &label,
                    )
                } else {
                    curriculum_step(
                        &mut trainer,
                        &main,
                        &easier,
                        generations,
                        &path,
                        rate,
                        &label,
                    )
                };
                let done = match step {
                    Ok(step) => {
                        let mut note = step.note(&main_name);
                        if derived {
                            // The detour's landscape is written beside the checkpoint so it
                            // can be opened and played; a failed write is part of the note,
                            // not a lost detour.
                            if let Err(err) = save_practice_level(&easier, &path) {
                                note.push_str(&format!(" (practice level not written: {err})"));
                            }
                        }
                        DetourDone {
                            genomes: Some(trainer.genomes),
                            note,
                        }
                    }
                    // A step that cannot run is not a reason to lose the run: the stall
                    // line still says the search has stopped, which is the honest thing to
                    // leave on screen.
                    Err(err) => DetourDone {
                        genomes: None,
                        note: format!("curriculum step failed: {err}"),
                    },
                };
                if profiling() {
                    eprintln!(
                        "lab detour {:?} off the packet path ({})",
                        started.elapsed(),
                        label
                    );
                }
                done
            });
        match spawned {
            Ok(handle) => self.detour = Some(Detour { handle }),
            Err(err) => self.push_note(format!(
                "curriculum step failed: cannot start the practice thread: {err}"
            )),
        }
    }

    /// Adopt a finished detour. Never blocks: a running one is left alone.
    fn poll_detour(&mut self) {
        self.settle_detour(false);
    }

    /// Adopt the detour, waiting for it if `wait` — the end of a capped run, where the note
    /// is the last thing the run has to say and no generation is waiting behind this one.
    fn settle_detour(&mut self, wait: bool) {
        let Some(detour) = &self.detour else {
            return;
        };
        if !wait && !detour.handle.is_finished() {
            return;
        }
        let detour = self.detour.take().expect("checked just above");
        match detour.handle.join() {
            Ok(done) => {
                let DetourDone { genomes, note } = done;
                self.push_note(note);
                if let Some(genomes) = genomes {
                    if let Err(err) = self.trainer.adopt(genomes) {
                        self.push_note(format!("curriculum step failed: {err}"));
                    }
                }
            }
            Err(_) => {
                self.push_note("curriculum step failed: the practice thread panicked".to_string())
            }
        }
    }

    pub fn next_packet(&mut self) -> Packet {
        let t_packet = Instant::now();
        // A detour that finished since the last generation is adopted *here*, on the
        // worker, so the population the next generation breeds from is the practised one.
        self.poll_detour();
        // The population snapshot is taken before breeding, so these are the exact
        // genomes (with the exact ids) the report describes.
        self.trainer.config.mutation_rate = self.cfg.mutation_rate;
        let t = Instant::now();
        let (report, population) = self.trainer.step_verbose(&self.level);
        if profiling() {
            eprintln!("lab step_verbose {:?}", t.elapsed());
        }
        let t = Instant::now();
        let showcase = self
            .trainer
            .showcase(&report, &population, self.best_k, self.random_k);
        if profiling() {
            eprintln!("lab showcase {:?}", t.elapsed());
        }

        // **Discovery detection (WP16).** A record that moved is asked what it found, once:
        // finding its shortcut re-runs the episode, and a generation that did not move the record
        // cannot have found anything the previous one did not have. A new find is written to the
        // codex here, on the worker, so a watched run never waits on the journal.
        let record = self
            .trainer
            .best_ever()
            .map(|best| (best.generation, best.genome.id));
        let mut discovery = None;
        if record != self.last_record {
            self.last_record = record;
            if let Some(best) = self.trainer.best_ever() {
                // Two phases: detect without evidence first, because capturing a replay re-runs the
                // episode. Only a find the journal has never seen pays that cost, and it pays it for
                // the evidence the codex is meant to keep.
                let found = evo::detect_discoveries(
                    &self.level,
                    &best.genome,
                    &best.result,
                    &self.eval,
                    best.generation,
                    false,
                );
                let fresh: Vec<String> = found
                    .iter()
                    .filter(|d| !self.codex.contains(&d.signature))
                    .map(|d| d.signature.clone())
                    .collect();
                let found = if fresh.is_empty() {
                    Vec::new()
                } else {
                    evo::detect_discoveries(
                        &self.level,
                        &best.genome,
                        &best.result,
                        &self.eval,
                        best.generation,
                        true,
                    )
                    .into_iter()
                    .filter(|d| fresh.contains(&d.signature))
                    .collect()
                };
                let new = self.codex.record_all(found);
                if !new.is_empty() {
                    if let Err(err) = self.codex.save(&self.codex_path) {
                        eprintln!("codex: {err}");
                    }
                    discovery = self.codex.entries().last().cloned();
                }
            }
        }

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
            .collect(); // The whole population, not only the eight lanes: the replay ghosts all of it, and
                        // the pairing that makes a ghost faithful is (genome, the seed it was scored on),
                        // which is exactly what the report's index-aligned results hold.
                        // By value, because `population` is this function's own copy and nothing below reads it
                        // again: cloning every genome a second time to build this vector cost a full population's
                        // worth of allocation per generation (~1.2 MB at pop 200) for a vector that is then the
                        // only owner of the same data. The `report.results` are borrowed-into by clone instead,
                        // because the report itself is still needed for the history and the deaths below.
        let t = Instant::now();
        let agents: Vec<Agent> = population
            .into_iter()
            .zip(report.results.iter())
            .map(|(genome, result)| Agent {
                genome,
                result: result.clone(),
            })
            .collect();
        if profiling() {
            eprintln!("lab agents {:?}", t.elapsed());
        }

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

        // The stall the panel is about to report is the trigger for the run's one
        // curriculum step (§12 M5). It reads the *same* stall rule the summary line
        // shows, so a run cannot step back to an easier level on a stall the viewer is
        // not being told about, and it happens before the packet is packaged so the note
        // and the history it describes arrive together.
        // The authored easier level steps back once per run (the plan's rule). The
        // derived practice re-arms on every *new* stall spell — a practice run has to
        // work through a spot, keep the gain, and meet the next spot — and never twice
        // within one, so a detour that learns nothing cannot loop (finding 50).
        let armed = self.detour.is_none()
            && match &self.plan.curriculum {
                Some(_) => self.curriculum.is_none(),
                None => report.max_fitness > self.detour_best,
            };
        if armed {
            if let Some(stall) = fitness_stall(&self.history) {
                self.begin_detour(stall.label(), report.max_fitness);
            }
        }

        // A capped run must not lose the detour it started: the note, the checkpoint and the
        // practice level are the run's last artifacts, and an abandoned thread would leave the
        // window with a curve that has no explanation of where it went. Waiting is bounded by
        // one detour and happens at the end of the run, which is the one place a stall costs
        // nobody a frame.
        let finished = self.plan.cap.is_some_and(|t| run >= t);
        if finished {
            self.settle_detour(true);
        }

        if profiling() {
            eprintln!(
                "lab packet total {:?} (generation {})",
                t_packet.elapsed(),
                report.generation
            );
        }
        Packet {
            generation: report.generation,
            max_fitness: report.max_fitness,
            avg_fitness: report.avg_fitness,
            min_fitness: report.min_fitness,
            wins: report.wins,
            solved: report.solved,
            genius: report.genius,
            pop_size: self.cfg.pop_size,
            // Read off the generation the trainer just produced, not recomputed here: the
            // step the children were bred with, the run's own staleness counter, and what
            // that counter bought.
            mutation_sigma: report.mutation_sigma,
            stagnation: self.trainer.stagnation(),
            immigrants: report.immigrants,
            archive_children: report.archive_children,
            repairs: report.repairs,
            archive_bits: report.archive_bits,
            archive_time_bits: report.archive_time_bits,
            fastest: report.fastest,
            fastest_agent: self.trainer.fastest_agent().cloned(),
            lanes,
            agents,
            history: self.history.clone(),
            deaths: self.deaths.clone(),
            traces: self.traces.entries().to_vec(),
            best: self.trainer.best_ever().cloned(),
            target: self.plan.cap,
            level_name: self.level.name.clone(),
            level_hash: self.level.scene_hash_string(),
            curriculum: self.curriculum.clone(),
            finished,
            discovery,
            codex_len: self.codex.len(),
            telemetry: report.telemetry.clone(),
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
    // rounded to the nearest sample. It is skipped only when it *is* the previous sample,
    // frame and all: comparing positions alone dropped the true last frame whenever the body
    // stopped moving between the final sample and the end, which is exactly what a body at
    // the flag and a grave both are.
    let p = runner.sim().player();
    let end = TracePoint {
        frame: runner.sim().stats().frames,
        x: p.x,
        y: p.y,
    };
    let already_recorded = points
        .last()
        .is_some_and(|q| q.frame == end.frame && q.x == end.x && q.y == end.y);
    if !already_recorded {
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

/// A restart the window asked for, handed to the worker.
///
/// One struct rather than a tuple because the champion is the third kind of starting point
/// besides "fresh" and "the checkpoint in the plan", and a tuple of four positional fields
/// at three call sites is how two of them come to disagree about which is which.
struct Restart {
    level: Level,
    seed: u64,
    stem: String,
    /// A champion to seed the new population from, already loaded and validated.
    champion: Option<ChampionSeed>,
}

#[derive(Default)]
struct State {
    /// A finished generation the window has not taken yet.
    queued: Option<Packet>,
    /// The window has finished animating and is ready for the next generation. This is
    /// the whole reason the run cannot train ahead of the viewer.
    want_next: bool,
    quit: bool,
    /// A restart request: the level as it stands, the seed, the stem its artifacts wear, and
    /// the saved champion to seed the new population from when the viewer asked to continue
    /// one rather than start from nothing.
    restart: Option<Restart>,
    mutation_rate: Option<f32>,
    finished: bool,
    /// A worker thread died of a panic rather than of `quit`.
    ///
    /// Set by [`WorkerGuard`] as the stack unwinds, because a panic that is only joined-and-
    /// dropped is indistinguishable from a run that finished (AUDIT.md B4).
    panicked: bool,
}

/// Reports a worker's death to the window as it unwinds.
///
/// **A panic used to look like success.** The scoring runs outside the lock, but the thread's
/// handle is thrown away at the join (`let _ = handle.join()`), and [`Lanes::is_finished`] then
/// answers `true` on its own: a run whose worker died mid-generation was drawn as a run that had
/// *finished*, holding on the last generation it managed with nothing on screen to say why. A
/// `Drop` guard is what survives an unwind — it is the only hook a panicking thread is guaranteed
/// to run — so it costs one field and one `if`.
///
/// The two ways out of the loop that are *not* a panic both leave a mark: `quit` is set by
/// [`Lab::stop`], and `finished` is set by the worker itself when the run reaches its generation
/// cap. Anything else on the way out is a death.
struct WorkerGuard {
    hub: Arc<Hub>,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        // `if let Ok` rather than `unwrap`: the only code that holds this lock is the handshake
        // around the generation, so a *poisoned* lock would mean the panic happened somewhere
        // this guard cannot help with — and panicking again inside a drop during a panic turns a
        // bad run into a dead process.
        if let Ok(mut state) = self.hub.state.lock() {
            if !state.quit && !state.finished {
                state.panicked = true;
                self.hub.cv.notify_all();
            }
        }
    }
}

/// How many cores the trainer may use: every one but the window's.
///
/// The worker and the window are on one machine, and a generation is an embarrassingly
/// parallel job that rayon would happily spread over **every** core — including the one the
/// frame being drawn is running on. That is why a run stutters exactly while it is working:
/// the window is not slow, it is queued behind its own trainer. Holding one core back leaves
/// the frame being drawn somewhere to be drawn, and the price is one Nth of the training
/// throughput on an N-core machine — which is the right trade for a view whose entire purpose
/// is to be watched while it trains. A one-core machine still gets one thread: a run that will
/// not start is worse than a run that stutters.
fn training_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .saturating_sub(1)
        .max(1)
}

/// The worker loop: one generation per request, parked in between.
fn work(hub: Arc<Hub>, mut generator: Generator, initial_rate: f32, pool: rayon::ThreadPool) {
    // Armed before the first generation and disarmed by neither branch: the guard reports a
    // *panic*, and the loop's own exits already say what they are.
    let _guard = WorkerGuard { hub: hub.clone() };
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

        if let Some(restart) = restart {
            generator.start_over(
                restart.level,
                restart.seed,
                rate.unwrap_or(initial_rate),
                restart.stem,
                restart.champion,
            );
        } else if let Some(rate) = rate {
            generator.set_mutation_rate(rate);
        }

        // `install` is what makes the whole generation — evaluation, the trace episode and any
        // curriculum work — run on this pool rather than rayon's global one. Without it the
        // sizing above would be decoration: `evaluate_suite` is a `par_iter`, and a `par_iter`
        // outside a pool uses every core on the machine.
        // **One generation per request, and no dial to multiply it.** The panel used to carry a
        // 1x/4x/16x/64x "thinking" ladder: a request computed that many generations and queued
        // only the last. It is gone because the trade is the wrong way round on the machine this
        // ships on. A batched request holds the evaluation pool for the whole batch and yields no
        // frame, so the window stutters for as long as the batch runs; and the generations it
        // skips are still real trainer steps, none of which was ever shown, so the run *looks*
        // stalled while it is working hardest. One generation per request is also the cadence
        // every measurement in FINDINGS.md was taken at.
        let packet = pool.install(|| generator.next_packet());
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

/// A scored generation on `level`, for the tests that need a packet to work on.
///
/// Test-only, and shared across this crate's test modules on purpose: the replay's roster
/// and the canvas that draws it both want a packet that came out of the real generator,
/// and two fixtures would be two chances for them to drift apart.
#[cfg(test)]
pub(crate) fn scored_packet(level: &Level, tag: &str) -> Packet {
    let mut generator = Generator::new(
        level.clone(),
        GaConfig {
            pop_size: 8,
            elite: 2,
            ..GaConfig::default()
        },
        DEFAULT_BEST_LANES,
        DEFAULT_RANDOM_LANES,
        RunPlan::default(),
        std::env::temp_dir().join(format!("evo-scored-packet-{tag}")),
        "scored-packet",
    )
    .expect("the generator starts");
    generator.next_packet()
}

/// The stem a run names its files with.
///
/// **The document's file stem first, its display name only as a fallback.** A level's name is what
/// the shelf card says and what a new save is called, but the file it lives in is what the rest of
/// the workspace is keyed on: the shelf looks for a champion beside the level under the level
/// file's stem, the title screen tries that stem before the name, and `train-cli` names its outputs
/// after it. Deriving the champion from the *name* instead was a real loss of data rather than an
/// inconsistency: a custom copy of a shipped level keeps the shipped level's name
/// ([`crate::editor::protect_shipped_path`] writes `custom-<stem>.json` for exactly that case), so
/// a run on the copy wrote `champions/<shipped stem>.json` and replaced the pack's own champion
/// with a policy trained on somebody else's geometry.
///
/// A document that has never been saved has no file to be keyed on, and its slugged name is the
/// name its first save will wear, so the two agree from the moment there is a file at all.
pub fn artifact_stem(level: &Level, path: Option<&Path>) -> String {
    path.and_then(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| slug(&level.name))
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
    /// The name this run's artifacts wear, from [`artifact_stem`]: the document's file stem when
    /// it has one, so a custom copy of a shipped level cannot overwrite the pack's champion.
    stem: String,
    pop_size: usize,
    /// The window has already asked for the generation it is waiting for, so it does not
    /// ask again every frame.
    asked: bool,
    /// The run's configuration, mirrored here for the champion file's metadata.
    config: GaConfig,
    /// Where this run's own files go: the champion the viewer saves, and the checkpoint a
    /// curriculum step writes. One directory for the run, so a run's artifacts are not
    /// scattered by whoever wrote them.
    out_dir: PathBuf,
    /// The checkpoint this run was started from, when it was resumed rather than started
    /// fresh (`--resume`). The window shows it once, because a number on screen that came
    /// from another level deserves saying so.
    origin: Option<String>,
}

impl Lab {
    /// Start a run on `level`. `target` caps the generations, for a scripted or
    /// screenshot run; `None` runs until the window stops it.
    /// `stem` is what the run's artifacts are called — see [`artifact_stem`]. It is passed in
    /// rather than derived here because it comes from the *document's path*, which is the app's
    /// knowledge and not the level's.
    pub fn start(
        level: &Level,
        cfg: GaConfig,
        best_k: usize,
        random_k: usize,
        plan: RunPlan,
        out_dir: PathBuf,
        stem: impl Into<String>,
    ) -> Result<Lab, String> {
        let mut cfg = cfg.sanitized(None);
        // The window starts from `GaConfig::default()`, so the level's own mechanics are the
        // only thing that can tell it a timed crossing needs memory. An explicit operator
        // setting does not exist in this front-end; if one is added, it is the `locked` half of
        // `apply_auto_shape` that keeps it.
        let _auto = evo::apply_auto_shape(&mut cfg, std::slice::from_ref(level), false, false);
        let recommended = evo::GaConfig::recommended_trials_for(level);
        if cfg.trials <= 1 && recommended > 1 {
            cfg.trials = recommended;
        }
        // A checkpoint that cannot be loaded is a startup error, not a run that quietly
        // trains something else: it is the same refusal the terminal's `--resume` makes.
        let stem = stem.into();
        let generator = Generator::new(
            level.clone(),
            cfg.clone(),
            best_k,
            random_k,
            plan,
            out_dir.clone(),
            stem.clone(),
        )?;
    let origin = generator.origin().map(str::to_string);
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
        // The trainer's pool, built once per run: the same `threads - 1` sizing for the whole
        // session, so a generation's cost and the window's smoothness are the same every time.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(training_threads())
            .thread_name(|i| format!("evaluate-{i}"))
            .build()
            .map_err(|e| format!("cannot start the evaluation pool: {e}"))?;
        let worker = {
            let hub = Arc::clone(&hub);
            thread::Builder::new()
                .name("evolution".into())
                .spawn(move || work(hub, generator, rate, pool))
                .map_err(|e| format!("cannot start the evolution thread: {e}"))?
        };
        Ok(Lab {
            hub,
            worker: Some(worker),
            eval: cfg.eval_config(),
            level: level.clone(),
            level_hash: level.scene_hash_string(),
            level_name: level.name.clone(),
            stem,
            pop_size: cfg.pop_size,
            asked: true,
            config: cfg,
            out_dir,
            origin,
        })
    }

    /// Where this run's files go.
    pub fn out_dir(&self) -> &Path {
        &self.out_dir
    }

    /// The stem this run names its files with (see [`artifact_stem`]).
    ///
    /// Set at start and on every restart, so a run moved onto another level names its artifacts
    /// after the level it is actually training on rather than the one it opened with.
    pub fn stem(&self) -> String {
        self.stem.clone()
    }

    /// The checkpoint this run started from, when it was resumed rather than started
    /// fresh.
    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
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

    /// The eval config the run's generations were scored with: the cadence a ghost
    /// of the starring agent has to replay under for its episode to be the recorded one
    /// rather than a fresh sample of the same genome.
    pub fn eval_config(&self) -> EvalConfig {
        self.eval
    }

    /// Build a replay of this run's quickest win — or, on a run that has never won, its
    /// best-scoring attempt ([`Replay`]) — with the generation's population as its ghosts.
    ///
    /// It lives here rather than in the window because the two things a roster needs —
    /// the level and the eval config the generation was scored with — are the run's, and
    /// the same pair [`Lanes::new`] is handed for the same reason. `None` when nothing has
    /// been scored yet: a replay of an unscored run would be a mode with nothing in it.
    pub fn replay(&self, packet: &Packet) -> Option<Replay> {
        Replay::new(packet, &self.level, &self.eval)
    }

    /// Was this run started on the level as it stands now?
    ///
    /// The hash covers the grid, the spawn and every entity box — walkers, chasers, flyers,
    /// turrets and movers ([`Level::scene_hash`]) — so this answers "the same level" rather
    /// than "the same file name": an edit that matters to the physics is an edit that makes
    /// the run's numbers a claim about a level nobody can see any more. A chaser placed after
    /// the run started is exactly such an edit, and leaving it out of the hash is what let the
    /// evolve view keep drawing a copy of the level the creature had been added to.
    pub fn trained_on(&self, level: &Level) -> bool {
        self.level_hash == level.scene_hash_string()
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

    /// Ask for the next generation — one *ahead* of the one on screen.
    ///
    /// The window calls this the moment it takes a generation, so the worker computes the
    /// successor while the viewer watches what is already there, and again on the frame a
    /// generation finishes with nothing queued behind it. That second call is what makes
    /// this the run's rate limit: a request while one is in flight is refused, so the
    /// worker never runs more than one generation past the screen, and a paused or
    /// scrubbing window parks it as soon as the generation in flight lands.
    ///
    /// Asking early rather than late is what keeps a *long* generation off the screen: the
    /// forty-generation practice detour a stall triggers is one packet of ~1.7 s at pop 200
    /// (`app::evolve_step`'s lookahead carries the measurement), and under the one-ahead
    /// rule that whole second of work happens while the previous generation plays.
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
    pub fn restart(&mut self, level: &Level, seed: u64, stem: impl Into<String>) {
        self.restart_with(level, seed, stem, None);
    }

    /// Start a new run whose population is seeded from a saved champion.
    ///
    /// The app's half of `train-cli train --from-champion`. The champion is loaded by
    /// [`champion_seed`] before this is called; the shape is checked here, on the window's
    /// thread, because the alternative is a worker that cannot build the population and
    /// falls back to a fresh one — a run that silently is not the one the viewer asked for,
    /// under an origin line claiming it is. On `Err` nothing about the current run has moved.
    pub fn restart_from_champion(
        &mut self,
        level: &Level,
        seed: u64,
        stem: impl Into<String>,
        champion: ChampionSeed,
    ) -> Result<(), String> {
        // The same shape rule `start_over` will apply on the worker: auto-detected memory is
        // part of the run's network, so a champion built without it is not this run's shape.
        let mut cfg = self.config.clone();
        evo::apply_auto_shape(&mut cfg, std::slice::from_ref(level), false, false);
        Trainer::from_champion(cfg, champion.genome.clone())?;
        self.origin = Some(champion.origin.clone());
        self.restart_with(level, seed, stem, Some(champion));
        Ok(())
    }

    /// The file `Save champion` writes for this run, and the file `Continue champion` reads.
    ///
    /// One path for both, so the control that offers to continue a champion can only ever
    /// offer the champion this level's runs actually save — the same argument as
    /// [`lab::save_champion`]'s naming rule.
    pub fn champion_path(&self) -> PathBuf {
        self.out_dir.join(format!("{}.json", self.stem))
    }

    /// Replace the population with a new run's on `level`, from `champion` when one was named.
    fn restart_with(
        &mut self,
        level: &Level,
        seed: u64,
        stem: impl Into<String>,
        champion: Option<ChampionSeed>,
    ) {
        self.level = level.clone();
        self.level_hash = level.scene_hash_string();
        self.level_name = level.name.clone();
        self.stem = stem.into();
        self.config.master_seed = seed;
        if champion.is_none() {
            self.origin = None;
        }
        let mut state = self.hub.state.lock().unwrap();
        state.restart = Some(Restart {
            level: level.clone(),
            seed,
            stem: self.stem.clone(),
            champion,
        });
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

    /// Whether the run's worker died of a panic.
    ///
    /// What the window needs to tell a *dead* run from a finished one: without this, a worker
    /// that panicked mid-generation was drawn as a run at its cap, frozen on the last generation
    /// it managed (AUDIT.md B4).
    pub fn panicked(&self) -> bool {
        self.hub.state.lock().map(|s| s.panicked).unwrap_or(false)
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

    fn hub() -> Arc<Hub> {
        Arc::new(Hub {
            state: Mutex::new(State::default()),
            cv: Condvar::new(),
        })
    }

    /// A worker that vanishes without finishing is reported as dead, and the two honest ways out
    /// of the loop are not.
    ///
    /// The bug this pins: the worker's panic was thrown away at the join, `is_finished` answered
    /// `true` on its own, and the window drew a run whose thread had died as a run at its cap —
    /// frozen on the last generation it managed, with nothing on screen to say why (AUDIT.md B4).
    #[test]
    fn a_worker_that_leaves_without_finishing_is_reported_as_dead() {
        // Dropping the guard with neither mark set is exactly what a panic looks like from
        // outside the thread: the stack unwinds through it.
        let vanished = hub();
        {
            let _guard = WorkerGuard {
                hub: vanished.clone(),
            };
        }
        assert!(
            vanished.state.lock().unwrap().panicked,
            "a worker that vanished must not read as a finished run"
        );

        // The window asked it to stop: a shutdown is not a death.
        let stopped = hub();
        {
            stopped.state.lock().unwrap().quit = true;
            let _guard = WorkerGuard {
                hub: stopped.clone(),
            };
        }
        assert!(!stopped.state.lock().unwrap().panicked);

        // The run reached its generation cap, which the worker marks before it leaves.
        let capped = hub();
        {
            capped.state.lock().unwrap().finished = true;
            let _guard = WorkerGuard {
                hub: capped.clone(),
            };
        }
        assert!(!capped.state.lock().unwrap().panicked);
    }

    /// A fresh run is not reported as dead, and a running one is not either.
    ///
    /// The banner that says "training stopped" hangs off this one predicate, so it has to be
    /// false in the ordinary case — an alarm that fires on a healthy run is worse than no alarm.
    #[test]
    fn a_live_run_is_not_reported_as_dead() {
        let level = shipped("l1-flat-run");
        let mut lab = Lab::start(
            &level,
            tiny_config(),
            2,
            2,
            RunPlan::default(),
            scratch("lab-panic"),
            "lab-panic",
        )
        .expect("thread starts");
        assert!(!lab.panicked(), "a run that has just started is not dead");
        let _ = lab.wait_for_first(Duration::from_secs(60));
        assert!(
            !lab.panicked(),
            "a run that has scored a generation is not dead"
        );
        lab.stop();
        assert!(!lab.panicked(), "stopping a run on purpose is not a death");
    }

    fn shipped(slug: &str) -> Level {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"));
        Level::from_json_file(&path).unwrap_or_else(|e| panic!("{slug}: {e}"))
    }

    /// **A custom copy of a shipped level wears its own file's stem, not the shipped name.**
    ///
    /// The bug this pins cost a real file: a run on `custom-l1-flat-run.json` — a copy of the pack's
    /// first level, which keeps the name `L1-flat-run` because that is what a copy is — wrote its
    /// champion to `champions/l1-flat-run.json`, replacing the pack's champion with a policy trained
    /// on different geometry. The level's display name is the shelf's word for the card; the file is
    /// what every other artifact in the workspace is keyed on, and the run has to agree with them.
    #[test]
    fn a_custom_copy_of_a_shipped_level_names_its_artifacts_after_its_own_file() {
        let level = shipped("l1-flat-run");
        assert_eq!(level.name, "L1-flat-run");
        assert_eq!(
            artifact_stem(&level, Some(Path::new("levels/custom-l1-flat-run.json"))),
            "custom-l1-flat-run"
        );
        // A document that has never been saved has no file to be keyed on yet, and its first save
        // is named after the slugged display name — so that is the stem until there is a file.
        assert_eq!(artifact_stem(&level, None), "l1-flat-run");

        // And the run wears what it was handed: the champion the window saves is
        // `<stem>.json`, and the checkpoint and practice level a detour writes share it.
        let lab = Lab::start(
            &level,
            tiny_config(),
            1,
            1,
            RunPlan {
                cap: Some(1),
                ..RunPlan::default()
            },
            scratch("lab-copy-stem"),
            artifact_stem(&level, Some(Path::new("levels/custom-l1-flat-run.json"))),
        )
        .expect("the run starts");
        assert_eq!(lab.stem(), "custom-l1-flat-run");
    }

    /// Take the packet the worker has ready, waiting for it.
    ///
    /// Requests nothing on purpose: *when* the worker is asked — at take time, which is the
    /// lookahead, or when an animation ends with nothing queued, which is the fallback — is what
    /// the test below is about, so asking is its caller's job rather than this helper's.
    fn take_waiting(lab: &mut Lab) -> Lanes {
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            if let Some(lanes) = lab.take() {
                return lanes;
            }
            assert!(
                Instant::now() < deadline,
                "the worker never produced a generation"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The lookahead's invariant: the window may ask for one generation ahead of the one on
    /// screen, and the worker never runs more than one past it.
    #[test]
    fn the_worker_never_runs_more_than_one_generation_past_the_window() {
        let mut lab = Lab::start(
            &shipped("l3-gauntlet"),
            tiny_config(),
            DEFAULT_BEST_LANES,
            DEFAULT_RANDOM_LANES,
            RunPlan::default(),
            scratch("lookahead"),
            "l3-gauntlet",
        )
        .expect("the run starts");

        // A fresh run has asked for its first generation already (that is what `Lab::start`
        // does), so the window's guard is closed until it takes one.
        assert!(
            lab.asked(),
            "a run asks for its first generation when it starts"
        );
        let first = take_waiting(&mut lab);
        assert_eq!(first.packet().generation, 0);

        // Taking a generation clears the ask — which is exactly what lets the window ask for
        // the next one the moment this one is on screen, instead of waiting out its animation.
        assert!(
            !lab.asked(),
            "taking a generation has to clear the ask, or the lookahead can never happen"
        );
        lab.request_next();
        assert!(
            lab.asked(),
            "with one generation in flight the window's own guard refuses a second request, \
             which is what bounds the lookahead to a single generation"
        );
        let second = take_waiting(&mut lab);
        assert_eq!(second.packet().generation, 1);

        // And now the worker is parked on the generation the window is holding: nothing has
        // asked for another one, and a run is not allowed to train ahead of its viewer. The
        // wait is many generations' worth of work at this population size.
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            lab.take().is_none(),
            "the worker computed a generation nobody asked for"
        );
        lab.stop();
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

    /// A scratch directory for a test's artifacts: the champion it writes and the
    /// checkpoint a curriculum step writes both land here rather than in the repository.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evo-app-{tag}-{}", std::process::id()));
        // A directory left behind by an earlier run must not be mistaken for this one's
        // output: "the checkpoint exists" is only evidence if nothing wrote it before this
        // test did.
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn new_generator(slug: &str, target: Option<u32>) -> Generator {
        generator_on(
            slug,
            RunPlan {
                cap: target,
                ..RunPlan::default()
            },
        )
    }

    fn generator_on(slug: &str, plan: RunPlan) -> Generator {
        generator_with(slug, tiny_config(), plan, "lab")
    }

    /// A generator writing its artifacts under their own tag: two tests running in
    /// parallel must not find each other's checkpoint.
    fn generator_with(slug: &str, cfg: GaConfig, plan: RunPlan, tag: &str) -> Generator {
        Generator::new(
            shipped(slug),
            cfg,
            DEFAULT_BEST_LANES,
            DEFAULT_RANDOM_LANES,
            plan,
            scratch(tag),
            slug,
        )
        .expect("a fresh generator starts")
    }

    /// A population that **cannot move**: no mutation, no crossover, no immigrants, so the
    /// best score is flat from generation 0 and the 25-generation stall rule is a fact about
    /// the fixture rather than about how a sensor version happened to change the landscape.
    /// The curriculum tests are about the stall *chain* — checkpoint, detour, return — not
    /// about whether a tiny population gets lucky on `l5-tightrope`, which is exactly the
    /// coupling a sensor bump used to break.
    fn stalling_config() -> GaConfig {
        GaConfig {
            pop_size: 2,
            elite: 1,
            tournament: 2,
            mutation_sigma: 0.0,
            mutation_rate: 0.0,
            crossover_rate: 0.0,
            fine_children: 0,
            immigrants: 0,
            archive_children: 0,
            adaptive_sigma: false,
            ..tiny_config()
        }
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
        generator.start_over(other.clone(), 99, 0.10, "other", None);
        assert_eq!(generator.level().name, other.name);
        let fresh = generator.next_packet();
        assert_eq!(
            fresh.generation, 0,
            "a fresh population starts at generation 0"
        );
        assert_eq!(
            fresh.history.len(),
            1,
            "the old chart is gone with the old run"
        );
        // A first generation always records (there is nothing to beat), so the record is
        // not empty — but everything in it belongs to the fresh run, which is what
        // "the old record is gone" actually means.
        assert!(
            fresh.traces.iter().all(|t| t.generation == 0),
            "an old route survived the restart: {:?}",
            fresh
                .traces
                .iter()
                .map(|t| t.generation)
                .collect::<Vec<_>>()
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

    /// The follow camera's rule, animated for real: the leader is a lane that is still
    /// standing, and no standing lane is further along than it is.
    #[test]
    fn the_leader_is_the_lane_furthest_along_that_is_still_standing() {
        let mut generator = new_generator("l2-first-gap", None);
        let packet = generator.next_packet();
        let level = generator.level().clone();
        let eval = tiny_config().eval_config();
        let mut lanes = Lanes::new(packet, &level, &eval);

        let standing_centres = |lanes: &Lanes| -> Vec<(f32, f32)> {
            (0..lanes.len())
                .filter_map(|i| {
                    let sim = lanes.sim(i);
                    let outcome = sim.outcome();
                    let player = sim.player();
                    (!outcome.is_finished() || outcome == Outcome::Won)
                        .then_some((player.x + player.w * 0.5, player.y + player.h * 0.5))
                })
                .collect()
        };

        let mut followed = 0usize;
        for _ in 0..400 {
            let standing = standing_centres(&lanes);
            match leader_centre(&lanes) {
                Some(centre) => {
                    followed += 1;
                    assert!(
                        standing.contains(&centre),
                        "the leader {centre:?} is not one of the standing lanes {standing:?}"
                    );
                    let furthest = standing
                        .iter()
                        .map(|(x, _)| *x)
                        .fold(f32::NEG_INFINITY, f32::max);
                    assert_eq!(
                        centre.0, furthest,
                        "a lane further along should have been followed"
                    );
                }
                None => assert!(
                    standing.is_empty(),
                    "nobody standing, so there is nobody to follow"
                ),
            }
            if lanes.is_finished() {
                break;
            }
            lanes.advance(1.0 / 60.0, 8);
        }
        assert!(followed > 0, "the rule never had anybody to follow");
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
        let mut lab = Lab::start(
            &level,
            tiny_config(),
            4,
            4,
            RunPlan {
                cap: Some(2),
                ..RunPlan::default()
            },
            scratch("lab-run"),
            "lab-run",
        )
        .expect("thread starts");
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
        let mut lab = Lab::start(
            &level,
            tiny_config(),
            2,
            2,
            RunPlan::default(),
            scratch("lab-fresh"),
            "lab-fresh",
        )
        .expect("thread starts");
        // A restart does not wait for the animation to finish: the window may want to
        // abandon a run it has already seen the first generation of.
        lab.restart(&shipped("l3-gauntlet"), 7, "l3-gauntlet");
        assert!(lab.trained_on(&shipped("l3-gauntlet")));
        let first = wait_for(&mut lab, Duration::from_secs(30)).expect("the fresh run reports");
        assert_eq!(first.packet().generation, 0);
        assert_eq!(first.packet().level_name, shipped("l3-gauntlet").name);
    }

    /// **A chaser placed after a run started makes the run stale.**
    ///
    /// The regression this pins: the run's identity was the terrain hash, which leaves out
    /// chasers, flyers, turrets and movers. Placing one changed the document but not the hash,
    /// so the window decided the run was still on "the same level" and kept drawing its own
    /// stale copy — a chaser visible in Edit and Play and missing in Evolve, which is the one
    /// view that draws a copy instead of the document.
    #[test]
    fn an_entity_placed_after_a_run_starts_makes_its_level_stale() {
        let level = shipped("l1-flat-run");
        let lab = Lab::start(
            &level,
            tiny_config(),
            1,
            1,
            RunPlan {
                cap: Some(1),
                ..RunPlan::default()
            },
            scratch("lab-stale-entity"),
            "lab-stale-entity",
        )
        .expect("thread starts");
        assert!(lab.trained_on(&level), "the run starts on the document");

        let mut edited = level.clone();
        edited.chasers.push(sim::Rect::new(4.0, 8.0, 0.8, 0.8));
        assert!(
            edited.content_hash() == level.content_hash(),
            "a chaser does not change the terrain hash, which champions are keyed on"
        );
        assert!(
            !lab.trained_on(&edited),
            "but it does change the run's level, so the evolve view must restart on it"
        );
    }

    /// The window's one write to disk, driven end to end: a real run's best-ever genome
    /// goes out as a champion file and comes back as one this build will replay.
    #[test]
    fn a_champion_saved_from_a_run_loads_back_on_the_level_it_came_from() {
        let mut generator = new_generator("l3-gauntlet", Some(1));
        let packet = generator.next_packet();
        let best = packet
            .best
            .clone()
            .expect("a scored generation has a best-ever genome");

        let dir = scratch("champion");
        let path = save_champion(
            &best,
            generator.level(),
            &tiny_config(),
            &dir,
            "l3-gauntlet",
        )
        .expect("the champion file is written");
        let loaded = ChampionFile::load_for_level(&path, generator.level())
            .expect("the file this run wrote loads back against its own level");

        assert_eq!(loaded.metadata.genome_id, best.genome.id);
        assert_eq!(loaded.metadata.generation, best.generation);
        assert_eq!(loaded.fitness, best.result.fitness);
        assert_eq!(
            loaded.metadata.level_hash,
            generator.level().content_hash_string()
        );
        assert_eq!(loaded.metadata.outcome, best.result.outcome.as_str());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// A real run's best-ever genome, written to a scratch champion file.
    ///
    /// The two seeds these tests need are "where the champion came from" and "where the
    /// continued run's artifacts go", so they are two scratch directories rather than one:
    /// `scratch` starts by deleting its directory, and a source file deleted by the second
    /// call would make the test pass on nothing.
    fn saved_champion(tag: &str) -> (Level, PathBuf) {
        let level = shipped("l1-flat-run");
        let mut source = Generator::new(
            level.clone(),
            tiny_config(),
            DEFAULT_BEST_LANES,
            DEFAULT_RANDOM_LANES,
            RunPlan::default(),
            scratch(&format!("{tag}-source")),
            "l1-flat-run",
        )
        .expect("the source run starts");
        let best = source
            .next_packet()
            .best
            .clone()
            .expect("a scored generation has a best-ever genome");
        let dir = scratch(&format!("{tag}-file"));
        let path = save_champion(&best, &level, &tiny_config(), &dir, "l1-flat-run")
            .expect("the champion writes");
        (level, path)
    }

    /// **Continue from a saved champion**: the window's half of `--from-champion`.
    ///
    /// The artifact a run leaves behind when it was stopped without a checkpoint is the
    /// champion file, and this is the path back from it into a population: slot 0 is the saved
    /// genome, the run starts at generation 0, and the champion's own recorded episode is its
    /// record — so the first generation ranks against a winner instead of from nothing, and a
    /// mastery league can arm at once.
    #[test]
    fn a_run_can_start_from_a_saved_champion() {
        let (level, path) = saved_champion("champion-seed");
        let champion = ChampionFile::load_for_level(&path, &level).expect("the file loads");

        let seeded = Generator::new(
            level.clone(),
            tiny_config(),
            DEFAULT_BEST_LANES,
            DEFAULT_RANDOM_LANES,
            RunPlan {
                seed_from_champion: Some(path.clone()),
                ..RunPlan::default()
            },
            scratch("champion-seed-run"),
            "l1-flat-run",
        )
        .expect("a champion seeds a run");

        let origin = seeded.origin().expect("the run says where it started");
        assert!(origin.contains("seeded from"), "{origin}");
        let held = seeded
            .best()
            .expect("the champion's episode is the new run's record");
        assert_eq!(held.genome.id, champion.metadata.genome_id);
        assert_eq!(held.result.fitness, champion.fitness);
        assert_eq!(held.result.frames, champion.metadata.frames);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    /// A restart can continue a champion, and a plain restart forgets it.
    ///
    /// The distinction is the whole meaning of the two controls: *Continue champion* starts a
    /// new run from the saved genome, while *Restart run* is a fresh population — replaying
    /// the champion on every restart would make the word mean the opposite of what the button
    /// says.
    #[test]
    fn a_restart_can_continue_a_saved_champion_and_a_plain_restart_forgets_it() {
        let (level, path) = saved_champion("champion-restart");
        let champion = ChampionFile::load_for_level(&path, &level).expect("the file loads");
        let seed = champion_seed(&path, &level).expect("the saved champion is trainable");

        let mut generator = Generator::new(
            level.clone(),
            tiny_config(),
            DEFAULT_BEST_LANES,
            DEFAULT_RANDOM_LANES,
            RunPlan {
                seed_from_champion: Some(path.clone()),
                ..RunPlan::default()
            },
            scratch("champion-restart-run"),
            "l1-flat-run",
        )
        .expect("the run starts");
        generator.start_over(level.clone(), 7, 0.05, "l1-flat-run", Some(seed));
        let origin = generator.origin().expect("the restart says where it started");
        assert!(origin.contains("seeded from"), "{origin}");
        let held = generator.best().expect("the record came with the champion");
        assert_eq!(held.genome.id, champion.metadata.genome_id);

        generator.start_over(level, 8, 0.05, "l1-flat-run", None);
        assert!(generator.origin().is_none(), "a plain restart starts from nothing");
        assert!(
            generator.plan().seed_from_champion.is_none(),
            "a champion is a way into a run, not a setting of every restart"
        );
        assert!(generator.best().is_none(), "a fresh population has no record yet");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    /// A champion that cannot seed this run is a refusal at the window, not a silent reset.
    #[test]
    fn a_champion_this_build_cannot_train_on_is_refused_before_the_population_is_dropped() {
        let (level, path) = saved_champion("champion-bad-seed");
        std::fs::write(&path, "{\"format\": \"not-a-champion\"}").expect("the file writes");
        let err = champion_seed(&path, &level).expect_err("a malformed file is refused");
        assert!(err.contains("champion"), "{err}");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    /// The window's `--curriculum`, driven end to end: a run that stops moving writes
    /// its stuck population down, trains it on the easier level, says so once, and
    /// carries on where it was.
    #[test]
    fn a_stalled_run_checkpoints_itself_and_hands_the_population_back() {
        // L5 at pop 2 does not improve for the 25 generations the stall rule asks for,
        // which is exactly the trigger; L1 is easier, and a small population wins it at
        // once, so the detour is cheap to run. Same shape as the terminal's own test of
        // the same feature, because it is the same feature.
        //
        // **The cap is 60, and it is a measurement rather than a round number.** At pop 2 the
        // run's last new best is generation 21 and the 25-flat window therefore opens at 46, so
        // a cap of 40 left no room for the stall the test is about to happen in (sensor v7's
        // extra input moved the trajectory, and this test caught it). Fourteen generations of
        // margin is the point: the run has to *stall inside the window*, and the test says so
        // when it does not.
        let dir = scratch("curriculum");
        let mut generator = generator_with(
            "l5-tightrope",
            stalling_config(),
            RunPlan {
                cap: Some(60),
                curriculum: Some(Curriculum {
                    level: shipped("l1-flat-run"),
                    generations: 2,
                }),
                ..RunPlan::default()
            },
            "curriculum",
        );

        // The note is sticky on purpose — "this run changed shape" is not something the
        // viewer should have to catch inside one generation — so the test waits for it to
        // appear, then insists that it never changes afterwards. A run that stepped a
        // second time would have a second set of numbers to report.
        let (mut note, mut first_seen, mut last) = (None, None, None);
        for _ in 0..60 {
            let packet = generator.next_packet();
            if let Some(text) = &packet.curriculum {
                match &note {
                    None => {
                        first_seen = Some(packet.generation);
                        assert!(
                            text.contains(&shipped("l1-flat-run").name),
                            "the note names the level the population was trained on: {text}"
                        );
                        assert!(text.contains("best there"), "{text}");
                        assert!(text.contains("back on"), "{text}");
                    }
                    Some(earlier) => assert_eq!(
                        earlier, text,
                        "the run stepped back a second time: the note changed"
                    ),
                }
                note = Some(text.clone());
            }
            last = Some(packet.clone());
            if packet.finished {
                break;
            }
        }
        let note = note.expect("L5 at pop 2 stalls, so the step should have fired");
        assert!(
            note.contains("stalled population"),
            "the note should say where the population came from: {note}"
        );
        assert!(
            note.contains(&shipped("l5-tightrope").name),
            "the note should say which level the run is back on: {note}"
        );

        // The checkpoint is the stuck population, under the level's own stem: that file
        // is the way back into this run, for the terminal's `--resume` and the window's.
        let path = dir.join("l5-tightrope.population.json");
        let checkpoint = PopulationFile::load(&path).expect("the step's checkpoint loads");
        assert_eq!(
            checkpoint.networks.len(),
            2,
            "the whole population is written"
        );
        assert!(
            checkpoint.note.contains("stalled"),
            "the checkpoint records why it exists: {:?}",
            checkpoint.note
        );
        assert!(
            checkpoint.generation > 0 && checkpoint.generation < 60,
            "the checkpoint was taken at the stall, not at the cap: generation {}",
            checkpoint.generation
        );
        // And it was taken when the note appeared, not at some later stall: the run
        // stepped back once, so the file a viewer would resume has the population the
        // first stall held.
        let seen = first_seen.expect("the note was seen");
        assert!(
            checkpoint.generation <= seen + 1,
            "the checkpoint (generation {}) is newer than the step the note reported \
             (generation {seen}), so the run stepped back twice",
            checkpoint.generation
        );

        // The run kept its own chart on its own level: the detour is a pause in the run,
        // not a new run, so the history is the main level's from start to finish.
        let last = last.expect("the loop scored at least one generation");
        assert_eq!(last.generation, 59, "a capped run still runs to its cap");
        assert_eq!(last.history.last().map(|p| p.generation), Some(59));
        assert_eq!(last.level_name, shipped("l5-tightrope").name);
    }

    /// **A stall's detour is handed off, so the generation it fires on is not forty
    /// generations long.**
    ///
    /// The freeze this pins: the practice detour used to run between two generations, so the
    /// packet that carried it took 1.73 s on `scratch-tower` against a 20-50 ms median, and the
    /// window — which only animates what the worker hands it — sat on a still frame for the
    /// difference. The packet now returns at its usual cost with the detour on a thread, and the
    /// next generation arrives while that thread is still working.
    #[test]
    fn a_stall_hands_its_practice_detour_to_another_thread() {
        let mut generator = generator_with(
            "l5-tightrope",
            stalling_config(),
            RunPlan {
                cap: Some(60),
                ..RunPlan::default()
            },
            "practice-thread",
        );
        let mut stall_packet = None;
        let mut afterwards = None;
        for _ in 0..60 {
            let packet = generator.next_packet();
            if fitness_stall(&packet.history).is_some() {
                if stall_packet.is_none() {
                    stall_packet = Some((packet.generation, generator.detour_in_flight()));
                } else if let Some((at, _)) = stall_packet {
                    if packet.generation > at {
                        afterwards = Some(packet.generation);
                        break;
                    }
                }
            }
        }
        let (at, in_flight) = stall_packet.expect("the run has to stall for this to say anything");
        assert!(
            in_flight,
            "the detour has to be on another thread the moment the stall's packet is returned: \
             inline it is the 1.7 s freeze this test exists for"
        );
        assert!(
            afterwards.is_some_and(|g| g > at),
            "the generation after the stall must arrive without waiting for the detour"
        );
    }

    /// A run without an easier level never steps back, however long it sits still: the
    /// stall line reports the stall, and nothing acts on it.
    #[test]
    fn a_run_with_no_easier_level_practises_where_it_is_stuck() {
        // Finding 50: with no authored fallback, a stalled run derives one — its own
        // level from the furthest stand its best genome has made — and steps back to
        // it. The note, the checkpoint and the practice level all appear, and the
        // note names the spot: "this run changed shape" is the viewer's to read.
        //
        // The fixture needs a best genome that has *stood* somewhere. A frozen population on
        // `l11` stands on the spawn stand and gets no further, which is a valid — if shallow —
        // stall. Warming it from the plan would put it over the plateau instead, and this test is
        // about the step back, not about how far the population got.
        let dir = scratch("practice");
        let mut generator = generator_with(
            "l11-the-elements",
            GaConfig {
                pop_size: 8,
                ..stalling_config()
            },
            RunPlan {
                cap: Some(60),
                // No warm start: the fixture is a population that cannot move, and a random
                // population's own spawn stand is enough for the derived practice level. A plan
                // clone would be a second thing this test is measuring.
                warm_from_plan: false,
                ..RunPlan::default()
            },
            "practice",
        );
        let mut stalled = false;
        let mut note = None;
        for _ in 0..60 {
            let packet = generator.next_packet();
            stalled |= fitness_stall(&packet.history).is_some();
            note = packet.curriculum.or(note);
            if packet.finished {
                break;
            }
        }
        assert!(
            stalled,
            "this run has to actually stall, or it proves nothing about stepping back"
        );
        let note = note.expect("a stalled run with no easier level practises where it is stuck");
        assert!(
            note.contains("practice at tile"),
            "the note names the spot it practised from: {note:?}"
        );
        assert!(
            dir.join("l11-the-elements.population.json").exists(),
            "the stuck population is checkpointed"
        );
        assert!(
            dir.join("l11-the-elements.practice.json").exists(),
            "the practice level is written beside the checkpoint"
        );
    }

    /// The window's `--resume`: the population, its ids and its generation continue from
    /// the checkpoint, and the run says where it came from.
    #[test]
    fn a_run_started_from_a_checkpoint_continues_it() {
        let dir = scratch("resume");
        let level = shipped("l3-gauntlet");
        let cfg = tiny_config();

        // Write a checkpoint the way a step does, out of a real run's own population.
        let mut source = Generator::new(
            level.clone(),
            cfg.clone(),
            2,
            2,
            RunPlan {
                cap: Some(3),
                ..RunPlan::default()
            },
            dir.clone(),
            "l3-gauntlet",
        )
        .expect("a fresh generator starts");
        let mut packet = source.next_packet();
        for _ in 1..3 {
            packet = source.next_packet();
        }
        let genomes: Vec<Genome> = packet
            .lanes
            .iter()
            .take(2)
            .map(|lane| lane.genome.clone())
            .collect();
        let checkpoint = PopulationFile::new(
            &genomes,
            packet.generation,
            &level,
            &cfg,
            "operator's own reason",
        );
        let path = dir.join("l3-gauntlet.population.json");
        checkpoint.save(&path).expect("the checkpoint is written");

        let mut resumed = Generator::new(
            level.clone(),
            cfg.clone(),
            2,
            2,
            RunPlan {
                cap: Some(1),
                resume: Some(path.clone()),
                ..RunPlan::default()
            },
            dir.clone(),
            "l3-gauntlet",
        )
        .expect("a checkpoint this build wrote loads");

        let origin = resumed
            .origin()
            .expect("a resumed run says where it came from");
        assert!(origin.starts_with("resumed "), "{origin}");
        assert!(origin.contains("population of 2"), "{origin}");
        assert!(
            origin.contains("operator's own reason"),
            "the checkpoint's own note comes through: {origin}"
        );

        let first = resumed.next_packet();
        assert_eq!(
            first.generation, checkpoint.generation,
            "a resumed run scores the checkpoint's own generation first — nothing is skipped \
             and nothing is renumbered"
        );
        let ids: Vec<u32> = first.lanes.iter().map(|lane| lane.genome_id).collect();
        assert!(
            ids.iter().all(|id| checkpoint.genome_ids.contains(id)),
            "the lanes are the checkpoint's genomes, not a fresh population: {ids:?} against {:?}",
            checkpoint.genome_ids
        );

        // And a checkpoint that cannot be read is a refusal, not a run on something else.
        let missing = Generator::new(
            level,
            cfg,
            2,
            2,
            RunPlan {
                resume: Some(dir.join("not-written-yet.json")),
                ..RunPlan::default()
            },
            dir,
            "l3-gauntlet",
        );
        assert!(
            missing.is_err(),
            "a checkpoint that does not load must not silently train a fresh population"
        );
    }

    /// **A fresh run on a routable level starts from a clone of its timed plan.**
    ///
    /// The window's Phase 1 warm start: the first generation's population is not random, the
    /// certified plan's own block actions produced the clone, and the run *says so* — the origin
    /// note is what the panel shows, so a warm start is never something the operator has to infer
    /// from the first generation's scores.
    ///
    /// The copy is written into this test's own directory rather than read from the shipped file:
    /// a sidecar beside `levels/l1-flat-run.json` is a developer artifact, and adopting one would
    /// mean measuring whatever graph this machine last wrote instead of what the code builds now.
    /// The flat level is also the cheapest routable fixture: its clone reaches the flag in the
    /// first DAgger round, so this test pays one fit rather than the six a pit level would.
    #[test]
    fn a_fresh_run_warmed_from_the_timed_plan_says_so_and_is_not_random() {
        let dir = scratch("plan-warm");
        std::fs::create_dir_all(&dir).unwrap();
        let level_path = dir.join("l1-flat-run.json");
        shipped("l1-flat-run")
            .to_json_file(&level_path)
            .expect("a copy of the level");
        let level = Level::from_json_file(&level_path).expect("the copy loads");
        let cfg = tiny_config();
        let warm = Generator::new(
            level.clone(),
            cfg.clone(),
            2,
            2,
            RunPlan {
                warm_from_plan: true,
                ..RunPlan::default()
            },
            dir.clone(),
            "l1-flat-run",
        )
        .expect("a warmed run starts");
        let note = warm
            .origin()
            .expect("a warmed run says where its first generation came from")
            .to_string();
        assert!(
            note.contains("warmed from this level's timed plan")
                && note.contains("legs")
                && note.contains("cloned"),
            "the note names the plan and the clone: {note}"
        );
        assert!(note.contains("2 of 8 genomes"), "{note}");

        // The same level, unwarmed, is the control: same seeds, same budget, a random population.
        let cold = Generator::new(level, cfg, 2, 2, RunPlan::default(), dir, "l1-flat-run")
            .expect("a plain run starts");
        assert!(
            cold.origin().is_none(),
            "nothing to declare about a population nobody handed over"
        );
    }

    /// **A plan warm start is a clone of the plan's own blocks, cut into its legs.** The fit is
    /// cheap here, because the claim is about the note and the population rather than about how far
    /// 300 gradient passes get; the shipped budget is exercised by a real run.
    #[test]
    fn a_plan_warm_start_names_the_legs_and_the_clone() {
        let level = shipped("l2-first-gap");
        let cfg = tiny_config();
        let imit = ImitateConfig {
            epochs: 20,
            ..ImitateConfig::default()
        };
        let (pop, note) = warm_seed_from_plan_with(
            &level,
            &cfg,
            sim::plan::PlanConfig {
                depth: 48,
                ..sim::plan::PlanConfig::default()
            },
            &imit,
        );
        let pop = pop.expect("a routable level warms");
        assert_eq!(pop.len(), cfg.pop_size);
        assert!(
            note.starts_with("warmed from this level's timed plan")
                && note.contains("legs /")
                && note.contains("cloned"),
            "the note names the plan's legs and the clone: {note}"
        );
    }

    /// **The case Phase 1 must report rather than paper over.** The oracle certifies the level,
    /// but this planner budget cannot find a line. The population starts random, and the origin
    /// note names the planner's failure — it does not quietly fall back to the tick-indexed route
    /// walk, which is the weaker demonstration the plan exists to replace.
    #[test]
    fn a_level_the_planner_cannot_route_starts_random_instead_of_from_a_weaker_route() {
        let level = shipped("l2-first-gap");
        let (pop, note) = warm_seed_from_plan_with(
            &level,
            &tiny_config(),
            sim::plan::PlanConfig {
                depth: 1,
                ..sim::plan::PlanConfig::default()
            },
            &ImitateConfig::default(),
        );
        assert!(pop.is_none(), "no certified plan, no warm population");
        assert!(note.starts_with("no warm start:"), "{note}");
        assert!(
            note.contains("beatable in principle"),
            "the note must distinguish the planner's budget failure from an unbeatable level: {note}"
        );
        assert!(
            !note.contains("walked") && !note.contains("route walk"),
            "the fallback is random, not the older walk: {note}"
        );
    }

    /// A restart is a fresh population: the checkpoint it may have started from is
    /// forgotten, while the fallback level — a setting of the run rather than of its
    /// population — is kept.
    #[test]
    fn a_restart_forgets_the_checkpoint_but_keeps_the_fallback() {
        let dir = scratch("restart-plan");
        let level = shipped("l1-flat-run");
        let cfg = tiny_config();
        let mut rng = sim::rng_from_seed(7);
        let genomes = vec![
            Genome::random(11, evo::default_shape(), &mut rng),
            Genome::random(12, evo::default_shape(), &mut rng),
        ];
        let checkpoint = PopulationFile::new(&genomes, 12, &level, &cfg, "a stall");
        let path = dir.join("l1-flat-run.population.json");
        checkpoint.save(&path).expect("the checkpoint is written");

        let mut generator = Generator::new(
            level.clone(),
            cfg,
            2,
            2,
            RunPlan {
                resume: Some(path),
                curriculum: Some(Curriculum {
                    level: shipped("l2-first-gap"),
                    generations: 3,
                }),
                ..RunPlan::default()
            },
            dir,
            "l1-flat-run",
        )
        .expect("the checkpoint loads");

        generator.start_over(shipped("l3-gauntlet"), 5, 0.05, "l3-gauntlet", None);
        assert!(
            generator.plan().resume.is_none(),
            "a restart reproduces nothing: it starts a fresh population"
        );
        assert!(
            generator.plan().curriculum.is_some(),
            "the fallback level belongs to the run, not to the population"
        );
        assert!(
            generator.origin().is_none(),
            "nothing to declare about a population nobody handed over"
        );
        assert_eq!(generator.next_packet().generation, 0);
    }

    /// Print what each part of a generation costs, on a run big enough to see the hitches in.
    ///
    /// Ignored because it is an instrument, not an assertion: it trains for real and prints to
    /// stderr. Run it with
    /// `cargo test -p app --release --lib profile_generations -- --ignored --nocapture` and read
    /// the `lab ...` lines. It exists because "the UI freezes" is not a bug report a profiler can
    /// take: the phases that can stall a frame are named here, so the next measurement says which.
    #[test]
    #[ignore]
    fn profile_generations() {
        let level = shipped("scratch-tower");
        let cfg = GaConfig {
            pop_size: 200,
            ..GaConfig::default()
        };
        let started = Instant::now();
        let mut generator = Generator::new(
            level,
            cfg,
            DEFAULT_BEST_LANES,
            DEFAULT_RANDOM_LANES,
            RunPlan::default(),
            scratch("profile"),
            "scratch-tower",
        )
        .expect("a fresh generator starts");
        eprintln!("lab Generator::new (warm start) {:?}", started.elapsed());
        for _ in 0..120 {
            let packet = generator.next_packet();
            eprintln!(
                "lab gen {} max {:.0}",
                packet.generation,
                packet.history.last().map(|h| h.max_fitness).unwrap_or(0.0)
            );
        }
    }

    #[test]
    fn dropping_the_lab_stops_the_worker() {
        let level = shipped("l1-flat-run");
        let lab = Lab::start(
            &level,
            tiny_config(),
            4,
            4,
            RunPlan::default(),
            scratch("lab-drop"),
            "lab-drop",
        )
        .expect("thread starts");
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
