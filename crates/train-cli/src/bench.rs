//! `bench` — a seed sweep as one command, and the paired test that reads two of them.
//!
//! ## Why this exists
//!
//! Every number this repository quotes about learning is a **sweep**: twelve seeds at
//! pop 200 for a fixed cap, `--stop-on-win`, and the measurement is the generation the
//! population first beat the level (§9's "the population is itself the difficulty
//! oracle"). `FINDINGS.md` prints the `for seed in $(seq 1 12)` loop that produced a row
//! rather than the row's machinery, and that is the honest thing to do with a hand-run
//! sweep — but it means a claim about the *trainer* costs an afternoon of copying
//! commands, counting solved seeds by eye, and comparing two piles of output with no
//! arithmetic behind the comparison. That is the gap this command closes, and it closes
//! it in the one way that keeps the claims falsifiable:
//!
//! * a **protocol** is a named, frozen (level, pop, generations, seeds) triple — the
//!   sweep the repository already runs, addressed by name instead of retyped;
//! * an **arm** is one run of that protocol with whatever flags were passed, recorded to
//!   a file together with every knob that was in force, so "which configuration produced
//!   this table" is a fact in the file rather than a note in someone's shell history;
//! * `--compare` reads two arms back and prints the **paired** result — per-seed winners,
//!   a two-sided exact sign test over the seeds where the two disagree, and the seeds
//!   that flipped.
//!
//! ## The two rules this command enforces on itself
//!
//! **No detours and no wall clock.** A bench arm drives [`evo::Trainer::step`] directly,
//! so the stall chain (§12 M5, findings 50–51) cannot fire: a seed's first win is the
//! first win of one uninterrupted run on one level, which is what every recorded row in
//! `FINDINGS.md` means. `--curriculum`, `--curriculum-generations`, `--csv` and
//! `--max-seconds` are therefore refused rather than accepted-and-ignored — a flag that
//! silently does nothing is the most annoying class of CLI bug there is, and a
//! wall-clock cap would make the arm machine-dependent, which is precisely the thing a
//! reproducible protocol is for.
//!
//! **`--stop-on-win` is built in.** The first-win generation is the measurement, so the
//! flag asks for the rule the command already follows; it is accepted (and the header
//! says so), unlike the four above, which would change *what* is measured.
//!
//! **`--stop-on-validated` is accepted as a stricter protocol**, and recorded in the arm
//! (`BenchArm::stop_on_validated`): the seed stops at the first generation whose *record* won every
//! fixed validation episode, not at the first generation whose best genome won its three drawn
//! trials. On `l9-wait-and-climb` that moved four seeds from a champion winning 22 of 48 fresh
//! episodes to 48 of 48, at a median cost of 20 generations — the measurement `FINDINGS.md`
//! records under "Solved, then validated". Two arms are not comparable across this field.
//!
//! ## What the sign test does and does not say
//!
//! For each seed the winner is the arm that solved where the other did not, or — when
//! both solved — the arm with the lower first-win generation. Ties carry no information
//! and are dropped, and the p-value is the exact two-sided binomial over the remaining
//! discordant pairs (`sign_test_p`). It reads the *direction* of each disagreement, not
//! the size of it: an arm that wins six seeds by one generation each and loses none is
//! p = 0.03 even though its whole effect is six generations, and an arm that wins once by
//! two hundred generations is p = 1.0. That is the correct reading for a tool that
//! decides whether a knob is doing something at all; it is not a claim about effect size,
//! and the median and mean are printed beside it for that.
//!
//! ## Reproducing a row
//!
//! ```text
//! train-cli bench --protocol chain --arm default      # 12 seeds, recorded to bench/
//! train-cli bench --protocol tower --arm default
//! train-cli bench --compare bench/chain-default.json,bench/chain-memory.json
//! ```
//!
//! `--level`, `--pop`, `--generations` and `--seed` override what the protocol supplies
//! (`--seed` is the *first* seed of the sweep, so `--seed 5 --seeds 2` runs seeds 5 and
//! 6), which is what makes the tool usable on a level the table does not know; the file
//! records the level it actually ran, so an overridden arm cannot be quoted as though it
//! were the protocol's own.
//!
//! ## Two arms that do not start from noise
//!
//! `--from-champion` and `--resume` are accepted here because they are accepted by
//! `train`, and through the same constructor ([`crate::commands::seeded_trainer`]) rather
//! than through a second copy of the logic: a warm arm is a real question — *does a
//! population seeded from a known point reach its first win sooner than one drawn from
//! noise?* — and it is the question a curriculum ladder is built on. Both are per-seed: a
//! seeded population's shell is drawn from that seed's own RNG stream, so twelve seeds of
//! `--from-champion` are twelve different populations around one common point, which is
//! exactly the sample a claim about seeding needs.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::args::Args;
use crate::commands::{load_level, seeded_trainer, train_config, usage, TRAIN_OPTS};

/// Options this subcommand adds to the `train` flags, which it also accepts (a bench arm
/// *is* a training configuration; only the sweep around it is new).
pub const BENCH_OPTS: &[&str] = &[
    "protocol",
    "arm",
    "seeds",
    "compare",
    "list",
];

/// One named protocol: the sweep a recorded measurement was taken with.
///
/// A protocol is deliberately *not* a search configuration — the flags are the arm. What
/// it fixes is the question: which level, how big a population, how long it is allowed,
/// and over how many seeds. Those four are what make two tables comparable, and they are
/// the four a reader of `FINDINGS.md` has to get right by hand today.
#[derive(Clone, Copy, Debug)]
pub struct Protocol {
    pub name: &'static str,
    /// Level path, relative to the workspace root, exactly as `--level` takes it.
    pub level: &'static str,
    pub pop: usize,
    pub generations: u32,
    pub seeds: u32,
    /// One line: what class of level this is, so the table is readable without the pack open.
    pub note: &'static str,
}

/// The protocols the repository's own numbers were taken with.
///
/// Every row is a *shape of difficulty* rather than a level for its own sake: a plateau,
/// a hazard floor, a chain of moves, and two of the pack's shipped levels for the honest
/// middle. The budgets are the ones `FINDINGS.md` quotes beside each table, so a row
/// there and a run here are the same measurement — and a protocol whose levels or budgets
/// move is a new protocol, not a silently different one, because the arm's file records
/// what it ran.
pub const PROTOCOLS: &[Protocol] = &[
    Protocol {
        name: "chain",
        level: "levels/l9-wait-and-climb.json",
        pop: 200,
        generations: 400,
        seeds: 12,
        note: "the pack's first chain: nine moves in a row, the slowest row in the pack",
    },
    Protocol {
        name: "tower",
        level: "levels/scratch-tower.json",
        pop: 200,
        generations: 300,
        seeds: 12,
        note: "a plateau: the population reaches a spot it cannot leave",
    },
    Protocol {
        name: "hazard-floor",
        level: "levels/scratch-spike-tight.json",
        pop: 200,
        generations: 300,
        seeds: 12,
        note: "the tightest hazard floor in the repo: one safe tile between spikes",
    },
    Protocol {
        name: "gauntlet",
        level: "levels/l3-gauntlet.json",
        pop: 200,
        generations: 300,
        seeds: 12,
        note: "the shipped pack's third level: walkers and gaps, learned in tens of generations",
    },
    Protocol {
        name: "tightrope",
        level: "levels/l5-tightrope.json",
        pop: 200,
        generations: 300,
        seeds: 12,
        note: "the shipped pack's tightest obstacle course",
    },
    Protocol {
        name: "elements",
        level: "levels/l11-the-elements.json",
        pop: 200,
        generations: 2000,
        seeds: 12,
        note: "the timed crossing: three moving decks over lava, under vents and a turret",
    },
    Protocol {
        name: "marathon",
        level: "levels/l6-marathon.json",
        pop: 200,
        generations: 300,
        seeds: 12,
        note: "the longest level in the pack: a walker lottery until the trial rule was wired",
    },
];

/// An arm recorded before `fine_sigma_scale` existed bred its fine children at the coarse step,
/// which is a fraction of 1.0.
fn unscaled_fine_sigma() -> f32 {
    1.0
}

/// The `start` of an arm file written before the field existed: a fresh random population is what
/// every arm recorded before it was (the seeded arms came with the field), and saying so is more
/// useful than an empty string.
fn unrecorded_start() -> String {
    "noise (recorded before the start field existed)".to_string()
}

/// One seed's outcome inside an arm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeedRow {
    pub seed: u64,
    /// The generation the population first touched the flag, 0-based and exactly as
    /// `FINDINGS.md` quotes it. `None` means the cap was reached unsolved.
    pub first_win: Option<u32>,
    /// The best `max_fitness` this seed's run reached — at the first win when there was
    /// one, at the cap when there was not. The number that separates "never learned" from
    /// "learned everything but the last move".
    pub best_fitness: f32,
}

/// A recorded arm: the sweep, what it measured, and every knob that was in force.
///
/// The configuration half is not decoration. A result file has to be quotable a year
/// later, which means it must survive its own command line: the flags that shaped the
/// search (the net's shape, the mutation size, the two extra mutation scales, the
/// quality-diversity settings, the ranking rule, the trial count and the decision
/// cadence) are all recorded here, and the version trio is recorded so an arm taken under
/// an older sim or fitness cannot be compared with a newer one by accident.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchArm {
    pub protocol: String,
    pub arm: String,
    pub level_name: String,
    pub level_hash: String,
    pub sim_version: String,
    pub sensor_version: u32,
    pub fitness_version: u32,
    pub pop: usize,
    pub generations: u32,
    pub seeds: u32,
    /// Hidden layer widths, i.e. the network's capacity (`GaConfig::shape.hidden`).
    pub hidden: Vec<usize>,
    /// Width of the recurrent state fed back into the next decision.
    pub feedback: usize,
    pub mutation_sigma: f32,
    pub fine_children: usize,
    /// The fine scale's step as a fraction of `mutation_sigma` (`GaConfig::fine_sigma_scale`).
    ///
    /// Recorded because it is a *second* sigma and the one the archive's children use: an arm
    /// taken at 1.0 is the pre-fix behaviour (fine in count, coarse in size) and is not the same
    /// search as the default 0.25. Defaulted to 1.0 on load, which is what every arm recorded
    /// before the field existed actually ran.
    #[serde(default = "unscaled_fine_sigma")]
    pub fine_sigma_scale: f32,
    pub immigrants: usize,
    pub archive_children: usize,
    pub archive_on_diversity: bool,
    pub novelty_weight: f32,
    /// How pessimistically a genome's trials were ranked (`GaConfig::rank_pessimism`):
    /// `0.0` is the mean of its trials, `1.0` is its worst trial.
    pub rank: f32,
    /// **The number of episodes per genome this run actually spent**, from
    /// [`evo::GaConfig::effective_trials`] — not the flag's value, because on a level with
    /// walkers the trainer judges a genome on more than one patrol phase and a reader who
    /// wants to reproduce the arm needs the number that was spent.
    pub trials: u32,
    pub auto_trials: bool,
    /// **The episode budget one generation really cost**, per genome, averaged over the population
    /// ([`evo::ga::episodes_per_genome`]): 1 on a walker-free level, 3 on a walker level, and 1.75 on
    /// a walker level that raced — which is the number a wall clock should be divided by before two
    /// arms are compared, because an arm that spent fewer episodes per generation is faster *for
    /// free* and the interesting difference is the generations, not the seconds.
    ///
    /// Optional because arms recorded before this field existed have no such reading, and inventing
    /// one for them would be putting a number in a file that nobody measured. `None` says so.
    #[serde(default)]
    pub episodes_per_genome: Option<f32>,
    /// **Which episodes a generation was judged on**: `true` is the whole population on one world
    /// (`GaConfig::common_episodes`, common random numbers), `false` is every genome its own draw.
    ///
    /// Recorded because it is a property of the *comparison* rather than of a single run: two arms
    /// whose rankings were taken against different worlds are not an A/B test, and a reader who
    /// compared them would be reading the difference between two episode policies as the effect of
    /// the flag they were sweeping. Defaulted to `false` on load, which is the policy every arm
    /// recorded before this field existed was actually taken under.
    #[serde(default)]
    pub common_episodes: bool,
    /// Whether the extra trials were spent on the whole population or only on the genomes still in
    /// contention (`GaConfig::race_episodes`). Recorded for the same reason as the pairing above: it
    /// changes how many episodes a run spent, which is half of what "it solved the level in 40
    /// generations instead of 60" costs.
    #[serde(default)]
    pub race_episodes: bool,
    pub action_repeat: u32,
    /// **What "solved" meant for this arm's stop**: `false` is the first generation whose best
    /// genome won every episode it was judged on (three patrol phases on a walker level), `true` is
    /// the first generation whose *record* won every fixed validation episode. Recorded because the
    /// two are different measurements of the same level, and a table that mixed them would be
    /// comparing a three-phase draw with a twelve-world verdict.
    #[serde(default)]
    pub stop_on_validated: bool,
    /// **Where each seed's population began**, as one of `noise`, `champion <path>` or
    /// `checkpoint <path>`.
    ///
    /// Recorded because it is the one thing about a warm arm that is not derivable from the file:
    /// the shape, the knobs, the budgets and the seeds are all in here, and the point the
    /// population was seeded *from* is a file on somebody's disk. An arm that said "10 of 12"
    /// without saying *from what* would be a table nobody could rebuild, which is the exact
    /// failure the rest of this struct exists to prevent.
    ///
    /// **Defaulted on load**, so an arm recorded before this field existed still comes back
    /// readable — with its start reported as unrecorded — instead of failing as malformed JSON and
    /// sending its reader after a corrupted file that is not corrupted. The same bargain a
    /// champion's provenance fields make: the provenance is added by a serde default, and only the
    /// version trio and the level hash are allowed to *refuse* a file.
    #[serde(default = "unrecorded_start")]
    pub start: String,
    pub rows: Vec<SeedRow>,
}

impl BenchArm {
    /// Seeds the arm measured, in order.
    pub fn seed_list(&self) -> Vec<u64> {
        self.rows.iter().map(|r| r.seed).collect()
    }

    /// First-win generations of the seeds that solved, ascending.
    pub fn solved_generations(&self) -> Vec<u32> {
        let mut solved: Vec<u32> = self.rows.iter().filter_map(|r| r.first_win).collect();
        solved.sort_unstable();
        solved
    }

    fn row(&self, seed: u64) -> Option<&SeedRow> {
        self.rows.iter().find(|r| r.seed == seed)
    }
}

// ---------------------------------------------------------------------------
// bench
// ---------------------------------------------------------------------------

pub fn bench(a: &Args) -> Result<(), String> {
    // The train flags are legal here on purpose: an arm *is* a training configuration.
    let mut opts: Vec<&str> = TRAIN_OPTS.to_vec();
    opts.extend_from_slice(BENCH_OPTS);
    usage(a.reject_unknown(&opts))?;

    if a.has_flag("list") {
        if a.opt("compare").is_some() {
            return Err(
                "usage: --list prints the protocol table and --compare reads two arms; one at a time"
                    .into(),
            );
        }
        print_protocols();
        return Ok(());
    }

    let quiet = a.has_flag("quiet");
    // `--stop-on-validated` is accepted here rather than refused: it changes *when* an arm stops,
    // which is a protocol question the arm records, not a run shape that has no meaning in a sweep.
    let stop_on_validated = a.has_flag("stop-on-validated");
    if stop_on_validated && a.has_flag("no-validation") {
        return Err(
            "usage: --stop-on-validated reads the validation set that --no-validation switches off".into(),
        );
    }

    if let Some(list) = a.opt("compare") {
        // `--compare` runs nothing, so a run flag beside it would be accepted and then
        // quietly ignored. Refusing is the whole point: the operator meant one of the two.
        reject_run_flags(a)?;
        return compare(&parse_compare(list)?, quiet);
    }

    reject_train_shape_flags(a)?;

    let name = usage(a.required_string("protocol"))?;
    let protocol = protocol_by_name(&name)?;
    let cfg = train_config(a)?;

    // The protocol supplies the level and the three budget numbers; each is overridable by
    // the flag of the same meaning, and every one of them is recorded below, so an arm that
    // overrode something cannot be read as the protocol's own row.
    let level_path = a.opt("level").unwrap_or(protocol.level).to_string();
    if a.opt("level").is_some() {
        println!(
            "note    --level overrides {name}'s own level ({}), so this arm is a protocol \
             shape on another level, not the protocol's row",
            protocol.level
        );
    }
    let level = load_level(&level_path)?;
    let pop = if a.opt("pop").is_some() {
        cfg.pop_size
    } else {
        protocol.pop
    };
    let generations = usage(a.usize_opt("generations", protocol.generations as usize))? as u32;
    let seeds = usage(a.usize_opt("seeds", protocol.seeds as usize))? as u32;
    let first_seed = usage(a.u64_opt("seed", 1))?;
    if generations == 0 {
        return Err("usage: --generations must be at least 1: an arm with no generations measures nothing".into());
    }
    if seeds == 0 {
        return Err("usage: --seeds must be at least 1".into());
    }
    let arm = a.string("arm", "default");
    // The label names the file, so it has to be a label: a slash or a space here would
    // write somewhere other than where the summary line says.
    if !arm
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "usage: --arm {arm:?} must be a file-name-safe label (letters, digits, - _ .)"
        ));
    }
    let out_dir = a.string("out", "bench");
    // How every seed of this arm began. `seeded_trainer` is the one place that decides it, so this
    // only has to *name* it — and it names it from the same two flags, so the file cannot disagree
    // with the run about where the population came from.
    let start = match (a.opt("from-champion"), a.opt("resume")) {
        (Some(path), _) => format!("champion {path}"),
        (_, Some(path)) => format!("checkpoint {path}"),
        _ => "noise".to_string(),
    };

    if !quiet {
        println!(
            "bench   {name}  arm {arm}  ({}, {}x{})",
            level.name, level.width, level.height
        );
        println!("hash    {}", level.content_hash_string());
        println!(
            "budget  pop {pop}  generations {generations}  seeds {first_seed}..={}  (stops at the {})",
            first_seed + seeds as u64 - 1,
            if stop_on_validated {
                "first validated solve"
            } else {
                "first win"
            }
        );
        println!(
            "recipe  hidden [{}]  feedback {}  sigma {:.2}  fine {} x{:.2}  immigrants {}  qd {}{}  novelty {:.1}  rank {:.2}  trials {} ({})  episodes {}  action_repeat {}",
            join_usize(&cfg.shape.hidden),
            cfg.shape.feedback,
            cfg.mutation_sigma,
            cfg.fine_children,
            cfg.fine_sigma_scale,
            cfg.immigrants,
            cfg.archive_children,
            if cfg.archive_on_diversity {
                " (gated on a stall in both senses)"
            } else {
                ""
            },
            cfg.novelty_weight,
            cfg.rank_pessimism,
            cfg.effective_trials(&level),
            if cfg.trials > 1 {
                "the operator's own"
            } else if cfg.auto_trials {
                "auto"
            } else {
                "--no-auto-trials"
            },
            if cfg.common_episodes {
                if cfg.race_episodes {
                    "paired, raced"
                } else {
                    "paired"
                }
            } else {
                "per-genome"
            },
            cfg.action_repeat
        );
        println!(
            "episodes {:.2} per genome per generation",
            evo::ga::episodes_per_genome(std::slice::from_ref(&level), pop, &cfg)
        );
        println!("start   {start}");
        println!(
            "version sim {} / sensors v{} / fitness v{}\n",
            sim::SIM_VERSION,
            sim::sensors::VERSION,
            evo::FITNESS_VERSION
        );
        println!("{:>6}  {:>10}  {:>12}", "seed", "first win", "best fitness");
    }

    // Each seed is a whole run: a fresh trainer from `--seed`, stepped until it wins or the
    // cap runs out. `Trainer::step` is the only entry point, so no curriculum can fire and
    // the RNG stream is the trainer's own.
    let mut rows: Vec<SeedRow> = Vec::with_capacity(seeds as usize);
    for offset in 0..seeds {
        let seed = first_seed + offset as u64;
        let mut run = cfg.clone();
        run.pop_size = pop;
        run.master_seed = seed;
        // The *same* constructor `train` uses, including `--from-champion` and `--resume`: a warm
        // arm (every seed of the sweep beginning from one known point plus a shell drawn from that
        // seed) is a whole class of question this tool has to be able to ask, and a sweep that
        // began its runs differently from the way a real run begins would answer a different one.
        let mut trainer = seeded_trainer(run, &level, a)?;
        let mut best_fitness = f32::NEG_INFINITY;
        let mut first_win: Option<u32> = None;
        for _ in 0..generations {
            let report = trainer.step(&level);
            if report.max_fitness > best_fitness {
                best_fitness = report.max_fitness;
            }
            // `solved`, not `wins`: a seed counts as solved when a genome won **every** episode it
            // was judged on. On a deterministic level the two are the same number; on a walker
            // level `wins` is a single patrol phase, and a sweep "solving" 12 of 12 seeds on one
            // phase per genome is a statement about the phases rather than about the search.
            //
            // With `--stop-on-validated` the bar is the record's fixed validation set instead: the
            // first generation whose *champion* has been shown to win twelve unseen worlds.
            let solved_now = if stop_on_validated {
                report.record_solved
            } else {
                report.solved > 0
            };
            if solved_now {
                first_win = Some(report.generation);
                break;
            }
        }
        if !quiet {
            match first_win {
                Some(g) => println!("{seed:>6}  {g:>10}  {best_fitness:>12.2}"),
                None => println!("{seed:>6}  {:>10}  {best_fitness:>12.2}", "unsolved"),
            }
        }
        rows.push(SeedRow {
            seed,
            first_win,
            best_fitness,
        });
    }

    let record = BenchArm {
        protocol: name.clone(),
        arm: arm.clone(),
        level_name: level.name.clone(),
        level_hash: level.content_hash_string(),
        sim_version: sim::SIM_VERSION.to_string(),
        sensor_version: sim::sensors::VERSION,
        fitness_version: evo::FITNESS_VERSION,
        pop,
        generations,
        seeds,
        hidden: cfg.shape.hidden.clone(),
        feedback: cfg.shape.feedback,
        mutation_sigma: cfg.mutation_sigma,
        fine_children: cfg.fine_children,
        fine_sigma_scale: cfg.fine_sigma_scale,
        immigrants: cfg.immigrants,
        archive_children: cfg.archive_children,
        archive_on_diversity: cfg.archive_on_diversity,
        novelty_weight: cfg.novelty_weight,
        rank: cfg.rank_pessimism,
        trials: cfg.effective_trials(&level),
        auto_trials: cfg.auto_trials,
        episodes_per_genome: Some(evo::ga::episodes_per_genome(
            std::slice::from_ref(&level),
            pop,
            &cfg,
        )),
        common_episodes: cfg.common_episodes,
        race_episodes: cfg.race_episodes,
        action_repeat: cfg.action_repeat,
        stop_on_validated,
        start,
        rows,
    };

    print_summary(&record);
    let path = Path::new(&out_dir).join(format!("{name}-{arm}.json"));
    write_arm(&path, &record)?;
    println!("wrote   {}", path.display());
    Ok(())
}

fn print_protocols() {
    println!(
        "{:<14}  {:<6}  {:>5}  {:>12}  level",
        "protocol", "seeds", "pop", "generations"
    );
    for p in PROTOCOLS {
        println!(
            "{:<14}  {:<6}  {:>5}  {:>12}  {}",
            p.name, p.seeds, p.pop, p.generations, p.level
        );
        println!("{:<14}  {}", "", p.note);
    }
}

fn protocol_by_name(name: &str) -> Result<&'static Protocol, String> {
    PROTOCOLS.iter().find(|p| p.name == name).ok_or_else(|| {
        let names: Vec<&str> = PROTOCOLS.iter().map(|p| p.name).collect();
        format!(
            "usage: unknown --protocol {name:?}; known protocols are {} (or --list)",
            names.join(", ")
        )
    })
}

fn print_summary(arm: &BenchArm) {
    let solved = arm.solved_generations();
    let n = arm.rows.len();
    if solved.is_empty() {
        println!(
            "\n        solved 0 of {n}  (no seed reached the flag in {} generations)",
            arm.generations
        );
        return;
    }
    let mean = solved.iter().map(|g| *g as f64).sum::<f64>() / solved.len() as f64;
    println!(
        "\n        solved {} of {}   median {:.1}   mean {:.1}   fastest {}   slowest {}",
        solved.len(),
        n,
        median(&solved),
        mean,
        solved[0],
        solved[solved.len() - 1]
    );
}

/// The median of an ascending list, as a fraction when the count is even — the same
/// reading `FINDINGS.md`'s difficulty table quotes (`L5 14.5` is two middles averaged).
fn median(sorted: &[u32]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        sorted[n / 2] as f64
    } else {
        (sorted[n / 2 - 1] as f64 + sorted[n / 2] as f64) / 2.0
    }
}

fn join_usize(values: &[usize]) -> String {
    values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Flags that would change *what* a bench arm measures, refused rather than ignored.
fn reject_train_shape_flags(a: &Args) -> Result<(), String> {
    // Each of these changes the shape of a run rather than the search inside it, and none
    // of them has a meaning in a seed sweep: a detour would move the first win away from
    // the run's own, a suite is several levels rather than one, a chart is one seed's
    // history, and a wall clock would make the cap depend on the machine.
    const REFUSED: &[&str] = &["suite", "curriculum", "curriculum-generations", "csv", "max-seconds"];
    for name in REFUSED {
        if a.opt(name).is_some() || a.has_flag(name) {
            return Err(format!(
                "usage: --{name} shapes a `train` run and has no meaning in a bench arm: bench \
                 drives one level, one seed at a time, for a fixed number of generations, with \
                 no detours and no chart. Run `train` with it, or drop it here"
            ));
        }
    }
    Ok(())
}

/// Anything that describes a *run* is refused beside `--compare`, which runs nothing.
fn reject_run_flags(a: &Args) -> Result<(), String> {
    let mut opts: Vec<&str> = TRAIN_OPTS.to_vec();
    opts.extend_from_slice(BENCH_OPTS);
    for name in opts {
        if matches!(name, "compare" | "quiet") {
            continue;
        }
        if a.opt(name).is_some() || a.has_flag(name) {
            return Err(format!(
                "usage: --compare reads two recorded arms and trains nothing, so --{name} would \
                 be accepted and then quietly ignored. Compare the two files, or pass the flag \
                 to the arm that produced them"
            ));
        }
    }
    Ok(())
}

/// Split `--compare a.json,b.json`.
///
/// A comma list rather than two arguments because the parser takes one value per option
/// (`args.rs`), which is also why `--suite levels/a.json,levels/b.json` is spelled that way.
fn parse_compare(list: &str) -> Result<Vec<String>, String> {
    let paths: Vec<String> = list
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if paths.len() != 2 {
        return Err(format!(
            "usage: --compare takes exactly two recorded arms, e.g. --compare bench/chain-default.json,bench/chain-memory.json (got {})",
            paths.len()
        ));
    }
    Ok(paths)
}

/// Read one recorded arm, refusing anything that is not a bench result.
fn read_arm(path: &str) -> Result<BenchArm, String> {
    let src = fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    serde_json::from_str::<BenchArm>(&src).map_err(|e| format!("{path}: not a bench result: {e}"))
}

fn write_arm(path: &Path, arm: &BenchArm) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(dir) {
                return Err(format!("cannot create {}: {e}", dir.display()));
            }
        }
    }
    let json =
        serde_json::to_string_pretty(arm).map_err(|e| format!("cannot serialize the arm: {e}"))?;
    fs::write(path, json + "\n").map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Two arms, paired seed by seed, with the exact sign test over the seeds they disagree on.
fn compare(paths: &[String], quiet: bool) -> Result<(), String> {
    let (left, right) = (read_arm(&paths[0])?, read_arm(&paths[1])?);

    // The comparison is only a comparison if the two arms asked the same question of the
    // same level under the same versions. Anything else is two numbers in one table.
    if left.protocol != right.protocol {
        return Err(format!(
            "cannot compare {} ({}) with {} ({}): different protocols",
            paths[0], left.protocol, paths[1], right.protocol
        ));
    }
    if left.level_hash != right.level_hash {
        return Err(format!(
            "cannot compare {} ({}) with {} ({}): different levels — a first-win generation is \
             a statement about one level",
            paths[0], left.level_name, paths[1], right.level_name
        ));
    }
    if (left.sim_version != right.sim_version)
        || (left.sensor_version != right.sensor_version)
        || (left.fitness_version != right.fitness_version)
    {
        return Err(format!(
            "cannot compare {} (sim {} / sensors v{} / fitness v{}) with {} (sim {} / sensors v{} / fitness v{}): \
             the version trio differs, so the episodes are not the same measurement",
            paths[0],
            left.sim_version,
            left.sensor_version,
            left.fitness_version,
            paths[1],
            right.sim_version,
            right.sensor_version,
            right.fitness_version
        ));
    }
    if left.stop_on_validated != right.stop_on_validated {
        return Err(format!(
            "cannot compare {} (stop: {}) with {} (stop: {}): the first-win generation of an arm \
             that stops on a validated solve is not the same measurement as one that stops on a \
             generation's own trials",
            paths[0],
            stop_label(left.stop_on_validated),
            paths[1],
            stop_label(right.stop_on_validated)
        ));
    }
    if left.seed_list() != right.seed_list() {
        return Err(format!(
            "cannot compare {} (seeds {:?}) with {} (seeds {:?}): bench pairs two arms by seed, so \
             both must have measured the same ones",
            paths[0],
            left.seed_list(),
            paths[1],
            right.seed_list()
        ));
    }

    println!(
        "compare {}\n        vs {}",
        paths[0], paths[1]
    );
    println!(
        "protocol {}  level {} ({})  pop {} vs {}  generations {} vs {}  seeds {}\n",
        left.protocol,
        left.level_name,
        &left.level_hash[..left.level_hash.len().min(12)],
        left.pop,
        right.pop,
        left.generations,
        right.generations,
        left.rows.len()
    );
    println!(
        "  seed  {:<12}  {:<10}  {:<10}",
        left.arm, right.arm, "winner"
    );

    let mut wins = 0usize;
    let mut losses = 0usize;
    let mut ties = 0usize;
    let mut gained: Vec<u64> = Vec::new();
    let mut held: Vec<u64> = Vec::new();
    let mut solved_left = 0usize;
    let mut solved_right = 0usize;

    for seed in left.seed_list() {
        let l = left.row(seed).and_then(|r| r.first_win);
        let r = right.row(seed).and_then(|r| r.first_win);
        solved_left += l.is_some() as usize;
        solved_right += r.is_some() as usize;
        // The winner is the arm that solved where the other did not, or the quicker of two
        // solvers. Neither-solved and both-equal are ties: no information, dropped.
        let (winner, flipped) = match (l, r) {
            (Some(a), Some(b)) if a < b => (left.arm.as_str(), true),
            (Some(a), Some(b)) if b < a => (right.arm.as_str(), true),
            (Some(_), None) => (left.arm.as_str(), true),
            (None, Some(_)) => (right.arm.as_str(), true),
            _ => (TIE, false),
        };
        if winner == TIE {
            ties += 1;
        } else if winner == left.arm.as_str() {
            wins += 1;
            if flipped {
                gained.push(seed);
            }
        } else {
            losses += 1;
            if flipped {
                held.push(seed);
            }
        }
        if !quiet {
            println!(
                "{seed:>6}  {:<12}  {:<10}  {:<10}",
                label(l),
                label(r),
                if winner == "tie" { "tie".to_string() } else { winner.to_string() }
            );
        }
    }

    let p = sign_test_p(wins, losses);
    println!(
        "\nsolved  {:<12} {} of {} ({} median)\n        {:<12} {} of {} ({} median)",
        left.arm,
        solved_left,
        left.rows.len(),
        median_label(&left.solved_generations()),
        right.arm,
        solved_right,
        right.rows.len(),
        median_label(&right.solved_generations()),
    );
    println!(
        "\npaired  {} {} win{}  —  {} {} win{}  —  {} tie{}",
        left.arm,
        wins,
        if wins == 1 { "" } else { "s" },
        right.arm,
        losses,
        if losses == 1 { "" } else { "s" },
        ties,
        if ties == 1 { "" } else { "s" },
    );
    println!(
        "        p = {p:.4} (two-sided exact sign test over {} discordant seed{})",
        wins + losses,
        if wins + losses == 1 { "" } else { "s" }
    );
    println!(
        "        this reads the *direction* of each disagreement, not its size: a win by one \
         generation and a win by two hundred count the same, and the medians above are where \
         the size lives"
    );
    if !gained.is_empty() || !held.is_empty() {
        println!(
            "flipped {} gained by {} {:?}; {} held by {} {:?}",
            gained.len(),
            left.arm,
            gained,
            held.len(),
            right.arm,
            held
        );
    }
    Ok(())
}

/// The label a paired row shows when neither arm has an opinion to read.
const TIE: &str = "tie";

/// What an arm's stop was, for the refusal above and the comparison header.
fn stop_label(validated: bool) -> &'static str {
    if validated {
        "validated"
    } else {
        "first win"
    }
}

fn label(first_win: Option<u32>) -> String {
    match first_win {
        Some(g) => g.to_string(),
        None => "unsolved".to_string(),
    }
}

fn median_label(solved: &[u32]) -> String {
    if solved.is_empty() {
        "none".to_string()
    } else {
        format!("{:.1}", median(solved))
    }
}

/// The two-sided **exact** sign test over discordant pairs, as a p-value.
///
/// With `n = wins + losses` disagreements, each equally likely to go either way under the
/// null, the probability of a split at least as lopsided as the one observed is
/// `2 × Σ_{i ≤ min(wins, losses)} C(n, i) / 2^n`, capped at 1 (where the doubling has
/// already overshot). Exact rather than a normal approximation because these sweeps are
/// twelve seeds: the approximation is not wrong so much as unearned at that size, and the
/// difference decides the honest wording of a claim — six wins and no losses is p = 0.031
/// here, not "significant at the 5 % level, approximately".
///
/// `0` disagreements is `1.0`: two arms that never disagreed are two arms this test has
/// nothing to say about, whatever their medians.
pub fn sign_test_p(wins: usize, losses: usize) -> f64 {
    let n = wins + losses;
    if n == 0 {
        return 1.0;
    }
    let k = wins.min(losses);
    let tail: f64 = (0..=k).map(|i| binomial_pmf(n, i)).sum();
    (2.0 * tail).min(1.0)
}

/// `C(n, k) / 2^n`, built as a product so no factorial can overflow at these sizes.
fn binomial_pmf(n: usize, k: usize) -> f64 {
    let k = k.min(n - k);
    let mut acc = 1.0f64;
    for i in 0..k {
        acc *= (n - i) as f64 / (i + 1) as f64;
    }
    acc / 2.0f64.powi(n as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Args {
        Args::parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .ok()
            .expect("parses")
    }

    /// `cargo test` runs with the CWD set to this package, so a path a test passes on the
    /// command line is made absolute (the same helper the other test modules use).
    fn wpath(rel: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
            .to_string_lossy()
            .to_string()
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("evo-bench-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_sign_test_matches_its_exact_known_cases() {
        // No disagreement: nothing to say.
        assert_eq!(sign_test_p(0, 0), 1.0);
        // One disagreement is a coin flip, and three is an eighth.
        assert_eq!(sign_test_p(1, 0), 1.0);
        assert_eq!(sign_test_p(2, 0), 0.5);
        assert_eq!(sign_test_p(3, 0), 0.25);
        // 6-0 is 2/64, the level a twelve-seed sweep can actually reach.
        assert!((sign_test_p(6, 0) - 0.03125).abs() < 1e-12);
        // 8-2: 2 × (1 + 10 + 45) / 1024.
        assert!((sign_test_p(8, 2) - 0.109375).abs() < 1e-12);
        // Symmetric in its arguments, by construction.
        for (w, l) in [(0, 0), (1, 3), (4, 2), (6, 0), (5, 5), (9, 1)] {
            assert_eq!(sign_test_p(w, l), sign_test_p(l, w), "{w}-{l}");
        }
        // Flatter is weaker, and the doubling never pushes past 1.
        assert!(sign_test_p(8, 2) < sign_test_p(6, 4));
        assert!(sign_test_p(6, 4) <= 1.0);
        assert_eq!(sign_test_p(5, 5), 1.0);
    }

    #[test]
    fn every_protocol_names_a_level_that_loads_and_a_unique_name() {
        let mut seen: Vec<&str> = Vec::new();
        for p in PROTOCOLS {
            assert!(
                !seen.contains(&p.name),
                "protocol name {:?} appears twice",
                p.name
            );
            seen.push(p.name);
            let level = load_level(&wpath(p.level))
                .unwrap_or_else(|e| panic!("protocol {} names {}: {e}", p.name, p.level));
            assert!(!level.name.is_empty(), "protocol {}: level has no name", p.name);
            assert!(p.seeds > 0, "protocol {}: no seeds", p.name);
            assert!(p.generations > 0, "protocol {}: no generations", p.name);
            assert!(p.pop >= 2, "protocol {}: a population of {}", p.name, p.pop);
            assert!(
                !p.note.is_empty(),
                "protocol {}: the table is read without the pack open, so the note has to say \
                 what class of level this is",
                p.name
            );
        }
        assert!(
            seen.contains(&"chain") && seen.contains(&"tower") && seen.contains(&"hazard-floor"),
            "the shapes the recorded measurements are grouped by are missing: {seen:?}"
        );
        // An unknown name is a usage error that lists what does exist.
        let err = protocol_by_name("nope").expect_err("unknown protocol");
        assert!(err.contains("chain"), "{err}");
    }

    #[test]
    fn a_tiny_arm_writes_a_file_that_reads_back_with_its_recipe() {
        let dir = tmpdir("tiny");
        let out = dir.to_string_lossy().to_string();
        let level = wpath("levels/l1-flat-run.json");
        bench(&a(&[
            "bench",
            "--protocol",
            "chain",
            // The chain protocol is minutes long; the *shape* of a protocol on the smoke
            // level is seconds, which is what this test is about.
            "--level",
            &level,
            "--seeds",
            "2",
            "--generations",
            "2",
            "--pop",
            "16",
            "--arm",
            "tiny",
            "--out",
            &out,
            "--quiet",
        ]))
        .expect("a tiny arm runs");

        let path = dir.join("chain-tiny.json");
        let read_back = read_arm(&path.to_string_lossy()).expect("the arm's own file reads back");
        assert_eq!(read_back.protocol, "chain");
        assert_eq!(read_back.arm, "tiny");
        assert_eq!(read_back.pop, 16);
        assert_eq!(read_back.generations, 2);
        assert_eq!(read_back.seeds, 2);
        assert_eq!(read_back.rows.len(), 2);
        assert_eq!(read_back.seed_list(), vec![1, 2]);
        assert_eq!(read_back.level_name, "L1-flat-run");
        assert_eq!(
            read_back.level_hash,
            load_level(&level).expect("level").content_hash_string()
        );
        assert_eq!(read_back.sim_version, sim::SIM_VERSION);
        assert_eq!(read_back.fitness_version, evo::FITNESS_VERSION);
        // The recipe half: the knobs a reader needs to reproduce the arm are in the file,
        // and a first win is always inside the cap it was measured under.
        assert_eq!(read_back.hidden, evo::default_shape().hidden);
        assert_eq!(read_back.auto_trials, evo::GaConfig::default().auto_trials);
        assert!(read_back.rows.iter().all(|r| r.best_fitness.is_finite()));
        assert!(read_back
            .rows
            .iter()
            .all(|r| r.first_win.is_none_or(|g| g < read_back.generations)));
        let text = fs::read_to_string(&path).expect("file exists");
        for key in ["\"rank\"", "\"trials\"", "\"hidden\"", "\"mutation_sigma\""] {
            assert!(text.contains(key), "the arm's file is missing {key}: {text}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flags_that_would_change_what_a_sweep_measures_are_refused() {
        let level = wpath("levels/l1-flat-run.json");
        // A detour moves the first win away from the run's own; a wall clock makes the cap
        // machine-dependent; a chart is one seed's history. Each is a usage error, not a
        // silently dropped flag.
        let cases: Vec<Vec<&str>> = vec![
            vec!["--curriculum", level.as_str()],
            vec!["--curriculum-generations", "0"],
            vec!["--csv", "/tmp/never-written-bench.csv"],
            vec!["--max-seconds", "1"],
            vec!["--suite", level.as_str()],
        ];
        for extra in &cases {
            let mut argv: Vec<&str> = vec![
                "bench",
                "--protocol",
                "chain",
                "--seeds",
                "1",
                "--generations",
                "1",
            ];
            argv.extend_from_slice(extra);
            let err = bench(&a(&argv)).expect_err("must be a usage error");
            assert!(err.contains(extra[0]), "{extra:?}: {err}");
        }
        // And a run flag beside --compare is refused too, because --compare runs nothing.
        let err = bench(&a(&["bench", "--compare", "a.json,b.json", "--pop", "10"]))
            .expect_err("must be a usage error");
        assert!(err.contains("--pop"), "{err}");
        // An unknown protocol lists the known ones.
        let err = bench(&a(&["bench", "--protocol", "nope", "--seeds", "1", "--generations", "1"]))
            .expect_err("must be a usage error");
        assert!(err.contains("chain"), "{err}");
        // The list mode is a table and nothing else.
        assert!(bench(&a(&["bench", "--list"])).is_ok());
    }
}
