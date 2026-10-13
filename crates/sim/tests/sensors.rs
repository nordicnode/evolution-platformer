//! Observation layout tests (§7.2, §11).
//!
//! These pin the layout by *name and meaning* rather than by one opaque golden
//! blob: a golden value tells you "something changed", while slot-by-slot
//! assertions tell you what changed and why. The golden blob is still present, in
//! the `golden_*` test, for exactly the accidents the named slots cannot catch.

mod common;

use sim::sensors;
use sim::AgentAction;

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

/// Index of patch cell `(dx, dy)` relative to the body's centre tile.
fn patch_index(dx: i32, dy: i32) -> usize {
    let r = sensors::PATCH_RADIUS;
    let side = 2 * r + 1;
    ((dy + r) as usize) * side as usize + (dx + r) as usize
}

#[test]
fn the_vector_has_the_documented_named_layout() {
    assert_eq!(sensors::N_INPUTS, 90, "§7.2 fixes the vector at 90 inputs");
    assert_eq!(sensors::VERSION, 1);

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
    assert_eq!(names[83], "gap_ahead");
    assert_eq!(names[84], "grounded");
    assert_eq!(names[85], "vx");
    assert_eq!(names[86], "vy");
    assert_eq!(names[87], "can_jump");
    assert_eq!(names[88], "goal_dir");
    assert_eq!(names[89], "bias");
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
    assert_eq!(obs[89], 1.0, "slot 89 is the constant bias");
    assert_eq!(obs[85], 0.0, "a resting body has zero normalised vx");
    assert_eq!(obs[86], 0.0, "a resting body has zero normalised vy");

    // Running right saturates vx at run_speed, so slot 85 saturates at 1.0.
    common::step_n(&mut sim, AgentAction::Run, 40);
    let obs = sim.observation();
    assert_eq!(
        obs[85], 1.0,
        "a full-speed rightwards run saturates the normalised vx slot"
    );
    assert!(
        (sim.player().vx / sim::PHYSICS.run_speed - 1.0).abs() < 1e-4,
        "slot 85 must be vx / run_speed, not some other normalisation"
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
fn the_gap_slot_fires_only_when_the_ground_runs_out_ahead() {
    let ledge = common::fixture("ledge", LEDGE, (9, 3), &[("goal", 1, 3)]);
    let mut sim = sim::Sim::new(&ledge, 41);
    common::settle(&mut sim);
    assert_eq!(
        sim.observation()[83],
        1.0,
        "standing on the last ground tile, the next {} columns are open",
        sensors::GAP_LOOKAHEAD
    );

    let solid = common::fixture("ground", GROUND, (8, 3), &[("goal", 1, 3)]);
    let mut sim2 = sim::Sim::new(&solid, 41);
    common::settle(&mut sim2);
    assert_eq!(sim2.observation()[83], 0.0, "ground continues ahead");
}

#[test]
fn the_goal_direction_slot_points_at_the_goal() {
    let right = common::fixture("goal-right", GROUND, (5, 3), &[("goal", 10, 3)]);
    let mut sim_right = sim::Sim::new(&right, 33);
    common::settle(&mut sim_right);
    assert_eq!(
        sim_right.observation()[88],
        1.0,
        "the goal is to the right, so slot 88 is +1"
    );

    let left = common::fixture("goal-left", GROUND, (5, 3), &[("goal", 1, 3)]);
    let mut sim_left = sim::Sim::new(&left, 33);
    common::settle(&mut sim_left);
    assert_eq!(
        sim_left.observation()[88],
        -1.0,
        "the goal is to the left, so slot 88 is -1"
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
}

/// Pinned hash of the observation vector at a fixed state.
///
/// The slot-by-slot tests above document the layout one field at a time; this
/// pins the whole 90-float vector so an accidental edit anywhere in the patch
/// loop is caught even if it lands in a slot nobody asserts on directly. If this
/// fails, the fix is a `sensors::VERSION` bump and a new constant -- never a
/// quiet update, because every champion file records the version (§8).
#[test]
fn golden_observation_hash_at_a_fixed_state() {
    const GOLDEN_OBSERVATION_HASH: u64 = 16_426_574_841_053_731_519;

    let level = common::fixture("ground", GROUND, (5, 3), &[("goal", 11, 3)]);
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
