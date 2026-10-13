//! Physics unit tests (§11): AABB collision corners, hazards, and enemy contact.
//!
//! Every fixture is a small ASCII-art level. Rows are top-down, `.` is air, `#` is
//! solid, `^` is a spike, `G` is a goal tile; `spawn` and entities are in tile
//! coordinates and an entity at `y` stands on the bottom edge of row `y`.

mod common;

use common::fixture;
use sim::physics::PHYSICS;
use sim::AgentAction;

/// Ground rows 4..5, open sky above. 12 columns wide.
const FLAT: &[&str] = &[
    "............",
    "............",
    "............",
    "............",
    "############",
    "############",
];

/// A ceiling block at columns 5..7 in row 2, ground rows 4..5.
const CEILING: &[&str] = &[
    "............",
    "............",
    ".....###....",
    "............",
    "############",
    "############",
];
const CEILING_ROW: i32 = 2;

/// A wall column at 9 in rows 2..3, ground rows 4..5.
const WALL: &[&str] = &[
    "............",
    "............",
    ".........#..",
    ".........#..",
    "############",
    "############",
];
const WALL_COLUMN: i32 = 9;

/// A spike sitting on the ground surface at column 4.
const SPIKE: &[&str] = &[
    "............",
    "............",
    "............",
    "....^.......",
    "############",
    "############",
];
const SPIKE_COLUMN: i32 = 4;

/// Ground that stops at column 9: running right walks off into a bottomless pit.
/// The goal sits behind the spawn so a falling body can never touch it.
const PIT: &[&str] = &[
    "............",
    "............",
    "............",
    "............",
    "##########..",
    "##########..",
];

/// A one-tile shaft at column 5, walled at columns 4 and 6, with a walker on the
/// floor. The walker patrols between the two walls and cannot leave column 5.
const SHAFT: &[&str] = &[
    "....#.#.....",
    "....#.#.....",
    "....#.#.....",
    "....#.#.....",
    "############",
    "############",
    "############",
    "############",
];
const SHAFT_COLUMN: i32 = 5;

#[test]
fn landing_settles_the_body_flush_on_the_surface() {
    let level = fixture("flat", FLAT, (1, 3), &[("goal", 11, 3)]);
    let surface = level.spawn.y + level.spawn.h;
    let mut sim = sim::Sim::new(&level, 7);
    common::step_n(&mut sim, AgentAction::Walk, 8);

    let player = sim.player();
    assert!(player.grounded, "the body should be standing after settling");
    assert_eq!(
        player.vy, 0.0,
        "a resting body must have zero vertical velocity"
    );
    assert!(
        (player.y - level.spawn.y).abs() < 1e-6,
        "the body should settle exactly where it spawned ({} vs {})",
        player.y,
        level.spawn.y
    );
    assert!(
        ((player.y + player.h) - surface).abs() < 1e-6,
        "the body's bottom {} should be flush with the surface {surface}",
        player.y + player.h
    );
    assert_eq!(sim.outcome(), sim::Outcome::Running);
}

#[test]
fn a_held_jump_peaks_between_the_tap_and_the_ideal_height() {
    let level = fixture("flat", FLAT, (1, 3), &[("goal", 11, 3)]);
    let surface = level.spawn.y + level.spawn.h;
    let mut sim = sim::Sim::new(&level, 11);
    common::settle(&mut sim);

    let mut min_bottom = f32::INFINITY;
    for _ in 0..80 {
        sim.step(AgentAction::JumpInPlace);
        let bottom = sim.player().y + sim.player().h;
        if bottom < min_bottom {
            min_bottom = bottom;
        }
    }
    let peak = surface - min_bottom;

    assert!(peak > 1.0, "the jump barely left the ground: peak {peak}");
    assert!(
        peak > PHYSICS.min_jump_height(),
        "a held jump (peak {peak}) must clear the shortest possible jump ({})",
        PHYSICS.min_jump_height()
    );
    assert!(
        peak <= PHYSICS.max_jump_height() + 1e-3,
        "a held jump (peak {peak}) must not exceed the ideal {} the physics bible \
         advertises (discrete Euler integration lands slightly under it)",
        PHYSICS.max_jump_height()
    );
    assert!(
        sim.player().grounded,
        "the body should be back on the ground after the flight"
    );
    assert_eq!(sim.outcome(), sim::Outcome::Running);
}

#[test]
fn a_head_bump_stops_the_body_flush_under_the_ceiling() {
    let level = fixture("ceiling", CEILING, (5, 3), &[("goal", 11, 3)]);
    let surface = level.spawn.y + level.spawn.h;
    let ceiling_bottom = sim::physics::bottom_of(CEILING_ROW);
    let mut sim = sim::Sim::new(&level, 13);
    common::settle(&mut sim);

    let mut min_y = f32::INFINITY;
    let mut min_bottom = f32::INFINITY;
    for _ in 0..60 {
        sim.step(AgentAction::JumpInPlace);
        min_y = min_y.min(sim.player().y);
        min_bottom = min_bottom.min(sim.player().y + sim.player().h);
    }

    assert!(
        (min_y - ceiling_bottom).abs() < 1e-4,
        "the body's top should stop flush against the ceiling at {ceiling_bottom}, \
         but reached {min_y} — a smaller value means the body tunnelled through"
    );
    assert!(
        surface - min_bottom < PHYSICS.min_jump_height(),
        "the ceiling should truncate the jump far below the shortest free jump \
         ({}), but the body rose {}",
        PHYSICS.min_jump_height(),
        surface - min_bottom
    );
    assert!(
        sim.player().grounded,
        "the body should fall back to the ground after the bump"
    );
    assert!(
        (sim.player().y - level.spawn.y).abs() < 1e-4,
        "and settle at its original height"
    );
}

#[test]
fn running_into_a_wall_stops_the_body_flush_against_the_wall_face() {
    let level = fixture("wall", WALL, (6, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 17);
    common::settle(&mut sim);

    let wall_face = WALL_COLUMN as f32;
    let expected_x = wall_face - PHYSICS.player_w;

    common::step_n(&mut sim, AgentAction::Run, 40);
    assert!(
        (sim.player().x - expected_x).abs() < 1e-4,
        "the body's left edge {} should rest flush against the wall face at \
         {expected_x}",
        sim.player().x
    );
    assert_eq!(
        sim.player().vx,
        0.0,
        "a blocked body keeps no horizontal velocity"
    );
    assert!(
        sim.player().x + sim.player().w <= wall_face + 1e-4,
        "the body must never overlap the wall"
    );

    // Pushing harder must not tunnel through.
    common::step_n(&mut sim, AgentAction::Run, 40);
    assert!(
        (sim.player().x - expected_x).abs() < 1e-4,
        "the body moved after being blocked: {}",
        sim.player().x
    );

    // And the other direction still works, so the block is not sticky.
    common::step_n(&mut sim, AgentAction::Retreat, 30);
    assert!(
        sim.player().vx < 0.0,
        "retreating should build leftward velocity, got {}",
        sim.player().vx
    );
    assert!(
        sim.player().x < expected_x,
        "retreating should move the body away from the wall"
    );
    assert_eq!(sim.outcome(), sim::Outcome::Running);
}

#[test]
fn spike_contact_ends_the_episode_and_records_where_it_happened() {
    let level = fixture("spike", SPIKE, (2, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 19);
    common::settle(&mut sim);

    let steps = common::run_until_finished(&mut sim, AgentAction::Run, 400);
    assert!(steps < 400, "the run should have ended, not hit the cap");
    assert_eq!(sim.outcome(), sim::Outcome::Died);
    let (dx, dy) = sim
        .stats()
        .death_pos
        .expect("a death must record a position");

    assert!(
        dx >= (SPIKE_COLUMN - 1) as f32 && dx < (SPIKE_COLUMN + 1) as f32,
        "death at x = {dx} should be right beside the spike column {SPIKE_COLUMN}"
    );
    assert!(
        (3.0..=4.0).contains(&dy),
        "death at y = {dy} should be at ground level"
    );
}

#[test]
fn running_off_the_end_of_the_ground_falls_out_of_the_world_and_kills() {
    let level = fixture("pit", PIT, (8, 3), &[("goal", 1, 3)]);
    let mut sim = sim::Sim::new(&level, 23);
    common::settle(&mut sim);

    let steps = common::run_until_finished(&mut sim, AgentAction::Run, 400);
    assert!(steps < 400, "the fall should have ended, not hit the cap");
    assert_eq!(sim.outcome(), sim::Outcome::Died);
    let (_, dy) = sim
        .stats()
        .death_pos
        .expect("a pit death must record a position");
    assert!(
        dy > level.height as f32,
        "a body that fell out should be below the grid (y = {dy}, height {})",
        level.height
    );
}

#[test]
fn stomping_a_walker_from_above_kills_it_and_bounces_the_player() {
    let level = fixture(
        "shaft",
        SHAFT,
        (SHAFT_COLUMN, 0),
        &[("walker", SHAFT_COLUMN, 3), ("goal", 10, 3)],
    );
    assert_eq!(level.walkers.len(), 1, "the shaft fixture ships one walker");

    let mut sim = sim::Sim::new(&level, 29);
    assert!(
        !sim.player().grounded,
        "the body starts in mid-air at the top of the shaft"
    );

    let mut steps = 0;
    while sim.stats().stomps == 0 && !sim.outcome().is_finished() && steps < 200 {
        sim.step(AgentAction::Walk);
        steps += 1;
    }

    assert_eq!(
        sim.stats().stomps,
        1,
        "dropping down the shaft should stomp the walker exactly once"
    );
    assert!(!sim.walkers()[0].alive, "a stomped walker is removed");
    assert_ne!(
        sim.outcome(),
        sim::Outcome::Died,
        "a stomp is not a side-hit"
    );
    assert!(
        sim.player().vy < 0.0,
        "a stomp bounces the body upward, got vy = {}",
        sim.player().vy
    );
    assert!(
        steps <= 60,
        "the stomp should happen early in the fall, but took {steps} ticks"
    );
    assert_eq!(sim.stats().death_pos, None);
}

#[test]
fn walking_into_a_walker_at_ground_level_is_a_side_hit_and_kills_the_player() {
    let level = fixture("open", FLAT, (2, 3), &[("walker", 6, 3), ("goal", 11, 3)]);
    assert_eq!(level.walkers.len(), 1, "the fixture ships one walker");

    let mut sim = sim::Sim::new(&level, 31);
    common::settle(&mut sim);

    let steps = common::run_until_finished(&mut sim, AgentAction::Run, 480);
    assert!(
        steps < 480,
        "the side-hit should have ended the run, not the cap"
    );
    assert_eq!(sim.outcome(), sim::Outcome::Died);
    assert_eq!(sim.stats().stomps, 0, "a side-hit is not a stomp");
    assert!(
        sim.walkers()[0].alive,
        "the walker survives a side-hit; only stomps remove it"
    );
    let (dx, _) = sim
        .stats()
        .death_pos
        .expect("a death must record a position");

    // Where the two bodies meet depends on which way the walker happens to be
    // patrolling: its spawn phase is seeded (`episode_variation.rs`), so a
    // hard-coded encounter coordinate would be a copy of one seed's luck. Assert
    // the physics relationship instead.
    let walker = sim.walkers()[0];
    assert!(
        sim.player().aabb().overlaps(&walker.aabb(&sim::PHYSICS)),
        "death must happen in contact with the walker: player {:?}, walker {:?}",
        sim.player().aabb(),
        walker.aabb(&sim::PHYSICS)
    );
    assert!(
        dx > 3.0 && dx < 11.0,
        "the bodies should meet in open ground, not at the spawn (x = 2.2) or past the goal: {dx}"
    );
}
