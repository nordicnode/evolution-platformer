//! The keyboard contract against the shipped content (development_plan.md §12 M1:
//! "a human can beat L1–L3").
//!
//! `render::play`'s unit tests pin the latch's *mechanics*: a press keeps its intent
//! alive for [`HOLD_WINDOW`], a release ends it at once, jump beats direction. This
//! file pins the number those mechanics are chosen for — how long one press keeps
//! its intent alive, measured against obstacles in the real pack.
//!
//! It needs its own test because the two numbers are set in different places and only
//! their conjunction is playable. The latch exists so that a slow terminal auto-repeat
//! is not a crawl (measured before it existed: four seconds of holding right advanced
//! the player 3 pixels), and it is wide enough to give a jump real airtime. The
//! *window* — the range of take-off moments that clear an obstacle — is a property of
//! the physics, and it is what a person at the keyboard feels as "I timed that".
//!
//! Measured on L2's first gap, by sweeping the take-off tick (this test does it in
//! CI):
//!
//! | press | take-off window |
//! |---|---|
//! | one frame (16 ms) | 9 ticks, 144 ms |
//! | one latched press (13 ticks, 208 ms) | **14 ticks, 224 ms** |
//! | held down (640 ms) | 19 ticks, 304 ms |
//!
//! A one-frame press is not punished, because the physics force-holds a jump for
//! `min_jump_hold` ticks so that a short action repeat cannot shrink every jump to a
//! hop. The latch's contribution is the extra third of a second of margin above that
//! floor — which is the difference between an obstacle you clear when you misjudge it
//! and one you clear only on the frame you meant.
//!
//! Not covered here: L3 and beyond. A player who holds right and jumps whenever they
//! see a hazard dies to L3's spikes at every reaction latency, and that is the
//! levels' design rather than a defect — with a jump every tick they win L1 and L2
//! and die on L3, because landing on a spike is fatal, so L3 needs *placed* jumps.
//! Whether a person can place them is what the human playtest is for (M1, M3's fun
//! gate); this file only pins the floors a first-time player has to clear.

use render::play::{FRAME_TIME, HOLD_WINDOW};
use sim::{AgentAction, Level, Outcome, Sim, Tile};

/// The shipped pack sits at the repository root, two levels up from this crate.
fn shipped(slug: &str) -> Level {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../levels")
        .join(format!("{slug}.json"));
    match Level::from_json_file(&path) {
        Ok(level) => level,
        Err(err) => panic!("shipped level {slug} failed to load from {path:?}: {err}"),
    }
}

/// Ticks one press is worth in `play`, derived from the two constants that decide it.
///
/// `play` steps exactly one tick per rendered frame, `FRAME_TIME` apart, and it asks
/// `Keys::is_down(now)` where `now` is the frame's start. A press at frame *k* is
/// therefore live for frame *k* and every frame whose start is still inside the
/// window: `HOLD_WINDOW / FRAME_TIME` more of them.
fn latch_ticks() -> u32 {
    (HOLD_WINDOW.as_millis() / FRAME_TIME.as_millis()) as u32 + 1
}

/// The episode's frame cap, from the sim's own rule (§7.4.6).
fn cap(level: &Level) -> u32 {
    level.width as u32 * sim::FRAMES_PER_TILE
}

/// Hold right for `takeoff` ticks, then one latched run-jump, then right again to the
/// end. Returns `(outcome, best_x)`.
///
/// This is exactly the action stream one press of space produces while right is held.
/// A take-off past the cap never jumps, which is the no-jump control.
fn after_one_press(level: &Level, takeoff: u32) -> (Outcome, f32) {
    let hold = latch_ticks();
    let mut sim = Sim::new(level, 1);
    for t in 0..cap(level) {
        let action = if t < takeoff {
            AgentAction::Run
        } else if t < takeoff.saturating_add(hold) {
            AgentAction::RunJump
        } else {
            AgentAction::Run
        };
        sim.step(action);
        if sim.outcome().is_finished() {
            break;
        }
    }
    (sim.outcome(), sim.stats().best_x)
}

/// The take-off window a latched press must give a player on the first gap: 12 ticks,
/// 192 ms, against the 14 ticks (224 ms) the shipped constants measure.
///
/// The floor is here so that shortening `HOLD_WINDOW` to make the controls feel
/// snappier cannot quietly turn the shipped levels into tick-perfect timing tests:
/// at a 50 ms latch the window drops to the nine ticks a one-frame press gives, and
/// a fifth of a second of mistiming is the least that can be called playable by hand.
const MIN_HUMAN_WINDOW_TICKS: usize = 12;

/// L2's first gap is columns 18–20; its far edge is 21. Clearing it means standing
/// beyond it, not merely entering it.
const PAST_L2_GAP: f32 = 21.5;

#[test]
fn the_shipped_latch_gives_the_first_gap_a_human_window() {
    let level = shipped("l2-first-gap");
    let (no_jump, no_jump_x) = after_one_press(&level, cap(&level) + 10);

    let window: Vec<u32> = (0..cap(&level))
        .filter(|t| after_one_press(&level, *t).1 > PAST_L2_GAP)
        .collect();
    let span = match (window.first(), window.last()) {
        (Some(first), Some(last)) => format!("{first}–{last}"),
        _ => "none".to_string(),
    };

    assert_eq!(
        no_jump,
        Outcome::Died,
        "without a press the gap has to stay fatal, or this test measures nothing: \
         {no_jump:?} at x {no_jump_x:.2}"
    );
    assert!(
        window.len() >= MIN_HUMAN_WINDOW_TICKS,
        "one latched press ({latch} ticks) has to clear the gap over a take-off window \
         a person can land: {} take-off(s) worked ({span}), against a floor of {} ticks \
         ({:.0} ms)",
        window.len(),
        MIN_HUMAN_WINDOW_TICKS,
        MIN_HUMAN_WINDOW_TICKS as f32 * FRAME_TIME.as_millis() as f32,
        latch = latch_ticks()
    );
}

/// Opens the level by holding right, which is the floor every other control sits on:
/// if a held key does not move the player, nothing else in `play` matters.
#[test]
fn holding_right_beats_the_opening_level() {
    let level = shipped("l1-flat-run");
    let mut sim = Sim::new(&level, 1);
    for _ in 0..cap(&level) {
        sim.step(AgentAction::Run);
        if sim.outcome().is_finished() {
            break;
        }
    }
    assert_eq!(
        sim.outcome(),
        Outcome::Won,
        "L1 is the level a first-time player meets: best_x {:.2} after {} frames",
        sim.stats().best_x,
        sim.stats().frames
    );
}

/// A player who keeps right held and presses jump when they see something in front of
/// them, `latency` ticks later — the shape of a first playtest, not a planner.
///
/// They see only what is in front of their body: a spike at body height in the next
/// column, or no ground under it. One press per airtime, because you cannot press
/// again with your feet off the floor.
fn reacting(level: &Level, latency: u32) -> (Outcome, f32) {
    let mut sim = Sim::new(level, 1);
    let mut seen: Vec<bool> = Vec::new();
    let mut latch_until = 0;
    for t in 0..cap(level) {
        let front = (sim.player().x + sim.player().w).floor() as i32 + 1;
        let feet = (sim.player().y + sim.player().h).floor() as i32;
        let hazard = [feet - 1, feet, feet + 1].iter().any(|row| {
            level.get(front, *row) == Tile::Spike || level.get(front + 1, *row) == Tile::Spike
        }) || !(level.is_solid(front, feet) || level.is_solid(front, feet + 1));
        seen.push(hazard);

        let saw = t >= latency && seen[(t - latency) as usize];
        let action = if t < latch_until {
            AgentAction::RunJump
        } else if saw && sim.player().grounded {
            latch_until = t + latch_ticks();
            AgentAction::RunJump
        } else {
            AgentAction::Run
        };
        sim.step(action);
        if sim.outcome().is_finished() {
            break;
        }
    }
    (sim.outcome(), sim.stats().best_x)
}

#[test]
fn a_reacting_player_clears_the_first_gap() {
    let l1 = shipped("l1-flat-run");
    let l2 = shipped("l2-first-gap");

    // 0 to 240 ms of reaction delay, in steps a person might have.
    let latencies: Vec<u32> = (0..=15).step_by(3).collect();

    for latency in &latencies {
        let (outcome, x) = reacting(&l1, *latency);
        assert_eq!(
            outcome,
            Outcome::Won,
            "L1 has nothing to react to, so any latency beats it: {} ms lost at x {x:.2}",
            latency * 16
        );
    }

    let cleared: Vec<u32> = latencies
        .iter()
        .copied()
        .filter(|latency| reacting(&l2, *latency).1 > PAST_L2_GAP)
        .collect();
    assert!(
        !cleared.is_empty(),
        "a player who reacts has to be able to clear the first gap, or the opening \
         jumps are memorisation: no latency in {latencies:?} worked"
    );
}
