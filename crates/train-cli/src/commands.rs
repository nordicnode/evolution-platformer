//! Subcommand implementations.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use evo::champion::replay as replay_champion;
use evo::ga::MUTATION_SIGMA_RANGE;
use evo::{
    curriculum_step, fit_demo, from_plan, from_route, fused_seeding, polish,
    practice_level_for_stall, practice_step, run_episode, save_practice_level, warm_population,
    ChampionFile, CommitteeFile, EsConfig, EsTrainer, FuseConfig, HumanDemo,
    EvalConfig, GaConfig, ImitateConfig, ImitationReport, PolishConfig, PopulationFile, RoutePlay,
    Trainer, DEFAULT_CURRICULUM_GENERATIONS, FITNESS_VERSION,
};
use render::chart::{fitness_stall, HistoryPoint};
use sim::physics::{AgentAction, PHYSICS};
use sim::practice;
use sim::reach;
use sim::{Level, Outcome, SIM_VERSION};

use crate::args::{Args, ParseError};

pub(crate) const TRAIN_OPTS: &[&str] = &[
    "level",
    "suite",
    "generations",
    "pop",
    "seed",
    "mut-rate",
    "mut-sigma",
    "elite",
    "tournament",
    "crossover",
    "action-repeat",
    "fine-children",
    "fine-sigma-scale",
    "jump-timing-share",
    "repair-children",
    "commitment",
    "mastery",
    "no-mastery",
    "polish",
    "polish-generations",
    "polish-batch",
    "polish-sigma",
    "validation-episodes",
    "no-validation",
    "stagnation",
    "immigrants",
    "archive-children",
    "archive-on-diversity",
    "technique-children",
    "cut-stuck",
    "cut-margin",
    "cut-refund",
    "novelty",
    "rank-pessimism",
    "reheat-factor",
    "growth-units",
    "growth-events",
    "lesson-every",
    "plateau-window",
    "plateau-cem-generations",
    "plateau-cem-events",
    "adaptive-sigma",
    "feedback",
    "hidden",
    "trials",
    "no-auto-trials",
    "common-episodes",
    "no-common-episodes",
    "race-episodes",
    "no-race-episodes",
    "resume",
    "from-champion",
    "from-route",
    "from-plan",
    "from-demo",
    "plan-depth",
    "plan-beam",
    "plan-bonus",
    "plan-bonus-generations",
    "snapshot-every",
    "snapshot-probes",
    "warm",
    "clone-epochs",
    "leg-rounds",
    "leg-rollouts",
    "replan-depth",
    "dagger-rounds",
    "correction-epochs",
    "no-meta",
    "no-jump-labels",
    "route-seed",
    "curriculum",
    "curriculum-generations",
    "out",
    "csv",
    "telemetry",
    "max-seconds",
    "stop-on-win",
    "stop-on-validated",
    "committee",
    "save-population",
    "generalist",
    "generalist-share",
    "quiet",
];
/// `es`: the baseline arm. Its flags are the ones that change what it measures — the batch, the
/// probe, the step — plus the cadence and the memory width, which have to match the GA's for the
/// comparison to be about the search rather than about the arithmetic.
const ES_OPTS: &[&str] = &[
    "level",
    "generations",
    "pop",
    "seed",
    "sigma",
    "learning-rate",
    "action-repeat",
    "feedback",
    "hidden",
    "trials",
    "max-seconds",
    "stop-on-win",
    "quiet",
];
const REACH_OPTS: &[&str] = &["level", "all", "levels-dir", "verbose", "timed", "seed"];
/// `fuzz`: the Phase 0 practice suite. `--out` writes the files; without it the command only
/// prints what it certified.
const FUZZ_OPTS: &[&str] = &["out"];

/// `routeplan`: the Phase 1 route compiler. One level, or `--all` over a directory of them.
const ROUTEPLAN_OPTS: &[&str] = &["level", "all", "levels-dir", "seed", "plan-depth", "out"];
const REPLAY_OPTS: &[&str] = &["champion", "level", "levels-dir"];
/// `committee`: the Item 10 gate as a command. It validates a saved committee against the run's
/// own fixed validation episodes and fails the shell if the vote loses to its best member.
const COMMITTEE_OPTS: &[&str] = &["file", "level", "levels-dir"];
const SCORE_OPTS: &[&str] = &["champion", "level", "suite", "levels-dir", "seed"];
const SHORTCUT_OPTS: &[&str] = &["champion", "level", "levels-dir", "seed"];
const HUNT_OPTS: &[&str] = &["level", "seed", "depth", "beam", "objective", "out"];
const SEGMENTS_OPTS: &[&str] = &["level", "seed", "depth", "beam", "out"];
const CAMPAIGN_OPTS: &[&str] = &[
    "level",
    "seed",
    "depth",
    "beam",
    "segment-generations",
    "generations",
    "pop",
    "out",
    "checkpoints",
    "fuse",
    "fuse-cross-rate",
];
const STYLES_OPTS: &[&str] = &["champion", "level", "levels-dir", "seed"];
const THINK_OPTS: &[&str] = &[
    "level",
    "seed",
    "horizon",
    "beam",
    "out",
    "trace",
    "plan",
    "plan-depth",
    "plan-beam",
];
const ISLANDS_OPTS: &[&str] = &[
    "level",
    "generations",
    "islands",
    "pop",
    "seed",
    "out",
    "migration-every",
    "stop-on-win",
];
const TAS_OPTS: &[&str] = &[
    "champion",
    "level",
    "levels-dir",
    "seed",
    "out",
    "iterations",
    "shift",
    "patience",
];
const BATTERY_OPTS: &[&str] = &["levels-dir", "seed", "depth", "beam", "verbose"];
const CODEX_OPTS: &[&str] = &[
    "champion",
    "level",
    "levels-dir",
    "codex-dir",
    "seed",
    "dry-run",
];
const BREAK_OPTS: &[&str] = &[
    "level",
    "champion",
    "levels-dir",
    "seed",
    "depth",
    "beam",
    "objective",
    "codex-dir",
    "dry-run",
];
const PLAY_OPTS: &[&str] = &["level", "seed"];
const RENDER_OPTS: &[&str] = &["level", "steps", "stride", "action", "seed"];
const SENSORS_OPTS: &[&str] = &[];

/// Turn a parser error into a message `main` will report as a usage error.
pub(crate) fn usage<T>(r: Result<T, ParseError>) -> Result<T, String> {
    r.map_err(|e| format!("usage: {e}"))
}

pub(crate) fn load_level(path: &str) -> Result<Level, String> {
    Level::from_json_file(path)
}

/// `train --polish`: refine one champion's output layer without touching the rest of its brain.
///
/// The champion is the *seed* (`--from-champion`) and the level is the claim it is judged against,
/// so this goes through [`ChampionFile::load_for_level`] rather than `load_seed`: a polish that
/// ranked a genome on a level it was not trained on would be measuring an untested claim. The
/// result is written as `<level>-polished.json`, never over the input: the file the polish started
/// from is the evidence that it improved something, and replacing it at the moment the comparison
/// matters is the silent-replacement failure this file's training path already refuses.
fn polish_champion(
    a: &Args,
    level: &Level,
    level_path: &str,
    out_dir: &str,
    quiet: bool,
) -> Result<(), String> {
    if a.opt("suite").is_some() {
        return Err(
            "usage: --polish refines one champion on one level; it does not take --suite".into(),
        );
    }
    let seed_path = a.opt("from-champion").ok_or_else(|| {
        "usage: --polish reads the genome from --from-champion <path>; a polish without a champion \
         is a training run wearing the wrong name"
            .to_string()
    })?;
    let champ = ChampionFile::load_for_level(seed_path, level)?;
    let genome = champ.to_genome()?;
    let mut cfg = PolishConfig {
        generations: usage(a.usize_opt("polish-generations", 30))? as u32,
        batch: usage(a.usize_opt("polish-batch", 32))?,
        sigma: usage(a.f32_opt("polish-sigma", 0.08))?,
        master_seed: usage(a.u64_opt("seed", 1))?,
        trials: usage(a.usize_opt("trials", 3))? as u32,
        action_repeat: champ.metadata.action_repeat,
        ..PolishConfig::default()
    };
    cfg = cfg.sanitized();
    // The held-out set is deliberately *not* `cfg.seeds()`: a candidate ranked on seeds it was
    // also certified on would pass by construction. A different salt and a fresh count is the
    // cheapest honest split, and it is the same standard `--stop-on-validated` applies.
    let held_out = usage(a.usize_opt("validation-episodes", 8))? as u32;
    let validate: Vec<u64> = (0..held_out.max(1))
        .map(|i| {
            cfg.master_seed
                .wrapping_mul(0xD1B5_4A32_D192_ED03)
                .wrapping_add(i as u64 + 1)
        })
        .collect();

    if !quiet {
        println!(
            "polish  {} on {} (cadence {})",
            seed_path, level.name, cfg.action_repeat
        );
        println!(
            "cem     {} generations  batch {}  sigma {:.3}  trials {}  held out {}",
            cfg.generations,
            cfg.batch,
            cfg.sigma,
            cfg.trials,
            validate.len()
        );
    }
    let started = Instant::now();
    let report = polish(level, &genome, cfg, &validate);
    let secs = started.elapsed().as_secs_f64();

    match &report.genome {
        Some(polished) => {
            let eval = EvalConfig {
                action_repeat: cfg.action_repeat,
                cut_stuck_frames: 0,
            };
            let result = run_episode(level, polished, &eval, validate[0]);
            let mut ga_cfg = GaConfig {
                action_repeat: cfg.action_repeat,
                ..GaConfig::default()
            };
            ga_cfg.shape = polished.shape.clone();
            let file =
                ChampionFile::new(polished, &result, level, champ.metadata.generation, &ga_cfg);
            let stem = Path::new(level_path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| level.name.clone());
            let out = Path::new(out_dir).join(format!("{stem}-polished.json"));
            file.save(&out)?;
            println!("{}", report.note);
            println!("wrote {} ({:.1} s)", out.display(), secs);
        }
        None => {
            println!("{}", report.note);
            println!("nothing written ({:.1} s)", secs);
        }
    }
    Ok(())
}

fn parse_action(name: &str) -> Result<AgentAction, String> {
    AgentAction::ALL
        .iter()
        .copied()
        .find(|a| a.name() == name)
        .ok_or_else(|| {
            let options: Vec<&str> = AgentAction::ALL.iter().map(|a| a.name()).collect();
            format!(
                "unknown action {name:?}; expected one of {}",
                options.join(", ")
            )
        })
}

// ---------------------------------------------------------------------------
// train
// ---------------------------------------------------------------------------

pub fn train(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(TRAIN_OPTS))?;
    // `--suite` and `--level` are alternatives rather than two spellings of one thing: a suite
    // *is* its list, so a run that names both has two answers to "what is being scored" and the
    // honest response is to refuse rather than to pick one of them silently.
    if a.opt("suite").is_some() && a.opt("level").is_some() {
        return Err(
            "usage: --suite and --level are alternatives (--suite names every level it scores on)\n         \
             try `train-cli --help`"
                .into(),
        );
    }
    let level_paths = match a.opt("suite") {
        Some(list) => parse_level_list(list)?,
        None => vec![usage(a.required_path("level"))?],
    };
    let mut levels: Vec<Level> = Vec::with_capacity(level_paths.len());
    for path in &level_paths {
        levels.push(load_level(path)?);
    }
    // A suite is more than one level; one level is an ordinary run however it was named. That is
    // what keeps `--suite levels/l1-flat-run.json` a legal no-op rather than a second code path.
    let suite = levels.len() > 1;
    // The level a champion is recorded on: the first of the suite. See the note printed with it.
    let level_path = level_paths[0].clone();
    let level = levels[0].clone();

    let generations = usage(a.usize_opt("generations", 30))?;
    let out_dir = a.string("out", "champions");
    let quiet = a.has_flag("quiet");
    // **The post-solve polish** (WP9): a separate, smaller search over the champion's output
    // layer, run before the GA loop would start. It takes the genome from `--from-champion`,
    // because a polish without a champion is a training run wearing the wrong name.
    if a.has_flag("polish") {
        return polish_champion(a, &level, &level_path, &out_dir, quiet);
    }
    let stop_on_win = a.has_flag("stop-on-win");
    // The stricter stop: the *record* has to be solved on its held-out set, not merely a generation
    // solved on its own three patrol phases. See `GenerationReport::record_solved`.
    let stop_on_validated = a.has_flag("stop-on-validated");
    if stop_on_validated && !stop_on_win {
        return Err(
            "usage: --stop-on-validated refines when to stop, it does not start stopping: pass \
             --stop-on-win beside it"
                .into(),
        );
    }
    if stop_on_validated && a.has_flag("no-validation") {
        return Err(
            "usage: --stop-on-validated needs a validation set, and --no-validation switches the \
             one it reads off"
                .into(),
        );
    }
    if stop_on_validated && suite {
        // The suite's own stop is already the conservative one (every level plus every trial), and
        // the record's validation set is drawn on **one** level. Accepting the pair would be a flag
        // that is silently ignored, which this file refuses everywhere.
        return Err(
            "usage: --stop-on-validated reads one level's validation set; a suite already stops \
             when every level is won, so pass --stop-on-win alone"
                .into(),
        );
    }
    // The stall chain (§12 M5), the terminal's copy of the window's: fall back to an
    // easier level when the run stops moving. With no `--curriculum` the fallback is
    // derived instead (finding 50) — the level itself, starting from the furthest stand
    // the run's best genome has made — and `--curriculum-generations 0` takes the whole
    // detour out of the run, which is the protocol every measurement before finding 50
    // was taken under.
    let curriculum_level = match a.opt("curriculum") {
        Some(path) => Some(load_level(path)?),
        None => None,
    };
    let curriculum_generations = usage(a.usize_opt(
        "curriculum-generations",
        DEFAULT_CURRICULUM_GENERATIONS as usize,
    ))? as u32;
    if curriculum_level.is_some() && curriculum_generations == 0 {
        return Err(
            "usage: --curriculum-generations must be at least 1 (with --curriculum)".into(),
        );
    }
    if suite && curriculum_level.is_some() {
        return Err(
            "usage: --curriculum falls back on one level; a suite is its own landscape".into(),
        );
    }
    let max_seconds: Option<f64> = match a.opt("max-seconds") {
        Some(_) => Some(usage(a.f64_opt("max-seconds", 0.0))?),
        None => None,
    };

    let mut cfg = train_config(a)?;
    // **The level tells the trainer what brain it needs** (WP2): a timed mechanic raises the
    // feedback width, a patrol level raises the pessimism, and an explicit flag is never
    // overridden. The header prints the answer and its reasons below.
    let auto_shape = evo::apply_auto_shape(
        &mut cfg,
        &levels,
        a.opt("feedback").is_some(),
        a.opt("rank-pessimism").is_some(),
    );
    if suite && cfg.plan_bonus > 0.0 {
        // One guide describes one level's route, and the ranking loop prices it on `levels[0]`.
        // Accepting the pair would shape a suite with a bonus that only the first level can earn,
        // which is a flag silently doing half of what it says.
        return Err(
            "usage: --plan-bonus follows one level's plan; a suite ranks by its worst level, so \
             pass a single --level"
                .into(),
        );
    }
    // What the header prints, when the header prints the *shape* of the search space: the
    // trainer's own clamps applied first, because a header that quotes `--hidden 1000` while the
    // run trains at 64 would be exactly the sort of number this file's comments exist to prevent.
    let shown = cfg.clone().sanitized(None);

    if !quiet {
        // A suite prints one line per level, because "which levels" *is* the experiment: the
        // question a suite run asks is whether one policy can hold all of them at once, and a
        // header that named only the first would hide half of the answer. A single level prints
        // the same three lines it always did.
        if suite {
            println!(
                "suite   {} levels, ranked by the worst of them (REVIEW.md §4 item 5)",
                levels.len()
            );
        }
        for l in &levels {
            println!("level   {} ({}x{})", l.name, l.width, l.height);
            println!("hash    {}", l.content_hash_string());
            println!("oracle  {}", reachability_line(l));
        }
        println!(
            "ga      pop {}  elite {}  tournament {}  sigma {:.2}  rate {:.3}  crossover {:.2}  action_repeat {}  trials {}",
            cfg.pop_size,
            cfg.elite,
            cfg.tournament,
            cfg.mutation_sigma,
            cfg.mutation_rate,
            cfg.crossover_rate,
            cfg.action_repeat,
            // The **effective** count, not the flag: a patrol level is judged on three phases
            // even when `--trials` is left at 1, and a header that printed the flag made that
            // invisible (the l11 run that won once said "trials 1" while the rule it named
            // would have said three).
            cfg.effective_trials(&levels[0])
        );
        // The net itself, which no line above describes: `ga` is the population, `search` is how
        // the population moves, and this is what it moves *in*. `--hidden` and `--feedback` are
        // the two flags that change the size of the search space by thousands of genes, and a run
        // whose header stayed silent about them would be a table row nobody could reproduce. The
        // rank is here for the same reason and a different one: it changes what "best" means, so a
        // reading taken with it on is not comparable with one taken with it off.
        println!(
            "net     {}  rank {}",
            shown.shape.label(),
            rank_label(shown.rank_pessimism)
        );
        if let Some(auto) = &auto_shape {
            println!("auto    {}", auto.line());
        }
        if cfg.mastery {
            println!(
                "league  mastery armed: once the record solves, winners are ranked by the clock \
                 (the crowned fitness still picks the champion)"
            );
        }
        // The episode budget is per level and comes from the trainer's own rule (three patrol
        // phases where a level has a patrol, one where it does not), so a suite prints what each
        // level actually costs rather than the flag's single number.
        if suite {
            let per: Vec<String> = levels
                .iter()
                .map(|l| format!("{} {}", cfg.effective_trials(l), l.name))
                .collect();
            println!(
                "trials  {} episodes per genome per generation",
                per.join(" | ")
            );
        }
        // The search's own shape, printed because it is what the run will *do* rather than
        // what it starts from: fine children are the second, local mutation scale, the
        // immigrants are reheated from the run's best-ever genome, the archive children are
        // bred from the quality-diversity record, and with adaptive sigma the printed 0.30 is
        // a starting value the run moves on its own.
        println!(
            "search  fine {}  sigma {}  immigrants {} reheated at {:.2}x ({:.2}) after {} still generations  qd {} from the archive",
            cfg.fine_children,
            if cfg.adaptive_sigma {
                format!("self-tuning {:.2}-{:.2} from {:.2}", MUTATION_SIGMA_RANGE[0], MUTATION_SIGMA_RANGE[1], cfg.mutation_sigma)
            } else {
                format!("fixed at {:.2}", cfg.mutation_sigma)
            },
            cfg.immigrants,
            cfg.reheat_factor,
            cfg.reheat_sigma(),
            cfg.stagnation_generations,
            cfg.archive_children
        );
        if cfg.technique_children > 0 {
            println!(
                "tech    {} children per generation bred from technique cells (Item 11)",
                cfg.technique_children
            );
        }
        if cfg.trials > 1 && !level.has_patrols() {
            println!(
                "note    --trials {} on a level with no patrols: every trial is the same episode,\n         so this only multiplies the cost",
                cfg.trials
            );
        }
        println!(
            "seed    {}   (sim {} / sensors v{} / fitness v{})",
            cfg.master_seed,
            SIM_VERSION,
            sim::sensors::VERSION,
            FITNESS_VERSION
        );
        println!(
            "\n{:>4}  {:>10}  {:>10}  {:>10}  {:>9}  {:>5}  {:>7}  {:>6}  {:<16}  learning",
            "gen", "max", "avg", "min", "wins", "sigma", "secs", "solved", "clock"
        );
    }

    // Hoisted out of the sign-off: the stall chain names its artifacts (the population
    // checkpoint, the derived practice level) after the run's stem, and a name computed
    // twice is a name two code paths can disagree about.
    let stem = Path::new(&level_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| level.name.clone());
    // A suite champion never takes the name a single-level champion of the same level would:
    // `<level>.json` in the champions directory belongs to the run that trained on that level
    // alone, and overwriting it with a policy that was *selected* on three levels would be the
    // silent-replacement failure `REVIEW.md` §2.8 records, in the one directory this tool writes
    // to without being asked.
    let stem = if suite { format!("{stem}-suite") } else { stem };

    // Where the population comes from: a fresh random one, a checkpoint, or a champion plus a
    // shell around it (`seeded_trainer`). It is one call rather than three branches here because
    // `bench` starts its arms the same way, and two copies of "how a run begins" is two answers
    // to "what did this table measure".
    let mut trainer = seeded_trainer(cfg, &level, a)?;
    let effective = trainer.config.clone();
    // **Plan-guided selection (WP5).** The guide is the planner's own trajectory; the bonus is
    // selection-only and decays to zero, so the champion and every recorded reading stay v9.
    if let Some(waypoints) = attach_plan(&mut trainer, &level, a, &effective)? {
        println!(
            "plan    {} waypoints attached at bonus {:.0}, decaying to zero over {} generations",
            waypoints, effective.plan_bonus, effective.plan_bonus_generations
        );
    }
    let started = Instant::now();
    let mut solved_at: Option<u32> = None;
    let mut generation_secs: Vec<f64> = Vec::with_capacity(generations);
    let mut history: Vec<HistoryPoint> = Vec::with_capacity(generations);
    let mut curriculum_taken: Option<String> = None;
    // The run's standing best when the last derived practice step fired: the gate that
    // lets a practice run take a step per *stall spell* rather than one per run, and
    // never two in the same spell.
    let mut detour_best = f32::NEG_INFINITY;

    if suite && !quiet {
        println!(
            "\n{:>4}  {:>10}  {:>10}  {:>10}  {:>9}  {:>5}  {:>7}  {:>5}  {:>6}  {:<16}  learning",
            "gen", "max", "avg", "min", "wins", "sigma", "secs", "suite", "solved", "clock"
        );
    }

    // **Phase 0 learning telemetry.** One JSON object per generation, so a run's learning curve
    // is a file a later phase can gate on rather than a screenshot. Opened before the loop
    // (and written inside it) so a run killed at `--max-seconds` still leaves every generation
    // it actually ran.
    let telemetry_path = a.opt("telemetry").map(|p| p.to_string());
    let mut telemetry_out = match &telemetry_path {
        Some(path) => {
            if let Some(parent) = Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            let file = fs::File::create(path)
                .map_err(|e| format!("cannot write {path}: {e}"))?;
            Some(std::io::BufWriter::new(file))
        }
        None => None,
    };

    for _ in 0..generations {
        let t0 = Instant::now();
        let report = if suite {
            trainer.step_suite(&levels)
        } else {
            trainer.step(&level)
        };
        let secs = t0.elapsed().as_secs_f64();
        generation_secs.push(secs);
        history.push(HistoryPoint {
            generation: report.generation,
            max_fitness: report.max_fitness,
            avg_fitness: report.avg_fitness,
            wins: report.wins,
        });
        if let Some(out) = telemetry_out.as_mut() {
            // **One line per level.** A suite's generation is not one census, so writing only
            // `report.telemetry` would label level zero's record as the run's. Each line carries
            // its own `level`, so a reader groups by `generation` and never has to guess.
            for record in &report.per_level {
                let line = serde_json::to_string(record)
                    .map_err(|e| format!("cannot serialize telemetry: {e}"))?;
                writeln!(out, "{line}").map_err(|e| {
                    format!(
                        "cannot write {}: {e}",
                        telemetry_path.as_deref().unwrap_or("telemetry")
                    )
                })?;
            }
        }

        // What "solved" means depends on the run, and it is the **conservative** reading on both.
        // On one level it is the first genome that won every episode it was judged on, which on a
        // deterministic level is simply the first genome on the flag — and on a walker level is not
        // the first genome that drew a quiet patrol phase. On a suite it is a genome that won
        // **every level and every trial of it**, because the claim a suite run makes is a
        // generalisation claim: stopping on the first level won would end the run on exactly the
        // specialist the reward was built to discourage.
        let solved_now = if suite {
            report.suite_solved > 0
        } else if stop_on_validated {
            report.record_solved
        } else {
            report.solved > 0
        };
        if solved_at.is_none() && solved_now {
            solved_at = Some(report.generation);
        }

        if !quiet {
            if suite {
                println!(
                    "{:>4}  {:>10.2}  {:>10.2}  {:>10.2}  {:>5}/{:<3}  {:>5.2}  {:>7.2}  {:>3}/{:<2}  {:>6}  {:<16}  l0: {}{}",
                    report.generation,
                    report.max_fitness,
                    report.avg_fitness,
                    report.min_fitness,
                    report.wins,
                    effective.pop_size,
                    report.mutation_sigma,
                    secs,
                    report.suite_solved,
                    effective.pop_size,
                    report.solved,
                    report.telemetry.clock_line(),
                    report.telemetry.best_end_line(),
                    if report.genius { "  <-- GENIUS AGENT" } else { "" }
                );
            } else {
                println!(
                    "{:>4}  {:>10.2}  {:>10.2}  {:>10.2}  {:>5}/{:<3}  {:>5.2}  {:>7.2}  {:>6}  {:<16}  {}{}",
                    report.generation,
                    report.max_fitness,
                    report.avg_fitness,
                    report.min_fitness,
                    report.wins,
                    effective.pop_size,
                    report.mutation_sigma,
                    secs,
                    report.solved,
                    report.telemetry.clock_line(),
                    report.telemetry.best_end_line(),
                    if report.genius {
                        "  <-- GENIUS AGENT"
                    } else {
                        ""
                    }
                );
                // Every level's solved rate, so a suite run says which level is the wall rather
                // than reporting one level's census as if it were the run's.
                println!(
                    "{}",
                    report
                        .per_level
                        .iter()
                        .map(|t| format!("{}: {}/{}", t.level, t.solved, t.pop))
                        .collect::<Vec<_>>()
                        .join("  |  ")
                );
            }
        }

        if stop_on_win && solved_at.is_some() {
            break;
        }
        if let Some(limit) = max_seconds {
            if started.elapsed().as_secs_f64() >= limit {
                if !quiet {
                    println!("--max-seconds {limit:.0} reached; stopping early");
                }
                break;
            }
        }

        // The stall chain, on the same stall rule the chart reads (§12 M5). The
        // authored easier level is the run's first choice and steps back once per run
        // (the plan's rule); without one the run practises where it is stuck
        // (finding 50) — one step per *stall spell*, so a run can work through a spot,
        // keep the gain, and meet the next. The detour's landscape is written beside
        // the checkpoint so it can be opened and played.
        let armed = match &curriculum_level {
            Some(_) => curriculum_taken.is_none(),
            None => report.max_fitness > detour_best,
        };
        if armed && curriculum_generations > 0 && !suite {
            if let Some(stall) = fitness_stall(&history) {
                let practice;
                let easier: &Level = match &curriculum_level {
                    Some(easier) => easier,
                    None => {
                        let derived = match practice_level_for_stall(&trainer, &level) {
                            Ok(derived) => derived,
                            Err(reason) => {
                                // Nowhere to practise from is part of the run's story,
                                // not a missing line: a silent skip is indistinguishable
                                // from a dead stall chain. Said once per stall spell —
                                // the arm closes until the run finds something better —
                                // and kept in the checkpoint note like any step.
                                detour_best = report.max_fitness;
                                let summary = format!(
                                    "curriculum: {reason}; the run stays where it is ({})",
                                    stall.label()
                                );
                                if !quiet {
                                    println!("{summary}");
                                }
                                curriculum_taken = Some(match curriculum_taken {
                                    Some(prev) => format!("{prev}\n{summary}"),
                                    None => summary,
                                });
                                continue;
                            }
                        };
                        practice = derived;
                        &practice
                    }
                };
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
                        effective.mutation_rate,
                        &stall.label(),
                    )?
                } else {
                    curriculum_step(
                        &mut trainer,
                        &level,
                        easier,
                        curriculum_generations,
                        &path,
                        effective.mutation_rate,
                        &stall.label(),
                    )?
                };
                detour_best = report.max_fitness;
                let summary = step.note(&level.name);
                if !quiet {
                    println!("{summary}");
                }
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
        if !quiet {
            println!("wrote   {path} (one learning record per generation)");
        }
    }

    // **A warm start with no search is a supported run, not a failed one.** `--generations 0`
    // exists so a gate can read the clone, its mastery and its skill line without training, and
    // the run really did all of that; reaching the champion block below would turn it into exit 1
    // with the whole report already on stdout. Every arm of `tools/phase3_gate.py` used to report
    // failure for exactly this reason, which made the status of a 32-run gate unreadable.
    if generations == 0 {
        if !quiet {
            println!("stop    --generations 0: warm start only, no champion written");
        }
        return Ok(());
    }

    let elapsed = started.elapsed();
    let best = trainer
        .best_ever()
        .ok_or_else(|| "no generation ran, so there is no champion".to_string())?;
    let mut champion = ChampionFile::new(
        &best.genome,
        &best.result,
        &level,
        best.generation,
        &effective,
    );
    // Item 8, step 3: the champion carries its own training/held-out gap, so a phase-memorising
    // record is visible in the artifact rather than inferred from a table later. Skipped on a
    // suite: the validation set is one level's, and a single number over the pack would average
    // away the level that memorised.
    if a.opt("suite").is_none() {
        champion.metadata.phase_gap = evo::measure_phase_gap(
            &level,
            &best.genome,
            &effective,
            best.generation,
            best.validated,
        );
    }

    // A **seeded** run writes to a name that steps aside rather than over the file it started
    // from. On a fine-tune — `--from-champion champions/l5-tightrope.json`, same level, same
    // directory — the default path *is* the seed's own path, so the run would replace the evidence
    // it started from with the result it produced, at the moment that evidence is most wanted
    // (`REVIEW.md` §2.8's silent replacement, in the one directory this tool writes to unasked).
    let champion_path = match a.opt("from-champion") {
        Some(seed) if same_file(seed, &Path::new(&out_dir).join(format!("{stem}.json"))) => {
            Path::new(&out_dir).join(format!("{stem}-seeded.json"))
        }
        _ => Path::new(&out_dir).join(format!("{stem}.json")),
    };
    champion.save(&champion_path)?;

    // Item 10: the run's top `k` genomes as a committee, written beside the champion and gated
    // before it is offered as the level's solver. The committee never replaces the champion and
    // no difficulty-table row reads it — it is a presentation of the same run's result.
    if a.opt("committee").is_some() {
        let k = usage(a.usize_opt("committee", 5))?.clamp(1, effective.pop_size);
        let mut pool: Vec<evo::Genome> = trainer.genomes.clone();
        pool.sort_by(|x, y| y.fitness.total_cmp(&x.fitness));
        let mut members: Vec<evo::Genome> = Vec::new();
        // The record first: it is the genome the run proved, and index 0 is the tie-break's
        // first voice, so the order is the claim.
        if let Some(record) = trainer.best_ever().map(|b| b.genome.clone()) {
            members.push(record);
        }
        for g in pool {
            if members.len() >= k {
                break;
            }
            if members
                .iter()
                .any(|m| m.weights == g.weights && m.biases == g.biases)
            {
                continue;
            }
            members.push(g);
        }
        let file = CommitteeFile::new(
            &level,
            effective.master_seed,
            effective.validation_episodes,
            effective.action_repeat,
            &members,
        )?;
        let path = CommitteeFile::path_for(&level);
        file.save(&path)?;
        let report = file.validate(&level)?;
        println!(
            "committee {} members, {} of {} validation episodes (best member {} of {})",
            members.len(),
            report.won,
            report.episodes,
            report.best_member_won,
            report.episodes
        );
        println!("saved   {}", path.display());
    }

    // Item 6's supply side: write the run's final population, so a suite run across the pack
    // can be the standing generalist that `--generalist` seeds from. The file is an ordinary
    // population checkpoint, notes included, not a second format.
    if let Some(path) = a.opt("save-population") {
        let file = PopulationFile::new(
            &trainer.genomes,
            trainer.generation,
            &level,
            &effective,
            format!(
                "final population of a {} generation run on {}",
                trainer.generation, level.name
            ),
        );
        file.save(path)?;
        println!("wrote   {path}");
    }

    if let Some(csv_path) = a.opt("csv") {
        write_csv(csv_path, trainer.history())?;
        if !quiet {
            println!("wrote {csv_path}");
        }
    }

    // The timing summary is the M2 benchmark requirement: record the real
    // seconds-per-generation on this machine rather than trusting the plan's
    // extrapolation from a 2009 benchmark.
    let mean = if generation_secs.is_empty() {
        0.0
    } else {
        generation_secs.iter().sum::<f64>() / generation_secs.len() as f64
    };
    let mut sorted = generation_secs.clone();
    sorted.sort_by(|x, y| x.total_cmp(y));
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let best_secs = sorted.first().copied().unwrap_or(0.0);

    println!();
    match solved_at {
        Some(g) if suite => println!("solved at generation {g} (every level won)"),
        Some(g) if stop_on_validated => {
            println!("solved at generation {g} (the record won every validation episode)")
        }
        Some(g) => println!("solved at generation {g} (first win)"),
        None => println!(
            "not solved in {} generations; best-evidence: max fitness {:.2}",
            generation_secs.len(),
            champion.fitness
        ),
    }
    // Item 8's reading, beside the verdict it qualifies: a record that wins every phase it trained
    // on and half the ones it did not is a different claim from one that wins both.
    if let Some(gap) = champion.metadata.phase_gap {
        println!(
            "gap     training {}/{} ({:.0} %)  held-out {}/{} ({:.0} %)  gap {:+.2}",
            gap.train_wins,
            gap.train_episodes,
            gap.training_rate() * 100.0,
            gap.held_wins,
            gap.held_episodes,
            gap.held_rate() * 100.0,
            gap.gap()
        );
    }
    // What the search did with its own knobs, not just what it scored. A run that never
    // stalled and a run that stalled twice look identical on a fitness chart that has
    // already flattened out, and "did it recover" is exactly what a reader is asking.
    let immigrants: usize = trainer.history().iter().map(|r| r.immigrants).sum();
    let bursts = trainer
        .history()
        .iter()
        .filter(|r| r.immigrants > 0)
        .count();
    println!(
        "search  sigma {:.2} -> {:.3}  {}  reheated {} in {} burst{}",
        effective.mutation_sigma,
        trainer.mutation_sigma(),
        if effective.adaptive_sigma {
            "self-tuned"
        } else {
            "fixed"
        },
        immigrants,
        bursts,
        if bursts == 1 { "" } else { "s" }
    );
    // And the quality-diversity half of the search, on its own line because it is a different
    // kind of fact: the immigrants above are a *reaction* to a plateau, while the archive
    // children arrive every generation and the niche count says how spread out the population
    // is. A converged run that is still finding bests is exactly the case the two numbers
    // disagree about, and it is the case a summary exists to show.
    let archive_children: usize = trainer.history().iter().map(|r| r.archive_children).sum();
    println!(
        "qd      {} children bred from the archive, {} of {} niches filled",
        archive_children,
        trainer.archive().occupied(),
        evo::ARCHIVE_BEHAVIOUR_CELLS
    );
    // Item 11's own reading, on the same line as the archive it draws from: the children spent
    // and the cells they came from. The plan's arm is judged on both numbers together, so a run
    // that reports one without the other is not the measurement.
    if trainer.config.technique_children > 0 {
        println!(
            "tech    {} children bred from {} technique cells",
            trainer.technique_children_bred(),
            trainer.technique().occupied()
        );
    }
    // Item 5, when the cut was on: the episodes it ended and the share of the pass that is, so
    // the arm's wall-clock number has a mechanism beside it.
    if trainer.config.cut_stuck_frames > 0 {
        println!(
            "cut     {} of {} episodes ended after {} no-progress frames",
            trainer.cut_early(),
            trainer.history().len() * trainer.genomes.len(),
            trainer.config.cut_stuck_frames
        );
        if trainer.config.cut_refund {
            println!(
                "refund  {} genome(s) re-read on one extra trial with the ticks the cut saved",
                trainer.cut_refunds()
            );
        }
    }
    // **Technique (WP13).** Reporting only, and on its own line because nothing in the crowned
    // fitness reads it: the archive keeps genomes for *how* they moved, and the firsts are the
    // named thresholds a presentation can talk about. A zero here is a finding about the pack, not
    // an error — the baseline may simply never stomp a walker as transport.
    let technique = trainer.technique();
    let totals = technique.seen();
    println!(
        "tech    {} technique cells | longest stomp chain {}, wall jumps {}, springs {}, crumble tiles burned {}, longest flight {:.2}s, peak speed {:.2}",
        technique.occupied(),
        totals.max_stomp_chain,
        totals.wall_jumps,
        totals.springs,
        totals.crumble_burned,
        totals.max_air_ticks as f32 * sim::DT,
        totals.peak_speed
    );
    for first in technique.firsts().iter().take(6) {
        println!(
            "first   {} (gen {}, genome {}) — {}",
            first.name, first.generation, first.genome_id, first.line
        );
    }
    if effective.snapshot_every > 0 {
        let (probes, wins) = trainer.snapshot_stats();
        println!(
            "snapshot {} frontier cells | {} probes started from one | {} won from there",
            trainer.snapshots().len(),
            probes,
            wins
        );
    }
    // The clock, and on its own line because it is the one reading here that is *not* about the
    // search: since fitness v8 a win is paid for the time it did not spend, so the number the
    // population was rewarded for is a number the terminal owes a reader. It is the run's own record
    // rather than the champion's episode, and those can be different agents: the best-scoring genome
    // is not necessarily the quickest win, and this is the time a person would race.
    match trainer.fastest() {
        Some(fast) => println!(
            "clock   {} ({} frames) at the flag (gen {} id {}, {:.0}% of the level's {} unspent, {} of {} finish bands filled)",
            sim::format_time(fast.frames),
            fast.frames,
            fast.generation,
            fast.genome_id,
            fast.spare() * 100.0,
            sim::format_time(fast.budget_frames),
            trainer.archive().time_bands(),
            evo::ARCHIVE_TIME_BINS
        ),
        None => println!("clock   no win to time: nothing reached the flag"),
    }
    if suite {
        // The claim a suite run makes, level by level, evaluated on the champion with the same
        // seeds its own generation used — so these are the numbers the run was selected on rather
        // than a fresh sample of them.
        let (_, per_level, _) =
            evo::ga::suite_score(&levels, &best.genome, &effective, best.generation);
        let listed: Vec<String> = levels
            .iter()
            .zip(&per_level)
            .map(|(l, f)| format!("{} {:.1}", l.name, f))
            .collect();
        println!(
            "suite   {} | worst {:.1} | {} of {} levels won",
            listed.join(" | "),
            per_level.iter().copied().fold(f32::INFINITY, f32::min),
            best.result.suite_wins,
            levels.len()
        );
        println!(
            "note    the champion is recorded on {} — fitness and frames below are that level's\n         episode, because a champion file is a claim about one level that `replay` checks",
            levels[0].name
        );
    }
    // The record's **held-out** reading, when the run took one: the episode below is the episode
    // that earned the record (`replay` reproduces it), and this is the same genome on episodes no
    // generation drew. A record that only wins its own episode shows up here as a gap between the
    // two numbers, which is exactly what it is.
    if let Some(validated) = best.validated {
        println!(
            "held    {:.2} mean, {} of {} fixed validation episodes won{} — see GaConfig::validation_episodes",
            validated.mean,
            validated.wins,
            validated.episodes,
            if validated.solved() {
                " (every one)"
            } else {
                ""
            }
        );
    }
    println!(
        "best    gen {} id {} fitness {:.2} ({} after {} frames, best_x {:.1}, jumps {}, checkpoints {})",
        champion.metadata.generation,
        champion.metadata.genome_id,
        champion.fitness,
        champion.metadata.outcome,
        champion.metadata.frames,
        champion.metadata.best_x,
        champion.metadata.jumps,
        champion.metadata.checkpoints
    );
    println!(
        "timing  {:>7.3} s/gen mean | {:>7.3} median | {:>7.3} best | {} generations in {:.1} s",
        mean,
        median,
        best_secs,
        generation_secs.len(),
        elapsed.as_secs_f64()
    );
    println!(
        "        ({:.0} population-episodes per second)",
        (generation_secs.len() * effective.pop_size) as f64 / elapsed.as_secs_f64().max(1e-9)
    );
    println!("champion {}", champion_path.display());
    println!("         {}", champion.summary());
    // Printed after the summary, and always: a run whose shape changed mid-flight
    // should say so where the numbers are.
    if let Some(summary) = &curriculum_taken {
        println!("{summary}");
    }

    Ok(())
}

/// The hidden-layer widths `--hidden` asks for, or the trainer's own default.
///
/// A comma list — `--hidden 32`, `--hidden 32,16` — because the parser takes one value per option
/// (`args.rs`) and because the list *is* the topology: the shipped net is one layer of 16, and the
/// question this flag exists to ask is whether a wider one searches better, which is a question
/// about a list rather than about a number. The widths are bounded by
/// [`evo::genome::MAX_HIDDEN_WIDTH`] and the count by [`evo::genome::MAX_HIDDEN_LAYERS`] in
/// `GaConfig::sanitized`, which is where the *run* is protected; what is refused here is the
/// spelling mistakes that would otherwise be silently repaired — a zero-width layer, a stray
/// comma, a word.
pub(crate) fn parse_hidden(a: &Args, default: &[usize]) -> Result<Vec<usize>, String> {
    let Some(spec) = a.opt("hidden") else {
        return Ok(default.to_vec());
    };
    let mut widths = Vec::new();
    for piece in spec.split(',') {
        let piece = piece.trim();
        let width = piece.parse::<usize>().map_err(|_| {
            format!(
                "usage: --hidden expects layer widths, e.g. --hidden 32 or --hidden 32,16 (got {piece:?})"
            )
        })?;
        if width == 0 {
            return Err("usage: --hidden 0 is not a narrower network, it is a broken one".into());
        }
        widths.push(width);
    }
    if widths.is_empty() {
        return Err("usage: --hidden needs at least one width, e.g. --hidden 32".into());
    }
    Ok(widths)
}

/// How [`evo::GaConfig::rank_pessimism`] reads on a header line.
///
/// Spelled out rather than left as `rank 0.00` for the same reason the sigma is printed with its
/// units: the two ends of this knob are different *questions* about a genome ("what does it usually
/// do" against "what does it do when the world is at its worst"), and "0.35" alone says nothing
/// about which of them a table row was taken with.
fn rank_label(pessimism: f32) -> String {
    if pessimism <= 0.0 {
        "mean of a genome's trials".to_string()
    } else if pessimism >= 1.0 {
        "worst trial of a genome's trials".to_string()
    } else {
        format!("mean, less {:.2} of the gap to the worst trial", pessimism)
    }
}

/// Do these two spellings name the same file? Used only to keep a seeded run from overwriting its
/// own seed.
///
/// The output path usually does not exist yet, so its parent is the thing that has to resolve: a
/// file name is only the same file if the directory around it is. When the parent cannot be
/// resolved at all (it does not exist yet either) the answer is `false`, and the run keeps the
/// default name — the conservative direction, since the cost of a wrong `true` here is a champion
/// saved under a name nobody expects.
fn same_file(seed: &str, out: &Path) -> bool {
    let seed = match Path::new(seed).canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };
    if seed.file_name() != out.file_name() {
        return false;
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    seed.parent() == Some(parent.as_path())
}

/// The trainer a run starts from.
///
/// Three answers, and the two flags that give the second and third are alternatives rather than
/// two knobs: `--resume` names a **population** (the artifact a stall writes, ids and generation
/// included) and `--from-champion` names a **single genome** that [`Trainer::from_champion`] turns
/// into one. Naming both is two contradictory answers to "where does this run start", so it is a
/// usage error rather than a precedence rule.
///
/// Factored out of `train` because `bench` starts its arms here too: a protocol run has to begin
/// the way a training run begins, or a sweep would be measuring a different starting population
/// from the one the operator asked for.
///
/// Both paths announce themselves. A resumed population says what it is and whether the level moved
/// (`PopulationFile::level_note`), and a seeded champion does the same through
/// [`ChampionFile::level_note`] — because the fitness before and after such a step are *not*
/// comparable, and a run that quietly changed level between two chart lines would be the most
/// easily misread artifact this tool can produce.
#[allow(clippy::let_and_return)]
pub(crate) fn seeded_trainer(cfg: GaConfig, level: &Level, a: &Args) -> Result<Trainer, String> {
    let mut trainer = seeded_trainer_core(cfg, level, a)?;
    if let Some(path) = a.opt("generalist") {
        if trainer.config.generalist_share <= 0.0 {
            return Err(
                "--generalist needs a positive --generalist-share: a population offered and \
                 silently ignored is the failure this refusal exists to prevent"
                    .into(),
            );
        }
        let population = PopulationFile::load(path)?;
        println!("generalist {}", population.summary());
        if let Some(note) = population.level_note(level) {
            println!("note    {note}");
        }
        let elites = population.to_genomes()?;
        let seeded = trainer.seed_generalist(&elites)?;
        println!(
            "seeded  {seeded} generalist elite(s) into generation zero ({:.0} % of the population)",
            trainer.config.generalist_share * 100.0
        );
    }
    Ok(trainer)
}

fn seeded_trainer_core(cfg: GaConfig, level: &Level, a: &Args) -> Result<Trainer, String> {
    let resume = a.opt("resume");
    let from_champion = a.opt("from-champion");
    let from_route = a.has_flag("from-route");
    let from_plan = a.has_flag("from-plan");
    let from_demo = a.opt("from-demo");
    let named = usize::from(resume.is_some())
        + usize::from(from_champion.is_some())
        + usize::from(from_route)
        + usize::from(from_plan)
        + usize::from(from_demo.is_some());
    if named > 1 {
        return Err(
            "usage: --resume, --from-champion, --from-route, --from-plan and --from-demo are \
             alternatives: they are five answers to \"where does this run start from\""
                .into(),
        );
    }
    if from_route {
        return route_seeded_trainer(cfg, level, a);
    }
    if let Some(path) = from_demo {
        return demo_seeded_trainer(cfg, level, a, path.to_string());
    }
    if from_plan {
        return plan_seeded_trainer(cfg, level, a);
    }
    if let Some(path) = resume {
        let checkpoint = PopulationFile::load(path)?;
        println!("resuming {}", checkpoint.summary());
        if let Some(note) = checkpoint.level_note(level) {
            println!("note    {note}");
        }
        let mut trainer = Trainer::resume(cfg, checkpoint.to_genomes()?, checkpoint.generation)?;
        if let Some(record) = checkpoint.best_genome()? {
            trainer.seed_record_from_checkpoint(level, record)?;
        }
        Ok(trainer)
    } else if let Some(path) = from_champion {
        let champion = ChampionFile::load_seed(path)?;
        let note = champion.level_note(level);
        if let Some(note) = note {
            println!("note    {note}");
        }
        let genome = champion.to_genome()?;
        println!(
            "seeded  {} ({} genes, resumed as generation 0 of a new run)",
            champion.summary(),
            genome.n_genes()
        );
        let mut trainer = Trainer::from_champion(cfg, genome.clone())?;
        // The champion's own recorded episode is the one reading this build can reproduce, and
        // it is what lets a seeded run keep the record (and arm the league) from generation 0.
        trainer.seed_record(level, genome, champion.metadata.episode_seed)?;
        Ok(trainer)
    } else {
        Ok(Trainer::new(cfg))
    }
}

/// Attach the planner's route when `--plan-bonus` asks for it.
///
/// Shared by `train` and `bench`, for the same reason [`seeded_trainer`] is: an arm that claims
/// to measure the plan bonus has to carry the guide, or it is two copies of the default wearing
/// different labels. That failure was real — `bench` accepted `--plan-bonus` and never called
/// [`Trainer::seed_plan`], so two l11 arms with and without the flag produced byte-identical
/// seed rows.
///
/// Returns the waypoint count when a guide was attached, `None` when the bonus is off. A run that
/// asks for a bonus the planner cannot serve is an error rather than a silent zero: the operator
/// asked for a different search, and a search that quietly reverted is the worst measurement.
pub(crate) fn attach_plan(
    trainer: &mut Trainer,
    level: &Level,
    a: &Args,
    cfg: &GaConfig,
) -> Result<Option<usize>, String> {
    if cfg.plan_bonus <= 0.0 {
        return Ok(None);
    }
    let plan_cfg = sim::plan::PlanConfig {
        depth: usage(a.usize_opt(
            "plan-depth",
            sim::plan::PlanConfig::default().depth as usize,
        ))? as u32,
        beam: usage(a.usize_opt("plan-beam", sim::plan::PlanConfig::default().beam))?,
        ..sim::plan::PlanConfig::default()
    };
    let route = sim::plan::solve(level, cfg.master_seed, plan_cfg).ok_or_else(|| {
        "--plan-bonus was asked for but the planner found no route, so there is nothing to \
         follow"
            .to_string()
    })?;
    Ok(Some(trainer.seed_plan(level, &route)))
}

/// A run warmed from **the level's own route**: the oracle walks the level, the walk is cloned into a
/// genome, and a quarter of the population starts from that clone.
///
/// `--from-route` is the level-generation half of the same idea `--from-champion` is the
/// fine-tuning half of. The reachability oracle can already walk a route it has proved
/// (`sim::reach`), and [`evo::route_play::replay`] can drive that route through the real sim — so on
/// a level the owner has just drawn, the run does not have to discover the route at all: it starts
/// from a policy that walks it.
///
/// **The clone is a walker, not a solver, and the reason is the decision cadence.** The oracle
/// presses a take-off on the *tick* its script reaches it (`sim::reach`'s sweeps use the sim's own
/// stepper), while a policy decides once every `action_repeat` ticks — four — and holds its arm in
/// between, so the demonstrated press has no tick of the policy's to be made on. Measured on
/// `l2-first-gap`: a 4000-epoch clone reproduces **every one** of the walk's 139 decisions and still
/// walks into the pit, on all eight seeds; and at `action_repeat = 1` the same fit solves `l2`, `l4`
/// and `l5` on all eight seeds with identical frame counts. `crate::evo::imitate`'s docs and
/// `crates/evo/tests/imitate.rs` carry the measurement. What the clone brings is still a policy that
/// covers ground: on `l2-first-gap`, `l3-gauntlet`, `l4-rhythm`, `l5-tightrope`, `l7-ascent` and
/// `l9-wait-and-climb` it lands at the 90th-100th percentile of a **random population's** fitness,
/// against a random *median* of zero.
///
/// **And its share of the population is a budget, not a detail.** Six seeds at pop 64, first-win
/// generations summed (and levels solved of six), the same master seed in every arm and every arm a
/// full population through the same `resume` path:
///
/// | warm of 64 | `l2-first-gap` | `l4-rhythm` | `l5-tightrope` | `l7-wall-school` |
/// |---|---|---|---|---|
/// | 0 (random) | 6/6, 59 | 5/6, 186 | 4/6, 195 | 6/6, 124 |
/// | 1 | 6/6, 56 | 6/6, 93 | 4/6, 207 | 6/6, 70 |
/// | 8 | 6/6, 37 | 5/6, 132 | 5/6, 156 | 6/6, 22 |
/// | **16 (default)** | **6/6, 22** | **6/6, 71** | **6/6, 43** | 6/6, 75 |
/// | 64 (all) | 5/6, 142 | 4/6, 222 | 6/6, 87 | 5/6, 136 |
///
/// A quarter of the population is the arm that solves every level on every seed and is two to four
/// times faster to a first win than random; warming *everything* is the worst arm — the clone's
/// basin is a high-fitness plateau with no win on it, and a population standing on one plateau has
///
/// **The table's arms each drew their own random remainder** ([`warm_population`] fills the clone's
/// slots first and takes the rest from the same stream), so it is a survey of arms rather than a
/// paired measurement of one population's slots. The paired arms are the ones pinned in
/// `crates/evo/tests/imitate.rs` — the same 48 random genomes under the same ids in both, sixteen
/// slots warm — and they read 12 generations against random's 15 on `l2-first-gap` and 24 against 130
/// on `l5-tightrope`, so the direction is the same one the table shows.
/// nothing to its left. The default is `pop / 4` for that reason, and `--warm` moves it.
///
/// The clone is printed with its own numbers, and a replay that did **not** reach the flag says so:
/// a partial walk is still a warm start (the clone is worth what it demonstrated and nothing past
/// where the walk stopped), but a run that was told its population can play this level should never
/// have to infer that the walk failed.
fn route_seeded_trainer(cfg: GaConfig, level: &Level, a: &Args) -> Result<Trainer, String> {
    let default_warm = (cfg.pop_size / 4).max(1);
    let warm = usage(a.usize_opt("warm", default_warm))?.clamp(1, cfg.pop_size);
    let epochs = usage(a.usize_opt("clone-epochs", ImitateConfig::default().epochs))?;
    let route_seed = usage(a.u64_opt("route-seed", cfg.master_seed))?;
    let shape = cfg.shape.clone();
    let eval = EvalConfig {
        action_repeat: cfg.action_repeat,
        cut_stuck_frames: 0,
    };
    let icfg = ImitateConfig {
        epochs,
        ..ImitateConfig::default()
    };
    let (play, report): (RoutePlay, ImitationReport) =
        from_route(level, route_seed, &eval, shape, &icfg)?;
    println!("route   {} (seed {route_seed})", play.note);
    println!("clone   {}", report.line());
    if !play.won() {
        println!(
            "note    the replay did not reach the flag, so the clone is a **partial** route: it \
             walks to where the walk stopped and is worth nothing past it"
        );
    }
    let mut rng = sim::rng::rng_from_seed(cfg.master_seed ^ 0x2E5E_ED2E_5EED_2E5E);
    let pop = warm_population(
        &report.genome,
        cfg.pop_size,
        warm - 1,
        cfg.mutation_sigma,
        cfg.mutation_rate,
        &mut rng,
    );
    println!(
        "seeded  {warm} of {} genomes from the clone (1 clone + {} mutated relatives), {} random",
        cfg.pop_size,
        warm - 1,
        cfg.pop_size.saturating_sub(warm)
    );
    Trainer::resume(cfg, pop, 0)
}

/// A run warmed from a **recorded human playthrough** (Item 7).
///
/// The strongest teaching signal the project has, fed through exactly the path a planner route
/// takes: the recorded decisions are the labels, the fit is the imitation fit, and the clone is
/// injected into `pop / 4` slots as an ordinary genome. No planner sits in the loop, on purpose —
/// a human who found a technique the planner does not know is worth *more* than a planner route,
/// not less, so filtering the demonstration through planner agreement would throw away the half
/// of the signal this path exists to capture.
///
/// The file is refused rather than repaired: wrong level hash, wrong physics or sensor version,
/// a different cadence, or an unfinished run are all errors. A demonstration has to reach the
/// flag to be one (the playtest writes only completed runs), and a clone fitted to a death is a
/// lesson in how to die.
fn demo_seeded_trainer(
    cfg: GaConfig,
    level: &Level,
    a: &Args,
    path: String,
) -> Result<Trainer, String> {
    let demo = HumanDemo::load(&path)?;
    demo.check_compat(level)?;
    if !demo.won {
        return Err(format!(
            "{} did not reach the flag: a demonstration has to be a run that finished the level",
            path
        ));
    }
    if demo.action_repeat != cfg.action_repeat.max(1) {
        return Err(format!(
            "{} was recorded at action_repeat {} but this run decides every {} ticks: a clone \
             fitted to another cadence learned the wrong decisions",
            path,
            demo.action_repeat,
            cfg.action_repeat.max(1)
        ));
    }
    let default_warm = (cfg.pop_size / 4).max(1);
    let warm = usage(a.usize_opt("warm", default_warm))?.clamp(1, cfg.pop_size);
    let epochs = usage(a.usize_opt("clone-epochs", ImitateConfig::default().epochs))?;
    let icfg = ImitateConfig {
        epochs,
        ..ImitateConfig::default()
    };
    let report = fit_demo(&demo, cfg.shape.clone(), &icfg)?;
    println!(
        "demo    {} ({} decisions, seed {})",
        path,
        demo.decisions.len(),
        demo.seed
    );
    println!("clone   {}", report.line());
    let mut rng = sim::rng::rng_from_seed(cfg.master_seed ^ 0x0D3E_0D3E_0D3E_0D3E);
    let pop = warm_population(
        &report.genome,
        cfg.pop_size,
        warm - 1,
        cfg.mutation_sigma,
        cfg.mutation_rate,
        &mut rng,
    );
    println!(
        "seeded  {warm} of {} genomes from the demonstration (1 clone + {} mutated relatives), \
         {} random",
        cfg.pop_size,
        warm - 1,
        cfg.pop_size.saturating_sub(warm)
    );
    Trainer::resume(cfg, pop, 0)
}

/// A run warmed from a **timed plan**: the planner searches the real sim at the policy's own
/// decision cadence, its route is replayed to prove it wins, and the demonstration is cloned into
/// a quarter of the population.
///
/// This is the half `--from-route` cannot do. A route-graph walk presses on ticks the policy does
/// not have a decision on, so on a level whose answer is a schedule it walks into the first
/// hazard; a plan is a sequence of block actions, so the clone is fitted on the control grid it
/// will actually use. The planner is offline: once the clone exists, the runtime is still the
/// same 1 931-gene net.
fn plan_seeded_trainer(cfg: GaConfig, level: &Level, a: &Args) -> Result<Trainer, String> {
    let default_warm = (cfg.pop_size / 4).max(1);
    let warm = usage(a.usize_opt("warm", default_warm))?.clamp(1, cfg.pop_size);
    // **Ten times the route clone's budget, because one press is the whole plan.** A walk clone is a
    // head start and a missed press costs a generation; a planned crossing is a script, and a missed
    // press at its one timed jump costs the demonstration. Measured on l11: 300 epochs agree 88% and
    // die at x 12, 3 000 agree 97% and reach the second deck. `--clone-epochs` overrides it.
    let epochs = usage(a.usize_opt(
        "clone-epochs",
        ImitateConfig::default().epochs.saturating_mul(10),
    ))?;
    let route_seed = usage(a.u64_opt("route-seed", cfg.master_seed))?;
    let shape = cfg.shape.clone();
    let eval = EvalConfig {
        action_repeat: cfg.action_repeat,
        cut_stuck_frames: 0,
    };
    let icfg = ImitateConfig {
        epochs,
        leg_rounds: usage(a.usize_opt("leg-rounds", ImitateConfig::default().leg_rounds))?,
        leg_rollouts: usage(a.usize_opt("leg-rollouts", ImitateConfig::default().leg_rollouts))?,
        replan_depth: usage(a.usize_opt("replan-depth", ImitateConfig::default().replan_depth as usize))?
            as u32,
        dagger_rounds: usage(a.usize_opt("dagger-rounds", ImitateConfig::default().dagger_rounds))?,
        // The fit that includes corrections, separate from the demonstration fit: a corrected
        // state is a few exemplars against the whole route, and the same budget can leave it
        // unlearned (FINDINGS.md, Phase 3). Zero keeps the demonstration's own budget.
        correction_epochs: usage(a.usize_opt(
            "correction-epochs",
            ImitateConfig::default().correction_epochs,
        ))?,
        // The meta-controller is on by default; `--no-meta` is the Phase 2 arm, where the plan's
        // blocks and the raw leg routes are the teacher. It exists so a gate can compare the two
        // teachers on the same tree rather than across commits.
        meta: !a.has_flag("no-meta"),
        // The widened labelling is on by default; `--no-jump-labels` is the Phase 2 teacher, kept
        // so the gate can measure the widening on one tree rather than across commits.
        jump_labels: !a.has_flag("no-jump-labels"),
        ..ImitateConfig::default()
    };
    let plan_cfg = sim::plan::PlanConfig {
        depth: usage(a.usize_opt(
            "plan-depth",
            sim::plan::PlanConfig::default().depth as usize,
        ))? as u32,
        beam: usage(a.usize_opt("plan-beam", sim::plan::PlanConfig::default().beam))?,
        ..sim::plan::PlanConfig::default()
    };
    let started = std::time::Instant::now();
    let (play, report, route) = from_plan(level, route_seed, &eval, shape, &icfg, plan_cfg)?;
    println!(
        "plan    {} (seed {route_seed}, {:.2}s)",
        route.note,
        started.elapsed().as_secs_f32()
    );
    println!("replay  {}", play.note);
    println!("clone   {}", report.line());
    // The number that decides whether this is a seed or a solution: the clone on the plan's
    // own episode, closed loop. Agreement is a fit statistic; this is the game.
    let solo = evo::run_episode(level, &report.genome, &eval, route_seed);
    println!(
        "clone plays {} after {} frames (fitness {:.2}, best_x {:.1}{})",
        solo.outcome.as_str(),
        solo.frames,
        solo.fitness,
        solo.best_x,
        solo.death_cause
            .map(|c| format!(", killed by {}", c.as_str()))
            .unwrap_or_default()
    );
    let mut rng = sim::rng::rng_from_seed(cfg.master_seed ^ 0x2E5E_ED2E_5EED_2E5E);
    let pop = warm_population(
        &report.genome,
        cfg.pop_size,
        warm - 1,
        cfg.mutation_sigma,
        cfg.mutation_rate,
        &mut rng,
    );
    println!(
        "seeded  {warm} of {} genomes from the planned clone (1 clone + {} mutated relatives), {} random",
        cfg.pop_size,
        warm - 1,
        cfg.pop_size.saturating_sub(warm)
    );
    Trainer::resume(cfg, pop, 0)
}

/// The GA configuration a `train` invocation asks for.
///
/// Factored out so the flag defaults have exactly **one** home ([`GaConfig::default`]) and a test
/// can compare the two directly. That is not tidiness: this function's predecessor wrote its own
/// `1.0` beside the flag name, the trainer's default later moved to `3.0`, and the command line
/// quietly went on asking for the old temperature for every run that did not name one — a whole
/// sweep's result that never reached a user, found only by reading the header of a real run.
///
/// It takes no level. It did, for one pass, because it used to choose the trial count from the
/// level's walkers; that rule now lives in the trainer and the caller asks
/// [`GaConfig::effective_trials`] for the answer, so the only thing the second parameter could do
/// here was sit unused.
pub(crate) fn train_config(a: &Args) -> Result<GaConfig, String> {
    let default = GaConfig::default();
    // Trials used to be chosen *here* — a hard-coded `3` beside the flag name for a level with
    // walkers — which is the same defect the doc comment above records for the reheat factor, and
    // it left `GaConfig::recommended_trials_for` as dead code while every other front-end (the
    // window, a resumed run, the API) judged walker levels on a single patrol phase. The rule now
    // lives in the trainer and this line is a pass-through: the flag is the operator's answer, and
    // `--no-auto-trials` releases the trainer's. The warning below stays in the caller because it
    // is about the pair — an explicit trial count on a level that cannot be stochastic.
    let trials = usage(a.usize_opt("trials", default.trials as usize))? as u32;
    let auto_trials = !a.has_flag("no-auto-trials");
    let adaptive_sigma = if a.has_flag("adaptive-sigma") {
        true
    } else {
        default.adaptive_sigma
    };
    let config = GaConfig {
        pop_size: usage(a.usize_opt("pop", default.pop_size))?,
        elite: usage(a.usize_opt("elite", default.elite))?,
        tournament: usage(a.usize_opt("tournament", default.tournament))?,
        mutation_rate: usage(a.f32_opt("mut-rate", default.mutation_rate))?,
        mutation_sigma: usage(a.f32_opt("mut-sigma", default.mutation_sigma))?,
        crossover_rate: usage(a.f32_opt("crossover", default.crossover_rate))?,
        fine_children: usage(a.usize_opt("fine-children", default.fine_children))?,
        jump_timing_share: usage(a.f32_opt("jump-timing-share", default.jump_timing_share))?,
        // Item 14: children bred from the best genome's own losing decisions. Off by default, like
        // every unmeasured arm; `--repair-children 3` is the window's configuration.
        repair_children: usage(a.usize_opt("repair-children", default.repair_children))?,
        // **On by default**, matching the window: a run that solves and then spends thousands of
        // generations on the crowned score is not improving the clock at all, and the second
        // phase only arms once the record has solved, so learning is unchanged. `--mastery`
        // remains accepted as the explicit spelling; `--no-mastery` is the old behaviour.
        mastery: !a.has_flag("no-mastery"),
        // Plan-proximity selection shaping (WP5). Off by default: it costs one extra episode per
        // genome per generation while the bonus is alive, and the plan is only worth following on
        // a level the planner can route.
        plan_bonus: usage(a.f32_opt("plan-bonus", default.plan_bonus))?,
        plan_bonus_generations: usage(a.usize_opt(
            "plan-bonus-generations",
            default.plan_bonus_generations as usize,
        ))? as u32,
        // Snapshot exploration (WP8). 0 is the shipped default: the archive is a separate
        // experimental arm and costs episodes only while its cadence is set.
        snapshot_every: usage(a.usize_opt("snapshot-every", default.snapshot_every as usize))?
            as u32,
        snapshot_probes: usage(a.usize_opt("snapshot-probes", default.snapshot_probes))?,
        // How big the fine scale's step is, as a fraction of `--mut-sigma`. A fine child moves one
        // to three genes, so it is local in count; this is what makes it local in size too. See
        // `GaConfig::fine_sigma_scale` — the archive's children use the same step.
        fine_sigma_scale: usage(a.f32_opt("fine-sigma-scale", default.fine_sigma_scale))?,
        // Episodes, on fixed seeds no generation draws, on which a record candidate and the record
        // it would replace are compared (`--no-validation` switches it off, which is the only way
        // to reproduce a pre-validation arm exactly).
        validation_episodes: if a.has_flag("no-validation") {
            0
        } else {
            usage(a.usize_opt("validation-episodes", default.validation_episodes as usize))? as u32
        },
        adaptive_sigma,
        stagnation_generations: usage(
            a.usize_opt("stagnation", default.stagnation_generations as usize),
        )? as u32,
        immigrants: usage(a.usize_opt("immigrants", default.immigrants))?,
        archive_children: usage(a.usize_opt("archive-children", default.archive_children))?,
        // Item 5: end an episode after this many no-progress ticks and score what was played.
        // Off at 0, which is the ordinary full episode.
        cut_stuck_frames: usage(a.usize_opt(
            "cut-stuck",
            default.cut_stuck_frames as usize,
        ))? as u32,
        // Item 5, step 1: a cut is kept only if the genome could not reach the selection cut even
        // with this much extra score. Defaults to a win plus its clock.
        cut_margin: usage(a.f32_opt("cut-margin", default.cut_margin))?,
        // Item 5, step 2: spend the ticks the cut saved on one extra trial for the genomes just
        // below the cut. Off by default, and a usage error without the cut itself.
        cut_refund: a.has_flag("cut-refund"),
        // Item 11: a share of the parents drawn from the technique archive's cells rather than
        // from the leaderboard. Off by default; the gate is a 24-seed paired arm with the
        // occupied-cell count reported beside it.
        technique_children: usage(a.usize_opt(
            "technique-children",
            default.technique_children,
        ))?,
        archive_on_diversity: a.has_flag("archive-on-diversity"),
        // The reward-level half of the quality-diversity question: fitness units paid to a genome
        // for behaving rarely, added to the **selection** score and never to a recorded one. Default
        // 0.0 comes from the trainer, like every other default here.
        novelty_weight: usage(a.f32_opt("novelty", default.novelty_weight))?,
        // How pessimistically a genome's own trials are ranked: 0.0 is their mean (the default,
        // and what every recorded number was taken with), 1.0 is the worst of them. See
        // `GaConfig::rank_pessimism` — like the novelty weight, it is a *ranking* knob and cannot
        // change a recorded number.
        rank_pessimism: usage(a.f32_opt("rank-pessimism", default.rank_pessimism))?,
        reheat_factor: usage(a.f32_opt("reheat-factor", default.reheat_factor))?,
        // Item 13: function-preserving width growth on the plateau. 0 units is off, which is
        // every run measured before the operator existed; the caps are documented on the field.
        growth_units: usage(a.usize_opt("growth-units", default.growth_units))?,
        growth_events: usage(a.usize_opt("growth-events", default.growth_events as usize))? as u32,
        // Item 4: mid-run teaching. Both halves are off by default and spend the run's own
        // episodes, so an arm needs its equal-budget control before either moves a default.
        lesson_every: usage(a.usize_opt("lesson-every", default.lesson_every as usize))? as u32,
        plateau_window: usage(a.usize_opt("plateau-window", default.plateau_window as usize))?
            as u32,
        plateau_cem_generations: usage(a.usize_opt(
            "plateau-cem-generations",
            default.plateau_cem_generations as usize,
        ))? as u32,
        plateau_cem_events: usage(
            a.usize_opt("plateau-cem-events", default.plateau_cem_events as usize),
        )? as u32,
        // Item 6: a share of generation zero from a standing generalist population trained
        // across the pack. The run's own start keeps the front slots.
        generalist_share: usage(a.f32_opt("generalist-share", default.generalist_share))?,
        action_repeat: usage(a.usize_opt("action-repeat", default.action_repeat as usize))? as u32,
        // Memory: how much of the network's own first hidden layer is fed back into the next
        // decision, which is what lets a policy count, wait and commit rather than react. The
        // default comes from the trainer's shape (0 — a memoryless net, which is what every
        // measurement in `REVIEW.md` was taken with), and the trainer clamps the width to
        // `genome::MAX_FEEDBACK`, so a typo is a slower run rather than a failed allocation.
        //
        // Capacity is the other half of the same question and comes through the same shape: the
        // shipped net is one hidden layer of 16 (`default_shape`), and `--hidden` is how a run asks
        // whether a wider one searches better. Both are *live* shapes, so the trainer's own clamps
        // (`GaConfig::sanitized`) are the contract — a mis-typed width is a smaller run, never a
        // failed allocation — and `--hidden`'s own parsing refuses only what is not a number at all.
        shape: evo::NetShape {
            hidden: parse_hidden(a, &default.shape.hidden)?,
            feedback: usage(a.usize_opt("feedback", default.shape.feedback))?,
            // Item 12's cadence head. Off by default: it is an arm until its equal-budget
            // comparison lands, and a head appended to every genome changes every gene count
            // in every recorded recipe.
            commitment: a.has_flag("commitment"),
            ..default.shape
        },
        // Trials are only distinguishable on levels that are actually stochastic
        // (those with walkers): elsewhere every trial is the same episode.
        trials,
        auto_trials,
        // Which episodes the population is judged on, and whether the extra trials are spent on
        // everything or only on what is still in contention. Both defaults come from the trainer;
        // `--no-common-episodes` is the arm every table in `REVIEW.md` taken before the field
        // existed was measured under, and it is the one an old reading is reproduced with.
        common_episodes: if a.has_flag("common-episodes") {
            true
        } else if a.has_flag("no-common-episodes") {
            false
        } else {
            default.common_episodes
        },
        race_episodes: if a.has_flag("race-episodes") {
            true
        } else if a.has_flag("no-race-episodes") {
            false
        } else {
            default.race_episodes
        },
        master_seed: usage(a.u64_opt("seed", default.master_seed))?,
    };
    // Item 5's two halves are one mechanism: a refund without a cut has nothing to spend, and
    // accepting the pair while ignoring one of them is the silent no-op this file refuses.
    if config.cut_refund && config.cut_stuck_frames == 0 {
        return Err(
            "usage: --cut-refund needs --cut-stuck <ticks>: there is no cut to refund without one"
                .into(),
        );
    }
    Ok(config)
}

// ---------------------------------------------------------------------------
// es
// ---------------------------------------------------------------------------

/// The baseline arm: a `(µ, λ)` evolution strategy over the same genome, on the same level,
/// paying for the same number of episodes.
///
/// **Why this exists, and what it can settle.** Every number in `REVIEW.md` compares this
/// project's search with an earlier version of itself, and that comparison cannot tell "the GA
/// is the binding constraint" from "the problem is". An ES on an equal episode budget can: it
/// makes exactly one move per generation — a rank-weighted step along the sampled mirror
/// directions — with no elites, no tournament, no crossover and no archive, so if a plain ES
/// matches the GA on a level, the remaining work is in the content and the fitness landscape
/// rather than in the breeding rules, and if it loses the search is earning its place.
///
/// What is deliberately *shared* with `train` is everything that decides what an episode *is*:
/// the level, the episode seed, the decision cadence, the trial count and the memory width. What
/// is deliberately not shared is every search knob, because elites and tournaments describe a
/// population this arm does not have. `--pop` is the batch here, and it means the same compute:
/// `λ` episodes per generation against the GA's 200 genomes per generation.
///
/// The arm ships no champion file: [`ChampionFile`] records the GA's own search knobs, and a
/// mean is a distribution rather than a policy anyone can replay. What it prints is what a
/// comparison needs — the generation the level fell at, and the fitness of the best point it
/// ever evaluated.
pub fn es(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(ES_OPTS))?;
    let level_path = usage(a.required_path("level"))?;
    let level = load_level(&level_path)?;

    let generations = usage(a.usize_opt("generations", 30))?;
    let quiet = a.has_flag("quiet");
    let stop_on_win = a.has_flag("stop-on-win");
    let max_seconds: Option<f64> = match a.opt("max-seconds") {
        Some(_) => Some(usage(a.f64_opt("max-seconds", 0.0))?),
        None => None,
    };

    let cfg = es_config(a)?;

    if !quiet {
        println!("level   {} ({}x{})", level.name, level.width, level.height);
        println!("hash    {}", level.content_hash_string());
        println!("oracle  {}", reachability_line(&level));
        println!(
            "es      pop {} ({} perturbations in {} mirrored pairs)  sigma {:.3}  lr {:.4}  action_repeat {}  trials {}",
            cfg.pop_size,
            cfg.batch(),
            cfg.batch() / 2,
            cfg.sigma,
            cfg.learning_rate,
            cfg.action_repeat,
            cfg.trials
        );
        println!(
            "seed    {}   (sim {} / sensors v{} / fitness v{})",
            cfg.master_seed,
            SIM_VERSION,
            sim::sensors::VERSION,
            FITNESS_VERSION
        );
        println!(
            "\n{:>4}  {:>10}  {:>10}  {:>7}",
            "gen", "max", "mean", "secs"
        );
    }

    let mut trainer = EsTrainer::new(cfg);
    let effective = trainer.config().clone();
    let started = Instant::now();
    let mut solved_at: Option<u32> = None;
    let mut generation_secs: Vec<f64> = Vec::with_capacity(generations);

    for _ in 0..generations {
        let t0 = Instant::now();
        let row = trainer.step(&level);
        let secs = t0.elapsed().as_secs_f64();
        generation_secs.push(secs);

        if solved_at.is_none() && row.solved {
            solved_at = Some(row.generation);
        }
        if !quiet {
            println!(
                "{:>4}  {:>10.2}  {:>10.2}  {:>7.2}{}",
                row.generation,
                row.max_fitness,
                row.mean_fitness,
                secs,
                if row.solved { "  <-- SOLVED" } else { "" }
            );
        }

        if stop_on_win && solved_at.is_some() {
            break;
        }
        if let Some(limit) = max_seconds {
            if started.elapsed().as_secs_f64() >= limit {
                if !quiet {
                    println!("--max-seconds {limit:.0} reached; stopping early");
                }
                break;
            }
        }
    }

    let elapsed = started.elapsed();
    let episodes = trainer.history().len() * effective.batch();
    let mean = if generation_secs.is_empty() {
        0.0
    } else {
        generation_secs.iter().sum::<f64>() / generation_secs.len() as f64
    };

    println!();
    match solved_at {
        Some(g) => println!("solved at generation {g} (first win)"),
        None => println!("not solved in {} generations", generation_secs.len()),
    }
    match trainer.best_ever() {
        Some((generation, _, fitness)) => println!(
            "best    gen {} fitness {:.2} (best evaluated point, not the mean)",
            generation, fitness
        ),
        None => println!("best    no generation ran, so nothing was evaluated"),
    }
    println!(
        "timing  {:>7.3} s/gen mean | {} generations in {:.1} s | {} episodes ({} per genome at trials {})",
        mean,
        generation_secs.len(),
        elapsed.as_secs_f64(),
        episodes,
        effective.batch(),
        effective.trials
    );

    Ok(())
}

/// The [`EsConfig`] an `es` invocation asks for.
///
/// As with [`train_config`], every default comes from the trainer's own `Default` rather than
/// from a second copy beside the flag name — the defect this project already paid for once, when
/// `train` kept asking for a reheat temperature the trainer had moved away from.
fn es_config(a: &Args) -> Result<EsConfig, String> {
    let default = EsConfig::default();
    Ok(EsConfig {
        pop_size: usage(a.usize_opt("pop", default.pop_size))?,
        sigma: usage(a.f32_opt("sigma", default.sigma))?,
        learning_rate: usage(a.f32_opt("learning-rate", default.learning_rate))?,
        master_seed: usage(a.u64_opt("seed", default.master_seed))?,
        // The same one-flag-plus-a-copy-of-the-default shape the GA path uses, so the memory
        // width *and the capacity* are comparable across the two arms — a baseline searching a
        // different net would be a comparison of two questions rather than of two searches.
        shape: evo::NetShape {
            hidden: parse_hidden(a, &default.shape.hidden)?,
            feedback: usage(a.usize_opt("feedback", default.shape.feedback))?,
            // Item 12's cadence head. Off by default: it is an arm until its equal-budget
            // comparison lands, and a head appended to every genome changes every gene count
            // in every recorded recipe.
            commitment: a.has_flag("commitment"),
            ..default.shape
        },
        action_repeat: usage(a.usize_opt("action-repeat", default.action_repeat as usize))? as u32,
        trials: usage(a.usize_opt("trials", default.trials as usize))? as u32,
    })
}

/// The levels `--suite` names, as paths.
///
/// One comma-separated list rather than a flag that accumulates, because the *order* is part of the
/// answer: level 0 is the level a champion is recorded on and the level the archive reads its
/// behaviour from, so `a,b` and `b,a` are different runs. Duplicates are refused rather than
/// deduplicated — a level named twice would double its weight in a sum and change nothing in a
/// minimum, which is exactly the sort of silent difference a suite exists to make visible.
fn parse_level_list(list: &str) -> Result<Vec<String>, String> {
    let mut paths: Vec<String> = Vec::new();
    for raw in list.split(',') {
        let path = raw.trim();
        if path.is_empty() {
            return Err(format!(
                "usage: --suite has an empty entry in {list:?}; expected --suite a.json,b.json"
            ));
        }
        if paths.iter().any(|p| p == path) {
            return Err(format!(
                "usage: --suite names {path:?} twice; each level is scored once per generation"
            ));
        }
        paths.push(path.to_string());
    }
    if paths.is_empty() {
        return Err(
            "usage: --suite needs at least one level, e.g. --suite levels/l2-first-gap.json".into(),
        );
    }
    Ok(paths)
}

fn write_csv(path: &str, history: &[evo::GenRow]) -> Result<(), String> {
    // The last four columns are the search's own reading rather than the run's score, and
    // they are in the CSV for the same reason they are on the chart: `mutation_sigma` is the
    // self-tuning size as it stood when that generation's children were bred, and
    // `immigrants` is how many random genomes were injected after it — the two facts that
    // separate "the run plateaued" from "the run plateaued and did something about it". The
    // last two are the quality-diversity half of the same story: `archive_children` is how
    // many of that generation's children came from the behavioural archive rather than the
    // ranking, and `archive_bits` is how many niches the archive held, which is the run's
    // diversity — the reading that falls when a population collapses onto one way of moving,
    // and which no column of scores can show.
    let mut out = String::from(
        "generation,max_fitness,avg_fitness,min_fitness,wins,best_id,genius,mutation_sigma,immigrants,archive_children,archive_bits\n",
    );
    for row in history {
        out.push_str(&format!(
            "{},{:.4},{:.4},{:.4},{},{},{},{:.4},{},{},{}\n",
            row.generation,
            row.max_fitness,
            row.avg_fitness,
            row.min_fitness,
            row.wins,
            row.best_id,
            row.genius as u8,
            row.mutation_sigma,
            row.immigrants,
            row.archive_children,
            row.archive_bits
        ));
    }
    if let Some(dir) = Path::new(path).parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
    }
    fs::write(path, out).map_err(|e| format!("cannot write {path}: {e}"))
}

fn reachability_line(level: &Level) -> String {
    let r = reach::report(level);
    if r.goal_reached {
        format!(
            "beatable in principle ({} platforms, start #{:?}, goal platform #{:?})",
            r.platforms.len(),
            r.start,
            r.goal_platform
        )
    } else {
        format!(
            "NOT beatable as built: {}",
            r.diagnosis.as_deref().unwrap_or("no explanation available")
        )
    }
}

// ---------------------------------------------------------------------------
// reach
// ---------------------------------------------------------------------------

/// **The adversarial battery (§9), as one command.**
///
/// Every `bat-*` level in the pack is asked the two questions a level has to answer before a
/// training run on it can mean anything: *is it beatable in principle* (the reach oracle) and
/// *can the timed planner route it*. The two are reported together, with the planner's own
/// expansion count and wall time, because the battery's job is to be a permanent regression gate
/// — a default change that makes one of these levels unroutable should say so here rather than
/// inside a 500-generation run that quietly never wins.
///
/// It is deliberately separate from `reach --all`: that command covers the pack, whose members
/// are rungs on a difficulty ladder; these are probes for one skill each, and the plan's gate
/// asks about them as a set.
pub fn battery(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(BATTERY_OPTS))?;
    let dir = a.string("levels-dir", "levels");
    let verbose = a.has_flag("verbose");
    let seed = usage(a.u64_opt("seed", 1))?;
    let defaults = sim::plan::PlanConfig::default();
    let cfg = sim::plan::PlanConfig {
        depth: usage(a.usize_opt("depth", defaults.depth as usize))? as u32,
        beam: usage(a.usize_opt("beam", defaults.beam))?,
        ..defaults
    };
    let mut paths: Vec<PathBuf> = pack_levels(&dir)?
        .into_iter()
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with("bat-"))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("no bat-*.json battery levels found in {dir:?}"));
    }

    println!(
        "battery {} level(s) from {dir} | planner depth {}, beam {}",
        paths.len(),
        cfg.depth,
        cfg.beam
    );
    let mut failures = 0usize;
    for path in &paths {
        let level = load_level(&path.to_string_lossy())?;
        let reach = reach::report(&level);
        let started = std::time::Instant::now();
        let plan = sim::plan::plan_level(&level, seed, cfg);
        let secs = started.elapsed().as_secs_f32();
        let ok = reach.goal_reached && plan.route.is_some();
        if !ok {
            failures += 1;
        }
        println!(
            "{} {:22} reach {:<5} plan {:<5} {:>5} expansions {:>6.2}s",
            if ok { "OK  " } else { "FAIL" },
            level.name,
            if reach.goal_reached { "yes" } else { "no" },
            if plan.route.is_some() { "yes" } else { "no" },
            plan.expanded,
            secs
        );
        if verbose || !ok {
            if let Some(why) = &reach.diagnosis {
                println!("       reach: {why}");
            }
            println!("       plan:  {}", plan.note);
        }
    }
    if failures > 0 {
        return Err(format!(
            "{failures} of {} battery level(s) failed the gate",
            paths.len()
        ));
    }
    Ok(())
}

/// **`fuzz`: the Phase 0 practice suite.** Generate every level, certify it with the reachability
/// oracle and the timed planner, print the table, and — with `--out` — write the level files.
///
/// The suite is fixed at [`practice::SUITE_LEN`] levels: `--out` does not change *what* is
/// generated, only where it lands. A level that fails either authority is reported and the command
/// exits non-zero, because a suite with an uncertified member is worse than no suite: every later
/// gate would be reading a problem whose answer nobody has checked.
pub fn fuzz(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(FUZZ_OPTS))?;
    if a.has_flag("out") && a.opt("out").is_none() {
        return Err("usage: --out needs a directory, e.g. --out levels/practice".into());
    }
    let out = a.opt("out").map(PathBuf::from);
    if let Some(dir) = &out {
        fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    println!(
        "suite   {} levels, {} archetypes x {} tiers (generator v{})",
        practice::SUITE_LEN,
        practice::ARCHETYPES.len(),
        practice::SUITE_LEN / practice::ARCHETYPES.len(),
        practice::SUITE_VERSION
    );
    println!(
        "\n{:<20}  {:>6}  {:>4}  {:>7}  {:>6}  {:>9}  {:>8}",
        "level", "oracle", "plan", "frames", "blocks", "oracle ms", "plan ms"
    );
    let mut uncertified = 0usize;
    let mut plan_total = 0.0f64;
    let mut reach_total = 0.0f64;
    let mut plan_worst = 0.0f64;
    for index in 0..practice::SUITE_LEN {
        let (level, cert) = practice::generate(index);
        let stem = practice::stem(index);
        // Timed on a **fresh parse**, so the reading is "open this file and certify it" rather
        // than a warm repeat against the route table `generate` just built. The oracle is timed
        // first; the plan search then reads the same warm level, so the two numbers do not
        // double-count each other.
        let json = level
            .to_json_pretty()
            .map_err(|e| format!("cannot write {stem}: {e}"))?;
        let fresh = Level::from_json_str(&json)
            .map_err(|e| format!("cannot re-read {stem}: {e}"))?;
        let t_reach = Instant::now();
        let _ = reach::report(&fresh);
        let reach_ms = t_reach.elapsed().as_secs_f64() * 1000.0;
        let t_plan = Instant::now();
        let _ = sim::plan::plan_level(
            &fresh,
            0x5EED,
            sim::plan::PlanConfig {
                depth: 48,
                ..sim::plan::PlanConfig::default()
            },
        );
        let plan_ms = t_plan.elapsed().as_secs_f64() * 1000.0;
        println!(
            "{:<20}  {:>6}  {:>4}  {:>7}  {:>6}  {:>9.1}  {:>8.1}",
            stem,
            if cert.beatable { "yes" } else { "NO" },
            if cert.planned { "yes" } else { "NO" },
            cert.frames
                .map(|f| f.to_string())
                .unwrap_or_else(|| "-".into()),
            cert.blocks
                .map(|b| b.to_string())
                .unwrap_or_else(|| "-".into()),
            reach_ms,
            plan_ms
        );
        if !cert.certified() {
            uncertified += 1;
            println!("    {}", cert.note);
        }
        plan_total += plan_ms;
        reach_total += reach_ms;
        plan_worst = plan_worst.max(plan_ms);
        if let Some(dir) = &out {
            std::fs::write(dir.join(format!("{stem}.json")), json)
                .map_err(|e| format!("cannot write {stem}: {e}"))?;
        }
    }
    println!(
        "\ntotal   oracle {reach_total:.1} ms   plan {plan_total:.1} ms (worst {plan_worst:.1} ms per level)"
    );
    if uncertified > 0 {
        return Err(format!(
            "{uncertified} of {} generated levels did not certify",
            practice::SUITE_LEN
        ));
    }
    if let Some(dir) = &out {
        println!(
            "wrote   {} level files to {}",
            practice::SUITE_LEN,
            dir.display()
        );
    }
    Ok(())
}

/// **Phase 1: compile a certified route plan.**
///
/// Prints the plan's legs, their margins and the crossings it closed; with `--out` it writes one
/// `*.routeplan.json` per level. The command fails on any level it cannot certify, so it is the
/// route compiler's gate as well as its reader: a plan that the oracle refuses, that the planner's
/// budget cannot route, or that does not survive a replay is a non-zero exit rather than a warning.
///
/// `--all --levels-dir levels/practice` is the Phase 1 acceptance run. The practice suite certifies
/// at depth 48; the command searches at 64 so `l11-the-elements` routes too, and `--plan-depth`
/// overrides the budget for experiments.
pub fn routeplan(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(ROUTEPLAN_OPTS))?;
    let seed = usage(a.u64_opt("seed", 1))?;
    // 64 by default: the planner's own default (32) is one expansion too shallow for
    // `l11-the-elements`' 11-leg route, and a compiled artifact that refuses the level the phase
    // exists for would be a misleading gate. `--plan-depth` lowers it for experiments.
    let depth = usage(a.usize_opt("plan-depth", 64))? as u32;
    let cfg = sim::plan::PlanConfig {
        depth,
        ..sim::plan::PlanConfig::default()
    };
    let paths: Vec<PathBuf> = if a.has_flag("all") {
        let dir = a.string("levels-dir", "levels");
        let mut found = pack_levels(&dir)?;
        found.sort();
        if found.is_empty() {
            return Err(format!("no *.json levels found in {dir:?}"));
        }
        found
    } else {
        vec![PathBuf::from(usage(a.required_path("level"))?)]
    };
    let out = a.opt("out").map(PathBuf::from);
    if let Some(dir) = &out {
        fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }

    let mut failed = 0usize;
    for path in &paths {
        let level = load_level(&path.to_string_lossy())?;
        let started = Instant::now();
        match sim::route_plan::compile(&level, seed, cfg) {
            Ok(plan) => {
                println!(
                    "PLANNED {} ({}) in {:.2}s",
                    level.name,
                    path.display(),
                    started.elapsed().as_secs_f32()
                );
                println!("  {}", plan.line());
                for leg in &plan.legs {
                    println!(
                        "  leg {:<2} blocks {:>4}..{:<4} hops {}  {:>7}  ends {:>6}  margin {}",
                        leg.index,
                        leg.first_block,
                        leg.end_block,
                        leg.hops,
                        sim::format_time(leg.frames),
                        sim::format_time(leg.end_frame),
                        sim::format_time(leg.margin_frames)
                    );
                }
                if let Some(dir) = &out {
                    let stem = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "level".into());
                    let json = serde_json::to_string_pretty(&plan)
                        .map_err(|e| format!("cannot serialize the plan: {e}"))?;
                    std::fs::write(dir.join(format!("{stem}.routeplan.json")), json)
                        .map_err(|e| format!("cannot write {stem}.routeplan.json: {e}"))?;
                }
            }
            Err(err) => {
                failed += 1;
                println!("NO PLAN  {} ({})", level.name, path.display());
                println!("  {}", err.line());
                if err.beatable() {
                    println!(
                        "  the level is certified beatable; this is a planner budget failure, and \
                         a warm start must report it rather than clone a weaker route"
                    );
                }
            }
        }
    }
    if failed > 0 {
        return Err(format!(
            "{failed} of {} levels have no certified route plan",
            paths.len()
        ));
    }
    if let Some(dir) = &out {
        println!("wrote   {} route plans to {}", paths.len(), dir.display());
    }
    Ok(())
}

pub fn reach(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(REACH_OPTS))?;
    let verbose = a.has_flag("verbose");

    let paths: Vec<PathBuf> = if a.has_flag("all") {
        let dir = a.string("levels-dir", "levels");
        let mut found = pack_levels(&dir)?;
        if found.is_empty() {
            return Err(format!("no *.json levels found in {dir:?}"));
        }
        found.sort();
        found
    } else {
        vec![PathBuf::from(usage(a.required_path("level"))?)]
    };

    let mut failures = 0usize;
    let timed = a.has_flag("timed");
    let seed = usage(a.u64_opt("seed", 1))?;
    for path in &paths {
        let level = load_level(&path.to_string_lossy())?;
        if timed {
            // The static oracle answers "is a crossing expressible"; this answers "is there a
            // schedule that walks it". The planner is the only thing in the tree that can see
            // a mover's phase and a turret's cooldown, so it is the honest second question.
            let started = std::time::Instant::now();
            let plan = sim::plan::plan_level(&level, seed, sim::plan::PlanConfig::default());
            println!(
                "{:?} {} ({}) [timed]",
                plan.route.is_some(),
                level.name,
                path.display()
            );
            println!(
                "         {} ({} expansions, {:.2}s)",
                plan.note,
                plan.expanded,
                started.elapsed().as_secs_f32()
            );
            if let Some(route) = &plan.route {
                println!(
                    "         route {} blocks, {} frames, {} rungs closed",
                    route.actions.len(),
                    route.frames,
                    route.hops
                );
                let head: Vec<&str> = route.actions.iter().take(16).map(|a| a.name()).collect();
                println!("         first blocks: {}", head.join(" "));
                let mut trace_sim = sim::Sim::new(&level, route.seed);
                let mut marks: Vec<String> = Vec::new();
                for (i, action) in route.actions.iter().enumerate() {
                    for _ in 0..sim::DECISION_TICKS {
                        trace_sim.step(*action);
                    }
                    if i % 20 == 0 || trace_sim.outcome() == sim::Outcome::Won {
                        marks.push(format!(
                            "{i}:({:.1},{:.1})",
                            trace_sim.player().x,
                            trace_sim.player().y
                        ));
                    }
                    if trace_sim.outcome().is_finished() {
                        break;
                    }
                }
                println!("         trace: {}", marks.join(" "));
            } else {
                println!(
                    "         frontier x {:.2}, y {:.2} at {} frames",
                    plan.best_x, plan.best_y, plan.frames
                );
                failures += 1;
            }
            println!();
            continue;
        }
        let report = reach::report(&level);
        let status = if report.goal_reached {
            "OK      "
        } else {
            "UNBEATABLE"
        };
        println!("{status} {} ({})", level.name, path.display());
        println!(
            "         platforms {}  reachable {}  start {:?}  goal {:?}",
            report.platforms.len(),
            report.reachable_platforms(),
            report.start,
            report.goal_platform
        );
        if let Some(msg) = &report.diagnosis {
            println!("         {msg}");
        }
        if let Some(c) = report.culprit {
            println!(
                "         first stuck jump: platform #{} at column {} offset {:.1}, {} {}, -> {:?}",
                c.from,
                c.takeoff_x,
                c.takeoff_offset,
                c.dir.name(),
                c.hold.name(),
                c.to
            );
        }
        if verbose {
            for p in &report.platforms {
                println!(
                    "         platform #{:<3} x {:<3}..{:<3} surface row {:<3} reached {}",
                    p.id, p.x0, p.x1, p.surface_row, report.reached[p.id]
                );
            }
            println!(
                "         jump capability: full {:.2} tiles high, {:.2} tiles across; tap {:.2} tiles high",
                PHYSICS.discrete_max_jump_height(),
                PHYSICS.max_jump_distance(),
                PHYSICS.min_jump_height()
            );
        }
        if !report.goal_reached {
            failures += 1;
        }
        println!();
    }

    if failures > 0 {
        return Err(format!(
            "{failures} of {} level(s) are not beatable as built",
            paths.len()
        ));
    }
    Ok(())
}

pub(crate) fn list_levels(dir: &str) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {dir}: {e}"))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {dir}: {e}"))?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            out.push(path);
        }
    }
    Ok(out)
}

/// The levels a `levels` directory **ships**: everything in it except the drawings a person
/// made there.
///
/// The pack is the repo's own set — the `l*` courses and the `scratch-*` fixtures the
/// measurements are taken on — and every one of them is expected to be beatable, which is a
/// claim `reach --all` and `cargo test` both check. A *drawing* is a different thing: the
/// editor writes one as `custom-<stem>.json` beside the pack when a person saves a level
/// (`crate::app`'s `protect_shipped_path`, which exists so that saving never clobbers a
/// shipped course), and a half-drawn level is allowed to be unbeatable — the editor's canvas
/// is a place to work, and refusing to save an unfinished level would be the worse rule.
///
/// So the two are separated here rather than by a weaker assertion: the pack keeps its
/// "every level is beatable" guarantee in full, and a drawing is diagnosed on request with
/// `reach --level <drawing>`, which is exactly the command a designer wants on the level
/// they are building. [`list_levels`] stays unfiltered for [`find_level_by_hash`], because a
/// champion trained on a person's own level has to be findable by the level it came from.
pub(crate) fn pack_levels(dir: &str) -> Result<Vec<PathBuf>, String> {
    Ok(list_levels(dir)?
        .into_iter()
        .filter(|p| !is_user_drawing(p))
        .collect())
}

/// Whether a path is a level a person drew and saved rather than one of the pack's own.
///
/// By name, and the name is a convention the editor already writes: `custom-<stem>.json`.
fn is_user_drawing(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| stem.starts_with("custom-"))
}

// ---------------------------------------------------------------------------
// committee
// ---------------------------------------------------------------------------

/// **Validate a committee file (Item 10's gate).**
///
/// The committee is a presentation of a run's result, never a genome and never a row in the
/// difficulty table, and this command is what lets a viewer show it as the level's solver: the
/// vote and its best member play the run's fixed validation episodes, and the shell fails if the
/// vote lost an episode the member won. It does not train, does not write anything, and does not
/// change a default.
pub fn committee(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(COMMITTEE_OPTS))?;
    let path = usage(a.required_path("file"))?;
    let file = CommitteeFile::load(&path)?;
    let level = match a.opt("level") {
        Some(p) => load_level(p)?,
        None => find_level_by_hash(
            &a.string("levels-dir", "levels"),
            &file.level_hash,
        )?
        .ok_or_else(|| {
            format!(
                "no level in {} hashes to {} (committee was built on {:?}); pass --level",
                a.string("levels-dir", "levels"),
                file.level_hash,
                file.level_name
            )
        })?,
    };
    file.check_compat(&level)?;
    let report = file.validate(&level)?;
    println!(
        "committee {} members on {} ({} validation episodes, seed {})",
        file.members.len(),
        level.name,
        report.episodes,
        file.master_seed
    );
    println!(
        "gate    committee won {} of {}, best member won {} of {} (frames {} against {})",
        report.won,
        report.episodes,
        report.best_member_won,
        report.episodes,
        report.frames,
        report.best_member_frames
    );
    if !report.passes() {
        return Err(format!(
            "the committee lost to its best member ({} against {} wins): keep the single \
             champion and do not show the vote as the level's solver",
            report.won, report.best_member_won
        ));
    }
    println!("pass    the committee is never worse than its best member on completion");
    Ok(())
}

// ---------------------------------------------------------------------------
// replay
// ---------------------------------------------------------------------------

pub fn replay(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(REPLAY_OPTS))?;
    let champion_path = usage(a.required_path("champion"))?;
    let raw = fs::read_to_string(&champion_path)
        .map_err(|e| format!("cannot read {champion_path}: {e}"))?;
    let champion =
        ChampionFile::from_json_str(&raw).map_err(|e| format!("{champion_path}: {e}"))?;

    // Find the level: explicit, or by matching the recorded level hash. Hash
    // matching is what makes a shared champion self-describing.
    let level = match a.opt("level") {
        Some(p) => load_level(p)?,
        None => find_level_by_hash(
            &a.string("levels-dir", "levels"),
            &champion.metadata.level_hash,
        )?
        .ok_or_else(|| {
            format!(
                "no level in {} hashes to {} (champion was trained on {:?}); pass --level",
                a.string("levels-dir", "levels"),
                champion.metadata.level_hash,
                champion.metadata.level_name
            )
        })?,
    };

    println!("champion {}", champion_path);
    println!("level    {} ({})", level.name, level.content_hash_string());
    println!("trained  {}", champion.summary());
    println!(
        "clock    {} ({} frames of the level's {})",
        sim::format_time(champion.metadata.frames),
        champion.metadata.frames,
        sim::format_time(sim::frame_budget(&level))
    );

    let (genome, result) = replay_champion(&level, &champion)?;
    let expected = champion.metadata.trajectory_hash.clone();
    let actual = format!("{:016x}", result.trajectory_hash);

    println!(
        "replayed {} after {} frames, fitness {:.2}, best_x {:.1}, stomps {}, jumps {}, checkpoints {}",
        result.outcome.as_str(),
        result.frames,
        result.fitness,
        result.best_x,
        result.stomps,
        result.jumps,
        result.checkpoints
    );
    if let Some(cause) = result.death_cause {
        if let Some((x, y)) = result.death_pos {
            println!(
                "why      killed by {} at ({x:.2}, {y:.2}) on frame {}",
                cause.as_str(),
                result.frames
            );
        }
    }

    if actual != expected {
        return Err(format!(
            "trajectory mismatch: champion records {expected}, replay produced {actual}"
        ));
    }
    if (result.fitness - champion.fitness).abs() > 1e-3 {
        return Err(format!(
            "fitness mismatch: champion records {:.4}, replay produced {:.4}",
            champion.fitness, result.fitness
        ));
    }
    // Touch the genome so a shape problem surfaces here rather than in a player.
    genome
        .validate()
        .map_err(|e| format!("champion genome does not validate: {e}"))?;
    println!("verified identical trajectory ({actual})");
    Ok(())
}

pub(crate) fn find_level_by_hash(dir: &str, hash: &str) -> Result<Option<Level>, String> {
    if !Path::new(dir).exists() {
        return Ok(None);
    }
    let mut paths = list_levels(dir)?;
    paths.sort();
    for path in paths {
        let level = Level::from_json_file(&path)?;
        if level.content_hash_string() == hash {
            return Ok(Some(level));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// score
// ---------------------------------------------------------------------------

/// Score a saved champion on one level or a whole suite of them.
///
/// **Why this exists.** Every claim this project has made about a policy has been a claim about a
/// *specialist*: `train` fits one level and a champion is played only where it was trained. The
/// suite trainer (REVIEW.md §4 item 5) is the first mechanism that asks a population to hold more
/// than one level at once, and a claim like that has to be checkable on an artifact — so this is
/// the read that makes it checkable: point it at any champion file, name the levels, and it says
/// what that one policy actually does on each of them. It is the generalisation half of what
/// `replay` is for the recorded-episode half.
///
/// **The episodes are fresh ones, and that is deliberate.** They are the episodes this genome
/// would have been judged on in generation 0 of a default run — the trainer's own episode policy
/// ([`evo::ga::episode_seed_for`], asked of `GaConfig::default()` with the master seed the champion
/// recorded) rather than the single episode the file carries. `replay` verifies the recorded one;
/// this asks the wider question, and asking it with the same seeds every time is what makes two
/// champions comparable — which is also why the seed is asked of the trainer rather than rebuilt
/// here: under `common_episodes` the whole population of a generation shares one episode, and a
/// second copy of that rule in this command is exactly the kind of quiet divergence the two ends of
/// this project have been bitten by before.
///
/// Trials come from the **trainer's own rule** rather than from what the champion was trained
/// with, so a level with walkers in it is judged on three patrol phases: a policy that solves a
/// patrol on one phase and dies on the next has not solved the level, and this is the command
/// where that would otherwise be the easiest thing to hide.
/// `shortcut`: what a champion skipped relative to the level's intended route (WP14).
///
/// A report, never a score. The episode is the champion's own — its recorded seed, its cadence —
/// so "this route skipped 40% of your level" is a statement about the run the file describes, not
/// about a second simulation with a different clock.
pub fn shortcut(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(SHORTCUT_OPTS))?;
    let champion_path = usage(a.required_path("champion"))?;
    let raw = fs::read_to_string(&champion_path)
        .map_err(|e| format!("cannot read {champion_path}: {e}"))?;
    let champion =
        ChampionFile::from_json_str(&raw).map_err(|e| format!("{champion_path}: {e}"))?;
    let genome = champion.to_genome()?;
    // The level is resolved by hash first, for the same reason `score` does: a renamed level is
    // still the level the policy was fitted to, while a hand-named one may not be.
    let level = match a.opt("level") {
        Some(path) => load_level(path)?,
        None => find_level_by_hash(
            &a.string("levels-dir", "levels"),
            &champion.metadata.level_hash,
        )?
        .ok_or_else(|| {
            format!(
                "no level in {} matches this champion's hash; name one with --level",
                a.string("levels-dir", "levels")
            )
        })?,
    };
    champion.check_compat(&level)?;
    let eval = EvalConfig {
        action_repeat: champion.metadata.action_repeat,
        cut_stuck_frames: 0,
    };
    let seed = usage(a.u64_opt("seed", champion.metadata.episode_seed))?;
    let report = evo::trace_shortcut(&level, &genome, &eval, seed);
    println!("level   {} ({})", level.name, level.content_hash_string());
    println!(
        "route   {} hops: {:?}",
        report.intended.len().saturating_sub(1),
        report.intended
    );
    println!("walked  {:?}", report.visited);
    println!("skipped {:?}", report.skipped);
    println!("{}", report.note);
    if report.won {
        println!(
            "shortcut {:.0}%{}",
            report.skip_percent(),
            report
                .first_skipped
                .map(|s| format!(", first skipped stand {s}"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

/// **TAS polish (WP12).** Take a validated champion's own episode to one action per tick and run a
/// deterministic local search over tick substitutions and press shifts.
///
/// The result is written as a route artifact, never as a champion: it is labelled machine-polished,
/// it records the one seed it is true under, and it carries the final state hash so a replay can
/// prove it walked the same line.
pub fn tas(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(TAS_OPTS))?;
    let champion_path = usage(a.required_path("champion"))?;
    let raw = fs::read_to_string(&champion_path)
        .map_err(|e| format!("cannot read {champion_path}: {e}"))?;
    let champion =
        ChampionFile::from_json_str(&raw).map_err(|e| format!("{champion_path}: {e}"))?;
    let genome = champion.to_genome()?;
    let level = match a.opt("level") {
        Some(path) => load_level(path)?,
        None => find_level_by_hash(
            &a.string("levels-dir", "levels"),
            &champion.metadata.level_hash,
        )?
        .ok_or_else(|| {
            format!(
                "no level in {} matches this champion's hash; name one with --level",
                a.string("levels-dir", "levels")
            )
        })?,
    };
    champion.check_compat(&level)?;
    let eval = EvalConfig {
        action_repeat: champion.metadata.action_repeat,
        cut_stuck_frames: 0,
    };
    let episode_seed = usage(a.u64_opt("seed", champion.metadata.episode_seed))?;
    let defaults = evo::tas::TasConfig::default();
    let cfg = evo::tas::TasConfig {
        iterations: usage(a.usize_opt("iterations", defaults.iterations))?,
        seed: usage(a.u64_opt("seed", defaults.seed))?,
        shift: usage(a.usize_opt("shift", defaults.shift as usize))? as u32,
        patience: usage(a.usize_opt("patience", defaults.patience))?,
    };
    let route = evo::tas::route_from_genome(&level, &genome, &eval, episode_seed);
    println!("level   {} ({})", level.name, level.content_hash_string());
    println!("start   {}", route.note);
    if !route.won() {
        return Err(route.note);
    }
    let report = evo::tas::polish(&level, &route, cfg);
    println!("{}", report.line());
    // The artifact is a claim only if it replays. Check here, before writing, so a broken search
    // cannot leave a file behind that looks like a route.
    let played = evo::tas::play(&level, report.route.seed, &report.route.actions);
    if !played.won() {
        return Err(format!(
            "tas produced a route that does not win on replay ({} at {} frames); not written",
            played.outcome.as_str(),
            played.frames
        ));
    }
    if played.final_state_hash != report.route.final_state_hash {
        return Err(format!(
            "tas route replays to state {:016x} but records {:016x}; not written",
            played.final_state_hash, report.route.final_state_hash
        ));
    }
    let path = a
        .opt("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}-tas.json", level.name)));
    let path_str = path.display().to_string();
    fs::write(&path, report.route.to_json_pretty()?)
        .map_err(|e| format!("cannot write {path_str}: {e}"))?;
    println!(
        "wrote   {} ({} frames, machine-polished, seed {})",
        path_str, report.route.frames, report.route.seed
    );
    Ok(())
}

/// **Segment distillation (WP4).** Plan the level, split the route at the ground stands it lands
/// on, and write one practice level per leg.
///
/// This is the artifact half of the package: each segment is a level with its spawn at the
/// segment's entry and its flag at its exit, plus the planner's block actions for that span. The
/// campaign that trains them in sequence is `curriculum::plan_segments` plus the existing detour
/// machinery; writing the levels first is what makes the split reviewable — a human can open a
/// segment and check that it is one crossing rather than a whole level wearing a smaller name.
pub fn segments(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(SEGMENTS_OPTS))?;
    let level_path = usage(a.required_path("level"))?;
    let level = load_level(&level_path)?;
    let seed = usage(a.u64_opt("seed", 1))?;
    let cfg = sim::plan::PlanConfig {
        depth: usage(a.usize_opt("depth", sim::plan::PlanConfig::default().depth as usize))? as u32,
        beam: usage(a.usize_opt("beam", sim::plan::PlanConfig::default().beam))?,
        ..sim::plan::PlanConfig::default()
    };
    let outcome = sim::plan::plan_level(&level, seed, cfg);
    println!("level   {} ({})", level.name, level.content_hash_string());
    println!("{}", outcome.note);
    let route = outcome.route.ok_or_else(|| {
        format!(
            "no planned route, so there are no segments: {}",
            outcome.note
        )
    })?;
    let segments = evo::plan_segments(&level, &route)?;
    let out_dir = a.string("out", "segments");
    fs::create_dir_all(&out_dir).map_err(|e| format!("cannot create {out_dir}: {e}"))?;
    let stem = Path::new(&level_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| level.name.clone());
    println!("segments {}", segments.len());
    for segment in &segments {
        let path =
            Path::new(&out_dir).join(format!("{stem}-segment-{:02}.json", segment.index + 1));
        segment.level.to_json_file(&path)?;
        println!(
            "  {:02}  ({}, {}) -> ({}, {})  blocks {}..{}  {} frames  {}",
            segment.index + 1,
            segment.entry.0,
            segment.entry.1,
            segment.exit.0,
            segment.exit.1,
            segment.start_block,
            segment.end_block,
            segment.frames,
            path.display()
        );
    }
    Ok(())
}

/// **Deliberate play.** The model-based controller looks at the real simulator instead of
/// sampling actions: at every decision block it searches the level's own physics a few blocks
/// ahead and takes the step that is still alive and closest to the route's next landing.
///
/// This is the missing half of the agent. The GA has no model — it can only tell whether a
/// finished episode scored — while this has the world itself, so `wait` is a candidate and a
/// mover's arrival is something it can see. The trajectory is written as JSON so the same
/// episode can be replayed or used as a demonstration.
pub fn think(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(THINK_OPTS))?;
    let level_path = usage(a.required_path("level"))?;
    let level = load_level(&level_path)?;
    let seed = usage(a.u64_opt("seed", 1))?;
    println!("level   {} ({})", level.name, level.content_hash_string());
    let started = Instant::now();
    // **Two kinds of deliberation, one command.** The default is the local controller:
    // cheap enough to run while a window is open, and it discovers waits and jump
    // timings from the physics. `--plan` is the timed planner, which searches whole
    // routes and therefore sees a deck's whole schedule; it costs seconds to a minute
    // and is the answer of record on a level the local horizon cannot untangle.
    let (report, actions, searched) = if a.has_flag("plan") {
        let plan_cfg = sim::plan::PlanConfig {
            depth: usage(a.usize_opt(
                "plan-depth",
                sim::plan::PlanConfig::default().depth as usize,
            ))? as u32,
            beam: usage(a.usize_opt("plan-beam", sim::plan::PlanConfig::default().beam))?,
            ..sim::plan::PlanConfig::default()
        };
        let outcome = sim::plan::plan_level(&level, seed, plan_cfg);
        println!(
            "plan    {} ({:.1}s)",
            outcome.note,
            started.elapsed().as_secs_f32()
        );
        let route = outcome
            .route
            .ok_or_else(|| format!("no timed route: {}", outcome.note))?;
        let eval = EvalConfig::default();
        let play = evo::timed_play(&level, &route, &eval);
        println!("replay  {}", play.note);
        let actions: Vec<AgentAction> = play.decisions.iter().map(|d| d.action).collect();
        // The walk's own final state, for the one reading `RoutePlay` does not carry: a
        // route is a claim about the flag, and `best_x` is what a person uses to see
        // where a failed one stopped.
        let mut final_sim = sim::Sim::new(&level, seed);
        for decision in &play.decisions {
            for _ in 0..eval.action_repeat.max(1) {
                if final_sim.outcome().is_finished() {
                    break;
                }
                final_sim.step(decision.action);
            }
        }
        let report = evo::ThinkReport {
            actions: actions.clone(),
            outcome: play.outcome,
            frames: play.frames,
            best_x: final_sim.stats().best_x,
            route_hops_closed: play.hops_closed,
            replans: 0,
            trail: Vec::new(),
            death_cause: None,
            note: format!(
                "planned a whole route in {} decisions and walked it in {} frames",
                actions.len(),
                play.frames
            ),
        };
        (
            report,
            actions,
            Some(("whole route", plan_cfg.depth, plan_cfg.beam)),
        )
    } else {
        let cfg = evo::ThinkConfig {
            horizon: usage(a.usize_opt("horizon", evo::ThinkConfig::default().horizon as usize))?
                as u32,
            beam: usage(a.usize_opt("beam", evo::ThinkConfig::default().beam))?,
            ..evo::ThinkConfig::default()
        };
        let report = evo::think(&level, seed, cfg);
        let actions = report.actions.clone();
        (
            report,
            actions,
            Some(("local horizon", cfg.horizon, cfg.beam)),
        )
    };
    println!("{}", evo::navigate::describe(&level, &report));
    if let Some(cause) = report.death_cause {
        println!("death   {cause}");
    }
    if let Some((what, first, second)) = searched {
        println!(
            "search  {}: {} / {}, {:.1}s",
            what,
            first,
            second,
            started.elapsed().as_secs_f32()
        );
    }
    if a.has_flag("trace") {
        for frame in &report.trail {
            println!(
                "  {:4}  x {:6.2} y {:6.2} vx {:+.3} vy {:+.3} {:<18} stand {:?} grounded {} aim ({:.1}, {:.1})",
                frame.frames,
                frame.x,
                frame.y,
                frame.vx,
                frame.vy,
                frame.action.name(),
                frame.stand,
                frame.grounded,
                frame.aim.0,
                frame.aim.1
            );
        }
    }
    if let Some(out) = a.opt("out") {
        let path = PathBuf::from(out);
        let stem = Path::new(&level_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| level.name.clone());
        let path = if path.extension().is_some() {
            path
        } else {
            path.join(format!("{stem}-think.json"))
        };
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let actions: Vec<u32> = actions.iter().map(|a| a.index() as u32).collect();
        let body = serde_json::json!({
            "format": "evo-platformer-thought",
            "level": level.name,
            "level_hash": level.content_hash_string(),
            "seed": seed,
            "outcome": report.outcome.as_str(),
            "frames": report.frames,
            "best_x": report.best_x,
            "actions": actions,
            "note": report.note,
        });
        fs::write(
            &path,
            serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("wrote   {}", path.display());
    }
    Ok(())
}

/// **Style titles (WP15).** Check a champion's own winning route against every constrained
/// category the verifier can prove, and print the titles it holds.
///
/// One candidate can hold several titles; a title nobody holds is printed as unclaimed rather
/// than omitted, because "the champion sprints" is a result about the level.
pub fn styles(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(STYLES_OPTS))?;
    let champion_path = usage(a.required_path("champion"))?;
    let raw = fs::read_to_string(&champion_path)
        .map_err(|e| format!("cannot read {champion_path}: {e}"))?;
    let champion =
        ChampionFile::from_json_str(&raw).map_err(|e| format!("{champion_path}: {e}"))?;
    let genome = champion.to_genome()?;
    let level = match a.opt("level") {
        Some(path) => load_level(path)?,
        None => find_level_by_hash(
            &a.string("levels-dir", "levels"),
            &champion.metadata.level_hash,
        )?
        .ok_or_else(|| {
            format!(
                "no level in {} matches this champion's hash; name one with --level",
                a.string("levels-dir", "levels")
            )
        })?,
    };
    champion.check_compat(&level)?;
    let eval = EvalConfig {
        action_repeat: champion.metadata.action_repeat,
        cut_stuck_frames: 0,
    };
    let seed = usage(a.u64_opt("seed", champion.metadata.episode_seed))?;
    let result = run_episode(&level, &genome, &eval, seed);
    let route = evo::tas::route_from_genome(&level, &genome, &eval, seed);
    println!("level   {} ({})", level.name, level.content_hash_string());
    println!(
        "champion genome {}: {} in {} frames, {} jumps, {} wall jump(s)",
        genome.id,
        result.outcome.as_str(),
        result.frames,
        result.jumps,
        result.wall_jumps
    );
    if !route.won() {
        return Err(format!(
            "the champion does not win on this seed, so no style title can be checked: {}",
            route.note
        ));
    }
    for style in evo::Style::ALL {
        let check = evo::style::check(style, &route, &result);
        println!(
            "  {:<14} {:<9} {}",
            style.title(),
            if check.eligible { "held" } else { "not held" },
            match check.reason {
                Some(reason) => reason,
                None => format!("{} ({})", style.constraint(), check.score),
            }
        );
    }
    Ok(())
}

/// **Segment campaign (WP4).** Plan the level, split it into legs, train each leg with the main
/// level in the suite, then spend the remaining budget on the whole level.
///
/// This is the package's usable form: a run that cannot learn the whole route is given one
/// crossing at a time without ever being allowed to forget the crossing before it. The per-leg
/// checkpoints are written beside the run for inspection, and the final champion is written by
/// the same [`ChampionFile`] path as every other run — a campaign is a training schedule, not a
/// different kind of result.
pub fn campaign(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(CAMPAIGN_OPTS))?;
    let level_path = usage(a.required_path("level"))?;
    let level = load_level(&level_path)?;
    let seed = usage(a.u64_opt("seed", 1))?;
    let plan_cfg = sim::plan::PlanConfig {
        depth: usage(a.usize_opt("depth", sim::plan::PlanConfig::default().depth as usize))? as u32,
        beam: usage(a.usize_opt("beam", sim::plan::PlanConfig::default().beam))?,
        ..sim::plan::PlanConfig::default()
    };
    let outcome = sim::plan::plan_level(&level, seed, plan_cfg);
    println!("level   {} ({})", level.name, level.content_hash_string());
    println!("{}", outcome.note);
    let route = outcome.route.ok_or_else(|| {
        format!(
            "no planned route, so there is no campaign: {}",
            outcome.note
        )
    })?;
    let segments = evo::plan_segments(&level, &route)?;
    let segment_generations = usage(a.usize_opt("segment-generations", 40))? as u32;
    let main_generations = usage(a.usize_opt("generations", 200))?;
    let pop = usage(a.usize_opt("pop", 200))?;
    let checkpoints = a.string("checkpoints", "checkpoints");
    fs::create_dir_all(&checkpoints).map_err(|e| format!("cannot create {checkpoints}: {e}"))?;
    let base = GaConfig {
        pop_size: pop,
        master_seed: seed,
        // The campaign is a mastery-shaped run: the league arms when the record solves and the
        // crowned fitness still governs the champion.
        mastery: true,
        ..GaConfig::default()
    }
    .sanitized(None);
    let mut trainer = Trainer::new(base.clone());
    let stem = Path::new(&level_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| level.name.clone());
    let checkpoint = Path::new(&checkpoints).join(format!("{stem}.campaign.population.json"));
    println!(
        "campaign {} segments × {} generations, then {} generations on the whole level (pop {})",
        segments.len(),
        segment_generations,
        main_generations,
        pop
    );
    let started = Instant::now();
    let campaign = evo::segment_campaign(
        &mut trainer,
        &level,
        &route,
        &segments,
        segment_generations,
        &checkpoint,
        base.mutation_rate,
    )?;
    for (i, step) in campaign.steps.iter().enumerate() {
        println!(
            "  leg {:02}/{}  {}",
            i + 1,
            campaign.segments,
            step.note(&level.name)
        );
    }
    println!(
        "segments done in {:.1} s; training the whole level",
        started.elapsed().as_secs_f64()
    );

    // **Item 9: fuse the leg specialists.** Before a single whole-level generation is trained,
    // breed pairwise crosses of distinct legs and put them in generation zero beside the
    // campaign's own whole-level genomes. The crossover is the shipped one — the experiment is
    // *who the parents are*, not a new operator — and the crosses lead the seeding list so a
    // caller asking for k slots gets k crosses rather than an interleaving nobody can attribute
    // a win to.
    //
    // The whole-level half of `fused_seeding`'s list is passed empty on purpose: those genomes
    // are already this trainer's population, and seeding them back into their own last slots
    // would spend slots on policies the run has. `fused_seeding` still de-duplicates, so a leg
    // whose clone reproduces an earlier cross does not take a second slot.
    let fuse = usage(a.usize_opt("fuse", 0))?;
    if fuse > 0 {
        let cross_rate = usage(a.f32_opt("fuse-cross-rate", 1.0))?;
        if !(0.0..=1.0).contains(&cross_rate) {
            return Err(format!(
                "usage: --fuse-cross-rate {cross_rate} is not a probability; pass a value in \
                 0.0..=1.0"
            ));
        }
        let legs = campaign.specialists().len();
        if legs < 2 {
            // A cross needs two parents. Saying so is better than silently seeding one clone or
            // nothing at all: the operator asked for a fused arm and is not getting one.
            return Err(format!(
                "--fuse needs at least two leg specialists, and this campaign fitted {legs}: the \
                 arm is two parents, not one leg offered twice"
            ));
        }
        let cfg = FuseConfig {
            shares: fuse,
            cross_rate,
        };
        let mut rng = sim::rng::rng_from_seed(seed ^ 0x9E37_79B9_7F4A_7C15);
        let crosses = fused_seeding(&campaign, &[], &cfg, &mut rng);
        let want = fuse.min(pop / 4).min(crosses.len());
        let seeded = trainer.seed_donors(&crosses, want)?;
        println!(
            "fused   {seeded} specialist cross(es) into generation zero ({} bred from {legs} leg \
             specialists, {:.0} % cross rate)",
            crosses.len(),
            cross_rate * 100.0
        );
    }
    for generation in 0..main_generations {
        let report = trainer.step(&level);
        if generation % 10 == 0 || generation + 1 == main_generations {
            println!(
                "generation {:5}  max {:10.1}  won {}  solved {}",
                report.generation, report.max_fitness, report.wins, report.solved
            );
        }
    }
    let best = trainer
        .best_ever()
        .ok_or_else(|| "the campaign produced no record".to_string())?;
    let mut champion =
        ChampionFile::new(&best.genome, &best.result, &level, best.generation, &base);
    champion.metadata.phase_gap =
        evo::measure_phase_gap(&level, &best.genome, &base, best.generation, best.validated);
    let out_dir = a.string("out", "champions");
    fs::create_dir_all(&out_dir).map_err(|e| format!("cannot create {out_dir}: {e}"))?;
    let out = Path::new(&out_dir).join(format!("{stem}-campaign.json"));
    champion.save(&out)?;
    println!(
        "best    fitness {:.1}, {} frames, won {}\nwrote   {} ({:.1} s)",
        best.result.mean_fitness,
        best.result.frames,
        best.result.won(),
        out.display(),
        started.elapsed().as_secs_f64()
    );
    if let Some(gap) = champion.metadata.phase_gap {
        println!(
            "gap     training {}/{} ({:.0} %)  held-out {}/{} ({:.0} %)  gap {:+.2}",
            gap.train_wins,
            gap.train_episodes,
            gap.training_rate() * 100.0,
            gap.held_wins,
            gap.held_episodes,
            gap.held_rate() * 100.0,
            gap.gap()
        );
    }
    Ok(())
}

/// **Islands (WP7).** Run several independent populations with their own settings and a ring of
/// migrants, then write the best record any island found.
///
/// Single-population training remains the default; this is the arm the plan's equal-episode gate
/// needs. Each island is a normal [`evo::Trainer`], so everything recorded about it — champion
/// validation, mastery, the technique archive — behaves exactly as it does in a one-population run.
pub fn islands(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(ISLANDS_OPTS))?;
    let level_path = usage(a.required_path("level"))?;
    let level = load_level(&level_path)?;
    let generations = usage(a.usize_opt("generations", 300))?;
    let count = usage(a.usize_opt("islands", 4))?;
    let pop = usage(a.usize_opt("pop", 200))?;
    let seed = usage(a.u64_opt("seed", 1))?;
    let migration_every = usage(a.usize_opt("migration-every", 25))? as u32;
    let mut base = GaConfig {
        pop_size: pop,
        master_seed: seed,
        ..GaConfig::default()
    };
    base = base.sanitized(None);
    let plan = evo::IslandConfig {
        islands: count.clamp(1, 12),
        migration_every,
        base: base.clone(),
    };
    let mut ring = evo::Islands::new(plan);
    let started = Instant::now();
    println!("level   {} ({})", level.name, level.content_hash_string());
    println!(
        "islands {}  pop {} each  migration every {} generations  seed {}",
        ring.islands.len(),
        base.pop_size,
        migration_every,
        seed
    );
    for generation in 0..generations {
        let step = ring.step(&level);
        if generation % 10 == 0 || generation + 1 == generations {
            let best = step
                .reports
                .iter()
                .map(|r| r.max_fitness)
                .fold(f32::NEG_INFINITY, f32::max);
            println!(
                "generation {:5}  best island maximum {:10.1}  solved {}",
                step.generation,
                best,
                step.reports.iter().map(|r| r.solved).sum::<usize>()
            );
        }
        if a.has_flag("stop-on-win") && ring.solved() {
            println!("solved after {} generations", generation + 1);
            break;
        }
    }
    let Some((island, best)) = ring.best() else {
        return Err("no island produced a record".into());
    };
    let mut champion =
        ChampionFile::new(&best.genome, &best.result, &level, best.generation, &base);
    champion.metadata.phase_gap =
        evo::measure_phase_gap(&level, &best.genome, &base, best.generation, best.validated);
    let out_dir = a.string("out", "champions");
    let stem = Path::new(&level_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| level.name.clone());
    let out = Path::new(&out_dir).join(format!("{stem}-islands.json"));
    champion.save(&out)?;
    println!(
        "best    island {island}: fitness {:.1}, {} frames, won {}\nwrote   {} ({:.1} s)",
        best.result.mean_fitness,
        best.result.frames,
        best.result.won(),
        out.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

const GENERALIST_OPTS: &[&str] = &[
    "levels",
    "generations",
    "pop",
    "seed",
    "out",
    "refresh",
    "quiet",
];

/// **The standing generalist population (Item 6).**
///
/// One population kept permanently trained across the pack, written as an ordinary
/// [`PopulationFile`] so `--generalist <path> --generalist-share F` can seed any run from it.
///
/// Two properties are the item, not decoration:
///
/// * **Rotation.** A generation is spent on one level, taken in round-robin order from
///   `--levels`, with the cursor derived from the checkpoint's own generation count. So every
///   level gets one generation in `n`, no level owns the population, and re-running the command
///   continues the rotation rather than restarting it at the first level. Training the pack as a
///   suite would rank by worst level instead; rotation is what the item asks for, and the two
///   differ in what the population is selected for, so this command trains one level at a time.
/// * **Refresh.** `--refresh <dir>` reads the champions a pack run produced and puts them in the
///   back slots, where the next generation's ranking judges them. The transfer therefore runs
///   both ways: the generalist seeds pack runs, and pack champions re-enter the generalist.
///
/// The checkpoint stamps the first level of `--levels`, which is a note rather than a gate
/// ([`PopulationFile`] is expected to move between levels) — a reader who resumes it on another
/// level is told so.
pub fn generalist(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(GENERALIST_OPTS))?;
    let list = a.string("levels", "");
    if list.trim().is_empty() {
        return Err(
            "usage: generalist needs --levels a.json,b.json: one level per generation, in rotation"
                .into(),
        );
    }
    let mut levels: Vec<Level> = Vec::new();
    for path in parse_level_list(&list)? {
        levels.push(load_level(&path)?);
    }
    let pop = usage(a.usize_opt("pop", 200))?;
    let seed = usage(a.u64_opt("seed", 1))?;
    let out_dir = a.string("out", "generalists");
    fs::create_dir_all(&out_dir).map_err(|e| format!("cannot create {out_dir}: {e}"))?;
    let path = Path::new(&out_dir).join("generalist.json");
    let base = GaConfig {
        pop_size: pop,
        master_seed: seed,
        ..GaConfig::default()
    }
    .sanitized(None);

    let mut trainer = if path.exists() {
        let checkpoint = PopulationFile::load(&path)?;
        println!("resuming {}", checkpoint.summary());
        let mut trainer = Trainer::resume(base, checkpoint.to_genomes()?, checkpoint.generation)?;
        if let Some(record) = checkpoint.best_genome()? {
            trainer.seed_record_from_checkpoint(&levels[0], record)?;
        }
        trainer
    } else {
        Trainer::new(base)
    };

    // Step 3: the pack's own champions re-enter the generalist between pack runs.
    if let Some(dir) = a.opt("refresh") {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .map_err(|e| format!("cannot read {dir}: {e}"))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
            .collect();
        entries.sort();
        let mut donors = Vec::new();
        let mut skipped = 0usize;
        for entry in &entries {
            match ChampionFile::load_seed(entry).and_then(|c| c.to_genome()) {
                Ok(genome) if genome.shape == trainer.config.shape => donors.push(genome),
                _ => skipped += 1,
            }
        }
        // A quarter of the population at most, the same cap every other side-payment makes: the
        // pack's champions are a refresh, not a replacement for the population that was trained.
        let want = donors.len().min(pop / 4);
        let taken = trainer.seed_donors(&donors, want)?;
        println!(
            "refresh {taken} champion(s) from {dir} take the last slots ({skipped} skipped: another \
             shape or not a champion)"
        );
    }

    let generations = usage(a.usize_opt("generations", 0))?;
    let quiet = a.has_flag("quiet");
    let started = Instant::now();
    for _ in 0..generations {
        let index = trainer.generation as usize % levels.len();
        let report = trainer.step(&levels[index]);
        if !quiet && (report.generation % 10 == 0 || report.solved > 0) {
            println!(
                "generation {:5}  {}  max {:10.1}  solved {}",
                report.generation,
                levels[index].name,
                report.max_fitness,
                report.solved
            );
        }
    }

    let best = trainer.best_ever().map(|b| b.genome.clone());
    let file = PopulationFile::new(
        &trainer.genomes,
        trainer.generation,
        &levels[0],
        &trainer.config,
        format!(
            "pack generalist over {} level(s), rotating one generation per level ({} generations \
             in this population)",
            levels.len(),
            trainer.generation
        ),
    )
    .with_best(best.as_ref());
    file.save(&path)?;
    println!(
        "generalist {} genomes, generation {}\nwrote   {} ({:.1} s)",
        trainer.genomes.len(),
        trainer.generation,
        path.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// **Trick hunt (WP14).** Search a non-goal objective and write the trajectory as a discovery
/// artifact.
///
/// The hunt never claims the level is solved: it reports the best reading of its objective (height,
/// airtime, speed or ground-free distance) and the trajectory that produced it. The artifact is a
/// [`evo::tas::TasRoute`] of kind [`evo::tas::TasKind::Discovered`], so it replays through the same
/// machinery as every other route and cannot be mistaken for a champion.
pub fn hunt(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(HUNT_OPTS))?;
    let level = load_level(&usage(a.required_path("level"))?)?;
    let objective = match a.string("objective", "height").as_str() {
        "height" => sim::plan::HuntObjective::Height,
        "airtime" | "air" => sim::plan::HuntObjective::Airtime,
        "speed" => sim::plan::HuntObjective::Speed,
        "distance" | "ground-free-distance" => sim::plan::HuntObjective::GroundFreeDistance,
        other => {
            return Err(format!(
                "unknown hunt objective {other:?} (height, airtime, speed, distance)"
            ))
        }
    };
    let cfg = sim::plan::PlanConfig {
        depth: usage(a.usize_opt("depth", sim::plan::PlanConfig::default().depth as usize))? as u32,
        beam: usage(a.usize_opt("beam", sim::plan::PlanConfig::default().beam))?,
        ..sim::plan::PlanConfig::default()
    };
    let seed = usage(a.u64_opt("seed", 1))?;
    let outcome = sim::plan::hunt(&level, seed, cfg, objective);
    println!("level   {} ({})", level.name, level.content_hash_string());
    println!("{}", outcome.note);
    let Some(route) = outcome.route else {
        return Err(outcome.note);
    };
    // The planner speaks in decision blocks; a TAS artifact speaks in physics ticks. Expanding
    // here is what makes the two the same trajectory rather than a route that replays one tick
    // out of every four.
    let repeat = evo::ACTION_REPEAT;
    let tick_actions: Vec<AgentAction> = route
        .actions
        .iter()
        .flat_map(|action| std::iter::repeat_n(*action, repeat as usize))
        .collect();
    let artifact = evo::tas::route_from_actions(&level, seed, &tick_actions, outcome.note.clone());
    // A discovery is only a discovery if it replays. Check the hash before writing, exactly as
    // TAS polish does, so an artifact in the wild always describes a trajectory this build runs.
    let played = evo::tas::play(&level, seed, &artifact.actions);
    if played.final_state_hash != artifact.final_state_hash {
        return Err(format!(
            "hunt trajectory replays to state {:016x} but records {:016x}; not written",
            played.final_state_hash, artifact.final_state_hash
        ));
    }
    let path = a
        .opt("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}-hunt.json", level.name)));
    let path_str = path.display().to_string();
    fs::write(&path, artifact.to_json_pretty()?)
        .map_err(|e| format!("cannot write {path_str}: {e}"))?;
    println!(
        "wrote   {} ({} blocks, {} frames, {} objective {:.3})",
        path_str,
        artifact.actions.len(),
        artifact.frames,
        outcome.objective.as_str(),
        outcome.best
    );
    Ok(())
}

/// **The trick codex (WP16).** Detect what a champion's winning episode discovered, print it, and
/// record it in the level's codex.
///
/// A codex entry carries a tick-resolution replay, which is why this is a command rather than
/// something a watched run does on every generation: the evidence is worth a full re-run of one
/// episode, and only a new find is worth keeping.
pub fn codex(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(CODEX_OPTS))?;
    let champion_path = usage(a.required_path("champion"))?;
    let raw = fs::read_to_string(&champion_path)
        .map_err(|e| format!("cannot read {champion_path}: {e}"))?;
    let champion =
        ChampionFile::from_json_str(&raw).map_err(|e| format!("{champion_path}: {e}"))?;
    let genome = champion.to_genome()?;
    let level = match a.opt("level") {
        Some(path) => load_level(path)?,
        None => find_level_by_hash(
            &a.string("levels-dir", "levels"),
            &champion.metadata.level_hash,
        )?
        .ok_or_else(|| {
            format!(
                "no level in {} matches this champion's hash; name one with --level",
                a.string("levels-dir", "levels")
            )
        })?,
    };
    champion.check_compat(&level)?;
    let eval = EvalConfig {
        action_repeat: champion.metadata.action_repeat,
        cut_stuck_frames: 0,
    };
    let seed = usage(a.u64_opt("seed", champion.metadata.episode_seed))?;
    let result = run_episode(&level, &genome, &eval, seed);
    println!("level   {} ({})", level.name, level.content_hash_string());
    println!(
        "episode {} in {} frames, {}",
        result.outcome.as_str(),
        result.frames,
        if result.won() {
            "a win"
        } else {
            "not a win — a codex records what a route found, so a loss has nothing to record"
        }
    );
    let found = evo::detect_discoveries(&level, &genome, &result, &eval, 0, true);
    if found.is_empty() {
        println!("found   nothing this build can name; the route used no catalogued technique and no shortcut");
        return Ok(());
    }
    for discovery in &found {
        println!(
            "found   {} — {} ({} frames, genome {})",
            discovery.name, discovery.line, discovery.frames, discovery.genome_id
        );
    }
    // The replay is the entry's evidence and `detect` has already replayed it to check it; say
    // which of the two answers the journal will carry rather than implying one.
    let with_evidence = found.iter().filter(|d| !d.replay.is_empty()).count();
    println!(
        "replay  {with_evidence} of {} find(s) carries tick actions that replay to the flag",
        found.len()
    );

    // Keyed on the **level's** file stem, which is the same key the window uses for its codex
    // and for every other artifact: a champion named `<level>-seeded.json` is a policy on the
    // same level and belongs in the same journal, not a second one named after the file.
    let stem = a
        .opt("level")
        .and_then(|path| {
            Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .or_else(|| {
            Path::new(&champion_path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| level.name.clone());
    // `--codex-dir` names the codex directory itself (the same one `<out>/codex/` is by default),
    // so a caller who points it at an existing journal keeps writing to that journal.
    let dir = a.string("codex-dir", &format!("champions/{}", evo::CODEX_DIR));
    let path = evo::Codex::path_in(Path::new(&dir), &stem);
    let mut journal = evo::Codex::load(&level, &path)?;
    let new = journal.record_all(found);
    if a.has_flag("dry-run") {
        println!("dry run: would record {new:?} in {}", path.display());
        return Ok(());
    }
    journal.save(&path)?;
    println!(
        "codex   {} entr{} in {} (new: {})",
        journal.len(),
        if journal.len() == 1 { "y" } else { "ies" },
        path.display(),
        if new.is_empty() {
            "none".to_string()
        } else {
            new.join(", ")
        }
    );
    Ok(())
}

/// **"Break my level" (WP16).** The level-design QA report: what the intended route is, what a
/// champion actually walked, what it skipped, what it discovered, and what a non-goal hunt finds
/// beyond the flag.
///
/// It is deliberately the same machinery the rest of the tree uses — `shortcut` for the diff,
/// the technique table for the names, `plan::hunt` for the trick — presented as one answer to the
/// question a level author asks: *what does this level actually teach?*
pub fn break_my_level(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(BREAK_OPTS))?;
    let champion = match a.opt("champion") {
        Some(path) => {
            let raw = fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            Some((path.to_string(), ChampionFile::from_json_str(&raw)?))
        }
        None => None,
    };
    let level = match (a.opt("level"), &champion) {
        (Some(path), _) => load_level(path)?,
        (None, Some((_, champion))) => find_level_by_hash(
            &a.string("levels-dir", "levels"),
            &champion.metadata.level_hash,
        )?
        .ok_or_else(|| {
            format!(
                "no level in {} matches this champion's hash; name one with --level",
                a.string("levels-dir", "levels")
            )
        })?,
        (None, None) => return Err("usage: break needs --level (or --champion)".to_string()),
    };
    let seed = usage(
        a.u64_opt(
            "seed",
            champion
                .as_ref()
                .map_or(1, |(_, c)| c.metadata.episode_seed),
        ),
    )?;
    let intended = evo::shortcut::intended_route(&level);
    println!("level    {} ({})", level.name, level.content_hash_string());
    println!(
        "intended {} stands, {} hops",
        intended.len(),
        intended.len().saturating_sub(1)
    );

    let mut discoveries: Vec<evo::Discovery> = Vec::new();
    if let Some((path, champion)) = &champion {
        champion.check_compat(&level)?;
        let genome = champion.to_genome()?;
        let eval = EvalConfig {
            action_repeat: champion.metadata.action_repeat,
            cut_stuck_frames: 0,
        };
        let result = run_episode(&level, &genome, &eval, seed);
        let report = evo::trace_shortcut(&level, &genome, &eval, seed);
        println!(
            "champion {}: {} in {} frames",
            path,
            result.outcome.as_str(),
            result.frames
        );
        println!("walked   {:?}", report.visited);
        println!("skipped  {:?}", report.skipped);
        println!("shortcut {:.0}% — {}", report.skip_percent(), report.note);
        discoveries = evo::detect_discoveries(&level, &genome, &result, &eval, 0, false);
        if discoveries.is_empty() {
            println!("found    no catalogued technique or shortcut");
        } else {
            for discovery in &discoveries {
                println!("found    {} — {}", discovery.name, discovery.line);
            }
        }
    } else {
        println!("champion none — pass --champion for the route diff; hunting anyway");
    }

    // The hunt is the half that is not about the flag: a trajectory that does more of one thing
    // than the intended route does. It is the report's "what else is here".
    let objective = match a.string("objective", "height").as_str() {
        "height" => sim::plan::HuntObjective::Height,
        "airtime" | "air" => sim::plan::HuntObjective::Airtime,
        "speed" => sim::plan::HuntObjective::Speed,
        "distance" | "ground-free-distance" => sim::plan::HuntObjective::GroundFreeDistance,
        other => {
            return Err(format!(
                "unknown hunt objective {other:?} (height, airtime, speed, distance)"
            ))
        }
    };
    let cfg = sim::plan::PlanConfig {
        depth: usage(a.usize_opt("depth", sim::plan::PlanConfig::default().depth as usize))? as u32,
        beam: usage(a.usize_opt("beam", sim::plan::PlanConfig::default().beam))?,
        ..sim::plan::PlanConfig::default()
    };
    let hunt = sim::plan::hunt(&level, seed, cfg, objective);
    println!("hunt     {}", hunt.note);
    if let Some(route) = &hunt.route {
        println!(
            "trick    {} blocks, {} frames, {} {:.3}",
            route.actions.len(),
            route.frames,
            hunt.objective.as_str(),
            hunt.best
        );
    }

    // The same journal `codex` writes, keyed on the level rather than the champion file: the
    // report is about the level, so a champion named `<level>-seeded.json` lands in the same
    // codex as the run that produced the shipped champion.
    if a.opt("codex-dir").is_some() || a.has_flag("dry-run") {
        let stem = |path: &str| {
            Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        };
        let stem = a
            .opt("level")
            .and_then(stem)
            .or_else(|| a.opt("champion").and_then(stem))
            .unwrap_or_else(|| level.name.clone());
        let dir = a.string("codex-dir", &format!("champions/{}", evo::CODEX_DIR));
        let path = evo::Codex::path_in(Path::new(&dir), &stem);
        if discoveries.is_empty() {
            println!(
                "codex    nothing this build can name; {} untouched",
                path.display()
            );
        } else if a.has_flag("dry-run") {
            let mut journal = evo::Codex::load(&level, &path)?;
            let new = journal.record_all(discoveries);
            println!("dry run: would record {new:?} in {}", path.display());
        } else {
            let mut journal = evo::Codex::load(&level, &path)?;
            let new = journal.record_all(discoveries);
            journal.save(&path)?;
            println!(
                "codex    {} (new: {})",
                path.display(),
                if new.is_empty() {
                    "none".to_string()
                } else {
                    new.join(", ")
                }
            );
        }
    }
    Ok(())
}

pub fn score(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(SCORE_OPTS))?;
    let champion_path = usage(a.required_path("champion"))?;
    let raw = fs::read_to_string(&champion_path)
        .map_err(|e| format!("cannot read {champion_path}: {e}"))?;
    let champion =
        ChampionFile::from_json_str(&raw).map_err(|e| format!("{champion_path}: {e}"))?;
    let genome = champion.to_genome()?;

    let levels: Vec<Level> = match a.opt("suite") {
        Some(list) => {
            let mut out = Vec::new();
            for path in parse_level_list(list)? {
                out.push(load_level(&path)?);
            }
            out
        }
        None if a.opt("level").is_some() => vec![load_level(&usage(a.required_path("level"))?)?],
        // With neither, the level is resolved by the champion's own content hash, which is the one
        // answer that cannot be wrong: a level that has been renamed or edited is simply not the
        // level the policy was fitted to.
        None => vec![find_level_by_hash(
            &a.string("levels-dir", "levels"),
            &champion.metadata.level_hash,
        )?
        .ok_or_else(|| {
            format!(
                "no level in {} matches this champion's hash; name one with --level or --suite",
                a.string("levels-dir", "levels")
            )
        })?],
    };

    let eval = EvalConfig {
        action_repeat: champion.metadata.action_repeat,
        cut_stuck_frames: 0,
    };
    let master = usage(a.u64_opt("seed", champion.metadata.training_config.master_seed))?;
    // The episode policy this command asks the trainer for, and the only thing it takes from the
    // default configuration: which episodes generation 0 of a default run would have used.
    let policy = GaConfig {
        master_seed: master,
        ..GaConfig::default()
    };

    println!(
        "champion  {} (gen {}, id {})",
        champion_path, champion.metadata.generation, champion.metadata.genome_id
    );
    println!(
        "          trained on {} at pop {} / seed {}",
        champion.metadata.level_name,
        champion.metadata.training_config.pop_size,
        champion.metadata.training_config.master_seed
    );

    // A provenance note rather than a refusal, and the reason the two commands differ.
    // `replay` is asked to verify the episode the file *recorded*, so a version-trio mismatch
    // is refused outright (`evo::champion::check_reproducibility`). This command asks the
    // opposite question — what the policy does on today's physics — so it runs, but the two
    // layers have to be told apart in what it prints: the summary line below is the file's
    // own numbers under its own build, while the trial result is today's. Silently mixing
    // them is exactly the kind of claim the version trio exists to stop.
    let stale = champion.metadata.sim_version != SIM_VERSION
        || champion.metadata.sensor_version != sim::sensors::VERSION
        || champion.metadata.fitness_version != evo::fitness::FITNESS_VERSION;
    if stale {
        println!(
            "          note: recorded under sim {} / sensors v{} / fitness v{}; this build is \
             sim {} / sensors v{} / fitness v{} — the trial result below is re-simulated on \
             today's physics, and only the recorded summary is the file's own",
            champion.metadata.sim_version,
            champion.metadata.sensor_version,
            champion.metadata.fitness_version,
            SIM_VERSION,
            sim::sensors::VERSION,
            evo::fitness::FITNESS_VERSION
        );
    }

    let mut solved = 0usize;
    let mut means: Vec<f32> = Vec::with_capacity(levels.len());
    for level in &levels {
        let trials = GaConfig::recommended_trials_for(level);
        let mut sum = 0.0f32;
        let mut wins = 0usize;
        let mut first: Option<evo::EpisodeResult> = None;
        for trial in 0..trials {
            let seed = evo::ga::episode_seed_for(&policy, 0, champion.metadata.genome_id, trial);
            let r = evo::run_episode(level, &genome, &eval, seed);
            sum += r.fitness;
            if r.won() {
                wins += 1;
            }
            if trial == 0 {
                first = Some(r);
            }
        }
        let mean = sum / trials as f32;
        means.push(mean);
        let first = first.expect("trials >= 1");
        // A level is solved only when *every* trial won, which is the same conservative reading the
        // suite trainer takes: on a walker level one phase is a coin flip.
        let won_all = wins == trials as usize;
        if won_all {
            solved += 1;
        }
        println!(
            "level     {} ({}x{}){}{}",
            level.name,
            level.width,
            level.height,
            if trials > 1 {
                format!("  {trials} trials")
            } else {
                String::new()
            },
            if won_all { "  WON" } else { "" }
        );
        println!(
            "          {:<9} fitness {:>9.2}  frames {:<5} best_x {:>5.1}  jumps {:>3}  checkpoints {:>2}  {} of {} trials won",
            first.outcome.as_str(),
            mean,
            first.frames,
            first.best_x,
            first.jumps,
            first.checkpoints,
            wins,
            trials
        );
    }

    if levels.len() > 1 {
        println!(
            "suite     {} of {} levels won  |  as a suite this policy scores {:.2} (its worst level)",
            solved,
            levels.len(),
            means.iter().copied().fold(f32::INFINITY, f32::min)
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// play
// ---------------------------------------------------------------------------

pub fn play(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(PLAY_OPTS))?;
    let level = load_level(&usage(a.required_path("level"))?)?;
    let seed = usage(a.u64_opt("seed", 1))?;

    let summary = render::play(&level, seed)
        .map_err(|e| format!("cannot start the interactive session: {e}"))?;

    println!();
    println!(
        "{} after {} frames ({}, best_x {:.1})",
        summary.outcome.as_str(),
        summary.frames,
        if summary.outcome == Outcome::Won {
            "nice"
        } else {
            "try again"
        },
        summary.best_x
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

pub fn render(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(RENDER_OPTS))?;
    let level = load_level(&usage(a.required_path("level"))?)?;
    let steps = usage(a.usize_opt("steps", 0))?;
    let stride = usage(a.usize_opt("stride", 20))?.max(1);
    let action = parse_action(&a.string("action", "run"))?;
    let seed = usage(a.u64_opt("seed", 1))?;

    println!("{}", render::legend());
    println!(
        "level {} ({}x{})  steps {}  stride {}  action {}\n",
        level.name,
        level.width,
        level.height,
        steps,
        stride,
        action.name()
    );

    let mut sim = sim::Sim::new(&level, seed);
    println!("{}\n{}", render::header(&sim), render::frame(&sim));
    let mut printed = 1usize;
    for step in 1..=steps {
        sim.step(action);
        if step % stride == 0 {
            println!("\n{}\n{}", render::header(&sim), render::frame(&sim));
            printed += 1;
        }
    }
    println!(
        "\nprinted {printed} frame(s); final outcome {}",
        sim.outcome().as_str()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// sensors
// ---------------------------------------------------------------------------

pub fn sensors(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(SENSORS_OPTS))?;

    println!(
        "observation v{}: {} inputs ({}x{} tile patch + {} features)",
        sim::sensors::VERSION,
        sim::sensors::N_INPUTS,
        sim::sensors::PATCH_RADIUS * 2 + 1,
        sim::sensors::PATCH_RADIUS * 2 + 1,
        sim::sensors::N_INPUTS - 81
    );
    for (i, name) in sim::sensors::input_names().iter().enumerate() {
        if i < 81 && i % 9 != 0 {
            continue;
        }
        if i >= 81 {
            println!("  [{i:>2}] {name}");
        } else {
            println!("  [{i:>2}] {name} ...");
        }
    }

    // The action table is stamped with the **sim** version, not the sensor one: the two
    // numbers move together only by coincidence (§8), and the action set lives in
    // `sim::physics` — a table printed under the wrong number is how a reader ends up
    // believing a champion was trained on a game it was not. This readout was printing the
    // sensor version here, which was wrong before it was misleading: the table gained its
    // two leftward sprinting arms in 0.6.0, when the pair happened to share a number.
    println!(
        "\nactions: {} discrete macro-actions (sim {}), argmax over network outputs",
        AgentAction::COUNT,
        sim::SIM_VERSION
    );
    for (i, (name, input, jump, sprint)) in sim::sensors::action_summary().iter().enumerate() {
        println!(
            "  [{i}] {:<14} input_x {:+}  jump {:<4} sprint {}",
            name,
            input,
            if *jump { "held" } else { "-" },
            if *sprint { "yes" } else { "-" }
        );
    }

    println!("\nphysics bible (sim {SIM_VERSION})");
    println!("  gravity             {}", PHYSICS.gravity);
    println!("  jump_v0             {}", PHYSICS.jump_v0);
    println!("  run_speed           {}", PHYSICS.run_speed);
    println!("  max_fall            {}", PHYSICS.max_fall);
    println!("  min_jump_hold       {} ticks", PHYSICS.min_jump_hold);
    println!("  coyote_ticks        {}", PHYSICS.coyote_ticks);
    println!(
        "  jump height         {:.3} tiles continuous / {:.3} discrete",
        PHYSICS.max_jump_height(),
        PHYSICS.discrete_max_jump_height()
    );
    println!(
        "  air time            {:.1} ticks",
        PHYSICS.air_time_ticks()
    );
    println!(
        "  jump distance       {:.3} tiles (max_jump_distance)",
        PHYSICS.max_jump_distance()
    );
    println!(
        "  sprint              {:.2} tiles/tick cap, {:.0} ticks of ramp, {:.3} tiles of run-up to earn it",
        PHYSICS.sprint_speed,
        PHYSICS.sprint_ramp_ticks(),
        PHYSICS.runway_to_speed(PHYSICS.sprint_speed)
    );
    println!(
        "  sprint jump         {:.3} tiles (sprint_jump_distance)",
        PHYSICS.sprint_jump_distance()
    );
    println!(
        "  tap jump height     {:.3} tiles",
        PHYSICS.min_jump_height()
    );
    println!(
        "  jump buffer         {} ticks (a press this early still fires on landing)",
        PHYSICS.jump_buffer_ticks
    );
    println!(
        "  wall slide          {:.2} tiles/tick fall while clinging",
        PHYSICS.wall_slide_fall
    );
    println!(
        "  wall jump           {:.3} tiles up, {:.2} tiles/tick away, {}-tick input lock",
        PHYSICS.wall_jump_height(),
        PHYSICS.wall_jump_push,
        PHYSICS.wall_jump_lock
    );
    println!(
        "\nfitness v{FITNESS_VERSION}: +{:.1}/pixel progress, {:.0} on death, +{:.0} on goal, {:.1} per jump, +{:.0} per checkpoint",
        evo::fitness::PROGRESS_PER_PIXEL,
        evo::fitness::DEATH_PENALTY,
        evo::fitness::GOAL_BONUS,
        evo::fitness::JUMP_PENALTY,
        evo::fitness::CHECKPOINT_BONUS
    );
    println!(
        "  no survival term: standing still earns nothing, so a frontier only moves by \
         crossing what is in front of it (fitness v3, the park trap)"
    );
    println!(
        "  checkpoints: paid per safe landing zone the body *stood on* past a hazard or a \
         hole, so a leap that ends in the air keeps its frontier and earns none of this"
    );
    println!(
        "  episode cap {} frames/tile, stuck after {} frames without new ground",
        sim::FRAMES_PER_TILE,
        sim::NO_PROGRESS_FRAMES
    );
    println!("  action repeat every {} physics ticks", evo::ACTION_REPEAT);
    println!("  per-frame wall time {:.1} ms", sim::DT * 1000.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn a(v: &[&str]) -> Args {
        Args::parse(&args(v)).ok().expect("parses")
    }

    /// `cargo test` runs with the CWD set to this package, not the workspace
    /// root, so every path a test passes on the command line is made absolute.
    /// (Changing the process CWD instead would race the other tests in the file.)
    fn wpath(rel: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
            .to_string_lossy()
            .to_string()
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evo-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn level_paths_load_from_the_shipped_pack() {
        let level = load_level(&wpath("levels/l1-flat-run.json")).expect("ships with the repo");
        assert_eq!(level.name, "L1-flat-run");
        assert_eq!(level.width, 60);
    }

    /// **The route plan's command is also its gate.** It writes an artifact a later phase can
    /// read back, and it rejects a flag it does not implement rather than silently ignoring it.
    #[test]
    fn routeplan_writes_a_plan_that_passes_its_own_boundary_checks() {
        let dir = tmpdir("routeplan");
        let out = dir.join("plans");
        routeplan(&a(&[
            "routeplan",
            "--level",
            &wpath("levels/practice/p01-gap-t0.json"),
            "--plan-depth",
            "48",
            "--out",
            out.to_str().expect("a utf-8 path"),
        ]))
        .expect("a certified practice level compiles");
        let text = fs::read_to_string(out.join("p01-gap-t0.routeplan.json"))
            .expect("the plan is written");
        let plan: sim::route_plan::RoutePlan =
            serde_json::from_str(&text).expect("the written plan parses back");
        assert!(!plan.legs.is_empty());
        assert_eq!(plan.legs[0].first_block, 0);
        assert_eq!(plan.legs.last().unwrap().end_block, plan.actions.len());
        assert_eq!(plan.legs.len() as u32, plan.covered);
        assert_eq!(plan.covered, plan.hops);
        assert_eq!(
            plan.legs.iter().map(|leg| leg.frames).sum::<u32>(),
            plan.frames
        );
        assert!(
            plan.legs
                .windows(2)
                .all(|pair| pair[0].end_block == pair[1].first_block),
            "the legs are contiguous"
        );

        // A flag the command does not implement is a usage error, not a silent no-op.
        assert!(routeplan(&a(&[
            "routeplan",
            "--level",
            &wpath("levels/practice/p01-gap-t0.json"),
            "--telemetry",
            "/tmp/ignored.jsonl",
        ]))
        .is_err());
    }

    #[test]
    fn every_action_name_round_trips() {
        for action in AgentAction::ALL {
            assert_eq!(parse_action(action.name()).ok(), Some(action));
        }
        assert!(parse_action("teleport").is_err());
    }

    /// Every level the pack ships is beatable as built, and its spawn is on solid ground.
    ///
    /// Scoped to [`pack_levels`] rather than to everything in the directory: a drawing a person
    /// saved there (`custom-*`) is their work in progress and may legitimately be unbeatable,
    /// and `reach --level <drawing>` is the command that tells them where it stops. The pack
    /// itself — every `l*` course and every `scratch-*` fixture a measurement is taken on — is
    /// still checked in full, which is the assertion this test exists for.
    #[test]
    fn reach_checks_all_shipped_levels_and_they_are_beatable() {
        let mut paths = pack_levels(&wpath("levels")).expect("levels dir exists");
        paths.sort();
        assert!(paths.len() >= 3, "expected the L1-L3 pack, found {paths:?}");
        // The pack is most of the directory, so a filter that quietly swallowed it would make
        // this test vacuous: every `l*` course has to be in the swept set.
        let shipped = list_levels(&wpath("levels"))
            .expect("levels dir exists")
            .into_iter()
            .filter(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|stem| stem.starts_with('l'))
            })
            .count();
        assert!(
            shipped > 0 && paths.len() >= shipped,
            "the pack sweep covers {} of the directory's {shipped} shipped courses",
            paths.len()
        );
        for path in paths {
            let level = Level::from_json_file(&path).expect("level parses");
            let report = reach::report(&level);
            assert!(
                report.goal_reached,
                "{} is not beatable as built: {:?}",
                path.display(),
                report.diagnosis
            );
            assert!(
                report.start.is_some(),
                "{} has no spawn platform",
                path.display()
            );
        }
    }

    /// A drawing a person saved beside the pack is still *loadable* and still *diagnosable*,
    /// which is what makes the pack sweep's scope a scoping rather than a blind spot.
    ///
    /// The assertion is about the two readings **agreeing**, not about the drawing's verdict:
    /// a level the oracle calls unbeatable must come back as a reported failure from the
    /// command, and a level it calls beatable must come back clean. Pinning the current verdict
    /// would turn this into a statement about one file, and the file belongs to a person who is
    /// allowed to change it.
    #[test]
    fn a_users_own_drawing_is_reported_on_request_whatever_the_verdict() {
        let drawn: Vec<PathBuf> = list_levels(&wpath("levels"))
            .expect("levels dir exists")
            .into_iter()
            .filter(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|stem| stem.starts_with("custom-"))
            })
            .collect();
        // The repo ships one: the tower a person drew over `l1-flat-run`'s canvas. If it ever
        // goes away this test stops proving anything, and it says so rather than passing quietly.
        assert!(
            !drawn.is_empty(),
            "no `custom-*` drawing is checked in, so the exclusion in `pack_levels` is untested"
        );
        for path in drawn {
            let level = Level::from_json_file(&path).expect("a drawing still parses");
            let report = reach::report(&level);
            assert!(
                report.start.is_some(),
                "{} has no spawn platform",
                path.display()
            );
            let command = reach(&a(&["reach", "--level", &path.to_string_lossy()]));
            match (&report.diagnosis, command) {
                // Beatable: nothing to diagnose and nothing to report.
                (None, Ok(())) => assert!(
                    report.goal_reached,
                    "{} had no diagnosis but is not reported as beaten",
                    path.display()
                ),
                // Unbeatable: the command fails, and it fails *because of this*.
                (Some(msg), Err(err)) => {
                    assert!(!msg.is_empty(), "{}", path.display());
                    assert!(
                        err.contains("not beatable"),
                        "{} is reported for the wrong reason: {err}",
                        path.display()
                    );
                }
                (None, Err(err)) => panic!(
                    "{}: the oracle found nothing wrong and the command failed anyway: {err}",
                    path.display()
                ),
                (Some(msg), Ok(())) => panic!(
                    "{}: the oracle says {:?} and the command passed",
                    path.display(),
                    msg
                ),
            }
        }
    }

    #[test]
    fn the_reach_subcommand_reports_success_for_the_pack() {
        let dir = wpath("levels");
        assert!(reach(&a(&["reach", "--all", "--levels-dir", &dir])).is_ok());
        assert!(reach(&a(&[
            "reach",
            "--level",
            &wpath("levels/l2-first-gap.json"),
            "--verbose"
        ]))
        .is_ok());
    }

    /// Train a tiny run and hand back `(level, champion path)`.
    fn tiny_run(tag: &str, generations: &str, pop: &str) -> (Level, PathBuf, PathBuf) {
        let dir = tmpdir(tag);
        let out = dir.to_string_lossy().to_string();
        let level_path = wpath("levels/l1-flat-run.json");
        train(&a(&[
            "train",
            "--level",
            &level_path,
            "--generations",
            generations,
            "--pop",
            pop,
            "--seed",
            "5",
            "--out",
            &out,
            "--quiet",
        ]))
        .expect("training runs");
        let champion = dir.join("l1-flat-run.json");
        (load_level(&level_path).expect("level"), champion, dir)
    }

    /// The stricter stop is a *refinement* of `--stop-on-win`, not a second way to stop, and it
    /// needs the reading it stops on. Both are usage errors rather than a flag that quietly does
    /// nothing — the failure mode this file refuses everywhere else.
    #[test]
    fn the_validated_stop_needs_the_win_stop_and_a_validation_set() {
        let level = wpath("levels/l1-flat-run.json");
        let on_its_own = train(&a(&[
            "train",
            "--level",
            &level,
            "--stop-on-validated",
            "--generations",
            "1",
        ]))
        .expect_err("--stop-on-validated without --stop-on-win is a usage error");
        assert!(on_its_own.contains("--stop-on-win"), "{on_its_own}");

        let without_set = train(&a(&[
            "train",
            "--level",
            &level,
            "--stop-on-win",
            "--stop-on-validated",
            "--no-validation",
            "--generations",
            "1",
        ]))
        .expect_err("--stop-on-validated with --no-validation is a usage error");
        assert!(without_set.contains("--no-validation"), "{without_set}");

        // A suite already has the conservative stop, and the validation set is one level's: the
        // pair is refused rather than accepted and ignored.
        let second = wpath("levels/l2-first-gap.json");
        let suite = format!("{level},{second}");
        let with_suite = train(&a(&[
            "train",
            "--suite",
            &suite,
            "--stop-on-win",
            "--stop-on-validated",
            "--generations",
            "1",
        ]))
        .expect_err("--stop-on-validated on a suite is a usage error");
        assert!(with_suite.contains("suite"), "{with_suite}");
    }

    /// **Item 9 end to end.** A campaign asked for `--fuse` breeds crosses of its own legs and
    /// seeds them into generation zero before the whole-level phase, on a level whose planned route
    /// really splits into more than one leg. And a campaign with one leg refuses: a cross needs two
    /// parents, so silently seeding a copy of one specialist would be the arm failing quietly.
    #[test]
    fn a_campaign_fuses_its_leg_specialists_and_refuses_a_single_leg() {
        let dir = tmpdir("campaign-fuse");
        let out = dir.to_string_lossy().to_string();
        let checkpoints = dir.join("ck").to_string_lossy().to_string();
        campaign(&a(&[
            "campaign",
            "--level",
            &wpath("levels/l2-first-gap.json"),
            "--segment-generations",
            "1",
            "--generations",
            "1",
            "--pop",
            "16",
            "--fuse",
            "4",
            "--seed",
            "3",
            "--out",
            &out,
            "--checkpoints",
            &checkpoints,
        ]))
        .expect("a fused campaign runs");
        assert!(dir.join("l2-first-gap-campaign.json").exists());

        let err = campaign(&a(&[
            "campaign",
            "--level",
            &wpath("levels/l1-flat-run.json"),
            "--segment-generations",
            "1",
            "--generations",
            "1",
            "--pop",
            "16",
            "--fuse",
            "4",
            "--seed",
            "3",
            "--out",
            &out,
            "--checkpoints",
            &checkpoints,
        ]))
        .expect_err("one leg cannot be crossed with itself");
        assert!(err.contains("two leg specialists"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// **Item 6 end to end.** The generalist rotates one generation per pack level, its cursor is
    /// the checkpoint's own generation (so a second invocation continues the rotation instead of
    /// restarting it), and `--refresh` accepts the champions a pack run wrote into the back slots.
    #[test]
    fn the_generalist_rotates_the_pack_and_refreshes_from_champions() {
        let dir = tmpdir("generalist");
        let out = dir.to_string_lossy().to_string();
        let champs = dir.join("champs");
        let champs_path = champs.to_string_lossy().to_string();
        let list = format!(
            "{},{}",
            wpath("levels/l1-flat-run.json"),
            wpath("levels/l2-first-gap.json")
        );

        generalist(&a(&[
            "generalist",
            "--levels",
            &list,
            "--generations",
            "4",
            "--pop",
            "16",
            "--seed",
            "2",
            "--out",
            &out,
            "--quiet",
        ]))
        .expect("a generalist trains");
        let first = PopulationFile::load(dir.join("generalist.json")).expect("checkpoint loads");
        assert_eq!(first.generation, 4);
        assert_eq!(first.networks.len(), 16);

        // A second call continues from generation 4, and the rotation continues with it: the
        // level of generation 4 is the one a four-generation cycle from index 0 has not just done.
        generalist(&a(&[
            "generalist",
            "--levels",
            &list,
            "--generations",
            "2",
            "--pop",
            "16",
            "--seed",
            "2",
            "--out",
            &out,
            "--quiet",
        ]))
        .expect("a generalist resumes");
        let second = PopulationFile::load(dir.join("generalist.json")).expect("checkpoint loads");
        assert_eq!(second.generation, 6, "the cursor is the checkpoint's own count");

        // Pack champions come back in through --refresh.
        train(&a(&[
            "train",
            "--level",
            &wpath("levels/l1-flat-run.json"),
            "--generations",
            "1",
            "--pop",
            "16",
            "--seed",
            "5",
            "--out",
            &champs_path,
            "--quiet",
        ]))
        .expect("a champion is written for the refresh");
        assert!(champs.join("l1-flat-run.json").exists());
        generalist(&a(&[
            "generalist",
            "--levels",
            &list,
            "--refresh",
            &champs_path,
            "--pop",
            "16",
            "--out",
            &out,
            "--quiet",
        ]))
        .expect("a refresh runs without training a generation");
        let refreshed = PopulationFile::load(dir.join("generalist.json")).expect("checkpoint loads");
        assert_eq!(refreshed.generation, 6, "a refresh trains nothing");
        let _ = fs::remove_dir_all(&dir);
    }

    /// **Item 5's two halves are one switch.** A refund with nothing to spend and a margin beside
    /// no cut are both usage errors rather than accepted-and-ignored flags, which is the rule this
    /// file applies to every experimental knob.
    #[test]
    fn the_cut_refund_needs_the_cut_it_spends() {
        let err = train_config(&a(&["train", "--cut-refund"])).expect_err("no cut to refund");
        assert!(err.contains("--cut-stuck"), "{err}");
        let cfg = train_config(&a(&["train", "--cut-stuck", "60", "--cut-refund"]))
            .expect("a cut with a refund is a run");
        assert_eq!(cfg.cut_stuck_frames, 60);
        assert!(cfg.cut_refund);
        // The default band is exact: cut only what is already below the cut.
        assert_eq!(cfg.cut_margin, 0.0);
        // And a positive margin is accepted, because some levels need the slack.
        let widened = train_config(&a(&[
            "train",
            "--cut-stuck",
            "60",
            "--cut-margin",
            "12000",
        ]))
        .expect("a widened band is a run");
        assert_eq!(widened.cut_margin, 12_000.0);
    }

    #[test]
    fn training_a_few_generations_produces_a_loadable_champion() {
        let dir = tmpdir("train-cli");
        let out = dir.to_string_lossy().to_string();
        let csv = dir.join("history.csv").to_string_lossy().to_string();
        let level_path = wpath("levels/l1-flat-run.json");
        train(&a(&[
            "train",
            "--level",
            &level_path,
            "--generations",
            "3",
            "--pop",
            "16",
            "--seed",
            "5",
            "--out",
            &out,
            "--csv",
            &csv,
            "--quiet",
        ]))
        .expect("training runs");

        let champion_path = dir.join("l1-flat-run.json");
        assert!(champion_path.exists(), "champion file must be written");
        let level = load_level(&level_path).expect("level");
        let champ = ChampionFile::load_for_level(&champion_path, &level).expect("champion loads");

        assert!(
            csv_has_rows(&csv, 3),
            "csv needs a header plus one row per generation"
        );
        assert!(champ.metadata.frames > 0);
        assert_eq!(champ.metadata.level_hash, level.content_hash_string());
        assert!(champ.to_genome().is_ok());
        assert_eq!(champ.metadata.sensor_version, sim::sensors::VERSION);

        let _ = fs::remove_dir_all(&dir);
    }

    /// A suite run scores every level it names and writes a champion that belongs to none of the
    /// single-level names — the naming is a guard, not cosmetics: `champions/` is the one directory
    /// this tool writes to without being asked, and a suite selected on three levels must not land
    /// on top of the champion trained on one of them (REVIEW.md §2.8's failure mode).
    #[test]
    fn a_suite_run_writes_its_own_champion_and_records_it_on_its_first_level() {
        let dir = tmpdir("train-cli-suite");
        let out = dir.to_string_lossy().to_string();
        let first = wpath("levels/l2-first-gap.json");
        let second = wpath("levels/l1-flat-run.json");
        train(&a(&[
            "train",
            "--suite",
            &format!("{first},{second}"),
            "--generations",
            "2",
            "--pop",
            "16",
            "--seed",
            "4",
            "--out",
            &out,
            "--quiet",
        ]))
        .expect("the suite runs");

        let suite_champion = dir.join("l2-first-gap-suite.json");
        assert!(suite_champion.exists(), "the suite writes its own champion");
        assert!(
            !dir.join("l2-first-gap.json").exists(),
            "and never the single-level name it would otherwise share"
        );

        let levels: Vec<Level> = vec![
            load_level(&first).expect("l2"),
            load_level(&second).expect("l1"),
        ];
        let champ = ChampionFile::load_for_level(&suite_champion, &levels[0])
            .expect("a suite champion is recorded on the suite's first level");
        assert_eq!(champ.metadata.level_hash, levels[0].content_hash_string());
        assert!(
            ChampionFile::load_for_level(&suite_champion, &levels[1]).is_err(),
            "and it is not a champion of the other level, which is what the hash is for"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// `--suite` is a list with an order, and the two ways a list can be nonsense are refused
    /// where they are typed rather than discovered as a slower run.
    #[test]
    fn a_suite_list_refuses_an_empty_entry_and_a_duplicate() {
        assert_eq!(
            parse_level_list("a.json,b.json").expect("a list parses"),
            vec!["a.json".to_string(), "b.json".to_string()]
        );
        assert_eq!(
            parse_level_list(" a.json , b.json ").expect("whitespace is trimmed"),
            vec!["a.json".to_string(), "b.json".to_string()]
        );
        assert!(
            parse_level_list("a.json,").is_err(),
            "a trailing comma is an empty entry, not a level"
        );
        assert!(parse_level_list("").is_err(), "an empty list names nothing");
        let dup = parse_level_list("a.json,a.json").expect_err("a duplicate is refused");
        assert!(dup.contains("twice"), "{dup}");

        // And naming both `--level` and `--suite` is a usage error rather than a priority rule.
        let both = a(&["train", "--level", "a.json", "--suite", "a.json,b.json"]);
        let err = train(&both).expect_err("two answers to what is being scored");
        assert!(
            err.contains("--suite and --level are alternatives"),
            "{err}"
        );
    }

    /// The guard for the whole class of bug, not only the instance that was found: a flag the
    /// user did not give must leave the trainer's own default in place. The defect this catches
    /// was real — `--reheat-factor` carried a second copy of its default, the trainer's moved to
    /// 3.0, and the command line went on asking for 1.0 for every run that did not name one.
    #[test]
    fn the_command_lines_defaults_are_the_trainers_defaults() {
        let level_path = wpath("levels/l1-flat-run.json");
        let level = load_level(&level_path).expect("level loads");
        let args = a(&["train", "--level", &level_path]);
        let cli = train_config(&args).expect("a bare train invocation parses");
        let default = GaConfig::default();
        assert_eq!(cli.pop_size, default.pop_size);
        assert_eq!(cli.elite, default.elite);
        assert_eq!(cli.tournament, default.tournament);
        assert_eq!(cli.mutation_rate, default.mutation_rate);
        assert_eq!(cli.mutation_sigma, default.mutation_sigma);
        assert_eq!(cli.crossover_rate, default.crossover_rate);
        assert_eq!(cli.fine_children, default.fine_children);
        assert_eq!(cli.adaptive_sigma, default.adaptive_sigma);
        assert_eq!(cli.stagnation_generations, default.stagnation_generations);
        assert_eq!(cli.immigrants, default.immigrants);
        assert_eq!(cli.archive_children, default.archive_children);
        assert_eq!(cli.novelty_weight, default.novelty_weight);
        assert_eq!(cli.rank_pessimism, default.rank_pessimism);
        assert_eq!(cli.reheat_factor, default.reheat_factor);
        assert_eq!(cli.action_repeat, default.action_repeat);
        assert_eq!(cli.shape.feedback, default.shape.feedback);
        assert_eq!(cli.shape.hidden, default.shape.hidden);
        assert_eq!(cli.trials, default.trials);
        assert_eq!(cli.master_seed, default.master_seed);

        // And the flag reaches the trainer's shape when it *is* given, without disturbing the
        // rest of the recipe.
        let asked = a(&["train", "--level", &level_path, "--feedback", "8"]);
        let asked = train_config(&asked).expect("parses");
        assert_eq!(asked.shape.feedback, 8);
        // The rarity bonus is a fitness-units number rather than a ratio, so the flag sets it
        // directly and the trainer bounds it (see `NOVELTY_WEIGHT_MAX`).
        let rarity = a(&["train", "--level", &level_path, "--novelty", "16"]);
        let rarity = train_config(&rarity).expect("parses");
        assert_eq!(rarity.novelty_weight, 16.0);
        assert_eq!(
            rarity.sanitized(None).novelty_weight,
            16.0,
            "a measured weight must survive the trainer's own clamping"
        );
        assert_eq!(asked.shape.inputs, default.shape.inputs);
        assert_eq!(asked.shape.hidden, default.shape.hidden);

        // A walker level doubles (triples) the episodes selection reads, and it does so through
        // the *trainer's* rule rather than through a number written down here — which is what the
        // old hard-coded `3` in this function was, and why `recommended_trials_for` sat unused.
        let walker_level = load_level(&wpath("levels/l3-gauntlet.json")).expect("l3 loads");
        let walker_cli = train_config(&args).expect("parses");
        assert_eq!(
            walker_cli.trials, default.trials,
            "the flag is a pass-through now"
        );
        assert_eq!(
            walker_cli.effective_trials(&walker_level),
            3,
            "walker levels are judged on three patrol phases by default"
        );
        assert_eq!(
            walker_cli.effective_trials(&level),
            1,
            "and a flat level on one"
        );
        let opted_out = a(&["train", "--level", &level_path, "--no-auto-trials"]);
        let opted_out = train_config(&opted_out).expect("parses");
        assert_eq!(
            opted_out.effective_trials(&walker_level),
            1,
            "--no-auto-trials is how the numbers taken before the rule existed are reproduced"
        );
    }

    /// The two flags that change what a run can *be* rather than how it steps: the capacity of the
    /// net, and the question a genome is ranked by.
    ///
    /// Both are checked at this level rather than only in the trainer's own tests because the way
    /// they fail is a command line that silently means something else: `--hidden` is a comma list
    /// parsed here, and `--rank-pessimism` is an `f32` that has to survive the trainer's clamps.
    #[test]
    fn the_capacity_and_ranking_flags_reach_the_trainer() {
        let level_path = wpath("levels/l1-flat-run.json");
        let asked = train_config(&a(&[
            "train",
            "--level",
            &level_path,
            "--hidden",
            "32,16",
            "--rank-pessimism",
            "0.5",
        ]))
        .expect("parses");
        assert_eq!(
            asked.shape.hidden,
            vec![32, 16],
            "a list is a topology, not a number"
        );
        assert_eq!(
            asked.shape.feedback,
            GaConfig::default().shape.feedback,
            "widening the net must not disturb the memory width it was not asked about"
        );
        assert_eq!(asked.rank_pessimism, 0.5);
        let run = asked.sanitized(None);
        assert_eq!(
            run.shape.hidden,
            vec![32, 16],
            "a width an operator asked for must survive the trainer's own clamping"
        );
        assert_eq!(run.rank_pessimism, 0.5, "and so must the pessimism");
        // A zero-width layer is refused here rather than repaired, because it is not a narrower
        // network: the layer above it would fan in to a row of zeros and the one below to nothing.
        let broken = train_config(&a(&["train", "--level", &level_path, "--hidden", "0"]))
            .expect_err("a zero-width layer is a usage error");
        assert!(broken.contains("--hidden 0"), "{broken}");
        let word = train_config(&a(&["train", "--level", &level_path, "--hidden", "wide"]))
            .expect_err("a word is not a width");
        assert!(word.contains("layer widths"), "{word}");
    }

    /// A run seeded from a champion starts from it, says which level it came from when that is not
    /// this one, and does not overwrite the file it started from.
    ///
    /// The last part is the one worth a test. A fine-tune names a champion in the same directory the
    /// run writes to, so the default champion path *is* the seed's own path: without the guard the
    /// run would replace the evidence it began from with the result it produced, one directory, no
    /// warning, and the "before" of a before-and-after gone (`REVIEW.md` §2.8).
    #[test]
    fn a_run_seeded_from_a_champion_starts_there_and_writes_beside_its_seed() {
        let (level, champion, dir) = tiny_run("seeded-run", "3", "16");
        let out = dir.to_string_lossy().to_string();
        let seed_arg = champion.to_string_lossy().to_string();
        let level_path = wpath("levels/l1-flat-run.json");
        let before =
            ChampionFile::load_for_level(&champion, &level).expect("the seed is a champion");

        train(&a(&[
            "train",
            "--level",
            &level_path,
            "--generations",
            "2",
            "--pop",
            "16",
            "--seed",
            "9",
            "--out",
            &out,
            "--from-champion",
            &seed_arg,
            "--quiet",
        ]))
        .expect("a seeded run trains");

        let after = ChampionFile::load_for_level(&champion, &level).expect("the seed still loads");
        assert_eq!(
            after.metadata.genome_id, before.metadata.genome_id,
            "the file the run started from is the file it started from"
        );
        assert_eq!(after.fitness, before.fitness);
        let seeded = dir.join("l1-flat-run-seeded.json");
        assert!(
            seeded.exists(),
            "a seeded run writes beside its seed rather than over it"
        );
        let written = ChampionFile::load_for_level(&seeded, &level)
            .expect("the seeded run's champion is a champion of this level");
        assert!(
            written.fitness.is_finite() && written.metadata.genome_id > 0,
            "a run that started from a genome still has to produce one"
        );

        // And the alternatives are alternatives: two answers to "where does this start" is a usage
        // error, checked *before* either file is read, which is why the checkpoint's path need not
        // exist for this assertion to mean something.
        let both = train(&a(&[
            "train",
            "--level",
            &level_path,
            "--resume",
            "nowhere.population.json",
            "--from-champion",
            &seed_arg,
            "--out",
            &out,
            "--quiet",
        ]))
        .expect_err("one run cannot start from both a population and a genome");
        assert!(both.starts_with("usage:"), "{both}");
        assert!(both.contains("alternatives"), "{both}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The baseline arm's flags default from the trainer too, for the same reason, and — the part
    /// that actually matters — the *episode budget* it reads is the GA's. An `es` invocation that
    /// quietly cost a different number of episodes per generation would make the comparison in
    /// `REVIEW.md` §4 a comparison of wall-clock settings rather than of searches.
    #[test]
    fn the_es_command_lines_defaults_are_the_trainers_defaults_and_the_gas_budget() {
        let level_path = wpath("levels/l1-flat-run.json");
        let args = a(&["es", "--level", &level_path]);
        let cli = es_config(&args).expect("a bare es invocation parses");
        let default = EsConfig::default();
        assert_eq!(cli.pop_size, default.pop_size);
        assert_eq!(cli.sigma, default.sigma);
        assert_eq!(cli.learning_rate, default.learning_rate);
        assert_eq!(cli.master_seed, default.master_seed);
        assert_eq!(cli.action_repeat, default.action_repeat);
        assert_eq!(cli.trials, default.trials);
        assert_eq!(cli.shape.feedback, default.shape.feedback);

        let ga = GaConfig::default();
        assert_eq!(cli.pop_size, ga.pop_size, "one batch is one GA generation");
        assert_eq!(cli.action_repeat, ga.action_repeat);
        assert_eq!(cli.shape.inputs, ga.shape.inputs);

        // And the flags do reach it, without disturbing the rest of the arm.
        let asked = a(&[
            "es",
            "--level",
            &level_path,
            "--sigma",
            "0.10",
            "--learning-rate",
            "0.02",
            "--feedback",
            "8",
            "--pop",
            "64",
        ]);
        let asked = es_config(&asked).expect("parses");
        assert_eq!(asked.sigma, 0.10);
        assert_eq!(asked.learning_rate, 0.02);
        assert_eq!(asked.shape.feedback, 8);
        assert_eq!(asked.batch(), 64);
        assert_eq!(asked.shape.inputs, default.shape.inputs);

        // A typo is a usage error rather than a second run of the GA with different flags.
        let bad = a(&["es", "--level", &level_path, "--mut-sigma", "0.4"]);
        assert!(bad.reject_unknown(ES_OPTS).is_err());
    }

    /// The arm end to end through the real command on a real level, which is the thing a unit
    /// test of the arithmetic cannot show: that the subcommand walks the level, updates the mean
    /// and survives its own summary — including the `--stop-on-win` and `--max-seconds` exits,
    /// each of which ends the loop somewhere other than the top.
    #[test]
    fn the_es_subcommand_runs_on_a_shipped_level() {
        let level_path = wpath("levels/l1-flat-run.json");
        for extra in [
            Vec::new(),
            vec!["--stop-on-win"],
            vec!["--max-seconds", "0.0001"],
        ] {
            let mut argv = vec![
                "es",
                "--level",
                level_path.as_str(),
                "--generations",
                "3",
                "--pop",
                "8",
                "--quiet",
            ];
            argv.extend(extra.iter().copied());
            es(&a(&argv)).unwrap_or_else(|e| panic!("es {extra:?} failed: {e}"));
        }

        // And the invocation is checked like every other subcommand: a missing level and a flag
        // that belongs to `train` are both usage errors rather than a quietly different run.
        assert!(es(&a(&["es"])).is_err());
        assert!(es(&a(&["es", "--level", &level_path, "--mut-sigma", "0.4"])).is_err());
    }

    fn csv_has_rows(path: &str, rows: usize) -> bool {
        match fs::read_to_string(path) {
            Ok(text) => {
                let lines: Vec<&str> = text.lines().collect();
                lines.len() == rows + 1 && lines[0].starts_with("generation,")
            }
            Err(_) => false,
        }
    }

    #[test]
    fn the_replay_subcommand_verifies_a_saved_champion() {
        let (_, champion_path, dir) = tiny_run("replay", "2", "8");
        let champion_path = champion_path.to_string_lossy().to_string();

        // No --level: the level is recovered by matching the champion's own level
        // hash against the level pack. `--levels-dir` is absolute because the test
        // process's CWD is this package, not the workspace root.
        let levels_dir = wpath("levels");
        replay(&a(&[
            "replay",
            "--champion",
            &champion_path,
            "--levels-dir",
            &levels_dir,
        ]))
        .expect("replay verifies");

        // With no matching level anywhere, it must say so rather than guess.
        let empty = tmpdir("no-levels");
        fs::create_dir_all(&empty).expect("mkdir");
        assert!(replay(&a(&[
            "replay",
            "--champion",
            &champion_path,
            "--levels-dir",
            &empty.to_string_lossy()
        ]))
        .is_err());
        let _ = fs::remove_dir_all(&empty);
        // An explicit level that does not match must be refused, not silently used.
        assert!(replay(&a(&[
            "replay",
            "--champion",
            &champion_path,
            "--level",
            &wpath("levels/l3-gauntlet.json")
        ]))
        .is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_rejects_a_tampered_champion() {
        let (_, champion_path, dir) = tiny_run("tamper", "1", "8");
        let text = fs::read_to_string(&champion_path).expect("read");
        // Tampered to a version this build cannot be, read from the constant rather than
        // written as a literal: a hard-coded "the wrong version" pair goes stale the moment
        // the version moves, and did — when fitness v2 landed, this test's literal 2 became
        // the *correct* version and the tampered file replayed happily.
        let current = format!("\"fitness_version\": {FITNESS_VERSION}");
        assert!(
            text.contains(&current),
            "the fixture records its fitness version: {current}"
        );
        let tampered = text.replace(
            &current,
            &format!("\"fitness_version\": {}", FITNESS_VERSION + 1),
        );
        assert_ne!(tampered, text, "fixture must actually change the file");
        let bad = dir.join("tampered.json");
        fs::write(&bad, tampered).expect("write");
        assert!(
            replay(&a(&["replay", "--champion", &bad.to_string_lossy()])).is_err(),
            "a champion from another fitness version must be refused"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_options_are_usage_errors_not_silent_no_ops() {
        let level = wpath("levels/l1-flat-run.json");
        assert!(train(&a(&["train", "--level", &level, "--poop", "3"])).is_err());
        assert!(reach(&a(&["reach", "--level", &level, "--nope"])).is_err());
        assert!(render(&a(&["render", "--level", &level, "--wat"])).is_err());
    }

    #[test]
    fn render_prints_frames_without_a_terminal() {
        render(&a(&[
            "render",
            "--level",
            &wpath("levels/l3-gauntlet.json"),
            "--steps",
            "40",
            "--stride",
            "20",
        ]))
        .expect("render runs offline");
        // A bogus action name is a runtime error, not a silent fallback.
        assert!(render(&a(&[
            "render",
            "--level",
            &wpath("levels/l1-flat-run.json"),
            "--action",
            "fly"
        ]))
        .is_err());
    }

    #[test]
    fn sensors_prints_the_layout() {
        sensors(&a(&["sensors"])).expect("sensors runs offline");
    }

    #[test]
    fn the_oracle_line_is_honest_about_an_unbeatable_level() {
        let broken = sim::Level::from_json_str(UNREACHABLE).expect("parses: it does have a goal");
        let line = reachability_line(&broken);
        assert!(line.starts_with("NOT beatable"), "{line}");
        assert!(reach(&a(&["reach", "--level", "/nonexistent/level.json"])).is_err());
    }

    /// A seven-tile pit between the start ledge (columns 0–5) and the ledge holding
    /// the goal (columns 13–15): wider than the 6.0-tile **sprint** jump distance, so
    /// no canonical take-off clears it and the goal is unreachable on foot.
    ///
    /// The width follows the abilities, and this fixture has had to grow twice. It was
    /// four tiles, chosen when the plan's 4.0-tile full-speed jump was the whole reach of
    /// the game — and a coarse take-off sweep called that unbeatable for the wrong reason
    /// (the pit was crossable; the goal was simply floating). Six tiles fixed that, and
    /// then sprint arrived and made six crossable too: a sprinting take-off from the last
    /// column covers 6.0 tiles and lands on the far ledge. Seven is one past the sprint,
    /// which is what "unbeatable on foot" means in a game with two gears.
    const UNREACHABLE: &str = "{
      \"format\": \"evo-platformer-level\",
      \"level_format_version\": 1,
      \"name\": \"unreachable-fixture\",
      \"width\": 20, \"height\": 8,
      \"spawn\": {\"x\": 1, \"y\": 5},
      \"tiles\": [
        \"....................\",
        \"....................\",
        \"....................\",
        \"....................\",
        \"....................\",
        \"....................\",
        \"######.......###....\",
        \"######.......###....\"
      ],
      \"entities\": [{\"type\": \"goal\", \"x\": 14, \"y\": 5}]
    }";
}
