//! `train-cli` — the headless entry point (development_plan.md §12 M1–M3).
//!
//! Subcommands:
//!
//! | command | what it does |
//! |---|---|
//! | `train` | evolve a population on a level, print the fitness chart, save a champion |
//! | `es` | the same level on the same episode budget, searched by an evolution strategy |
//! | `watch` | the same training, animated: best-4 + random-4 per generation (M3) |
//! | `reach` | run the jump-reachability oracle on one level or a whole directory |
//! | `replay` | re-simulate a saved champion and verify it reproduces |
//! | `play` | human keyboard playtest in the terminal (M1 exit criterion) |
//! | `render` | print ASCII frames of a level with a fixed action, for eyeballing the sim |
//! | `sensors` | print the sensor layout and action table |
//! | `bench` | run a recorded seed-sweep protocol and compare two arms, paired |
//!
//! No subcommand needs a window, so this is the fast path: rayon over all cores,
//! no GPU, no rendering. The planned web build replaces *this* binary, not the
//! crates underneath it.

mod args;
mod bench;
mod commands;
mod watch;

use std::process::ExitCode;

use args::{Args, ParseError};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match run(&raw) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage) => {
            eprintln!("\ntry `train-cli --help`");
            ExitCode::from(2)
        }
        Err(Failure::Error(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

enum Failure {
    /// Bad invocation: exit 2, as most Unix tools do.
    Usage,
    /// Runtime failure: exit 1.
    Error(String),
}

impl From<String> for Failure {
    fn from(s: String) -> Self {
        Failure::Error(s)
    }
}

fn run(raw: &[String]) -> Result<(), Failure> {
    let args = match Args::parse(raw) {
        Ok(a) => a,
        Err(ParseError::HelpRequested) => {
            print!("{}", help());
            return Ok(());
        }
        Err(ParseError::VersionRequested) => {
            println!("train-cli {VERSION}");
            println!("sim {} / sensors v{} / fitness v{}", sim::SIM_VERSION, sim::sensors::VERSION, evo::FITNESS_VERSION);
            return Ok(());
        }
        Err(ParseError::Bad(msg)) => {
            eprintln!("error: {msg}");
            return Err(Failure::Usage);
        }
    };

    let result = match args.command.as_str() {
        "train" => commands::train(&args),
        "es" => commands::es(&args),
        "watch" => watch::watch(&args),
        "reach" => commands::reach(&args),
        "replay" => commands::replay(&args),
        "score" => commands::score(&args),
        "play" => commands::play(&args),
        "render" => commands::render(&args),
        "sensors" => commands::sensors(&args),
        "bench" => bench::bench(&args),
        other => Err(format!(
            "unknown subcommand {other:?} (try --help for the list)"
        )),
    };

    match result {
        Ok(()) => Ok(()),
        Err(msg) => {
            // A usage error raised inside a command is still a usage error.
            if msg.starts_with("usage:") {
                eprintln!("error: {}", msg.trim_start_matches("usage: "));
                Err(Failure::Usage)
            } else {
                Err(Failure::Error(msg))
            }
        }
    }
}

fn help() -> String {
    format!(
        r#"train-cli {VERSION} -- evolution-platformer headless driver

USAGE:
    train-cli <COMMAND> [OPTIONS]

COMMANDS:
    train     Evolve a population on a level (or a whole suite of them, see --suite),
              print the fitness chart, save a champion.
    es        Search the same level with an evolution strategy instead of the GA, on the
              same episode budget. The independent baseline: it is here to say whether
              the search or the problem is the binding constraint. See ES OPTIONS.
    watch     Train with the evolution on screen: chart, best-4 + random-4 agents per
              generation, speed control, pause, death markers, genius splash (M3).
    reach     Check that a level is beatable in principle (jump-reachability oracle).
    replay    Re-simulate a saved champion and confirm it reproduces its trajectory.
    score     Score a saved champion on one level or a suite of them: what one policy
              actually does on each, which is the read a suite's claim has to survive.
    play      Play a level yourself with the keyboard, in the terminal.
    render    Print ASCII frames of a level, for eyeballing physics and entities.
    sensors   Print the observation layout, the action table and the physics bible.
    bench     Run one of the recorded seed-sweep protocols, record the arm, and compare two
              recorded arms with a paired sign test (see BENCH OPTIONS).

    --help, -h      Show this text.
    --version, -V   Show version and the sim/sensor/fitness version trio.

TRAIN OPTIONS:
    --level <PATH>        Level JSON to train on. [required unless --suite is given]
    --suite <A,B,...>     Train ONE population on several levels at once, every generation
                          ranked by the level it is **worst** at: the standard route to a
                          policy that generalises rather than memorises, and the item the
                          review ranks largest (REVIEW.md §4 item 5). Level 0 is where a
                          champion is recorded (written as `<level0>-suite.json` so a
                          single-level champion of the same level is never overwritten) and
                          where the quality-diversity archive reads its behaviour. The
                          aggregate is a **minimum and not the sum**: one flag is worth
                          10 000 while a whole level of progress is worth about 1 000, so a
                          sum would let the easiest level own the score and would teach the
                          population to specialise — the one thing a suite is for.
                          `--stop-on-win` means **every** level won, and each level keeps
                          its own episode budget (three patrol phases where it has walkers).
                          Duplicates and an empty entry are refused rather than guessed at.
    --novelty <F>         Fitness units paid to a genome for behaving **rarely**, added to the
                          score the population is ranked and bred by — and never to a number
                          that is recorded: the chart, the champion's fitness and `replay`
                          still read the real episode, so a novelty run is comparable with
                          every other run in the notes. The bonus is `F x (1 - share of the
                          population in the genome's behavioural cell)`, so `--novelty 16`
                          pays a genome alone in its niche one tile of ground. Off by
                          default (0.0); measured on the plateau and on the chain level in
                          REVIEW.md §4, and offered rather than shipped for the reason that
                          section records.                             [default 0.0]
    --rank-pessimism <F>  Where between the **mean** of a genome's own trials and the **worst**
                          of them the ranking sits: 0.0 (the default) is the mean, 1.0 is the
                          worst trial, and 0.5 scores a genome halfway between what it usually
                          does and what it did at its worst. A *ranking* knob only — it changes
                          the score selection and the tournament read and never a number the
                          run records, so a pessimistic run stays comparable with every other
                          run in the notes. Free on a level with no walkers: one episode is
                          its own floor. It is the knob that answers finding 14 (an elite
                          re-rolled out of the run by a patrol phase it never saw).
                                                                            [default 0.0]
    --generations <N>     Generations to run.                                [default 30]
    --pop <N>             Population size.                          [default 200 native]
    --seed <N>            Master seed; the whole run is reproducible.        [default 1]
    --mut-rate <R>        Per-gene mutation probability (0.03-0.10).      [default 0.05]
    --mut-sigma <S>       Gaussian mutation sigma when the search starts (0.2-0.5).
                                                                          [default 0.30]
    --adaptive-sigma      Let the run tune its own mutation size, between 0.05 and 1.00.
                          Off by default because it was measured against a fixed sigma on
                          this level pack and lost on the hard level: on a plateau the
                          success rate says nothing about whether a smaller step would
                          land, and shrinking freezes the lineage that is stuck. See
                          `evo::ga` for the three readings that were measured, and
                          `REVIEW.md` for the numbers.
    --fine-children <N>   Offspring per generation bred by perturbing only one to three
                          genes of the generation's best genome (a step) rather than the
                          ~86 genes a normal child moves (a jump). 0 disables.  [default 8]
    --stagnation <N>      Generations without a new best before the population is given a
                          shake. 0 disables.                                [default 20]
    --immigrants <N>      How many arrive when that happens. They are **reheated copies of
                          the run's best-ever genome** — every gene moved — so the batch
                          explores a long way from the best point found instead of
                          restarting from noise.                        [default 8]
    --archive-children <N>
                          Offspring per generation bred from the quality-diversity archive
                          instead of the ranking: perturbations of the best genome found in
                          one of the archive's niches, which is the kind of genome a
                          plateau's next obstacle needs and the ranking usually throws
                          away. Fires every generation, not only on a stall. **Off by
                          default**, because it is measured as a loss on the hardest
                          level in the pack: 10 of 12 seeds solved with it off, 7 at 2 and
                          6 at 4, against wins that come earlier on easier levels. It is
                          a knob to try on a level that is stuck, not a setting to ship.
                                                                            [default 0]
    --archive-on-diversity
                          Breed those archive children only when the run is out of ideas in
                          both senses: no new best for --stagnation generations *and* no new
                          behavioural niche over the same window. Measured on `scratch-tower`
                          at 12 seeds: the ungated trickle 7 of 12 at a mean first-win
                          generation of 121, gated 9 of 12 at 76, gate off 10 of 12 at 94 — so
                          the gate is the first archive reading that is not a loss on a
                          plateau. **Off by default anyway**, because on a hazard floor the
                          gate is what costs: `scratch-spike-tight` solves 3 of 12 with the
                          ungated trickle against 1 with the gate, since a run grinding across
                          a hazard floor is always finding something new and the condition
                          never arrives. Use it on a plateau.
    --reheat-factor <X>   How hot that batch is, as a multiple of the run's own mutation
                          size, held inside the 0.05-1.00 sigma band. 1.0 throws it as far as
                          an ordinary mutation step but at every gene rather than five percent
                          of them; 2.0 reaches twice as far. Swept over the hardest level in
                          the pack at 12 and then 24 seeds, where 3.0 solved 14 of 24 against
                          1.0's 10 and nothing anywhere favoured a cooler batch, so 3.0 (0.90
                          at the shipped sigma) is the default. Lower it for a run that wants
                          the gentler batch.                            [default 3.0]
    --elite <N>           Genomes carried over unchanged.                     [default 4]
    --tournament <N>      Tournament selection size (3-5).                    [default 4]
    --crossover <R>       Crossover probability.                          [default 0.75]
    --action-repeat <N>   Physics ticks between network queries (3-6).        [default 4]
    --feedback <N>        Width of the network's memory: that many values of its own first
                          hidden layer are fed back into the next decision, so the policy
                          has a state of its own rather than being a pure function of the
                          frame in front of it — which is what lets it count, wait for a
                          walker, and commit to a manoeuvre across several decisions. 0,
                          the default, is a memoryless net: the recipe every measurement
                          in `REVIEW.md` was taken with. Costs `N x 16` extra genes, and
                          is clamped to 64.
                                                                            [default 0]
    --hidden <W[,W...]>   Width of each hidden layer: `--hidden 32` is one layer of 32,
                          `--hidden 32,16` is two. The shipped recipe is one layer of 16 —
                          1 739 genes — and this is the flag that asks whether a bigger net
                          searches better. Clamped to 64 per layer and four layers by the
                          trainer, so a typo is a smaller run rather than a failed allocation,
                          and the net a run actually trains is printed in its header.
    --trials <N>          Episodes per genome, averaged. Only distinguishes anything
                          on levels with walkers, v1's one stochastic element. Left at its
                          default, a level **with walkers in it** is judged on three patrol
                          phases rather than one, through the trainer's own rule
                          (`GaConfig::recommended_trials_for`), because a single phase is a
                          re-roll of the RNG rather than a measurement of a genome.
                                                                            [default 1]
    --no-auto-trials      Release that rule and judge every level on the literal `--trials`
                          count, which is how a number measured before the rule existed is
                          reproduced (and how the one-phase runs in the notes were taken).
    --no-common-episodes  Judge every genome on **its own** episodes rather than on the
                          generation's shared ones: `episode_seed(master, generation,
                          genome id, trial)` instead of `common_episode_seed(master,
                          generation, trial)`. This is the policy every measurement taken
                          before the flag existed was made under, so it is how an older
                          table is reproduced; it also switches racing off, because a race
                          cannot be sounded on unpaired episodes. See REVIEW.md §4.
    --race-episodes       Spend the second and third trial only on the genomes still in
                          contention: the population is ranked on trial 0, the better half
                          is played on trial 1, the better quarter on trial 2. A walker
                          level's three-phase budget therefore costs 1.75 episodes per
                          genome instead of 3.00 — about 29 % of the wall clock at pop 200
                          on `l3-gauntlet` — with the same first-win counts on the shipped
                          protocols. On by default; the flag is for saying so explicitly.
    --no-race-episodes    Judge every genome on every trial, which is what a flat three-trial
                          pass costs, for comparing against the raced default.
    --curriculum <PATH>   Easier level to fall back to when the run stalls.
    --curriculum-generations <N>
                          Generations spent there before returning. 0 takes the detour
                          out of the run altogether — the protocol measurements before
                          finding 50 were taken under.                       [default 40]
    --resume <PATH>       Start from a **population checkpoint** — `<out>/<level>.population.json`,
                          the artifact a stall writes — instead of a fresh population: the
                          population, its genome ids (part of every episode seed) and its
                          generation numbering all continue. A checkpoint taken on another
                          level is accepted and reported rather than refused; that is the
                          curriculum's whole point.
    --from-champion <PATH>
                          Start from one saved champion plus a shell of reheated copies of
                          it: the run begins at generation 0 with the champion in slot 0.
                          The champion's own level is deliberately *not* required to match —
                          that is what makes this the way to fine-tune on a level you edited
                          or to carry one level's policy into a harder one — and the mismatch
                          is printed. The shape must match: pass the shape it was trained
                          with. A run seeded this way writes `<level>-seeded.json` when it
                          would otherwise overwrite its own seed.
    --from-route          Warm the population from **this level's own route**: the reachability
                          oracle walks the level, the walk is cloned into a genome, and that
                          clone plus mutated relatives fills part of generation 0. What it buys
                          is a starting population that already covers ground — measured on the
                          shipped pack at the 90th-100th percentile of a random population's
                          fitness — so the search is spent on the obstacle rather than on
                          learning to walk. It is **not** a solver, and the reason is the decision
                          cadence: the oracle presses a take-off on the tick its script reaches
                          it while a policy decides once every four ticks, so a clone that
                          reproduces every one of the walk's decisions still walks into the pit
                          (at a one-tick cadence the same clone solves the level on every seed).
                          The run reports the replay's own verdict (`route ...`) before it starts.
    --warm <N>            How many of the population's slots the clone fills; the rest are random,
                          and that remainder is the design rather than slack. Measured, six
                          seeds at pop 64 per arm: a quarter of the population solves every
                          level on every seed and reaches a first win two to four times sooner
                          than random, and warming *every* slot is the **worst** arm — the
                          clone's basin is a plateau with no win on it.   [default pop/4]
    --clone-epochs <N>    Gradient passes over the replay's demonstration when cloning it.
                                                                [default 300]
    --route-seed <N>      Episode seed of the replay that produces the demonstration: the walk is
                          deterministic, so this is what makes a warm start reproducible.
                                                                    [default --seed]
    --out <DIR>           Champion output directory.                  [default champions]
    --csv <PATH>          Also write the (generation, max, avg) history as CSV.
    --max-seconds <S>     Stop early after S wall-clock seconds.
    --stop-on-win         Stop as soon as any agent touches the flag.
    --quiet               Print only the final summary and the champion path.

ES OPTIONS:
    --level <PATH>        Level to search.                                 [required]
    --generations <N>     Generations to run.                                [default 30]
    --pop <N>             Perturbations per generation: λ, rounded down to an even number
                          because a mirror without its pair is half an estimate.
                                                                            [default 200]
    --seed <N>            Master seed; the whole run is reproducible.        [default 1]
    --sigma <S>           Size of the probe, in gene units. Smaller than a mutation
                          sigma on purpose: a step wants a steady direction, not a
                          saturation on the biggest mutant.                [default 0.05]
    --learning-rate <L>   How far along the rank-weighted direction the mean moves each
                          generation.                                         [default 0.01]
    --action-repeat <N>   Physics ticks between network queries. Matched to the GA's.
    --feedback <N>        Memory width, exactly as for `train`.
    --trials <N>          Episodes per genome, averaged — matched to the GA's.
    --max-seconds <S>     Stop early after S wall-clock seconds.
    --stop-on-win         Stop as soon as any member of the batch touches the flag.
    --quiet               Print only the final summary.

WATCH OPTIONS:
    --level <PATH>        Level to evolve on. Required unless --champion is given.
    --champion <PATH>     Watch a saved champion replay instead of training.
    --generations <N>     Generations to watch.                                [default 300]
    --pop <N>             Population size.                          [default 200 native]
    --seed <N>            Master seed.                                       [default 1]
    --speed <N>           Physics ticks per rendered frame, snapped to 1-50.   [default 8]
    --best <N>            Best-ranked agents drawn per generation.              [default 4]
    --random <N>          Random outsiders drawn per generation.                [default 4]
    --death-window <N>    Generations a death marker keeps showing.           [default 25]
    --frames <N>          Stop after N rendered frames (always on when headless).[default 6]
    --headless            Plain text frames, no keyboard. Implied without a TTY.
    --out <DIR>           Where the best-ever champion is saved.      [default champions]
    --mut-rate, --mut-sigma, --adaptive-sigma, --fine-children, --stagnation,
    --immigrants, --archive-children, --reheat-factor, --elite, --tournament, --crossover,
    --action-repeat, --feedback, --hidden, --rank-pessimism, --trials
                          As for `train`.
    --curriculum <PATH>   Easier level to fall back to when the run stalls.
    --curriculum-generations <N>
                          Generations spent there before returning.        [default 40]
    --resume <PATH>       Start from a population checkpoint instead of a fresh one.

BENCH OPTIONS:
    A seed sweep as one command: run a named protocol over every seed, record the arm and
    every knob that was in force to <out>/<protocol>-<arm>.json, then read two arms back
    paired, seed by seed. `bench` accepts every TRAIN flag, because an arm *is* a training
    configuration and nothing else; it refuses the flags that would change what a sweep
    measures (--suite, --curriculum, --curriculum-generations, --csv, --max-seconds), and
    it never fires a detour or reads a clock: it drives the trainer one level at a time for
    a fixed number of generations and stops at the first win, which is exactly what
    FINDINGS.md's own `for seed in $(seq 1 12)` loops do by hand.

    --protocol <NAME>     The sweep: a frozen (level, pop, generations, seeds) row whose
                          name is how a table in the notes is reproduced. `--list` prints
                          the table.                                      [required]
    --arm <LABEL>         What this arm is called. It names the result file, and it is the
                          label `--compare` prints.                    [default default]
    --seeds <N>           Seeds to sweep, starting at `--seed`.  [default the protocol's]
    --level <PATH>        Run the protocol's shape on another level. An arm records the
    --pop <N>             level it actually ran, so an overridden arm cannot be quoted as
    --generations <N>     the protocol's own row.
    --seed <N>            The *first* seed of the sweep.                   [default 1]
    --out <DIR>           Where result files are written.              [default bench]
    --compare <A,B>       Read two recorded arms and print the paired comparison: which arm
                          won each seed (solved where the other did not, or solved sooner),
                          the solved counts and medians, the two-sided exact sign test over
                          the seeds the arms disagree on, and the seeds that flipped. The
                          test reads the direction of each disagreement and not its size.
    --list                Print the protocol table and exit.
    --quiet               Drop the per-seed lines; the summary and the file remain.

STALLING (train and watch):
    On a stall the run writes the stuck population to
    <out>/<level>.population.json and trains it on an easier level for
    --curriculum-generations, then resumes the original level with that population.
    The chart, the stall line and the saved champion stay measurements of the
    original level.

    With --curriculum the easier level is the one you named, and one step per run.
    Without it, the run practises where it is stuck (findings 50–51): the same level
    with the spawn moved to the furthest spot its best genome has stood on and the
    flag moved to the stand the route to the flag reaches next — one crossing the
    jump oracle can make, not the nearest stand a straight line prefers. Written to
    <out>/<level>.practice.json so it can be opened and played. Every episode of the
    detour is then the next move rather than the whole chain, the detour is judged on
    the move and the climb together (a two-level suite, ranked by the worst), and the
    step re-arms whenever the run stalls at a new spot.

KEYS (watch):
    space pause/resume   -/+ speed   m/M mutation rate   n next generation
    left/right scrub the recorded champion routes   h back to live
    f follow/whole level   s save the best-ever champion   q quit

REACH OPTIONS:
    --level <PATH>        One level to check.
    --all                 Check every level in --levels-dir.
    --levels-dir <DIR>    Directory to sweep with --all.              [default levels]
    --verbose             List every platform and every edge.

REPLAY OPTIONS:
    --champion <PATH>     Champion JSON to replay.                      [required]
    --level <PATH>        Level it was trained on; omitted = infer from name.
    --frames <N>          Render up to N ASCII frames of the replay.

SCORE OPTIONS:
    --champion <PATH>     Champion JSON to score.                       [required]
    --suite <A,B,...>     Levels to score it on. Omitted, a single level is resolved from the
                          champion's own content hash in --levels-dir, which is the one
                          answer that cannot be wrong.
    --level <PATH>        One level to score it on instead.
    --levels-dir <DIR>    Directory the hash is looked up in.            [default levels]
    --seed <N>            Master seed the fresh episodes are drawn from. [default the
                          champion's own training seed]
                          The episodes are the ones this genome would have faced in
                          generation 0 of its own run — reproducible, and *not* the single
                          episode the file records, which is what `replay` verifies. Each
                          level is judged on the trainer's own trial count (three patrol
                          phases where it has walkers), and a level counts as solved only
                          when every one of its trials won.

PLAY OPTIONS:
    --level <PATH>        Level to play.                                [required]
    --seed <N>            Episode seed.                                      [default 1]

RENDER OPTIONS:
    --level <PATH>        Level to render.                              [required]
    --steps <N>           Physics steps to simulate.                         [default 0]
    --stride <N>          Print a frame every N steps.                       [default 20]
    --action <NAME>       Fixed action each tick: run, run_jump, retreat,
                          brake, jump_in_place, walk.                      [default run]
    --seed <N>            Episode seed.                                      [default 1]

EXAMPLES:
    train-cli train --level levels/l1-flat-run.json --generations 40
    train-cli train --level levels/l2-first-gap.json --pop 200 --csv out/l2.csv
    train-cli bench --protocol chain --arm default
    train-cli bench --protocol tower --arm hidden32 --hidden 32
    train-cli bench --compare bench/tower-default.json,bench/tower-hidden32.json
    train-cli train --level levels/l5-tightrope.json --from-champion champions/l3-gauntlet.json
    train-cli es --level levels/scratch-tower.json --generations 300 --stop-on-win
    train-cli reach --all
    train-cli watch --level levels/l2-first-gap.json
    train-cli watch --champion champions/l1-flat-run.json --speed 16
    train-cli replay --champion champions/l1-flat-run.json
    train-cli play --level levels/l1-flat-run.json
    train-cli render --level levels/l3-gauntlet.json --steps 400 --stride 40
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_and_version_succeed() {
        assert!(run(&args(&["--help"])).is_ok());
        assert!(run(&args(&["--version"])).is_ok());
        assert!(run(&args(&["-h"])).is_ok());
    }

    #[test]
    fn an_unknown_subcommand_is_a_runtime_error() {
        assert!(matches!(run(&args(&["frobnicate"])), Err(Failure::Error(_))));
    }

    #[test]
    fn a_missing_required_option_is_a_usage_error() {
        assert!(matches!(run(&args(&["train"])), Err(Failure::Usage)));
        assert!(matches!(run(&args(&["render"])), Err(Failure::Usage)));
    }

    #[test]
    fn an_unreadable_level_is_a_runtime_error_not_a_usage_error() {
        let outcome = run(&args(&["reach", "--level", "levels/does-not-exist.json"]));
        assert!(matches!(outcome, Err(Failure::Error(_))));
    }

    #[test]
    fn help_text_mentions_every_subcommand_the_dispatcher_accepts() {
        let text = help();
        for cmd in [
            "train", "es", "watch", "reach", "replay", "play", "render", "sensors", "bench",
        ] {
            assert!(text.contains(cmd), "help text omits {cmd}");
        }
    }
}
