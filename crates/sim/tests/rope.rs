//! The rope: a line a body catches in the air and swings on.
//!
//! The mechanics are checked as *properties of the motion* rather than against numbers copied out
//! of a run: a grab happens where the hands meet the line, the arc rises above the point the body
//! caught it, the speed on release is the arc's and not a walk's, and letting go is how a swing
//! ends. What each test names is the property, because the constants behind them are the ones a
//! level author is meant to retune.

use sim::physics::AgentAction;
use sim::reach::{goal_routes, report_with, ReachOptions, RouteMove};
use sim::{Level, Sim};

/// A hall with a ceiling at columns 5..=9, a rope hanging from it at column 7, and a flag at the
/// far end. The rope's first cell is row 4, tied to the ceiling's underside at y = 4.0, and it
/// hangs seven tiles, so its end is at y = 11.0 — exactly the top of a body standing on the floor
/// in row 12, which is where a jump reaches it.
fn swing_hall() -> Level {
    let rows = [
        "..............................",
        "..............................",
        "..............................",
        ".....#####....................",
        "..............................",
        "..............................",
        "..............................",
        "..............................",
        "..............................",
        "..............................",
        "..............................",
        "..............................",
        "##############################",
        "##############################",
    ];
    let tiles = rows
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(",");
    let src = format!(
        "{{\"format\":\"evo-platformer-level\",\"level_format_version\":4,\
         \"name\":\"swing-hall\",\"width\":30,\"height\":14,\
         \"spawn\":{{\"x\":7,\"y\":11}},\
         \"tiles\":[{tiles}],\
         \"entities\":[\
         {{\"type\":\"rope\",\"x\":7,\"y\":4,\"tiles\":7}},\
         {{\"type\":\"goal\",\"x\":26,\"y\":11}}]}}"
    );
    Level::from_json_str(&src).expect("the swing hall parses")
}

/// **A jump under the line is caught by it.** The grab is contact with the hands, so a body that
/// jumps where the rope hangs holds it on the way up rather than needing a second button.
#[test]
fn a_jump_under_the_rope_catches_it() {
    let level = swing_hall();
    let mut sim = Sim::new(&level, 7);
    assert!(
        !sim.player().on_rope(),
        "a body on the floor is not on a rope"
    );
    sim.step(AgentAction::JumpInPlace);
    sim.step(AgentAction::JumpInPlace);
    assert!(
        sim.player().on_rope(),
        "the jump should have caught the line at hand height {:.2}",
        sim.player().y
    );
    let grip = sim.player().rope.expect("a grip");
    let rope = level.ropes[0];
    assert_eq!(grip.anchor_x, rope.anchor().0);
    assert_eq!(grip.anchor_y, rope.anchor().1);
    assert_eq!(
        rope.anchor(),
        (7.5, 4.0),
        "the line is tied to the ceiling's underside, not a tile below it"
    );
    assert_eq!(rope.end_y(), 11.0, "the line ends at hand height here");
    assert!(
        grip.length <= grip.max_length,
        "a grip is never longer than the line: {} of {}",
        grip.length,
        grip.max_length
    );
}

/// **The swing is a pendulum, not a slide.** Pumped to the right, the body crosses the bottom of
/// the arc and rises on the far side — above the height it caught the line at — which is the
/// inertia the object exists for.
#[test]
fn a_pumped_swing_rises_on_the_far_side() {
    let level = swing_hall();
    let mut sim = Sim::new(&level, 7);
    sim.step(AgentAction::JumpInPlace);
    sim.step(AgentAction::JumpInPlace);
    assert!(sim.player().on_rope());
    let caught_at = sim.player().y;
    let caught_x = sim.player().x;

    let mut peak = caught_at;
    let mut far = caught_x;
    for _ in 0..90 {
        sim.step(AgentAction::RunJump);
        if !sim.player().on_rope() {
            break;
        }
        peak = peak.min(sim.player().y);
        far = far.max(sim.player().x);
    }
    assert!(
        far - caught_x > 1.0,
        "the arc should carry the body right: {caught_x:.2} -> {far:.2}"
    );
    assert!(
        peak < caught_at - 0.2,
        "the body should rise on the far side of the arc: caught at {caught_at:.2}, peaked at {peak:.2}"
    );
}

/// **Letting go keeps the swing.** The speed a body leaves with is the arc's, which is faster
/// than any gear it could have been running at — and it keeps travelling after the line is gone.
#[test]
fn letting_go_keeps_the_speed_the_arc_built() {
    let level = swing_hall();
    let mut sim = Sim::new(&level, 7);
    sim.step(AgentAction::JumpInPlace);
    sim.step(AgentAction::JumpInPlace);
    assert!(sim.player().on_rope());
    // Hold a direction and let the arc build: the pump is in phase half the time and out of it
    // half the time, which is a person leaning on a rope rather than a motor, so the swing finds
    // its own amplitude. Keep the fastest frame — that is the bottom of a pass, and the moment
    // releasing is worth anything; letting go at an extreme is how a body drops, not how it
    // crosses.
    let mut fastest = (0.0f32, sim.snapshot());
    for _ in 0..240 {
        sim.step(AgentAction::RunJump);
        if !sim.player().on_rope() {
            break;
        }
        let speed = (sim.player().vx * sim.player().vx + sim.player().vy * sim.player().vy).sqrt();
        if speed > fastest.0 {
            fastest = (speed, sim.snapshot());
        }
    }
    assert!(
        fastest.0 > sim::PHYSICS.run_speed,
        "a swinging body should be quicker than a run: {:.3} against {:.3}",
        fastest.0,
        sim::PHYSICS.run_speed
    );

    // Let go at that pass. The grip ends, and the release does not *spend* the speed: the tick
    // that lets go is also a tick of air, so the body coasts with the air's own friction — a few
    // thousandths — which is why this is a bound rather than an equality.
    let mut sim = fastest.1;
    let before = sim.player().vx;
    sim.step(AgentAction::Walk);
    assert!(!sim.player().on_rope(), "letting go ends the grip");
    assert!(
        before.signum() == sim.player().vx.signum()
            && (sim.player().vx - before).abs() <= sim::PHYSICS.brake_friction,
        "the release does not spend the velocity: {before:.4} -> {:.4}",
        sim.player().vx
    );

    // And it is a trajectory, not a stop: an arc's worth of speed carries the body over ground
    // no jump from a stand could reach.
    let released_at = sim.player().x;
    for _ in 0..30 {
        sim.step(AgentAction::Walk);
    }
    assert!(
        (sim.player().x - released_at).abs() > 0.5,
        "the body should carry on after letting go: {released_at:.2} -> {:.2}",
        sim.player().x
    );
}

/// **A rope is not ground.** Standing under one and walking changes nothing: it has no collision,
/// and a body that never jumps never catches it.
#[test]
fn a_rope_is_not_ground() {
    let level = swing_hall();
    let mut sim = Sim::new(&level, 7);
    for _ in 0..20 {
        sim.step(AgentAction::Walk);
    }
    assert!(!sim.player().on_rope());
    assert!(sim.player().grounded, "the floor is still the floor");
}

/// A rope survives a save and a load: the entity the writer emits is the entity the loader reads.
#[test]
fn a_rope_round_trips_through_the_file() {
    let level = swing_hall();
    let json = level.to_json_pretty().expect("the level writes");
    assert!(json.contains("\"type\": \"rope\""), "{json}");
    assert!(
        json.contains(&format!(
            "\"level_format_version\": {}",
            sim::level::LEVEL_FORMAT_VERSION
        )),
        "the writer emits the build's format version: {json}"
    );
    let back = Level::from_json_str(&json).expect("what it writes, it reads");
    assert_eq!(back.ropes, level.ropes);
    assert_eq!(back.ropes[0].length, 7.0);
}

/// **A rope is painted a cell at a time, and the cells below a line grow it.**
///
/// This is the brush's rule: a click is a cell, not a length. The first cell needs a ceiling; each
/// cell under an existing line extends that line rather than starting another, which is what makes
/// a drag down a wall one rope.
#[test]
fn a_rope_grows_a_cell_at_a_time() {
    let mut level = swing_hall();
    assert!(level.rope_cell_placeable(9, 4), "under the ceiling");
    assert!(level.add_rope_cell(9, 4));
    assert_eq!(level.ropes[1].length, 1.0, "one cell");
    assert!(level.add_rope_cell(9, 5));
    assert!(level.add_rope_cell(9, 6));
    assert_eq!((level.ropes[1].col, level.ropes[1].row), (9, 4));
    assert_eq!(level.ropes[1].length, 3.0, "one line, grown downward");
    assert_eq!(level.ropes.len(), 2, "not one line per cell");

    // A cell inside the line is already rope; one with no ceiling has nothing to hang from.
    assert!(!level.rope_cell_placeable(9, 5));
    assert!(!level.rope_cell_placeable(2, 4));
}

/// **A rope is found along the whole line, not only at the cell it is tied to.** The eyedropper
/// and the eraser act where the line is drawn — which for anything longer than a couple of tiles
/// is its middle — so a rope has to answer for every cell it hangs through, and for no cell
/// beside them.
#[test]
fn a_rope_is_found_along_the_whole_line() {
    let mut level = swing_hall();
    let (col, row, length) = (7, 4, 7);
    assert_eq!(level.rope_at(col, row).map(|r| r.length), Some(7.0));
    assert_eq!(
        level.enemy_at(col, row + length - 1),
        Some("rope"),
        "the last cell of the line"
    );
    assert_eq!(level.enemy_at(col, row + 2), Some("rope"), "its middle");
    assert!(level.rope_at(col, row + length).is_none(), "below the end");
    assert!(level.rope_at(col + 1, row + 2).is_none(), "beside the line");

    // And the eraser takes the line from its middle.
    assert!(level.remove_enemy_at(col, row + 2));
    assert!(level.ropes.is_empty());
}

/// The editor's placement rule: a rope needs a ceiling to hang from and air to hang through.
#[test]
fn a_rope_is_placed_only_under_something_solid() {
    let mut level = swing_hall();
    assert!(level.rope_placeable(7, 4), "under the ceiling");
    assert!(!level.rope_placeable(2, 4), "nothing above it");
    assert!(!level.rope_placeable(7, 12), "inside the floor");
    assert!(level.add_rope_tile(6, 4, 3));
    assert_eq!(level.ropes.len(), 2);
    assert!(level.remove_rope_at(6, 4));
    assert_eq!(level.ropes.len(), 1);
}

/// **The observation names the rope.** On the line the flag is up; on the floor under it the
/// offsets point at the line, which is the aim a grab is made of.
#[test]
fn the_observation_says_where_the_rope_is() {
    let level = swing_hall();
    let mut sim = Sim::new(&level, 7);
    let names = sim::sensors::input_names();
    let on_rope = names.iter().position(|n| n == "on_rope").expect("a slot");
    let dx = names.iter().position(|n| n == "rope_dx").expect("a slot");
    assert_eq!(on_rope, 108);
    assert_eq!(names.len(), sim::sensors::N_INPUTS);

    let standing = sim.observation();
    assert_eq!(standing[on_rope], 0.0);
    assert!(
        standing[dx].abs() < 0.1,
        "the body stands under the line: {}",
        standing[dx]
    );

    sim.step(AgentAction::JumpInPlace);
    sim.step(AgentAction::JumpInPlace);
    assert_eq!(sim.observation()[on_rope], 1.0);
}

/// Same level, same seed, same actions ⇒ the same state, swing included. The grip is state, so a
/// hash that left it out would call two different arcs the same frame.
#[test]
fn a_swing_is_deterministic() {
    let level = swing_hall();
    let mut a = Sim::new(&level, 11);
    let mut b = Sim::new(&level, 11);
    let mut gripped = false;
    for i in 0..120 {
        let action = match i % 3 {
            0 => AgentAction::RunJump,
            1 => AgentAction::Run,
            _ => AgentAction::JumpInPlace,
        };
        a.step(action);
        b.step(action);
        gripped |= a.player().on_rope();
        assert_eq!(a.state_hash(), b.state_hash(), "diverged at tick {i}");
    }
    // And the grip is *in* the hashed state rather than beside it: the frames compared above are
    // not all free-fall frames, so a hash that forgot the anchor or the length would have called
    // two different arcs the same frame and this loop would not have caught it.
    assert!(
        gripped,
        "no tick of the loop was on the rope, so the comparison never covered a grip"
    );
}

/// The scratch level shipped for the rope parses with its line in it.
#[test]
fn the_scratch_rope_level_has_its_rope() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("levels/scratch-rope.json");
    let level = Level::from_json_file(&path).expect("the scratch level loads");
    assert_eq!(level.ropes.len(), 1, "one rope hangs over the pit");
    let rope = level.ropes[0];
    assert_eq!((rope.col, rope.row), (9, 4));
    assert_eq!(rope.length, 7.0);
    assert_eq!(rope.anchor(), (9.5, 4.0), "tied to the ceiling's underside");
    assert_eq!(
        rope.end_y(),
        11.0,
        "its end hangs at a standing body's hands"
    );
}

/// **The object does what it is for: a body jumps at the line and swings across the pit.**
///
/// This is the shipped scratch level, driven by a policy a person could play: run at the lip,
/// jump, hold on with the button and lean into the arc, then let go over the far side. The
/// assertion is the crossing itself — the body comes to rest on ground it could not have reached
/// in one jump, and it got there along the line.
#[test]
fn a_body_swings_across_the_scratch_pit() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("levels/scratch-rope.json");
    let level = Level::from_json_file(&path).expect("the scratch level loads");
    let pit_end = 13.0;
    let mut sim = Sim::new(&level, 3);

    // Approach: run until the body is at the lip (its right edge over column 7).
    for _ in 0..80 {
        sim.step(AgentAction::Run);
        if sim.player().x + 0.6 >= 6.9 {
            break;
        }
    }
    // Jump at the rope, and hold the button for the grip and the pump.
    let mut caught = false;
    let mut released_at = None;
    let mut far = 0.0f32;
    let mut grip_seen = 0.0f32;
    for t in 0..220 {
        let action = if sim.player().on_rope() {
            caught = true;
            // Lean right through the arc, and let go at its **far end** — the top of the swing
            // on the far side, where the body's horizontal speed turns over. That is the moment
            // a person lets go of a rope, and the whole of the timing the object asks for.
            if sim.player().vx <= 0.0 && sim.player().x > 11.0 && released_at.is_none() {
                released_at = Some(t);
                AgentAction::Walk
            } else {
                AgentAction::RunJump
            }
        } else if caught {
            AgentAction::Run
        } else {
            AgentAction::RunJump
        };
        sim.step(action);
        far = far.max(sim.player().x);
        if let Some(grip) = sim.player().rope {
            grip_seen = grip.length;
        }
        if released_at.is_some() && sim.player().grounded {
            break;
        }
    }
    assert!(caught, "the jump never caught the line");
    assert!(
        grip_seen > 1.0,
        "the swing held a {grip_seen:.2}-tile line, which is a hang rather than an arc"
    );
    assert!(
        released_at.is_some(),
        "the swing never came back down past the pit"
    );
    assert!(
        far > pit_end && sim.player().x > pit_end,
        "the body did not cross the pit: it peaked at x {far:.2} and came to rest at {:.2}",
        sim.player().x
    );
    assert!(sim.player().grounded, "and it should have landed");
}

/// The scratch level, loaded once per test that needs it.
fn scratch_rope() -> Level {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("levels/scratch-rope.json");
    Level::from_json_file(&path).expect("the scratch level loads")
}

/// **The oracle used to call this level unbeatable.** The pit is wider than a sprint jump, so the
/// line is the only way across — and the verdict has to say both halves of that: beatable, and
/// beatable *because of the rope*. The second half is what makes this a test of the move rather
/// than of the level: with `rope` switched off the same terrain is exactly as sealed as it was
/// before the swing was modelled, which is the control the sweep needs to be believed.
#[test]
fn the_oracle_crosses_the_scratch_pit_only_with_the_rope() {
    let level = scratch_rope();
    let with = report_with(&level, ReachOptions::default());
    assert!(
        with.goal_reached,
        "the oracle says the rope level cannot be beaten: {:?}",
        with.diagnosis
    );
    let without = report_with(
        &level,
        ReachOptions {
            rope: false,
            ..ReachOptions::default()
        },
    );
    assert!(
        !without.goal_reached,
        "the pit is wider than any jump, so a verdict without the rope has to be a refusal"
    );
}

/// The graph's plan for the crossing is a **swing**, with the tick it lets go at: a route edge
/// that said "jump" at a take-off six tiles from the line would be a plan no body can press.
#[test]
fn the_route_graph_carries_the_crossing_as_a_swing() {
    let level = scratch_rope();
    let routes = goal_routes(&level);
    let swing = (0..routes.len())
        .flat_map(|stand| routes.crossings_from(stand))
        .find(|edge| matches!(edge.plan, RouteMove::Rope { .. }))
        .expect("the only crossing over the pit is the one the rope makes");
    let RouteMove::Rope { dir, release } = swing.plan else {
        unreachable!("found by the filter above");
    };
    assert_eq!(
        dir,
        sim::reach::TakeoffDir::Right,
        "the pit is to the right"
    );
    assert!(
        release > 0,
        "letting go on the grab tick is not a swing that crosses anything"
    );
    let traj = sim::reach::simulate_rope_swing(
        &level,
        &sim::reach::extract_platforms(&level),
        swing.takeoff_x,
        swing.surface_row,
        dir,
        release,
    );
    assert!(
        traj.landed_at.is_some_and(|(col, _)| col >= 13),
        "the plan's own swing has to land on the far bank, not in the pit: {:?}",
        traj.landed_at
    );
}
