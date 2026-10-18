//! The elements: water, lava and vines.
//!
//! Three tiles, three rules, and one shared question — **can a body get out of this?** — which is
//! what separates a hazard that adds a route from a hazard that ends one. Every test here is
//! written against the end of that question rather than against a constant: the sink rate is
//! checked by measuring a descent, the breach by whether a swimmer reaches a lip, the vine by
//! whether a climber reaches a ledge, and a vent by whether its ball comes back down.
//!
//! The fixtures are ASCII art because that is the level format, so a test's terrain is legible in
//! the source — and because a pool that is one row too shallow is a bug the picture shows.

mod common;

use common::fixture;
use sim::{AgentAction, Outcome, Sim, Tile};

/// A pool with its surface a tile below the rim, walls either side, and a floor.
///
/// ```
/// ................
/// ................
/// ................
/// ##~~~~~~~~~~~~##
/// ##~~~~~~~~~~~~##
/// ##~~~~~~~~~~~~##
/// ################
/// ```
///
/// The spawn is in the air above the water, so a body that does nothing **falls in** — which is
/// the first half of the mechanic and the half a swimmer has to undo.
fn pool() -> sim::Level {
    fixture(
        "pool",
        &[
            "..............",
            "..............",
            "..............",
            "##~~~~~~~~~~##",
            "##~~~~~~~~~~##",
            "##~~~~~~~~~~##",
            "##############",
            "##############",
        ],
        (4, 2),
        &[("goal", 1, 5)],
    )
}

/// The same pool with the rim one tile above the water on the left, which is the lip a swimmer
/// has to be able to leave by.
fn pool_with_lip() -> sim::Level {
    fixture(
        "pool-lip",
        &[
            "................",
            "................",
            "#####...........",
            "####~~~~~~~~~~~.",
            "####~~~~~~~~~~~.",
            "####~~~~~~~~~~~.",
            "################",
            "################",
        ],
        (5, 4),
        &[("goal", 0, 5)],
    )
}

/// A shaft with a vine in it and a ledge at the top, one column to the right.
fn vine_shaft() -> sim::Level {
    fixture(
        "vine-shaft",
        &[
            "................",
            ".....V..........",
            ".....V######....",
            ".....V..........",
            ".....V..........",
            ".....V..........",
            "################",
            "################",
        ],
        (5, 5),
        &[("goal", 11, 1)],
    )
}

/// Step `n` ticks of `action`, and stop early if the episode ends.
fn step_n(sim: &mut Sim, action: AgentAction, n: u32) {
    for _ in 0..n {
        if sim.outcome().is_finished() {
            return;
        }
        sim.step(action);
    }
}

/// The body's feet, in world y.
fn feet(sim: &Sim) -> f32 {
    sim.player().y + sim.player().h
}

// ---------------------------------------------------------------------------
// water
// ---------------------------------------------------------------------------

/// **Water takes the fall out of an arrival and then pulls the body down slowly.**
///
/// Two claims in one measurement, because they are one behaviour: the entry cap (a body arriving
/// at terminal speed must not carry the fall through the pool) and the sink rate (a body that does
/// nothing descends at `water_sink`, not at `gravity`). A pool whose arrival was not capped would
/// put a body on the bottom in three ticks and make swimming a formality; a pool that did not sink
/// would be a floor the body could stand on in mid-air.
#[test]
fn water_absorbs_a_fall_and_then_sinks_the_body_slowly() {
    let mut sim = Sim::new(&pool(), 7);

    // Fall in. The body starts a tile above the surface and enters the water under gravity.
    step_n(&mut sim, AgentAction::Brake, 20);
    assert!(
        sim.player().in_water,
        "the body should be in the pool by frame twenty (y = {})",
        sim.player().y
    );

    // From here it is the water's body, not the fall's: the speed has been taken out of it and
    // the sink is the water's own rate.
    step_n(&mut sim, AgentAction::Brake, 10);
    assert!(
        (sim.player().vy - sim::PHYSICS.water_sink).abs() < 1e-4,
        "a body that does not swim should sink at water_sink, got {}",
        sim.player().vy
    );

    let before = sim.player().y;
    step_n(&mut sim, AgentAction::Brake, 20);
    let descended = sim.player().y - before;
    let expected = sim::PHYSICS.water_sink * 20.0;
    assert!(
        (descended - expected).abs() < 0.02,
        "twenty ticks of sinking should be {expected:.3} tiles, got {descended:.3}"
    );
}

/// **Holding jump swims up, and a fresh press at the surface is a push off it.**
///
/// The two halves of the button, measured against the surface line. The swim is what gets a body
/// out of the middle of a pool; the push is what gets it out of the pool at all, and it is the
/// reason `water_breach_height` is a number a level author can aim a lip at.
#[test]
fn jump_swims_and_a_press_at_the_surface_leaves_the_water() {
    let ph = sim::PHYSICS;
    let mut sim = Sim::new(&pool(), 7);
    step_n(&mut sim, AgentAction::Brake, 200);
    assert!(sim.player().in_water);
    assert!(
        sim.player().grounded,
        "the body should be on the pool's floor"
    );

    let deep = sim.player().y;
    step_n(&mut sim, AgentAction::JumpInPlace, 20);
    let rose = deep - sim.player().y;
    assert!(
        rose > ph.water_swim * 20.0 * 0.6,
        "swimming should lift the body, moved {rose:.3} in twenty ticks"
    );
    assert!(
        sim.player().in_water,
        "twenty ticks of swimming is not enough to leave a three-tile pool"
    );

    // Swim to the surface, let go for a couple of ticks (the press has to be *fresh*), then press
    // again: that press is the push.
    step_n(&mut sim, AgentAction::JumpInPlace, 60);
    step_n(&mut sim, AgentAction::Brake, 2);
    let surface = sim.level().water_top(4, 5).expect("the pool is water");
    let pushed_from = sim.player().y;
    sim.step(AgentAction::JumpInPlace);

    // Track the climb: the height above the surface is what a lip has to be shorter than.
    let mut peak = pushed_from;
    for _ in 0..20 {
        sim.step(AgentAction::JumpInPlace);
        peak = peak.min(sim.player().y);
    }
    let clearance = surface - (peak + sim.player().h);
    assert!(
        clearance > 0.75,
        "a push off the surface should clear roughly {:.2} tiles, got {clearance:.2}",
        ph.water_breach_height()
    );
}

/// **A swimmer can leave a pool over a lip one tile above the water.**
///
/// The end of the water mechanic rather than a constant: a pool a body cannot get out of is a
/// death trap with extra steps, and the level format lets an author draw one. This drives the
/// whole thing — swim up, press at the surface, drift over the rim, land — on a level whose only
/// route out is the push.
#[test]
fn a_swimmer_leaves_a_pool_over_its_lip() {
    let mut sim = Sim::new(&pool_with_lip(), 3);
    // Sink to the bottom of the pool, well away from the surface.
    step_n(&mut sim, AgentAction::Brake, 90);
    assert!(sim.player().in_water);

    // Swim up to the surface.
    let mut ticks = 0;
    while sim.player().in_water && ticks < 200 {
        sim.step(AgentAction::JumpInPlace);
        ticks += 1;
    }
    assert!(ticks < 200, "the body never reached the surface");
    // Release, then press: a fresh press is the push.
    step_n(&mut sim, AgentAction::Brake, 2);
    sim.step(AgentAction::JumpInPlace);
    // Drift left over the rim while the push carries the body up. Steering costs the jump
    // button, which is why the push is a launch rather than a jump: see `PlayerState::breach`.
    step_n(&mut sim, AgentAction::RetreatJump, 40);

    assert_eq!(
        sim.outcome(),
        Outcome::Running,
        "the swimmer died at ({:.2}, {:.2}) instead of leaving the pool",
        sim.player().x,
        sim.player().y
    );
    assert!(
        sim.player().grounded,
        "the swimmer did not land on the rim (at {:.2}, {:.2})",
        sim.player().x,
        sim.player().y
    );
    assert!(
        (feet(&sim) - 2.0).abs() < 1e-3,
        "the rim's surface is at y = 2.0, the body stands at {:.3}",
        feet(&sim)
    );
}

/// **Water drags horizontally: a sprint is not a way through a pool.**
#[test]
fn water_caps_horizontal_speed() {
    let mut sim = Sim::new(&pool(), 11);
    step_n(&mut sim, AgentAction::Brake, 40);
    assert!(sim.player().in_water);
    step_n(&mut sim, AgentAction::Run, 30);
    assert!(
        sim.player().vx.abs() <= sim::PHYSICS.water_speed + 1e-5,
        "a body in water should be capped at water_speed, got {}",
        sim.player().vx
    );
}

// ---------------------------------------------------------------------------
// vines
// ---------------------------------------------------------------------------

/// **A vine is climbed while jump is held and slid down when it is released.**
#[test]
fn a_vine_is_climbed_and_slid_down() {
    let ph = sim::PHYSICS;
    let mut sim = Sim::new(&vine_shaft(), 5);
    step_n(&mut sim, AgentAction::Brake, 4);
    assert!(sim.player().grounded);

    // Jump off the floor and onto the vine: it is grabbed on the first airborne tick.
    sim.step(AgentAction::JumpInPlace);
    sim.step(AgentAction::JumpInPlace);
    assert!(
        sim.player().on_vine,
        "the body should have grabbed the vine"
    );
    assert!(
        (sim.player().vy + ph.vine_climb).abs() < 1e-5,
        "holding jump on a vine should climb at vine_climb, got {}",
        sim.player().vy
    );

    let before = sim.player().y;
    step_n(&mut sim, AgentAction::JumpInPlace, 20);
    let climbed = before - sim.player().y;
    assert!(
        (climbed - ph.vine_climb * 20.0).abs() < 0.05,
        "twenty ticks of climbing should be {:.2} tiles, got {climbed:.2}",
        ph.vine_climb * 20.0
    );

    // Let go: a slide, not a fall. `vy` is the vine's own rate, far below `max_fall`.
    step_n(&mut sim, AgentAction::Brake, 1);
    assert!(sim.player().on_vine);
    assert!(
        (sim.player().vy - ph.vine_slide).abs() < 1e-5,
        "releasing a vine should slide at vine_slide, got {}",
        sim.player().vy
    );
    let high = sim.player().y;
    step_n(&mut sim, AgentAction::Brake, 10);
    let slid = sim.player().y - high;
    assert!(
        (slid - ph.vine_slide * 10.0).abs() < 0.05,
        "ten ticks of sliding should be {:.2} tiles, got {slid:.2}",
        ph.vine_slide * 10.0
    );
}

/// **A climber reaches the ledge at the top of the vine.**
///
/// The vine's own version of the pool's lip: a climb that cannot end anywhere is a decoration. The
/// climber leaves the vine at its top, keeps the upward velocity it had, and the ledge beside the
/// shaft is where it lands — which is what makes a vine a *route*.
#[test]
fn a_climber_reaches_the_ledge_above_the_vine() {
    let mut sim = Sim::new(&vine_shaft(), 5);
    step_n(&mut sim, AgentAction::Brake, 4);

    // Hold jump to climb. No horizontal input while climbing: the vine is one column wide, and a
    // body pressing right the whole way up would leave it halfway.
    sim.step(AgentAction::JumpInPlace);
    for _ in 0..4 {
        if sim.player().on_vine {
            break;
        }
        sim.step(AgentAction::JumpInPlace);
    }
    assert!(sim.player().on_vine, "the jump should end on the vine");
    for _ in 0..300 {
        if !sim.player().on_vine {
            break;
        }
        sim.step(AgentAction::JumpInPlace);
    }
    // Now step off it to the right. The body keeps the outward velocity it had and drifts over the
    // ledge, which is exactly how a person leaves a vine.
    step_n(&mut sim, AgentAction::Run, 40);

    assert_eq!(sim.outcome(), Outcome::Running);
    assert!(
        sim.player().grounded,
        "the climber did not reach a standing position (at {:.2}, {:.2})",
        sim.player().x,
        sim.player().y
    );
    assert!(
        (feet(&sim) - 2.0).abs() < 1e-3,
        "the ledge's surface is at y = 2.0, the body stands at {:.3}",
        feet(&sim)
    );
}

/// A vine hanging from the lip of a ledge, over the lava pit that starts one column to its
/// right — the shape a person draws first, and the one the window's agents kept falling into.
///
/// ```text
/// ................
/// ................   <- air above the ledge
/// ...######.......   <- the one-tile ledge the vine hangs from
/// ...V............
/// ...V............   <- the run, eight tiles down to the lip
/// ...V............
/// ####LLLLLL######   <- ground to the vine's column, then lava
/// ####LLLLLL######
/// ```
///
/// The spawn is on the left ground and the flag on the right, so the only way across the pit is
/// up the vine and over the ledge.
fn vine_over_lava() -> sim::Level {
    fixture(
        "vine-over-lava",
        &[
            "................",
            "................",
            "................",
            "................",
            "...######.......",
            "...V............",
            "...V............",
            "...V............",
            "...V............",
            "...V............",
            "...V............",
            "...V............",
            "...V............",
            "####LLLLLL######",
            "####LLLLLL######",
        ],
        (0, 12),
        &[("goal", 14, 12)],
    )
}

/// **A vine catches a body that arrives at a run, and holding right does not pull it off.**
///
/// The bug this pins is the one the window showed: an agent running right at a vine over a pit
/// grabbed it, climbed for a tile and a half, drifted out of the column because it was still
/// holding right, and fell into the lava. A vine was therefore a route the oracle could walk and
/// a population could not — the policy that runs right is the only one the level rewards
/// anywhere else, and at the vine it was fatal.
///
/// The measurement is the whole crossing, with the run button held the entire time: jump from the
/// lip, be caught, climb the eight-tile run, pull up onto the ledge, stand on it. No frame of the
/// climb is allowed to end the episode, and the body is never allowed to leave the vine's column
/// before the top — which is exactly what "the vine holds" means.
#[test]
fn a_vine_catches_a_running_body_and_holds_it_while_the_climb_is_pressed() {
    let level = vine_over_lava();
    let mut sim = Sim::new(&level, 11);
    let vine_col = 3;

    // Run at the lip, then jump and keep both the direction and the climb held.
    let mut grabbed = false;
    let mut highest = f32::INFINITY;
    let mut grab_x = 0.0_f32;
    for _ in 0..600 {
        if sim.outcome().is_finished() {
            break;
        }
        let action = if sim.player().x >= 2.6 {
            AgentAction::SprintJump
        } else {
            AgentAction::Run
        };
        sim.step(action);
        if sim.player().on_vine {
            if !grabbed {
                grab_x = sim.player().x;
            }
            grabbed = true;
            highest = highest.min(sim.player().y);
            let x = sim.player().x;
            // The hold is the claim: while the climb is pressed below the exit window, the
            // direction must not move the body along the column at all. (It may *straddle* the
            // column when it arrives — the grab does not centre it — but it may not slide.)
            assert!(
                (x - grab_x).abs() < 1e-3,
                "a pressing body slid from x {grab_x:.2} to {x:.2} on the vine (feet {:.2})",
                feet(&sim)
            );
        }
        if grabbed && sim.player().grounded {
            break;
        }
    }

    assert!(
        grabbed,
        "the run never caught the vine (at {:.2}, {:.2})",
        sim.player().x,
        sim.player().y
    );
    assert!(
        grab_x < vine_col as f32 + 1.0 && grab_x + sim.player().w > vine_col as f32,
        "the body was said to be on column {vine_col} at x {grab_x:.2}"
    );
    assert_eq!(
        sim.outcome(),
        Outcome::Running,
        "the crossing ended as {:?} at ({:.2}, {:.2})",
        sim.outcome(),
        sim.player().x,
        sim.player().y
    );
    // Eight tiles of climb from the lip at row 13 to the ledge's surface at row 4.
    assert!(
        highest <= 13.0 - 7.0,
        "the body climbed only {:.1} tiles before the vine let go",
        13.0 - highest
    );
    assert!(sim.player().grounded, "the pull-up never ended standing");
    assert!(
        (feet(&sim) - 4.0).abs() < 1e-3,
        "the body should be standing on the ledge (feet {:.2}, at x {:.2})",
        feet(&sim),
        sim.player().x
    );
}

// ---------------------------------------------------------------------------
// lava
// ---------------------------------------------------------------------------

/// A lava pool at the level's floor, wide enough that walking into it is unavoidable.
fn lava_pool() -> sim::Level {
    fixture(
        "lava-pool",
        &[
            "................",
            "................",
            "................",
            "................",
            "................",
            "#####LLLLLL#####",
            "################",
            "################",
        ],
        (2, 4),
        &[("goal", 13, 4)],
    )
}

/// **Lava is the cell, and touching it ends the episode.**
#[test]
fn lava_kills_on_contact() {
    let mut sim = Sim::new(&lava_pool(), 1);
    step_n(&mut sim, AgentAction::Run, 120);
    assert_eq!(
        sim.outcome(),
        Outcome::Died,
        "walking into a lava pool must end the episode (at {:.2}, {:.2})",
        sim.player().x,
        sim.player().y
    );
}

/// A vent: one lava cell with open air above it, and a rim beside it the body can stand on while
/// its box overhangs the mouth of the fountain.
///
/// ```
/// ................
/// ................
/// ................
/// ................
/// ................
/// ####L###########
/// ################
/// ```
fn lava_vent() -> sim::Level {
    fixture(
        "lava-vent",
        &[
            "................",
            "................",
            "................",
            "................",
            "................",
            "####L###########",
            "################",
            "################",
        ],
        (3, 4),
        &[("goal", 13, 4)],
    )
}

/// **A vent throws a ball that rises about a jump's height and comes back.**
///
/// The whole rhythm: no ball at first, one ball per period, an arc of `2·v0/g` ticks that peaks at
/// `lava_height`, and a ball that is gone by the time the next one is thrown. The arc is the same
/// geometry as the jump ladder (`v0²/(2g)`), which is why the numbers are stated as physics
/// methods rather than re-typed here.
#[test]
fn a_lava_vent_throws_a_ball_up_and_gravity_brings_it_back() {
    let ph = sim::PHYSICS;
    let level = lava_vent();
    assert_eq!(
        level.lava_vents(),
        vec![(4, 5)],
        "one open lava cell is one vent"
    );

    let mut sim = Sim::new(&level, 1);
    assert!(
        sim.lava_balls().is_empty(),
        "a level does not start mid-throw"
    );

    // Stand still on the rim (out of the fountain's way is not possible here — the body is
    // *over* the vent — so the test steps until the ball is thrown and stops there).
    let mut thrown_at = None;
    for i in 0..ph.lava_period as usize + 2 {
        sim.step(AgentAction::Brake);
        if !sim.lava_balls().is_empty() {
            thrown_at = Some(i);
            break;
        }
    }
    let thrown_at = thrown_at.expect("a vent should throw within one period");

    let mut peak = f32::INFINITY;
    let mut ticks = 0;
    while !sim.lava_balls().is_empty() && ticks < 200 {
        peak = peak.min(sim.lava_balls()[0].y);
        sim.step(AgentAction::Brake);
        ticks += 1;
    }
    let vent_top = 5.0;
    let rise = vent_top - peak;
    assert!(
        (rise - ph.lava_height()).abs() < 0.2,
        "a ball should rise {:.2} tiles, rose {rise:.2}",
        ph.lava_height()
    );
    assert!(
        (ticks as f32 - ph.lava_air_time_ticks()).abs() < 6.0,
        "a ball should be in the air about {:.0} ticks, was {}",
        ph.lava_air_time_ticks(),
        ticks
    );
    assert!(
        thrown_at < ph.lava_period as usize,
        "the first throw is inside the first period"
    );
}

/// **A lava ball kills, and it is the ball rather than the pool.**
///
/// The body stands on the rim with its box over the vent's mouth: it is never in a lava cell, so
/// the only thing that can end the episode is a ball. The episode is still running one tick before
/// the vent fires and over one tick after — which pins the death to the throw and not to the
/// standing.
#[test]
fn a_lava_ball_kills_a_body_over_the_vent() {
    let ph = sim::PHYSICS;
    let mut sim = Sim::new(&lava_vent(), 1);
    // Walk until the box overhangs the vent's mouth, then stand there. The body is supported by
    // the rim column it still covers, and it is over row four — air — not over lava.
    step_n(&mut sim, AgentAction::Run, 10);
    step_n(&mut sim, AgentAction::Brake, 6);
    assert!(
        sim.player().x > 3.7 && sim.player().x < 4.0,
        "the body should be over the vent's mouth, at x = {}",
        sim.player().x
    );
    assert_eq!(sim.outcome(), Outcome::Running, "the rim holds the body up");

    // Stand there until the last tick before the vent's first throw.
    while sim.stats().frames + 1 < ph.lava_period {
        sim.step(AgentAction::Brake);
    }
    assert_eq!(
        sim.outcome(),
        Outcome::Running,
        "nothing has been thrown yet"
    );

    step_n(&mut sim, AgentAction::Brake, 2);
    assert_eq!(
        sim.outcome(),
        Outcome::Died,
        "the ball should have hit the body standing over the vent (at {:.2}, {:.2})",
        sim.player().x,
        sim.player().y
    );
}

/// **Two runs of one seed see the same fountain.** A vent is a property of the level and its balls
/// are state, so both halves have to be deterministic: same throws, same frames, same hash.
#[test]
fn lava_balls_are_deterministic() {
    let level = lava_vent();
    let mut a = Sim::new(&level, 42);
    let mut b = Sim::new(&level, 42);
    for _ in 0..(sim::PHYSICS.lava_period + 20) {
        a.step(AgentAction::Brake);
        b.step(AgentAction::Brake);
        assert_eq!(a.state_hash(), b.state_hash(), "the fountains diverged");
        assert_eq!(a.lava_balls().len(), b.lava_balls().len());
    }
}

// ---------------------------------------------------------------------------
// the observation
// ---------------------------------------------------------------------------

/// **The patch can tell the three elements apart**, which is the whole of what makes them
/// learnable: a body that saw water as air would fall in, and one that saw lava as water would
/// swim into it.
///
/// Read at the patch's centre cell (index 40 of the 9×9 block), which is the tile the body is in.
#[test]
fn the_sensor_patch_reads_water_lava_and_vine_as_their_own_values() {
    let centre = 40;
    let read = |tile: char| {
        let rows = vec![
            "..........".to_string(),
            "..........".to_string(),
            format!(".....{}....", tile),
            format!(".....{}....", tile),
            "##########".to_string(),
        ];
        let level = fixture("sensor-cell", &rows, (5, 2), &[("goal", 0, 1)]);
        let sim = Sim::new(&level, 1);
        sim.observation()[centre]
    };

    let water = read('~');
    let lava = read('L');
    let vine = read('V');
    let air = read('.');

    assert!(
        water < air,
        "water must not read as air: {water} against {air}"
    );
    assert!(
        lava < water,
        "lava must read worse than water: {lava} against {water}"
    );
    assert!(
        vine > air,
        "a vine must read as better than air: {vine} against {air}"
    );
    assert_eq!(
        lava,
        sim::sensors::ENC_LAVA,
        "lava reads exactly as a spike does: the two are the same decision"
    );
    assert_eq!(water, sim::sensors::ENC_WATER);
    assert_eq!(vine, sim::sensors::ENC_VINE);
    assert_eq!(
        Tile::Water.glyph(),
        '~',
        "and the glyphs are the legend's own"
    );
}

// ---------------------------------------------------------------------------
// vines: the block a vine hangs from
// ---------------------------------------------------------------------------

/// A vine hanging from the middle of a **one-tile-thick** block, with the floor below it.
///
/// ```text
/// ..............
/// ..............   <- the top of the block, where the flag is
/// ....######....   <- the block the vine hangs from
/// ......V.......
/// ......V.......
/// ......V.......
/// ......V.......
/// ..............
/// ##############
/// ##############
/// ```
///
/// This is the case the vine had no answer for: a body climbs the vine, meets the block's
/// underside, and — before the pull-up — has nowhere to go, because the only way on is *over*
/// the thing it is holding. The block is one tile thick on purpose: a thicker one has no room
/// above its anchor, so a mantle there would put the body inside a wall.
fn vine_block() -> sim::Level {
    fixture(
        "vine-block",
        &[
            "..............",
            "..............",
            "....######....",
            "......V.......",
            "......V.......",
            "......V.......",
            "......V.......",
            "..............",
            "##############",
            "##############",
        ],
        (6, 7),
        // The flag sits on the block but away from the vine's column, so the pull-up is measured
        // standing rather than the episode ending mid-haul: a body passing *through* the flag on
        // its way over the lip would end the episode before it had finished the move.
        &[("goal", 10, 1)],
    )
}

/// **Holding the climb at the top of a vine hauls the body onto the block it hangs from.**
///
/// One press does both halves of the move: the jump that catches the vine is the climb, and the
/// climb held at the block's underside is the pull-up. The measurement is where the body ends up —
/// standing on the block's top surface, which is the tile the vine is attached to — rather than
/// that a flag was touched, because a body could in principle be carried there by a bug.
#[test]
fn holding_the_climb_hauls_the_body_onto_the_block_the_vine_hangs_from() {
    let level = vine_block();
    let mut sim = Sim::new(&level, 3);
    let mut mantled = false;
    for _ in 0..240 {
        if sim.outcome().is_finished() {
            break;
        }
        sim.step(AgentAction::JumpInPlace);
        mantled |= sim.player().mantling();
    }
    assert!(
        mantled,
        "the climb never became a pull-up (ended at x {:.2}, y {:.2}, ground {})",
        sim.player().x,
        sim.player().y,
        sim.player().grounded
    );
    // Feet on the block's top edge, which is row 2; the body occupies row 1.
    let feet = sim.player().y + sim.player().h;
    assert!(
        (feet - 2.0).abs() < 0.01 && sim.player().grounded,
        "the body should be standing on the block (feet at {feet:.2}, grounded {})",
        sim.player().grounded
    );
    // And the block is not just a place to land: it is a stand, and the flag on it can be walked
    // to from where the pull-up left the body.
    let mut reached = false;
    for _ in 0..180 {
        if sim.outcome().is_finished() {
            reached = sim.outcome() == Outcome::Won;
            break;
        }
        sim.step(AgentAction::Run);
    }
    assert!(
        reached,
        "the flag on the block should be walkable from the lip"
    );
}

/// **A vine under a block with no room above it is a dead end, not a route.**
///
/// The negative half of the pull-up. A two-tile-thick lid has no stand on top of its anchor — the
/// tile above is solid too — so there is nothing to haul onto, and a body that mantled anyway
/// would be inside the wall. It climbs, holds, and stays on the vine.
#[test]
fn a_vine_under_a_lid_with_no_room_above_it_is_a_dead_end() {
    let level = fixture(
        "vine-lid",
        &[
            "..............",
            "....######....",
            "....######....",
            "......V.......",
            "......V.......",
            "......V.......",
            "......V.......",
            "..............",
            "##############",
            "##############",
        ],
        (6, 7),
        &[("goal", 12, 7)],
    );
    let mut sim = Sim::new(&level, 3);
    for _ in 0..240 {
        sim.step(AgentAction::JumpInPlace);
        assert!(
            !sim.player().mantling(),
            "there is no stand above a two-tile lid, so there is nothing to pull up onto"
        );
    }
    assert!(
        sim.player().y > 2.5,
        "the body must stay below the lid, at y = {}",
        sim.player().y
    );
}

/// **The elements are solvable on a timetable, and this is the benchmark.**
///
/// `l11-the-elements` is the pack's stress test: a lava lake crossed on three moving decks, under
/// two dozen vents and a turret whose shot line covers the decks. The route oracle answers "a
/// crossing exists" by parking every mover at the ends of its travel, which is why the level read
/// as unbeatable in practice long after it read as reachable in principle. The planner steps the
/// real sim at the policy's own block cadence, so its answer is the executable one: this test
/// walks the returned action list back through a fresh sim and requires the flag.
///
/// Ignored by default because it is a benchmark, not a unit: a few seconds in release and much
/// slower in a debug build. Run it with
/// `cargo test -p sim --release --test elements -- --ignored timed_plan`.
#[test]
#[ignore = "the l11 benchmark: seconds in release, much slower in debug"]
fn a_timed_plan_crosses_the_elements() {
    let level = sim::Level::from_json_str(include_str!("../../../levels/l11-the-elements.json"))
        .expect("the shipped level parses");
    let route = sim::plan::solve(&level, 1, sim::plan::PlanConfig::default())
        .expect("the timed planner should find a schedule across the lake");
    assert!(
        route.frames > 600,
        "the route has to be longer than the anti-camper timer to be a timed crossing ({} frames)",
        route.frames
    );
    assert!(
        route.hops >= 10,
        "the plan should close most of the route's rungs, closed {}",
        route.hops
    );

    let mut sim = sim::Sim::new(&level, route.seed);
    for action in &route.actions {
        for _ in 0..sim::DECISION_TICKS {
            if sim.outcome().is_finished() {
                break;
            }
            sim.step(*action);
        }
    }
    assert_eq!(
        sim.outcome(),
        Outcome::Won,
        "the planner's own actions must win when replayed (ended at ({:.2}, {:.2}) after {} frames)",
        sim.player().x,
        sim.player().y,
        sim.stats().frames
    );
}

/// **The oracle gives the vine's block as a route.** The verdict is the claim that matters to a
/// level designer: before the pull-up this level read as unreachable, because the only way to the
/// flag is over the block the vine hangs from.
#[test]
fn the_oracle_reaches_the_top_of_the_block_a_vine_hangs_from() {
    let level = vine_block();
    let report = sim::reach::report(&level);
    assert!(
        report.goal_reached,
        "the oracle should route a vine pull-up: {report:?}"
    );
}
