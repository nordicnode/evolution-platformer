//! `evo-app` — the desktop game shell (development_plan.md §12 M4).
//!
//! One window, three things to do in it:
//!
//! * **Edit** — drag tiles and entities out of a palette onto the level, paint with
//!   the brush, watch the reachability oracle say whether what you drew is beatable.
//! * **Play** — take the keyboard and try it yourself, with the same physics and the
//!   same action set the evolution sees.
//! * **Watch** — (next, with `evo` wired in) evolve a population on the level and
//!   watch it learn.
//!
//! Everything is drawn from the deterministic core: this binary owns no physics.
//!
//! ```text
//! evo-app [--level levels/l6-marathon.json] [--assets assets] [--screenshot out.png]
//! ```

mod app;
mod atlas;
mod editor;
mod play;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
evo-app — the evolution-platformer desktop shell

USAGE:
    evo-app [OPTIONS]

OPTIONS:
    --level <PATH>        Level to open.                       [default l1-flat-run]
    --assets <DIR>        Directory holding tileset.json/png.  [default assets]
    --screenshot <PATH>   Render one frame, write it as a PNG, and exit. Used to
                          verify the renderer without a human watching.
    --help, -h            Show this text.
";

struct Options {
    level: PathBuf,
    assets: PathBuf,
    screenshot: Option<PathBuf>,
}

fn parse(raw: &[String]) -> Result<Option<Options>, String> {
    let mut level = None;
    let mut assets = None;
    let mut screenshot = None;
    let mut i = 0;
    while i < raw.len() {
        let arg = raw[i].as_str();
        let mut value = |name: &str| -> Result<String, String> {
            i += 1;
            raw.get(i)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg {
            "--help" | "-h" => return Ok(None),
            "--level" => level = Some(PathBuf::from(value("--level")?)),
            "--assets" => assets = Some(PathBuf::from(value("--assets")?)),
            "--screenshot" => screenshot = Some(PathBuf::from(value("--screenshot")?)),
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

    match app::run(options.level, options.assets, options.screenshot) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
