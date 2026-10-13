//! The determinism contract (§5, §11).
//!
//! `determinism_hash` is described in the plan as "the project's spine". These
//! tests run on the long runway fixture rather than on a shipped level, because a
//! 60-tile level terminates in ~600–2400 frames — too soon to exercise a
//! 10_000-frame budget. The runway's width puts its frame cap at 80_000 and its
//! goal ~19_900 ticks away, so a run really does simulate every requested frame.

mod common;

use sim::AgentAction;

const SEED: u64 = 0xA11CE;

#[test]
fn the_same_seed_and_actions_produce_the_same_state_hash() {
    let level = common::long_runway();
    let actions = common::forward_actions(0x5EED, 10_000);

    let mut a = sim::Sim::new(&level, SEED);
    let mut b = sim::Sim::new(&level, SEED);

    for (frame, action) in actions.iter().enumerate() {
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "state hash diverged before frame {frame}"
        );
        a.step(*action);
        b.step(*action);
    }

    assert_eq!(a.state_hash(), b.state_hash(), "final state hash");
    assert_eq!(a.outcome(), b.outcome());
    assert_eq!(a.stats().frames, b.stats().frames);
    assert_eq!(a.stats().progress_pixels, b.stats().progress_pixels);
    assert_eq!(a.rng_probe(), b.rng_probe());

    // The episode must actually have been simulated for the whole budget, or the
    // comparison above proves very little.
    assert_eq!(
        a.stats().frames,
        10_000,
        "the fixture should keep the episode running for the whole budget, but it \
         finished as {:?} at frame {}",
        a.outcome(),
        a.stats().frames
    );
    assert_eq!(
        a.outcome(),
        sim::Outcome::Running,
        "the long runway agent should still be running at 10_000 frames"
    );
    assert!(
        a.stats().progress_pixels > 0.0,
        "the runway agent should have covered ground"
    );
}

/// Pinned hash for the canonical long-runway episode.
///
/// This is the project's spine (§11): any change to physics, tile semantics,
/// hazard handling, or episode termination must be a deliberate `sim_version`
/// bump plus an update to this constant, never a silent drift. The *stronger*
/// check is the un-ignored run-twice equality above; this one additionally pins
/// the value against the whole codebase's history.
#[test]
fn golden_state_hash_after_ten_thousand_frames() {
    // Recorded on x86-64 with rustc 1.98, no fast-math. The sim uses no libm
    // transcendentals, so this is stable across native builds on one target; it
    // is *not* promised to match a wasm32 build (§5.2 accepts that).
    const GOLDEN_LONG_RUNWAY_HASH: u64 = 1_466_572_030_440_511;

    let level = common::long_runway();
    let mut sim = sim::Sim::new(&level, SEED);
    for action in common::forward_actions(0x5EED, 10_000) {
        sim.step(action);
    }

    assert_eq!(
        sim.state_hash(),
        GOLDEN_LONG_RUNWAY_HASH,
        "the state hash after 10_000 frames drifted; if that was intentional, bump \
         SIM_VERSION and update this constant"
    );
}

#[test]
fn progress_never_decreases_and_the_frame_cap_holds() {
    let level = common::long_runway();
    let actions = common::forward_actions(0xC0FFEE, 3_000);
    let mut sim = sim::Sim::new(&level, 7);

    let mut prev_progress = 0.0f32;
    let mut prev_best_x = level.spawn.x;
    let mut prev_frames = 0u32;

    for (frame, action) in actions.iter().enumerate() {
        sim.step(*action);
        let stats = sim.stats();

        assert!(
            stats.progress_pixels >= prev_progress,
            "progress went backwards at frame {frame}: {prev_progress} then {}",
            stats.progress_pixels
        );
        assert!(
            stats.best_x >= prev_best_x,
            "best_x went backwards at frame {frame}: {prev_best_x} then {}",
            stats.best_x
        );
        assert_eq!(
            stats.frames,
            prev_frames + 1,
            "frame {frame} should advance the counter by exactly one"
        );
        assert!(
            stats.frames <= sim.max_frames(),
            "frame {} exceeded the level's cap of {}",
            stats.frames,
            sim.max_frames()
        );

        prev_progress = stats.progress_pixels;
        prev_best_x = stats.best_x;
        prev_frames = stats.frames;
    }

    assert!(
        prev_progress > 0.0,
        "the runway agent should have made measurable progress"
    );
    assert_eq!(sim.stats().frames, 3_000);
}

#[test]
fn an_agent_that_never_moves_is_stopped_by_the_anti_camper_rule() {
    let level = common::long_runway();
    let mut sim = sim::Sim::new(&level, 3);

    // `Walk` applies no horizontal input at all, so the body stands still.
    let cap = sim.max_frames() + 8;
    let mut steps = 0;
    while !sim.outcome().is_finished() && steps < cap {
        sim.step(AgentAction::Walk);
        steps += 1;
    }

    assert_ne!(
        sim.outcome(),
        sim::Outcome::Running,
        "an episode must terminate: {} steps taken",
        steps
    );
    assert_eq!(
        sim.stats().frames,
        steps,
        "every step of a live episode should be counted"
    );
    assert_eq!(
        sim.outcome(),
        sim::Outcome::Stuck,
        "a do-nothing agent must be cut off by the no-progress rule, not run to \
         the {} frame cap",
        sim.max_frames()
    );
    assert!(
        sim.stats().frames <= sim::NO_PROGRESS_FRAMES,
        "the anti-camper rule should fire at {} frames, but the episode ran to {}",
        sim::NO_PROGRESS_FRAMES,
        sim.stats().frames
    );
    assert_eq!(
        sim.stats().progress_pixels,
        0.0,
        "a camper makes no progress"
    );
}

#[test]
fn a_finished_episode_is_frozen() {
    let level = common::long_runway();
    let mut sim = sim::Sim::new(&level, 5);
    common::step_n(&mut sim, AgentAction::Walk, sim::NO_PROGRESS_FRAMES + 4);
    assert!(sim.outcome().is_finished(), "the camper rule should have fired");

    let frozen_hash = sim.state_hash();
    let frozen_frames = sim.stats().frames;
    for action in AgentAction::ALL {
        sim.step(action);
    }
    assert_eq!(
        sim.state_hash(),
        frozen_hash,
        "stepping a finished episode must be a no-op, not a second episode"
    );
    assert_eq!(sim.stats().frames, frozen_frames);
}

#[test]
fn the_rng_probe_is_stable_and_seed_dependent() {
    let level = common::long_runway();

    let mut a = sim::Sim::new(&level, 0xBEEF);
    let mut b = sim::Sim::new(&level, 0xBEEF);

    assert_eq!(a.rng_probe(), b.rng_probe(), "same seed, same stream position");
    assert_eq!(
        a.rng_probe(),
        a.rng_probe(),
        "probing the RNG must not advance it"
    );
    assert_eq!(
        a.state_hash(),
        a.state_hash(),
        "state_hash must be pure: a probing side effect would make it drift"
    );

    for _ in 0..64 {
        a.step(AgentAction::Run);
        b.step(AgentAction::Run);
    }
    assert_eq!(a.rng_probe(), b.rng_probe());
    assert_eq!(a.state_hash(), b.state_hash());

    let c = sim::Sim::new(&level, 0xDEAD_BEEF);
    assert_ne!(
        a.rng_probe(),
        c.rng_probe(),
        "different episode seeds must produce different streams"
    );
    assert_ne!(
        a.state_hash(),
        c.state_hash(),
        "the episode seed must reach the state"
    );
}
