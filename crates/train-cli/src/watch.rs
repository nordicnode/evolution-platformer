//! Watch mode — the M3 deliverable and the instrument for the fun gate (§10, §12).
//!
//! Training is fast enough to be invisible: a whole generation at pop 200 costs
//! ~14 ms, and L3 is solved before generation 25. So watch mode is **not** "training
//! at 50x speed" — there is nothing to see if you simply run the trainer faster. It
//! is deliberately *slower* than training: each generation is evaluated once, and
//! then the eight episodes the plan asks for (best-4 + random-4, §10 Tier 1.5) are
//! re-run tick by tick so a human can watch them. The animation is the rate limit;
//! `--speed` is how many physics ticks pass per rendered frame, so it is literally
//! "how much of the game do I skip between frames".
//!
//! ## Fidelity
//!
//! Every lane is driven by [`evo::EpisodeRunner`] with the exact seed
//! [`evo::Trainer::showcase`] recorded, so a lane animates **the episode that was
//! scored**, not a fresh episode on the same genome. Turn any lane's fitness into
//! the animation and they agree; that is asserted in `evo`'s tests
//! (`every_showcase_lane_replays_the_episode_that_was_evaluated`). A watch mode that
//! animated a *different* episode would be lying about the numbers printed next to
//! it, which would make the whole chart worthless.
//!
//! ## Two things this module refuses to do
//!
//! * It never trains on a timer. One generation, then the animation, then the next
//!   generation: the GA's RNG stream sees exactly the same draws whether or not
//!   anyone is watching (§5.6, asserted in `watching_does_not_change_what_is_evolved`).
//! * It does not need a TTY. Without one it runs **headless**: same trainer, same
//!   lanes, same frame composition, printed as plain text and with no keyboard. That
//!   is what makes the watch layout testable from `cargo test`, and it is the honest
//!   answer for a dumb terminal or a pipe.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::thread::sleep;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use evo::{
    ChampionFile, EpisodeRunner, EvalConfig, GaConfig, LaneKind, ShowcaseLane, Trainer,
};
use render::ascii::{GLYPH_WALKER_ALIVE, GLYPH_WALKER_DEAD};
use render::chart::{
    death_cluster, death_strip, fitness_chart, fitness_stall, fitness_trend, format_fitness,
    progress_bar, Stall,
};
use render::{render_scene, Body, HistoryPoint, Scene, Viewport, FRAME_TIME};
use sim::{Level, Outcome};

use crate::args::Args;
use crate::commands::{find_level_by_hash, load_level, usage};

const WATCH_OPTS: &[&str] = &[
    "level",
    "champion",
    "levels-dir",
    "generations",
    "pop",
    "seed",
    "mut-rate",
    "mut-sigma",
    "elite",
    "tournament",
    "crossover",
    "action-repeat",
    "trials",
    "out",
    "speed",
    "best",
    "random",
    "death-window",
    "frames",
    "headless",
];

/// The speed ladder. `+`/`-` step through it rather than doubling forever, so the
/// range the plan asks for (1x-50x, §10 Tier 1.4) is seven keystrokes wide and no
/// keypress can push the run past the top or stall it at zero.
const SPEEDS: [u32; 7] = [1, 2, 4, 8, 16, 32, 50];

/// Physics ticks per rendered frame at startup. 8x is the compromise the fun gate
/// needs: fast enough that a 5 000-tick episode is a ten-second generation, slow
/// enough to see a jump happen.
const DEFAULT_SPEED: u32 = 8;

/// Extra frames past `level_max / speed` before a generation is abandoned, so an
/// agent that survives to the episode cap cannot hold the view hostage.
const SLACK_FRAMES: usize = 30;

const MIN_CHART_ROWS: usize = 4;
const MAX_CHART_ROWS: usize = 10;
const MIN_SCENE_ROWS: usize = 5;

/// Rendered frames a headless run draws before stopping on its own.
const DEFAULT_HEADLESS_FRAMES: usize = 6;

/// Frame size used when there is no terminal to ask (headless, pipes, tests).
const HEADLESS_COLS: usize = 100;
const HEADLESS_ROWS: usize = 34;

// ---------------------------------------------------------------------------
// watch
// ---------------------------------------------------------------------------

pub fn watch(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(WATCH_OPTS))?;

    let levels_dir = a.string("levels-dir", "levels");
    let champion_path = a.opt("champion").map(|s| s.to_string());
    let level = resolve_level(a, &levels_dir, champion_path.as_deref())?;
    // The champion file is named after the level file, exactly as `train` names it.
    let stem = match a.opt("level") {
        Some(path) => Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| level.name.clone()),
        None => level.name.clone(),
    };

    let generations = usage(a.usize_opt("generations", 300))?;
    let pop_size = usage(a.usize_opt("pop", 200))?;
    let best_k = usage(a.usize_opt("best", 4))?;
    let random_k = usage(a.usize_opt("random", 4))?;
    if best_k + random_k == 0 {
        return Err("usage: --best and --random cannot both be zero: nothing would be drawn".into());
    }
    let death_window = usage(a.usize_opt("death-window", 25))? as u32;
    let out_dir = a.string("out", "champions");

    let mut live = LiveState {
        speed: nearest_speed(usage(a.usize_opt("speed", DEFAULT_SPEED as usize))? as u32),
        ..LiveState::default()
    };

    // Headless is a real mode, not a fallback that half-works: no alternate screen,
    // no raw mode, no blocking read, and no wall-clock pacing (a scripted run should
    // take as long as it takes).
    let headless = a.has_flag("headless") || !io::stdout().is_terminal();
    let frame_cap = if headless || a.opt("frames").is_some() {
        Some(usage(a.usize_opt("frames", DEFAULT_HEADLESS_FRAMES))?)
    } else {
        None
    };
    if frame_cap == Some(0) {
        return Err("usage: --frames must be at least 1".into());
    }

    let mut screen = Screen::open(headless, frame_cap).map_err(io_error)?;

    if let Some(path) = &champion_path {
        let champion = watch_champion(&mut screen, &level, path, &mut live)?;
        return finish_champion(&mut screen, &level, &champion);
    }

    let cfg = GaConfig {
        pop_size,
        elite: usage(a.usize_opt("elite", 4))?,
        tournament: usage(a.usize_opt("tournament", 4))?,
        mutation_rate: usage(a.f32_opt("mut-rate", 0.05))?,
        mutation_sigma: usage(a.f32_opt("mut-sigma", 0.30))?,
        crossover_rate: usage(a.f32_opt("crossover", 0.75))?,
        action_repeat: usage(a.usize_opt("action-repeat", 4))? as u32,
        trials: usage(a.usize_opt("trials", 1))? as u32,
        master_seed: usage(a.u64_opt("seed", 1))?,
        ..GaConfig::default()
    };

    println!(
        "watching {} ({}x{})  pop {}  seed {}  lanes {}+{}  speed {}{}",
        level.name,
        level.width,
        level.height,
        cfg.pop_size,
        cfg.master_seed,
        best_k,
        random_k,
        speed_label(live.speed),
        if headless { "  [headless]" } else { "" }
    );
    println!(
        "sim {} / sensors v{} / fitness v{} / oracle: {}",
        sim::SIM_VERSION,
        sim::sensors::VERSION,
        evo::FITNESS_VERSION,
        oracle_line(&level)
    );

    let mut trainer = Trainer::new(cfg);
    let eval: EvalConfig = trainer.config.eval_config();
    let pop_size = trainer.config.pop_size;
    let mut history: Vec<HistoryPoint> = Vec::new();
    let mut deaths: Vec<(f32, f32, u32)> = Vec::new();
    let mut note: Option<String> = None;
    let mut generations_run = 0usize;

    for _ in 0..generations {
        if live.quit {
            break;
        }
        // The population snapshot is taken before breeding, so these are the exact
        // genomes (with the exact ids) the report describes.
        let (report, population) = trainer.step_verbose(&level);
        generations_run += 1;
        history.push(HistoryPoint {
            generation: report.generation,
            max_fitness: report.max_fitness,
            avg_fitness: report.avg_fitness,
            wins: report.wins,
        });

        let lanes = trainer.showcase(&report, &population, best_k, random_k);
        let info = lane_info(&lanes);
        let mut runners: Vec<EpisodeRunner<'_>> = lanes
            .iter()
            .map(|lane| EpisodeRunner::new(&level, &population[lane.index], &eval, lane.seed))
            .collect();

        let ctx = GenCtx {
            generation: report.generation,
            generations,
            pop_size,
            wins: report.wins,
            genius: report.genius,
            history: &history,
            deaths: &deaths,
            note: note.as_deref(),
        };
        animate_lanes(&mut screen, &level, &mut runners, &info, &ctx, &mut live)
            .map_err(io_error)?;

        // Where this generation's lanes died (Tier 2.6). Kept for a rolling window:
        // a forty-generation-old corpse says nothing about where the population is
        // stuck *now*.
        for runner in &runners {
            if let Some((x, y)) = runner.sim().stats().death_pos {
                deaths.push((x, y, report.generation));
            }
        }
        deaths.retain(|(_, _, g)| report.generation.saturating_sub(*g) <= death_window);
        note = None;

        // `s` saves the best-ever champion; handled between generations so the file
        // is written once per keypress and the note outlives the key.
        if live.save_requested {
            live.save_requested = false;
            note = Some(save_champion(&trainer, &level, &stem, &out_dir)?);
        }
    }

    // A watched run is a real run: its best genome is saved exactly like `train`'s,
    // so watching instead of training wastes nothing.
    let saved = if generations_run > 0 {
        Some(save_champion(&trainer, &level, &stem, &out_dir)?)
    } else {
        None
    };
    finish_training(&mut screen, &level, &stem, &trainer, saved, &out_dir)
}

/// A terminal failure is a runtime error, not a usage error.
fn io_error(e: io::Error) -> String {
    format!("terminal error: {e}")
}

/// Guard against a terminal that reports no size at all.
///
/// A pty with no window size attached (a CI shell, `script`, a detached multiplexer
/// pane) reports 0 columns, and a zero-width frame is an empty frame: the screen
/// stays blank and the watch mode looks hung. Falling back to a normal 80-column
/// view is the honest reading of "I do not know how big I am".
fn usable_size(cols: u16, rows: u16) -> (usize, usize) {
    if cols == 0 || rows == 0 {
        (HEADLESS_COLS, HEADLESS_ROWS)
    } else {
        (cols as usize, rows as usize)
    }
}

fn resolve_level(a: &Args, levels_dir: &str, champion_path: Option<&str>) -> Result<Level, String> {
    match (a.opt("level"), champion_path) {
        (Some(path), _) => load_level(path),
        (None, Some(path)) => {
            let raw =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            let champion =
                ChampionFile::from_json_str(&raw).map_err(|e| format!("{path}: {e}"))?;
            find_level_by_hash(levels_dir, &champion.metadata.level_hash)?.ok_or_else(|| {
                format!(
                    "no level in {levels_dir} hashes to {} (champion was trained on {:?}); pass --level",
                    champion.metadata.level_hash, champion.metadata.level_name
                )
            })
        }
        (None, None) => Err("usage: --level <PATH> is required (or --champion <PATH>)".into()),
    }
}

// ---------------------------------------------------------------------------
// lanes
// ---------------------------------------------------------------------------

/// A lane's identity, copied out of the report so the animation loop holds no
/// borrow on the population beyond its own runners.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LaneInfo {
    label: char,
    best: bool,
    genome_id: u32,
    rank: usize,
    /// Fitness of trial 0: the episode this lane replays.
    fitness: f32,
    /// The metric selection ranked on.
    mean_fitness: f32,
}

/// Random-lane labels, with `e` and `x` left out.
///
/// Those are the walker glyphs ([`GLYPH_WALKER_ALIVE`], [`GLYPH_WALKER_DEAD`]), and
/// a lane that looks like an enemy is worse than no label at all.
const RANDOM_LABELS: &[u8] = b"abcdfghijklmnopqrstuvwyz";

fn lane_info(lanes: &[ShowcaseLane]) -> Vec<LaneInfo> {
    // Labels come from each lane's **position within its group**, not from
    // `lane.rank`: the trainer numbers random lanes `best_k + n`, so using the rank
    // would start the outsiders at the fifth letter of the alphabet (or collide with
    // the walker glyphs) whenever four best lanes precede them.
    let (mut best, mut random) = (0usize, 0usize);
    lanes
        .iter()
        .map(|lane| {
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
            LaneInfo {
                label: lane_label(lane.kind, ordinal),
                best: lane.kind == LaneKind::Best,
                genome_id: lane.genome_id,
                rank: lane.rank,
                fitness: lane.fitness,
                mean_fitness: lane.mean_fitness,
            }
        })
        .collect()
}

/// Best lanes are digits in ranking order, random ones are letters: one character
/// each, and the champion is always `1`, which is what the eye tracks.
///
/// Both sequences wrap rather than panicking, so `--best 20` produces a confusing
/// picture instead of a crash. With the default 4 + 4 split every label is unique.
fn lane_label(kind: LaneKind, ordinal: usize) -> char {
    match kind {
        LaneKind::Best => char::from(b'1' + (ordinal % 9) as u8),
        LaneKind::Random => char::from(RANDOM_LABELS[ordinal % RANDOM_LABELS.len()]),
    }
}

/// The glyph a lane's agent is drawn with.
///
/// A finished lane keeps its identity only if it finished *at the flag*. A death
/// becomes `+` because the interesting fact about a dead agent is where it died,
/// and eight distinct glyphs lying in a heap say less than eight graves do.
fn lane_glyph(label: char, finished: bool, outcome: Outcome) -> char {
    if finished && outcome != Outcome::Won {
        '+'
    } else {
        label
    }
}

// ---------------------------------------------------------------------------
// live controls
// ---------------------------------------------------------------------------

/// Controls the viewer holds between frames.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LiveState {
    speed: u32,
    paused: bool,
    /// Follow the leading agent's camera, or hold the whole level still.
    follow: bool,
    quit: bool,
    /// `s` was pressed; the caller must save and then show a note.
    save_requested: bool,
}

impl Default for LiveState {
    fn default() -> LiveState {
        LiveState {
            speed: DEFAULT_SPEED,
            paused: false,
            follow: true,
            quit: false,
            save_requested: false,
        }
    }
}

impl LiveState {
    fn step_speed(&mut self, up: bool) {
        self.speed = if up {
            next_speed_up(self.speed)
        } else {
            next_speed_down(self.speed)
        };
    }
}

/// What one keypress means. Split out of the event loop so the whole key map is
/// testable without a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyAction {
    None,
    SpeedUp,
    SpeedDown,
    TogglePause,
    ToggleFollow,
    SkipGeneration,
    Save,
    Quit,
}

fn key_action(code: KeyCode, modifiers: KeyModifiers) -> KeyAction {
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => KeyAction::Quit,
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => KeyAction::Quit,
        KeyCode::Char(' ') => KeyAction::TogglePause,
        KeyCode::Char('+') | KeyCode::Char('=') => KeyAction::SpeedUp,
        KeyCode::Char('-') | KeyCode::Char('_') => KeyAction::SpeedDown,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('.') | KeyCode::Enter => {
            KeyAction::SkipGeneration
        }
        KeyCode::Char('f') | KeyCode::Char('F') => KeyAction::ToggleFollow,
        KeyCode::Char('s') | KeyCode::Char('S') => KeyAction::Save,
        _ => KeyAction::None,
    }
}

/// Apply a keypress, returning whether the viewer asked to skip this generation's
/// animation.
fn apply_key(action: KeyAction, live: &mut LiveState) -> bool {
    match action {
        KeyAction::None => false,
        KeyAction::SpeedUp => {
            live.step_speed(true);
            false
        }
        KeyAction::SpeedDown => {
            live.step_speed(false);
            false
        }
        KeyAction::TogglePause => {
            live.paused = !live.paused;
            false
        }
        KeyAction::ToggleFollow => {
            live.follow = !live.follow;
            false
        }
        KeyAction::Save => {
            live.save_requested = true;
            false
        }
        KeyAction::SkipGeneration => true,
        KeyAction::Quit => {
            live.quit = true;
            false
        }
    }
}

/// Snap an arbitrary number onto the ladder, so `--speed 12` behaves like the
/// nearest keypress instead of creating a speed no key can get back to.
fn nearest_speed(requested: u32) -> u32 {
    let top = *SPEEDS.last().expect("the ladder is non-empty");
    let clamped = requested.clamp(1, top);
    *SPEEDS
        .iter()
        .min_by_key(|s| s.abs_diff(clamped))
        .expect("the ladder is non-empty")
}

fn next_speed_up(speed: u32) -> u32 {
    let i = SPEEDS
        .iter()
        .position(|s| *s >= speed)
        .unwrap_or(SPEEDS.len() - 1);
    SPEEDS[(i + 1).min(SPEEDS.len() - 1)]
}

fn next_speed_down(speed: u32) -> u32 {
    let i = SPEEDS.iter().rposition(|s| *s <= speed).unwrap_or(0);
    SPEEDS[i.saturating_sub(1)]
}

fn speed_label(speed: u32) -> String {
    format!("{speed}x")
}

// ---------------------------------------------------------------------------
// frame composition
// ---------------------------------------------------------------------------

/// Rows reserved per section, decided before the viewport is built so the renderer
/// and the caller agree on how tall the level view is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    chart_rows: usize,
    scene_rows: usize,
}

/// Fixed rows: two status lines, the generation bar, the death strip and two hint
/// lines. The genius splash, the save note and the stall line each add one.
const FIXED_ROWS: usize = 6;

/// The chart takes a third of the spare rows, the level view the rest.
///
/// On a terminal too short for both minimums the sections keep their minimums and
/// the frame is truncated at draw time: losing the hint lines is survivable, losing
/// the level view is not.
fn layout(rows: usize, genius: bool, has_note: bool, stalled: bool) -> Layout {
    let reserved = FIXED_ROWS + usize::from(genius) + usize::from(has_note) + usize::from(stalled);
    let spare = rows.saturating_sub(reserved);
    let chart_rows = (spare / 3).clamp(MIN_CHART_ROWS, MAX_CHART_ROWS);
    Layout {
        chart_rows,
        scene_rows: spare.saturating_sub(chart_rows).max(MIN_SCENE_ROWS),
    }
}

/// Everything one frame needs: borrowed data, no logic of its own, so
/// [`render_frame`] is a pure function of it and can be tested to the character.
struct Frame<'a> {
    level_name: &'a str,
    level_width_tiles: i32,
    generation: u32,
    generations: usize,
    alive: usize,
    lane_count: usize,
    pop_size: usize,
    wins: usize,
    live: LiveState,
    history: &'a [HistoryPoint],
    deaths: &'a [(f32, f32, u32)],
    /// The level view, already rendered: one string per row, no trailing newline.
    scene_lines: &'a [String],
    note: Option<&'a str>,
    genius: bool,
    /// Set once the best fitness has stopped moving (§13: frame stalling as
    /// difficulty information, not as a failure of the run).
    stall: Option<Stall>,
    /// The tile column deaths cluster at, in the same death window the strip uses.
    cluster: Option<(i32, usize)>,
    progress: Progress,
}

/// How far into a generation's animation budget this frame is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Progress {
    index: usize,
    budget: usize,
}

fn render_frame(f: &Frame, cols: usize, rows: usize, l: Layout) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(rows);

    // 1. Where we are. Always visible (Tier 1.1). The level name gets a fixed
    //    budget: it is the least important thing on the line, and a long one must
    //    not be able to push the flags off an 80-column terminal.
    let mut head = format!(
        "{:<14} gen {}/{}  lanes {}/{} alive  speed {}",
        clip(f.level_name, 14),
        f.generation,
        f.generations.max(1),
        f.alive,
        f.lane_count,
        speed_label(f.live.speed)
    );
    if f.live.paused {
        head.push_str("  PAUSED");
    }
    if !f.live.follow {
        head.push_str("  whole level");
    }
    lines.push(clip(&head, cols));

    // 2. The numbers the chart is drawn from, and the plan's own reading of them
    //    (Tier 1.2: "is this population learning?").
    match f.history.last() {
        Some(last) => lines.push(clip(
            &format!(
                "max {:>9}  avg {:>9}  wins {}/{}   {}",
                format_fitness(last.max_fitness),
                format_fitness(last.avg_fitness),
                f.wins,
                f.pop_size,
                fitness_trend(f.history).label()
            ),
            cols,
        )),
        None => lines.push(clip("warming up: no generation scored yet", cols)),
    }

    // 3. Why the numbers stopped moving, when they have. This is the one piece of
    //    the view that talks about the *level* rather than the population: the
    //    stall count says the search is out of new ideas, and the death cluster says
    //    which obstacle it is out of ideas about. Deaths go first because a
    //    truncated line must keep the actionable half.
    if let Some(stall) = f.stall {
        let line = match f.cluster {
            // Deaths first: a truncated line must keep the actionable half, and
            // "where they die" is what a level author can act on.
            Some((tile, deaths)) => format!(
                "deaths cluster at x {tile} ({deaths} in {})   {}",
                f.deaths.len(),
                stall.label()
            ),
            // No deaths in the window is not a fact worth a phrase of its own: the
            // strip below already shows an empty row, and a line reporting the
            // absence of evidence beside a stall invites reading it as the cause.
            None => stall.label(),
        };
        lines.push(clip(&line, cols));
    }

    // 4. The genius splash (Tier 1.3) — the one deliberate piece of drama.
    if f.genius {
        lines.push(clip(
            "*** GENIUS AGENT: best fitness jumped more than 1.5x in one generation ***",
            cols,
        ));
    }
    if let Some(note) = f.note {
        lines.push(clip(note, cols));
    }

    // 5. The chart.
    lines.append(&mut fitness_chart(f.history, cols, l.chart_rows));

    // 6. Which generation, and how far into its animation we are.
    let fraction = (f.generation as f32 + 1.0) / f.generations.max(1) as f32;
    let bar_width = cols.saturating_sub(34).clamp(8, 40);
    lines.push(clip(
        &format!(
            "gen {} {} of {}   t {}/{} frames",
            progress_bar(fraction, bar_width),
            f.generation + 1,
            f.generations.max(1),
            f.progress.index,
            f.progress.budget
        ),
        cols,
    ));

    // 7. The level, padded to a fixed height so the death strip cannot wander.
    for row in f.scene_lines.iter().take(l.scene_rows) {
        lines.push(clip(row, cols));
    }
    for _ in f.scene_lines.len().min(l.scene_rows)..l.scene_rows {
        lines.push(String::new());
    }

    // 8. Where they died, across the whole level (Tier 2.6).
    lines.push(death_strip(f.deaths, f.level_width_tiles, cols));

    // 9. What the viewer can do, and what the glyphs mean.
    lines.push(clip(
        "keys  space pause   -/+ speed   n next generation   f follow/whole   s save champion   q quit",
        cols,
    ));
    let legend = format!(
        "marks 1-4 best lanes, a-d random lanes, + died, {} walker, {} dead walker, ! goal",
        GLYPH_WALKER_ALIVE, GLYPH_WALKER_DEAD
    );
    lines.push(clip(&legend, cols));

    lines.truncate(rows);
    lines.join("\n")
}

fn clip(s: &str, cols: usize) -> String {
    if s.chars().count() <= cols {
        s.to_string()
    } else {
        s.chars().take(cols).collect()
    }
}

// ---------------------------------------------------------------------------
// the animation loop
// ---------------------------------------------------------------------------

/// Per-generation numbers the frame needs but the runners do not own.
struct GenCtx<'a> {
    generation: u32,
    generations: usize,
    pop_size: usize,
    wins: usize,
    genius: bool,
    history: &'a [HistoryPoint],
    deaths: &'a [(f32, f32, u32)],
    note: Option<&'a str>,
}

/// Frames a generation is allowed to draw at this speed: enough to watch the whole
/// episode, plus slack so a survivor cannot hold the view.
fn frame_budget(level_max_frames: u32, speed: u32) -> usize {
    (level_max_frames as usize).div_ceil(speed.max(1) as usize) + SLACK_FRAMES
}

/// Step every lane to its end (or until the human does something), drawing as it
/// goes. Returns when the lanes are finished, the viewer skips or quits, or the
/// headless frame cap is reached.
fn animate_lanes(
    screen: &mut Screen,
    level: &Level,
    runners: &mut [EpisodeRunner<'_>],
    lanes: &[LaneInfo],
    ctx: &GenCtx<'_>,
    live: &mut LiveState,
) -> io::Result<()> {
    debug_assert_eq!(runners.len(), lanes.len());
    let level_max = runners.first().map(|r| r.sim().max_frames()).unwrap_or(1);
    // The budget counts *stepped* frames, not drawn ones: while paused the animation
    // must hold still, and a generation that burned its budget while the viewer was
    // looking away would jump forward the moment they unpaused.
    let mut stepped = 0usize;

    loop {
        if screen.interactive {
            let mut skip = false;
            while event::poll(std::time::Duration::from_millis(0))? {
                let Ok(Event::Key(key)) = event::read() else {
                    continue;
                };
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if apply_key(key_action(key.code, key.modifiers), live) {
                    skip = true;
                }
            }
            if live.quit || skip {
                return Ok(());
            }
        }

        let stepping = !live.paused;
        if stepping {
            for runner in runners.iter_mut() {
                for _ in 0..live.speed {
                    if !runner.is_finished() {
                        runner.step();
                    }
                }
            }
        }
        if stepping {
            stepped += 1;
        }
        let budget = frame_budget(level_max, live.speed);

        draw_state(
            screen,
            level,
            runners,
            lanes,
            ctx,
            live,
            Progress {
                index: stepped,
                budget,
            },
        )?;

        if screen.budget_exhausted() || runners.iter().all(|r| r.is_finished()) || stepped >= budget {
            return Ok(());
        }
        // Wall-clock pacing for human eyes only, exactly like `play`: the sim has no
        // notion of time and determinism never depends on this sleep. When paused it
        // still redraws, or a keypress would appear to do nothing.
        sleep(FRAME_TIME);
    }
}

/// Compose and draw one frame from the current runner state.
fn draw_state(
    screen: &mut Screen,
    level: &Level,
    runners: &[EpisodeRunner<'_>],
    lanes: &[LaneInfo],
    ctx: &GenCtx<'_>,
    live: &LiveState,
    progress: Progress,
) -> io::Result<()> {
    let (cols, rows) = screen.size();
    // Stall and death-cluster are derived here rather than passed in: they are a
    // pure function of the history and the death window the frame already carries,
    // so there is exactly one place that can disagree about them.
    let stall = fitness_stall(ctx.history);
    let cluster = stall.and_then(|_| death_cluster(ctx.deaths, level.width));
    let l = layout(rows, ctx.genius, ctx.note.is_some(), stall.is_some());

    // Lane bodies, and the walkers of the lane the camera is on. Walkers come from
    // exactly one lane: eight walker sprites overlapping in a smear would be a worse
    // lie than showing the leader's, which is the lane the viewport follows.
    let mut lane_bodies: Vec<Body> = Vec::with_capacity(runners.len());
    let mut alive_lanes = 0usize;
    let mut leader: Option<usize> = None;
    let mut leader_x = f32::NEG_INFINITY;
    for (i, runner) in runners.iter().enumerate() {
        let sim = runner.sim();
        let player = sim.player();
        let outcome = sim.outcome();
        let finished = outcome.is_finished();
        if !finished {
            alive_lanes += 1;
        }
        // A winner is still standing on the flag, so it counts as alive for draw
        // order; only a death goes underneath.
        let body_alive = !(finished && outcome != Outcome::Won);
        lane_bodies.push(Body::new(
            player.x,
            player.y,
            player.w,
            player.h,
            lane_glyph(lanes[i].label, finished, outcome),
            body_alive,
        ));
        let center_x = player.x + player.w * 0.5;
        if body_alive && center_x > leader_x {
            leader_x = center_x;
            leader = Some(i);
        }
    }

    let mut bodies: Vec<Body> = Vec::with_capacity(lane_bodies.len() + 4);
    // The champion (lane 0) goes first so `Scene::primary()` — the body drawn last,
    // on top of everything — is the agent the view is following.
    if let Some(first) = lane_bodies.first() {
        bodies.push(*first);
    }
    let walker_lane = leader.unwrap_or(0).min(runners.len().saturating_sub(1));
    if let Some(runner) = runners.get(walker_lane) {
        for walker in runner.sim().walkers() {
            bodies.push(Body::new(
                walker.x,
                walker.y,
                walker.w,
                walker.h,
                if walker.alive {
                    GLYPH_WALKER_ALIVE
                } else {
                    GLYPH_WALKER_DEAD
                },
                walker.alive,
            ));
        }
    }
    bodies.extend(lane_bodies.iter().skip(1));

    let (cx, cy) = match leader {
        Some(i) => {
            let p = runners[i].sim().player();
            (p.x + p.w * 0.5, p.y + p.h * 0.5)
        }
        None => (level.width as f32 * 0.5, level.height as f32 * 0.5),
    };
    // "Whole level" is the same camera rule with the centre pinned to the middle of
    // the level, so it is a genuine whole-level view whenever the level fits.
    let (cx, cy) = if live.follow {
        (cx, cy)
    } else {
        (level.width as f32 * 0.5, level.height as f32 * 0.5)
    };
    let view = Viewport::following_body(level, cx, cy, cols as u16, l.scene_rows as u16);

    let scene = Scene::new(level, bodies);
    let scene_lines: Vec<String> = render_scene(&scene, view)
        .lines()
        .map(|s| s.to_string())
        .collect();

    let frame = Frame {
        level_name: &level.name,
        level_width_tiles: level.width,
        generation: ctx.generation,
        generations: ctx.generations,
        alive: alive_lanes,
        lane_count: lanes.len(),
        pop_size: ctx.pop_size,
        wins: ctx.wins,
        live: *live,
        history: ctx.history,
        deaths: ctx.deaths,
        scene_lines: &scene_lines,
        note: ctx.note,
        genius: ctx.genius,
        stall,
        cluster,
        progress,
    };
    screen.draw(&render_frame(&frame, cols, rows, l))
}

// ---------------------------------------------------------------------------
// champion mode
// ---------------------------------------------------------------------------

/// Watch one saved champion, tick by tick, exactly as it was recorded.
fn watch_champion(
    screen: &mut Screen,
    level: &Level,
    path: &str,
    live: &mut LiveState,
) -> Result<ChampionFile, String> {
    // `load_for_level` checks the level hash, the sim/sensor/fitness versions and
    // the action repeat: watching a champion on the wrong level would be a
    // meaningless animation, so it is refused rather than shown.
    let champion = ChampionFile::load_for_level(path, level)?;
    let genome = champion.to_genome()?;
    let eval = EvalConfig {
        action_repeat: champion.metadata.action_repeat.max(1),
    };
    let lanes = vec![LaneInfo {
        label: '1',
        best: true,
        genome_id: champion.metadata.genome_id,
        rank: 0,
        fitness: champion.fitness,
        mean_fitness: champion.fitness,
    }];

    println!("champion {path}");
    println!("         {}", champion.summary());
    println!(
        "watching it replay seed {} on {}",
        champion.metadata.episode_seed, level.name
    );

    let history = vec![HistoryPoint {
        generation: champion.metadata.generation,
        max_fitness: champion.fitness,
        avg_fitness: champion.fitness,
        wins: usize::from(champion.metadata.outcome == "won"),
    }];
    let ctx = GenCtx {
        generation: champion.metadata.generation,
        generations: 1,
        pop_size: 1,
        wins: history[0].wins,
        genius: false,
        history: &history,
        deaths: &[],
        note: None,
    };
    let mut runners = vec![EpisodeRunner::new(
        level,
        &genome,
        &eval,
        champion.metadata.episode_seed,
    )];
    animate_lanes(screen, level, &mut runners, &lanes, &ctx, live)
        .map_err(|e| format!("cannot draw the champion replay: {e}"))?;
    let result = runners.pop().expect("exactly one lane").result();
    println!(
        "replayed {} after {} frames, fitness {:.2}, best_x {:.1}",
        result.outcome.as_str(),
        result.frames,
        result.fitness,
        result.best_x
    );
    match replay_verdict(&result, &champion) {
        ReplayVerdict::Verified => println!("the animation matches the recorded episode"),
        ReplayVerdict::Mismatch { watched, recorded } => eprintln!(
            "warning: the animation scored {watched:.2} but the champion records {recorded:.2}; \
             this champion does not reproduce"
        ),
        ReplayVerdict::Truncated { played } => println!(
            "stopped after {played} of {} frames, so there is nothing to compare \
             (let it run to the end, or drop --frames)",
            champion.metadata.frames
        ),
    }
    Ok(champion)
}

/// How a watched champion episode compares with the champion's own record.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ReplayVerdict {
    /// Played to the end and scored what the file records.
    Verified,
    /// Played to the end and scored something else: a real mismatch.
    Mismatch { watched: f32, recorded: f32 },
    /// The viewer stopped the animation early (`q`, or a `--frames` cap). A partial
    /// episode has a partial score, and calling that a mismatch would be a false
    /// alarm about a champion that is perfectly fine — which is exactly what
    /// `watch --champion --frames N` would have done by default.
    Truncated { played: u32 },
}

fn replay_verdict(result: &evo::EpisodeResult, champion: &ChampionFile) -> ReplayVerdict {
    if result.outcome == Outcome::Running {
        return ReplayVerdict::Truncated {
            played: result.frames,
        };
    }
    if (result.fitness - champion.fitness).abs() > 1e-3 {
        ReplayVerdict::Mismatch {
            watched: result.fitness,
            recorded: champion.fitness,
        }
    } else {
        ReplayVerdict::Verified
    }
}

// ---------------------------------------------------------------------------
// saving, and the sign-off
// ---------------------------------------------------------------------------

fn save_champion(
    trainer: &Trainer,
    level: &Level,
    stem: &str,
    out_dir: &str,
) -> Result<String, String> {
    let best = trainer
        .best_ever()
        .ok_or_else(|| "no generation has been evaluated yet".to_string())?;
    let champion = ChampionFile::new(
        &best.genome,
        &best.result,
        level,
        best.generation,
        &trainer.config,
    );
    std::fs::create_dir_all(out_dir).map_err(|e| format!("cannot create {out_dir}: {e}"))?;
    let path = Path::new(out_dir).join(format!("{stem}.json"));
    champion.save(&path)?;
    Ok(format!(
        "saved champion gen {} fitness {:.2} -> {}",
        best.generation,
        best.result.fitness,
        path.display()
    ))
}

fn finish_training(
    screen: &mut Screen,
    level: &Level,
    stem: &str,
    trainer: &Trainer,
    saved: Option<String>,
    out_dir: &str,
) -> Result<(), String> {
    // Restore the terminal before printing, or the summary is written into the
    // alternate screen and vanishes with it.
    screen.close();
    println!();
    println!("watched {} ({} generations)", level.name, trainer.history().len());
    if let Some(best) = trainer.best_ever() {
        println!(
            "best    gen {} id {} fitness {:.2} ({} after {} frames, best_x {:.1})",
            best.generation,
            best.genome.id,
            best.result.fitness,
            best.result.outcome.as_str(),
            best.result.frames,
            best.result.best_x
        );
    }
    if let Some(row) = trainer.history().last() {
        println!(
            "last    gen {} max {:.2} avg {:.2} wins {}/{}",
            row.generation, row.max_fitness, row.avg_fitness, row.wins, trainer.config.pop_size
        );
    }
    match &saved {
        Some(note) => println!("{note}"),
        None => println!("nothing saved: no generation was scored"),
    }
    if saved.is_some() {
        let dir = out_dir.trim_end_matches('/');
        println!("hint    `train-cli replay --champion {dir}/{stem}.json` re-verifies it");
    }
    Ok(())
}

fn finish_champion(screen: &mut Screen, level: &Level, champion: &ChampionFile) -> Result<(), String> {
    screen.close();
    println!();
    println!("watched champion {} on {}", champion.metadata.genome_id, level.name);
    println!("        {}", champion.summary());
    Ok(())
}

fn oracle_line(level: &Level) -> String {
    let report = sim::reach::report(level);
    if report.goal_reached {
        "beatable in principle".to_string()
    } else {
        format!(
            "NOT beatable as built: {}",
            report.diagnosis.as_deref().unwrap_or("no explanation available")
        )
    }
}

// ---------------------------------------------------------------------------
// the terminal
// ---------------------------------------------------------------------------

/// The drawing surface: the terminal's alternate screen, or plain stdout.
///
/// `Drop` restores the terminal even on panic or an early `?`, which is the whole
/// reason this is a type rather than a pair of calls.
struct Screen {
    interactive: bool,
    drawn: usize,
    cap: Option<usize>,
}

impl Screen {
    fn open(headless: bool, cap: Option<usize>) -> io::Result<Screen> {
        let interactive = !headless && io::stdout().is_terminal();
        if interactive {
            enable_raw_mode()?;
            execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        } else {
            println!("watch: no interactive terminal, drawing plain text frames (no keyboard)");
        }
        Ok(Screen {
            interactive,
            drawn: 0,
            cap,
        })
    }

    fn size(&self) -> (usize, usize) {
        if self.interactive {
            match terminal::size() {
                Ok((cols, rows)) => usable_size(cols, rows),
                Err(_) => (HEADLESS_COLS, HEADLESS_ROWS),
            }
        } else {
            (HEADLESS_COLS, HEADLESS_ROWS)
        }
    }

    fn budget_exhausted(&self) -> bool {
        self.cap.is_some_and(|cap| self.drawn >= cap)
    }

    fn draw(&mut self, frame: &str) -> io::Result<()> {
        self.drawn += 1;
        let (cols, _) = self.size();
        let mut stdout = io::stdout();
        if self.interactive {
            execute!(stdout, MoveTo(0, 0), Clear(ClearType::FromCursorDown))?;
        } else {
            println!("--- frame {} ---", self.drawn);
        }
        for line in frame.lines() {
            writeln!(stdout, "{}", clip(line, cols))?;
        }
        stdout.flush()
    }

    /// Leave the alternate screen. Idempotent, so the sign-off and `Drop` can both
    /// call it.
    fn close(&mut self) {
        if self.interactive {
            let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            self.interactive = false;
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render::chart::{AVG_GLYPH, MAX_GLYPH};
    use std::path::{Path, PathBuf};

    fn a(v: &[&str]) -> Args {
        let owned: Vec<String> = v.iter().map(|s| s.to_string()).collect();
        Args::parse(&owned).ok().expect("parses")
    }

    /// `cargo test` runs with the CWD set to this package, not the workspace root.
    fn wpath(rel: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
            .to_string_lossy()
            .to_string()
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evo-watch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn a_lane(rank: usize, kind: LaneKind, id: u32) -> ShowcaseLane {
        ShowcaseLane {
            index: rank,
            rank,
            genome_id: id,
            kind,
            fitness: 100.0 - rank as f32,
            mean_fitness: 90.0 - rank as f32,
            seed: 7 + rank as u64,
        }
    }

    // -- keys -----------------------------------------------------------------

    #[test]
    fn every_documented_key_maps_to_an_action() {
        assert_eq!(
            key_action(KeyCode::Char(' '), KeyModifiers::NONE),
            KeyAction::TogglePause
        );
        assert_eq!(
            key_action(KeyCode::Char('+'), KeyModifiers::NONE),
            KeyAction::SpeedUp
        );
        assert_eq!(
            key_action(KeyCode::Char('='), KeyModifiers::SHIFT),
            KeyAction::SpeedUp
        );
        assert_eq!(
            key_action(KeyCode::Char('-'), KeyModifiers::NONE),
            KeyAction::SpeedDown
        );
        assert_eq!(
            key_action(KeyCode::Char('n'), KeyModifiers::NONE),
            KeyAction::SkipGeneration
        );
        assert_eq!(
            key_action(KeyCode::Enter, KeyModifiers::NONE),
            KeyAction::SkipGeneration
        );
        assert_eq!(
            key_action(KeyCode::Char('f'), KeyModifiers::NONE),
            KeyAction::ToggleFollow
        );
        assert_eq!(
            key_action(KeyCode::Char('s'), KeyModifiers::NONE),
            KeyAction::Save
        );
        assert_eq!(
            key_action(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyAction::Quit
        );
        assert_eq!(key_action(KeyCode::Esc, KeyModifiers::NONE), KeyAction::Quit);
        assert_eq!(
            key_action(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyAction::Quit
        );
        assert_eq!(key_action(KeyCode::F(7), KeyModifiers::NONE), KeyAction::None);
    }

    #[test]
    fn keys_drive_the_live_state() {
        let mut live = LiveState::default();
        assert!(!apply_key(
            key_action(KeyCode::Char(' '), KeyModifiers::NONE),
            &mut live
        ));
        assert!(live.paused);
        assert!(!apply_key(
            key_action(KeyCode::Char(' '), KeyModifiers::NONE),
            &mut live
        ));
        assert!(!live.paused);

        assert!(!apply_key(
            key_action(KeyCode::Char('f'), KeyModifiers::NONE),
            &mut live
        ));
        assert!(!live.follow);

        assert!(apply_key(
            key_action(KeyCode::Char('n'), KeyModifiers::NONE),
            &mut live
        ));
        assert!(!live.quit, "skipping a generation is not quitting");

        assert!(!apply_key(
            key_action(KeyCode::Char('s'), KeyModifiers::NONE),
            &mut live
        ));
        assert!(live.save_requested);

        apply_key(
            key_action(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut live,
        );
        assert!(live.quit);
    }

    #[test]
    fn the_speed_ladder_tops_out_at_50x_and_floors_at_1x() {
        assert_eq!(next_speed_up(1), 2);
        assert_eq!(next_speed_up(50), 50, "cannot exceed the documented range");
        assert_eq!(next_speed_down(1), 1, "cannot stall the animation at zero");
        assert_eq!(next_speed_down(50), 32);

        let mut live = LiveState::default();
        for _ in 0..20 {
            live.step_speed(true);
        }
        assert_eq!(live.speed, 50);
        for _ in 0..20 {
            live.step_speed(false);
        }
        assert_eq!(live.speed, 1);
    }

    #[test]
    fn an_arbitrary_speed_snaps_onto_the_ladder() {
        assert_eq!(nearest_speed(0), 1);
        assert_eq!(nearest_speed(1), 1);
        assert_eq!(nearest_speed(3), 2);
        assert_eq!(nearest_speed(4), 4);
        assert_eq!(nearest_speed(12), 8);
        assert_eq!(nearest_speed(999), 50);
        // Every ladder entry must be reachable by keys, or `--speed` could set a
        // value the viewer can never get back to.
        for s in SPEEDS {
            assert_eq!(nearest_speed(s), s);
        }
    }

    #[test]
    fn the_frame_budget_covers_a_whole_episode_at_the_chosen_speed() {
        // At 1x, a 5 000-tick episode needs at least 5 000 frames; at 50x, 100.
        assert_eq!(frame_budget(5_000, 1), 5_000 + SLACK_FRAMES);
        assert_eq!(frame_budget(5_000, 50), 100 + SLACK_FRAMES);
        // Faster viewing is never allowed to shrink the budget below the episode.
        assert!(frame_budget(5_000, 50) < frame_budget(5_000, 1));
        // A degenerate speed must not divide by zero or return nonsense.
        assert_eq!(frame_budget(10, 0), 10 + SLACK_FRAMES);
    }

    // -- lanes ----------------------------------------------------------------

    #[test]
    fn lane_labels_are_one_character_and_distinct() {
        // The ranks are the ones `Trainer::showcase` actually produces: random lanes
        // are numbered `best_k + n`, i.e. 4, 5, 6, 7 behind four best lanes.
        let lanes = [
            a_lane(0, LaneKind::Best, 10),
            a_lane(1, LaneKind::Best, 11),
            a_lane(4, LaneKind::Random, 12),
            a_lane(5, LaneKind::Random, 13),
            a_lane(6, LaneKind::Random, 14),
            a_lane(7, LaneKind::Random, 15),
        ];
        let info = lane_info(&lanes);
        assert_eq!(
            info.iter().map(|l| l.label).collect::<Vec<char>>(),
            vec!['1', '2', 'a', 'b', 'c', 'd']
        );
        let unique: std::collections::BTreeSet<char> = info.iter().map(|l| l.label).collect();
        assert_eq!(unique.len(), info.len());
        assert!(info.iter().all(|l| l.label.is_ascii_graphic()));
        // The champion carries the id and fitness the report gave it, not a copy.
        assert_eq!(info[0].genome_id, 10);
        assert_eq!(info[0].fitness, 100.0);
        assert_eq!(info[0].mean_fitness, 90.0);
        assert!(info[0].best && !info[2].best);
        // The rank is kept as data, but it is not what the label is derived from.
        assert_eq!(info[2].rank, 4);
    }

    #[test]
    fn no_lane_label_can_be_mistaken_for_an_enemy_or_a_mark() {
        // What matters is the glyphs drawn in the *same region* as the lanes: walkers
        // (`e` alive, `x` dead), the `+` a dead lane becomes, and the goal. The chart
        // glyphs live above the level view, so sharing a letter with them is fine.
        let reserved = [GLYPH_WALKER_ALIVE, GLYPH_WALKER_DEAD, '+', '!'];
        for ordinal in 0..40 {
            for kind in [LaneKind::Best, LaneKind::Random] {
                let label = lane_label(kind, ordinal);
                assert!(label.is_ascii_graphic(), "{label:?} is not drawable");
                assert!(
                    !reserved.contains(&label),
                    "lane ordinal {ordinal} of {kind:?} collides with a scene glyph"
                );
            }
        }
        // A 26-lane random group still never reaches `e` or `x`, and wraps instead
        // of panicking.
        let many: Vec<ShowcaseLane> = (0..30)
            .map(|i| a_lane(i, LaneKind::Random, i as u32))
            .collect();
        let info = lane_info(&many);
        assert_eq!(info.len(), 30);
        assert!(!info.iter().any(|l| l.label == 'e' || l.label == 'x'));
    }

    #[test]
    fn a_dead_lane_is_drawn_as_a_grave_and_a_winner_keeps_its_identity() {
        assert_eq!(lane_glyph('3', false, Outcome::Running), '3');
        assert_eq!(lane_glyph('3', true, Outcome::Won), '3');
        assert_eq!(lane_glyph('3', true, Outcome::Died), '+');
        assert_eq!(lane_glyph('c', true, Outcome::Stuck), '+');
        assert_eq!(lane_glyph('c', true, Outcome::TimedOut), '+');
    }

    // -- layout and frame -----------------------------------------------------

    fn history(n: u32) -> Vec<HistoryPoint> {
        (0..n)
            .map(|i| HistoryPoint {
                generation: i,
                max_fitness: 20.0 + i as f32 * 40.0,
                avg_fitness: 10.0 + i as f32 * 12.0,
                wins: 0,
            })
            .collect()
    }

    fn frame(cols: usize, rows: usize, genius: bool, note: Option<&str>) -> String {
        frame_with(cols, rows, genius, note, None)
    }

    /// A frame with an optional stall built from `history(9)` plus `extra` flat
    /// generations, which is how a stalled run actually looks: the history keeps
    /// growing while the best stops moving.
    fn stalled_frame(cols: usize, rows: usize, flat: u32) -> String {
        let mut h = history(9);
        let best = h.last().expect("non-empty").max_fitness;
        let start = h.len() as u32;
        for i in 0..flat {
            h.push(HistoryPoint {
                generation: start + i,
                max_fitness: best,
                avg_fitness: best * 0.5,
                wins: 0,
            });
        }
        frame_with(cols, rows, false, None, Some(h))
    }

    /// `override_history` rather than `history`: the test module has a `history(n)`
    /// helper, and a parameter of that name would shadow it into an `Option`.
    fn frame_with(
        cols: usize,
        rows: usize,
        genius: bool,
        note: Option<&str>,
        override_history: Option<Vec<HistoryPoint>>,
    ) -> String {
        let level = load_level(&wpath("levels/l3-gauntlet.json")).expect("level loads");
        let owned = override_history.unwrap_or_else(|| history(9));
        let deaths = vec![(10.0, 12.0, 3u32), (10.4, 12.0, 4), (44.0, 12.0, 5)];
        let stall = fitness_stall(&owned);
        let cluster = stall.and_then(|_| death_cluster(&deaths, level.width));
        let stalled = stall.is_some();
        let scene: Vec<String> = (0..layout(rows, genius, note.is_some(), stalled).scene_rows)
            .map(|i| format!("scene row {i}"))
            .collect();
        let f = Frame {
            level_name: &level.name,
            level_width_tiles: level.width,
            generation: 3,
            generations: 100,
            alive: 6,
            lane_count: 8,
            pop_size: 200,
            wins: 1,
            live: LiveState::default(),
            history: &owned,
            deaths: &deaths,
            scene_lines: &scene,
            note,
            genius,
            stall,
            cluster,
            progress: Progress {
                index: 120,
                budget: 675,
            },
        };
        render_frame(&f, cols, rows, layout(rows, genius, note.is_some(), stalled))
    }

    #[test]
    fn the_stall_line_names_the_stall_and_the_death_cluster() {
        let text = stalled_frame(100, 34, CUTOFF);
        assert!(
            text.contains("no new best in 25 generations (best"),
            "a stalled run must say so: {text}"
        );
        assert!(
            text.contains("deaths cluster at x 10 (2 in 3)"),
            "x=10 and x=10.4 are the same tile, so they are one cluster of two: {text}"
        );
        // The reading is the stall, not the tail's shape: an improvement 34
        // generations ago must not still be captioned as learning.
        assert!(!text.contains("learning: max and avg both rising"));
        assert!(text.contains("stalled: no new best in a long time"));
        // The extra row comes out of the chart and the scene, like the genius
        // splash and the save note: the frame keeps the height it was given.
        assert_eq!(text.lines().count(), 34);
        // And with no deaths in the window the row does not invent a cluster to
        // report — the strip below already shows an empty row.
        let no_deaths = frame_with_stall(100, 34, &[]);
        assert!(no_deaths.contains("no new best in 25 generations (best"));
        assert!(!no_deaths.contains("deaths cluster"));
    }

    /// A stalled frame with an explicit death window, for the no-deaths case.
    fn frame_with_stall(cols: usize, rows: usize, deaths: &[(f32, f32, u32)]) -> String {
        let level = load_level(&wpath("levels/l3-gauntlet.json")).expect("level loads");
        let mut h = history(9);
        let best = h.last().expect("non-empty").max_fitness;
        for _ in 0..CUTOFF {
            h.push(HistoryPoint {
                generation: h.len() as u32,
                max_fitness: best,
                avg_fitness: best * 0.5,
                wins: 0,
            });
        }
        let stall = fitness_stall(&h).expect("stalled");
        let cluster = death_cluster(deaths, level.width);
        assert!(cluster.is_none(), "this helper is for the empty window");
        let scene: Vec<String> = vec!["scene".to_string()];
        let f = Frame {
            level_name: &level.name,
            level_width_tiles: level.width,
            generation: 30,
            generations: 100,
            alive: 8,
            lane_count: 8,
            pop_size: 200,
            wins: 0,
            live: LiveState::default(),
            history: &h,
            deaths,
            scene_lines: &scene,
            note: None,
            genius: false,
            stall: Some(stall),
            cluster,
            progress: Progress {
                index: 1,
                budget: 60,
            },
        };
        render_frame(&f, cols, rows, layout(rows, false, false, true))
    }

    /// The stall cutoff, read from the chart module so the watch view and this test
    /// cannot disagree about it while both passing.
    const CUTOFF: u32 = render::chart::STALL_GENERATIONS;

    #[test]
    fn the_stall_line_waits_for_the_cutoff_and_clears_on_an_improvement() {
        let early = stalled_frame(100, 34, CUTOFF - 1);
        assert!(
            !early.contains("stalled"),
            "{CUTOFF} flat generations is the cutoff, so {} is not a stall yet",
            CUTOFF - 1
        );
        let late = stalled_frame(100, 34, CUTOFF);
        assert!(late.contains("stalled"), "one more generation is");

        // An improvement resets the clock, even after a long flat stretch.
        let mut h = history(9);
        let best = h.last().expect("non-empty").max_fitness;
        for _ in 0..CUTOFF {
            h.push(HistoryPoint {
                generation: h.len() as u32,
                max_fitness: best,
                avg_fitness: best * 0.5,
                wins: 0,
            });
        }
        let stalled = frame_with(100, 34, false, None, Some(h.clone()));
        assert!(stalled.contains("stalled"));
        h.push(HistoryPoint {
            generation: h.len() as u32,
            max_fitness: best * 2.0,
            avg_fitness: best,
            wins: 0,
        });
        let recovered = frame_with(100, 34, false, None, Some(h));
        assert!(
            !recovered.contains("stalled"),
            "a new best ends the stall: {recovered}"
        );
    }

    #[test]
    fn a_normal_terminal_is_filled_exactly_and_a_tiny_one_keeps_its_minimums() {
        for rows in [20usize, 24, 34, 60] {
            for genius in [false, true] {
                for note in [false, true] {
                    for stalled in [false, true] {
                        let l = layout(rows, genius, note, stalled);
                        let reserved = FIXED_ROWS
                            + usize::from(genius)
                            + usize::from(note)
                            + usize::from(stalled);
                        assert_eq!(
                            l.chart_rows + l.scene_rows + reserved,
                            rows,
                            "rows {rows} genius {genius} note {note} stalled {stalled} \
                             must use the whole screen"
                        );
                        assert!(l.chart_rows >= MIN_CHART_ROWS);
                        assert!(l.scene_rows >= MIN_SCENE_ROWS);
                    }
                }
            }
        }
        // Too short for everything: the chart and the level view keep their
        // minimums and the frame is truncated, rather than the level view shrinking
        // to nothing.
        for rows in [8usize, 12] {
            let l = layout(rows, false, false, false);
            assert!(l.chart_rows >= MIN_CHART_ROWS);
            assert!(l.scene_rows >= MIN_SCENE_ROWS);
            let text = frame(100, rows, false, None);
            assert!(text.lines().count() <= rows);
        }
    }

    #[test]
    fn the_frame_shows_every_tier_one_element() {
        let text = frame(100, 34, false, None);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.len() <= 34, "frame taller than the terminal");

        // Tier 1.1: generation counter and alive/total agents, on the first line.
        assert!(lines[0].starts_with("L3-gauntlet"), "{}", lines[0]);
        assert!(lines[0].contains("gen 3/100"), "{}", lines[0]);
        assert!(lines[0].contains("lanes 6/8 alive"), "{}", lines[0]);
        assert!(lines[0].contains("speed 8x"), "{}", lines[0]);

        // Tier 1.2: the numbers, the chart, and the reading in words.
        assert!(lines[1].contains("learning"), "{}", lines[1]);
        assert!(lines[1].contains("wins 1/200"), "{}", lines[1]);
        assert!(lines[1].contains("max"), "{}", lines[1]);
        assert!(
            lines.iter().any(|l| l.contains(MAX_GLYPH)),
            "the max series must be plotted"
        );
        assert!(
            lines.iter().any(|l| l.contains(AVG_GLYPH)),
            "the avg series must be plotted"
        );

        // Tier 1.4's controls, and Tier 1.5's best/random legend.
        assert!(lines.iter().any(|l| l.contains("space pause")));
        assert!(lines
            .iter()
            .any(|l| l.contains("best lanes") && l.contains("random lanes")));

        // The death strip spans the whole level width, one character per column of
        // the level, with a single death in a column drawn as ':' (2-3 '+', 4+ '#').
        let strip: Vec<char> = lines
            .iter()
            .find(|l| l.chars().count() == 100 && l.chars().all(|c| ".:+#".contains(c)))
            .expect("a full-width death strip")
            .chars()
            .collect();
        assert_eq!(strip.len(), 100);
        // Marks at level tiles 10.0, 10.4 and 44.0 of a 60-tile level map to columns
        // 16, 17 and 73, and nowhere else.
        let marked: Vec<usize> = strip
            .iter()
            .enumerate()
            .filter(|(_, c)| **c != '.')
            .map(|(i, _)| i)
            .collect();
        assert_eq!(marked, vec![16, 17, 73]);

        for line in &lines {
            assert!(line.chars().count() <= 100, "line too wide: {line:?}");
        }
    }

    #[test]
    fn a_terminal_that_reports_no_size_falls_back_instead_of_drawing_nothing() {
        // `script`, a CI shell and a detached multiplexer pane all report 0 columns.
        // A zero-width frame is empty, so without this the screen stays blank and
        // watch mode looks hung.
        assert_eq!(usable_size(0, 0), (HEADLESS_COLS, HEADLESS_ROWS));
        assert_eq!(usable_size(0, 40), (HEADLESS_COLS, HEADLESS_ROWS));
        assert_eq!(usable_size(80, 0), (HEADLESS_COLS, HEADLESS_ROWS));
        assert_eq!(usable_size(120, 40), (120, 40));
        assert_eq!(usable_size(20, 12), (20, 12), "a small terminal is still a terminal");
    }
    #[test]
    fn a_paused_or_unfollowed_view_says_so() {
        let level = load_level(&wpath("levels/l1-flat-run.json")).expect("level loads");
        let h = history(5);
        let scene = vec!["row".to_string()];
        let f = Frame {
            level_name: &level.name,
            level_width_tiles: level.width,
            generation: 0,
            generations: 10,
            alive: 8,
            lane_count: 8,
            pop_size: 8,
            wins: 0,
            live: LiveState {
                paused: true,
                follow: false,
                speed: 50,
                ..LiveState::default()
            },
            history: &h,
            deaths: &[],
            scene_lines: &scene,
            note: None,
            genius: false,
            stall: None,
            cluster: None,
            progress: Progress {
                index: 1,
                budget: 60,
            },
        };
        let text = render_frame(&f, 80, 24, layout(24, false, false, false));
        assert!(text.contains("PAUSED"));
        assert!(text.contains("whole level"));
        assert!(text.contains("speed 50x"));
    }

    #[test]
    fn the_genius_splash_and_the_save_note_have_their_own_lines() {
        let text = frame(
            100,
            34,
            true,
            Some("saved champion gen 7 fitness 42.00 -> champions/l3.json"),
        );
        assert!(text.contains("GENIUS AGENT"));
        assert!(text.contains("saved champion gen 7"));
        // The extra lines come out of the chart and the scene, not out of the
        // terminal: the frame keeps its height either way.
        let plain = frame(100, 34, false, None);
        assert_eq!(plain.lines().count(), 34);
        assert_eq!(text.lines().count(), 34);
    }

    #[test]
    fn a_narrow_terminal_is_clipped_rather_than_wrapped() {
        for cols in [12usize, 24, 40, 61] {
            let text = frame(cols, 20, false, None);
            for line in text.lines() {
                assert!(
                    line.chars().count() <= cols,
                    "cols {cols}: {line:?} is {} wide",
                    line.chars().count()
                );
            }
        }
    }

    // -- the animation loop ---------------------------------------------------

    /// A one-lane generation, ready to animate: the smallest real call into
    /// [`animate_lanes`], with a headless screen capped at `frames` draws.
    fn animate_one(
        level: &Level,
        frames: usize,
        live: LiveState,
    ) -> (LiveState, u32, u32) {
        use evo::Genome;
        let cfg = GaConfig {
            pop_size: 4,
            master_seed: 1,
            ..GaConfig::default()
        };
        let eval = cfg.eval_config();
        let genome = Genome::zeros(0, cfg.shape.clone());
        let lanes = vec![LaneInfo {
            label: '1',
            best: true,
            genome_id: 0,
            rank: 0,
            fitness: 0.0,
            mean_fitness: 0.0,
        }];
        let history: Vec<HistoryPoint> = Vec::new();
        let ctx = GenCtx {
            generation: 0,
            generations: 10,
            pop_size: 4,
            wins: 0,
            genius: false,
            history: &history,
            deaths: &[],
            note: None,
        };
        let mut screen = Screen::open(true, Some(frames)).expect("headless screen");
        let mut live = live;
        let mut runners = vec![EpisodeRunner::new(level, &genome, &eval, 1)];
        animate_lanes(&mut screen, level, &mut runners, &lanes, &ctx, &mut live).expect("draws");
        (
            live,
            runners[0].sim().stats().frames,
            screen.drawn as u32,
        )
    }

    #[test]
    fn pausing_freezes_the_physics_and_stops_burning_the_frame_budget() {
        let level = load_level(&wpath("levels/l1-flat-run.json")).expect("level loads");
        let speed = LiveState {
            speed: 4,
            ..LiveState::default()
        };

        let (_, frames, drawn) = animate_one(&level, 3, speed);
        assert!(frames > 0, "an unpaused animation must step the sim");
        assert_eq!(frames, 3 * 4, "three frames at 4x is twelve ticks");
        assert_eq!(drawn, 3, "the headless cap ends the generation");

        // Paused: the same three draws, no physics at all. The budget must not be
        // consumed either, or unpausing would dump the viewer into the next
        // generation having seen nothing.
        let paused = LiveState { paused: true, ..speed };
        let (live, frames, drawn) = animate_one(&level, 3, paused);
        assert_eq!(frames, 0, "pause must freeze the physics");
        assert_eq!(drawn, 3);
        assert!(live.paused, "still paused after the loop: nothing resumed it");
    }

    // -- end to end -----------------------------------------------------------

    #[test]
    fn a_headless_run_watches_generations_and_saves_the_champion() {
        let dir = tmpdir("run");
        let level = wpath("levels/l2-first-gap.json");
        let out = dir.to_string_lossy().to_string();
        watch(&a(&[
            "watch",
            "--level",
            &level,
            "--generations",
            "2",
            "--pop",
            "16",
            "--frames",
            "2",
            "--seed",
            "5",
            "--out",
            &out,
        ]))
        .expect("a headless watch run works without a TTY");

        let champion = dir.join("l2-first-gap.json");
        assert!(champion.exists(), "watch must save the best-ever champion");
        let loaded = load_level(&level).expect("level");
        let champ = ChampionFile::load_for_level(&champion, &loaded).expect("champion loads");
        assert!(champ.metadata.generation < 2, "at most two generations ran");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truncated_champion_replay_is_not_reported_as_a_mismatch() {
        // `--frames` and `q` cut the animation short. A partial episode has a
        // partial score, and reporting that as a mismatch would send a user hunting
        // for a bug in a champion that reproduces perfectly.
        use evo::Genome;
        let level = load_level(&wpath("levels/l2-first-gap.json")).expect("level loads");
        let cfg = GaConfig {
            pop_size: 4,
            master_seed: 1,
            ..GaConfig::default()
        };
        let eval = cfg.eval_config();
        let genome = Genome::zeros(0, cfg.shape.clone());
        let full = evo::run_episode(&level, &genome, &eval, 7);
        let champion = ChampionFile::new(&genome, &full, &level, 0, &cfg);
        assert_eq!(replay_verdict(&full, &champion), ReplayVerdict::Verified);

        let mut runner = EpisodeRunner::new(&level, &genome, &eval, 7);
        for _ in 0..10 {
            runner.step();
        }
        let partial = runner.result();
        assert_eq!(partial.outcome, Outcome::Running);
        assert_eq!(
            replay_verdict(&partial, &champion),
            ReplayVerdict::Truncated {
                played: partial.frames
            }
        );

        // The same finished episode with a different score is a genuine mismatch.
        let mut wrong = full.clone();
        wrong.fitness += 1.0;
        assert_eq!(
            replay_verdict(&wrong, &champion),
            ReplayVerdict::Mismatch {
                watched: wrong.fitness,
                recorded: champion.fitness,
            }
        );
    }

    #[test]
    fn champion_mode_watches_a_saved_champion_and_refuses_a_mismatched_level() {
        let dir = tmpdir("champion");
        let out = dir.to_string_lossy().to_string();
        let level_path = wpath("levels/l2-first-gap.json");
        // Produce a champion with the ordinary trainer, then watch it.
        crate::commands::train(&a(&[
            "train",
            "--level",
            &level_path,
            "--generations",
            "1",
            "--pop",
            "8",
            "--seed",
            "3",
            "--out",
            &out,
            "--quiet",
        ]))
        .expect("training runs");
        let champion = dir.join("l2-first-gap.json").to_string_lossy().to_string();

        watch(&a(&[
            "watch",
            "--champion",
            &champion,
            "--level",
            &level_path,
            "--frames",
            "2",
            "--speed",
            "50",
        ]))
        .expect("watching a champion works");

        // Level omitted: recovered from the champion's own level hash.
        watch(&a(&[
            "watch",
            "--champion",
            &champion,
            "--levels-dir",
            &wpath("levels"),
            "--frames",
            "1",
        ]))
        .expect("the level is found by hash");

        // The wrong level is a refusal, not a silent mismatch: the whole point of a
        // champion is that it reproduces one specific level.
        let bad = watch(&a(&[
            "watch",
            "--champion",
            &champion,
            "--level",
            &wpath("levels/l3-gauntlet.json"),
            "--frames",
            "1",
        ]));
        assert!(
            bad.is_err(),
            "a champion must not be watched on another level"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_rejects_unknown_options_and_impossible_arguments() {
        assert!(watch(&a(&["watch", "--level", "x.json", "--poop", "1"])).is_err());
        assert!(watch(&a(&["watch"])).is_err());
        assert!(watch(&a(&["watch", "--level", "x.json", "--best", "0", "--random", "0"])).is_err());
        assert!(watch(&a(&["watch", "--level", "x.json", "--frames", "0"])).is_err());
        assert!(watch(&a(&["watch", "--level", "/nonexistent/level.json"])).is_err());
    }
}
