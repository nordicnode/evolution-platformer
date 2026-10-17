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
const SHIPPED: [(&str, i32, i32, usize); 12] = [
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
    // The demo for the enemies-and-objects pass: a one-way shelf, a flyer, a chaser, a
    // turret and a moving platform in one level. The walker count is the *patrols* the level
    // file lists, and this one has none: its chaser is a `chaser` entity, and the sim appends
    // chasers to its walker vector only once an episode starts.
    ("l10-the-hunt", 60, 15, 0),
    // The demo for the elements pass, and the first level in the pack with a tile that is not
    // ground in it: a lava pool that throws, a water basin two deep that has to be swum out of,
    // and a vine down the far wall of the plateau's pit. All three are on the route a body
    // walks; the pit's vine is one of *two* ways out of it, which is what keeps the level
    // reachable to an oracle that cannot climb (`sim::reach`).
    ("l11-the-elements", 72, 18, 0),
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

    // A version this build has never seen. Every older version is accepted on purpose (each
    // one is a strict subset of this one), so the refusal case is the *next* version rather
    // than the last one.
    let bad_version = common::fixture_src(
        "bad-version",
        &rows,
        (1, 0),
        &goal,
        sim::level::LEVEL_FORMAT_VERSION + 1,
    );
    assert!(
        sim::Level::from_json_str(&bad_version).is_err(),
        "a format version this build does not know must be refused, not guessed at"
    );

    for old in 1..sim::level::LEVEL_FORMAT_VERSION {
        let old_version = common::fixture_src("old-version", &rows, (1, 0), &goal, old);
        assert!(
            sim::Level::from_json_str(&old_version).is_ok(),
            "a v{old} file is a subset of v{} and must still load",
            sim::level::LEVEL_FORMAT_VERSION
        );
    }

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

/// The route table is one answer, shared by every copy of one terrain.
///
/// This is the difference between a window that runs and a window that stutters. `Level`
/// derives `Clone`, and a bare `OnceLock` is *copied* by `Clone`: a copy taken before the
/// first ask started with an empty cache and paid the whole sweep itself. Every copy of one
/// terrain exists at once in the app — the run's own document, the eight lanes animated over
/// it, each ghost in a replay, and every episode's sim inside the trainer — and the sweep is
/// 24 ms on a 60x15 level, so at eight or a hundred copies it was not a cost, it was the
/// frame budget spent several times over.
///
/// Two claims, and the second is the reason the first is safe: a clone reuses the graph built
/// for its terrain, and a level whose terrain has *changed* does not — it builds a graph for
/// the ground it has now, while the copies still holding the old ground keep the one that
/// describes them.
#[test]
fn a_route_table_is_shared_by_every_clone_of_one_terrain_and_dropped_by_an_edit() {
    let sky = "....".to_string();
    let level = common::fixture(
        "shared-routes",
        &[sky.clone(), sky.clone(), "####".into()],
        (1, 1),
        &[("goal", 3, 1)],
    );

    // **The copy is taken cold, before anything has asked for a route.** That order is the whole
    // of the bug this pins: a copy made *after* the first build inherits the finished graph even
    // with a bare `OnceLock`, so cloning afterwards would pass on the code that stuttered. Every
    // copy the window and the trainer make is made before the first ask.
    let cold = level.clone();
    let built = level.routes();
    assert!(
        std::ptr::eq(built, cold.routes()),
        "a copy taken before the first ask must reuse the one route table built for this \
         terrain, not sweep the whole level for a second one"
    );
    assert!(
        std::ptr::eq(built, level.clone().routes()),
        "and a copy taken after it must reuse it too"
    );
    assert!(
        std::ptr::eq(built, cold.clone().routes()),
        "nor must a copy of a copy"
    );

    // An edit is a different terrain: it gets its own graph, and the copies that still hold the
    // terrain the first graph was built for keep it.
    let mut edited = level.clone();
    assert!(edited.set_tile(1, 1, sim::Tile::Solid), "the edit lands on the grid");
    assert!(
        !std::ptr::eq(built, edited.routes()),
        "an edited level must not answer with a graph built for the ground it used to have"
    );
    assert!(
        std::ptr::eq(built, level.routes()),
        "and the level that was not edited must not lose the graph it already had"
    );
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
    // The glyph round trip, over **every tile the format has** (`Tile::ALL`, so a tile added to the
    // enum cannot quietly skip this). It matters most for the two trap tiles, because they are the
    // two a level file can *lose* on a save: a pad or a brittle tile that came back as plain ground
    // would be a level that plays differently from its file, and no rendering test could see it.
    for tile in sim::Tile::ALL {
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

    // The trap tiles, which are the two carve-outs from that rule and each for its own reason: a
    // brittle tile is ground until the sim says it is not, and a pad is deliberately not ground at
    // all — a body that could stand on one would never be thrown by it.
    for tile in [
        sim::Tile::Cracked,
        sim::Tile::CrackedSand,
        sim::Tile::CrackedBrick,
        sim::Tile::CrackedPlank,
    ] {
        assert!(tile.is_solid(), "{tile:?} is ground while it lasts");
        assert!(tile.is_cracked(), "{tile:?} is brittle");
        assert_ne!(tile.glyph(), tile.base_material().glyph());
        assert!(tile.base_material().is_solid(), "and its material is ground");
    }
    assert!(!sim::Tile::Spring.is_solid(), "a pad is not ground");
    assert!(sim::Tile::Spring.is_spring());
    assert_ne!(sim::Tile::Cracked.glyph(), sim::Tile::Solid.glyph());
    assert_ne!(sim::Tile::Spring.glyph(), sim::Tile::Cracked.glyph());

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

// ---------------------------------------------------------------------------
// The route cache beside a level file
// ---------------------------------------------------------------------------

/// A diagnosis no oracle would ever produce, written into a cache so that a *hit* is visible.
///
/// Timing a second load would be evidence about this machine; handing the cache an answer
/// nobody would compute and seeing it come back is evidence about the code, because the only
/// way out of the file is into the answer.
const SENTINEL: &str = "this verdict came out of the cache, not out of a sweep";

/// A private directory for one cache test, named after the test so the tests can run in
/// parallel without sharing a level file — or the sidecar derived from it.
fn cache_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("evo-route-cache-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// The level the cache tests are written against: two stands, a pit between them, the flag on
/// the far one.
///
/// Deliberately small — sweeping it is microseconds, so these tests can only be about *whether*
/// an answer was computed — and deliberately not one flat floor, so the route table is a graph
/// (two nodes, one crossing) rather than a single stand with nothing to say.
fn cache_fixture(name: &str) -> sim::Level {
    let sky = "..........".to_string();
    common::fixture(
        name,
        &[
            sky.clone(),
            sky.clone(),
            sky.clone(),
            "###...####".to_string(),
        ],
        (1, 2),
        &[("goal", 8, 2)],
    )
}

/// One row of a cache test's edit table: what the edit is called, and how to make it.
///
/// Named because clippy is right about the bare array type: `[(&str, fn(&mut sim::Level)); 5]`
/// says the shape twice and the meaning not at all.
type Edit = (&'static str, fn(&mut sim::Level));

/// Where a level's cache lives: its file's own name plus `.routes`.
///
/// Written out here rather than asked of the crate, deliberately: the name is part of the
/// contract these tests are about (the file has to sit beside the level and must not look like
/// one to the tools that list levels by extension), and a test that asks the implementation
/// where it put something can only ever agree with it.
fn sidecar_of(path: &std::path::Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.routes", path.display()))
}

/// Rewrite a cache file, so that a value out of it can be told from a value computed fresh.
fn poison(path: &std::path::Path, edit: impl Fn(&mut serde_json::Value)) {
    let src = std::fs::read_to_string(path).expect("the cache was written");
    let mut doc: serde_json::Value = serde_json::from_str(&src).expect("the cache is JSON");
    edit(&mut doc);
    std::fs::write(path, doc.to_string()).expect("the poisoned cache is written");
}

/// Poison both halves: a graph that calls every stand `u32::MAX` crossings from the flag (no
/// sweep produces that) and a verdict with a diagnosis no oracle says.
///
/// The two assertions at the end are not decoration: a poison that failed to land would make
/// every test built on it pass while proving nothing.
fn poison_answers(path: &std::path::Path) {
    poison(path, |doc| {
        doc["routes"]["hops"][0] = serde_json::json!(u32::MAX);
        doc["verdict"]["report"]["diagnosis"] = serde_json::json!(SENTINEL);
    });
    let poisoned = std::fs::read_to_string(path).expect("the cache reads back");
    assert!(
        poisoned.contains(&u32::MAX.to_string()) && poisoned.contains(SENTINEL),
        "the poison has to be in the file for a hit to be visible: {poisoned}"
    );
}

/// A level's answers are kept beside its file, and the next process reads them instead of
/// sweeping.
///
/// The cache appears with the first *answer*, not with the level, and a poisoned cache is
/// served whole: the route table through `routes()` and the verdict through `report()`, which
/// are the two asks that cost milliseconds on a real level.
#[test]
fn a_reopened_level_answers_from_the_cache_beside_its_file() {
    let dir = cache_dir("adopt");
    let path = dir.join("reopened.json");
    cache_fixture("reopened")
        .to_json_file(&path)
        .expect("the level is written");
    let sidecar = sidecar_of(&path);

    let cold = sim::Level::from_json_file(&path).expect("the level loads");
    assert!(
        !sidecar.exists(),
        "nothing is cached before anything has been asked: the file is derived from an answer"
    );
    let _ = cold.routes();
    assert!(
        sidecar.exists(),
        "the route table is kept beside the level file it describes"
    );
    let _ = sim::reach::report(&cold);
    poison_answers(&sidecar);

    let reopened = sim::Level::from_json_file(&path).expect("the level loads again");
    assert_eq!(
        reopened.routes().hops_to_flag(0),
        Some(u32::MAX),
        "the level answers with the route table beside its file, not with a second sweep"
    );
    assert_eq!(
        sim::reach::report(&reopened).diagnosis.as_deref(),
        Some(SENTINEL),
        "and with the verdict beside it, not with a second report"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A level that has *changed* is a different level, and answers for itself.
///
/// Every input the two answers are functions of is checked on its own, because a key can be
/// wrong in two directions and only one of them is cheap: a miss costs a sweep, while a hit for
/// a level the answers do not describe reads a jump that cannot be made off ground that is not
/// there. The poisoned cache is what tells the two apart — a correct answer here is only
/// evidence if a stale one would have been visibly wrong.
#[test]
fn a_changed_level_is_not_answered_by_the_cache_it_left_behind() {
    let dir = cache_dir("invalidated");
    let path = dir.join("edited.json");
    let original = cache_fixture("edited");
    let src = original.to_json_pretty().expect("the level serializes");
    original
        .to_json_file(&path)
        .expect("the level is written");
    let sidecar = sidecar_of(&path);

    let warm = sim::Level::from_json_file(&path).expect("the level loads");
    let _ = warm.routes();
    let _ = sim::reach::report(&warm);

    // The edits: a tile, the spawn, the flag, the name and a walker. The last two change
    // nothing about either answer, and are here because the key is the *content* hash — the
    // honest way to be wrong about this is to key on the file's timestamp and then rename a
    // level, so the cheap-and-wrong version of this key is the one worth pinning.
    fn paint(level: &mut sim::Level) {
        assert!(level.set_tile(4, 2, sim::Tile::Solid), "the paint lands");
    }
    fn move_spawn(level: &mut sim::Level) {
        level.set_spawn_tile(3, 2);
    }
    fn move_flag(level: &mut sim::Level) {
        assert!(level.set_goal_tile(5, 2), "the flag lands");
    }
    fn rename(level: &mut sim::Level) {
        level.name = "renamed".into();
    }
    fn add_walker(level: &mut sim::Level) {
        assert!(level.add_walker_tile(8, 2), "the walker lands");
    }
    let edits: [Edit; 5] = [
        ("a tile", paint),
        ("the spawn", move_spawn),
        ("the flag", move_flag),
        ("the name", rename),
        ("a walker", add_walker),
    ];

    for (what, edit) in edits {
        let mut changed = sim::Level::from_json_str(&src).expect("the original text");
        edit(&mut changed);
        changed
            .to_json_file(&path)
            .expect("the changed level is written");
        poison_answers(&sidecar);

        let reopened = sim::Level::from_json_file(&path).expect("the changed level loads");
        let expected = sim::Level::from_json_str(
            &changed.to_json_pretty().expect("the changed level serializes"),
        )
        .expect("and parses back");
        assert_ne!(
            reopened.routes().hops_to_flag(0),
            Some(u32::MAX),
            "{what}: the cache of the level this used to be answered, and it is not that level"
        );
        assert_eq!(
            reopened.routes(),
            expected.routes(),
            "{what}: the route table beside the file is not this terrain's"
        );
        assert_eq!(
            sim::reach::report(&reopened),
            sim::reach::report(&expected),
            "{what}: the verdict beside the file is not this terrain's"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A cache that is not this level's answer is a **miss**: never an error, and never a guess.
///
/// The variants are the ways a sidecar can stop being usable, in the order they happen in
/// practice: another build's file (both versions), a level that was edited and saved (the
/// hash), a file caught mid-write, and a file that is not this tool's JSON at all. Every one of
/// them has to leave the level loading and answering correctly, which is the whole reason the
/// key exists rather than a "the cache is definitely up to date" flag somewhere.
#[test]
fn an_unusable_route_cache_is_a_miss_rather_than_a_failure() {
    let dir = cache_dir("unusable");
    let path = dir.join("unusable.json");
    cache_fixture("unusable")
        .to_json_file(&path)
        .expect("the level is written");
    let sidecar = sidecar_of(&path);

    // This terrain's own answers, computed with no cache anywhere near them: what every miss
    // below has to produce.
    let truth = sim::Level::from_json_str(
        &cache_fixture("unusable")
            .to_json_pretty()
            .expect("serializes"),
    )
    .expect("parses");
    let warm = sim::Level::from_json_file(&path).expect("the level loads");
    let _ = warm.routes();
    let _ = sim::reach::report(&warm);
    poison_answers(&sidecar);
    let poisoned = std::fs::read_to_string(&sidecar).expect("the cache reads back");

    // The contrast that makes every case below mean something: with a matching key, this file
    // is used, poison and all.
    let adopted = sim::Level::from_json_file(&path).expect("the level loads");
    assert_eq!(
        adopted.routes().hops_to_flag(0),
        Some(u32::MAX),
        "a cache with this level's key is adopted"
    );

    // A file with only half the answers is not broken, it is *early*: the oracle's verdict and
    // the route table are produced by different asks, so whichever comes first is written on its
    // own and the other half lands on the next write.
    poison(&sidecar, |doc| doc["routes"] = serde_json::Value::Null);
    let half = sim::Level::from_json_file(&path).expect("the level loads");
    assert_eq!(
        half.routes(),
        truth.routes(),
        "the route table the file has not got yet is computed"
    );
    assert_eq!(
        sim::reach::report(&half).diagnosis.as_deref(),
        Some(SENTINEL),
        "and the verdict it does have is still used"
    );

    let doc: serde_json::Value = serde_json::from_str(&poisoned).expect("the cache is JSON");
    let mut variants: Vec<(&str, String)> = vec![
        ("not JSON at all", "half a file {".to_string()),
        ("an empty file", String::new()),
        (
            "a file caught mid-write",
            poisoned[..poisoned.len() / 2].to_string(),
        ),
    ];
    for (what, field, value) in [
        ("another tool's format", "format", serde_json::json!("someone-elses-cache")),
        ("a format version from the future", "version", serde_json::json!(9999)),
        ("another sim build", "sim_version", serde_json::json!("0.0.1")),
        ("another move set", "move_set", serde_json::json!(9999)),
        (
            "another level's hash",
            "level_hash",
            serde_json::json!("fnv1a64:0000000000000000"),
        ),
    ] {
        let mut patched = doc.clone();
        patched[field] = value;
        variants.push((what, patched.to_string()));
    }

    for (what, contents) in variants {
        std::fs::write(&sidecar, contents).expect("the broken cache is written");
        let level = sim::Level::from_json_file(&path).expect("a level loads despite the cache");
        assert_ne!(
            level.routes().hops_to_flag(0),
            Some(u32::MAX),
            "{what}: the file was answered from even though it is not this level's"
        );
        assert_eq!(level.routes(), truth.routes(), "{what}: recomputed exactly");
        assert_eq!(
            sim::reach::report(&level),
            sim::reach::report(&truth),
            "{what}: the verdict recomputed exactly too"
        );
    }

    // And no cache at all, which is the state every level starts in.
    std::fs::remove_file(&sidecar).expect("the cache is removed");
    let none = sim::Level::from_json_file(&path).expect("a level loads with no cache");
    assert_eq!(none.routes(), truth.routes());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A document that has been edited — or has had the ground broken out from under it — is not
/// the document its file describes, so it neither answers from that file's cache nor writes to
/// it.
///
/// Both halves matter, and they are different rules for different reasons. Reading has to stop
/// because the answers are about terrain the document no longer has, and the interesting case
/// is the editor: a level that adopted its verdict on the first frame must not keep serving it
/// after the user has moved the flag. Writing has to stop because the file's key is the content
/// hash, and a document that has drifted is a document whose hash no longer describes the file
/// it was loaded from.
///
/// Breaking a tile during an episode is the second half of this: the route table survives it on
/// purpose (the compass answers for the level as *authored*, so a bridge giving way must not
/// swing it), while the file's cache goes.
#[test]
fn an_edited_document_neither_answers_from_nor_writes_its_files_cache() {
    let dir = cache_dir("edited-document");
    let path = dir.join("doc.json");
    cache_fixture("doc")
        .to_json_file(&path)
        .expect("the level is written");
    let sidecar = sidecar_of(&path);

    let warm = sim::Level::from_json_file(&path).expect("the level loads");
    let _ = warm.routes();
    let _ = sim::reach::report(&warm);
    poison_answers(&sidecar);
    let poisoned = std::fs::read_to_string(&sidecar).expect("the cache reads back");

    // The edit, on a document whose cache is in use before the edit lands.
    let mut edited = sim::Level::from_json_file(&path).expect("the level loads");
    assert_eq!(
        edited.routes().hops_to_flag(0),
        Some(u32::MAX),
        "the document starts from a cache it is really using"
    );
    assert!(edited.set_tile(7, 2, sim::Tile::Solid), "the edit lands");
    let as_text = edited.to_json_pretty().expect("the document serializes");
    let expected = sim::Level::from_json_str(&as_text).expect("and parses back");
    assert_eq!(
        sim::reach::report(&edited),
        sim::reach::report(&expected),
        "an edited document's verdict is computed, not read off the file"
    );
    assert_eq!(
        std::fs::read_to_string(&sidecar).expect("the cache still reads back"),
        poisoned,
        "nor does an edited document write its own content into the file's cache"
    );

    // And the ground giving way, which is the same rule from the other side.
    let mut broken = sim::Level::from_json_file(&path).expect("the level loads");
    assert_eq!(broken.routes().hops_to_flag(0), Some(u32::MAX));
    broken.burn_tile(7, 3);
    assert_ne!(
        sim::reach::report(&broken).diagnosis.as_deref(),
        Some(SENTINEL),
        "a broken floor is a different level and gets a verdict of its own"
    );
    assert_eq!(
        std::fs::read_to_string(&sidecar).expect("the cache still reads back"),
        poisoned,
        "while the cache of the level it used to be is left where it is"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
