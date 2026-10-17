//! Observation layout tests (§7.2, §11).
//!
//! These pin the layout by *name and meaning* rather than by one opaque golden
//! blob: a golden value tells you "something changed", while slot-by-slot
//! assertions tell you what changed and why. The golden blob is still present, in
//! the `golden_*` test, for exactly the accidents the named slots cannot catch.

mod common;

use sim::sensors;
use sim::AgentAction;

/// Ground rows 4..5, 30 columns wide, with the flag past the sensor's sixteen-tile horizon.
///
/// The width is the point: [`GOAL_VIEW_DISTANCE`](sensors::GOAL_VIEW_DISTANCE) is sixteen tiles, so
/// on this fixture the goal vector — and the route waypoint that falls back to it — read their
/// clamped extremes while the body is on the far side of the level. That is the state in which the
/// pre-v10 vector carries no sub-tile position at all, which is what the phase slots are for.
const CORRIDOR: &[&str] = &[
    "..............................",
    "..............................",
    "..............................",
    "..............................",
    "##############################",
    "##############################",
];

/// Ground rows 4..5, 12 columns wide.
const GROUND: &[&str] = &[
    "............",
    "............",
    "............",
    "............",
    "############",
    "############",
];

/// Same ground, with a spike on the surface of column 6 — one tile to the right
/// of a body standing in column 5.
const ADJACENT_SPIKE: &[&str] = &[
    "............",
    "............",
    "............",
    "......^.....",
    "############",
    "############",
];

/// A shaft: ground rows 8..9 with eight rows of air above and the flag near the top.
/// This is the shape a horizontal-only goal sensor cannot describe — from the bottom
/// of a shaft, a flag overhead and a flag down a corridor both read as "not to either
/// side".
const SHAFT: &[&str] = &[
    ".....",
    ".....",
    ".....",
    ".....",
    ".....",
    ".....",
    ".....",
    ".....",
    "#####",
    "#####",
];

/// A pen: ground rows 4..5 with two wall columns (5 and 9) standing on it, so a walker
/// placed between them patrols a fixed run and turns around at both ends, while the body
/// that is watching stands outside the run and cannot be reached. This is what the
/// direction slot is measured on — a walker that never turns would not show a flip.
const PEN: &[&str] = &[
    "............",
    "............",
    ".....#...#..",
    ".....#...#..",
    "############",
    "############",
];

/// Ground rows 4..5 with a wall column at 9 rising to the top of the grid: a body
/// running right presses into its face, which is what the wall slots are measured on.
const WALL_FACE: &[&str] = &[
    ".........#..",
    ".........#..",
    ".........#..",
    ".........#..",
    "############",
    "############",
];

/// Ground stops at column 9 in a 20-wide level, so all three lookahead columns
/// are genuinely open. (The out-of-bounds column reads as solid, so a gap test
/// must keep the level wide enough for the lookahead to see air.)
const LEDGE: &[&str] = &[
    "....................",
    "....................",
    "....................",
    "....................",
    "##########..........",
    "##########..........",
];

/// [`LEDGE`] mirrored: ground from column 10 rightward, open air to the left of it.
/// The pair is what makes the gap test a test of *symmetry* rather than of arithmetic.
const MIRROR_LEDGE: &[&str] = &[
    "....................",
    "....................",
    "....................",
    "....................",
    "..........##########",
    "..........##########",
];

/// Index of patch cell `(dx, dy)` relative to the body's centre tile.
fn patch_index(dx: i32, dy: i32) -> usize {
    let r = sensors::PATCH_RADIUS;
    let side = 2 * r + 1;
    ((dy + r) as usize) * side as usize + (dx + r) as usize
}

#[test]
fn the_vector_has_the_documented_named_layout() {
    // 100 = the plan's 90 plus the three ability inputs appended by v2, v3's second goal
    // axis, v4's second gap probe, v7's enemy heading, v8's two waypoint axes, and v10's two
    // sub-tile phases. Every earlier index keeps its slot, which is what makes the diff between
    // the layouts reviewable.
    assert_eq!(
        sensors::N_INPUTS,
        100,
        "§7.2's 90 plus v2's three, v3's and v4's one each, v7's enemy_dir, v8's waypoint and \
         v10's two phases"
    );
    // v5 bounded the enemy slots and v6 widened that bound; both moved the reading without
    // moving a slot. v7 and v8 are appends, which is why this number moved: the whole
    // reason the layout is asserted by name rather than by one opaque blob.
    //
    // v9 is the *other* kind of change and the reason this constant is asserted at all: nothing in
    // the layout moved — no index changed meaning and `N_INPUTS` above is unmoved — and what moved
    // is what two cells of the patch *say* (a brittle tile and a pad, see [`sensors::ENC_CRUMBLE`]).
    // A champion replayed against a build with pads in it is being read a world it was never
    // trained on, which is a version bump whether the vector got wider or not.
    assert_eq!(sensors::VERSION, 10);

    let names = sensors::input_names();
    assert_eq!(names.len(), sensors::N_INPUTS);

    // The 9x9 patch, row-major, top-left first.
    assert_eq!(names[patch_index(-4, -4)], "tile[-4,-4]");
    assert_eq!(names[patch_index(0, 0)], "tile[+0,+0]");
    assert_eq!(names[patch_index(4, 4)], "tile[+4,+4]");
    assert_eq!(patch_index(0, 0), 40);
    assert_eq!(patch_index(4, 4), 80);

    // Then the eight scalar features, in the documented order.
    assert_eq!(names[81], "enemy_dx");
    assert_eq!(names[82], "enemy_dy");
    assert_eq!(names[83], "gap_right");
    assert_eq!(names[84], "grounded");
    assert_eq!(names[85], "vx");
    assert_eq!(names[86], "vy");
    assert_eq!(names[87], "can_jump");
    assert_eq!(names[88], "goal_dx");
    assert_eq!(names[89], "goal_dy");
    assert_eq!(names[90], "bias");

    // v2 appends the ability inputs, so nothing above moved.
    assert_eq!(names[91], "wall_dir");
    assert_eq!(names[92], "clinging");
    assert_eq!(names[93], "sprint_charge");
    assert_eq!(names[94], "gap_left", "v4 appends the leftward probe");
    assert_eq!(names[95], "enemy_dir", "v7 appends the walker's heading");
    assert_eq!(names[96], "waypoint_dx", "v8 appends the route's next waypoint");
    assert_eq!(names[97], "waypoint_dy");
    assert_eq!(
        names[98], "phase_x",
        "v10 appends where the body is inside its own tile"
    );
    assert_eq!(names[99], "phase_y");
    assert_eq!(names.len(), 90 + 3 + sensors::N_ABILITY_INPUTS + 2 + 2);
}

/// **Where the goal vector is clamped, the phase slots are the only sub-tile position there is.**
///
/// This is the fixture the version bump is written for: the flag is past
/// [`sensors::GOAL_VIEW_DISTANCE`], so both goal axes *and* both waypoint axes read their clamped
/// extremes, and every remaining pre-v10 input — the whole 81-cell patch, both gap probes, the
/// booleans, the two velocities excepted (those are state, not position) — is a function of the tile
/// the body is in. So while the body runs across one tile, every other slot holds still and the two
/// phases sweep; `phase_x` tracks the body's own fraction of its tile one to one.
#[test]
fn inside_one_tile_only_the_phase_slots_move() {
    let level = common::fixture("corridor", CORRIDOR, (2, 3), &[("goal", 28, 3)]);
    let mut sim = sim::Sim::new(&level, 11);
    common::settle(&mut sim);

    // The flag is past the sensor's own horizon, so `goal_*` and `waypoint_*` read their clamped
    // extremes rather than a distance: that is the state this test is about.
    let first = sim.observation();
    assert_eq!(first[88], 1.0, "the flag is saturated to the right");
    let tile = (sim.player().x + sim.player().w * 0.5).floor();

    // `Run` for a third of a tile: the body is at rest (its centre exactly at the tile's middle,
    // phase 0.0) and stays inside its starting tile for the whole sweep.
    let mut phases = Vec::new();
    for tick in 0..4 {
        sim.step(AgentAction::Run);
        let p = sim.player();
        let here = sim.observation();
        let centre = p.x + p.w * 0.5;
        assert_eq!(
            centre.floor(),
            tile,
            "the sweep has to stay inside one tile to say anything (tick {tick}, centre {centre})"
        );
        for i in 0..98 {
            // The velocities are the body's *state*; every other pre-v10 slot is the body's tile.
            if i == 85 || i == 86 {
                continue;
            }
            assert_eq!(
                here[i], first[i],
                "slot {i} ({}) moved inside one tile: {} against {}",
                sensors::input_names()[i],
                here[i],
                first[i]
            );
        }
        let want = (2.0 * (centre - centre.floor()) - 1.0).clamp(-1.0, 1.0);
        assert!(
            (here[98] - want).abs() < 1e-5,
            "phase_x {} should be the body's own fraction of its tile ({want})",
            here[98]
        );
        phases.push(here[98]);
    }

    assert!(
        phases.windows(2).all(|w| w[1] > w[0]),
        "running right has to sweep the phase upward: {phases:?}"
    );
    assert!(
        phases[3] - phases[0] > 0.15,
        "and by enough to time a press on: {phases:?}"
    );

    // The vertical slot is the same claim on the other axis: a jump carries the body through its
    // own row, and the phase is what says how far. (The patch legitimately moves here — the body
    // crosses rows — so this half asserts the slot's *range*, not the invariance above.)
    let ground_phase = sim.observation()[99];
    sim.step(AgentAction::JumpInPlace);
    let mut vertical = Vec::new();
    for _ in 0..12 {
        sim.step(AgentAction::JumpInPlace);
        vertical.push(sim.observation()[99]);
    }
    let v_spread = vertical.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - vertical.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(
        v_spread > 0.05,
        "a jump has to move phase_y: ground {ground_phase}, flight {vertical:?}"
    );
}

/// **The slot no amount of training could recover.** `enemy_dx`/`enemy_dy` say where a walker
/// is; nothing said which way it was walking, and that is not a fact a level file holds — a
/// patrol's initial direction is drawn from the episode RNG, which is the only gameplay
/// randomness in the sim. So this test is about a *new observation* rather than a new input:
/// the reading has to follow the walker as it turns, not just exist.
#[test]
fn the_enemy_direction_slot_follows_the_walker_as_it_turns() {
    let level = common::fixture("pen", PEN, (2, 3), &[("walker", 7, 3), ("goal", 10, 3)]);
    assert_eq!(level.walkers.len(), 1, "the fixture must put a walker in the pen");
    let mut sim = sim::Sim::new(&level, 47);
    common::settle(&mut sim);

    let obs = sim.observation();
    let dir = sim.walkers()[0].dir;
    assert!(
        dir == 1 || dir == -1,
        "the walker is walking one way or the other, got {dir}"
    );
    assert_eq!(obs[95], dir as f32, "the slot is the walker's own direction");
    assert!(
        obs[81].abs() > 0.0,
        "and it is only reported for a walker that is actually in sight"
    );

    // Walk long enough for the pen's walls to turn it around more than once, and check the
    // slot tracks the body on every tick — not merely at the start and the end.
    let mut flips = 0;
    let mut last = obs[95];
    for tick in 0..400 {
        sim.step(AgentAction::Walk);
        let read = sim.observation()[95];
        assert_eq!(
            read,
            sim.walkers()[0].dir as f32,
            "tick {tick}: the slot disagrees with the walker it describes"
        );
        if read != last {
            flips += 1;
            last = read;
        }
    }
    assert!(
        flips >= 1,
        "a walker penned between two walls must turn around inside 400 ticks, \
         or this test cannot see the reading that matters"
    );
    assert!(
        !sim.outcome().is_finished(),
        "the body watching from outside the pen must survive the test"
    );

    // Out of sight is `0.0`, exactly like the two position slots it is appended after: there
    // is no walker to have a heading, and a phantom ±1 from across the level would be the same
    // lie v5 set out to remove.
    let far = common::fixture("pen-far", PEN, (2, 3), &[("walker", 10, 3), ("goal", 10, 3)]);
    let mut sim_far = sim::Sim::new(&far, 47);
    common::settle(&mut sim_far);
    let far_obs = sim_far.observation();
    assert_eq!(
        (far_obs[81], far_obs[82], far_obs[95]),
        (0.0, 0.0, 0.0),
        "a walker beyond ENEMY_VIEW_DISTANCE has no position and no heading"
    );
}

#[test]
fn the_patch_reads_air_above_and_solid_below_a_settled_body() {
    let level = common::fixture("ground", GROUND, (5, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 5);
    common::settle(&mut sim);
    let obs = sim.observation();

    assert_eq!(
        obs[patch_index(0, 0)],
        sensors::ENC_EMPTY,
        "the tile containing the body's centre is air when standing on open ground"
    );
    assert_eq!(
        obs[patch_index(0, 1)],
        sensors::ENC_SOLID,
        "the tile one row below the centre is the ground the body stands on"
    );
    for dy in -sensors::PATCH_RADIUS..=0 {
        assert_eq!(
            obs[patch_index(0, dy)],
            sensors::ENC_EMPTY,
            "the sky above the body must read empty at dy = {dy}"
        );
    }
    assert!(
        obs.iter().all(|v| v.is_finite() && v.abs() <= 1.0),
        "every observation value must be finite and inside [-1, 1]"
    );
}

#[test]
fn a_spike_tile_is_visible_and_encoded_distinctly() {
    assert_ne!(sensors::ENC_SPIKE, sensors::ENC_SOLID);
    assert_ne!(sensors::ENC_SPIKE, sensors::ENC_EMPTY);
    assert_ne!(sensors::ENC_ENEMY, sensors::ENC_SPIKE);

    let level = common::fixture("adjacent-spike", ADJACENT_SPIKE, (5, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 9);
    // The spike is one tile to the right, so merely standing here is safe.
    common::settle(&mut sim);
    assert_eq!(sim.outcome(), sim::Outcome::Running);

    let obs = sim.observation();
    assert_eq!(
        obs[patch_index(1, 0)],
        sensors::ENC_SPIKE,
        "the spike one tile to the right must be visible, and distinct from solid"
    );
    assert_eq!(
        obs[patch_index(1, 1)],
        sensors::ENC_SOLID,
        "the ground under the spike is still solid"
    );
    assert_eq!(
        obs[patch_index(-1, 1)],
        sensors::ENC_SOLID,
        "the ground under the body is solid"
    );
    assert_eq!(obs[patch_index(-1, 0)], sensors::ENC_EMPTY);
}

#[test]
fn the_boolean_and_velocity_slots_track_the_body() {
    let level = common::fixture("ground", GROUND, (1, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 21);
    common::settle(&mut sim);

    let obs = sim.observation();
    assert_eq!(obs[84], 1.0, "slot 84 is grounded");
    assert_eq!(obs[87], 1.0, "slot 87 is can-jump");
    assert_eq!(obs[90], 1.0, "slot 90 is the constant bias");
    assert_eq!(obs[85], 0.0, "a resting body has zero normalised vx");
    assert_eq!(obs[86], 0.0, "a resting body has zero normalised vy");

    // v2 normalises vx by `sprint_speed`, not `run_speed`: at run speed the slot reads
    // 0.67 and only a sprinting body reaches 1.0. The old normalisation saturated here,
    // which would have made a running body and a sprinting one indistinguishable — the
    // one thing a speed sensor exists to tell apart.
    common::step_n(&mut sim, AgentAction::Run, 40);
    let obs = sim.observation();
    assert!(
        (obs[85] - sim::PHYSICS.run_speed / sim::PHYSICS.sprint_speed).abs() < 1e-4,
        "a full-speed run must read run_speed/sprint_speed, got {}",
        obs[85]
    );
    assert!(
        obs[85] < 1.0 && obs[85] > 0.5,
        "a run is fast and visibly not a sprint: {}",
        obs[85]
    );
    assert!(
        (sim.player().vx / sim::PHYSICS.sprint_speed - obs[85]).abs() < 1e-4,
        "slot 85 must be vx / sprint_speed, not some other normalisation"
    );
    assert_eq!(obs[84], 1.0, "still grounded while running along the ground");

    // The jump needs the button released first: it is edge-triggered.
    sim.step(AgentAction::RunJump);
    let obs = sim.observation();
    assert_eq!(obs[84], 0.0, "a rising body is not grounded");
    assert!(
        obs[86] < 0.0,
        "a rising body has negative vy, got {}",
        obs[86]
    );
    assert_eq!(obs[87], 0.0, "can-jump must be false mid-air");
    assert_eq!(
        obs[86],
        (sim.player().vy / sim::PHYSICS.max_fall).clamp(-1.0, 1.0),
        "slot 86 must be vy / max_fall"
    );
}

#[test]
fn the_gap_slots_fire_on_the_side_the_ground_runs_out() {
    // Standing still on the last ground tile: the right-hand probe sees the hole and the
    // left-hand one sees the ledge behind it. That the body is at *rest* is the point of
    // the test — v3's single probe read `PlayerState::facing`, which fell back to "right"
    // for a body with no velocity, so an agent that had not moved yet was told about the
    // ground to its right whatever was behind it.
    let ledge = common::fixture("ledge", LEDGE, (9, 3), &[("goal", 1, 3)]);
    let mut sim = sim::Sim::new(&ledge, 41);
    common::settle(&mut sim);
    assert_eq!(sim.player().vx, 0.0, "the body must be at rest for this to mean anything");
    assert_eq!(
        sim.observation()[83],
        1.0,
        "standing on the last ground tile, the next {} columns to the right are open",
        sensors::GAP_LOOKAHEAD
    );
    assert_eq!(
        sim.observation()[94],
        0.0,
        "...and the ground it is standing on continues to the left"
    );

    // The same body, the same instant, on the mirrored ledge: the two slots swap. A probe
    // that followed the direction of travel would read the same thing on both levels here,
    // because both bodies are stationary.
    let mirror = common::fixture("mirror-ledge", MIRROR_LEDGE, (10, 3), &[("goal", 18, 3)]);
    let mut mirrored = sim::Sim::new(&mirror, 41);
    common::settle(&mut mirrored);
    assert_eq!(mirrored.player().vx, 0.0);
    assert_eq!(
        mirrored.observation()[83],
        0.0,
        "ground continues to the right of the mirrored ledge"
    );
    assert_eq!(
        mirrored.observation()[94],
        1.0,
        "the open air is on the left, and the left slot is what names it"
    );

    let solid = common::fixture("ground", GROUND, (8, 3), &[("goal", 1, 3)]);
    let mut sim2 = sim::Sim::new(&solid, 41);
    common::settle(&mut sim2);
    assert_eq!(sim2.observation()[83], 0.0, "ground continues both ways on flat ground");
    assert_eq!(sim2.observation()[94], 0.0);
}

/// The body's *velocity* must not change what the gap slots say. This is the v4 claim as an
/// assertion: the pair describes the world beside the feet, not the last decision.
#[test]
fn the_gap_slots_ignore_the_direction_the_body_is_moving() {
    let level = common::fixture("ledge", LEDGE, (9, 3), &[("goal", 1, 3)]);
    let standing = sim::Sim::new(&level, 41);
    let mut running = sim::Sim::new(&level, 41);
    common::step_n(&mut running, AgentAction::Run, 1);
    let mut leaving = sim::Sim::new(&level, 41);
    common::step_n(&mut leaving, AgentAction::Retreat, 1);

    for (name, s) in [("standing", &standing), ("running right", &running), ("running left", &leaving)] {
        let obs = s.observation();
        assert_eq!(obs[83], 1.0, "{name}: the hole to the right is still to the right");
        assert_eq!(obs[94], 0.0, "{name}: and the ledge to the left is still to the left");
    }
}

/// The goal slots are a *vector*, not a horizontal sign: they must name which way the flag
/// lies on both axes, because a level may put the goal above or below as well as beyond.
#[test]
fn the_goal_slots_point_at_the_goal_on_both_axes() {
    let right = common::fixture("goal-right", GROUND, (5, 3), &[("goal", 10, 3)]);
    let mut sim = sim::Sim::new(&right, 33);
    common::settle(&mut sim);
    let obs = sim.observation();
    assert!(obs[88] > 0.0, "the goal is to the right: goal_dx {}", obs[88]);
    assert!(
        obs[89].abs() < 0.1,
        "and at the body's height, so goal_dy is near zero: {}",
        obs[89]
    );
    // Recomputed from the live body and goal centres rather than hardcoding the fixture's
    // geometry, so this pins the *normalisation* and not just the sign.
    let (gx, gy) = right.goals[0].center();
    let (px, py) = sim.player().aabb().center();
    assert_eq!(
        obs[88],
        ((gx - px) / sensors::GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0),
        "goal_dx must be dx / GOAL_VIEW_DISTANCE"
    );
    assert_eq!(
        obs[89],
        ((gy - py) / sensors::GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0),
        "goal_dy must be dy / GOAL_VIEW_DISTANCE"
    );

    let left = common::fixture("goal-left", GROUND, (5, 3), &[("goal", 1, 3)]);
    let mut sim_left = sim::Sim::new(&left, 33);
    common::settle(&mut sim_left);
    assert!(
        sim_left.observation()[88] < 0.0,
        "the goal is to the left, so goal_dx is negative: {}",
        sim_left.observation()[88]
    );

    // The vertical case, which the old single horizontal sign could not express at all: a
    // flag six rows overhead and a flag down a corridor used to read identically, because
    // both are "not to either side".
    let shaft = common::fixture("shaft", SHAFT, (1, 7), &[("goal", 2, 1)]);
    let mut sim_up = sim::Sim::new(&shaft, 33);
    common::settle(&mut sim_up);
    let obs = sim_up.observation();
    assert!(
        obs[89] < -0.25,
        "the goal is above, and y grows downward, so goal_dy must be negative: {}",
        obs[89]
    );
    let (gx, gy) = shaft.goals[0].center();
    let (px, py) = sim_up.player().aabb().center();
    assert_eq!(
        obs[89],
        ((gy - py) / sensors::GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0),
        "the vertical component is exact here too"
    );
    assert!(
        (gx - px).abs() < 2.0 && (gy - py).abs() > 5.0,
        "the fixture really does put the flag far above and barely to the side ({} across, \
         {} up)",
        gx - px,
        py - gy
    );
}

/// Both goal components must be recomputed from the live body, not latched at spawn:
/// the sensor is what lets a randomised body *find* the flag, so it has to track it.
#[test]
fn the_goal_slots_track_the_body_as_it_moves() {
    let level = common::fixture("shaft-run", SHAFT, (1, 7), &[("goal", 2, 1)]);
    let mut sim = sim::Sim::new(&level, 34);
    common::settle(&mut sim);
    let grounded = sim.observation()[89];
    assert!(grounded < -0.3, "the flag is overhead: {grounded}");

    // A jump lifts the body toward the flag, so the vertical component must shrink in
    // magnitude. If the slot were latched at spawn this would not move.
    sim.step(AgentAction::JumpInPlace);
    let rising = sim.observation()[89];
    assert!(
        rising > grounded,
        "rising toward an overhead goal must raise goal_dy: {grounded} -> {rising}"
    );

    // And landing restores the reading: the goal never moved, only the body did.
    common::step_n(&mut sim, AgentAction::Brake, 60);
    common::settle(&mut sim);
    let landed = sim.observation()[89];
    assert!(
        (landed - grounded).abs() < 1e-2,
        "landing restores the reading: {grounded} -> {landed}"
    );
}

#[test]
fn the_enemy_slots_point_at_the_nearest_live_walker() {
    let level = common::fixture("open", GROUND, (5, 3), &[("walker", 7, 3), ("goal", 11, 3)]);
    assert_eq!(level.walkers.len(), 1);

    let mut sim = sim::Sim::new(&level, 47);
    common::settle(&mut sim);
    let obs = sim.observation();

    assert!(
        obs[81] > 0.0,
        "the walker is to the right, so enemy_dx should be positive, got {}",
        obs[81]
    );
    assert!(
        obs[82].abs() < 0.2,
        "the walker is at the body's height, so enemy_dy should be near zero, got {}",
        obs[82]
    );
    assert!(
        (-1.0..=1.0).contains(&obs[81]) && (-1.0..=1.0).contains(&obs[82]),
        "enemy slots stay normalised into [-1, 1]: {} / {}",
        obs[81],
        obs[82]
    );

    // Exact, self-relative check of the normalisation: recompute it from the live
    // body centres rather than hardcoding the fixture's geometry.
    let (wx, wy) = sim.walkers()[0].aabb(&sim::PHYSICS).center();
    let (px, py) = sim.player().aabb().center();
    assert_eq!(
        obs[81],
        ((wx - px) / sensors::ENEMY_VIEW_DISTANCE).clamp(-1.0, 1.0),
        "enemy_dx must be dx / ENEMY_VIEW_DISTANCE"
    );
    assert_eq!(
        obs[82],
        ((wy - py) / sensors::ENEMY_VIEW_DISTANCE).clamp(-1.0, 1.0),
        "enemy_dy must be dy / ENEMY_VIEW_DISTANCE"
    );

    // A walker beyond ENEMY_VIEW_DISTANCE is out of sight and reads (0.0, 0.0).
    let far = common::fixture("far-enemy", GROUND, (1, 3), &[("walker", 10, 3), ("goal", 11, 3)]);
    let mut sim_far = sim::Sim::new(&far, 47);
    common::settle(&mut sim_far);
    let obs_far = sim_far.observation();
    assert_eq!(
        (obs_far[81], obs_far[82]),
        (0.0, 0.0),
        "a walker 9 tiles away is beyond ENEMY_VIEW_DISTANCE and should read as no enemy in sight"
    );
}

/// The three v2 slots, measured on a body that is doing the thing they describe.
///
/// Worth their own test rather than a line in the layout test: these are the inputs a
/// wall climb and a sprint have to be learnable *from*, so the question is not "is the
/// slot there" but "does it say the truth about the body".
#[test]
fn the_ability_slots_report_the_wall_and_the_sprint() {
    let level = common::fixture("wall-face", WALL_FACE, (2, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 61);
    common::settle(&mut sim);

    // Standing still, nothing is against anything and the sprint is not charged.
    let obs = sim.observation();
    assert_eq!(obs[91], 0.0, "no wall while standing in the open");
    assert_eq!(obs[92], 0.0, "and no slide");
    assert_eq!(obs[93], 0.0, "a resting body has no sprint charge");

    // Running into the wall face: the wall slot names the side it is on. (Long enough to
    // cross the open ground *and* come to rest against the wall — the slot reports a body
    // that is pressed against something, not one that can see it.)
    common::step_n(&mut sim, AgentAction::Run, 100);
    let obs = sim.observation();
    assert_eq!(obs[91], 1.0, "the wall is on the body's right");
    assert_eq!(obs[92], 0.0, "a grounded body is not sliding down anything");

    // Jump while pressing into it, then keep pressing: the body rises and comes back
    // down the face, which is the cling the wall jump is taken from.
    sim.step(AgentAction::RunJump);
    common::step_n(&mut sim, AgentAction::Run, 30);
    let obs = sim.observation();
    assert_eq!(obs[91], 1.0, "still pressed against the wall");
    assert_eq!(obs[92], 1.0, "and falling: the slide is active");
    assert!(!sim.player().grounded);
    assert!(
        sim.player().vy <= sim::PHYSICS.wall_slide_fall + 1e-4,
        "the slide must cap the fall at {}, got {}",
        sim::PHYSICS.wall_slide_fall,
        sim.player().vy
    );
    assert!(
        sim.player().vy < sim::PHYSICS.max_fall,
        "a slide has to be slower than a fall, or the wall buys nothing"
    );

    // Let go of the direction and the cling ends on the next tick: it is a *pressed*
    // state, so a body cannot slide down a wall it is not pushing into.
    sim.step(AgentAction::Walk);
    let obs = sim.observation();
    assert_eq!(obs[91], 0.0, "releasing the direction drops the wall contact");
    assert_eq!(obs[92], 0.0, "and the slide with it");

    // Sprint: the charge ramps while it is asked for and coasts back down when it is not.
    let mut flat = sim::Sim::new(
        &common::fixture("ground", GROUND, (1, 3), &[("goal", 11, 3)]),
        62,
    );
    common::settle(&mut flat);
    assert_eq!(flat.observation()[93], 0.0);
    common::step_n(&mut flat, AgentAction::Sprint, 40);
    let obs = flat.observation();
    assert_eq!(obs[93], 1.0, "40 ticks of sprinting is a full charge");
    assert!(
        (flat.player().vx - sim::PHYSICS.sprint_speed).abs() < 1e-4,
        "and the body is really at the sprint cap: {}",
        flat.player().vx
    );
    assert_eq!(obs[85], 1.0, "slot 85 saturates only for a sprint");
    common::step_n(&mut flat, AgentAction::Run, 3);
    let coasting = flat.observation()[93];
    assert!(
        coasting > 0.0 && coasting < 1.0,
        "releasing sprint coasts the charge down rather than snapping it: {coasting}"
    );
    assert!(
        flat.player().vx > sim::PHYSICS.run_speed,
        "and the body keeps the speed it built while the cap falls: {}",
        flat.player().vx
    );
}

/// Pinned hash of the observation vector at a fixed state.
///
/// The slot-by-slot tests above document the layout one field at a time; this
/// pins the whole 96-float vector so an accidental edit anywhere in the patch
/// loop is caught even if it lands in a slot nobody asserts on directly. If this
/// fails, the fix is a `sensors::VERSION` bump and a new constant -- never a
/// quiet update, because every champion file records the version (§8).
///
/// Re-pinned for **v4**, which is the one deliberate change it has ever had
/// besides v2's and v3's: `gap_ahead` became `gap_right` + `gap_left`, so index 83
/// changed meaning and index 94 is new. Re-pinned again for **v6**, which moved the
/// enemy vector's normaliser from four tiles to eight; the fixture now carries a walker
/// four tiles to the right, because until v6 the fixture had none and slots 81-82 were
/// pinned as a constant zero — a drift in them would have gone unnoticed here.
///
/// Re-pinned a third time for **v7**, which appends slot 95 (`enemy_dir`). This is what the
/// golden blob is for: that fixture's walker is in sight, so the new slot is a drawn `±1`
/// rather than a zero, and an accidental change to a walker's own direction — or to the
/// tie-break between two walkers — lands on this number.
///
/// Re-pinned a fourth time for **v8**, which appends slots 96-97 (`waypoint_dx, waypoint_dy`).
/// The fixture is a one-crossing floor with the flag to the right, so both new slots are drawn
/// non-zero here: an accidental change to which stand the route aims at, or to the waypoint's
/// normaliser, lands on this number.
#[test]
fn golden_observation_hash_at_a_fixed_state() {
    const GOLDEN_OBSERVATION_HASH: u64 = 13_055_861_740_400_240_297;

    let level = common::fixture("ground", GROUND, (5, 3), &[("walker", 9, 3), ("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 5);
    common::settle(&mut sim);
    let obs = sim.observation();

    assert_eq!(
        sim::hash::hash_f32_slice(&obs),
        GOLDEN_OBSERVATION_HASH,
        "the observation vector drifted; if that was intentional, bump \
         sensors::VERSION and update this constant"
    );
}

/// **The two trap tiles are visible in the patch, and they do not look like the tiles they replace.**
///
/// A brittle tile is solid and a pad is not, so both of them *could* have been reported as the tiles
/// they resemble — and both would then be invisible to a policy that has to decide whether to cross
/// a bridge or to land on a pad. This is the pin for `sensors::VERSION` 9: the four new readings are
/// distinct from `ENC_SOLID`, `ENC_EMPTY` and each other, on the tile the body is standing beside.
#[test]
fn a_cracked_tile_and_a_spring_read_differently_in_the_patch() {
    /// Row 2 of the fixture: a pad at column 2, brittle ground at column 4, sound ground at 6.
    const TRAPS: &[&str] = &[
        "............",
        "............",
        "..v.d.#.....",
        "............",
        "############",
        "############",
    ];

    let level = common::fixture("traps", TRAPS, (1, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 5);
    common::settle(&mut sim);
    let obs = sim.observation();

    // The patch is 9x9 row-major around the tile the body's centre is in, top-left first.
    let cell = |dx: i32, dy: i32| -> f32 { obs[((dy + 4) * 9 + (dx + 4)) as usize] };

    assert_eq!(cell(0, 1), sensors::ENC_SOLID, "control: the ground under it");
    assert_eq!(cell(1, -1), sensors::ENC_SPRING, "the pad reads as a pad");
    assert_eq!(cell(3, -1), sensors::ENC_CRUMBLE, "the brittle tile reads as brittle");
    assert_eq!(cell(5, -1), sensors::ENC_SOLID, "the sound ground beside it reads as ground");

    // And the readings are mutually distinct, which is the whole claim: a body can tell a pad from
    // the air it stands in, and brittle ground from the ground that will still be there next frame.
    for (a, b) in [
        (sensors::ENC_SPRING, sensors::ENC_SOLID),
        (sensors::ENC_SPRING, sensors::ENC_EMPTY),
        (sensors::ENC_SPRING, sensors::ENC_CRUMBLE),
        (sensors::ENC_CRUMBLE, sensors::ENC_SOLID),
        (sensors::ENC_CRUMBLE, sensors::ENC_EMPTY),
        (sensors::ENC_CRUMBLE, sensors::ENC_SPIKE),
    ] {
        assert_ne!(a, b, "two tiles the agent has to tell apart read the same");
    }
}
