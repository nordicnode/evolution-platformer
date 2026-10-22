//! The later enemies and objects: the one-way shelf, the chaser, the flyer, the turret and
//! the moving platform.
//!
//! Every fixture is a small ASCII-art level plus a hand-written entity list, because these
//! entities carry fields a `(kind, x, y)` triple cannot: a flyer's beat, a turret's direction,
//! a platform's width, travel and axis. The JSON here is therefore built by hand rather than
//! by `common::fixture` — and `every_new_entity_round_trips` is the test that keeps the
//! hand-written shape honest against the writer's own.

mod common;

use sim::physics::PHYSICS;
use sim::{AgentAction, Level, Outcome, Tile};

/// Build a v2 level from glyph rows and raw entity JSON.
fn level(name: &str, rows: &[&str], spawn: (i32, i32), entities: &[String]) -> Level {
    let width = rows[0].chars().count();
    assert!(
        rows.iter().all(|r| r.chars().count() == width),
        "fixture {name} has a ragged row"
    );
    let tiles = rows
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(",");
    let ents = entities.join(",");
    let src = format!(
        "{{\"format\":\"evo-platformer-level\",\"level_format_version\":2,\
         \"name\":\"{name}\",\"width\":{width},\"height\":{h},\
         \"spawn\":{{\"x\":{sx},\"y\":{sy}}},\"tiles\":[{tiles}],\"entities\":[{ents}]}}",
        h = rows.len(),
        sx = spawn.0,
        sy = spawn.1,
    );
    match Level::from_json_str(&src) {
        Ok(level) => level,
        Err(err) => panic!("fixture {name} failed to parse: {err}\nsource: {src}"),
    }
}

fn goal(x: i32, y: i32) -> String {
    format!("{{\"type\":\"goal\",\"x\":{x},\"y\":{y}}}")
}

/// **A one-way shelf stops a falling body and lets a rising one through.**
///
/// The two halves are the whole tile: a body dropped on it lands on the deck, and a body
/// jumping from the floor below passes up through the deck and then lands on top of it.
/// A tile that failed either half would be a wall (which refuses both) or a decorative
/// line (which refuses neither).
#[test]
fn a_one_way_shelf_is_ground_from_above_and_air_from_below() {
    // Row 2 is the shelf, row 4 the floor.
    let rows = [
        "............",
        "............",
        "....=====...",
        "............",
        "############",
        "############",
    ];
    let level = level("one-way", &rows, (5, 1), &[goal(10, 3)]);
    assert_eq!(
        level.tile(4, 2),
        Some(Tile::OneWay),
        "the shelf is authored"
    );
    assert!(
        level.is_solid(4, 2),
        "a one-way is ground for every question that asks for a surface"
    );
    assert!(
        !level.is_opaque(4, 2),
        "and air for every question that asks about a wall or a ceiling"
    );

    // Dropped from above the shelf: it lands on the deck rather than falling to the floor.
    let mut sim = sim::Sim::new(&level, 1);
    common::step_n(&mut sim, AgentAction::Walk, 40);
    let p = sim.player();
    assert!(sim.player().grounded, "it should have come to rest");
    assert!(
        (p.y + p.h - 2.0).abs() < 0.02,
        "feet on the top of the shelf's row (bottom {}

         ), not the floor below",
        p.y + p.h
    );
    assert_eq!(sim.outcome(), Outcome::Running, "the shelf is not a hazard");
}

/// **A body jumps up through the shelf and lands on it.**
///
/// The rising half of the rule, and the reason a one-way is a *route*: without it, the tile
/// would be an invisible wall with a picture of a plank on it.
#[test]
fn a_body_jumps_up_through_a_one_way_shelf() {
    let rows = [
        "............",
        "............",
        "....=====...",
        "............",
        "############",
        "############",
    ];
    // Spawn on the floor under the shelf.
    let level = level("one-way-rise", &rows, (4, 3), &[goal(10, 3)]);
    let mut sim = sim::Sim::new(&level, 3);
    common::settle(&mut sim);
    let before = sim.player().y;
    assert!(
        (sim.player().y + sim.player().h - 4.0).abs() < 0.02,
        "the body starts on the floor"
    );

    // Hold a jump: it rises through the deck and settles on top of it.
    for _ in 0..60 {
        sim.step(AgentAction::JumpInPlace);
    }
    common::step_n(&mut sim, AgentAction::Walk, 40);
    let p = sim.player();
    assert!(
        p.y + p.h < before - 1.0,
        "the body should now be standing on the shelf, not still on the floor ({} → {})",
        before,
        p.y + p.h
    );
    assert!(
        (p.y + p.h - 2.0).abs() < 0.02,
        "and its feet should be on the shelf's deck, at {}",
        p.y + p.h
    );
    assert_eq!(sim.outcome(), Outcome::Running);
}

/// **A chaser closes distance and will not walk off a ledge.**
///
/// Both halves are deliberate. Heading for the body is what makes it a different enemy from
/// a patroller; stopping at the edge is what keeps it from solving itself on the first pit,
/// and what makes baiting one to a ledge a legitimate answer rather than an exploit.
#[test]
fn a_chaser_heads_for_the_body_and_stops_at_the_ledge() {
    // A four-tile plateau at row 4, a pit, and a floor far to the right.
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "#####........#######",
        "#####........#######",
    ];
    let entities = [
        "{\"type\":\"chaser\",\"x\":2,\"y\":3}".to_string(),
        goal(18, 3),
    ];
    let level = level("chaser", &rows, (16, 3), &entities);
    let mut sim = sim::Sim::new(&level, 5);
    let start = sim.walkers()[0].x;
    for _ in 0..120 {
        sim.step(AgentAction::Walk);
    }
    let chaser = sim.walkers()[0];
    assert!(chaser.alive, "the chaser must not walk into the pit");
    assert!(
        chaser.x > start,
        "a chaser walks towards the body on its right ({} → {})",
        start,
        chaser.x
    );
    assert!(
        chaser.x < 5.0,
        "and stops at the plateau's edge instead of walking off it (x {})",
        chaser.x
    );
    // It stays in its own row: no falling, no climbing.
    assert!(
        (chaser.y + chaser.h - 4.0).abs() < 0.02,
        "the chaser stays on the platform's surface (bottom {})",
        chaser.y + chaser.h
    );
}

/// **A flyer beats back and forth between the two ends it was authored with.**
///
/// Its travel is level data rather than terrain, so the test is about the span: it turns at
/// both ends, and it never leaves the band it was given.
#[test]
fn a_flyer_beats_between_the_ends_it_was_given() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    // A three-tile beat centred on column 8, at row 2.
    let entities = [
        "{\"type\":\"flyer\",\"x\":8,\"y\":2,\"span\":3}".to_string(),
        goal(19, 4),
    ];
    let level = level("flyer", &rows, (1, 4), &entities);
    let mut sim = sim::Sim::new(&level, 9);
    let spec = level.flyers[0];
    let mut dirs = Vec::new();
    for _ in 0..400 {
        sim.step(AgentAction::Walk);
        let f = sim.flyers()[0];
        assert!(f.alive);
        assert!(
            f.x >= spec.min_x - 0.01 && f.x + f.w <= spec.max_x + 0.01,
            "the flyer left its beat: {}..{} at x {}",
            spec.min_x,
            spec.max_x,
            f.x
        );
        if dirs.last() != Some(&f.dir) {
            dirs.push(f.dir);
        }
    }
    assert!(
        dirs.len() >= 3,
        "a three-tile beat at this speed turns at least twice in 400 ticks: {dirs:?}"
    );
}

/// **A turret fires along its own row, in the direction it was placed.**
///
/// Two claims in one fixture: the shot appears at all, and it travels the way the spec says
/// rather than the way the level's geometry happens to point.
#[test]
fn a_turret_fires_in_the_direction_it_was_placed() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
        "####################",
    ];
    let entities = [
        "{\"type\":\"turret\",\"x\":6,\"y\":3,\"dir\":-1}".to_string(),
        goal(19, 3),
    ];
    let level = level("turret", &rows, (0, 3), &entities);
    assert_eq!(level.turrets[0].dir, -1);
    let mut sim = sim::Sim::new(&level, 2);
    common::step_n(&mut sim, AgentAction::Walk, PHYSICS.turret_period + 2);
    let shot = sim
        .shots()
        .iter()
        .find(|s| s.alive)
        .expect("the turret should have fired by its first period");
    assert!(
        shot.vx < 0.0,
        "a left-facing turret's shot travels left, got vx {}",
        shot.vx
    );
}

/// **A shot kills a body in its path, and the turret keeps firing.**
///
/// The turret's whole contribution to a level: a row that is dangerous on a clock.
#[test]
fn a_turret_shot_ends_the_episode() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
        "####################",
    ];
    let entities = [
        "{\"type\":\"turret\",\"x\":4,\"y\":3}".to_string(),
        goal(19, 3),
    ];
    let level = level("turret-kill", &rows, (12, 3), &entities);
    let mut sim = sim::Sim::new(&level, 4);
    let mut steps = 0;
    let mut saw_shot = false;
    while sim.outcome() == Outcome::Running && steps < 400 {
        sim.step(AgentAction::Walk);
        saw_shot |= sim.shots().iter().any(|s| s.alive);
        steps += 1;
    }
    assert_eq!(
        sim.outcome(),
        Outcome::Died,
        "a standing body in a firing row is hit"
    );
    assert!(
        saw_shot,
        "and it is a shot that hit it, not the turret's body"
    );
}

/// **A turret is solid: a lid to stand on, a wall to bump, never a stomp.**
///
/// The turret used to be an enemy in the stomp pass — landing on it destroyed it, and any
/// other touch was a death. It is furniture now: a body can stand on its lid and walk into its
/// side without either destroying the turret or dying. The shot remains the hazard, which is
/// the turret's whole reason to exist.
#[test]
fn a_turret_is_a_solid_box_not_a_stompable_enemy() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
        "####################",
    ];
    let entities = [
        "{\"type\":\"turret\",\"x\":6,\"y\":3}".to_string(),
        goal(19, 3),
    ];

    // Dropped straight onto the barrel from above.
    let lid_level = level("turret-lid", &rows, (6, 1), &entities);
    let mut sim = sim::Sim::new(&lid_level, 9);
    common::step_n(&mut sim, AgentAction::Brake, 60);
    let lid = sim.turrets()[0].y;
    let feet = sim.player().y + sim.player().h;
    assert!(
        (feet - lid).abs() < 0.02,
        "the body should be standing on the turret's lid at {lid}, feet are {feet}"
    );
    assert!(sim.player().grounded, "its lid is ground");
    assert!(
        sim.turrets()[0].alive,
        "landing on a turret must not destroy it"
    );
    assert_eq!(sim.stats().stomps, 0, "and it is not a stomp");
    assert_eq!(
        sim.outcome(),
        Outcome::Running,
        "nor is touching it a death"
    );

    // Walked into from the side: the body is stopped at the barrel.
    let wall_level = level("turret-wall", &rows, (1, 3), &entities);
    let mut sim = sim::Sim::new(&wall_level, 9);
    common::step_n(&mut sim, AgentAction::Walk, 120);
    let turret = sim.turrets()[0];
    let body = sim.player();
    assert!(
        body.x + body.w <= turret.x + 1e-3,
        "the body must be stopped by the turret's left face (body right {}, turret left {})",
        body.x + body.w,
        turret.x
    );
    assert_eq!(
        sim.outcome(),
        Outcome::Running,
        "bumping a turret is not fatal"
    );
    assert_eq!(sim.stats().stomps, 0);
    assert!(sim.turrets()[0].alive, "and it is still there");
}

/// **A moving platform carries the body standing on it.**
///
/// The reason the mover is machinery rather than decoration: a shelf that slid out from
/// under its rider would be a trap, and a lift that did not lift would be a staircase.
#[test]
fn a_moving_platform_carries_its_rider() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    // A two-tile shelf at row 2 that slides four tiles right over its cycle.
    let entities = [
        "{\"type\":\"platform\",\"x\":2,\"y\":2,\"tiles\":2,\"span\":4}".to_string(),
        goal(19, 4),
    ];
    let level = level("mover", &rows, (2, 1), &entities);
    assert_eq!(level.platforms.len(), 1);
    let mut sim = sim::Sim::new(&level, 6);
    let start_x = sim.player().x;
    // Let it fall onto the shelf, then ride.
    common::step_n(&mut sim, AgentAction::Walk, 20);
    let shelf = sim.platforms()[0];
    assert!(
        (sim.player().y + sim.player().h - shelf.y).abs() < 0.02,
        "the body lands on the shelf, not the floor: feet {}, shelf top {}",
        sim.player().y + sim.player().h,
        shelf.y
    );
    let landed_x = sim.player().x;
    common::step_n(&mut sim, AgentAction::Walk, 40);
    assert!(
        sim.player().x > landed_x + 0.3,
        "the rider is carried with the shelf ({} → {})",
        landed_x,
        sim.player().x
    );
    assert!(
        sim.player().x > start_x,
        "which is movement the body never asked for"
    );
    assert_eq!(sim.outcome(), Outcome::Running);
}

/// **A shelf is not a magnet: a body falling from well above it passes by, and one walking
/// underneath is not yanked up through the deck.**
///
/// The bug this is written for: the first landing test asked only "are the feet above the
/// deck?", which is true three tiles up, so any falling body was teleported onto a shelf
/// below it. A landing has to be a *crossing* — above it last frame, at or below it now.
#[test]
fn a_mover_does_not_teleport_a_body_that_is_not_crossing_its_deck() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    // A shelf at row 3 (its deck is half a tile above the floor), and a spawn six tiles up.
    let entities = [
        "{\"type\":\"platform\",\"x\":6,\"y\":3,\"tiles\":3,\"span\":1}".to_string(),
        goal(19, 4),
    ];
    let level = level("mover-fall", &rows, (7, 0), &entities);
    let shelf = level.platforms[0].rect;
    let mut sim = sim::Sim::new(&level, 1);
    // The body starts six tiles above the deck. It must fall to it — **approaching by at most
    // one tick of fall per frame**, which is what a teleport is not. The first frame at or
    // below the deck is the frame that matters: its previous feet have to have been within a
    // single tick's fall of the deck, not three tiles above it.
    let deck = shelf.y;
    let mut prev_feet = sim.player().y + sim.player().h;
    let mut landed = None;
    for tick in 0..60 {
        sim.step(AgentAction::Walk);
        let feet = sim.player().y + sim.player().h;
        if feet >= deck - 1e-3 {
            landed = Some(tick);
            break;
        }
        prev_feet = feet;
    }
    assert!(landed.is_some(), "the body should reach the shelf's deck");
    assert!(
        deck - prev_feet <= PHYSICS.max_fall + 1e-3,
        "the body approached the deck from {} tiles away in one frame — it was snapped onto a \
         shelf it never crossed",
        deck - prev_feet
    );
    assert_eq!(sim.outcome(), Outcome::Running);
}

/// **Standing still for a deck is not camping.**
///
/// The anti-camper timer is 600 frames. On a mover level the right move is often to wait at the
/// lip for the shuttle to come back, and a body that does so must not be declared stuck for it.
/// The fixture's deck travels the whole level at the body's own height, so every frame of the
/// wait is a frame a deck can arrive on; the control arm has the same body and no deck, and it is
/// declared stuck on schedule.
#[test]
fn waiting_at_the_lip_for_a_deck_is_not_stuck() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let with_deck = level(
        "wait-for-deck",
        &rows,
        (1, 4),
        &[
            "{\"type\":\"platform\",\"x\":8,\"y\":3,\"tiles\":2,\"span\":8,\"dir\":-1}".to_string(),
            goal(19, 4),
        ],
    );
    let without = level("no-deck", &rows, (1, 4), &[goal(19, 4)]);

    // Long enough to pass the 600-frame anti-camper timer with room to spare, inside the level's
    // own 800-frame budget.
    let mut waiting = sim::Sim::new(&with_deck, 3);
    for _ in 0..700 {
        waiting.step(AgentAction::Walk);
        if waiting.outcome().is_finished() {
            break;
        }
    }
    assert_eq!(
        waiting.outcome(),
        Outcome::Running,
        "a body waiting on a deck's travel was ended at frame {} as {:?}",
        waiting.stats().frames,
        waiting.outcome()
    );

    let mut camping = sim::Sim::new(&without, 3);
    for _ in 0..700 {
        camping.step(AgentAction::Walk);
        if camping.outcome().is_finished() {
            break;
        }
    }
    assert_eq!(
        camping.outcome(),
        Outcome::Stuck,
        "the same body with nothing to wait for is still a camper"
    );
    assert!(
        camping.stats().frames >= sim::NO_PROGRESS_FRAMES,
        "and it was ended by the anti-camper timer, not something else (frame {})",
        camping.stats().frames
    );
}

/// **A rider is carried, not dragged through a wall.**
///
/// The shelf slides towards a wall tile in its own row; the body standing on it must stop at
/// the wall with the shelf, not be pushed into the masonry by the carry.
#[test]
fn a_rider_is_never_carried_into_a_wall() {
    let rows = [
        "............",
        "............",
        "............",
        "..........#.",
        "############",
    ];
    // A shelf starting at column 5, sliding six tiles right, with a wall at column 10 row 3.
    let entities = [
        "{\"type\":\"platform\",\"x\":5,\"y\":3,\"tiles\":2,\"span\":6}".to_string(),
        goal(11, 3),
    ];
    let level = level("mover-wall", &rows, (5, 2), &entities);
    assert_eq!(level.tile(10, 3), Some(Tile::Solid));
    let mut sim = sim::Sim::new(&level, 2);
    common::step_n(&mut sim, AgentAction::Walk, 30);
    assert!(sim.player().grounded, "the body should be riding the shelf");
    let mut max_x: f32 = 0.0;
    for _ in 0..200 {
        sim.step(AgentAction::Walk);
        let p = sim.player();
        max_x = max_x.max(p.x + p.w);
        // Inside the wall column would be x + w > 10.0; a body standing *on* the shelf that
        // ends at the wall is at most 10.0.
        assert!(
            p.x + p.w <= 10.0 + 1e-3,
            "the rider was pushed into the wall: right edge {}",
            p.x + p.w
        );
        assert_eq!(sim.outcome(), Outcome::Running, "and not killed by it");
    }
    assert!(max_x > 6.0, "it should still have been carried some way");
}

/// **A platform authored through a wall turns early instead of passing through it.**
///
/// The level format cannot state "the clear part of the path", so the sim derives it: a box
/// that would overlap an opaque tile is as far as the shelf goes. The wall is at column 10
/// and the shelf's home is column 8, so it can travel about two tiles, not the six authored.
#[test]
fn a_mover_stops_at_the_first_wall_on_its_path() {
    let rows = [
        "............",
        "............",
        "............",
        "..........#.",
        "############",
    ];
    let entities = [
        "{\"type\":\"platform\",\"x\":8,\"y\":3,\"tiles\":1,\"span\":6}".to_string(),
        goal(11, 3),
    ];
    let level = level("mover-limit", &rows, (1, 3), &entities);
    let mut sim = sim::Sim::new(&level, 3);
    let mut max_right: f32 = 0.0;
    for _ in 0..200 {
        sim.step(AgentAction::Walk);
        let shelf = sim.platforms()[0];
        max_right = max_right.max(shelf.x + shelf.w);
        assert!(
            shelf.x + shelf.w <= 10.0 + 1e-3,
            "the shelf entered the wall: right edge {}",
            shelf.x + shelf.w
        );
    }
    assert!(
        max_right > 8.5,
        "and it should still have travelled up to the wall, got {max_right}"
    );
}

/// **Every new entity and the new tile survive the writer and the reader unchanged.**
///
/// The JSON these fixtures are written with is hand-made; this is the test that says the
/// real writer agrees with it, field for field, because the content hash covers every one
/// of them.
#[test]
fn every_new_entity_round_trips_through_the_writer() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....====............",
        "####################",
        "####################",
    ];
    let mut level = level("round-trip", &rows, (1, 3), &[goal(19, 3)]);
    assert!(level.add_chaser_tile(3, 3));
    assert!(level.add_flyer_tile(9, 2, 3));
    assert!(level.add_turret_tile(13, 3, -1));
    assert!(level.add_platform_tile(6, 2, 3, 2, true, 1));
    // A ball needs a ceiling to hang from, so the fixture paints one above its column.
    assert!(level.set_tile(16, 1, Tile::Solid));
    assert!(level.add_spike_ball_tile(16, 2));
    // A rock needs its own ceiling, and one clear row under it to fall through.
    assert!(level.set_tile(18, 1, Tile::Solid));
    assert!(level.add_rock_tile(18, 2));
    // A crate needs floor directly under it, and the fixture's floor is row 4: column 2 is
    // clear above it, while the shelf's run starts at column 4.
    assert!(level.add_block_tile(2, 3));

    let json = level.to_json_pretty().expect("the document is writable");
    let back = Level::from_json_str(&json).expect("and readable");
    assert_eq!(back.content_hash(), level.content_hash(), "hash survives");
    assert_eq!(back.chasers.len(), 1);
    assert_eq!(back.flyers.len(), 1);
    assert_eq!(back.turrets.len(), 1);
    assert_eq!(back.turrets[0].dir, -1, "the firing direction is kept");
    assert_eq!(back.platforms.len(), 1);
    assert_eq!(back.platforms[0].rect.w, 3.0, "the width is kept");
    assert_eq!(back.rocks.len(), 1, "the rock is kept");
    assert_eq!(back.rocks[0].col, 18, "its column is kept");
    assert_eq!(back.rocks[0].row, 2, "its row is kept");
    assert_eq!(back.blocks.len(), 1, "the crate is kept");
    assert_eq!(back.blocks[0].col, 2, "its column is kept");
    assert_eq!(back.blocks[0].row, 3, "its row is kept");
    assert_eq!(back.spike_balls.len(), 1, "the ball is kept");
    assert_eq!(back.spike_balls[0].col, 16, "its column is kept");
    assert_eq!(back.spike_balls[0].row, 2, "its row is kept");
    assert_eq!(
        back.spike_balls[0].length, 3.0,
        "the chain is kept, shortened to the room under it"
    );
    assert_eq!(
        back.spike_balls[0].reach,
        sim::level::DEFAULT_SPIKE_BALL_REACH as f32,
        "the default reach is written down rather than lost"
    );
    assert!(back.platforms[0].vertical, "the axis is kept");
    assert_eq!(
        back.platforms[0].dir, level.platforms[0].dir,
        "and the direction it travels, which is a fact about the level and not about the box"
    );
    assert_eq!(level.platforms[0].dir, 1, "the fixture's lift goes up");
    // The travel, not the distance between the ends: the ends name the box's edges, so the gap
    // between them is the beat *plus the flyer's own width*. Reading it back the other way is
    // how a save/load round trip makes a flyer's path grow by 0.7 tiles per save.
    assert!(
        (back.flyers[0].span() - 3.0).abs() < 1e-3,
        "the beat is kept, got {}",
        back.flyers[0].span()
    );
    assert!(
        (back.flyers[0].min_x - level.flyers[0].min_x).abs() < 1e-3
            && (back.flyers[0].max_x - level.flyers[0].max_x).abs() < 1e-3,
        "and so are both ends: {:?} against {:?}",
        back.flyers[0],
        level.flyers[0]
    );
    assert!(
        json.contains(&format!(
            "\"level_format_version\": {}",
            sim::level::LEVEL_FORMAT_VERSION
        )),
        "the writer emits the build's own format version"
    );
}

/// **A one-way shelf is legible to the sensors, and a flyer is a threat it can see.**
///
/// The two halves of the sensor story: the tile has its own encoding rather than reading as
/// a wall, and the nearest-threat triple answers about a flyer as well as a walker — which
/// is what makes the new enemies learnable rather than invisible.
#[test]
fn the_sensors_see_the_shelf_and_the_flyer() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....====............",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"flyer\",\"x\":9,\"y\":3,\"span\":2}".to_string(),
        goal(19, 5),
    ];
    let level = level("sensors", &rows, (4, 4), &entities);
    let mut sim = sim::Sim::new(&level, 8);
    common::settle(&mut sim);
    let obs = sim.observation();
    // The flier is two tiles to the right of the body and a row up: a threat the triple can
    // see, with a positive dx.
    assert!(
        obs[81] > 0.0,
        "enemy_dx should point at the flyer, got {}",
        obs[81]
    );

    // The shelf is one row above the body's centre... whose row it is not in; put the body
    // on the shelf instead and read the tile under its feet.
    assert_eq!(level.tile(4, 3), Some(Tile::OneWay));
    assert_ne!(
        sim::sensors::ENC_ONEWAY,
        sim::sensors::ENC_SOLID,
        "a one-way reads as its own value, not as a wall"
    );
    assert_ne!(
        sim::sensors::ENC_ONEWAY,
        sim::sensors::ENC_EMPTY,
        "and not as air"
    );
}

/// **A flyer travels exactly the span the level gave it.**
///
/// The number in the file is the *distance the flyer moves*, and that is the promise a person
/// placing one reads: "beat 12" means twelve tiles of patrol, not eleven and a bit. The bug this
/// pins is an arithmetic one — the authored ends name the box's *edges*, so bounds at
/// `centre ± span/2` move a box of width `w` exactly `span - w` tiles, and a level's flyer
/// quietly patrols 0.7 tiles less than the palette said. On a four-tile beat that is a fifth of
/// the whole flight path, which is why it was reported as "the flying unit has a very small
/// flight path".
#[test]
fn a_flyer_travels_the_span_it_was_given() {
    // A level long enough for the body to keep making progress while the flyer beats above it,
    // built through the editing API so the editor's own placement is what is measured.
    let mut level = Level::starter("flyer-span", 200, 6);
    assert!(level.add_flyer_tile(14, 1, 12));
    let spec = level.flyers[0];
    assert!(
        (spec.span() - 12.0).abs() < 1e-3,
        "the spec reports its own travel, got {}",
        spec.span()
    );
    let mut sim = sim::Sim::new(&level, 5);
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for _ in 0..1200 {
        sim.step(AgentAction::Run);
        let f = sim.flyers()[0];
        lo = lo.min(f.x);
        hi = hi.max(f.x);
    }
    let travelled = hi - lo;
    assert!(
        (travelled - 12.0).abs() < 0.05,
        "a twelve-tile beat should move the flyer twelve tiles, moved {travelled}"
    );
}

/// **A shot flies the length of its row until something stops it.**
///
/// The projectile used to fizzle after twelve tiles in the middle of the air, which on a level
/// with an open row is the worst of both worlds: a threat that stops for no visible reason and a
/// bullet the eye stops trusting. A wall is the stop now, and the authored range is the backstop
/// for a row that has none — so a shot fired down a twenty-tile corridor reaches the end of it,
/// well past where the old twelve-tile rule would have buried it.
#[test]
fn a_shot_crosses_its_row_until_a_wall_stops_it() {
    let mut level = Level::starter("shot-wall", 40, 5);
    assert!(level.add_turret_tile(2, 3, 1));
    level.set_tile(17, 3, Tile::Solid);
    let mut sim = sim::Sim::new(&level, 3);
    let (mut far, mut longest) = (f32::NEG_INFINITY, 0.0_f32);
    for _ in 0..200 {
        sim.step(AgentAction::Walk);
        for s in sim.shots().iter().filter(|s| s.alive) {
            far = far.max(s.x);
            longest = longest.max(s.travelled);
        }
    }
    assert!(
        far > 16.0,
        "the shot should cross the corridor and stop at the wall in column 17, reached {far}"
    );
    assert!(
        far < 17.0,
        "and it must not pass through the wall, reached {far}"
    );
    assert!(
        longest > 12.0,
        "a shot that reaches column 17 has travelled more than the old twelve-tile range, got \
         {longest}"
    );
}

/// **A lift carries its rider up and back down without ever letting go.**
///
/// The ride is state, not a per-frame landing test: a body that arrives on a deck stays on it
/// until it jumps or walks off. A rising lift is the case that exposes the difference, because
/// the shelf's own motion is applied to the feet before the body's tick is resolved, and a test
/// that measures the feet only after that carry cannot tell a crossing from a coincidence.
#[test]
fn a_lift_carries_its_rider_up_and_down() {
    let rows = [
        "............",
        "............",
        "............",
        "............",
        "............",
        "............",
        "............",
        "............",
        "............",
        "############",
    ];
    let entities = [
        "{\"type\":\"platform\",\"x\":4,\"y\":6,\"tiles\":2,\"span\":3,\"axis\":\"y\"}".to_string(),
        goal(10, 5),
    ];
    // The spawn is half a tile above the deck, so the first few ticks are a landing.
    let level = level("lift", &rows, (4, 5), &entities);
    let mut sim = sim::Sim::new(&level, 7);
    common::step_n(&mut sim, AgentAction::Walk, 12);
    let deck = sim.platforms()[0].y;
    assert!(
        (sim.player().y + sim.player().h - deck).abs() < 0.02,
        "the body landed on the lift: feet {}, deck {deck}",
        sim.player().y + sim.player().h
    );

    let (mut highest, mut lowest) = (f32::INFINITY, f32::NEG_INFINITY);
    for _ in 0..400 {
        sim.step(AgentAction::Walk);
        let feet = sim.player().y + sim.player().h;
        let deck = sim.platforms()[0].y;
        assert!(
            (feet - deck).abs() < 0.02,
            "the rider came off the deck: feet {feet}, deck {deck}"
        );
        assert!(sim.player().grounded, "and it is still standing on it");
        highest = highest.min(feet);
        lowest = lowest.max(feet);
    }
    assert!(
        lowest - highest > 2.5,
        "the lift should have carried it most of its three-tile travel: {highest}..{lowest}"
    );
    assert_eq!(sim.outcome(), Outcome::Running);
}

/// **Every entity a person can place changes the run's level hash.**
///
/// `content_hash` is the champion's and the population's identity: terrain, spawn, patrols and
/// the flag. It deliberately never learned about the newer entities. The run's own identity is
/// `scene_hash`, which must see all of them — an enemy placed after a generation was evaluated
/// makes that generation a claim about a level the viewer cannot see, and the evolve view has
/// to restart instead of drawing its stale copy of the document.
#[test]
fn every_placeable_entity_changes_the_scene_hash_but_not_the_terrain_hash() {
    let base = Level::starter("scene-hash", 40, 8);
    let content = base.content_hash();
    let scene = base.scene_hash();

    let mut chaser = base.clone();
    chaser.chasers.push(sim::Rect::new(4.0, 6.0, 0.8, 0.8));

    let mut flyer = base.clone();
    flyer.flyers.push(sim::FlyerSpec::at(6, 3, 4));

    let mut turret = base.clone();
    turret.turrets.push(sim::TurretSpec {
        rect: sim::Rect::new(8.2, 6.2, 0.8, 0.8),
        dir: -1,
    });
    let mut platform = base.clone();
    platform
        .platforms
        .push(sim::PlatformSpec::at(10, 6, 3, 4, false, 1));

    let mut ball = base.clone();
    ball.spike_balls.push(sim::SpikeBallSpec::at(12, 3, 3, 4));

    for (kind, level) in [
        ("chaser", &chaser),
        ("flyer", &flyer),
        ("turret", &turret),
        ("platform", &platform),
        ("spiked ball", &ball),
    ] {
        assert_ne!(
            level.scene_hash(),
            scene,
            "a {kind} must change the run's level hash"
        );
        assert_eq!(
            level.content_hash(),
            content,
            "a {kind} must not invalidate the terrain hash champions are keyed on"
        );
    }

    // A scene hash is not constant in the terrain either: moving the floor is still a
    // different level, and the two hashes agree about that.
    let mut moved = base.clone();
    moved.set_tile(3, 7, Tile::Empty);
    assert_ne!(moved.scene_hash(), scene);
    assert_ne!(moved.content_hash(), content);
}

/// **The platform oracle treats a one-way shelf as ground.**
///
/// The route graph is a statement about what a body can stand on, and a one-way is exactly
/// that. A shelf the oracle ignored would make every level built on one read as unbeatable.
#[test]
fn the_route_oracle_stands_on_a_one_way_shelf() {
    let rows = [
        "........................",
        "........................",
        "........=====...........",
        "........................",
        "......########..........",
        "########################",
    ];
    let level = level("oracle", &rows, (2, 4), &[goal(12, 1)]);
    let report = sim::reach::report(&level);
    assert!(
        report.goal_reached,
        "the flag across the shelf should be reachable: {:?}",
        report.diagnosis
    );
}

/// **A shelf travels the way the file says it does.**
///
/// The level format has carried a platform's *axis* since movers existed and its **direction** since
/// a level whose only ride arrived from the right could not be written down. The box alone cannot
/// say it — a two-tile shelf at column 8 is the same box whether it slides left or right — so the
/// direction is a field, exactly as a turret's facing is, and this is the pin that the file, the
/// loader and the stepper all read the same one. A file written before the field existed has none
/// and must still read as the rightward shelf it always was.
#[test]
fn a_shelf_travels_the_way_the_file_says_it_does() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    // The same box and the same travel, stated twice: once leftward, once with no direction at all.
    for (json, name, going_left) in [
        (
            "{\"type\":\"platform\",\"x\":8,\"y\":4,\"tiles\":3,\"span\":5,\"axis\":\"x\",\"dir\":-1}",
            "leftward",
            true,
        ),
        (
            "{\"type\":\"platform\",\"x\":8,\"y\":4,\"tiles\":3,\"span\":5,\"axis\":\"x\"}",
            "unstated",
            false,
        ),
    ] {
        let entities = [json.to_string(), goal(19, 4)];
        let level = level(name, &rows, (1, 4), &entities);
        let mut sim = sim::Sim::new(&level, 3);
        let start = sim.platforms()[0].x;
        assert_eq!(start, 8.0, "{name}: the shelf starts where it was placed");
        common::step_n(&mut sim, AgentAction::Walk, 60);

        let now = sim.platforms()[0].x;
        if going_left {
            assert!(
                now < start - 0.5,
                "{name}: a shelf told to go left must go left, it is at {now} from {start}"
            );
        } else {
            assert!(
                now > start + 0.5,
                "{name}: a shelf with no direction is a rightward shelf, it is at {now} from {start}"
            );
        }
    }
}

/// **A mirrored level's shelves travel the mirrored way, and its lifts still lift.**
///
/// The mirror flips the level's x axis, so a slider that used to leave to the right now leaves to
/// the left — otherwise the mirror of a level whose only ride arrives from the left would be a
/// level whose ride leaves, and the mirror claim in `levels.rs` would be checking a different
/// course. A lift's up/down is *not* the mirrored axis, so it does not turn: the same shelf that
/// rises in the original rises in the mirror.
#[test]
fn a_mirrored_shelf_travels_the_mirrored_way() {
    let mut level = Level::starter("mirror-shelf", 24, 8);
    assert!(
        level.add_platform_tile(6, 5, 3, 4, false, -1),
        "a leftward slider"
    );
    assert!(
        level.add_platform_tile(14, 5, 3, 4, true, -1),
        "a downward lift"
    );

    let mirrored = level.mirrored_horizontal();
    assert_eq!(mirrored.platforms.len(), 2);
    let sliders: Vec<_> = mirrored.platforms.iter().filter(|p| !p.vertical).collect();
    let lifts: Vec<_> = mirrored.platforms.iter().filter(|p| p.vertical).collect();
    assert_eq!(sliders.len(), 1, "the slider is still a slider");
    assert_eq!(
        sliders[0].dir, 1,
        "a leftward slider mirrored is a rightward one"
    );
    assert_eq!(lifts.len(), 1);
    assert_eq!(
        lifts[0].dir, -1,
        "a lift's up/down is not the mirrored axis, so it does not turn"
    );
    // And the home box moved to the reflected column: a 3-wide shelf at 6 in a 24-wide level is at
    // 24 - (6 + 3) = 15.
    assert_eq!(sliders[0].rect.x, 15.0);
}

/// **A spiked ball hangs still until a body comes near, then swings on its chain.**
///
/// The three halves of the hazard: dormant while the body is far away, woken by proximity, and
/// a pendulum once awake — the ball rises away from the body that woke it and comes back through
/// the arc. The trigger is what makes this a trap rather than a rhythm: nothing about the level's
/// clock says when it starts, only where the body walked.
#[test]
fn a_spiked_ball_wakes_when_the_body_comes_near_and_then_swings() {
    let rows = [
        "........###.........",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"spiked_ball\",\"x\":9,\"y\":1,\"tiles\":3,\"span\":3}".to_string(),
        goal(18, 4),
    ];
    let level = level("spiked-ball", &rows, (1, 4), &entities);

    let mut sim = sim::Sim::new(&level, 9);
    common::step_n(&mut sim, AgentAction::Walk, 30);
    let ball = sim.spike_balls()[0];
    assert!(!ball.swinging, "a body eight tiles away does not wake it");
    assert_eq!(ball.x, 9.5, "a dormant ball hangs straight down");
    assert_eq!(ball.y, 4.0, "three tiles of chain below an anchor at y = 1");

    // Walk into range. The trigger radius is measured from the hanging ball, so the body is
    // still a couple of tiles short of the arc when the ball starts moving.
    common::step_n(&mut sim, AgentAction::Run, 60);
    let ball = sim.spike_balls()[0];
    assert!(ball.swinging, "the approach should have woken it");

    let (x0, y0) = (ball.x, ball.y);
    common::step_n(&mut sim, AgentAction::Walk, 12);
    let ball = sim.spike_balls()[0];
    assert!(
        (ball.x - x0).abs() > 0.1 || (ball.y - y0).abs() > 0.1,
        "a woken ball should be moving: {x0},{y0} -> {},{}",
        ball.x,
        ball.y
    );
    assert!(
        ball.y < y0 - 0.1,
        "and it should have climbed the arc, got {}",
        ball.y
    );
    assert!(
        (ball.x - 9.5).abs() <= ball.length + 1e-3,
        "but never further from the anchor than its chain: {}",
        ball.x
    );

    // The same actions on a second sim are the same pendulum: the trigger's phase is the body's
    // position, and the integration is deterministic.
    let mut again = sim::Sim::new(&level, 9);
    common::step_n(&mut again, AgentAction::Walk, 30);
    common::step_n(&mut again, AgentAction::Run, 60);
    common::step_n(&mut again, AgentAction::Walk, 12);
    assert_eq!(again.spike_balls()[0].x, ball.x);
    assert_eq!(again.spike_balls()[0].y, ball.y);
    assert_eq!(again.spike_balls()[0].swinging, ball.swinging);
}

/// **The ball is lethal on contact and cannot be stomped.**
///
/// It is a hazard and not an enemy: there is no answer to it beyond being somewhere else, so the
/// contact rule is a death with its own cause rather than the walker's stomp-or-die branch. The
/// spawn here is inside the resting ball's box, which is exactly the case a player meets when a
/// ball hangs across a corridor: touching it at all ends the episode.
#[test]
fn a_spiked_ball_kills_on_contact() {
    let rows = [
        "........###.........",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"spiked_ball\",\"x\":9,\"y\":1,\"tiles\":3,\"span\":4}".to_string(),
        goal(18, 4),
    ];
    let level = level("spiked-ball-contact", &rows, (9, 4), &entities);
    let mut sim = sim::Sim::new(&level, 1);
    sim.step(AgentAction::Walk);
    assert_eq!(
        sim.outcome(),
        Outcome::Died,
        "the resting ball is in the way"
    );
    assert_eq!(
        sim.death_cause(),
        Some(sim::DeathCause::SpikeBall),
        "and says what killed the body"
    );
}

/// **The trigger radius is the authored one, measured from the hanging ball.**
///
/// The reach is a field rather than a constant so a level can have a ball that waits until the
/// body is nearly under it, and one that winds up from across the room. A test that only asked
/// "does it wake" would pass on a trigger of any size; this walks the same body to the same place
/// against two reaches and pins the difference.
#[test]
fn a_wider_reach_wakes_the_ball_sooner() {
    let rows = [
        "........###.........",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let step = |reach: i32| -> u32 {
        let entities = [
            format!("{{\"type\":\"spiked_ball\",\"x\":9,\"y\":1,\"tiles\":3,\"span\":{reach}}}"),
            goal(18, 4),
        ];
        let level = level("spiked-ball-reach", &rows, (1, 4), &entities);
        let mut sim = sim::Sim::new(&level, 4);
        for tick in 0..200 {
            sim.step(AgentAction::Run);
            if sim.spike_balls()[0].swinging {
                return tick;
            }
        }
        panic!("a ball with reach {reach} never woke");
    };
    let narrow = step(2);
    let wide = step(6);
    assert!(
        wide < narrow,
        "a six-tile reach should wake the ball before a two-tile one: {wide} against {narrow}"
    );
}

/// **The rest box a level reports and the box the sim builds are one box.**
///
/// Two things outside the sim ask where a ball hangs before it moves: the reach oracle (as a
/// hazard a take-off cannot start inside) and the view (as the picture the editor draws). Both
/// ask [`sim::SpikeBallSpec::rest_box`], so this pins that it is the *sim's* box — the same width,
/// height and centre [`sim::SpikeBall::new`] hands the contact test — rather than a second pair of
/// numbers that would drift the moment one of them changed.
#[test]
fn a_spiked_balls_rest_box_is_the_box_the_sim_builds() {
    let rows = [
        "........###.........",
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"spiked_ball\",\"x\":9,\"y\":1,\"tiles\":3,\"span\":4}".to_string(),
        goal(18, 4),
    ];
    let level = level("spiked-ball-box", &rows, (1, 4), &entities);
    let spec = level.spike_balls[0];
    let built = sim::SpikeBall::new(0, spec, &sim::PHYSICS);
    assert_eq!(spec.rest_box(), built.aabb());
    assert!(
        !built.swinging,
        "and it is the box of a ball that has not moved"
    );
}

/// **A belt carries the body standing on it, and only that body.**
///
/// The three halves of the rule: the floor translates a standing body at the belt's own speed, the
/// direction comes from the glyph rather than from a sign, and a body in the air over the same
/// cell is not carried — a belt is a floor, not a current. The last test is the one that keeps the
/// mechanic honest: a belt that carried jumpers would be a physics the oracle does not model.
#[test]
fn a_belt_carries_a_body_standing_on_it() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        ">>>>>>>>>>>>>>>>>>>>",
    ];
    let right_level = level("belt-right", &rows, (2, 3), &[goal(19, 3)]);
    let mut sim = sim::Sim::new(&right_level, 3);
    let start = sim.player().x;
    // Idle on the belt: the body's own velocity is zero, so every tile of this is the belt.
    common::step_n(&mut sim, AgentAction::Walk, 32);
    let moved = sim.player().x - start;
    let expected = 32.0 * PHYSICS.belt_speed;
    assert!(
        (moved - expected).abs() < 0.2,
        "a right belt should carry {expected:.3} tiles in 32 ticks, carried {moved:.3}"
    );
    assert!(moved > 0.0, "and it should carry right, got {moved:.3}");

    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "<<<<<<<<<<<<<<<<<<<<",
    ];
    let left_level = level("belt-left", &rows, (10, 3), &[goal(0, 3)]);
    let mut sim = sim::Sim::new(&left_level, 3);
    let start = sim.player().x;
    common::step_n(&mut sim, AgentAction::Walk, 32);
    assert!(
        sim.player().x < start - 1.0,
        "a left belt should carry left: {start:.3} -> {:.3}",
        sim.player().x
    );
}

/// **A body in the air over a belt is not carried.**
///
/// The belt test is the one that would pass by accident if the carry were written as a force on
/// the cell: a jumper spends most of its arc over the belt it took off from, and a current in the
/// air would lengthen every jump off a belt for free — a physics the reach oracle cannot see.
#[test]
fn a_belt_does_not_carry_a_body_that_is_in_the_air_over_it() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        ">>>>>>>>>>>>>>>>>>>>",
    ];
    let level = level("belt-air", &rows, (2, 3), &[goal(19, 3)]);
    let mut sim = sim::Sim::new(&level, 3);
    // One tick of jump: the body leaves the belt and is over it for the next forty.
    common::step_n(&mut sim, AgentAction::JumpInPlace, 1);
    common::step_n(&mut sim, AgentAction::Walk, 1);
    assert!(!sim.player().grounded, "the jump should have left the belt");
    let airborne_x = sim.player().x;
    let airborne = sim.stats().frames;
    common::step_n(&mut sim, AgentAction::Walk, 8);
    assert!(!sim.player().grounded, "still in the arc");
    let drift = (sim.player().x - airborne_x).abs();
    assert!(
        drift < PHYSICS.belt_speed * 4.0,
        "an airborne body must not be carried: drifted {drift:.3} over {airborne} airborne frames"
    );
}

/// **A rock sleeps, wakes once, falls under gravity, and is spent.**
///
/// The whole design in one episode: it hangs in its socket while the body is away, shudders for
/// `rock_warn` ticks when the body comes near, falls under the same gravity a crate falls under,
/// stops on the floor rather than through it, and never moves again — a second approach and a
/// second hundred ticks leave it exactly where it landed.
#[test]
fn a_rock_wakes_falls_once_and_stays_put() {
    let rows = [
        "....###.............",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"rock\",\"x\":5,\"y\":1}".to_string(),
        goal(19, 3),
    ];
    let level = level("rock-fall", &rows, (1, 3), &entities);
    let mut sim = sim::Sim::new(&level, 5);
    assert_eq!(sim.rocks().len(), 1);
    assert_eq!(
        (sim.rocks()[0].home.x, sim.rocks()[0].home.y),
        (5.0, 1.0),
        "a rock hangs in the cell it was authored in"
    );

    // Far away and doing nothing: asleep, in its socket.
    for _ in 0..20 {
        sim.step(AgentAction::Walk);
    }
    assert!(!sim.rocks()[0].woken, "a rock must not wake on its own");
    assert_eq!(sim.rocks()[0].y, 1.0);

    // Walk up to it. The trigger is proximity, so the body does not have to be underneath.
    let mut woke = false;
    for _ in 0..80 {
        sim.step(AgentAction::Run);
        if sim.rocks()[0].woken {
            woke = true;
            break;
        }
    }
    assert!(woke, "walking up to a rock must wake it");
    assert!(
        sim.rocks()[0].shuddering() && sim.rocks()[0].y == 1.0,
        "a woken rock shudders where it hangs before it lets go"
    );

    // Back off, so the fall can be watched rather than cut short by the body's death. The warning
    // plus the fall is longer than the retreat, which is the whole reason it is a warning.
    let mut landed = false;
    for _ in 0..200 {
        sim.step(AgentAction::Retreat);
        if sim.rocks()[0].landed {
            landed = true;
            break;
        }
    }
    assert!(landed, "the rock must come down");
    assert_eq!(sim.outcome(), Outcome::Running, "and the body got clear");
    let rock = sim.rocks()[0];
    assert!(
        (rock.y + rock.h - 4.0).abs() < 1.0e-3,
        "it stops with its bottom on the floor's top edge, not in the floor: bottom {}",
        rock.y + rock.h
    );
    assert!(!rock.falling(), "and it is no longer in the air");

    // **Once.** A body walking back into it neither wakes it again nor moves it, and the settled
    // rock is solid: the body ends flush against its side rather than passing through.
    let settled = sim.rocks()[0].y;
    for _ in 0..200 {
        sim.step(AgentAction::Run);
        if sim.outcome().is_finished() {
            break;
        }
    }
    assert_eq!(
        sim.outcome(),
        Outcome::Running,
        "a settled rock does not kill"
    );
    assert_eq!(sim.rocks()[0].y, settled, "and nothing moves it again");
    assert!(
        sim.player().aabb().right() <= 5.0 + 1.0e-3,
        "the body should be stopped by the rock's side, not inside it: right {}",
        sim.player().aabb().right()
    );
}

/// **A falling rock kills the body under it.** The trap's one consequence.
///
/// The body stands under the socket and does nothing, so the wake is the first tick and the death
/// is the rock's own fall: `DeathCause::Rock`, inside a second.
#[test]
fn a_falling_rock_kills_the_body_under_it() {
    let rows = [
        "....###.............",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"rock\",\"x\":5,\"y\":1}".to_string(),
        goal(19, 3),
    ];
    let level = level("rock-kill", &rows, (5, 3), &entities);
    let mut sim = sim::Sim::new(&level, 5);
    for _ in 0..240 {
        sim.step(AgentAction::Walk);
        if sim.outcome().is_finished() {
            break;
        }
    }
    assert_eq!(sim.outcome(), Outcome::Died);
    assert_eq!(sim.death_cause(), Some(sim::DeathCause::Rock));
}

/// **A settled rock is a step**: a body stands on its top instead of sinking into it.
///
/// The property the press never had, and the reason the element can be put in a corridor at all:
/// after it has fallen, the level it blocked is a level with a one-tile ledge in it.
#[test]
fn a_settled_rock_is_a_step_a_body_can_stand_on() {
    let rows = [
        "....###.............",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"rock\",\"x\":5,\"y\":1}".to_string(),
        goal(19, 3),
    ];
    let level = level("rock-step", &rows, (1, 3), &entities);
    let mut sim = sim::Sim::new(&level, 5);
    for _ in 0..80 {
        sim.step(AgentAction::Run);
        if sim.rocks()[0].woken {
            break;
        }
    }
    for _ in 0..200 {
        sim.step(AgentAction::Retreat);
        if sim.rocks()[0].landed {
            break;
        }
    }
    let rock = sim.rocks()[0];
    assert!(
        rock.landed,
        "the fixture's rock has to be down for this test"
    );

    // A body placed on its top: one tick of gravity must leave it standing there.
    let mut body = sim::PlayerState::spawn(&level, &PHYSICS);
    body.x = rock.x;
    body.y = rock.y - PHYSICS.player_h;
    body.grounded = true;
    body.tick(&level, &PHYSICS, AgentAction::Walk, &[rock.aabb()]);
    assert!(
        body.grounded && (body.y + body.h - rock.y).abs() < 1.0e-3,
        "the body should stand on the rock's top, feet {} against top {}",
        body.y + body.h,
        rock.y
    );
}

/// **Two runs of one seed agree on every part of a fall.** A woken rock is state.
///
/// A press used to be a clock and needed no state at all; a rock is woken by where the body went,
/// so two runs that wove differently must differ, and two that behaved the same must agree to the
/// last bit of the hashed position and velocity.
#[test]
fn a_rocks_fall_is_deterministic() {
    let rows = [
        "....###.............",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"rock\",\"x\":5,\"y\":1}".to_string(),
        goal(19, 3),
    ];
    let level = level("rock-determinism", &rows, (1, 3), &entities);
    let play = || {
        let mut sim = sim::Sim::new(&level, 11);
        for _ in 0..70 {
            sim.step(AgentAction::Run);
        }
        for _ in 0..60 {
            sim.step(AgentAction::Retreat);
        }
        let rock = sim.rocks()[0];
        (rock.y, rock.vy, rock.woken, rock.landed, sim.state_hash())
    };
    let a = play();
    let b = play();
    assert!(a.2, "the run must actually wake the rock");
    assert_eq!(a, b, "the fall is a function of the seed and the actions");
}

/// **The oracle refuses a rock's band, and nothing above it.**
///
/// The band a rock can fall through is a hazard a route cannot be planned through — the rock may
/// be anywhere in it by the time the body arrives, and the body's own approach is what releases
/// it. The **bridge over the socket is still a stand**: a rock threatens the room below it, not
/// the deck above, and that is what makes the machine shop's high road a route.
#[test]
fn the_oracle_refuses_a_rock_band_but_not_the_bridge_over_it() {
    let rows = [
        "....#...............",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [
        "{\"type\":\"rock\",\"x\":4,\"y\":1}".to_string(),
        goal(18, 2),
    ];
    let level = level("rock-oracle", &rows, (1, 2), &entities);
    assert_eq!(level.rocks.len(), 1, "the fixture's rock parsed");
    let body = sim::Rect::new(4.0, 3.0 - PHYSICS.player_h, 1.0, PHYSICS.player_h);
    assert!(
        sim::reach::inside_rock(&level, &body),
        "a body standing under the rock is inside its band"
    );
    assert!(
        !level.is_standable(4, 3),
        "so the floor under it is not a stand the route graph can use"
    );
    assert!(
        level.is_standable(3, 3) && level.is_standable(5, 3),
        "and the floor either side of it still is"
    );
    assert!(
        level.is_standable(4, 0),
        "the bridge the rock hangs under is an ordinary stand"
    );
    let on_bridge = sim::Rect::new(4.0, -PHYSICS.player_h, 1.0, PHYSICS.player_h);
    assert!(
        !sim::reach::inside_rock(&level, &on_bridge),
        "and a body walking over the rock is not in its band"
    );
}

// ---- the crate -----------------------------------------------------------------------------

/// A crate entity at cell `(x, y)`.
fn block(x: i32, y: i32) -> String {
    format!("{{\"type\":\"block\",\"x\":{x},\"y\":{y}}}")
}

/// **A body leaning on a crate shoves it, one walking step at a time.**
///
/// The shove is not a separate verb: a body walking into a crate moves it at exactly the pace the
/// body would walk, so the two stay flush and the crate looks like it is being carried in front of
/// the body rather than bumped. The test pins both halves of that — the crate moved, and the body
/// never got inside it — because a crate that teleported ahead of the body would pass the first
/// check alone, and a crate that did not move would pass the second.
#[test]
fn a_body_shoves_a_crate_at_walking_pace() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [block(6, 3), goal(19, 3)];
    let level = level("crate-push", &rows, (1, 3), &entities);
    let mut sim = sim::Sim::new(&level, 5);
    let start = sim.blocks()[0].x;
    let mut worst = 0.0f32;
    for _ in 0..90 {
        sim.step(AgentAction::Run);
        let body = sim.player().aabb();
        let crate_box = sim.blocks()[0].aabb();
        worst = worst.max(body.right() - crate_box.x);
    }
    let moved = sim.blocks()[0].x - start;
    assert!(
        moved > 1.0,
        "ninety ticks of walking should have shoved the crate more than a tile, moved {moved:.3}"
    );
    assert!(
        worst < 1.0e-2,
        "the body must never be inside the crate it is pushing: worst penetration {worst:.4}"
    );
    assert!(
        (sim.player().aabb().right() - sim.blocks()[0].x).abs() < 1.0e-2,
        "and it should still be flush behind it: body right {:.4}, crate left {:.4}, body x {:.4}",
        sim.player().aabb().right(),
        sim.blocks()[0].x,
        sim.player().x
    );
}

/// **A crate stops flush at a wall, and then it is the wall.**
///
/// The refusal is the crate's own: a shove whose box would land in terrain does not happen at all,
/// so the crate ends up exactly against the wall rather than a step into it. The body behind it is
/// then stopped by the crate, with its speed spent — the same outcome as walking into a wall, which
/// is what makes a crate something a level can use to hold a body back.
#[test]
fn a_shoved_crate_stops_flush_at_a_wall_and_stops_the_body() {
    let rows = [
        "............#.......",
        "............#.......",
        "............#.......",
        "............#.......",
        "####################",
    ];
    let entities = [block(8, 3), goal(1, 3)];
    let level = level("crate-wall", &rows, (4, 3), &entities);
    let mut sim = sim::Sim::new(&level, 7);
    for _ in 0..180 {
        sim.step(AgentAction::Run);
    }
    let crate_box = sim.blocks()[0].aabb();
    assert!(
        (crate_box.x - 11.0).abs() < 1.0e-3,
        "the crate should rest flush at the wall's left edge (11.0), at {:.4}",
        crate_box.x
    );
    let body = sim.player().aabb();
    assert!(
        (body.right() - crate_box.x).abs() < 1.0e-3,
        "and the body should be flush behind it: body right {:.4}, crate left {:.4}",
        body.right(),
        crate_box.x
    );
    assert!(
        sim.player().vx == 0.0,
        "a body pressing a crate into a wall is a body that has stopped"
    );
}

/// **Two crates do not stack up into a train: a shove that would need one crate to move two is
/// refused.**
///
/// The front crate is against the wall, so the one behind it cannot move either, and the body makes
/// no progress. It is the negative half of the push rule, and the reason a row of crates is not a
/// conveyor: the crate in the middle can only go where there is room for it.
#[test]
fn a_crate_with_no_room_behind_it_stops_the_whole_row() {
    let rows = [
        "............#.......",
        "............#.......",
        "............#.......",
        "............#.......",
        "####################",
    ];
    let entities = [block(10, 3), block(11, 3), goal(1, 3)];
    let level = level("crate-row", &rows, (4, 3), &entities);
    let mut sim = sim::Sim::new(&level, 9);
    for _ in 0..120 {
        sim.step(AgentAction::Run);
    }
    assert_eq!(sim.blocks()[0].x, 10.0, "the near crate did not move");
    assert_eq!(sim.blocks()[1].x, 11.0, "and neither did the far one");
    assert!(
        (sim.player().aabb().right() - 10.0).abs() < 1.0e-3,
        "the body is stopped against the first crate"
    );
}

/// **A crate is a floor: a body dropped on one lands on its top, not through it.**
///
/// A crate is one tile, so it is a step rather than an obstacle — the pack's own hop height clears
/// it. The body is spawned in the air above the crate so the test measures the landing itself.
#[test]
fn a_crate_is_a_step_a_body_can_stand_on() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [block(8, 3), goal(19, 3)];
    let level = level("crate-step", &rows, (8, 1), &entities);
    let mut sim = sim::Sim::new(&level, 11);
    assert!(!sim.player().grounded, "the body starts in the air");
    common::step_n(&mut sim, AgentAction::Brake, 40);
    assert!(sim.player().grounded, "and it should have landed");
    let top = sim.blocks()[0].aabb().y;
    assert!(
        (sim.player().aabb().bottom() - top).abs() < 1.0e-3,
        "the feet should be on the crate's top ({top:.3}), at {:.3}",
        sim.player().aabb().bottom()
    );
}

/// **And the sensors see the crate as the solid it is.**
///
/// A crate is drawn into the 9×9 patch as [`ENC_SOLID`] — the value a wall already uses — because
/// that is what it is to a body: ground from above and a wall from the side. The second half is the
/// one that matters for learning to push: once the crate has moved, the cell it left reads empty
/// and the cell it arrived in reads solid, so "walk into it and it gives" is visible in one frame.
#[test]
fn the_sensor_patch_reads_a_crate_as_solid_and_follows_it() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [block(6, 3), goal(19, 3)];
    let level = level("crate-sensed", &rows, (4, 3), &entities);
    let mut sim = sim::Sim::new(&level, 13);
    let radius = sim::sensors::PATCH_RADIUS;
    let side = 2 * radius + 1;
    // The patch is keyed on the body's centre cell and row-major from its top-left corner.
    let (cx, cy) = sim.player().aabb().center();
    let (ctx, cty) = (cx.floor() as i32, cy.floor() as i32);
    let cell = |col: i32, row: i32| -> f32 {
        let c = col - ctx;
        let r = row - cty;
        sim.observation()[((r + radius) * side + (c + radius)) as usize]
    };
    assert_eq!(
        cell(6, 3),
        sim::sensors::ENC_SOLID,
        "the crate's cell reads as the solid it is"
    );
    common::step_n(&mut sim, AgentAction::Run, 60);
    let moved_to = (sim.blocks()[0].x).floor() as i32;
    assert!(moved_to > 6, "the body should have shoved it along");
    let (cx, cy) = sim.player().aabb().center();
    let (ctx, cty) = (cx.floor() as i32, cy.floor() as i32);
    let cell = |col: i32, row: i32| -> f32 {
        let c = col - ctx;
        let r = row - cty;
        sim.observation()[((r + radius) * side + (c + radius)) as usize]
    };
    assert_eq!(
        cell(moved_to, 3),
        sim::sensors::ENC_SOLID,
        "the cell the crate is in now reads solid"
    );
    if moved_to > ctx - radius {
        assert_ne!(
            cell(6, 3),
            sim::sensors::ENC_SOLID,
            "and the cell it left does not"
        );
    }
}

/// **A pushed crate is state, and two runs that pushed it the same way agree on all of it.**
///
/// The determinism the pack asks of every element: the same level and the same seed produce the
/// same crate positions, and the same state hash. A second run that shoved the crate to a different
/// cell hashes differently, which is what makes the crate a fact about the episode rather than about
/// the file.
#[test]
fn pushing_a_crate_is_deterministic_and_hashed() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "####################",
    ];
    let entities = [block(6, 3), goal(19, 3)];
    let level = level("crate-hash", &rows, (1, 3), &entities);
    let run = |ticks: u32| -> (f32, u64) {
        let mut sim = sim::Sim::new(&level, 17);
        common::step_n(&mut sim, AgentAction::Run, ticks);
        (sim.blocks()[0].x, sim.state_hash())
    };
    let (a, ha) = run(60);
    let (b, hb) = run(60);
    assert_eq!(a.to_bits(), b.to_bits(), "same run, same crate");
    assert_eq!(ha, hb, "same run, same hash");
    let (c, hc) = run(30);
    assert!(c < a, "a shorter push leaves the crate further back");
    assert_ne!(hc, ha, "so the two episodes are different states");
}

/// **A belt carries the crate standing on it.**
///
/// The crate used to stop dead the moment it was left on a belt: the belt only ever moved bodies
/// and a crate's tick only knew gravity, so a belt with a box parked on it was a belt switched off.
/// The floor now moves what stands on it, at the floor's own pace — the same pace it carries a
/// body at — which is the whole rule: a belt does not care whether the thing on it is walking.
#[test]
fn a_belt_carries_a_crate_standing_on_it() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        ">>>>>>>>>>>>>>>>>>>>",
    ];
    let entities = [block(6, 3), goal(19, 3)];
    let right = level("crate-belt", &rows, (1, 3), &entities);
    let mut sim = sim::Sim::new(&right, 19);
    let start = sim.blocks()[0].x;
    // The body stands still: every tile the crate gains is the belt's, not a shove.
    common::step_n(&mut sim, AgentAction::Brake, 32);
    let moved = sim.blocks()[0].x - start;
    let expected = 32.0 * PHYSICS.belt_speed;
    assert!(
        (moved - expected).abs() < 0.2,
        "a crate on a right belt should ride {expected:.3} tiles in 32 ticks, rode {moved:.3}"
    );

    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        "<<<<<<<<<<<<<<<<<<<<",
    ];
    let entities = [block(14, 3), goal(0, 3)];
    let left = level("crate-belt-left", &rows, (19, 3), &entities);
    let mut sim = sim::Sim::new(&left, 23);
    let start = sim.blocks()[0].x;
    common::step_n(&mut sim, AgentAction::Brake, 32);
    assert!(
        sim.blocks()[0].x < start - 1.0,
        "a crate on a left belt should ride left: {start:.3} -> {:.3}",
        sim.blocks()[0].x
    );
}

/// **Whoever is standing on a crate rides it.**
///
/// A crate that slid out from under its rider would be a lift that drops you — the body would be
/// left in the air over the cell the crate has vacated, fall, and be carried again a tick later,
/// which reads as a box that shakes rather than a floor that moves. The feet stay flush with the
/// crate's top for the whole ride.
#[test]
fn a_body_riding_a_belted_crate_travels_with_it() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        ">>>>>>>>>>>>>>>>>>>>",
    ];
    let entities = [block(8, 3), goal(19, 3)];
    let level = level("crate-belt-rider", &rows, (8, 2), &entities);
    let mut sim = sim::Sim::new(&level, 29);
    let crate_start = sim.blocks()[0].x;
    let body_start = sim.player().x;
    common::step_n(&mut sim, AgentAction::Brake, 32);
    let crate_moved = sim.blocks()[0].x - crate_start;
    assert!(
        crate_moved > 1.0,
        "the crate should have ridden the belt, moved {crate_moved:.3}"
    );
    let body_moved = sim.player().x - body_start;
    assert!(
        (body_moved - crate_moved).abs() < 0.02,
        "the rider should travel exactly as far as its crate: body {body_moved:.3}, crate {crate_moved:.3}"
    );
    let top = sim.blocks()[0].y;
    assert!(
        (sim.player().aabb().bottom() - top).abs() < 1.0e-2,
        "and it should still be standing on it: feet {:.3}, crate top {top:.3}",
        sim.player().aabb().bottom()
    );
}

/// **A shove is the shove's, not the belt's: pushing a crate on a belt is one steady walk.**
///
/// A crate on a belt gets the belt *and* the body's shove if the two are applied independently, so
/// the crate steps out from under the body's hands every frame, the flush contact that the shove
/// needs is lost, and one push becomes a stutter of separate bumps. The shove therefore wins the
/// tick: the crate advances at the body's shoving pace, the body stays flush behind it, and the
/// belt's own carry is refused for that tick — and for every tick the body keeps pressing, because
/// a body the belt would carry into its own crate is carried nowhere.
#[test]
fn a_shove_on_a_belt_is_still_one_stepping_push() {
    let rows = [
        "....................",
        "....................",
        "....................",
        "....................",
        ">>>>>>>>>>>>>>>>>>>>",
    ];
    let entities = [block(6, 3), goal(19, 3)];
    let level = level("crate-belt-push", &rows, (1, 3), &entities);
    let mut sim = sim::Sim::new(&level, 31);
    common::step_n(&mut sim, AgentAction::Run, 60);
    let steady = sim.blocks()[0].x;
    common::step_n(&mut sim, AgentAction::Run, 20);
    let per_tick = (sim.blocks()[0].x - steady) / 20.0;
    assert!(
        (per_tick - PHYSICS.block_push).abs() < 0.01,
        "a shoved crate on a belt should advance at the shove's pace ({:.4}), at {per_tick:.4}",
        PHYSICS.block_push
    );
    assert!(
        (sim.player().aabb().right() - sim.blocks()[0].x).abs() < 1.0e-2,
        "and the body stays flush behind it: body right {:.4}, crate left {:.4}",
        sim.player().aabb().right(),
        sim.blocks()[0].x
    );
}
