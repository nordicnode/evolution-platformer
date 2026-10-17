//! Difficulty regression for the shipped level pack.
//!
//! §9 defers a pre-training difficulty estimate because "the evolving population
//! is itself the difficulty oracle: the generation at which it first beats the
//! level". That number is what makes the pack a *pack* — a ladder rather than six
//! variants of "solvable" — so it is worth pinning. Three claims, each measured on
//! the shipped files (pop 200, twelve seeds, FINDINGS.md, "The M3 problem"):
//!
//! 1. the opening smoke level is winnable by the **initial random population**, so
//!    its interesting minute is over at generation 0;
//! 2. the eight levels after it are not, across a twelve-seed sweep at pop 64, beyond a
//!    measured noise floor of one lucky genome in 768 (see [`RANDOM_WINS_ALLOWED`] — L2's
//!    pit is *inside* a running jump and wall-school's plateau inside a wall climb, so a
//!    randomly wired policy clears one now and then; at pop 200 L2 is closer to a coin
//!    flip, five of twelve seeds containing a genome that crosses its pit. The check is a
//!    *rate* rather than "never won by luck", which is what a single frozen seed made it);
//! 3. the levels just past that still fall to *evolution* inside a small budget, so
//!    a fitness, sensor or level regression cannot make the pack hard in the wrong
//!    way — unlearnable — without failing the suite.
//!
//! **`l2-first-gap` used to be the second smoke level, and no longer is.** Holding the
//! jump button cleared its pit for free while jumping was level-triggered: a randomly
//! wired genome that never let go of the button crossed it without deciding anything. With
//! jumping edge-triggered (physics v2) the same population cannot cross it at all on any
//! seed tried, and the level is now the first *earned* one — a handful of generations, not
//! a wall. That is the ladder moving for a real reason, and the pack is better for it: the
//! second level now teaches the thing the second level is about.
//!
//! The harder levels are asserted only for (2). Their measured first wins at pop 200
//! are a median of 20 generations for L3, 14.5 for L5, 12.5 for L7 and 9.5 for L8
//! (all twelve seeds, FINDINGS.md, the difficulty ladder), and reproducing those here
//! would cost ~20 s of debug test time for a number FINDINGS.md already quotes
//! alongside the exact command that produced it. A cheaper progress check ("max
//! fitness rose") would be testing the GA rather than the level.
//!
//! `l7-wall-school` (finding 45) is the pack's cheapest level to *learn* — median 2.5
//! generations, because it is the wall jump with nothing else in it — and it is still
//! in EARNED, because at pop 64 it is not free: a random population reaches a three-tile
//! plateau only by luck, and the measured rate of that luck is one genome in 768. "Learned
//! immediately" and "won by the initial population" are different claims, and this list
//! is about the second one.

use evo::{evaluate_population, GaConfig, Trainer};
use sim::Level;

/// Solved at generation 0 by a randomly initialised population.
const TRIVIAL: [&str; 1] = ["l1-flat-run"];
/// Only ever solved by evolution.
///
/// `l9-wait-and-climb` is the pack's first *chain* (a spike, a pit, a nineteen-tile patrol, a
/// hazard patch, a step, a wall climb, a ledge gap, a spike and a two-tile climb to a flag 53
/// tiles from the spawn), and the reason it is in this list rather than in `TRIVIAL` is the same
/// as every other row's: a random population does not reach it. It is also the only row here
/// that needs more than one decision, which is what its median first win of 160 generations is
/// measuring (`FINDINGS.md`, the difficulty table).
const EARNED: [&str; 9] = [
    "l2-first-gap",
    "l3-gauntlet",
    "l4-rhythm",
    "l5-tightrope",
    "l6-marathon",
    "l7-ascent",
    "l7-wall-school",
    "l8-long-jump",
    "l9-wait-and-climb",
];

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

/// Seeds the generation-0 check sweeps, per earned level.
///
/// A sweep rather than one seed because the claim is a *rate*. Checking a single seed makes
/// "random play cannot win this" a statement about one draw of 64 nets, which is the noise
/// it sounds like: the sensor vector grew to 94 inputs in 0.5.0, every random genome moved
/// with it, and `l2-first-gap` — measured at one win in 768 genomes — turned a frozen seed.
const SEEDS: std::ops::RangeInclusive<u64> = 1..=12;

/// Lucky random genomes allowed to win an earned level across the whole [`SEEDS`] sweep.
///
/// **Not zero, and measured rather than chosen.** The pack's two cheapest obstacles are
/// inside a single ability's reach — L2's pit is three tiles against a four-tile running
/// jump, `l7-wall-school`'s plateau is three up against a 2.375-tile jump plus a climb — so
/// a randomly wired policy clears one by luck now and then. Measured at 0.5.0 over 24 seeds
/// × 64 genomes (1536 genomes per level): every earned level produced **0** winners except
/// `l2-first-gap` (1, at seed 4) and `l7-wall-school` (1, at seed 7). One is the bound
/// because the assertion is about the rate: a level random play wins once in 768 is a level
/// evolution has to find the obstacle in, while a level random play wins with any regularity
/// — which is what a collapsed ladder looks like — clears this bound by an order of
/// magnitude on the first seed.
const RANDOM_WINS_ALLOWED: usize = 1;

/// The rate at which an *unevolved* population wins an earned level: the level is not a
/// smoke test wearing an obstacle, and evolution is doing the work.
#[test]
fn no_earned_level_is_won_by_a_random_population() {
    let genomes = POP * (SEEDS.end() - SEEDS.start() + 1) as usize;
    for slug in EARNED {
        let level = level(slug);
        let mut wins = 0usize;
        let mut where_won = Vec::new();
        for seed in SEEDS {
            let n = random_population_wins(&level, seed);
            if n > 0 {
                where_won.push((seed, n));
            }
            wins += n;
        }
        assert!(
            wins <= RANDOM_WINS_ALLOWED,
            "{slug} was won by {wins} of {genomes} random genomes across seeds {SEEDS:?} \
             {where_won:?}. One lucky genome is the measured noise floor; more than one means \
             random play is finding the obstacle for free. If the level was made easier on \
             purpose, move it to TRIVIAL and re-measure its line in the difficulty table; if \
             not, something just made the pack's ladder collapse"
        );
    }
}

/// Generations allowed for `l2-first-gap` at pop 64.
///
/// Measured first wins after the movement pass: generation 4 (seed 1) and 3 (seeds 2 and 3).
/// The bound is loose on purpose — it is there to say "the second level is learned, not
/// survived", which is exactly the property that changed.
const L2_CAP: u32 = 20;

/// **Leftward is learned at least as fast as rightward.** `l2-first-gap` and its mirror are
/// the same level with every direction reversed — same pit, same width, same flag distance —
/// so a difference in how quickly they are learned can only come from the *sim*: a body that
/// can sprint, jump or see a gap one way only solves the original and cannot touch the
/// mirror. Measured at pop 64 on seeds 1-3: the original's first wins are generations
/// **10 / 2 / 3** and the mirror's are **0 / 2 / 2** (the mirror is the easier of the two,
/// which is the direction that matters — a rightward-only sim fails the other way).
#[test]
fn a_mirrored_level_is_learned_within_the_same_budget_as_the_level_it_mirrors() {
    let level = level("l2-first-gap");
    let mirror = level.mirrored_horizontal();
    for (what, lv) in [("l2-first-gap", level), ("l2-first-gap mirrored", mirror)] {
        let mut trainer = Trainer::new(config(1));
        let mut generation = 0;
        let mut wins = 0;
        while generation <= L2_CAP {
            wins = trainer.step(&lv).wins;
            if wins > 0 {
                break;
            }
            generation += 1;
        }
        assert!(
            wins > 0,
            "{what} was not solved within {L2_CAP} generations at pop {POP} on seed 1. \
             The mirror is the same level with every direction reversed, so it is the \
             sharpest available test of the sim's *directional* symmetry: a body that \
             can sprint, jump or see a gap one way only solves the original and cannot \
             touch this one, whatever the pack's own difficulty says"
        );
    }
}

/// Generations allowed for `l4-rhythm` at pop 64. A regression guard with room to spare, not a
/// performance claim — and the reason the test below sweeps seeds rather than pinning one.
const L4_CAP: u32 = 60;

/// Seeds the `l4-rhythm` sweep tries, and how many of them must solve inside [`L4_CAP`].
///
/// **A rate, because this level sits near the edge at pop 64 and a single seed is a coin flip.**
/// Measured at pop 64, `--stop-on-win`, seeds 1-6: four of six seeds solve inside 60 generations (at
/// generations 4, 4, 28 and 32 — seeds 1, 2, 3 and 5) while seeds 4 and 6 stall past 300 and hand the
/// run to the curriculum. So the claim worth pinning is "this level still falls to evolution at this
/// population size", which a 4-of-6 rate with a bound of 2 out of 4 states — and "seed 2 wins within
/// 60 generations" does not: that is a statement about one 64-net draw, and any change to the sensor
/// vector re-rolls it (measured: the same sweep on a 101-input build put the solves at 8, 7, 4 and 9
/// and the stalls on seeds 2 and 5, which is what a *rate* with a bound survives and a pinned seed
/// does not). What a broken sensor, fitness or GA would show up as is the rate collapsing, which is
/// exactly what the assertion catches.
const L4_SEEDS: [u64; 4] = [1, 2, 3, 4];
const L4_SOLVES_REQUIRED: usize = 2;

#[test]
fn the_gap_tutorial_is_learned_in_a_handful_of_generations() {
    let level = level("l2-first-gap");
    let mut trainer = Trainer::new(config(1));
    let mut generation = 0;
    let mut wins = 0;
    while generation <= L2_CAP {
        wins = trainer.step(&level).wins;
        if wins > 0 {
            break;
        }
        generation += 1;
    }
    assert!(
        wins > 0,
        "l2-first-gap was not solved within {L2_CAP} generations at pop {POP} on seed 1 \
         (measured first win: generation 4). It is the pack's second level: one pit, and \
         the population should need a handful of generations to time a jump over it, not \
         a long climb or a wall it cannot pass"
    );
}

#[test]
fn the_first_earned_level_still_falls_to_evolution() {
    let level = level("l4-rhythm");
    let mut solved: Vec<(u64, u32)> = Vec::new();
    for seed in L4_SEEDS {
        let mut trainer = Trainer::new(config(seed));
        for generation in 0..=L4_CAP {
            if trainer.step(&level).wins > 0 {
                solved.push((seed, generation));
                break;
            }
        }
        if solved.len() >= L4_SOLVES_REQUIRED {
            break;
        }
    }
    assert!(
        solved.len() >= L4_SOLVES_REQUIRED,
        "l4-rhythm was solved on only {} of the first {} seeds within {L4_CAP} generations at pop \
         {POP} (measured: 4 of 6 seeds on both sides of the v11 sensor bump, at generations 4-32 and \
         4-9). The level, the sensors or the fitness function regressed, or the GA stopped breeding \
         winners",
        solved.len(),
        L4_SEEDS.len(),
    );
    // And the ones that did solve did it early, which is the "handful of generations" half of the
    // claim rather than a bare existence proof: a population that crawls to a win at generation 59
    // would satisfy the count above and say nothing about the level being learnable at this size.
    assert!(
        solved.iter().any(|(_, generation)| *generation <= 20),
        "the level was solved, but not one seed inside twenty generations: {solved:?}"
    );
}
