//! Shipped content tests (§6, §9, §11): the level pack loads, is beatable in
//! principle, terminates, keeps its entities on the map, and malformed level
//! files are refused.

mod common;

use sim::AgentAction;

/// The shipped pack: `(slug, width, height, walkers)`.
///
/// A table rather than a bare list, because a level's *geometry* is content. Width
/// sets the frame cap (`width × FRAMES_PER_TILE`), and every first-win number
/// quoted in FINDINGS.md was measured against exactly the level these numbers
/// describe, so a silent resize must fail here rather than quietly invalidate the
/// difficulty table.
///
/// The pack is ordered by measured difficulty (FINDINGS.md, "The M3 problem"): L1 is solved
/// by the *initial random population*, L2–L8 need evolution. L7 and L8 are the movement
/// pass's two levels and are *about* their ability — see
/// [`the_ability_levels_require_their_ability`].
///
/// **L9 is the content pass's level**: the first one in the pack that is a *chain* rather than
/// an obstacle — a spike, a pit, a nineteen-tile walker patrol, a hazard patch, a two-tile step, a
/// three-tile wall climb, a gap in the ledge, another spike and a two-tile climb, with the spawn at
/// column 2 and the flag at column 56. It is here in the table with the rest because everything the
/// pack asserts about a level (its size, its one flag, its round trip, its mirror) has to hold for
/// it too.
const SHIPPED: [(&str, i32, i32, usize); 10] = [
    ("l1-flat-run", 60, 15, 0),
    ("l2-first-gap", 60, 15, 0),
    ("l3-gauntlet", 60, 15, 1),
    ("l4-rhythm", 67, 15, 0),
    ("l5-tightrope", 70, 15, 0),
    ("l6-marathon", 80, 15, 1),
    ("l7-ascent", 60, 15, 0),
    ("l7-wall-school", 60, 15, 0),
    ("l8-long-jump", 60, 15, 1),
    ("l9-wait-and-climb", 60, 15, 1),
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
        assert_eq!(level.goals.len(), 1, "{slug} must have exactly one flag");
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

/// The two levels the movement pass added are about their ability, and this is the test
/// that says so rather than the comment that claims it.
///
/// Each is reachable with the game as it is and *unreachable* with the one ability
/// switched off, which is the strongest available statement that the level teaches that
/// ability and that nothing else in the movement set quietly replaces it. It is also the
/// reason [`sim::reach::ReachOptions`] exists: a claim like this is only worth making if
/// the tool can be asked the question both ways.
#[test]
fn the_ability_levels_require_their_ability() {
    use sim::reach::{report, report_with, ReachOptions};

    // Two levels, one claim, checked the same way twice: the ascent (a move asked for twice)
    // and the wall school (the move on its own, finding 45).
    for slug in ["l7-ascent", "l7-wall-school"] {
        let level = common::level(slug);
        let with_walls = report(&level);
        assert!(
            with_walls.goal_reached,
            "{slug} must be beatable with wall jumps: {:?}",
            with_walls.diagnosis
        );
        let without_walls = report_with(
            &level,
            ReachOptions {
                wall_climb: false,
                ..ReachOptions::default()
            },
        );
        assert!(
            !without_walls.goal_reached,
            "{slug} is three tiles up — past the 2.375 a full jump reaches — so without wall \
             climbs the level must be unreachable; it is reachable another way, and the level \
             no longer teaches what it is named for"
        );
    }

    let long = common::level("l8-long-jump");
    let with_sprint = report(&long);
    assert!(
        with_sprint.goal_reached,
        "l8-long-jump must be beatable with sprint: {:?}",
        with_sprint.diagnosis
    );
    let without_sprint = report_with(
        &long,
        ReachOptions {
            sprint: false,
            ..ReachOptions::default()
        },
    );
    assert!(
        !without_sprint.goal_reached,
        "l8-long-jump's pits are five tiles and a running jump covers four, so without \
         sprints the level must be unreachable"
    );
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
            // The predicate is the editor's too, so a walker this test refuses is one the
            // editor will not let you place — the rule has one definition, not two.
            let foot_row = (walker.y + walker.h).floor() as i32;
            assert!(
                level.walker_stands_on_ground(walker.x.floor() as i32, foot_row - 1),
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

/// A walker out of the grid is *invisible to the reachability oracle* but *visible to every
/// agent* — bounded by [`sim::sensors::ENEMY_VIEW_DISTANCE`]. Both halves of that asymmetry
/// are worth pinning, because together they explain why the off-grid walker of the previous
/// test was worth a test in the first place.
///
/// A walker beyond the view distance is out of sight and reads `(0.0, 0.0)`, while a walker
/// inside it reports its own normalised offset — the offset, not a threshold, so the test
/// does not have to be rewritten every time the view distance is measured again.
#[test]
fn an_off_grid_or_distant_walker_is_out_of_sight_until_approached() {
    let width = 24;
    let sky = ".".repeat(width);
    let ground = "#".repeat(width);
    let rows = vec![sky.clone(), sky.clone(), sky.clone(), ground];

    let close = common::fixture(
        "walker-close",
        &rows,
        (1, 2),
        &[("walker", 4, 2), ("goal", 22, 2)],
    );
    let distant = common::fixture(
        "walker-distant",
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

    // The fixture stands the walker three tiles to the right of the body, so the reading is
    // that offset over the view distance.
    let near = enemy_slots(&close);
    assert!(
        (near.0 - 3.0 / sim::sensors::ENEMY_VIEW_DISTANCE).abs() < 0.05 && near.0 > 0.0,
        "a walker 3 tiles away is inside ENEMY_VIEW_DISTANCE and reads its offset over it: {near:?}"
    );
    assert_eq!(
        enemy_slots(&distant),
        (0.0, 0.0),
        "a walker 17 tiles away is beyond ENEMY_VIEW_DISTANCE and reads (0, 0)"
    );
    assert_eq!(
        enemy_slots(&outside),
        (0.0, 0.0),
        "an off-grid walker 400 tiles away is out of sight and reads (0, 0)"
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

    // A level has one flag: two entities, or a 'G' tile beside one, are a file that cannot
    // say which flag the population is meant to reach. The editor can hold a document with
    // no flag mid-edit, but a level *on disk* is exactly one.
    let two_goals = common::fixture_src(
        "two-goals",
        &rows,
        (1, 0),
        &[("goal", 3, 0), ("goal", 2, 0)],
        1,
    );
    assert!(
        sim::Level::from_json_str(&two_goals).is_err(),
        "a second goal entity must be refused, not silently dropped"
    );

    let goal_entity_and_tile = common::fixture_src("goal-and-G", &rows, (1, 0), &goal, 1)
        .replace("\"....\"", "\"G...\"");
    assert!(
        sim::Level::from_json_str(&goal_entity_and_tile).is_err(),
        "a 'G' tile beside a goal entity is a second goal"
    );
}

/// The mirror of every shipped level is beatable in principle too — and that is a claim about
/// the *sim*, not about the pack.
///
/// The pack is entirely rightward: every level is entered at its left and its flag is
/// somewhere to the right. That is a fine thing for a level author to do and a dangerous
/// thing to *only* do, because a capability that exists in one direction looks complete
/// while nothing exercises the other — the failure is invisible until somebody builds a
/// leftward level. It has already happened once: the action table had no leftward sprint, so
/// the mirror of `l8-long-jump` — the very level that teaches sprint — was reported
/// unbeatable by this oracle, which could not express the move either. Every shipped level
/// is therefore checked twice here, and the mirrored half is the half that catches a
/// one-directional `physics`, `reach` or level-editing rule.
#[test]
fn every_shipped_levels_mirror_is_reachable_in_principle() {
    for (slug, width, height, _) in SHIPPED {
        let level = common::level(slug);
        let mirror = level.mirrored_horizontal();

        assert_eq!(mirror.width, width, "{slug}: a mirror is the same size");
        assert_eq!(mirror.height, height, "{slug}: a mirror is the same size");
        assert_eq!(
            mirror.goals.len(),
            1,
            "{slug}: a mirrored level still holds exactly one flag"
        );
        let (sx, sy) = level.spawn_tile();
        assert_eq!(
            mirror.spawn_tile(),
            (width - 1 - sx, sy),
            "{slug}: the mirrored spawn is the spawn's own column, reflected"
        );

        let report = sim::reach::report(&mirror);
        assert!(
            report.goal_reached,
            "{slug}'s mirror should be beatable in principle, but: {:?}. A level that is \
             beatable only left-to-right is a capability missing from the sim, not a level \
             design (the leftward sprint was exactly this)",
            report.diagnosis
        );
        assert!(
            report.culprit.is_none(),
            "{slug}'s mirror is beatable, so it has no stuck jump to blame: {:?}",
            report.culprit
        );
    }
}

/// Sand is a material, not a mechanic: the same level in sand simulates identically.
///
/// This is the whole of what `Tile::Sand` promises, and it is worth a test because the
/// promise is what makes the material *safe*: a level author can paint a beach wherever a
/// floor would go and the level's difficulty is unchanged. Three claims, in order of
/// strength:
///
/// * the glyph round-trips, so opening and saving a level cannot turn sand into soil — the
///   one failure a rendering test could never see;
/// * sand is solid, so a body stands on it;
/// * and two levels that differ *only* in which of the two glyphs their floor is written in
///   produce the same state hash and the same observation on every one of four hundred
///   ticks. The second is the half that matters for evolution: the population sees sand as
///   it sees soil, so a beach is not a new problem to learn.
#[test]
fn sand_is_a_material_and_not_a_mechanic() {
    // The glyph round trip, including that sand, brick, plank and soil are different glyphs.
    for tile in [
        sim::Tile::Empty,
        sim::Tile::Solid,
        sim::Tile::Sand,
        sim::Tile::Brick,
        sim::Tile::Plank,
        sim::Tile::Spike,
    ] {
        assert_eq!(
            sim::Tile::from_glyph(tile.glyph()),
            Some(tile),
            "the glyph {:?} does not read back as the tile it came from",
            tile.glyph()
        );
    }
    assert_ne!(sim::Tile::Sand.glyph(), sim::Tile::Solid.glyph());
    assert_ne!(sim::Tile::Brick.glyph(), sim::Tile::Solid.glyph());
    assert_ne!(sim::Tile::Plank.glyph(), sim::Tile::Solid.glyph());
    assert!(sim::Tile::Sand.is_solid(), "sand has to hold a body up");
    assert!(sim::Tile::Brick.is_solid(), "brick has to hold a body up");
    assert!(sim::Tile::Plank.is_solid(), "plank has to hold a body up");

    let sky = "....".to_string();
    let rows = vec![sky.clone(), sky.clone(), "#SBW".to_string()];
    let level = common::fixture("beach", &rows, (1, 1), &[("goal", 3, 1)]);
    assert_eq!(level.tile(0, 2), Some(sim::Tile::Solid));
    assert_eq!(level.tile(1, 2), Some(sim::Tile::Sand));
    assert_eq!(level.tile(2, 2), Some(sim::Tile::Brick));
    assert_eq!(level.tile(3, 2), Some(sim::Tile::Plank));
    let round_tripped = sim::Level::from_json_str(
        &level.to_json_pretty().expect("a level serializes"),
    )
    .expect("and reads back");
    assert_eq!(
        round_tripped.content_hash(),
        level.content_hash(),
        "materials did not survive a save and a load"
    );

    // And the same level, with only the floor's material changed.
    let soil = common::fixture("soil-floor", &[sky.clone(), sky.clone(), "####".into()], (1, 1), &[("goal", 3, 1)]);
    let sand = common::fixture("sand-floor", &[sky.clone(), sky.clone(), "SSSS".into()], (1, 1), &[("goal", 3, 1)]);
    let brick = common::fixture("brick-floor", &[sky.clone(), sky.clone(), "BBBB".into()], (1, 1), &[("goal", 3, 1)]);
    let plank = common::fixture("plank-floor", &[sky.clone(), sky.clone(), "WWWW".into()], (1, 1), &[("goal", 3, 1)]);
    let mut a = sim::Sim::new(&soil, 1);
    let mut b = sim::Sim::new(&sand, 1);
    let mut c = sim::Sim::new(&brick, 1);
    let mut d = sim::Sim::new(&plank, 1);
    let actions = common::forward_actions(7, 400);
    for (tick, action) in actions.iter().enumerate() {
        a.step(*action);
        b.step(*action);
        c.step(*action);
        d.step(*action);
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "the two floors diverge at tick {tick}: sand is not behaving as soil"
        );
        assert_eq!(a.state_hash(), c.state_hash(), "brick diverges at tick {tick}");
        assert_eq!(a.state_hash(), d.state_hash(), "plank diverges at tick {tick}");
        assert_eq!(
            a.observation(),
            b.observation(),
            "an agent can tell sand from soil at tick {tick}, which makes it a new problem \
             to learn rather than a different colour of ground"
        );
        assert_eq!(a.observation(), c.observation());
        assert_eq!(a.observation(), d.observation());
    }
    assert_eq!(a.outcome(), b.outcome());
    assert!(a.player().grounded, "the body should be standing on the floor");
}
