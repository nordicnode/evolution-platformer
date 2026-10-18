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
            "{\"type\":\"platform\",\"x\":8,\"y\":3,\"tiles\":2,\"span\":8,\"dir\":-1}"
                .to_string(),
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

    let json = level.to_json_pretty().expect("the document is writable");
    let back = Level::from_json_str(&json).expect("and readable");
    assert_eq!(back.content_hash(), level.content_hash(), "hash survives");
    assert_eq!(back.chasers.len(), 1);
    assert_eq!(back.flyers.len(), 1);
    assert_eq!(back.turrets.len(), 1);
    assert_eq!(back.turrets[0].dir, -1, "the firing direction is kept");
    assert_eq!(back.platforms.len(), 1);
    assert_eq!(back.platforms[0].rect.w, 3.0, "the width is kept");
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

    for (kind, level) in [
        ("chaser", &chaser),
        ("flyer", &flyer),
        ("turret", &turret),
        ("platform", &platform),
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
    assert!(level.add_platform_tile(6, 5, 3, 4, false, -1), "a leftward slider");
    assert!(level.add_platform_tile(14, 5, 3, 4, true, -1), "a downward lift");

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
