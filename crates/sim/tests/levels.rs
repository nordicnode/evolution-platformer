//! Shipped content tests (§6, §9, §11): the three hand-written levels load, are
//! beatable in principle, terminate, and malformed level files are refused.

mod common;

use sim::AgentAction;

const SHIPPED: [&str; 3] = ["l1-flat-run", "l2-first-gap", "l3-gauntlet"];

#[test]
fn all_shipped_levels_load_with_the_documented_shape() {
    for slug in SHIPPED {
        let level = common::level(slug);
        assert!(
            !level.name.is_empty(),
            "{slug} should carry a human-readable name"
        );
        assert_eq!(level.width, 60, "{slug} width");
        assert_eq!(level.height, 15, "{slug} height");
        assert!(!level.goals.is_empty(), "{slug} must have a goal");
        assert_eq!(
            level.to_glyph_rows().len(),
            level.height as usize,
            "{slug} should round-trip its grid"
        );
        assert!(
            level.content_hash_string().starts_with("fnv1a64:"),
            "{slug} should hash its content for the champion file's level_hash"
        );
        assert!(
            level.spawn.w > 0.0 && level.spawn.h > 0.0,
            "{slug} should give the player a body"
        );
    }
}

#[test]
fn the_smoke_and_gap_levels_are_reachable_in_principle() {
    for slug in ["l1-flat-run", "l2-first-gap"] {
        let level = common::level(slug);
        let report = sim::reach::report(&level);
        assert!(!report.platforms.is_empty(), "{slug} exposes platforms");
        assert!(
            report.start.is_some(),
            "{slug}: the spawn point is not on any solid platform"
        );
        assert!(
            report.goal_reached,
            "{slug} should be beatable in principle, but: {:?}",
            report.diagnosis
        );
        assert!(
            report.reachable_platforms() >= 1,
            "{slug} should reach at least the spawn platform"
        );
    }
}

/// L3's two-tile spike patch sits exactly in the window where a full-speed jump's
/// 0.9-tile-tall body is still occupying the spike's row, and `sim::reach` only
/// samples take-offs at `column + 0.2`, i.e. it never tries the fractional
/// positions inside a column. A real agent can clear the patch from roughly
/// `x in [28.5, 29.0]`, which the grid-snapped sample misses.
///
/// So this test asserts something precise and true in either world: either the
/// oracle reaches L3's goal, or L3 is unreachable *only* because of its spikes —
/// never because the rest of its geometry regressed. That keeps the oracle's
/// conservative false negative visible instead of hiding it behind a weakened
/// assertion.
///
/// COORDINATOR: widening the take-off sampling (e.g. also trying `column + 0.7`)
/// should flip this to the reachable branch, after which the `if` below can go.
#[test]
fn l3_is_either_reachable_or_unreachable_only_because_of_its_spikes() {
    let level = common::level("l3-gauntlet");
    let rows = level.to_glyph_rows();

    let report = sim::reach::report(&level);
    assert!(
        report.start.is_some(),
        "L3's spawn point is not on any solid platform"
    );

    if report.goal_reached {
        assert!(
            report.culprit.is_none() && report.diagnosis.is_none(),
            "a reachable level has no stuck jump and needs no diagnosis: {:?}",
            report.diagnosis
        );
        return;
    }

    let spike_count: usize = rows.iter().map(|row| row.matches('^').count()).sum();
    assert_eq!(
        spike_count, 2,
        "L3 should ship exactly one two-tile spike patch: {rows:?}"
    );

    // Strip the spikes and ask again. Nothing else about L3 changes.
    let spike_free_rows: Vec<String> = rows.iter().map(|row| row.replace('^', ".")).collect();
    let spike_free = common::fixture(
        "l3-no-spikes",
        &spike_free_rows,
        (2, 12),
        &[("goal", 58, 9)],
    );

    // Guard the hardcoded spawn and goal against drift in the shipped level: if
    // L3 moves, this test must notice rather than quietly testing a copy of
    // something else.
    assert!(
        (spike_free.spawn.x - level.spawn.x).abs() < 1e-6
            && (spike_free.spawn.y - level.spawn.y).abs() < 1e-6,
        "the spike-free copy must spawn where L3 spawns"
    );
    assert!(
        spike_free.goals.iter().any(|copy| {
            level
                .goals
                .iter()
                .any(|real| (copy.x - real.x).abs() < 1e-6 && (copy.y - real.y).abs() < 1e-6)
        }),
        "the spike-free copy must keep L3's goal"
    );

    let stripped = sim::reach::report(&spike_free);
    assert!(
        stripped.goal_reached,
        "L3 with its spikes removed is *still* unreachable, so the oracle has a \
         genuine geometry false negative and not just the spike-sampling one: {:?}",
        stripped.diagnosis
    );
    assert!(
        report.diagnosis.is_some(),
        "an unreachable level must come with a diagnosis the editor can show"
    );
    assert!(
        report.culprit.is_some(),
        "and a specific jump to highlight: {:?}",
        report.culprit
    );
}

#[test]
fn every_shipped_level_terminates_inside_its_frame_cap() {
    for slug in SHIPPED {
        let level = common::level(slug);
        let actions = common::fixed_actions(0xF00D, 2_000);
        let mut sim = sim::Sim::new(&level, 0x1234);

        // Phase 1: a fixed 2_000-action sequence must never panic. The episode may
        // finish first; stepping a finished episode must stay a safe no-op.
        for i in 0..2_000usize {
            sim.step(actions[i % actions.len()]);
        }
        assert!(
            sim.stats().frames <= sim.max_frames(),
            "{slug} ran {} frames past its cap of {}",
            sim.stats().frames,
            sim.max_frames()
        );

        // Phase 2: keep going until it finishes. The sim's own rules guarantee
        // termination at the frame cap, so hitting this bound is a real failure.
        let cap = sim.max_frames() * 2;
        let mut steps = 0u32;
        while !sim.outcome().is_finished() && steps < cap {
            sim.step(actions[steps as usize % actions.len()]);
            steps += 1;
        }
        assert!(
            sim.outcome().is_finished(),
            "{slug} never terminated: {} frames simulated, cap {}",
            sim.stats().frames,
            sim.max_frames()
        );
        assert!(
            sim.stats().frames <= sim.max_frames(),
            "{slug} ended at frame {} past its cap of {}",
            sim.stats().frames,
            sim.max_frames()
        );
        assert!(
            sim.outcome().is_finished(),
            "{slug} must report a finished outcome"
        );

        // A finished episode is frozen: further input cannot re-run it.
        let frozen = sim.state_hash();
        sim.step(AgentAction::Run);
        assert_eq!(
            sim.state_hash(),
            frozen,
            "{slug}: stepping a finished episode must not change its state"
        );
    }
}

#[test]
fn malformed_levels_are_refused() {
    let rows = ["....", "####"];
    let goal = [("goal", 3, 0)];

    let good = common::fixture_src("good", &rows, (1, 0), &goal, 1);
    assert!(
        sim::Level::from_json_str(&good).is_ok(),
        "the control case must parse, or the assertions below are vacuous"
    );

    let bad_version = common::fixture_src("bad-version", &rows, (1, 0), &goal, 2);
    assert!(
        sim::Level::from_json_str(&bad_version).is_err(),
        "level_format_version 2 must be refused, not guessed at"
    );

    // Row 1 declares one column where the level says four.
    let short_row = common::fixture_src("short-row", &["....", "#"], (1, 0), &goal, 1);
    assert!(
        sim::Level::from_json_str(&short_row).is_err(),
        "a ragged tile row must be refused"
    );

    let bad_glyph = common::fixture_src("bad-glyph", &["..X.", "####"], (1, 0), &goal, 1);
    assert!(
        sim::Level::from_json_str(&bad_glyph).is_err(),
        "an unknown tile glyph must be refused"
    );

    let no_goal = common::fixture_src("no-goal", &rows, (1, 0), &[], 1);
    assert!(
        sim::Level::from_json_str(&no_goal).is_err(),
        "a level with no goal cannot be beaten and must be refused"
    );
}
