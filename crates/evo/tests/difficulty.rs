//! Difficulty regression for the shipped level pack.
//!
//! §9 defers a pre-training difficulty estimate because "the evolving population
//! is itself the difficulty oracle: the generation at which it first beats the
//! level". That number is what makes the pack a *pack* — a ladder rather than six
//! variants of "solvable" — so it is worth pinning. Three claims, each measured on
//! the shipped files (pop 200, twelve seeds, README "The M3 problem"):
//!
//! 1. the two smoke levels are winnable by the **initial random population**, so
//!    their interesting minute is over at generation 0;
//! 2. the four levels after them are not, on any seed tried;
//! 3. the first earned level still falls to *evolution* inside a small budget, so
//!    a fitness, sensor or level regression cannot make the pack hard in the wrong
//!    way — unlearnable — without failing the suite.
//!
//! L5 and L6 are asserted only for (2). Their measured first wins are generation
//! 68 and 40 at pop 200 (10 and 11 of 12 seeds), and reproducing those here would
//! cost ~20 s of debug test time for a number the README already quotes alongside
//! the exact command that produced it. A cheaper progress check ("max fitness
//! rose") would be testing the GA rather than the level.

use evo::{evaluate_population, GaConfig, Trainer};
use sim::Level;

/// Solved at generation 0 by a randomly initialised population.
const TRIVIAL: [&str; 2] = ["l1-flat-run", "l2-first-gap"];
/// Only ever solved by evolution.
const EARNED: [&str; 4] = ["l3-gauntlet", "l4-rhythm", "l5-tightrope", "l6-marathon"];

/// Small on purpose: the suite should stay fast, and a claim about the pack is
/// scale-free as long as the population is big enough to contain lucky genomes.
const POP: usize = 64;

fn level(slug: &str) -> Level {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../levels")
        .join(format!("{slug}.json"));
    match Level::from_json_file(&path) {
        Ok(level) => level,
        Err(err) => panic!("shipped level {slug} failed to load: {err}"),
    }
}

fn config(seed: u64) -> GaConfig {
    GaConfig {
        master_seed: seed,
        pop_size: POP,
        ..Default::default()
    }
}

/// Winners in the *unevolved* population, i.e. at generation 0.
fn random_population_wins(level: &Level, seed: u64) -> usize {
    let cfg = config(seed);
    let trainer = Trainer::new(cfg.clone());
    evaluate_population(level, &trainer.genomes, &cfg, 0)
        .iter()
        .filter(|r| r.won())
        .count()
}

#[test]
fn the_smoke_levels_are_won_by_a_random_population() {
    for slug in TRIVIAL {
        let level = level(slug);
        for seed in [1, 2] {
            let wins = random_population_wins(&level, seed);
            assert!(
                wins > 0,
                "{slug} should be beaten by {POP} random genomes at generation 0 \
                 (seed {seed} produced {wins} winners), which is what makes it a \
                 smoke test: a level with no obstacles at all"
            );
        }
    }
}

#[test]
fn no_earned_level_is_won_by_a_random_population() {
    for slug in EARNED {
        let level = level(slug);
        for seed in [1, 2, 3, 4] {
            let wins = random_population_wins(&level, seed);
            assert_eq!(
                wins, 0,
                "{slug} was won by {wins} of {POP} random genomes at generation 0 \
                 (seed {seed}). If the level was made easier on purpose, move it \
                 to TRIVIAL and re-measure its line in the README table; if not, \
                 something just made the pack's ladder collapse"
            );
        }
    }
}

/// Generations allowed for `l4-rhythm` at pop 64. The measured first win is
/// generation 17 (seed 2), and generation 43 on the slowest of two seeds, so this
/// bound is a regression guard with room to spare, not a performance claim.
const L4_CAP: u32 = 60;

#[test]
fn the_first_earned_level_still_falls_to_evolution() {
    let level = level("l4-rhythm");
    let mut trainer = Trainer::new(config(2));
    let mut generation = 0;
    let mut wins = 0;
    while generation <= L4_CAP {
        wins = trainer.step(&level).wins;
        if wins > 0 {
            break;
        }
        generation += 1;
    }
    assert!(
        wins > 0,
        "l4-rhythm was not solved within {L4_CAP} generations at pop {POP} on seed \
         2 (measured first win: generation 17). The level, the sensors or the \
         fitness function regressed, or the GA stopped breeding winners"
    );
}
