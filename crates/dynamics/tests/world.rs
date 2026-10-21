//! What the engine promises, stated as behaviour rather than as a solver pass count.
//!
//! Every test here is a thing a player could see: a pendulum that keeps time, a line that is as
//! long as it is, a body that lands on a floor instead of through it, a crate that comes to rest,
//! and a world that computes the same numbers twice. The numbers are the sim's own — one tile,
//! one tick, gravity 0.05 tiles per tick squared.

use dynamics::{Grid, Open, Vec2, World};

const GRAVITY: Vec2 = Vec2 { x: 0.0, y: 0.05 };

/// A pinned pivot with a bob on a rigid rod.
fn pendulum(angle_radians: f32, length: f32) -> (World, usize) {
    let mut world = World::new(GRAVITY);
    world.iterations = 64;
    let pivot = world.pin(Vec2::ZERO);
    let bob = world.add_body(
        Vec2::new(length * angle_radians.sin(), length * angle_radians.cos()),
        1.0,
    );
    world.connect(pivot, bob, length);
    (world, bob)
}

#[test]
fn a_pendulum_keeps_its_period() {
    let length = 7.0;
    let (mut world, bob) = pendulum(10.0_f32.to_radians(), length);
    let expected = 2.0 * std::f32::consts::PI * (length / GRAVITY.y).sqrt();

    // Measured over several swings rather than one, because a single period is only a hundred
    // ticks and a rounding error in when the crossing is detected is a percent of that.
    let mut crossings: Vec<f32> = Vec::new();
    let mut last = world.body(bob).pos.x;
    for tick in 0..2000 {
        world.step(&Open);
        let x = world.body(bob).pos.x;
        if last > 0.0 && x <= 0.0 {
            crossings.push(tick as f32);
        }
        last = x;
    }
    assert!(
        crossings.len() >= 5,
        "the bob only crossed the vertical {} times in 2000 ticks",
        crossings.len()
    );
    // Only the crossings that go right-to-left are counted, and there is one of those per full
    // swing, so the spacing between four of them is four periods.
    let period = (crossings[4] - crossings[0]) / 4.0;
    let error = (period - expected).abs() / expected;
    assert!(
        error < 0.02,
        "a {length}-tile pendulum should swing in {expected:.2} ticks, took {period:.2}"
    );
}

#[test]
fn a_chain_is_exactly_as_long_as_its_links() {
    let mut world = World::new(GRAVITY);
    // Long lines converge slowly: a correction travels one link per pass, so a twenty-link chain
    // needs hundreds of passes to hang its own weight without stretching. This is the engine's
    // documented cost, and the number here is what it takes to be within half a percent.
    world.iterations = 200;
    let anchor = world.pin(Vec2::new(5.0, 0.0));
    let mut previous = anchor;
    for i in 1..=20 {
        let body = world.add_body(Vec2::new(5.0, i as f32 * 0.25), 1.0);
        world.connect(previous, body, 0.25);
        previous = body;
    }
    for _ in 0..400 {
        world.step(&Open);
    }
    let drawn: f32 = world
        .links()
        .iter()
        .map(|link| world.body(link.a).pos.distance(world.body(link.b).pos))
        .sum();
    assert!(
        (drawn - 5.0).abs() < 0.02,
        "a five-tile chain drew {drawn:.4} tiles"
    );
}

#[test]
fn a_released_line_lands_on_the_floor_and_stops() {
    // Ten tiles of line hung from a pin at row 1, starting out sideways six tiles above a floor
    // it is far too long to hang clear of. It has to fall, land, and coil.
    let mut grid = Grid::new(20, 16, false);
    grid.fill_row(7, 0, 19, true);
    let mut world = World::new(GRAVITY);
    world.iterations = 48;
    let anchor = world.pin(Vec2::new(2.0, 1.0));
    let mut previous = anchor;
    for i in 1..=40 {
        let body = world.add_body(Vec2::new(2.0 + i as f32 * 0.25, 1.0), 1.0);
        {
            let b = world.body_mut(body);
            b.damping = 0.96;
            b.friction = 0.4;
        }
        world.connect(previous, body, 0.25);
        previous = body;
    }
    for _ in 0..900 {
        world.step(&grid);
    }

    for (i, body) in world.bodies().iter().enumerate() {
        assert!(
            body.pos.y < 7.0,
            "body {i} ended at {:.3}, which is inside the floor",
            body.pos.y
        );
        assert!(body.pos.is_finite(), "body {i} left the number line");
    }
    assert!(
        world.max_speed() < 0.02,
        "the line is still moving at {:.3} tiles a tick after fifteen seconds",
        world.max_speed()
    );
    assert!(
        world.contacts().len() > 4,
        "the line is not lying on the floor at all"
    );
}

#[test]
fn a_fast_body_cannot_pass_through_a_wall() {
    // A one-tile wall at column 6, and a body aimed at it fast enough to be on its far side by the
    // end of a whole tick.
    let mut grid = Grid::new(20, 20, false);
    for ty in 0..20 {
        grid.set(6, ty, true);
    }
    let mut world = World::new(Vec2::ZERO);
    let body = world.add_body(Vec2::new(2.0, 5.5), 1.0);
    world.body_mut(body).vel = Vec2::new(3.0, 0.0);
    for _ in 0..20 {
        world.step(&grid);
    }
    assert!(
        world.body(body).pos.x < 6.0,
        "the body went through the wall and ended at x = {:.2}",
        world.body(body).pos.x
    );

    // And the same for a floor.
    let mut floor = Grid::new(20, 20, false);
    floor.fill_row(12, 0, 19, true);
    let mut world = World::new(Vec2::ZERO);
    let body = world.add_body(Vec2::new(5.0, 4.0), 1.0);
    world.body_mut(body).vel = Vec2::new(0.0, 4.0);
    for _ in 0..20 {
        world.step(&floor);
    }
    assert!(
        world.body(body).pos.y < 12.0,
        "the body went through the floor and ended at y = {:.2}",
        world.body(body).pos.y
    );
}

#[test]
fn a_dropped_crate_lands_flat_and_stays_square() {
    let mut grid = Grid::new(20, 30, false);
    grid.fill_row(20, 0, 19, true);
    let mut world = World::new(GRAVITY);
    let corners = world.add_box(Vec2::new(6.0, 5.0), Vec2::new(1.0, 1.0), 4.0);
    for corner in corners {
        let body = world.body_mut(corner);
        body.friction = 0.3;
        body.restitution = 0.0;
    }
    for _ in 0..500 {
        world.step(&grid);
    }

    // The corners are still a one-tile square: the edges and braces did their job, and the crate
    // did not shear into a parallelogram on landing.
    for i in 0..4 {
        let a = world.body(corners[i]).pos;
        let b = world.body(corners[(i + 1) % 4]).pos;
        assert!(
            (a.distance(b) - 1.0).abs() < 0.03,
            "edge {i} is {:.3} tiles after landing",
            a.distance(b)
        );
    }
    for corner in corners {
        let y = world.body(corner).pos.y;
        assert!(
            (18.9..20.0).contains(&y),
            "a corner came to rest at y = {y:.3}, not above the floor"
        );
    }
    assert!(
        world.is_resting(0.02),
        "the crate is still sliding at {:.3} tiles a tick",
        world.max_speed()
    );
}

#[test]
fn a_grip_holds_a_point_on_a_link() {
    let mut world = World::new(Vec2::ZERO);
    world.iterations = 64;
    let a = world.pin(Vec2::ZERO);
    let b = world.add_body(Vec2::new(1.0, 0.0), 1.0);
    let c = world.add_body(Vec2::new(2.0, 0.0), 1.0);
    world.connect(a, b, 1.0);
    world.connect(b, c, 1.0);
    // The middle of the second link, pulled to a place a two-link chain can actually reach: one
    // link turned down sixty degrees and the next laid straight on from it.
    let reachable = Vec2::new(1.0, -3.0_f32.sqrt() / 2.0);
    let grip = world.grip(b, c, 0.5, reachable);
    for _ in 0..120 {
        world.step(&Open);
    }

    let held = world.body(b).pos.lerp(world.body(c).pos, 0.5);
    assert!(
        held.distance(reachable) < 0.02,
        "the grip sat {:.3} tiles from its target",
        held.distance(reachable)
    );
    assert!((world.body(b).pos.distance(world.body(a).pos) - 1.0).abs() < 0.02);
    assert!((world.body(c).pos.distance(world.body(b).pos) - 1.0).abs() < 0.02);

    // Let go: nothing is holding the chain now, so the second link swings down under gravity.
    world.release_grip(grip);
    world.gravity = GRAVITY;
    for _ in 0..200 {
        world.step(&Open);
    }
    assert!(
        world.body(c).pos.y > world.body(a).pos.y + 0.5,
        "the released chain did not hang"
    );
}

#[test]
fn the_same_world_steps_to_the_same_bits_twice() {
    let build = || {
        let mut grid = Grid::new(24, 24, false);
        grid.fill_row(18, 0, 23, true);
        let mut world = World::new(GRAVITY);
        let anchor = world.pin(Vec2::new(3.0, 2.0));
        let mut previous = anchor;
        for i in 1..=24 {
            let body = world.add_body(Vec2::new(3.0 + i as f32 * 0.25, 2.0), 1.0);
            {
                let b = world.body_mut(body);
                b.damping = 0.97;
                b.friction = 0.35;
            }
            world.connect(previous, body, 0.25);
            previous = body;
        }
        world.add_box(Vec2::new(12.0, 4.0), Vec2::new(1.0, 1.0), 2.0);
        world.gravity = GRAVITY;
        (grid, world)
    };

    let (grid, mut first) = build();
    let (_, mut second) = build();
    for _ in 0..240 {
        first.step(&grid);
        second.step(&grid);
    }
    assert_eq!(first.bodies().len(), second.bodies().len());
    for (i, (a, b)) in first.bodies().iter().zip(second.bodies()).enumerate() {
        assert_eq!(
            (a.pos.x.to_bits(), a.pos.y.to_bits()),
            (b.pos.x.to_bits(), b.pos.y.to_bits()),
            "body {i} diverged: {:?} against {:?}",
            a.pos,
            b.pos
        );
        assert_eq!(
            (a.vel.x.to_bits(), a.vel.y.to_bits()),
            (b.vel.x.to_bits(), b.vel.y.to_bits()),
            "body {i} ended with a different velocity"
        );
    }
}

#[test]
fn a_falling_crate_that_lands_on_one_corner_tips_over() {
    // Rotation is not a stored quantity; it falls out of four corners and six links. A crate
    // dropped on a corner has to end up flat, which can only happen if it turned.
    let mut grid = Grid::new(24, 30, false);
    grid.fill_row(20, 0, 23, true);
    let mut world = World::new(GRAVITY);
    // A unit square tilted by twenty-five degrees. The tilt is what breaks the symmetry: dropped
    // truly flat, or as a diamond, it would balance on its lowest corner forever, and a test that
    // asks it to tip has to give it a reason to.
    let centre = Vec2::new(8.0, 6.0);
    let (s, c) = 25.0_f32.to_radians().sin_cos();
    let corners = [
        centre + Vec2::new(0.5 * (c - s), 0.5 * (s + c)),
        centre + Vec2::new(0.5 * (c + s), 0.5 * (s - c)),
        centre + Vec2::new(-0.5 * (c - s), -0.5 * (s + c)),
        centre + Vec2::new(-0.5 * (c + s), -0.5 * (s - c)),
    ];
    let ids = corners.map(|p| world.add_body(p, 1.0));
    for i in 0..4 {
        let next = (i + 1) % 4;
        world.connect(ids[i], ids[next], corners[i].distance(corners[next]));
        world.body_mut(ids[i]).friction = 0.4;
    }
    world.connect(ids[0], ids[2], corners[0].distance(corners[2]));
    world.connect(ids[1], ids[3], corners[1].distance(corners[3]));

    for _ in 0..800 {
        world.step(&grid);
    }

    let ys: Vec<f32> = ids.iter().map(|i| world.body(*i).pos.y).collect();
    let lowest = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let highest = ys.iter().cloned().fold(f32::INFINITY, f32::min);
    // Flat means one tile of height; still balanced on the corner would be the diagonal, 1.41.
    assert!(
        (lowest - highest - 1.0).abs() < 0.2,
        "the crate did not tip flat: corners at {ys:?}"
    );
    // And the two corners it is standing on are the two lowest: an edge, not a point.
    let mut sorted = ys.clone();
    sorted.sort_by(f32::total_cmp);
    assert!(
        (sorted[3] - sorted[2]).abs() < 0.1,
        "the crate came to rest on a corner: corners at {ys:?}"
    );
    assert!(
        world.is_resting(0.03),
        "the crate never settled, moving at {:.3}",
        world.max_speed()
    );
}
