//! Shipped content tests (§6, §9, §11): the level pack loads, is beatable in
//! principle, terminates, keeps its entities on the map, and malformed level
//! files are refused.

mod common;

use sim::AgentAction;

/// The shipped pack: `(slug, width, height, walkers)`.
///
/// A table rather than a bare list, because a level's *geometry* is content. Width
/// sets the frame cap (`width × FRAMES_PER_TILE`), and every first-win number
/// quoted in the README was measured against exactly the level these numbers
/// describe, so a silent resize must fail here rather than quietly invalidate the
/// difficulty table.
///
/// The pack is ordered by measured difficulty (README, "the M3 problem"): L1 and
/// L2 are solved by the *initial random population*, L3–L6 need evolution.
const SHIPPED: [(&str, i32, i32, usize); 6] = [
    ("l1-flat-run", 60, 15, 0),
    ("l2-first-gap", 60, 15, 0),
    ("l3-gauntlet", 60, 15, 1),
    ("l4-rhythm", 67, 15, 0),
    ("l5-tightrope", 70, 15, 0),
    ("l6-marathon", 80, 15, 1),
];

#[test]
fn all_shipped_levels_load_with_the_documented_shape() {
    for (slug, width, height, walkers) in SHIPPED {
        let level = common::level(slug);
        assert!(
            !level.name.is_empty(),
            "{slug} should carry a human-readable name"
        );
        assert_eq!(level.width, width, "{slug} width");
        assert_eq!(level.height, height, "{slug} height");
        assert_eq!(level.walkers.len(), walkers, "{slug} walker count");
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
        assert!(
            !sim::reach::extract_platforms(&level).is_empty(),
            "{slug} should expose at least one platform"
        );
    }
}

/// Every level in the pack must be beatable in principle.
///
/// This used to be the interesting test: with a five-entry take-off sweep the
/// oracle lost L3, because its spike patch is only clearable from a fractional
/// position inside a column that a coarse sweep never sampled (see
/// `sim::reach::TAKEOFF_OFFSETS`). The sweep is fine-grained now, so the honest
/// assertion — "every shipped level is reachable, no caveats" — is also the
/// simplest one, and L3 is the regression test for the oracle's false negatives.
#[test]
fn every_shipped_level_is_reachable_in_principle() {
    for (slug, ..) in SHIPPED {
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
            report.culprit.is_none() && report.diagnosis.is_none(),
            "a reachable level has no stuck jump and needs no diagnosis: {:?}",
            report.diagnosis
        );
    }
}

/// Entities must lie on the map, and walkers must be standing on something.
///
/// This is the check that catches the authoring mistake this pack actually made:
/// a walker placed at `x = 84` in an 80-wide level is not refused by the loader —
/// it simply freezes in mid-air off the right edge, where it can neither be
/// stomped nor avoided, while still showing up in every agent's enemy sensors.
/// A level file that "loads fine" is not the same as a level file that is right.
#[test]
fn every_shipped_level_keeps_its_entities_inside_the_grid() {
    for (slug, ..) in SHIPPED {
        let level = common::level(slug);
        let (w, h) = (level.width as f32, level.height as f32);
        let bounds = |label: &str, r: &sim::Rect| {
            assert!(
                r.x >= 0.0 && r.x + r.w <= w && r.y >= 0.0 && r.y + r.h <= h,
                "{slug}: {label} at ({}, {}) sized {}×{} is outside the {w}×{h} grid",
                r.x,
                r.y,
                r.w,
                r.h
            );
        };
        for (i, goal) in level.goals.iter().enumerate() {
            bounds(&format!("goal {i}"), goal);
        }
        for (i, walker) in level.walkers.iter().enumerate() {
            bounds(&format!("walker {i}"), walker);
            let foot_row = (walker.y + walker.h).floor() as i32;
            let c0 = walker.x.floor() as i32;
            let c1 = (walker.x + walker.w - f32::EPSILON).floor() as i32;
            assert!(
                (c0..=c1).any(|c| level.is_solid(c, foot_row)),
                "{slug}: walker {i} at x={} row {foot_row} stands on nothing, \
                 so it will hang in the air for the whole episode",
                walker.x
            );
        }
    }
}

#[test]
fn every_shipped_level_terminates_inside_its_frame_cap() {
    for (slug, ..) in SHIPPED {
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

/// A walker out of the grid is *invisible to the reachability oracle* but *visible
/// to every agent*: the enemy sensor reports the nearest live walker at any
/// distance, clamped to the view radius. Both halves of that asymmetry are worth
/// pinning, because together they explain why the off-grid walker of the previous
/// test was worth a test in the first place.
#[test]
fn an_off_grid_walker_is_still_in_the_enemy_sensors() {
    let width = 24;
    let sky = ".".repeat(width);
    let ground = "#".repeat(width);
    let rows = vec![sky.clone(), sky.clone(), sky.clone(), ground];

    let inside = common::fixture(
        "walker-in-grid",
        &rows,
        (1, 2),
        &[("walker", 18, 2), ("goal", 22, 2)],
    );
    let outside = common::fixture(
        "walker-off-grid",
        &rows,
        (1, 2),
        &[("walker", 400, 2), ("goal", 22, 2)],
    );

    let enemy_slots = |level: &sim::Level| {
        let mut sim = sim::Sim::new(level, 7);
        sim.step(AgentAction::Walk);
        let obs = sim.observation();
        (obs[81], obs[82])
    };

    let near = enemy_slots(&inside);
    let far = enemy_slots(&outside);
    assert!(
        near.0 > 0.99 && far.0 > 0.99,
        "a walker 17 tiles away and one 400 tiles away both saturate the \
         normalised enemy dx slot: near {near:?} far {far:?}"
    );
    assert_eq!(
        near.1, far.1,
        "dy should not depend on how far to the right the walker is"
    );

    // The same level without any walker reads the slots as "no enemy".
    let none = common::fixture("no-walker", &rows, (1, 2), &[("goal", 22, 2)]);
    assert_eq!(
        enemy_slots(&none),
        (0.0, 0.0),
        "with no walker at all the enemy slots are zero"
    );
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
