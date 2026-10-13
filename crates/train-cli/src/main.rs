//! `train-cli` — the headless entry point (development_plan.md §12 M1–M3).
//!
//! Subcommands:
//!
//! | command | what it does |
//! |---|---|
//! | `train` | evolve a population on a level, print the fitness chart, save a champion |
//! | `watch` | the same training, animated: best-4 + random-4 per generation (M3) |
//! | `reach` | run the jump-reachability oracle on one level or a whole directory |
//! | `replay` | re-simulate a saved champion and verify it reproduces |
//! | `play` | human keyboard playtest in the terminal (M1 exit criterion) |
//! | `render` | print ASCII frames of a level with a fixed action, for eyeballing the sim |
//! | `sensors` | print the sensor layout and action table |
//!
//! No subcommand needs a window, so this is the fast path: rayon over all cores,
//! no GPU, no rendering. The planned web build replaces *this* binary, not the
//! crates underneath it.

mod args;
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
        "watch" => watch::watch(&args),
        "reach" => commands::reach(&args),
        "replay" => commands::replay(&args),
        "play" => commands::play(&args),
        "render" => commands::render(&args),
        "sensors" => commands::sensors(&args),
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
    train     Evolve a population on a level, print the fitness chart, save a champion.
    watch     Train with the evolution on screen: chart, best-4 + random-4 agents per
              generation, speed control, pause, death markers, genius splash (M3).
    reach     Check that a level is beatable in principle (jump-reachability oracle).
    replay    Re-simulate a saved champion and confirm it reproduces its trajectory.
    play      Play a level yourself with the keyboard, in the terminal.
    render    Print ASCII frames of a level, for eyeballing physics and entities.
    sensors   Print the observation layout, the action table and the physics bible.

    --help, -h      Show this text.
    --version, -V   Show version and the sim/sensor/fitness version trio.

TRAIN OPTIONS:
    --level <PATH>        Level JSON to train on.                        [required]
    --generations <N>     Generations to run.                                [default 30]
    --pop <N>             Population size.                          [default 200 native]
    --seed <N>            Master seed; the whole run is reproducible.        [default 1]
    --mut-rate <R>        Per-gene mutation probability (0.03-0.10).      [default 0.05]
    --mut-sigma <S>       Gaussian mutation sigma (0.2-0.5).              [default 0.30]
    --elite <N>           Genomes carried over unchanged.                     [default 4]
    --tournament <N>      Tournament selection size (3-5).                    [default 4]
    --crossover <R>       Crossover probability.                          [default 0.75]
    --action-repeat <N>   Physics ticks between network queries (3-6).        [default 4]
    --trials <N>          Episodes per genome, averaged. Only distinguishes anything
                          on levels with walkers, v1's one stochastic element.
                                                                            [default 1]
    --out <DIR>           Champion output directory.                  [default champions]
    --csv <PATH>          Also write the (generation, max, avg) history as CSV.
    --max-seconds <S>     Stop early after S wall-clock seconds.
    --stop-on-win         Stop as soon as any agent touches the flag.
    --quiet               Print only the final summary and the champion path.

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
    --mut-rate, --mut-sigma, --elite, --tournament, --crossover, --action-repeat,
    --trials              As for `train`.
    --curriculum <PATH>   Easier level to fall back to when the run stalls.
    --curriculum-generations <N>
                          Generations spent there before returning.        [default 40]
    --resume <PATH>       Start from a population checkpoint instead of a fresh one.

STALLING (watch):
    With --curriculum, the first stall writes the stuck population to
    <out>/<level>.population.json and trains it on the easier level for
    --curriculum-generations, then resumes the original level with that population.
    One step per run. The chart, the stall line and the saved champion stay
    measurements of the original level.

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
            "train", "watch", "reach", "replay", "play", "render", "sensors",
        ] {
            assert!(text.contains(cmd), "help text omits {cmd}");
        }
    }
}
