//! Physics unit tests (§11): AABB collision corners, hazards, and enemy contact.
//!
//! Every fixture is a small ASCII-art level. Rows are top-down, `.` is air, `#` is
//! solid, `^` is a spike, `G` is a goal tile; `spawn` and entities are in tile
//! coordinates and an entity at `y` stands on the bottom edge of row `y`.

mod common;

use common::fixture;
use sim::physics::{PlayerState, CRUMBLE_TICKS, PHYSICS, SPIKE_TOP_CLEARANCE};
use sim::{AgentAction, Tile};

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

/// A three-tile wall at column 9, standing on the ground, with a standable top at row 1.
///
/// Three tiles is past the 2.44 a discrete full jump reaches from the ground surface, so
/// nothing but a wall jump gets up it — which is what makes it the fixture for the ability
/// rather than for a lucky jump.
const TALL_WALL: &[&str] = &[
    "............",
    ".........#..",
    ".........#..",
    ".........#..",
    "############",
    "############",
];
const TALL_WALL_TOP: f32 = 1.0;

/// Ground rows 4..5 at columns 0..=14, a five-tile pit, then ground again.
///
/// Five tiles is inside a sprint jump's 6.00 and outside a run jump's 4.00, with fourteen
/// tiles of runway behind the edge, so the same policy lives or dies on which gear it is in.
const SPRINT_GAP: &[&str] = &[
    ".........................",
    ".........................",
    ".........................",
    ".........................",
    "###############.....#####",
    "###############.....#####",
];
const SPRINT_GAP_EDGE: f32 = 15.0;

/// A one-tile step up at column 6: lower floor at row 5, upper floor at row 4. Walking up
/// onto the upper floor crosses nothing.
const STEP: &[&str] = &[
    "............",
    "............",
    "............",
    "............",
    "......######",
    "############",
];

/// The same shape with a two-tile drop at column 6 instead of a step: upper floor at row 3,
/// nothing below it until row 5, so the body would fall rather than step down.
const DROP: &[&str] = &[
    "............",
    "............",
    "............",
    "......######",
    "............",
    "############",
];

/// A single spike at column 8 with a real approach runway on either side.
///
/// The 12-wide `SPIKE` fixture above is too short to jump from: a sprint jump carries the
/// body 6 tiles, so on a 12-column level the landing spot is chosen by the right wall rather
/// than by the jump. Here the approach, the flight and the landing all fit.
const SPIKE_RUNWAY: &[&str] = &[
    "....................",
    "....................",
    "....................",
    "........^...........",
    "####################",
    "####################",
];
const SPIKE_RUNWAY_COLUMN: i32 = 8;

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
    assert!(
        player.grounded,
        "the body should be standing after settling"
    );
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

/// How far a hop carries the body: run up to a full sprint, hold the jump for `hold_blocks`
/// decision blocks with the direction still held, then use `tail` for the rest of the
/// flight and measure how far the body has moved when it touches down.
///
/// This is the *only* handle the agent has on a jump's length. It picks one action per
/// decision block and the action is held for the whole block, so the length of a hop is
/// whatever the sim does with "how many blocks of held jump, then what".
fn hop_carry(hold_blocks: u32, tail: AgentAction) -> f32 {
    let sky = ".".repeat(30);
    let ground = "#".repeat(30);
    let rows = vec![
        sky.clone(),
        sky.clone(),
        sky.clone(),
        sky.clone(),
        ground.clone(),
        ground,
    ];
    let level = fixture("hop-runway", &rows, (1, 3), &[("goal", 28, 3)]);
    let mut sim = sim::Sim::new(&level, 7);
    common::settle(&mut sim);
    common::step_n(&mut sim, AgentAction::Run, 30);
    common::step_n(&mut sim, AgentAction::Sprint, 30);
    let takeoff = sim.player().x;
    common::step_n(
        &mut sim,
        AgentAction::SprintJump,
        hold_blocks * sim::DECISION_TICKS,
    );
    let mut guard = 0;
    while !sim.player().grounded && !sim.outcome().is_finished() && guard < 200 {
        sim.step(tail);
        guard += 1;
    }
    assert!(guard < 200, "the body never came back down");
    sim.player().x - takeoff
}

/// **A short hop is expressible, and the air brake is what makes it so.**
///
/// This is the pin for finding 47, and the second half of the same report as the jump
/// ladder: an agent on a floor of one-tile spikes with one safe tile between them "only
/// ever jumps at the full length". The ladder test above fixed the *height* axis; this is
/// the *length* one, and it turned out to be a hole rather than a tuning problem.
///
/// A jump's carry is `speed × airtime`, and airtime follows from the height, so the
/// shortest coasted hop at sprint speed still carries 2.83 tiles — wider than a safe pad
/// plus the body that has to stand on it. Taking off from the left edge of a pad, the
/// body must clear the spike tile and then fit *inside* the next pad, which is `player_w`
/// narrower than a tile. Nothing in the action table could do that, because `brake` took
/// `air_friction` in the air and was therefore **bit-identical to `walk`**: the one action
/// whose entire purpose is "stop moving" did nothing at all when the body was moving
/// through the air, which is the only time a hop's length is decided. The measured
/// consequence is the whole of finding 47: every hop the agent owned overshot the pad, a
/// population could only cross such a level by overshooting and reversing, and that level
/// sat unsolved through 200 000 episodes while its two-safe-tile sibling was solved in 26
/// generations.
#[test]
fn braking_in_the_air_is_what_makes_a_short_hop_expressible() {
    let ph = &PHYSICS;
    // The window a hop has to land in, in tiles from take-off: far enough to clear the
    // hazard tile and land on the pad beyond it, and no further than still leaves the
    // body inside that pad.
    let lo = 2.0 * sim::TILE;
    let hi = lo + (sim::TILE - ph.player_w);
    let coast: Vec<f32> = (1..=10).map(|h| hop_carry(h, AgentAction::Walk)).collect();
    let brake: Vec<f32> = (1..=10).map(|h| hop_carry(h, AgentAction::Brake)).collect();

    // Coasting cannot hit the window at any length of hold: the shortest hop the action
    // space has already overshoots it. That *is* the report.
    assert!(
        coast.iter().all(|c| *c > hi),
        "every coasted hop should overshoot the {lo:.2}-{hi:.2} tile window, got {coast:?}"
    );
    // Braking in the air reaches it, and reaches it with a hold of a few blocks rather
    // than by luck at one magic value.
    assert!(
        brake.iter().any(|c| *c >= lo && *c <= hi),
        "no braked hop lands in the {lo:.2}-{hi:.2} tile window a one-tile pad allows: {brake:?}"
    );
    // Carry is a knob, not a switch: every extra block of hold carries further...
    for (i, pair) in brake.windows(2).enumerate() {
        assert!(
            pair[1] > pair[0],
            "hold {} carries {:.3} and hold {} carries {:.3}: a block of hold buys no \
             distance, so hop length is not aimable",
            i + 1,
            pair[0],
            i + 2,
            pair[1]
        );
    }
    // ...and the top of the ladder is still the full sprint jump, so nothing was given up.
    assert!(
        (brake[9] - ph.sprint_jump_distance()).abs() < 0.2,
        "a fully held sprint jump carries {:.3}, and should be the {:.3} the physics bible \
         promises",
        brake[9],
        ph.sprint_jump_distance()
    );
    // The two tails are finally different actions in the air. They were not.
    assert!(
        coast[0] > brake[0] + 1.0,
        "brake ({:.3}) and walk ({:.3}) should not be the same action mid-flight",
        brake[0],
        coast[0]
    );
}

/// The geometry half of the checkpoint rule, read straight off the level before the sim is
/// asked to agree with it: which landing zones are ones you had to cross something to get to.
#[test]
fn a_landing_zone_is_a_checkpoint_only_when_something_had_to_be_crossed() {
    // One spike at column 4 on an otherwise flat floor: the floor is two zones, the ledge
    // before the spike and the run after it, and only the second is a checkpoint.
    let spike = fixture("spike", SPIKE, (1, 3), &[("goal", 11, 3)]);
    for col in 0..SPIKE_COLUMN {
        assert_eq!(
            spike.checkpoint_zone(col, 4),
            None,
            "column {col} is the spawn ledge: nothing was crossed to stand on it"
        );
    }
    assert_eq!(
        spike.checkpoint_zone(SPIKE_COLUMN, 4),
        None,
        "the spike itself is not a place to stand"
    );
    for col in SPIKE_COLUMN + 1..12 {
        assert_eq!(
            spike.checkpoint_zone(col, 4),
            Some(SPIKE_COLUMN + 1),
            "column {col} is the far side of the spike, and the same zone as the rest of it: \
             a landing further along the run is the same arrival, not a worse one"
        );
    }

    // A level with no hazards has no checkpoints at all — which is why the flat levels'
    // landscape is untouched by the term.
    let flat = fixture("flat", FLAT, (1, 3), &[("goal", 11, 3)]);
    for col in 0..12 {
        assert_eq!(
            flat.checkpoint_zone(col, 4),
            None,
            "flat ground, column {col}"
        );
    }

    // And the far side of a pit is one, for the same reason a spike's far side is.
    let gap = fixture("sprint-gap", SPRINT_GAP, (1, 3), &[("goal", 24, 3)]);
    assert_eq!(gap.checkpoint_zone(19, 4), None, "the pit is not a landing");
    for col in 20..25 {
        assert_eq!(
            gap.checkpoint_zone(col, 4),
            Some(20),
            "column {col} is the far ledge of the pit, all one zone"
        );
    }

    // A one-tile step up at column 6, with the upper floor at row 4 and the lower one at
    // row 5. Walking up onto it is not a crossing, and the difference matters: the two-row
    // test in `Level::zone_is_checkpoint` is what keeps the pack's raised platforms out of
    // the count. Measured, the one-row rule made L2 pay for four landings where it only has
    // one pit, and L7-ascent for three where it has two ledges.
    let step = fixture("step", STEP, (1, 4), &[("goal", 11, 3)]);
    for col in 6..12 {
        assert_eq!(
            step.checkpoint_zone(col, 4),
            None,
            "column {col} is the top of a step: the body walked up onto it from ground one \
             row below, so nothing was crossed"
        );
    }
    // Only the exact step-down is excused, though. Two rows below, there is nowhere to land
    // and the body would have fallen — so the same geometry with a two-tile drop does count.
    let drop = fixture("drop", DROP, (1, 4), &[("goal", 11, 3)]);
    for col in 6..12 {
        assert_eq!(
            drop.checkpoint_zone(col, 3),
            Some(6),
            "column {col} is a ledge with a two-tile drop beside it, which is a crossing"
        );
    }

    // And the directional variant (`checkpoint_zone_from`) is symmetric across left-to-right
    // and right-to-left crossings: arriving from the right checks the zone's right edge.
    let mirror_spike = spike.mirrored_horizontal();
    let mirror_col = mirror_spike.width - 1 - SPIKE_COLUMN;
    for col in (mirror_col + 1)..mirror_spike.width {
        assert_eq!(
            mirror_spike.checkpoint_zone_from(col, 4, mirror_spike.width - 2),
            None,
            "the spawn ledge on the right side of the mirrored spike is not a checkpoint"
        );
    }
    for col in 0..mirror_col {
        assert_eq!(
            mirror_spike.checkpoint_zone_from(col, 4, mirror_col + 1),
            Some(0),
            "column {col} on the far (left) side of the mirrored spike is a checkpoint when approached from the right"
        );
    }
}

/// **A checkpoint is credited on arrival, not on the way past.**
///
/// This is the distinction the term exists for. The frontier is credited for anywhere the
/// body reaches — including the airspace over a pit it is about to fall into — so a failed
/// leap already keeps the ground it covered. The checkpoint is the half a failed leap cannot
/// keep, so it is credited only while the body is standing on the zone, which is what makes
/// the requirement stricter than the frontier rather than a relabelling of it.
#[test]
fn a_checkpoint_is_credited_on_arrival_and_not_for_flying_over_the_zone() {
    let level = fixture("spike-runway", SPIKE_RUNWAY, (1, 3), &[("goal", 19, 3)]);
    let mut sim = sim::Sim::new(&level, 11);
    common::settle(&mut sim);
    // Run up to speed, and take off with room to spare before the spike's right edge
    // reaches the body: a body that is still grounded with its edge over the hazard is
    // already dead, and a test that dies on the way to its own fixture proves nothing.
    while sim.player().x < 6.0 && !sim.outcome().is_finished() {
        sim.step(AgentAction::Sprint);
    }
    assert_eq!(
        sim.outcome(),
        sim::Outcome::Running,
        "the approach must survive"
    );
    sim.step(AgentAction::SprintJump);

    // Fly. Every airborne frame above the far zone is a frame *inside* the zone's columns
    // and still not on it.
    let mut airborne_frames_over_the_zone = 0;
    let mut guard = 0;
    while !sim.player().grounded && !sim.outcome().is_finished() && guard < 200 {
        guard += 1;
        let col = sim::Level::tile_x(sim.player().x + sim.player().w * 0.5);
        if col > SPIKE_RUNWAY_COLUMN {
            airborne_frames_over_the_zone += 1;
            assert_eq!(
                sim.stats().checkpoints,
                0,
                "the body is airborne over the far zone at column {col}: flying over a \
                 landing zone is not arriving on it"
            );
        }
        assert_eq!(
            sim.outcome(),
            sim::Outcome::Running,
            "the fixture jump should clear the spike, and died at x = {}",
            sim.player().x
        );
        sim.step(AgentAction::SprintJump);
    }

    assert!(
        airborne_frames_over_the_zone > 0,
        "the jump is supposed to carry the body over the far zone in the air, or this test \
         is not testing the thing it says it is"
    );
    assert_eq!(
        sim.stats().checkpoints,
        1,
        "landing on the far side credits its zone, once"
    );
    // Guard the mechanism, not just the outcome: the count came from standing, so it must
    // not have been there while airborne.
    assert!(
        sim.player().grounded,
        "the body should be on the ground here"
    );
}

/// A zone pays once. Walking the same stretch of ground again is not a second arrival.
#[test]
fn a_checkpoint_zone_pays_once_however_often_the_body_stands_on_it() {
    let level = fixture("sprint-gap", SPRINT_GAP, (1, 3), &[("goal", 24, 3)]);
    let mut sim = sim::Sim::new(&level, 107);
    common::settle(&mut sim);
    while sim.player().x < SPRINT_GAP_EDGE - 0.6 && !sim.outcome().is_finished() {
        sim.step(AgentAction::Sprint);
    }
    // Take off, then hold the jump for the whole flight — a full sprint jump is 40 ticks in
    // the air, so stopping short of that leaves the body descending into the pit with the
    // test asserting about a body that never landed.
    sim.step(AgentAction::SprintJump);
    let mut guard = 0;
    while !sim.player().grounded && !sim.outcome().is_finished() && guard < 200 {
        sim.step(AgentAction::SprintJump);
        guard += 1;
    }
    assert!(
        sim.player().grounded && sim.player().x > SPRINT_GAP_EDGE + 5.0,
        "the sprint jump should have landed on the far ledge, at x = {}",
        sim.player().x
    );
    assert_eq!(
        sim.stats().checkpoints,
        1,
        "the far ledge is one checkpoint"
    );

    // Walk right, then left again along the ledge it landed on. Same zone throughout.
    common::step_n(&mut sim, AgentAction::Run, 40);
    assert_eq!(
        sim.stats().checkpoints,
        1,
        "walking further along a zone is not a new one"
    );
    common::step_n(&mut sim, AgentAction::Retreat, 12);
    assert_eq!(
        sim.stats().checkpoints,
        1,
        "and walking back onto ground already stood on does not pay again"
    );
}

/// The pit case, which is the one the term was argued for: a failed leap and a successful
/// one are the same distance apart, and only one of them is an arrival.
#[test]
fn a_failed_pit_leap_keeps_its_frontier_and_earns_no_checkpoint() {
    let level = fixture("sprint-gap", SPRINT_GAP, (1, 3), &[("goal", 24, 3)]);

    fn attempt(
        level: &sim::Level,
        run: AgentAction,
        jump: AgentAction,
    ) -> (sim::Outcome, f32, u32) {
        let mut sim = sim::Sim::new(level, 107);
        common::settle(&mut sim);
        let mut jumped = false;
        let mut steps = 0;
        while !sim.outcome().is_finished() && steps < 900 {
            let action = if jumped || sim.player().x >= SPRINT_GAP_EDGE - 0.6 {
                jumped = true;
                jump
            } else {
                run
            };
            sim.step(action);
            steps += 1;
        }
        (sim.outcome(), sim.stats().best_x, sim.stats().checkpoints)
    }

    let (run_outcome, run_x, run_checkpoints) =
        attempt(&level, AgentAction::Run, AgentAction::RunJump);
    assert_eq!(run_outcome, sim::Outcome::Died);
    assert!(
        run_x > SPRINT_GAP_EDGE,
        "the failed leap still kept the ground it covered in the air: x = {run_x}"
    );
    assert_eq!(
        run_checkpoints, 0,
        "and it arrived nowhere, so the term pays nothing — the frontier it kept is the \
         difference between the two terms"
    );

    let (sprint_outcome, sprint_x, sprint_checkpoints) =
        attempt(&level, AgentAction::Sprint, AgentAction::SprintJump);
    assert_ne!(sprint_outcome, sim::Outcome::Died);
    assert!(sprint_x >= 20.0, "x = {sprint_x}");
    assert_eq!(
        sprint_checkpoints, 1,
        "the same leap, arrived: one checkpoint on the far ledge"
    );
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

/// **A spike kills where it is drawn, and not in the air above its tips.**
///
/// The sprite leaves the top three of its sixteen pixel rows transparent, deliberately: a
/// spike tile drawn all the way to its ceiling stops reading as a hazard and starts reading
/// as part of the wall above it (`tools/art/terrain.py::spikes`). The hazard check tested
/// the whole tile anyway, so every spike carried an invisible kill band 0.1875 tiles tall
/// above its topmost drawn pixel. On a hand-drawn level whose only route onto a platform
/// crossed such a spike, 70% of a swept 6,480 take-off/hold pairs died without the body
/// ever touching a drawn pixel, and the whole route reduced to one take-off position with
/// 0.025 tiles of clearance — the report behind `REVIEW.md` §2.12, and the reason the box
/// now follows the drawing.
///
/// Both halves are pinned here: the band above the tips is air, and below it the spike is
/// as lethal as it has always been — including for a body *standing* in its cell, which is
/// what makes a spike-topped column unstandable for the checkpoint and oracle rules.
#[test]
fn a_spike_kills_where_it_is_drawn_and_not_in_the_air_above_its_tips() {
    let ph = &PHYSICS;
    let level = fixture("spike", SPIKE, (2, 3), &[("goal", 11, 3)]);
    let drawn_top = 3.0 + SPIKE_TOP_CLEARANCE;
    assert!(
        (drawn_top - 3.1875).abs() < 1e-6,
        "the spike sprite's opaque pixels start three of sixteen cell rows below the top \
         (assets/tileset.png, cell 11,6: bbox rows 3..16), so the topmost drawn pixel is at \
         3.1875; the constant says {drawn_top}"
    );

    // A resting body placed by its bottom edge, as `PlayerState::spawn` would put it.
    fn placed(level: &sim::Level, ph: &sim::physics::Physics, x: f32, bottom: f32) -> PlayerState {
        let mut p = PlayerState::spawn(level, ph);
        p.x = x;
        p.y = bottom - ph.player_h;
        p.grounded = true;
        p
    }

    // Standing flush against the spike's left face, the body is in the next tile over and
    // nothing of it is in the hazard. This is the case the report was written about.
    let mut flush = placed(&level, ph, SPIKE_COLUMN as f32 - ph.player_w, 4.0);
    assert!(
        !flush.tick(&level, ph, AgentAction::Walk).touched_spike,
        "a body standing beside the spike, with its box ending at the cell's edge, does not \
         touch it"
    );

    // Inside the cell, in the blank band above the tips: clear. The body is placed one
    // tick's fall above the tips (gravity is 0.0125 tiles/tick and the hazard check runs
    // at the end of the tick), so it is still above every drawn pixel when it is tested.
    let mut in_the_band = placed(&level, ph, SPIKE_COLUMN as f32 + 0.2, drawn_top - 0.02);
    assert!(
        !in_the_band
            .tick(&level, ph, AgentAction::Walk)
            .touched_spike,
        "a body that never reaches down to the topmost drawn pixel has cleared the spike"
    );

    // A hundredth of a tile lower, the drawing is in the way.
    let mut into_the_drawing = placed(&level, ph, SPIKE_COLUMN as f32 + 0.2, drawn_top + 0.02);
    assert!(
        into_the_drawing
            .tick(&level, ph, AgentAction::Walk)
            .touched_spike,
        "a body whose box reaches into the drawn part of the spike is killed"
    );

    // And a body standing *in* the spike's cell — the tile the body walks through — is
    // still killed, which is what `Level::is_standable` and the checkpoint zones assume.
    let mut standing_in_it = placed(&level, ph, SPIKE_COLUMN as f32 + 0.2, 4.0);
    assert!(
        standing_in_it
            .tick(&level, ph, AgentAction::Walk)
            .touched_spike,
        "a body standing in a spike's own cell is inside the obstacle"
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

// ---------------------------------------------------------------------------
// Jumping is edge-triggered. These four tests are the reason it is.
// ---------------------------------------------------------------------------

/// Holding the jump button jumps **once**.
///
/// This is the fix the whole movement pass exists for. Level-triggered jumping meant a
/// body holding the button re-jumped on every landing: a perfectly periodic hop, one jump
/// per airtime, forever, costing nothing and deciding nothing. Measured on the shipped
/// levels, an evolved champion's jump rate was indistinguishable from that mechanical
/// cadence (2.58 jumps per 100 frames against the 2.50 of a held button).
#[test]
fn a_held_jump_jumps_once_and_a_fresh_press_jumps_again() {
    let level = fixture("flat", FLAT, (1, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 101);
    common::settle(&mut sim);
    assert_eq!(sim.stats().jumps, 0);

    // Five seconds of a held button. One jump, not twelve.
    common::step_n(&mut sim, AgentAction::JumpInPlace, 300);
    assert_eq!(
        sim.stats().jumps,
        1,
        "holding the jump button must jump exactly once"
    );
    assert!(
        sim.player().grounded && sim.player().vy == 0.0,
        "and the body must be standing there afterwards, not hovering"
    );

    // Release and press again: the second jump is available, so the *edge* is what is
    // gated rather than the jump button being spent for the episode.
    sim.step(AgentAction::Walk);
    sim.step(AgentAction::JumpInPlace);
    assert_eq!(sim.stats().jumps, 2, "a fresh press is a second jump");
    assert!(!sim.player().grounded, "and it left the ground");

    // A press *while airborne* is buffered, not a double jump: the body is one tick off
    // the ground and there is no second jump to give it. Nothing fires now, and the buffer
    // expires long before the landing it was actually aimed at.
    sim.step(AgentAction::Run);
    sim.step(AgentAction::RunJump);
    assert_eq!(
        sim.stats().jumps,
        2,
        "a fresh press in mid-air must not double jump"
    );

    // Landing first, then pressing: the same rhythm a hopping policy has to learn.
    common::step_n(&mut sim, AgentAction::Run, 60);
    assert!(sim.player().grounded, "the body should be back down");
    sim.step(AgentAction::Run);
    sim.step(AgentAction::RunJump);
    assert_eq!(
        sim.stats().jumps,
        3,
        "a release-then-press rhythm on the ground keeps jumping, which is the deliberate \
         version of what a held button used to do for free"
    );
}

/// A press just before touchdown still fires — the knob that keeps edge-triggering
/// learnable.
///
/// The network decides every four ticks, so without a buffer an edge-triggered jump turns
/// "jump now" into a timing puzzle whose only lesson is to press early. The buffer is what
/// lets a deliberate press be forgiving *without* letting a held button jump again, and
/// the two halves are asserted together here so neither can be tuned away unnoticed.
#[test]
fn a_press_just_before_touchdown_is_remembered_and_an_old_one_is_not() {
    let level = fixture("flat", FLAT, (1, 3), &[("goal", 11, 3)]);
    let surface = level.spawn.y + level.spawn.h;
    let mut sim = sim::Sim::new(&level, 103);
    common::settle(&mut sim);

    // One jump, then fall. The second press comes two ticks after take-off, which is as far
    // from the landing as a press can be: the whole rest of the jump lies between it and the
    // ground. The buffer only lasts `jump_buffer_ticks`, so it is long gone by then.
    //
    // Deliberately not a larger constant. This used to press at twenty ticks, which was far
    // from the landing only while the shortest jump was two decision blocks long; at one
    // block the flight is shorter than the number, the press landed inside the buffer and
    // *correctly* fired. Two ticks is derived from the shape of a jump rather than from a
    // trajectory, so it stays true when the ladder moves again.
    sim.step(AgentAction::JumpInPlace);
    common::step_n(&mut sim, AgentAction::Walk, 2);
    assert!(
        !sim.player().grounded,
        "two ticks after take-off the body should be off the ground"
    );
    let buffer = sim::PHYSICS.jump_buffer_ticks;
    sim.step(AgentAction::JumpInPlace);
    common::step_n(&mut sim, AgentAction::Walk, 60);
    assert_eq!(
        sim.stats().jumps,
        1,
        "a press made at take-off is far more than {buffer} ticks before the landing, so it \
         must be forgotten rather than fired late"
    );

    // Now the buffered case: press while the feet are within a tick or two of the ground.
    sim.step(AgentAction::JumpInPlace);
    let mut pressed = false;
    let mut steps = 0;
    while !sim.outcome().is_finished() && steps < 400 {
        let player = sim.player();
        let airborne = !player.grounded;
        let close = (player.y + player.h) >= surface - 0.5;
        // Descending *and* close: right after take-off the body is also close to the
        // ground, and a press there is aimed at nothing.
        let descending = player.vy > 0.0;
        let action = if airborne && descending && close && !pressed {
            pressed = true;
            AgentAction::JumpInPlace
        } else {
            AgentAction::Walk
        };
        sim.step(action);
        steps += 1;
        if pressed && sim.stats().jumps == 3 {
            break;
        }
    }
    assert!(
        pressed,
        "the fixture should offer a landing to press just before"
    );
    assert_eq!(
        sim.stats().jumps,
        3,
        "a press inside the {} tick buffer must fire on landing, but the body just landed",
        PHYSICS.jump_buffer_ticks
    );
    assert!(
        !sim.player().grounded,
        "and the buffered jump left the ground"
    );
}

/// Sprint is a gear with a price: it clears a gap a run cannot, and it needs the runway.
///
/// Run to the edge of a five-tile pit, jump, and the outcome depends only on which action
/// the body was in while it built up speed. This is the ability end to end — sensors,
/// speed cap, jump distance — rather than an assertion about a constant.
#[test]
fn a_sprint_jump_clears_a_gap_a_running_jump_falls_into() {
    let level = fixture("sprint-gap", SPRINT_GAP, (1, 3), &[("goal", 23, 3)]);

    // Run or sprint to the lip, then jump once and hold it.
    fn attempt(level: &sim::Level, run: AgentAction, jump: AgentAction) -> (sim::Outcome, f32) {
        let mut sim = sim::Sim::new(level, 107);
        common::settle(&mut sim);
        let mut jumped = false;
        let mut steps = 0;
        while !sim.outcome().is_finished() && steps < 900 {
            let action = if jumped || sim.player().x >= SPRINT_GAP_EDGE - 0.6 {
                jumped = true;
                jump
            } else {
                run
            };
            sim.step(action);
            steps += 1;
        }
        (sim.outcome(), sim.stats().best_x)
    }

    let (run_outcome, run_x) = attempt(&level, AgentAction::Run, AgentAction::RunJump);
    assert_eq!(
        run_outcome,
        sim::Outcome::Died,
        "a running jump over five tiles must fall in the pit (reached x = {run_x})"
    );
    assert!(
        run_x < 20.0,
        "and it must not have reached the far ledge: x = {run_x}"
    );

    let (sprint_outcome, sprint_x) = attempt(&level, AgentAction::Sprint, AgentAction::SprintJump);
    assert_ne!(
        sprint_outcome,
        sim::Outcome::Died,
        "a sprint jump over the same pit must clear it"
    );
    assert!(
        sprint_x >= 20.0,
        "and land on the far ledge, not in the pit: x = {sprint_x}"
    );
}

/// The wall jump: automatic off a cling, and impossible on a held button.
///
/// Super Meat Boy's rule — a wall is a thing you are against, so the verb does not change —
/// plus the edge trigger that makes a climb a rhythm rather than a mash. The last assertion
/// is the important one: holding the button next to a wall does *not* climb it.
#[test]
fn a_wall_jump_leaves_the_wall_and_a_held_button_cannot_climb() {
    let level = fixture("tall-wall", TALL_WALL, (1, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 109);
    common::settle(&mut sim);

    // Into the wall, then a ground jump, then hold the direction so the body comes back
    // down the face: at the apex it is pressed against the wall and falling, which is the
    // cling a wall jump is taken from.
    common::step_n(&mut sim, AgentAction::Run, 100);
    assert_eq!(
        sim.player().wall_dir(),
        1,
        "the body should be pressed against the wall"
    );
    sim.step(AgentAction::RunJump);
    common::step_n(&mut sim, AgentAction::Run, 26);
    assert!(
        sim.player().clinging(),
        "26 ticks after a jump the body should be sliding down the face"
    );
    let jumps_before = sim.stats().jumps;

    // A fresh press: up and away from the wall.
    sim.step(AgentAction::RunJump);
    assert_eq!(sim.stats().jumps, jumps_before + 1, "the wall jump fired");
    // The tick's own gravity lands after the launch, so the velocity the body leaves with
    // is `wall_jump_v0` one tick in: the assertion is the bible relation, not a constant.
    let expected = -(PHYSICS.wall_jump_v0 - PHYSICS.gravity);
    assert!(
        (sim.player().vy - expected).abs() < 1e-6,
        "a wall jump should launch at wall_jump_v0 = {} less one tick of gravity ({expected}), \
         got {}",
        PHYSICS.wall_jump_v0,
        sim.player().vy
    );
    assert!(
        sim.player().vx < 0.0,
        "and it pushes *away* from the wall it was on the right of: vx = {}",
        sim.player().vx
    );

    // Now hold the button down for five seconds against the same wall. The body comes
    // back, clings, slides — and never jumps again, because no fresh press arrived.
    let held_from = sim.stats().jumps;
    let mut highest = f32::INFINITY;
    common::step_n(&mut sim, AgentAction::Run, 2);
    for _ in 0..300 {
        sim.step(AgentAction::RunJump);
        highest = highest.min(sim.player().y);
    }
    assert_eq!(
        sim.stats().jumps,
        held_from,
        "a held jump button must not climb a wall"
    );
    assert!(
        sim.player().grounded,
        "with no fresh press the body slides down the face and lands"
    );
    assert!(
        highest > TALL_WALL_TOP - PHYSICS.player_h - PHYSICS.max_jump_height(),
        "and it never got more than one jump's worth up the wall: highest y = {highest}"
    );
}

/// The climb a policy that *does* press repeatedly gets: three tiles of wall, which no
/// single jump reaches.
///
/// The rhythm is a hold through the ascent and a two-tick release, because that is the
/// shortest gap that makes each press fresh. It is the same rhythm the reachability
/// oracle scripts (`sim::reach::simulate_wall_climb`), which is not a coincidence: a level
/// validated with it has to be climbable by a policy that can actually be expressed, and
/// this is that policy in the sim rather than in the oracle's own copy of the physics.
#[test]
fn pressing_repeatedly_off_a_wall_climbs_past_a_single_jump() {
    let level = fixture("tall-wall", TALL_WALL, (1, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 111);
    common::settle(&mut sim);
    common::step_n(&mut sim, AgentAction::Run, 100);

    let hold = (PHYSICS.air_time_ticks() * 0.5).ceil() as u32;
    let period = hold + 2;
    let mut highest = f32::INFINITY;
    let mut stood_on_top = false;
    for t in 0..max_climb_ticks() {
        let action = if t % period < hold {
            AgentAction::RunJump
        } else {
            AgentAction::Run
        };
        sim.step(action);
        highest = highest.min(sim.player().y);
        if sim.player().grounded && (sim.player().y + sim.player().h - TALL_WALL_TOP).abs() < 1e-3 {
            stood_on_top = true;
        }
    }

    assert!(
        sim.stats().jumps >= 3,
        "a three-tile climb is several wall jumps, got {}",
        sim.stats().jumps
    );
    assert!(
        stood_on_top,
        "the climb should end standing on the wall's top at y = {TALL_WALL_TOP}, \
         but the highest the body got was y = {highest}"
    );
    assert!(
        highest < TALL_WALL_TOP - PHYSICS.player_h + 0.01,
        "and it must have gone higher than any single jump could: highest y = {highest}"
    );
}

/// Ticks the wall-climb rhythm is allowed in the test above; the oracle's own budget, so the
/// two cannot disagree about how long a climb may take.
fn max_climb_ticks() -> u32 {
    sim::reach::MAX_CLIMB_TICKS
}

/// Walking into a pit is still fatal, and this is the measurement behind the claim that
/// the movement abilities did not make the pack's pits free.
///
/// Every pit in the shipped pack is a gap in the bottom two rows of the grid: nothing
/// under it but the end of the world. Its sides *are* walls a body can cling to, but the
/// wall is behind a body that walks off at running speed — turning around costs more
/// ground than the fall leaves. Simulated with the most generous recovery a policy can
/// make, pressing back into the wall on the first airborne tick: the body leaves the lip
/// at `run_speed`, falls out of the world at frame 107, and never touches the wall at all
/// (zero cling ticks). This is why the difficulty ladder's movement came from control
/// rather than from pit rescue, and why `l5-tightrope`'s four-tile pits still kill
/// populations (FINDINGS.md, the difficulty ladder).
#[test]
fn walking_into_a_pit_is_still_fatal() {
    let level = fixture("pit", PIT_SHAPE, (2, 12), &[("goal", 19, 12)]);
    let mut sim = sim::Sim::new(&level, 127);
    common::settle(&mut sim);

    let mut cling_ticks = 0;
    while !sim.outcome().is_finished() {
        if sim.player().clinging() {
            cling_ticks += 1;
        }
        // Run off the edge, then press back into the wall the body fell off for the whole
        // fall: the wall is right there, and the question is whether the body can reach
        // it before the world runs out.
        let action = if sim.player().grounded {
            AgentAction::Run
        } else {
            AgentAction::Retreat
        };
        sim.step(action);
    }

    assert_eq!(
        sim.outcome(),
        sim::Outcome::Died,
        "a body that walks off the lip at running speed should fall out of the world \
         (ended at x = {}, y = {})",
        sim.player().x,
        sim.player().y
    );
    assert_eq!(
        cling_ticks,
        0,
        "and it must not have touched the pit wall on the way: the turn costs more ground \
         than {} ticks of fall allow",
        sim.stats().frames
    );
}

/// Ticks for a body already at the run cap to be brought to a standstill by pressing against it,
/// on the ground or in the air. Airborne, the body leaves the ground on a `RunJump` so it carries
/// the run's momentum with it and the only thing under test is the turn.
fn ticks_to_reverse(level: &sim::Level, airborne: bool) -> u32 {
    let mut sim = sim::Sim::new(level, 3);
    common::settle(&mut sim);
    common::step_n(&mut sim, AgentAction::Run, 12);
    assert!(
        (sim.player().vx - PHYSICS.run_speed).abs() < 1e-5,
        "the fixture should be at the run cap, not {}",
        sim.player().vx
    );
    if airborne {
        sim.step(AgentAction::RunJump);
        assert!(
            !sim.player().grounded,
            "the body should have left the ground"
        );
    } else {
        assert!(sim.player().grounded, "this half is the one on the ground");
    }
    let mut ticks = 0;
    while sim.player().vx > 0.0 {
        sim.step(AgentAction::Retreat);
        ticks += 1;
        assert!(ticks <= 64, "the body never crossed zero");
    }
    ticks
}

/// A **grounded** turn is crisp, and the same press **in the air** is not: that difference *is*
/// `Physics::turn_accel_scale`, and both halves are claims the bible derives rather than numbers
/// this test invented.
///
/// The airborne half is not a leftover, it is the load-bearing one. A body that walks off a pit
/// lip at running speed must not be able to press back onto the pit wall before the world runs
/// out, because that is what makes every pit in the shipped pack lethal
/// ([`walking_into_a_pit_is_still_fatal`] measures it: zero cling ticks). A turn as crisp in the
/// air as on the ground makes that lip rescue reachable and quietly turns each pit into a wall
/// climb — which is a difficulty change, not a comfort setting — so the scale is grounded-only
/// and this test is what keeps it that way.
#[test]
fn a_turn_is_crisper_on_the_ground_than_in_the_air() {
    let level = fixture("turn", FLAT, (2, 3), &[("goal", 11, 3)]);
    let cap = PHYSICS.run_speed;
    let grounded = ticks_to_reverse(&level, false);
    let airborne = ticks_to_reverse(&level, true);

    // The grounded turn is the bible's own figure. `turn_ticks` is the continuous division and
    // the sim lands on its ceiling — a tick is the atom of the clock, so a turn that needs four
    // and a bit ticks takes five of them — which is why the expectation is rounded up here
    // rather than written down as a literal.
    let derived = (PHYSICS.turn_ticks(cap)).ceil() as u32;
    assert_eq!(
        grounded,
        derived,
        "a grounded turn from the cap is {derived} ticks by the bible (turn_ticks = {}), \
         but took {grounded}",
        PHYSICS.turn_ticks(cap)
    );

    // The airborne turn is the *unscaled* acceleration, which is what it was before the scale
    // existed: `run_accel × air_accel_scale`, with no turn term at all.
    let air_derived = (cap / (PHYSICS.run_accel * PHYSICS.air_accel_scale)).ceil() as u32;
    assert!(
        airborne >= air_derived - 1 && airborne <= air_derived + 1,
        "an airborne turn must keep the old acceleration: {air_derived} ticks by the bible, \
         took {airborne}"
    );
    assert!(
        airborne >= grounded * 3,
        "the air has to stay the slow place to change your mind, or the pits stop being fatal: \
         {grounded} grounded against {airborne} airborne ticks"
    );
}

/// The way back out of a pit is the air time off the lip, not the wall.
///
/// A body that jumps *at* the lip and turns around in the air lands back on the ledge it
/// jumped from — one jump, no cling. Forty ticks of ascent is enough to spend the forward
/// speed and come back over the ledge, where a body that merely walks off has already
/// lost. Both halves of this pair are the same fixture, so the only difference is the
/// decision at the edge.
#[test]
fn a_body_that_jumps_off_the_lip_can_turn_around_and_land_back() {
    let level = fixture("pit", PIT_SHAPE, (2, 12), &[("goal", 19, 12)]);
    let mut sim = sim::Sim::new(&level, 127);
    common::settle(&mut sim);

    let mut airborne = false;
    let mut landed_back = false;
    for _ in 0..400u32 {
        let action = if !airborne {
            if sim.player().x > 9.2 {
                AgentAction::RunJump
            } else {
                AgentAction::Run
            }
        } else {
            AgentAction::RetreatJump
        };
        sim.step(action);
        if !sim.player().grounded {
            airborne = true;
        }
        if airborne && sim.player().grounded {
            landed_back = true;
            break;
        }
        if sim.outcome().is_finished() {
            break;
        }
    }

    assert!(
        landed_back,
        "the body should turn in the air and land on the ledge it jumped from \
         (outcome {:?}, x = {}, y = {})",
        sim.outcome(),
        sim.player().x,
        sim.player().y
    );
    assert_ne!(
        sim.outcome(),
        sim::Outcome::Died,
        "turning around is not a death"
    );
    assert_eq!(
        sim.stats().jumps,
        1,
        "one jump, held through the ascent: the turn is the whole episode"
    );
}

/// Floor rows 13..14 with a three-tile pit at columns 10..12, and no floor under it: the
/// shape every pit in the shipped pack has.
const PIT_SHAPE: &[&str] = &[
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "##########...#######",
    "##########...#######",
];

// --- the trap pass: springs and brittle ground ------------------------------
//
// Two tiles, one behaviour each, and both of them are *interactions*: a pad only exists for a body
// that falls onto it and a brittle tile only exists for a body that stands on it. So the fixtures
// below are episodes rather than collision setups, and every assertion is about what happened to a
// body over time.

/// Floor rows 4..5, twelve columns, and a pad in the cell a jumping body passes through.
///
/// The pad is at row 2, which is deliberately *inside* the arc of a full jump from this floor: a
/// body rising from bottom 4.0 to its 2.5-tile apex has its own box straddle that cell, so a
/// fixture like this is the one that can tell "any overlap launches" apart from "a falling body
/// launches" — which is the whole of the pad's rule.
const PAD_IN_THE_ARC: &[&str] = &[
    "............",
    "............",
    ".v..........",
    "............",
    "############",
    "############",
];

#[test]
fn a_spring_launches_a_falling_body_upward_and_keeps_its_momentum() {
    // A running jump onto a pad, with the pad sitting in the floor's own surface row so the body
    // lands on it the way a player would: at speed, from above, on the way down.
    const RUNWAY: &[&str] = &[
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "............vv......",
        "####################",
    ];
    let level = fixture("pad-runway", RUNWAY, (1, 6), &[("goal", 18, 6)]);
    let mut sim = sim::Sim::new(&level, 4);
    common::settle(&mut sim);

    // Run up, jump, and keep running: the body has to arrive at the pad with horizontal speed it
    // would notice losing, which is what makes the momentum half of this test mean anything.
    let mut prev_vx = sim.player().vx;
    let mut prev_vy = sim.player().vy;
    let mut launch = None;
    for frame in 0..600u32 {
        let action = if sim.player().x < 8.5 {
            AgentAction::Run
        } else {
            AgentAction::RunJump
        };
        prev_vx = sim.player().vx;
        prev_vy = sim.player().vy;
        sim.step(action);
        if sim.player().vy == -PHYSICS.spring_v0 {
            launch = Some(frame);
            break;
        }
        assert!(
            !sim.outcome().is_finished(),
            "the body never reached the pad (outcome {:?} at x = {}, frame {frame})",
            sim.outcome(),
            sim.player().x
        );
    }
    let frame = launch.expect("the body must land on the pad and be thrown up");
    let vx_at_launch = sim.player().vx;

    // It was falling, and it was running: the pad's rule is about a body *arriving* on it.
    assert!(
        prev_vy > 0.0,
        "the pad launched a body that was not falling (vy was {prev_vy})"
    );
    assert!(
        vx_at_launch > PHYSICS.run_speed * 0.9,
        "the body should have arrived at the pad at running speed, not {vx_at_launch}"
    );
    // **Momentum is kept**, and asserted as "the pad wrote nothing horizontal": the only change in
    // `vx` across the launch frame is the acceleration the body's own action asked for. A pad that
    // zeroed it would show the whole run speed here, which is ten times this tolerance.
    assert!(
        (vx_at_launch - prev_vx).abs() <= PHYSICS.run_accel + 1e-6,
        "the launch frame changed vx by {}, more than the body's own steering can explain: the pad \
         is not momentum-preserving",
        vx_at_launch - prev_vx
    );
    assert!(
        sim.player().vy == -PHYSICS.spring_v0,
        "the launch velocity is the bible's own constant, exactly"
    );
    // A bounce is not a decision: the agent spent one jump on the way to the pad, and the pad's
    // throw is not charged as another (see `evo::fitness::JUMP_PENALTY` for what that pays for).
    assert_eq!(
        sim.stats().jumps,
        1,
        "the pad's throw must not be counted as a jump the agent made"
    );

    // And it out-jumps a jump, which is the reason a level author reaches for one: the rise is the
    // bible's 4.90 tiles, measured as the highest the body's feet get above the surface it launched
    // from, against the 2.4 a held jump reaches.
    let surface = sim.player().y + sim.player().h;
    let mut apex = 0.0f32;
    for _ in 0..200u32 {
        sim.step(AgentAction::Run);
        let rise = surface - (sim.player().y + sim.player().h);
        apex = apex.max(rise);
        if sim.player().grounded || sim.outcome().is_finished() {
            break;
        }
        assert_ne!(sim.outcome(), sim::Outcome::Died, "a pad is not a hazard");
    }
    assert!(
        apex > PHYSICS.discrete_max_jump_height() + 1.0,
        "a pad has to buy clearly more height than a jump ({apex:.2} against {:.2})",
        PHYSICS.discrete_max_jump_height()
    );
    assert!(
        (apex - PHYSICS.spring_height()).abs() < 0.3,
        "the rise should be the derived {} tiles, measured {apex:.2}",
        PHYSICS.spring_height()
    );

    // And the body that was thrown is still going the way it was going: it lands *past* where the
    // pad threw it from, which is the carry a level is built around. The flight carried it over the
    // flag at the far end of the runway, so the episode is allowed to be over — what it may not be
    // is a death, and what it may not be is a landing back on the pad it left.
    assert!(
        sim.player().x > 13.0,
        "a running spring launch carries the body well past the pad, landed at x = {}",
        sim.player().x
    );
    assert_ne!(sim.outcome(), sim::Outcome::Died, "a pad is not a hazard");
    let _ = frame;
}

#[test]
fn a_spring_does_not_launch_a_body_that_is_not_falling() {
    let level = fixture("pad-arc", PAD_IN_THE_ARC, (1, 3), &[("goal", 11, 3)]);
    let mut sim = sim::Sim::new(&level, 9);
    common::settle(&mut sim);
    assert!(
        sim.player().grounded,
        "the control case needs a body on the ground"
    );

    // The body rises through the pad's cell on the way up, and a body that is *rising* must not be
    // thrown: it is on its way past the pad, not onto it. (It is thrown on the way back down, when
    // it really has fallen onto the pad from above — that is the pad working, and the assertion at
    // the end of this half says so.)
    let mut rising_inside_the_pad = false;
    let mut launched_while_rising = false;
    let mut thrown_on_the_way_down = false;
    for _ in 0..160u32 {
        let rising = sim.player().vy < 0.0;
        sim.step(AgentAction::JumpInPlace);
        let inside = sim
            .player()
            .aabb()
            .overlaps(&sim::Rect::new(1.0, 2.0, 1.0, 1.0));
        if rising && inside {
            rising_inside_the_pad = true;
        }
        if rising && sim.player().vy <= -PHYSICS.spring_v0 {
            launched_while_rising = true;
        }
        if !rising && sim.player().vy == -PHYSICS.spring_v0 {
            thrown_on_the_way_down = true;
        }
        if sim.outcome().is_finished() {
            break;
        }
    }
    assert!(
        rising_inside_the_pad,
        "the fixture has to actually put the body through the pad's cell while it is rising, or this \
         test is vacuous"
    );
    assert!(
        !launched_while_rising,
        "a body inside a pad's cell that is not falling onto it must not be launched: the pad's rule \
         is about *arriving* on it"
    );
    assert!(
        thrown_on_the_way_down,
        "and the same body, falling back onto the pad, must be thrown — otherwise this test would \
         pass just as well against a pad that does nothing at all"
    );

    // And a body walking across a pad on the ground is not launched either: `vy` is zero for a body
    // standing on something, so the pad is a floor you can walk over and a target you have to land
    // on. This is the half that keeps a pad in a corridor from being an obstacle.
    let level = fixture("pad-corridor", PAD_IN_THE_ARC, (1, 3), &[("goal", 11, 3)]);
    let mut walker = sim::Sim::new(&level, 9);
    common::settle(&mut walker);
    let mut lowest_walk = 0.0f32;
    for _ in 0..80u32 {
        walker.step(AgentAction::Run);
        lowest_walk = lowest_walk.min(walker.player().vy);
    }
    assert_eq!(
        lowest_walk, 0.0,
        "walking over a pad is not landing on it: vy must never go negative"
    );
}

/// Ground row 7, the first ten columns of it brittle: a floor that is a bridge with a deadline.
const BRITTLE_BRIDGE: &[&str] = &[
    "............",
    "............",
    "............",
    "............",
    "............",
    "............",
    "............",
    "#dddddddddd#",
];

#[test]
fn a_cracked_tile_holds_a_body_up_and_then_takes_itself_out_of_the_level() {
    let authored = fixture("brittle", BRITTLE_BRIDGE, (1, 6), &[("goal", 11, 6)]);
    let mut sim = sim::Sim::new(&authored, 11);
    // The route table this episode steers by, taken before anything breaks. As a raw pointer:
    // the body has to be stepped to break the tile, and stepping it needs `&mut sim`.
    let routes = std::ptr::from_ref(sim.level().routes());

    // Frame 1 is the frame the body is standing on brittle ground, so it is the frame the fuse
    // starts. Before anything else: a cracked tile is *solid*, which is the whole reason a brittle
    // bridge is a bridge rather than a row of holes.
    sim.step(AgentAction::Walk);
    assert!(sim.player().grounded, "the spawn is on the tile below it");
    assert_eq!(
        sim.level().tile(1, 7),
        Some(Tile::Cracked),
        "a cracked tile holds a body up; it is only brittle, not missing"
    );

    // The fuse is exactly `CRUMBLE_TICKS` frames long, counted rather than asserted loosely:
    // "a while" is what a fuse is when nobody can say how long. A tile first stood on during frame
    // F is underfoot for frames F..=F+CRUMBLE_TICKS-1 and gone on frame F+CRUMBLE_TICKS, so the
    // body is grounded on every frame up to the last one of the fuse and the tile leaves the level
    // on the frame after — which is the whole of what a level author is told when the number is
    // quoted at them.
    for _ in 0..(CRUMBLE_TICKS - 1) {
        sim.step(AgentAction::Walk);
        assert_eq!(
            sim.level().tile(1, 7),
            Some(Tile::Cracked),
            "the tile must last the whole fuse, frame {}",
            sim.stats().frames
        );
        assert!(sim.player().grounded, "and hold the body up while it does");
    }
    assert_eq!(
        sim.stats().frames,
        CRUMBLE_TICKS,
        "the last frame of the fuse is a frame the body is still standing on it"
    );
    sim.step(AgentAction::Walk);
    assert_eq!(
        sim.level().tile(1, 7),
        Some(Tile::Empty),
        "standing on brittle ground for CRUMBLE_TICKS takes it out of the level"
    );

    // **The route table outlives the ground it was built on.** A brittle tile burning out under the
    // body is an episode's own damage, not an edit to the level: the flag has not moved, so the
    // compass the waypoint sensor steers by does not swing, and the geodesic `hops_to_flag` the
    // score reads stays a reading of the level's route rather than of one body's damage. The
    // physics is exact either way — it reads the live grid, as the assertion above shows — so what
    // is shared is only the route.
    //
    // It is also the difference between a level with cracked ground being trainable and being
    // unaffordable: the sweep behind a route table is milliseconds, and it used to be run again for
    // every tile every episode broke (measured on a 60x15 level with six brittle tiles: 0.299
    // s/generation re-sweeping, 0.015 with the route kept).
    assert!(
        std::ptr::eq(routes, std::ptr::from_ref(sim.level().routes())),
        "a broken tile must not redraw the route table: the flag it points at has not moved"
    );

    // A tile the body never stood on never lights: brittleness is a trap for the body that *chooses*
    // to cross it, not a floor with a countdown on it.
    assert_eq!(
        sim.level().tile(6, 7),
        Some(Tile::Cracked),
        "a cracked tile nobody stands on is untouched"
    );

    // **The document is not the episode.** The authored level still holds the tile it was drawn
    // with, so a run can be replayed against it and the level file is never the thing a body
    // destroys.
    assert_eq!(
        authored.tile(1, 7),
        Some(Tile::Cracked),
        "the sim mutates its own copy of the level, not the one it was built from"
    );

    // And with the tile gone the body is standing on nothing, over nothing: the fall ends in the
    // pit the level always had under that floor.
    let y0 = sim.player().y;
    let mut died = false;
    for _ in 0..120u32 {
        sim.step(AgentAction::Walk);
        if sim.outcome() == sim::Outcome::Died {
            died = true;
            break;
        }
    }
    assert!(
        sim.player().y > y0 + 1.0 || died,
        "the body should fall through the hole it made (y {y0} -> {})",
        sim.player().y
    );
    assert!(died, "and the fall out of the world is a death");
}

#[test]
fn a_walkers_patrol_does_not_light_a_cracked_floor() {
    // The player stands on sound ground at the left end; the patrol lives on the brittle span.
    let level = fixture(
        "walker-brittle",
        BRITTLE_BRIDGE,
        (0, 6),
        &[("walker", 6, 6), ("goal", 11, 6)],
    );
    let mut sim = sim::Sim::new(&level, 3);
    for _ in 0..300u32 {
        sim.step(AgentAction::Walk);
    }
    assert!(
        sim.walkers().iter().any(|w| w.alive),
        "the patrol has to still be there, or this proves nothing"
    );
    for tx in 1..=10 {
        assert_eq!(
            sim.level().tile(tx, 7),
            Some(Tile::Cracked),
            "a walker crossed column {tx} and the floor it walks on is still there: a patrol must \
             not demolish the level it lives in"
        );
    }
}

/// **The four brittle materials are one behaviour**, so a cracked beach, wall and decking are the
/// same trap drawn in a different ground — the assertion `sand_is_a_material_and_not_a_mechanic`
/// makes for the sound four, made for the brittle ones.
///
/// Four levels that differ only in which glyph their floor is written in, with the same action
/// sequence run through all four: every frame has to agree on the state *and* on the observation,
/// the tile has to leave the level on the same tick, and the tiles the body never stood on have to
/// survive. The observation half is the one that matters to the population — all four read the same
/// `ENC_CRUMBLE`, so which material an author chose is not a new problem to learn — and the state
/// half is what makes "one behaviour" true rather than aspirational: the fuse, the removal and the
/// hole left behind are the same arithmetic in all four.
#[test]
fn the_four_brittle_materials_are_one_behaviour() {
    let sky = "............".to_string();
    let level = |glyph: char| {
        let mut rows = vec![sky.clone(); 7];
        rows.push(format!("#{}{glyph}#", glyph.to_string().repeat(9)));
        fixture("brittle-material", &rows, (1, 6), &[("goal", 11, 6)])
    };

    let glyphs = ['d', 's', 'b', 'w'];
    let mut sims: Vec<(char, sim::Sim)> = glyphs
        .iter()
        .map(|g| (*g, sim::Sim::new(&level(*g), 5)))
        .collect();
    let mut broke_at: Vec<Option<u32>> = vec![None; sims.len()];
    for frame in 1..=120u32 {
        for (i, (_, sim)) in sims.iter_mut().enumerate() {
            sim.step(AgentAction::Walk);
            if broke_at[i].is_none() && sim.level().tile(1, 7) == Some(Tile::Empty) {
                broke_at[i] = Some(frame);
            }
        }
        for pair in sims.windows(2) {
            assert_eq!(
                pair[0].1.state_hash(),
                pair[1].1.state_hash(),
                "the {:?} and {:?} floors diverge at frame {frame}: a brittle material is behaving \
                 as something other than brittle ground",
                pair[0].0,
                pair[1].0
            );
            assert_eq!(
                pair[0].1.observation(),
                pair[1].1.observation(),
                "an agent can tell the {:?} and {:?} floors apart at frame {frame}, which makes a \
                 material a mechanic",
                pair[0].0,
                pair[1].0
            );
        }
    }

    // The tile the body stood on left the level, on the same tick in all four.
    assert!(
        broke_at[0].is_some(),
        "no floor broke in 120 frames of standing on it, so this test proves nothing"
    );
    for (glyph, when) in glyphs.iter().zip(&broke_at) {
        assert_eq!(
            *when, broke_at[0],
            "the {glyph:?} floor broke on a different frame: {broke_at:?}"
        );
    }

    // And the tiles nobody stood on are still there in every one of the four.
    for (glyph, sim) in &sims {
        for tx in 2..=10 {
            assert_eq!(
                sim.level().tile(tx, 7).map(|t| t.is_cracked()),
                Some(true),
                "{glyph:?}: column {tx} was never stood on and must still be brittle ground"
            );
        }
    }
}
