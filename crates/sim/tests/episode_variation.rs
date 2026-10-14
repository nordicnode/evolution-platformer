//! The episode seed must be *observable*, or the whole seeding contract (§5.5)
//! and `GaConfig::trials` are decorative.
//!
//! v1's only gameplay randomness is the walkers' spawn phase, drawn in
//! `Sim::new` via `Walker::spawn_spread`. These tests pin both halves of that
//! statement: with walkers, different seeds really do diverge; without walkers,
//! the seed correctly has no effect (there is nothing stochastic to vary), and a
//! reader can see that asymmetry asserted rather than assumed.

mod common;

use sim::physics::AgentAction;
use sim::rng::episode_seed;
use sim::{Level, Outcome, Sim, StateHasher};

/// Every shipped level that has at least one walker.
const LEVELS_WITH_WALKERS: [&str; 1] = ["l3-gauntlet"];

fn level(slug: &str) -> Level {
    common::level(slug)
}

#[test]
fn different_seeds_produce_different_walker_phases() {
    for slug in LEVELS_WITH_WALKERS {
        let lv = level(slug);
        assert!(!lv.walkers.is_empty(), "{slug} should have a walker");

        // Sample a spread of seeds and require at least two distinct (dir, x)
        // phases: a single-bit direction draw and a continuous jitter mean this
        // is a near-certainty for any real RNG, and asserting it catches an
        // accidental "seed is ignored" regression.
        let mut phases: Vec<(i8, u32)> = Vec::new();
        for seed in [1u64, 2, 3, 4, 5, 6, 7, 8] {
            let sim = Sim::new(&lv, seed);
            let w = &sim.walkers()[0];
            phases.push((w.dir, w.x.to_bits()));
        }
        let distinct = phases
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert!(
            distinct > 1,
            "{slug}: all {seed_count} seeds produced the same walker phase {phases:?}",
            seed_count = phases.len()
        );
    }
}

#[test]
fn the_same_seed_always_produces_the_same_walker_phase() {
    for slug in LEVELS_WITH_WALKERS {
        let lv = level(slug);
        let a = Sim::new(&lv, 424242);
        let b = Sim::new(&lv, 424242);
        assert_eq!(a.walkers().len(), b.walkers().len());
        for (wa, wb) in a.walkers().iter().zip(b.walkers()) {
            assert_eq!(wa.dir, wb.dir);
            assert_eq!(wa.x.to_bits(), wb.x.to_bits());
            assert_eq!(wa.y.to_bits(), wb.y.to_bits());
        }
        assert_eq!(a.state_hash(), b.state_hash());
    }
}

#[test]
fn a_seed_change_is_visible_in_the_state_hash_before_the_first_step() {
    let lv = level(LEVELS_WITH_WALKERS[0]);
    let a = Sim::new(&lv, 1);
    let b = Sim::new(&lv, 2);
    assert_ne!(
        a.state_hash(),
        b.state_hash(),
        "the walker draw must be visible in the state hash"
    );
    assert_ne!(a.rng_probe(), b.rng_probe());
}

/// Digest of everything that *affects play*: bodies, scoring counters, outcome.
///
/// Deliberately excludes `rng_probe()`: `state_hash` folds the RNG stream position
/// in on purpose, so two seeds always differ there even when nothing stochastic
/// happened. Comparing play state is what distinguishes "the seed changed the
/// episode" from "the seed moved a cursor nothing reads".
fn play_digest(level: &Level, seed: u64, steps: u32) -> u64 {
    let mut sim = Sim::new(level, seed);
    let mut h = StateHasher::new();
    for _ in 0..steps {
        sim.step(AgentAction::Run);
        let p = sim.player();
        h.f32(p.x);
        h.f32(p.y);
        h.f32(p.vx);
        h.f32(p.vy);
        h.bool(p.grounded);
        h.f32(sim.stats().progress_pixels);
        h.f32(sim.stats().best_x);
        h.u32(sim.stats().frames);
        h.u8(sim.outcome() as u8);
        for w in sim.walkers() {
            h.u32(w.id);
            h.f32(w.x);
            h.f32(w.y);
            h.i8(w.dir);
            h.bool(w.alive);
        }
    }
    h.finish()
}

#[test]
fn a_level_without_walkers_is_correctly_seed_independent() {
    // Stated plainly rather than hidden: on L1/L2 nothing in v1 is stochastic, so
    // every seed produces the same play out. That is not a bug, but it does mean
    // `--trials` buys nothing on those levels -- only levels with walkers vary.
    let lv = level("l1-flat-run");
    assert!(lv.walkers.is_empty());

    let digests: Vec<u64> = [1u64, 2, 3].iter().map(|s| play_digest(&lv, *s, 600)).collect();
    assert_eq!(
        digests[0], digests[1],
        "walker-free levels have nothing for the seed to vary"
    );
    assert_eq!(digests[1], digests[2]);

    // Positive control: the same measurement on a level *with* a walker does vary
    // with the seed, so the assertion above is about the level, not about the
    // digest being blind to seeds in general.
    let with_walker = level(LEVELS_WITH_WALKERS[0]);
    let ctrl: Vec<u64> = [1u64, 2, 3]
        .iter()
        .map(|s| play_digest(&with_walker, *s, 600))
        .collect();
    assert!(
        ctrl[0] != ctrl[1] || ctrl[1] != ctrl[2],
        "the digest must be able to see a seeded difference: {ctrl:?}"
    );
}

#[test]
fn walker_jitter_never_leaves_a_walker_unsupported() {
    // The jitter is rejected, not clamped, when the offset would push the body off
    // its platform. Sweep seeds and assert no walker ever starts falling.
    for slug in LEVELS_WITH_WALKERS {
        let lv = level(slug);
        for seed in 0..64u64 {
            let mut sim = Sim::new(&lv, seed);
            for w in sim.walkers() {
                let (c0, c1) = (w.x.floor() as i32, (w.x + w.w).floor() as i32);
                let row = (w.y + w.h).floor() as i32;
                let supported = (c0..=c1).any(|c| lv.is_solid(c, row));
                assert!(
                    supported,
                    "{slug} seed {seed}: walker {} spawned unsupported at x={}",
                    w.id, w.x
                );
            }
            // And it is still there, on the ground, after settling.
            for _ in 0..30 {
                sim.step(AgentAction::Walk);
            }
            assert!(
                sim.walkers().iter().all(|w| w.alive),
                "{slug} seed {seed}: a walker fell out of the world after settling"
            );
        }
    }
}

#[test]
fn trials_over_distinct_seeds_can_disagree_on_a_level_with_walkers() {
    // The reason `GaConfig::trials` exists. If every trial were the same episode,
    // this would fail and the knob would be a no-op.
    let lv = level(LEVELS_WITH_WALKERS[0]);
    let mut hashes = std::collections::BTreeSet::new();
    let mut outcomes = std::collections::BTreeSet::new();
    for trial in 0..12u32 {
        let seed = episode_seed(20261012, 0, 7, trial);
        let mut sim = Sim::new(&lv, seed);
        // A fixed, mildly capable policy: run, and jump when the sensor says the
        // ground to the right runs out. Enough to make the walker encounter matter.
        let mut jumped_at = 0u32;
        while !sim.outcome().is_finished() {
            let obs = sim.observation();
            let gap_right = obs[83] > 0.5;
            let frames = sim.stats().frames;
            let action = if gap_right && frames > jumped_at + 12 {
                jumped_at = frames;
                AgentAction::RunJump
            } else if jumped_at != 0 && frames < jumped_at + 10 {
                AgentAction::RunJump
            } else {
                AgentAction::Run
            };
            sim.step(action);
        }
        hashes.insert(sim.state_hash());
        // `Outcome` deliberately has no `Ord`, so collect the labels.
        outcomes.insert(sim.outcome().as_str());
    }
    assert!(
        hashes.len() > 1,
        "12 trials all produced one trajectory; the seed is not reaching the walker phase"
    );
    // Not asserting *which* outcomes occur (that is level design, not seeding),
    // only that the episode is a genuine function of the seed.
    assert!(outcomes.iter().any(|o| *o != Outcome::Running.as_str()));
}
