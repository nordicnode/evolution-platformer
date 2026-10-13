//! Subcommand implementations.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use evo::champion::replay as replay_champion;
use evo::{ChampionFile, GaConfig, Trainer, FITNESS_VERSION};
use sim::physics::{AgentAction, PHYSICS};
use sim::reach;
use sim::{Level, Outcome, SIM_VERSION};

use crate::args::{Args, ParseError};

const TRAIN_OPTS: &[&str] = &[
    "level",
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
    "csv",
    "max-seconds",
    "stop-on-win",
    "quiet",
];
const REACH_OPTS: &[&str] = &["level", "all", "levels-dir", "verbose"];
const REPLAY_OPTS: &[&str] = &["champion", "level", "levels-dir"];
const PLAY_OPTS: &[&str] = &["level", "seed"];
const RENDER_OPTS: &[&str] = &["level", "steps", "stride", "action", "seed"];
const SENSORS_OPTS: &[&str] = &[];

/// Turn a parser error into a message `main` will report as a usage error.
fn usage<T>(r: Result<T, ParseError>) -> Result<T, String> {
    r.map_err(|e| format!("usage: {e}"))
}

fn load_level(path: &str) -> Result<Level, String> {
    Level::from_json_file(path)
}

fn parse_action(name: &str) -> Result<AgentAction, String> {
    AgentAction::ALL
        .iter()
        .copied()
        .find(|a| a.name() == name)
        .ok_or_else(|| {
            let options: Vec<&str> = AgentAction::ALL.iter().map(|a| a.name()).collect();
            format!("unknown action {name:?}; expected one of {}", options.join(", "))
        })
}

// ---------------------------------------------------------------------------
// train
// ---------------------------------------------------------------------------

pub fn train(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(TRAIN_OPTS))?;
    let level_path = usage(a.required_path("level"))?;
    let level = load_level(&level_path)?;

    let generations = usage(a.usize_opt("generations", 30))?;
    let out_dir = a.string("out", "champions");
    let quiet = a.has_flag("quiet");
    let stop_on_win = a.has_flag("stop-on-win");
    let max_seconds: Option<f64> = match a.opt("max-seconds") {
        Some(_) => Some(usage(a.f64_opt("max-seconds", 0.0))?),
        None => None,
    };

    let cfg = GaConfig {
        pop_size: usage(a.usize_opt("pop", 200))?,
        elite: usage(a.usize_opt("elite", 4))?,
        tournament: usage(a.usize_opt("tournament", 4))?,
        mutation_rate: usage(a.f32_opt("mut-rate", 0.05))?,
        mutation_sigma: usage(a.f32_opt("mut-sigma", 0.30))?,
        crossover_rate: usage(a.f32_opt("crossover", 0.75))?,
        action_repeat: usage(a.usize_opt("action-repeat", 4))? as u32,
        // Trials are only distinguishable on levels that are actually stochastic
        // (those with walkers): elsewhere every trial is the same episode.
        trials: usage(a.usize_opt("trials", 1))? as u32,
        master_seed: usage(a.u64_opt("seed", 1))?,
        ..GaConfig::default()
    };

    if !quiet {
        println!("level   {} ({}x{})", level.name, level.width, level.height);
        println!("hash    {}", level.content_hash_string());
        println!(
            "oracle  {}",
            reachability_line(&level)
        );
        println!(
            "ga      pop {}  elite {}  tournament {}  sigma {:.2}  rate {:.3}  crossover {:.2}  action_repeat {}  trials {}",
            cfg.pop_size, cfg.elite, cfg.tournament, cfg.mutation_sigma,
            cfg.mutation_rate, cfg.crossover_rate, cfg.action_repeat, cfg.trials
        );
        if cfg.trials > 1 && level.walkers.is_empty() {
            println!(
                "note    --trials {} on a level with no walkers: every trial is the same episode,\n         so this only multiplies the cost",
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
            "\n{:>4}  {:>10}  {:>10}  {:>10}  {:>9}  {:>7}",
            "gen", "max", "avg", "min", "wins", "secs"
        );
    }

    let mut trainer = Trainer::new(cfg);
    let effective = trainer.config.clone();
    let started = Instant::now();
    let mut solved_at: Option<u32> = None;
    let mut generation_secs: Vec<f64> = Vec::with_capacity(generations);

    for _ in 0..generations {
        let t0 = Instant::now();
        let report = trainer.step(&level);
        let secs = t0.elapsed().as_secs_f64();
        generation_secs.push(secs);

        if solved_at.is_none() && report.wins > 0 {
            solved_at = Some(report.generation);
        }

        if !quiet {
            println!(
                "{:>4}  {:>10.2}  {:>10.2}  {:>10.2}  {:>5}/{:<3}  {:>7.2}{}",
                report.generation,
                report.max_fitness,
                report.avg_fitness,
                report.min_fitness,
                report.wins,
                effective.pop_size,
                secs,
                if report.genius { "  <-- GENIUS AGENT" } else { "" }
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
    let best = trainer
        .best_ever()
        .ok_or_else(|| "no generation ran, so there is no champion".to_string())?;
    let champion = ChampionFile::new(
        &best.genome,
        &best.result,
        &level,
        best.generation,
        &effective,
    );

    let stem = Path::new(&level_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| level.name.clone());
    let champion_path = Path::new(&out_dir).join(format!("{stem}.json"));
    champion.save(&champion_path)?;

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
    let median = sorted
        .get(sorted.len() / 2)
        .copied()
        .unwrap_or(0.0);
    let best_secs = sorted.first().copied().unwrap_or(0.0);

    println!();
    match solved_at {
        Some(g) => println!("solved at generation {g} (first win)"),
        None => println!(
            "not solved in {} generations; best-evidence: max fitness {:.2}",
            generation_secs.len(),
            champion.fitness
        ),
    }
    println!(
        "best    gen {} id {} fitness {:.2} ({} after {} frames, best_x {:.1})",
        champion.metadata.generation,
        champion.metadata.genome_id,
        champion.fitness,
        champion.metadata.outcome,
        champion.metadata.frames,
        champion.metadata.best_x
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

    Ok(())
}

fn write_csv(path: &str, history: &[evo::GenRow]) -> Result<(), String> {
    let mut out = String::from("generation,max_fitness,avg_fitness,min_fitness,wins,best_id,genius\n");
    for row in history {
        out.push_str(&format!(
            "{},{:.4},{:.4},{:.4},{},{},{}\n",
            row.generation,
            row.max_fitness,
            row.avg_fitness,
            row.min_fitness,
            row.wins,
            row.best_id,
            row.genius as u8
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

pub fn reach(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(REACH_OPTS))?;
    let verbose = a.has_flag("verbose");

    let paths: Vec<PathBuf> = if a.has_flag("all") {
        let dir = a.string("levels-dir", "levels");
        let mut found = list_levels(&dir)?;
        if found.is_empty() {
            return Err(format!("no *.json levels found in {dir:?}"));
        }
        found.sort();
        found
    } else {
        vec![PathBuf::from(usage(a.required_path("level"))?)]
    };

    let mut failures = 0usize;
    for path in &paths {
        let level = load_level(&path.to_string_lossy())?;
        let report = reach::report(&level);
        let status = if report.goal_reached { "OK      " } else { "UNBEATABLE" };
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
                c.from, c.takeoff_x, c.takeoff_offset, c.dir.name(), c.hold.name(), c.to
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

fn list_levels(dir: &str) -> Result<Vec<PathBuf>, String> {
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

// ---------------------------------------------------------------------------
// replay
// ---------------------------------------------------------------------------

pub fn replay(a: &Args) -> Result<(), String> {
    usage(a.reject_unknown(REPLAY_OPTS))?;
    let champion_path = usage(a.required_path("champion"))?;
    let raw = fs::read_to_string(&champion_path)
        .map_err(|e| format!("cannot read {champion_path}: {e}"))?;
    let champion = ChampionFile::from_json_str(&raw)
        .map_err(|e| format!("{champion_path}: {e}"))?;

    // Find the level: explicit, or by matching the recorded level hash. Hash
    // matching is what makes a shared champion self-describing.
    let level = match a.opt("level") {
        Some(p) => load_level(p)?,
        None => find_level_by_hash(&a.string("levels-dir", "levels"), &champion.metadata.level_hash)?
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
    println!("frames   {}", champion.metadata.frames);

    let (genome, result) = replay_champion(&level, &champion)?;
    let expected = champion.metadata.trajectory_hash.clone();
    let actual = format!("{:016x}", result.trajectory_hash);

    println!(
        "replayed {} after {} frames, fitness {:.2}, best_x {:.1}, stomps {}",
        result.outcome.as_str(),
        result.frames,
        result.fitness,
        result.best_x,
        result.stomps
    );

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

fn find_level_by_hash(dir: &str, hash: &str) -> Result<Option<Level>, String> {
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
    println!("\nprinted {printed} frame(s); final outcome {}", sim.outcome().as_str());
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

    println!(
        "\nactions v1: {} discrete macro-actions, argmax over network outputs",
        AgentAction::COUNT
    );
    for (i, (name, input, jump)) in sim::sensors::action_summary().iter().enumerate() {
        println!(
            "  [{i}] {:<14} input_x {:+}  jump {}",
            name,
            input,
            if *jump { "held" } else { "-" }
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
    println!("  air time            {:.1} ticks", PHYSICS.air_time_ticks());
    println!(
        "  jump distance       {:.3} tiles (max_jump_distance)",
        PHYSICS.max_jump_distance()
    );
    println!("  tap jump height     {:.3} tiles", PHYSICS.min_jump_height());
    println!(
        "\nfitness v{FITNESS_VERSION}: +{:.1}/pixel progress, +{:.2}/frame survived, {:.0} on death, +{:.0} on goal",
        evo::fitness::PROGRESS_PER_PIXEL,
        evo::fitness::SURVIVAL_PER_FRAME,
        evo::fitness::DEATH_PENALTY,
        evo::fitness::GOAL_BONUS
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

    #[test]
    fn every_action_name_round_trips() {
        for action in AgentAction::ALL {
            assert_eq!(parse_action(action.name()).ok(), Some(action));
        }
        assert!(parse_action("teleport").is_err());
    }

    #[test]
    fn reach_checks_all_shipped_levels_and_they_are_beatable() {
        let mut paths = list_levels(&wpath("levels")).expect("levels dir exists");
        paths.sort();
        assert!(paths.len() >= 3, "expected the L1-L3 pack, found {paths:?}");
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
        let tampered = text.replace("\"fitness_version\": 1", "\"fitness_version\": 2");
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

    /// A six-tile pit: wider than the 4.0-tile full-speed jump distance.
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
        \"#######....###......\",
        \"#######....###......\"
      ],
      \"entities\": [{\"type\": \"goal\", \"x\": 14, \"y\": 5}]
    }";
}
