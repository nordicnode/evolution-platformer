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
    curriculum_step, practice_level_for_stall, practice_step, save_practice_level, ChampionFile,
    EpisodeRunner, EvalConfig, GaConfig, Genome, LaneKind, PopulationFile, ShowcaseLane, Trainer,
    DEFAULT_CURRICULUM_GENERATIONS, MUTATION_RATES,
};
use render::ascii::{GLYPH_WALKER_ALIVE, GLYPH_WALKER_DEAD};
use render::chart::{
    death_cluster, death_strip, fitness_chart, fitness_stall, fitness_trend, format_fitness,
    progress_bar, Stall,
};
use render::{
    render_scene, Body, ChampionTrace, HistoryPoint, Scene, TraceLog, TracePoint, Viewport,
    FRAME_TIME, TRACE_SAMPLE_FRAMES,
};
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
    "fine-children",
    "jump-timing-share",
    "repair-children",
    "commitment",
    "mastery",
    "no-mastery",
    "stagnation",
    "immigrants",
    "reheat-factor",
    "growth-units",
    "growth-events",
    "adaptive-sigma",
    "action-repeat",
    "feedback",
    "hidden",
    "rank-pessimism",
    "trials",
    "no-auto-trials",
    "common-episodes",
    "no-common-episodes",
    "race-episodes",
    "no-race-episodes",
    "out",
    "telemetry",
    "speed",
    "best",
    "random",
    "death-window",
    "frames",
    "headless",
    "curriculum",
    "curriculum-generations",
    "resume",
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
        return Err(
            "usage: --best and --random cannot both be zero: nothing would be drawn".into(),
        );
    }
    let death_window = usage(a.usize_opt("death-window", 25))? as u32;
    let out_dir = a.string("out", "champions");

    // Curriculum and resume (§12 M5, finding 22). The easier level is loaded up
    // front so a typo is a usage error at startup rather than a surprise forty
    // generations into a stalled run.
    let resume_path = a.opt("resume").map(|s| s.to_string());
    let curriculum_level = match a.opt("curriculum") {
        Some(path) => Some(load_level(path)?),
        None => None,
    };
    let curriculum_generations = usage(a.usize_opt(
        "curriculum-generations",
        DEFAULT_CURRICULUM_GENERATIONS as usize,
    ))? as u32;
    if curriculum_level.is_some() && curriculum_generations == 0 {
        return Err("usage: --curriculum-generations must be at least 1".into());
    }
    // Champion mode replays one saved genome and never trains, so both of these
    // would be accepted and then quietly do nothing.
    if champion_path.is_some() && (curriculum_level.is_some() || resume_path.is_some()) {
        return Err(
            "usage: --curriculum and --resume train a population; --champion replays one genome"
                .into(),
        );
    }

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

    let mut cfg = GaConfig {
        pop_size,
        elite: usage(a.usize_opt("elite", 4))?,
        tournament: usage(a.usize_opt("tournament", 4))?,
        // Snapped to the same ladder the `m`/`M` keys walk, so the rate the run
        // starts at is a rate the viewer can get back to and the value on screen is
        // always the value in the config (§10 Tier 2.9).
        mutation_rate: nearest_mut_rate(usage(
            a.f32_opt("mut-rate", GaConfig::default().mutation_rate),
        )?),
        mutation_sigma: usage(a.f32_opt("mut-sigma", 0.30))?,
        fine_children: usage(a.usize_opt("fine-children", 8))?,
        jump_timing_share: usage(
            a.f32_opt("jump-timing-share", GaConfig::default().jump_timing_share),
        )?,
        // Item 14: repair children bred from the best genome's own losing decisions. The default
        // lives in the config, as for `train`, so a watched run and a trained one agree.
        repair_children: usage(a.usize_opt(
            "repair-children",
            GaConfig::default().repair_children,
        ))?,
        // The same default as `train` and the window: after the record solves, the second phase
        // ranks winners by the clock. `--no-mastery` is the plain run.
        mastery: !a.has_flag("no-mastery"),
        adaptive_sigma: if a.has_flag("adaptive-sigma") {
            true
        } else {
            GaConfig::default().adaptive_sigma
        },
        stagnation_generations: usage(a.usize_opt("stagnation", 20))? as u32,
        immigrants: usage(a.usize_opt("immigrants", 8))?,
        // As in `train`: the default lives in the config, not in a second copy here.
        reheat_factor: usage(a.f32_opt("reheat-factor", GaConfig::default().reheat_factor))?,
        crossover_rate: usage(a.f32_opt("crossover", 0.75))?,
        // And the ranking rule, for the same reason: a watched run and a trained run with the same
        // flags have to be the same run, or the panel is showing numbers no table can quote.
        rank_pessimism: usage(a.f32_opt("rank-pessimism", GaConfig::default().rank_pessimism))?,
        action_repeat: usage(a.usize_opt("action-repeat", 4))? as u32,
        // As in `train`: the width of the recurrent state and the capacity of the hidden layers,
        // both defaulted from the trainer's own shape rather than from a second copy here, and both
        // clamped by the trainer. The header line below prints the net a viewer is watching, which
        // is the point of watching a capacity sweep rather than reading its table afterwards.
        shape: evo::NetShape {
            hidden: crate::commands::parse_hidden(a, &GaConfig::default().shape.hidden)?,
            feedback: usage(a.usize_opt("feedback", GaConfig::default().shape.feedback))?,
            commitment: a.has_flag("commitment"),
            ..GaConfig::default().shape
        },
        // As in `train`: the patrol rule lives in the trainer (`GaConfig::effective_trials`), so
        // watching a run judges it the way training it does — which the window, this mode and the
        // API all get from the same place.
        trials: usage(a.usize_opt("trials", GaConfig::default().trials as usize))? as u32,
        auto_trials: !a.has_flag("no-auto-trials"),
        // The episode policy, so a watched run is the run `train` would have made with the same
        // flags: pairing is what makes the patrol phase in the panel a *common* difficulty rather
        // than a per-lane draw, and a panel showing unpaired lanes beside a paired table would be
        // showing two different searches.
        common_episodes: if a.has_flag("common-episodes") {
            true
        } else if a.has_flag("no-common-episodes") {
            false
        } else {
            GaConfig::default().common_episodes
        },
        race_episodes: if a.has_flag("race-episodes") {
            true
        } else if a.has_flag("no-race-episodes") {
            false
        } else {
            GaConfig::default().race_episodes
        },
        master_seed: usage(a.u64_opt("seed", 1))?,
        ..GaConfig::default()
    };
    let auto_shape = evo::apply_auto_shape(
        &mut cfg,
        std::slice::from_ref(&level),
        a.opt("feedback").is_some(),
        a.opt("rank-pessimism").is_some(),
    );
    live.mut_rate = cfg.mutation_rate;

    println!(
        "watching {} ({}x{})  pop {}  seed {}  lanes {}+{}  speed {}  mut {}{}",
        level.name,
        level.width,
        level.height,
        cfg.pop_size,
        cfg.master_seed,
        best_k,
        random_k,
        speed_label(live.speed),
        mut_label(live.mut_rate),
        if headless { "  [headless]" } else { "" }
    );
    println!(
        "sim {} / sensors v{} / fitness v{} / oracle: {}",
        sim::SIM_VERSION,
        sim::sensors::VERSION,
        evo::FITNESS_VERSION,
        oracle_line(&level)
    );
    if let Some(auto) = &auto_shape {
        println!("auto    {}", auto.line());
    }

    // A run started from a checkpoint is the plan's "restart from where it got
    // stuck": the population, its genome ids (part of every episode seed) and the
    // generation number all continue from the file.
    let mut trainer = match resume_path.as_deref() {
        Some(path) => {
            let checkpoint = PopulationFile::load(path)?;
            println!("resuming {}", checkpoint.summary());
            if let Some(note) = checkpoint.level_note(&level) {
                println!("note    {note}");
            }
            let mut trainer =
                Trainer::resume(cfg.clone(), checkpoint.to_genomes()?, checkpoint.generation)?;
            if let Some(record) = checkpoint.best_genome()? {
                trainer.seed_record_from_checkpoint(&level, record)?;
            }
            trainer
        }
        None => Trainer::new(cfg),
    };
    let eval: EvalConfig = trainer.config.eval_config();
    let pop_size = trainer.config.pop_size;
    let mut history: Vec<HistoryPoint> = Vec::new();
    let mut curriculum_taken: Option<String> = None;
    // The run's standing best when the last derived practice step fired: the gate that
    // lets a practice run take a step per *stall spell* rather than one per run, and
    // never two in the same spell.
    let mut detour_best = f32::NEG_INFINITY;
    let mut deaths: Vec<(f32, f32, u32)> = Vec::new();
    let mut traces = TraceLog::new();
    let mut note: Option<String> = None;
    let mut generations_run = 0usize;
    // The quickest win the run had recorded at the last generation, so a *new* one can be said out
    // loud. It moves silently otherwise: a generation that shaves a tenth of a second off the best
    // time moves no chart line by more than a pixel, and since fitness v8 the clock is what the
    // search is partly paid for — so the moment the record falls is a moment worth a line.
    let mut fastest_seen: Option<evo::FastWin> = None;

    // **Phase 0 learning telemetry.** A watched run is a real run, so it can leave the same
    // JSONL record a headless one does — one object per generation. The screen shows the same
    // readings in the desktop window's Details panel; the file is what makes a watched run's
    // curve checkable after the fact.
    let telemetry_path = a.opt("telemetry").map(|p| p.to_string());
    let mut telemetry_out = match &telemetry_path {
        Some(path) => {
            let file = std::fs::File::create(path)
                .map_err(|e| format!("cannot write {path}: {e}"))?;
            Some(std::io::BufWriter::new(file))
        }
        None => None,
    };

    for _ in 0..generations {
        if live.quit {
            break;
        }
        // Anything the viewer changed with the keyboard applies from this
        // generation on, so a rate change shows up in the very next offspring.
        apply_live_config(&mut trainer, &live);
        // The population snapshot is taken before breeding, so these are the exact
        // genomes (with the exact ids) the report describes.
        let (report, population) = trainer.step_verbose(&level);
        generations_run += 1;
        if let Some(out) = telemetry_out.as_mut() {
            let line = serde_json::to_string(&report.telemetry)
                .map_err(|e| format!("cannot serialize telemetry: {e}"))?;
            writeln!(out, "{line}")
                .map_err(|e| format!("cannot write {telemetry_path:?}: {e}"))?;
        }
        history.push(HistoryPoint {
            generation: report.generation,
            max_fitness: report.max_fitness,
            avg_fitness: report.avg_fitness,
            wins: report.wins,
        });

        let lanes = trainer.showcase(&report, &population, best_k, random_k);
        let info = lane_info(&lanes);

        // Record this generation's champion route when it is an improvement (Tier
        // 2.7). Asked *before* paying: the recording costs one extra episode, and a
        // stalled run — the case the scrubber exists for — improves on almost no
        // generation at all.
        if traces.would_record(report.max_fitness) {
            if let Some(lane) = lanes.first() {
                traces.observe(record_trace(
                    &level,
                    &eval,
                    &population[lane.index],
                    lane.seed,
                    report.generation,
                    report.max_fitness,
                ));
            }
        }
        let mut runners: Vec<EpisodeRunner> = lanes
            .iter()
            .map(|lane| {
                EpisodeRunner::new(&level, population[lane.index].clone(), &eval, lane.seed)
            })
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
            traces: &traces,
            // Read off the report the search just produced: the size these children were
            // bred with, the run's own staleness counter, how many immigrants that counter
            // bought, and how much of the behavioural archive the run is spread across.
            search: SearchState {
                sigma: report.mutation_sigma,
                stagnation: trainer.stagnation(),
                immigrants: report.immigrants,
                archive_children: report.archive_children,
                archive_bits: report.archive_bits,
                archive_time_bits: report.archive_time_bits,
            },
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

        // A quicker win than this run has ever had, said out loud. The note line is the view's own
        // instrument for a fact no column of scores can show, which is what a record falling is: the
        // stopwatch, not the fitness.
        let fastest_now = trainer.fastest();
        if fastest_now != fastest_seen {
            if let Some(fast) = fastest_now {
                note = Some(format!(
                    "*** QUICKEST WIN: {} ({} frames) at the flag (gen {} id {}, {:.0}% of the level's {} unspent) ***",
                    sim::format_time(fast.frames),
                    fast.frames,
                    fast.generation,
                    fast.genome_id,
                    fast.spare() * 100.0,
                    sim::format_time(fast.budget_frames)
                ));
            }
            fastest_seen = fastest_now;
        }

        // `s` saves the best-ever champion; handled between generations so the file
        // is written once per keypress and the note outlives the key.
        if live.save_requested {
            live.save_requested = false;
            note = Some(save_champion(&trainer, &level, &stem, &out_dir)?);
        }

        // The stall the view has just reported is the trigger for a curriculum step
        // (§12 M5: stall detector -> death-spot highlight -> curriculum resume). It
        // uses the *same* stall rule the status line shows, so the run cannot act on
        // a stall the viewer is not being told about.
        //
        // The authored easier level steps back once per run (the plan's rule). The
        // derived practice re-arms on every *new* stall spell — a practice run has to
        // work through a spot, keep the gain, and meet the next spot — and never
        // twice within one, so a detour that learns nothing cannot loop (finding 50).
        let armed = match &curriculum_level {
            Some(_) => curriculum_taken.is_none(),
            None => report.max_fitness > detour_best,
        };
        if armed {
            if let Some(stall) = fitness_stall(&history) {
                // The authored easier level is the run's first choice. Without one,
                // the run practises *where it is stuck* instead (finding 50): its own
                // level from the furthest stand its best genome has made, so every
                // episode of the detour is the next move instead of the whole chain.
                let practice;
                let easier: &Level = match &curriculum_level {
                    Some(easier) => easier,
                    None => {
                        let derived = match practice_level_for_stall(&trainer, &level) {
                            Ok(derived) => derived,
                            Err(reason) => {
                                // Nowhere to practise from is said out loud too: the
                                // stall line says the search stopped, this says why it
                                // is not stepping back. Once per stall spell — the arm
                                // closes until the run finds something better.
                                detour_best = report.max_fitness;
                                note = Some(format!(
                                    "curriculum: {reason}; the run stays where it is ({})",
                                    stall.label()
                                ));
                                continue;
                            }
                        };
                        practice = derived;
                        &practice
                    }
                };
                // The step itself is `evo`'s, because the window runs the same one:
                // a checkpoint and an excursion are the run's, not a front-end's.
                let path = Path::new(&out_dir).join(format!("{stem}.population.json"));
                let step = if curriculum_level.is_none() {
                    // The practice detour keeps the main level in its judgement, so
                    // what it learns comes back able to reach the spot it left.
                    save_practice_level(easier, &path)?;
                    practice_step(
                        &mut trainer,
                        &level,
                        easier,
                        curriculum_generations,
                        &path,
                        live.mut_rate,
                        &stall.label(),
                    )?
                } else {
                    curriculum_step(
                        &mut trainer,
                        &level,
                        easier,
                        curriculum_generations,
                        &path,
                        live.mut_rate,
                        &stall.label(),
                    )?
                };
                detour_best = report.max_fitness;
                let summary = step.note(&level.name);
                note = Some(summary.clone());
                // Every step keeps its line: a run that practised its way past three
                // spots says so three times, not once.
                curriculum_taken = Some(match curriculum_taken {
                    Some(prev) => format!("{prev}\n{summary}"),
                    None => summary,
                });
            }
        }
    }

    if let (Some(path), Some(mut out)) = (&telemetry_path, telemetry_out) {
        out.flush()
            .map_err(|e| format!("cannot write {path}: {e}"))?;
        println!("wrote   {path} (one learning record per generation)");
    }

    // A watched run is a real run: its best genome is saved exactly like `train`'s,
    // so watching instead of training wastes nothing.
    let saved = if generations_run > 0 {
        Some(save_champion(&trainer, &level, &stem, &out_dir)?)
    } else {
        None
    };
    finish_training(&mut screen, &level, &stem, &trainer, saved, &out_dir)?;
    // Printed after the summary, and always: a run whose shape changed mid-flight
    // should say so where the numbers are, not only in a frame the viewer may have
    // scrolled past.
    if let Some(summary) = &curriculum_taken {
        println!("{summary}");
    }
    Ok(())
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
            let champion = ChampionFile::from_json_str(&raw).map_err(|e| format!("{path}: {e}"))?;
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

/// Random-lane labels: the alphabet minus every glyph the scene draws.
///
/// A lane's letter is drawn in the rows the level is drawn in, so a label that is also a tile is a
/// lane that looks like terrain, and a lane that looks like an *enemy* is worse than no label at
/// all. [`reserved_glyphs`] is that set, and it is derived from [`sim::level`]'s own tile list
/// rather than written out here, because the hand-written version of this comment went stale the
/// moment two tiles arrived: the trap pass took `d`, `s`, `b`, `w` and `v` for brittle ground and
/// pads, and the alphabet below went on offering all five as agent names — with a guard test that
/// passed, since checking a lane label is `ascii_graphic` and not `e`/`x` says nothing about a
/// letter that has quietly become a wall of brick.
const RANDOM_LABELS: &[u8] = b"acfghijklmnopqrtuyz";

/// Every glyph the level scene puts on the screen: the tiles, the walkers, the goal, the grave.
///
/// The tiles come from [`sim::Tile::ALL`], so a tile added to the sim is a letter taken out of the
/// label alphabet without anyone having to remember this function exists. It is the *check* on
/// [`RANDOM_LABELS`] rather than the source of it — the alphabet is a constant and this is the
/// question asked of it — which is why its one reader is the test below.
#[cfg(test)]
fn reserved_glyphs() -> Vec<char> {
    let mut glyphs: Vec<char> = sim::Tile::ALL.iter().map(|tile| tile.glyph()).collect();
    glyphs.push(GLYPH_WALKER_ALIVE);
    glyphs.push(GLYPH_WALKER_DEAD);
    // The two marks that are not tiles: the goal trigger the scene overlays, and the grave a dead
    // lane becomes ([`lane_glyph`]).
    glyphs.push('!');
    glyphs.push('+');
    glyphs
}

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
// recorded champion paths (§10 Tier 2.7)
// ---------------------------------------------------------------------------

/// Record one generation's champion route, in full.
///
/// Run as its **own** episode rather than sampled off the animation, for the same
/// reason the lanes run their own: the record has to be a fact about the run, not
/// about how long the viewer happened to watch. A paused, skipped or frame-capped
/// animation would truncate the path — and on a stalled run, exactly the run this
/// feature exists for, the viewer is the most likely to have walked away.
///
/// The caller asks [`TraceLog::would_record`] first, so this costs one episode per
/// *improvement*: at pop 200 a generation is 200 episodes, so a recorded generation
/// is ~0.5% more expensive and every other generation pays nothing.
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
    loop {
        let frames = runner.sim().stats().frames;
        if frames % TRACE_SAMPLE_FRAMES == 0 {
            let p = runner.sim().player();
            points.push(TracePoint {
                frame: frames,
                x: p.x,
                y: p.y,
            });
        }
        if runner.is_finished() {
            break;
        }
        runner.step();
    }

    // The final position is the one that matters most — the flag, or the grave — and
    // an episode rarely ends on a sample boundary, so it is recorded exactly rather
    // than rounded to the nearest sample.
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

// ---------------------------------------------------------------------------
// live controls
// ---------------------------------------------------------------------------

/// Controls the viewer holds between frames.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LiveState {
    speed: u32,
    /// Per-gene mutation probability, adjustable while the run is on screen
    /// (§10 Tier 2.9), always one of [`MUTATION_RATES`].
    mut_rate: f32,
    paused: bool,
    /// Follow the leading agent's camera, or hold the whole level still.
    follow: bool,
    /// The recorded generation being shown, or `None` for the live view (§10
    /// Tier 2.7).
    ///
    /// Held as a **generation**, never as an index into the record: the log drops
    /// its oldest entry when it fills, which shifts every index by one and would
    /// silently re-point the cursor at a different route than the viewer chose.
    trace_cursor: Option<u32>,
    quit: bool,
    /// `s` was pressed; the caller must save and then show a note.
    save_requested: bool,
}

impl Default for LiveState {
    fn default() -> LiveState {
        LiveState {
            speed: DEFAULT_SPEED,
            mut_rate: GaConfig::default().mutation_rate,
            paused: false,
            follow: true,
            trace_cursor: None,
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

    fn step_mut_rate(&mut self, up: bool) {
        self.mut_rate = if up {
            next_mut_rate_up(self.mut_rate)
        } else {
            next_mut_rate_down(self.mut_rate)
        };
    }

    /// Scrub one recorded improvement older, stopping at the oldest.
    fn step_trace_back(&mut self, traces: &TraceLog) {
        self.trace_cursor = traces.step_back(self.trace_cursor);
    }

    /// Scrub one newer; one step past the newest returns to the live view.
    fn step_trace_forward(&mut self, traces: &TraceLog) {
        self.trace_cursor = traces.step_forward(self.trace_cursor);
    }

    /// Back to live, whatever the scrubber was showing.
    fn trace_live(&mut self) {
        self.trace_cursor = None;
    }
}

/// What one keypress means. Split out of the event loop so the whole key map is
/// testable without a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyAction {
    None,
    SpeedUp,
    SpeedDown,
    MutRateUp,
    MutRateDown,
    /// Walk the recorded champion paths (§10 Tier 2.7).
    TraceBack,
    TraceForward,
    TraceLive,
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
        KeyCode::Char('m') => KeyAction::MutRateUp,
        KeyCode::Char('M') => KeyAction::MutRateDown,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('.') | KeyCode::Enter => {
            KeyAction::SkipGeneration
        }
        KeyCode::Char('f') | KeyCode::Char('F') => KeyAction::ToggleFollow,
        KeyCode::Char('s') | KeyCode::Char('S') => KeyAction::Save,
        KeyCode::Left => KeyAction::TraceBack,
        KeyCode::Right => KeyAction::TraceForward,
        KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Home => KeyAction::TraceLive,
        _ => KeyAction::None,
    }
}

/// Apply a keypress, returning whether the viewer asked to skip this generation's
/// animation.
fn apply_key(action: KeyAction, live: &mut LiveState, traces: &TraceLog) -> bool {
    match action {
        KeyAction::None => false,
        KeyAction::TraceBack => {
            live.step_trace_back(traces);
            false
        }
        KeyAction::TraceForward => {
            live.step_trace_forward(traces);
            false
        }
        KeyAction::TraceLive => {
            live.trace_live();
            false
        }
        KeyAction::SpeedUp => {
            live.step_speed(true);
            false
        }
        KeyAction::SpeedDown => {
            live.step_speed(false);
            false
        }
        KeyAction::MutRateUp => {
            live.step_mut_rate(true);
            false
        }
        KeyAction::MutRateDown => {
            live.step_mut_rate(false);
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

fn mut_label(rate: f32) -> String {
    format!("{rate:.2}")
}

/// Snap an arbitrary rate onto the ladder, so `--mut-rate 0.07` behaves like the
/// nearest keypress rather than creating a rate no key can get back to. The
/// trainer's own `sanitized` clamp is a backstop, not the contract: the contract is
/// that the value on screen is always one of [`MUTATION_RATES`].
fn nearest_mut_rate(requested: f32) -> f32 {
    if !requested.is_finite() {
        return GaConfig::default().mutation_rate;
    }
    let mut best = MUTATION_RATES[0];
    for rate in MUTATION_RATES {
        if (rate - requested).abs() < (best - requested).abs() {
            best = rate;
        }
    }
    best
}

fn next_mut_rate_up(rate: f32) -> f32 {
    let i = MUTATION_RATES.iter().position(|r| *r >= rate).unwrap_or(0);
    MUTATION_RATES[(i + 1).min(MUTATION_RATES.len() - 1)]
}

fn next_mut_rate_down(rate: f32) -> f32 {
    let i = MUTATION_RATES.iter().rposition(|r| *r <= rate).unwrap_or(0);
    MUTATION_RATES[i.saturating_sub(1)]
}

/// Push the live knobs into the trainer, which breeds the *next* generation from
/// the configuration it currently holds.
///
/// Factored out of the run loop so "the key changes the evolution, not just the
/// on-screen number" is a testable claim rather than a comment.
fn apply_live_config(trainer: &mut Trainer, live: &LiveState) {
    trainer.config.mutation_rate = live.mut_rate;
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

/// Fixed rows: three status lines (the header, the fitness numbers and what the search
/// is doing to itself), the generation bar, the death strip and two hint lines.
const FIXED_ROWS: usize = 7;

/// The optional rows, each of which is a fact worth a line of its own.
///
/// A struct rather than four positional booleans: the layout is asked for one of
/// these in half a dozen places, and `layout(rows, true, false, false, true)` is a
/// call nobody can check at a glance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ExtraRows {
    /// The genius splash (Tier 1.3).
    genius: bool,
    /// The save note.
    note: bool,
    /// The stall line (finding 19).
    stalled: bool,
    /// The recorded-champion row (Tier 2.7) — also the only place the scrub keys
    /// are advertised, because the two hint lines have no room left at 80 columns.
    recorded: bool,
}

impl ExtraRows {
    fn count(self) -> usize {
        usize::from(self.genius)
            + usize::from(self.note)
            + usize::from(self.stalled)
            + usize::from(self.recorded)
    }
}

/// The chart takes a third of the spare rows, the level view the rest.
///
/// On a terminal too short for both minimums the sections keep their minimums and
/// the frame is truncated at draw time: losing the hint lines is survivable, losing
/// the level view is not.
fn layout(rows: usize, extra: ExtraRows) -> Layout {
    let spare = rows.saturating_sub(FIXED_ROWS + extra.count());
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
    /// The recorded champion routes the scrubber walks (Tier 2.7).
    traces: &'a TraceLog,
    /// The recorded generation being shown, or `None` while the view is live.
    scrub: Option<u32>,
    /// What the search is doing to itself (mutation size, stagnation, immigrants, and how
    /// much of the behavioural archive it is spread across).
    search: SearchState,
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
    // Single spaces between fields on purpose: at 80 columns the flags at the end
    // (`PAUSED`, `whole level`) are the difference between a viewer waiting and a
    // viewer thinking the thing has hung, and double spacing was enough to clip
    // them off once the live mutation rate joined the line.
    // A scrubbed view describes the *record*, not the live run: the numbers printed
    // beside a picture have to belong to it (Tier 2.7).
    let record = f.scrub.and_then(|g| f.traces.get(g));
    let mut head = match record {
        Some(trace) => format!(
            "{:<14} record {}/{}: gen {}  max {}  {}",
            clip(f.level_name, 14),
            f.traces
                .index_of(trace.generation)
                .map(|i| i + 1)
                .unwrap_or(0),
            f.traces.len(),
            trace.generation,
            format_fitness(trace.max_fitness),
            if trace.won { "solved" } else { "not solved" }
        ),
        None => format!(
            "{:<14} gen {}/{} lanes {}/{} alive speed {} mut {}",
            clip(f.level_name, 14),
            f.generation,
            f.generations.max(1),
            f.alive,
            f.lane_count,
            speed_label(f.live.speed),
            mut_label(f.live.mut_rate)
        ),
    };
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

    // 2b. What the search is doing to *itself*, which the fitness numbers cannot say.
    //     `mut` on the header is the viewer's own rate; sigma is the size the children of
    //     this generation were actually bred with, which with adaptive sigma the run has
    //     been moving on its own. Watching it fall is watching the run stop exploring and
    //     start polishing, and watching it rise is a plateau being attacked.
    let mut search = format!("search  sigma {:.3}", f.search.sigma);
    if f.search.stagnation > 0 {
        search.push_str(&format!("   no new best in {}", f.search.stagnation));
    }
    // The quality-diversity half of the same story, in the desktop window's own words (see
    // `app::panel::search_line`): a viewer who reads "diversity 9/48 niches" here and in the
    // window is reading the same number off the same run, which is the rule that keeps the two
    // front-ends one game. The denominator is the archive's *behavioural* cells and not the grid's
    // whole 240: the grid is 48 ways of moving × 5 finish bands, and counting a run across a third
    // of the behaviours out of the total would report it as an eighth explored — the window would
    // say 16/48 beside this line's 16/240, which is the disagreement this parity rule exists to
    // stop. Both clauses are skipped when they are zero rather than printed as zeros — before the
    // first generation there is no archive to report, and `diversity 0/48` beside a live run would
    // read as a population that had collapsed rather than one that has not started.
    if f.search.archive_children > 0 {
        search.push_str(&format!(
            "   +{} from the archive",
            f.search.archive_children
        ));
    }
    if f.search.archive_bits > 0 {
        search.push_str(&format!(
            "   diversity {}/{} niches   speed {}/{} bands",
            f.search.archive_bits,
            evo::ARCHIVE_BEHAVIOUR_CELLS,
            f.search.archive_time_bits,
            evo::ARCHIVE_TIME_BINS
        ));
    }
    lines.push(clip(&search, cols));

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
        // The reheated batch arrives only after a stall, so its size belongs on the line that
        // says how long the stall has been. It is the difference between a run that has
        // converged and a run that has converged and done something about it.
        let line = match f.search.immigrants {
            0 => line,
            n => format!("{line}   +{n} reheated genomes"),
        };
        lines.push(clip(&line, cols));
    }

    // 3b. The record the scrubber walks (Tier 2.7): how many improvements are kept
    //     and the span they cover, or which one is on screen. This row is also where
    //     the scrub keys live, because the two hint lines below have no room left at
    //     80 columns and an undocumented control is not a control.
    if let Some(trace) = record {
        let index = f
            .traces
            .index_of(trace.generation)
            .map(|i| i + 1)
            .unwrap_or(0);
        // The trail's glyph is named here, on the row that owns it: the death strip
        // at the bottom of the frame uses the same character for "one death in this
        // column", and two meanings for one glyph in one frame is a defect unless
        // the frame says which is which.
        lines.push(clip(
            &format!(
                "record {}/{}   left/right scrub   h live   : marks the route",
                index,
                f.traces.len()
            ),
            cols,
        ));
    } else if let Some((first, last)) = f.traces.span() {
        lines.push(clip(
            &format!(
                "record {} improvements kept (gen {first}..{last})   left/right scrub   h live",
                f.traces.len()
            ),
            cols,
        ));
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
        "keys  space pause  -/+ speed  m/M mutation  n next  f follow  s save  q quit",
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
    traces: &'a TraceLog,
    /// What the search is doing to *itself* this generation: the self-tuning mutation
    /// size, how long the best has stood still, how many reheated genomes that bought, and how
    /// many behavioural niches the archive holds. Beside the fitness row rather than in it,
    /// because these are the trainer's readings of its own search and not the population's
    /// score.
    search: SearchState,
}

/// The trainer's own reading of its search, carried into the frame.
///
/// All five are read off the generation that was just scored, so a viewer watching the
/// size fall is watching the run stop exploring and start polishing — the one thing a
/// fitness chart cannot show, because a chart of a converged run and a chart of a run
/// that is quietly searching a small neighbourhood look identical on it.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct SearchState {
    /// The mutation size children were bred with ([`evo::GenRow::mutation_sigma`]).
    sigma: f32,
    /// Generations since the best fitness last improved.
    stagnation: u32,
    /// Genomes bred into the generation that follows the scored one, each a reheat of the
    /// run's best-ever genome. Non-zero only after a stagnation, so it is also the signal that
    /// the run has stopped moving and done something about it.
    immigrants: usize,
    /// Children of the quality-diversity archive bred into the generation that follows the
    /// scored one ([`evo::GenRow::archive_children`]). Unlike the immigrants these arrive every
    /// generation, so the number is a fact about the search's shape rather than about a stall.
    archive_children: usize,
    /// Behavioural niches the archive held at the end of the scored generation, of
    /// [`evo::ARCHIVE_BEHAVIOUR_CELLS`]. The run's diversity reading: it falls when the population
    /// collapses onto one way of moving, which no column of scores can show.
    archive_bits: usize,
    /// Finish bands holding a win, of [`evo::ARCHIVE_TIME_BINS`]. The run's *speed* coverage, and a
    /// different reading from the one above: it climbs only when the population learns to solve the
    /// level quicker, never when it finds a new way of moving. Zero before anything has finished,
    /// which is why the frame prints it only beside a non-empty archive.
    archive_time_bits: usize,
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
    runners: &mut [EpisodeRunner],
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
                if apply_key(key_action(key.code, key.modifiers), live, ctx.traces) {
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

        if screen.budget_exhausted() || runners.iter().all(|r| r.is_finished()) || stepped >= budget
        {
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
    runners: &[EpisodeRunner],
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
    let extra = ExtraRows {
        genius: ctx.genius,
        note: ctx.note.is_some(),
        stalled: stall.is_some(),
        recorded: !ctx.traces.is_empty(),
    };
    let l = layout(rows, extra);

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

    // What the level view shows: the live lanes, or one recorded champion route
    // (Tier 2.7). The two never mix — a past route drawn among live agents would be
    // a picture of two different generations at once, and the numbers beside it
    // would belong to neither.
    let recorded = live.trace_cursor.and_then(|g| ctx.traces.get(g));

    let (cx, cy) = match recorded {
        // A scrubbed view centres on where the route stopped: that is the obstacle
        // the moment being shown is about.
        Some(trace) => match trace.end() {
            Some(p) => (p.x, p.y),
            None => (level.width as f32 * 0.5, level.height as f32 * 0.5),
        },
        None => match leader {
            Some(i) => {
                let p = runners[i].sim().player();
                (p.x + p.w * 0.5, p.y + p.h * 0.5)
            }
            None => (level.width as f32 * 0.5, level.height as f32 * 0.5),
        },
    };
    // "Whole level" is the same camera rule with the centre pinned to the middle of
    // the level, so it is a genuine whole-level view whenever the level fits.
    let (cx, cy) = if live.follow {
        (cx, cy)
    } else {
        (level.width as f32 * 0.5, level.height as f32 * 0.5)
    };
    let view = Viewport::following_body(level, cx, cy, cols as u16, l.scene_rows as u16);

    let scene = Scene::new(
        level,
        match recorded {
            Some(trace) => trace.bodies(),
            None => bodies,
        },
    );
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
        traces: ctx.traces,
        scrub: live.trace_cursor,
        progress,
        search: ctx.search,
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
        cut_stuck_frames: 0,
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
    // Champion mode replays one saved genome, so it has no record to scrub: a
    // record exists to compare a population's improvements against each other, and
    // there is exactly one route here.
    let traces = TraceLog::new();
    let ctx = GenCtx {
        generation: champion.metadata.generation,
        generations: 1,
        pop_size: 1,
        wins: history[0].wins,
        genius: false,
        history: &history,
        deaths: &[],
        note: None,
        traces: &traces,
        // A replay has no search behind it: the genome is already chosen, so the frame
        // must not imply a mutation size is being tuned around it.
        search: SearchState::default(),
    };
    let mut runners = vec![EpisodeRunner::new(
        level,
        genome.clone(),
        &eval,
        champion.metadata.episode_seed,
    )];
    animate_lanes(screen, level, &mut runners, &lanes, &ctx, live)
        .map_err(|e| format!("cannot draw the champion replay: {e}"))?;
    let result = runners.pop().expect("exactly one lane").result();
    println!(
        "replayed {} after {} frames, fitness {:.2}, best_x {:.1}, jumps {}, checkpoints {}",
        result.outcome.as_str(),
        result.frames,
        result.fitness,
        result.best_x,
        result.jumps,
        result.checkpoints
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
    println!(
        "watched {} ({} generations)",
        level.name,
        trainer.history().len()
    );
    // The clock, before the best agent rather than after it: what a viewer carries away from a
    // watching session is *how quickly* the level was solved, and `best` below is a different
    // record — the highest-scoring genome, which is not always the quickest win.
    match trainer.fastest() {
        Some(fast) => println!(
            "clock   {} ({} frames) at the flag (gen {} id {}, {:.0}% of the level's {} unspent)",
            sim::format_time(fast.frames),
            fast.frames,
            fast.generation,
            fast.genome_id,
            fast.spare() * 100.0,
            sim::format_time(fast.budget_frames),
        ),
        None => println!("clock   no win to time: nothing reached the flag"),
    }
    if let Some(best) = trainer.best_ever() {
        println!(
            "best    gen {} id {} fitness {:.2} ({} after {} frames, best_x {:.1}, jumps {}, checkpoints {})",
            best.generation,
            best.genome.id,
            best.result.fitness,
            best.result.outcome.as_str(),
            best.result.frames,
            best.result.best_x,
            best.result.jumps,
            best.result.checkpoints
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

fn finish_champion(
    screen: &mut Screen,
    level: &Level,
    champion: &ChampionFile,
) -> Result<(), String> {
    screen.close();
    println!();
    println!(
        "watched champion {} on {}",
        champion.metadata.genome_id, level.name
    );
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
            report
                .diagnosis
                .as_deref()
                .unwrap_or("no explanation available")
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
    /// The most recent frame, kept so a test can assert on what was actually drawn
    /// — the composition, not just the rows it was composed from.
    last: String,
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
            last: String::new(),
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
        self.last = frame.to_string();
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

    /// [`apply_key`] against an empty record.
    ///
    /// The scrub keys are the only ones that read the record, and they get their
    /// own test with a real one — so everything else can keep the two-argument
    /// shape it had before there was a record to scrub.
    fn press(action: KeyAction, live: &mut LiveState) -> bool {
        apply_key(action, live, &TraceLog::new())
    }

    // -- keys -----------------------------------------------------------------

    #[test]
    fn the_mutation_keys_walk_the_shared_ladder() {
        // The ladder itself is asserted where it lives, in `evo::ga` — this is only
        // about what the *keys* do with it.
        let default = GaConfig::default().mutation_rate;

        // Snapping, so `--mut-rate 0.07` behaves like a keypress rather than creating
        // a rate no key can return to.
        assert_eq!(nearest_mut_rate(0.07), 0.08);
        assert_eq!(nearest_mut_rate(0.031), 0.03);
        assert_eq!(nearest_mut_rate(99.0), 0.10);
        assert_eq!(nearest_mut_rate(-4.0), MUTATION_RATES[0]);
        assert_eq!(nearest_mut_rate(f32::NAN), default);

        // Both ends are hard stops, not wraparounds.
        assert_eq!(next_mut_rate_up(0.10), 0.10);
        assert_eq!(next_mut_rate_down(0.03), 0.03);
        assert_eq!(next_mut_rate_up(0.03), 0.05);
        assert_eq!(next_mut_rate_down(0.10), 0.08);
    }

    /// The keyboard has to reach *evolution*, not just the number on screen. Two
    /// trainers from the same seed, one of them turned up to the top rung: after a
    /// generation the populations must differ, which is only true if
    /// `apply_live_config` feeds the breeder rather than the status line.
    #[test]
    fn the_live_mutation_rate_changes_the_offspring() {
        let level = load_level(&wpath("levels/l4-rhythm.json")).expect("level loads");
        let cfg = GaConfig {
            pop_size: 24,
            master_seed: 9,
            ..GaConfig::default()
        };
        let mut quiet = Trainer::new(cfg.clone());
        let mut lively = Trainer::new(cfg);
        let mut live = LiveState::default();
        apply_live_config(&mut quiet, &live);

        live.mut_rate = *MUTATION_RATES.last().expect("ladder");
        apply_live_config(&mut lively, &live);
        assert_eq!(
            lively.config.mutation_rate,
            *MUTATION_RATES.last().expect("ladder")
        );

        quiet.step(&level);
        lively.step(&level);
        let same = quiet
            .genomes
            .iter()
            .zip(&lively.genomes)
            .all(|(a, b)| a.weights == b.weights);
        assert!(
            !same,
            "raising the mutation rate through the live controls must change the \
             next generation; identical populations mean the knob is only cosmetic"
        );
    }

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
            key_action(KeyCode::Char('m'), KeyModifiers::NONE),
            KeyAction::MutRateUp
        );
        assert_eq!(
            key_action(KeyCode::Char('M'), KeyModifiers::SHIFT),
            KeyAction::MutRateDown
        );
        assert_eq!(
            key_action(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyAction::Quit
        );
        assert_eq!(
            key_action(KeyCode::Esc, KeyModifiers::NONE),
            KeyAction::Quit
        );
        assert_eq!(
            key_action(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyAction::Quit
        );
        assert_eq!(
            key_action(KeyCode::F(7), KeyModifiers::NONE),
            KeyAction::None
        );
    }

    #[test]
    fn keys_drive_the_live_state() {
        let mut live = LiveState::default();
        assert!(!press(
            key_action(KeyCode::Char(' '), KeyModifiers::NONE),
            &mut live
        ));
        assert!(live.paused);
        assert!(!press(
            key_action(KeyCode::Char(' '), KeyModifiers::NONE),
            &mut live
        ));
        assert!(!live.paused);

        assert!(!press(
            key_action(KeyCode::Char('f'), KeyModifiers::NONE),
            &mut live
        ));
        assert!(!live.follow);

        assert!(press(
            key_action(KeyCode::Char('n'), KeyModifiers::NONE),
            &mut live
        ));
        assert!(!live.quit, "skipping a generation is not quitting");

        assert!(!press(
            key_action(KeyCode::Char('s'), KeyModifiers::NONE),
            &mut live
        ));
        assert!(live.save_requested);

        // Mutation is a ladder too: up twice from the default, then pinned at the
        // top; down three times, then pinned at the bottom.
        let default_rate = LiveState::default().mut_rate;
        for _ in 0..2 {
            press(
                key_action(KeyCode::Char('m'), KeyModifiers::NONE),
                &mut live,
            );
        }
        assert_eq!(live.mut_rate, *MUTATION_RATES.last().expect("ladder"));
        press(
            key_action(KeyCode::Char('m'), KeyModifiers::NONE),
            &mut live,
        );
        assert_eq!(live.mut_rate, *MUTATION_RATES.last().expect("ladder"));
        for _ in 0..5 {
            press(
                key_action(KeyCode::Char('M'), KeyModifiers::NONE),
                &mut live,
            );
        }
        assert_eq!(live.mut_rate, MUTATION_RATES[0]);
        assert!(
            default_rate > MUTATION_RATES[0],
            "the default has to have a rung below it or the slider only goes one way"
        );

        press(
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
        // The expected labels are read off the two sequences ([`lane_label`]'s own) rather than
        // spelled out here, which is what this assertion used to do: it said `['1','2','a','b',
        // 'c','d']`, and the trap pass took `b` and `d` out of the label alphabet, so the literal
        // had become a second copy of the rule — the kind that can only ever disagree with it.
        let expected: Vec<char> = ['1', '2']
            .into_iter()
            .chain(RANDOM_LABELS.iter().take(4).map(|b| *b as char))
            .collect();
        assert_eq!(
            info.iter().map(|l| l.label).collect::<Vec<char>>(),
            expected
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
        // What matters is the glyphs drawn in the *same region* as the lanes: the tiles of the
        // level, the walkers (`e` alive, `x` dead), the `+` a dead lane becomes, and the goal. The
        // chart glyphs live above the level view, so sharing a letter with them is fine. The list is
        // the module's own ([`reserved_glyphs`]) and covers every tile, so a tile that takes a
        // letter out of the alphabet fails here rather than in a picture of eight lanes sitting on a
        // wall of brick.
        let reserved = reserved_glyphs();
        for label in RANDOM_LABELS {
            assert!(
                !reserved.contains(&(*label as char)),
                "the label alphabet offers {:?}, which is a glyph the scene draws",
                *label as char
            );
        }
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

    fn frame(cols: usize, rows: usize, genius: bool, note: Option<&'static str>) -> String {
        frame_with(
            cols,
            rows,
            FrameOpts {
                genius,
                note,
                ..Default::default()
            },
        )
    }

    /// A record of three improvements, with gaps between them: 0, 40 and 97 were
    /// improvements and the generations in between were not, which is what a real
    /// record looks like.
    fn recorded_log() -> TraceLog {
        let mut log = TraceLog::new();
        for (generation, max_fitness) in [(0u32, 100.0f32), (40, 200.0), (97, 400.0)] {
            log.observe(ChampionTrace {
                generation,
                max_fitness,
                won: generation == 97,
                points: vec![TracePoint {
                    frame: 0,
                    x: 2.0,
                    y: 12.0,
                }],
            });
        }
        log
    }

    /// Options for [`frame_with`], all off by default.
    ///
    /// Five positional arguments (two of them `Option`s of different meanings) was
    /// the shape this had before the record row existed; naming the fields instead
    /// makes each call site say what it is actually asking for.
    #[derive(Default)]
    struct FrameOpts {
        genius: bool,
        note: Option<&'static str>,
        history: Option<Vec<HistoryPoint>>,
        /// Attach the recorded improvements from [`recorded_log`].
        record: bool,
        /// Show this recorded generation instead of the live view. Implies
        /// `record` even when the caller forgot to ask for one.
        scrub: Option<u32>,
        /// The search line's own state; zero sigma by default, which is what a frame
        /// whose run has not been told anything looks like.
        search: SearchState,
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
        frame_with(
            cols,
            rows,
            FrameOpts {
                history: Some(h),
                ..Default::default()
            },
        )
    }

    fn frame_with(cols: usize, rows: usize, opts: FrameOpts) -> String {
        let level = load_level(&wpath("levels/l3-gauntlet.json")).expect("level loads");
        let genius = opts.genius;
        let note = opts.note;
        let owned = opts.history.unwrap_or_else(|| history(9));
        let traces = if opts.record || opts.scrub.is_some() {
            recorded_log()
        } else {
            TraceLog::new()
        };
        let scrub = opts.scrub;
        let search = opts.search;
        let deaths = vec![(10.0, 12.0, 3u32), (10.4, 12.0, 4), (44.0, 12.0, 5)];
        let stall = fitness_stall(&owned);
        let cluster = stall.and_then(|_| death_cluster(&deaths, level.width));
        let stalled = stall.is_some();
        let extra = ExtraRows {
            genius,
            note: note.is_some(),
            stalled,
            recorded: !traces.is_empty(),
        };
        let scene: Vec<String> = (0..layout(rows, extra).scene_rows)
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
            traces: &traces,
            scrub,
            progress: Progress {
                index: 120,
                budget: 675,
            },
            search,
        };
        render_frame(&f, cols, rows, layout(rows, extra))
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
        let traces = TraceLog::new();
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
            traces: &traces,
            scrub: None,
            progress: Progress {
                index: 1,
                budget: 60,
            },
            search: SearchState::default(),
        };
        render_frame(
            &f,
            cols,
            rows,
            layout(
                rows,
                ExtraRows {
                    stalled: true,
                    ..Default::default()
                },
            ),
        )
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
        let stalled = frame_with(
            100,
            34,
            FrameOpts {
                history: Some(h.clone()),
                ..Default::default()
            },
        );
        assert!(stalled.contains("stalled"));
        h.push(HistoryPoint {
            generation: h.len() as u32,
            max_fitness: best * 2.0,
            avg_fitness: best,
            wins: 0,
        });
        let recovered = frame_with(
            100,
            34,
            FrameOpts {
                history: Some(h),
                ..Default::default()
            },
        );
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
                        for recorded in [false, true] {
                            let extra = ExtraRows {
                                genius,
                                note,
                                stalled,
                                recorded,
                            };
                            let l = layout(rows, extra);
                            let reserved = FIXED_ROWS + extra.count();
                            assert_eq!(
                                l.chart_rows + l.scene_rows + reserved,
                                rows,
                                "rows {rows} genius {genius} note {note} stalled {stalled} \
                                 recorded {recorded} must use the whole screen"
                            );
                            assert!(l.chart_rows >= MIN_CHART_ROWS);
                            assert!(l.scene_rows >= MIN_SCENE_ROWS);
                        }
                    }
                }
            }
        }
        // Too short for everything: the chart and the level view keep their
        // minimums and the frame is truncated, rather than the level view shrinking
        // to nothing.
        for rows in [8usize, 12] {
            let l = layout(rows, ExtraRows::default());
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
        // Tier 2.9: the live mutation knob is on screen, because a control whose
        // current value is invisible is a control the viewer cannot use.
        assert!(lines[0].contains("mut 0.05"), "{}", lines[0]);

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
        assert_eq!(
            usable_size(20, 12),
            (20, 12),
            "a small terminal is still a terminal"
        );
    }
    #[test]
    fn a_paused_or_unfollowed_view_says_so() {
        let level = load_level(&wpath("levels/l1-flat-run.json")).expect("level loads");
        let h = history(5);
        let scene = vec!["row".to_string()];
        let traces = TraceLog::new();
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
                mut_rate: *MUTATION_RATES.last().expect("ladder"),
                ..LiveState::default()
            },
            history: &h,
            deaths: &[],
            scene_lines: &scene,
            note: None,
            genius: false,
            stall: None,
            cluster: None,
            traces: &traces,
            scrub: None,
            progress: Progress {
                index: 1,
                budget: 60,
            },
            search: SearchState::default(),
        };
        let text = render_frame(&f, 80, 24, layout(24, ExtraRows::default()));
        assert!(text.contains("PAUSED"));
        assert!(text.contains("whole level"));
        assert!(text.contains("speed 50x"));
        assert!(text.contains("mut 0.10"));
        // The header has to survive the narrowest terminal a flag can matter on.
        // Every value here is at its widest: a 12-character level name, a
        // three-digit generation, a full-speed run and the top mutation rung.
        let head = text.lines().next().expect("a frame has a header");
        assert!(
            head.chars().count() <= 80,
            "the header is {} columns and lost a flag at 80: {head:?}",
            head.chars().count()
        );
        for flag in ["PAUSED", "whole level", "mut 0.10", "lanes 8/8 alive"] {
            assert!(head.contains(flag), "{flag:?} was clipped off: {head:?}");
        }
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

    // -- curriculum resume (§12 M5) -------------------------------------------

    /// A stalled run checkpoints itself, steps back to the easier level, and can be
    /// restarted from that checkpoint: one test, because the point is that the file
    /// one half writes is the file the other half reads.
    #[test]
    fn a_stalled_run_checkpoints_itself_and_can_be_restarted_from_it() {
        let dir = tmpdir("curriculum");
        let out = dir.to_string_lossy().to_string();
        let stalled = wpath("levels/l5-tightrope.json");
        let easier = wpath("levels/l1-flat-run.json");

        // L5 at pop 2 does not improve for the 25 generations the stall rule asks
        // for, which is exactly the trigger. L1 is the easier level: a random
        // population wins it at generation 0, so the detour is cheap to run.
        //
        // The run is **frozen** — no mutation, crossover, fine children or immigrants — so its best
        // score is flat from generation 0 and the 25-generation stall rule is a fact about the
        // fixture, not about where a sensor version happened to put pop 2's luck on l5. (Before
        // this it was a mutable pop 2 with a cap tuned to whenever its last new best fell; sensors
        // v7 moved that number, and v13 moved it past the cap again.) The window is what the panel
        // uses (`render::chart::STALL_GENERATIONS`), not a flag, so the run is given room anyway.
        let stalled_args = a(&[
            "watch",
            "--level",
            &stalled,
            "--pop",
            "2",
            "--mut-sigma",
            "0",
            "--mut-rate",
            "0",
            "--crossover",
            "0",
            "--fine-children",
            "0",
            "--immigrants",
            "0",
            "--generations",
            "60",
            "--curriculum",
            &easier,
            "--curriculum-generations",
            "2",
            "--out",
            &out,
            "--headless",
            "--frames",
            "1",
        ]);
        watch(&stalled_args).expect("a stalled run with a curriculum completes");

        let path = dir.join("l5-tightrope.population.json");
        assert!(path.exists(), "the stuck population is written down");
        let checkpoint = evo::PopulationFile::load(&path).expect("the checkpoint loads");
        assert_eq!(checkpoint.networks.len(), 2);
        assert!(
            checkpoint.note.contains("stalled"),
            "the checkpoint records why it exists: {:?}",
            checkpoint.note
        );
        let ids: Vec<u32> = checkpoint.genome_ids.clone();

        // Restart from it, on the level it came from.
        let resumed_out = tmpdir("curriculum-resumed");
        let out2 = resumed_out.to_string_lossy().to_string();
        let resumed_args = a(&[
            "watch",
            "--level",
            &stalled,
            "--resume",
            path.to_string_lossy().as_ref(),
            "--generations",
            "3",
            "--out",
            &out2,
            "--headless",
            "--frames",
            "1",
        ]);
        watch(&resumed_args).expect("a run restarted from a checkpoint completes");

        // The champion it saved has to come out of the population it was handed:
        // an id from the checkpoint, never a fresh one, and never a generation
        // earlier than the checkpoint's own.
        let level = load_level(&stalled).expect("level loads");
        let champion =
            evo::ChampionFile::load_for_level(resumed_out.join("l5-tightrope.json"), &level)
                .expect("the resumed run saved a champion for this level");
        // Either a member of the handed population or a child bred from it, whose id is issued
        // after every checkpoint id. What must not happen is a champion from a *fresh* run, whose
        // ids would start at zero and fall at or below the checkpoint's last id.
        let last = ids.iter().copied().max().unwrap_or(0);
        assert!(
            ids.contains(&champion.metadata.genome_id) || champion.metadata.genome_id > last,
            "champion id {} did not come from the resumed run (population {:?})",
            champion.metadata.genome_id,
            ids
        );
        assert!(
            champion.metadata.generation >= checkpoint.generation,
            "a resumed run continues the checkpoint's generation numbering, not its own"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&resumed_out);
    }

    #[test]
    fn the_curriculum_is_off_unless_asked_for_and_an_impossible_one_is_a_usage_error() {
        // No `--curriculum`: a stalled run must behave exactly as it did before the
        // knob existed — no checkpoint file appears as a side effect.
        let dir = tmpdir("curriculum-off");
        let out = dir.to_string_lossy().to_string();
        let stalled_args = a(&[
            "watch",
            "--level",
            &wpath("levels/l5-tightrope.json"),
            "--pop",
            "2",
            // Short enough that the run does not reach a stall. The point of this case is that
            // *configuring nothing* writes nothing; a stall's derived practice detour is a
            // separate mechanism with its own test above.
            "--generations",
            "5",
            "--out",
            &out,
            "--headless",
            "--frames",
            "1",
        ]);
        watch(&stalled_args).expect("runs");
        assert!(!dir.join("l5-tightrope.population.json").exists());

        // A curriculum level that cannot be loaded is refused before training.
        let resumed_args = a(&[
            "watch",
            "--level",
            &wpath("levels/l5-tightrope.json"),
            "--generations",
            "1",
            "--curriculum",
            &wpath("levels/does-not-exist.json"),
            "--headless",
        ]);
        assert!(
            watch(&resumed_args).is_err(),
            "a missing curriculum level is an error"
        );

        // Zero detour generations would be a silent no-op.
        let zero_args = a(&[
            "watch",
            "--level",
            &wpath("levels/l5-tightrope.json"),
            "--generations",
            "1",
            "--curriculum",
            &wpath("levels/l1-flat-run.json"),
            "--curriculum-generations",
            "0",
            "--headless",
        ]);
        assert!(watch(&zero_args).is_err());

        // Neither knob means anything in champion-replay mode, and saying nothing
        // while ignoring them is exactly the silent no-op this CLI refuses to do.
        let d = a(&[
            "watch",
            "--champion",
            &wpath("champions/l1-flat-run.json"),
            "--curriculum",
            &wpath("levels/l1-flat-run.json"),
            "--headless",
        ]);
        assert!(watch(&d).is_err());
        let e = a(&[
            "watch",
            "--champion",
            &wpath("champions/l1-flat-run.json"),
            "--resume",
            &wpath("champions/l1-flat-run.population.json"),
            "--headless",
        ]);
        assert!(watch(&e).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- the record (Tier 2.7) -----------------------------------------------

    #[test]
    fn the_record_keeps_the_route_the_run_actually_scored() {
        // The record must be a fact about the *run*, not about the animation: the
        // route is re-simulated from the genome and seed the champion lane was
        // scored with, and has to reproduce that episode's outcome, its final
        // position and its length. If this drifts, the scrubber would be showing a
        // different episode than the fitness printed beside it.
        let level = load_level(&wpath("levels/l2-first-gap.json")).expect("level loads");
        let cfg = GaConfig {
            pop_size: 8,
            master_seed: 3,
            ..GaConfig::default()
        };
        let eval = cfg.eval_config();
        let mut trainer = Trainer::new(cfg);
        let (report, population) = trainer.step_verbose(&level);
        let lanes = trainer.showcase(&report, &population, 4, 4);
        let lane = lanes.first().expect("a showcase always has a champion");

        let trace = record_trace(
            &level,
            &eval,
            &population[lane.index],
            lane.seed,
            report.generation,
            report.max_fitness,
        );
        assert_eq!(trace.generation, report.generation);
        assert_eq!(trace.max_fitness, report.max_fitness);
        assert!(
            !trace.points.is_empty(),
            "a route has at least one position"
        );

        // The same episode, driven independently.
        let mut runner =
            EpisodeRunner::new(&level, population[lane.index].clone(), &eval, lane.seed);
        while !runner.is_finished() {
            runner.step();
        }
        let end = {
            let p = runner.sim().player();
            (p.x, p.y)
        };
        let scored = runner.result();
        assert_eq!(
            trace.won,
            scored.won(),
            "the record agrees about the outcome"
        );
        assert_eq!(
            trace.frames(),
            scored.frames,
            "the route ends on the scored episode's last frame"
        );
        let last = trace.end().expect("non-empty");
        assert_eq!(
            (last.x, last.y),
            end,
            "the route ends where the episode ended, not at a rounded sample"
        );

        // Samples land on the documented interval; only the final one is exact.
        for pair in trace.points.windows(2) {
            if pair[1].frame == trace.frames() {
                continue;
            }
            assert_eq!(pair[1].frame - pair[0].frame, TRACE_SAMPLE_FRAMES);
        }
    }

    #[test]
    fn the_scrub_keys_walk_the_record_and_return_to_live() {
        assert_eq!(
            key_action(KeyCode::Left, KeyModifiers::NONE),
            KeyAction::TraceBack
        );
        assert_eq!(
            key_action(KeyCode::Right, KeyModifiers::NONE),
            KeyAction::TraceForward
        );
        assert_eq!(
            key_action(KeyCode::Char('h'), KeyModifiers::NONE),
            KeyAction::TraceLive
        );

        let log = recorded_log(); // improvements at generations 0, 40 and 97
        let mut live = LiveState::default();
        assert_eq!(live.trace_cursor, None, "a fresh view is live");

        let press_key = |live: &mut LiveState, code: KeyCode| {
            apply_key(key_action(code, KeyModifiers::NONE), live, &log);
        };

        // Left from live lands on the newest improvement, then walks back.
        press_key(&mut live, KeyCode::Left);
        assert_eq!(live.trace_cursor, Some(97));
        press_key(&mut live, KeyCode::Left);
        assert_eq!(live.trace_cursor, Some(40));
        press_key(&mut live, KeyCode::Left);
        assert_eq!(live.trace_cursor, Some(0));
        // The oldest is a hard stop, not a wrap to the newest: a scrubber that
        // circled would make it impossible to tell you were at the beginning.
        press_key(&mut live, KeyCode::Left);
        assert_eq!(live.trace_cursor, Some(0));

        // Right walks forward, and one step past the newest is live again.
        press_key(&mut live, KeyCode::Right);
        assert_eq!(live.trace_cursor, Some(40));
        press_key(&mut live, KeyCode::Right);
        assert_eq!(live.trace_cursor, Some(97));
        press_key(&mut live, KeyCode::Right);
        assert_eq!(live.trace_cursor, None);
        press_key(&mut live, KeyCode::Right);
        assert_eq!(live.trace_cursor, None, "still live");

        // `h` comes back from anywhere in the record.
        press_key(&mut live, KeyCode::Left);
        press_key(&mut live, KeyCode::Left);
        assert_eq!(live.trace_cursor, Some(40));
        press_key(&mut live, KeyCode::Char('h'));
        assert_eq!(live.trace_cursor, None);

        // With nothing recorded the scrub keys are inert rather than a panic.
        let mut empty = LiveState::default();
        apply_key(
            key_action(KeyCode::Left, KeyModifiers::NONE),
            &mut empty,
            &TraceLog::new(),
        );
        assert_eq!(empty.trace_cursor, None);
    }

    #[test]
    fn the_record_row_reports_the_record_and_labels_the_scrubbed_view() {
        // Live, with a record available: how many improvements, and from when. The
        // row is also the only advertisement for the scrub keys.
        let live = frame_with(
            100,
            34,
            FrameOpts {
                record: true,
                ..Default::default()
            },
        );
        assert!(
            live.contains("record 3 improvements kept (gen 0..97)"),
            "a live view with a record says what it has: {live}"
        );
        assert!(live.contains("left/right scrub"));
        assert!(live.contains("h live"));
        assert!(
            live.contains("lanes 6/8 alive"),
            "the live view still describes the live run: {live}"
        );

        // Scrubbed: the head describes the record, not the live run. A number next
        // to a picture has to belong to that picture.
        let scrubbed = frame_with(
            100,
            34,
            FrameOpts {
                scrub: Some(40),
                ..Default::default()
            },
        );
        assert!(
            scrubbed.contains("record 2/3: gen 40  max 200.0  not solved"),
            "the head names the record being shown: {scrubbed}"
        );
        assert!(
            scrubbed.contains("record 2/3   left/right scrub   h live   : marks the route"),
            "the row names the trail glyph it owns: {scrubbed}"
        );
        assert!(
            !scrubbed.contains("lanes 6/8 alive"),
            "the live lane count must not sit beside a recorded route: {scrubbed}"
        );

        // The newest entry is the one that solved the level, and it says so.
        let solved = frame_with(
            100,
            34,
            FrameOpts {
                scrub: Some(97),
                ..Default::default()
            },
        );
        assert!(
            solved.contains("record 3/3: gen 97  max 400.0  solved"),
            "{solved}"
        );

        // The record row is paid for out of the chart and the scene, exactly like
        // the stall and genius rows: the frame keeps its height either way.
        assert_eq!(live.lines().count(), 34);
        assert_eq!(scrubbed.lines().count(), 34);
    }

    #[test]
    fn a_scrubbed_view_draws_the_recorded_route_without_the_live_lanes() {
        // The glue rather than the parts: with a cursor set, what reaches the screen
        // is the recorded route over the level, labelled as the record.
        use render::trace::GLYPH_TRAIL;
        let level = load_level(&wpath("levels/l3-gauntlet.json")).expect("level loads");
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
        // A route built by hand, so the test does not depend on what evolution
        // happens to do: sixteen samples walking right along the ground.
        let mut traces = TraceLog::new();
        assert!(traces.observe(ChampionTrace {
            generation: 7,
            max_fitness: 42.0,
            won: false,
            points: (0..16)
                .map(|i| TracePoint {
                    frame: i * TRACE_SAMPLE_FRAMES,
                    x: 2.0 + i as f32 * 0.5,
                    y: 12.0,
                })
                .collect(),
        }));
        let ctx = GenCtx {
            generation: 7,
            generations: 10,
            pop_size: 4,
            wins: 0,
            genius: false,
            history: &history,
            deaths: &[],
            note: None,
            traces: &traces,
            search: SearchState::default(),
        };
        let mut screen = Screen::open(true, Some(1)).expect("headless screen");
        let mut live = LiveState {
            trace_cursor: Some(7),
            ..LiveState::default()
        };
        let mut runners = vec![EpisodeRunner::new(&level, genome.clone(), &eval, 1)];
        animate_lanes(&mut screen, &level, &mut runners, &lanes, &ctx, &mut live).expect("draws");

        let drawn = screen.last.clone();
        assert!(
            drawn.contains("record 1/1: gen 7  max 42.00  not solved"),
            "a scrubbed frame says what it is showing: {drawn}"
        );
        assert!(
            drawn.contains(GLYPH_TRAIL),
            "the recorded route is drawn: {drawn}"
        );
        assert!(
            !drawn.contains("lanes 1/1 alive"),
            "the live lanes are not what is on screen: {drawn}"
        );
    }

    // -- the animation loop ---------------------------------------------------

    /// A one-lane generation, ready to animate: the smallest real call into
    /// [`animate_lanes`], with a headless screen capped at `frames` draws.
    fn animate_one(level: &Level, frames: usize, live: LiveState) -> (LiveState, u32, u32) {
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
        let traces = TraceLog::new();
        let ctx = GenCtx {
            generation: 0,
            generations: 10,
            pop_size: 4,
            wins: 0,
            genius: false,
            history: &history,
            deaths: &[],
            note: None,
            traces: &traces,
            search: SearchState::default(),
        };
        let mut screen = Screen::open(true, Some(frames)).expect("headless screen");
        let mut live = live;
        let mut runners = vec![EpisodeRunner::new(level, genome.clone(), &eval, 1)];
        animate_lanes(&mut screen, level, &mut runners, &lanes, &ctx, &mut live).expect("draws");
        (live, runners[0].sim().stats().frames, screen.drawn as u32)
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
        let paused = LiveState {
            paused: true,
            ..speed
        };
        let (live, frames, drawn) = animate_one(&level, 3, paused);
        assert_eq!(frames, 0, "pause must freeze the physics");
        assert_eq!(drawn, 3);
        assert!(
            live.paused,
            "still paused after the loop: nothing resumed it"
        );
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

        let mut runner = EpisodeRunner::new(&level, genome.clone(), &eval, 7);
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
        assert!(watch(&a(&[
            "watch", "--level", "x.json", "--best", "0", "--random", "0"
        ]))
        .is_err());
        assert!(watch(&a(&["watch", "--level", "x.json", "--frames", "0"])).is_err());
        assert!(watch(&a(&["watch", "--level", "/nonexistent/level.json"])).is_err());
    }
}
