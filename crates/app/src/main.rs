//! `evo-app` — the desktop game shell (development_plan.md §12 M4).
//!
//! One window, three things to do in it:
//!
//! * **Edit** — drag tiles and entities out of a palette onto the level, paint with the
//!   brush, watch the reachability oracle say whether what you drew is beatable.
//! * **Play** — take the keyboard and try it yourself, with the same physics and the
//!   same action set the evolution sees.
//! * **Evolve** — the M3 watch mode, in the window: a population trains on the level on
//!   its own thread while the generation it just finished is animated over the terrain,
//!   with the fitness chart, the run's readings and the recorded champion routes.
//! * **Replay** — the run's answer, once the viewer has stopped pushing the search: its
//!   quickest winning agent playing the level at real time, with the population of the last
//!   generation it scored ghosted on the same clock. (A run that has not won yet shows its
//!   best-scoring attempt instead, and says so.)
//!
//! Everything is drawn from the deterministic core: this binary owns no physics, and the
//! trainer it hosts is the same `evo` the headless driver uses.
//!
//! The run the window hosts can be the terminal's run, not a simplified one: `--pop`,
//! `--seed`, `--generations`, `--champions`, `--resume` and `--curriculum` are
//! `train-cli watch`'s flags with the same meanings, so a run started in the window is a
//! run the terminal can resume and a checkpoint written by either front-end is readable
//! by the other.
//!
//! ```text
//! evo-app [--level levels/l6-marathon.json] [--assets assets] [--splash] [--play]
//!         [--evolve] [--replay] [--pop 200] [--seed 1] [--generations 300]
//!         [--champions champions] [--resume champions/l5-tightrope.population.json]
//!         [--curriculum levels/l3-gauntlet.json] [--curriculum-generations 40]
//!         [--screenshot out.png]
//! ```

mod app;
mod atlas;
mod editor;
mod help;
mod lab;
mod panel;
mod play;
mod replay;
mod shelf;
mod splash;
mod theme;
mod ui;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;

use app::LabOptions;

const USAGE: &str = "\
evo-app — the evolution-platformer desktop shell

USAGE:
    evo-app [OPTIONS]

OPTIONS:
    --level <PATH>        Level to open.                       [default l1-flat-run]
    --assets <DIR>        Directory holding tileset.json/png.  [default assets]
    --splash              Show the title screen and wait for a key. It is the default
                          opening for an interactive launch; pass --no-splash to go
                          straight to the editor, or --splash to see it in front of
                          a scripted frame as --splash --screenshot does. The screen
                          plays the run's saved champion for the level being opened
                          when --champions holds one that won, and a demo run otherwise.
    --no-splash           Skip the title screen and open the editor on the first frame.
    --play                Open in the playtest, with the clock running from the spawn.
    --evolve              Open in the evolution view and start training at once.
    --replay              Open in the replay of the run's quickest win: it starts a run
                          like --evolve, takes it to its cap when one was given, and then
                          plays back the episode that reached the flag fastest at real
                          time, with the population of the last generation it scored
                          ghosted. A run that has not won shows its best-scoring attempt.
    --no-ghosts           Open that replay with the population hidden, showing that one
                          agent alone (the `g` key and the panel's checkbox do the same
                          thing while the window is up).
    --pop <N>             Population size for that run.        [default 200]
    --seed <N>            Master seed; every run is reproducible from it. [default 1]
    --generations <N>     Stop after N generations.            [default: until closed]
    --champions <DIR>     Where Save champion writes, and where a stalled run writes
                          its checkpoint.                     [default champions]
    --resume <PATH>       Start from a population checkpoint instead of a fresh
                          population (as `train-cli watch --resume` does).
    --curriculum <PATH>   Easier level to fall back to on the first stall.
    --curriculum-generations <N>
                          Generations to spend there before returning.
                                                                 [default 40]
    --screenshot <PATH>   Render one frame, write it as a PNG, and exit. Used to
                          verify the renderer without a human watching; with
                          --evolve it waits for a real generation first.
    --help, -h            Show this text.
";

struct Options {
    level: PathBuf,
    assets: PathBuf,
    screenshot: Option<PathBuf>,
    lab: LabOptions,
    /// Whether to open on the title screen. `None` is the default, which is "on for a person
    /// at a window, off for a scripted frame" — see [`app::run`].
    splash: Option<bool>,
}

fn parse(raw: &[String]) -> Result<Option<Options>, String> {
    let mut level = None;
    let mut assets = None;
    let mut screenshot = None;
    let mut lab = LabOptions::default();
    // Unset is the common case, and it means "the default": on for an interactive launch,
    // off for a scripted one. The flags are for overriding that.
    let mut splash = None;
    let mut i = 0;
    while i < raw.len() {
        let arg = raw[i].as_str();
        let mut value = |name: &str| -> Result<String, String> {
            i += 1;
            raw.get(i)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        let mut number = |name: &str| -> Result<u64, String> {
            let text = value(name)?;
            text.parse::<u64>()
                .map_err(|_| format!("{name} needs a number, got {text:?}"))
        };
        match arg {
            "--help" | "-h" => return Ok(None),
            "--level" => level = Some(PathBuf::from(value("--level")?)),
            "--assets" => assets = Some(PathBuf::from(value("--assets")?)),
            "--screenshot" => screenshot = Some(PathBuf::from(value("--screenshot")?)),
            "--splash" => splash = Some(true),
            "--no-splash" => splash = Some(false),
            // A population of one has no selection pressure, and a zero-generation run
            // would never score anything: both are refused rather than clamped, because a
            // scripted `--generations 0` is a mistake in the script, not a preference.
            "--play" => lab.play = true,
            "--evolve" => lab.evolve = true,
            // `--replay` is `--evolve` plus one thing done afterwards, so it is not an
            // error to ask for both: whichever lands, the run is started and the replay is
            // what the window ends up showing.
            "--replay" => lab.replay = true,
            "--no-ghosts" => lab.ghosts = false,
            "--pop" => {
                let pop = number("--pop")?;
                if pop < 2 {
                    return Err("usage: --pop must be at least 2".into());
                }
                lab.pop_size = pop as usize;
            }
            "--seed" => lab.seed = number("--seed")?,
            "--generations" => {
                let generations = number("--generations")?;
                if generations == 0 {
                    return Err("usage: --generations must be at least 1".into());
                }
                lab.generations = Some(generations as u32);
            }
            "--champions" => lab.champions = PathBuf::from(value("--champions")?),
            "--resume" => lab.resume = Some(PathBuf::from(value("--resume")?)),
            // The easier level is *not* read here: whether it exists is the app's answer,
            // because it is the run that needs it, and a run that is never started must
            // not fail over a file it would never have opened.
            "--curriculum" => lab.curriculum = Some(PathBuf::from(value("--curriculum")?)),
            "--curriculum-generations" => {
                let generations = number("--curriculum-generations")?;
                if generations == 0 {
                    return Err("usage: --curriculum-generations must be at least 1".into());
                }
                lab.curriculum_generations = generations as u32;
            }
            other => return Err(format!("unknown option {other:?} (try --help)")),
        }
        i += 1;
    }

    // The default level is resolved relative to the workspace root, so the binary
    // works whether it is launched from there or from anywhere else.
    let level = level.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels/l1-flat-run.json")
    });
    let assets = assets
        .or_else(|| {
            let local = PathBuf::from("assets");
            if local.is_dir() {
                Some(local)
            } else {
                None
            }
        })
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"));

    Ok(Some(Options {
        level,
        assets,
        screenshot,
        lab,
        splash,
    }))
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse(&raw) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    match app::run(
        options.level,
        options.assets,
        options.screenshot,
        options.lab,
        options.splash,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
