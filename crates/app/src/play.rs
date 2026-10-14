//! Playtest the level being edited, in the same window.
//!
//! This is M4's "human playtest mode inside the editor" and it is the fastest
//! possible answer to "is what I just drew actually fun to play?" — the level never
//! leaves the document, so there is no export, no reload, no separate binary.
//!
//! Two things are deliberate here.
//!
//! * **The action mapping is not reimplemented.** [`action_for`] is the terminal
//!   playtest's own function (`render::play`), so the keys mean the same thing in
//!   both, and a change to the priority order cannot silently apply to only one of
//!   them.
//! * **The sim runs on a fixed timestep with an accumulator** (§5.1), not one tick
//!   per rendered frame. A frame that takes 30 ms must advance the physics 30 ms
//!   worth, or the game would slow down on a busy machine — and the terminal
//!   playtest's one-tick-per-frame behaviour would be a *different game*.

use egui::{Context, Key};
use sim::{AgentAction, Level, Sim};

use crate::view::AgentAnim;

pub use render::play::action_for;

/// Physics ticks per second, from the sim's own timestep.
const TICK: f32 = sim::DT;

/// How much simulation a single frame may catch up on. A long stall (a breakpoint,
/// a slow redraw) must not teleport the player through a spike.
const MAX_CATCHUP: f32 = 0.25;

/// Which intents are held down right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sprint: bool,
    pub brake: bool,
}

impl Keys {
    /// Read the held keys out of egui's input state.
    ///
    /// A GUI reports the real key state, so unlike the terminal path there is no
    /// latch: a held key is simply held.
    ///
    /// Sprint is shift, read as a modifier rather than as a key: the terminal playtest has
    /// no key event for "shift alone" either, so both paths express sprint the same way —
    /// `shift` plus whatever you are doing.
    pub fn read(ctx: &Context) -> Keys {
        ctx.input(|i| Keys {
            left: i.key_down(Key::ArrowLeft) || i.key_down(Key::A),
            right: i.key_down(Key::ArrowRight) || i.key_down(Key::D),
            jump: i.key_down(Key::Space) || i.key_down(Key::ArrowUp) || i.key_down(Key::W),
            sprint: i.modifiers.shift,
            brake: i.key_down(Key::ArrowDown) || i.key_down(Key::S),
        })
    }

    pub fn action(&self) -> AgentAction {
        action_for(self.left, self.right, self.jump, self.sprint, self.brake)
    }
}

/// One playtest of one level.
pub struct Session {
    sim: Sim,
    seed: u64,
    accumulator: f32,
    /// Wall-clock seconds since the session started, for animation only.
    pub clock: f32,
    /// The player's animation: the run cycle, the landing pose and the dust. Owned here
    /// because it is a fact about the *last few frames*, which a stateless view cannot
    /// recover from the sim alone.
    anim: AgentAnim,
}

impl Session {
    pub fn new(level: &Level, seed: u64) -> Session {
        Session {
            sim: Sim::new(level, seed),
            seed,
            accumulator: 0.0,
            clock: 0.0,
            anim: AgentAnim::new(),
        }
    }

    /// Start over on the current level — after editing it, or after dying.
    pub fn restart(&mut self, level: &Level) {
        let clock = self.clock;
        *self = Session::new(level, self.seed);
        self.clock = clock;
    }

    /// The player's animation for this frame.
    pub fn anim(&self) -> &AgentAnim {
        &self.anim
    }

    /// Advance by `dt` seconds of wall clock, in whole sim ticks, asking `decide` for the
    /// action of each tick as it is about to run.
    ///
    /// This is the seam an evolved champion comes in through. A champion's episode is a
    /// *sequence of per-tick actions*, decided every `action_repeat` ticks
    /// ([`evo`]'s `EpisodeRunner`), and it can only be reproduced by being asked tick by
    /// tick: the frame is the wrong unit for it, because a frame runs zero, one or two
    /// ticks depending on how long it took, and a decision taken once per frame would slide
    /// along the episode as the frame rate changed. Taking it inside the loop is what makes
    /// the picture on screen the episode that was scored, at any frame rate.
    ///
    /// [`Session::advance`] is this with a decision that never changes: a playtester holds
    /// keys, and a held key is the same action on every tick of the frame.
    pub fn advance_actions(&mut self, dt: f32, mut decide: impl FnMut(&Sim) -> AgentAction) {
        self.clock += dt;
        self.accumulator += dt.min(MAX_CATCHUP);
        let mut stepped = 0u32;
        while self.accumulator >= TICK {
            self.accumulator -= TICK;
            let action = decide(&self.sim);
            self.sim.step(action);
            stepped += 1;
        }
        // The animation is advanced by the *simulated* time, not the wall clock: a frame
        // that ran no ticks must not make the run cycle or the dust timers move, or the
        // legs would run while the agent stands still on a slow machine.
        if stepped > 0 {
            self.anim.advance(&self.sim, stepped as f32 * TICK);
        }
    }

    /// Advance by `dt` seconds of wall clock with one key state held for all of it.
    pub fn advance(&mut self, dt: f32, keys: Keys) {
        let action = keys.action();
        self.advance_actions(dt, |_| action);
    }

    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// The line under the canvas: how the run is going.
    pub fn summary(&self) -> String {
        let stats = self.sim.stats();
        // Closer-to-the-flag rather than further-along-the-level: the score *is* the distance
        // closed on the goal, so the readout has to be the same quantity or it contradicts
        // the champion's own fitness (see `EpisodeStats::progress_pixels`). "flag" is how far
        // the body has ever been from it, which is the number that means something on a level
        // whose goal is above rather than to the right; `x` stays because it is what a
        // side-scroller watcher is used to reading.
        let progress = if stats.goal_distance_pixels > 0.0 {
            100.0 * stats.progress_pixels / stats.goal_distance_pixels
        } else {
            100.0
        };
        format!(
            "{}  frame {}/{}  x {:.1}  flag {:.1} tiles  progress {:.0}%  stomps {}  jumps {}  {}",
            self.sim.outcome().as_str(),
            stats.frames,
            self.sim.max_frames(),
            stats.best_x,
            stats.closest_approach,
            progress,
            stats.stomps,
            stats.jumps,
            if stats.frames == 0 {
                "ready"
            } else {
                "press R to restart"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::{AgentAction, Level};

    /// Holding a key and asking for its action every tick are the same episode.
    ///
    /// The per-tick form exists for a champion, whose decisions do change inside a frame;
    /// the playtest's does not, and this is the test that says so. A frame long enough to
    /// carry two ticks is part of the fixture on purpose: it is the case where "decide once
    /// per frame" and "decide once per tick" differ, and the one the champion's cadence
    /// depends on.
    ///
    /// The level is the flat one, and deliberately not `l3-gauntlet` any more: holding right
    /// plus jump used to bunny-hop over everything, and now it is one jump followed by a run
    /// into the first pit. A policy that dies in three seconds cannot carry a 240-frame
    /// comparison of frame sizes, and the claim here is about the session's tick discipline
    /// rather than about the policy.
    #[test]
    fn a_held_key_and_a_fixed_decision_are_the_same_episode() {
        let level = shipped("l1-flat-run");
        let keys = Keys {
            right: true,
            jump: true,
            ..Keys::default()
        };
        let mut held = Session::new(&level, 11);
        let mut asked = Session::new(&level, 11);
        for frame in 0..240 {
            let dt = if frame % 7 == 0 { 0.03 } else { 1.0 / 60.0 };
            let action = keys.action();
            held.advance(dt, keys);
            asked.advance_actions(dt, |_| action);
        }
        assert_eq!(held.sim().state_hash(), asked.sim().state_hash());
        assert_eq!(held.sim().stats().frames, asked.sim().stats().frames);
        assert!(held.sim().stats().frames > 240);
    }

    /// A decision that does change within a frame is taken on every tick of it.
    ///
    /// The mirror of the test above, and the property the champion depends on: the same
    /// wall-clock, delivered in different frame sizes, is the same episode — because the
    /// decisions are made against the sim's tick count and never against the frame's.
    #[test]
    fn per_tick_decisions_are_the_same_episode_at_any_frame_rate() {
        let level = shipped("l2-first-gap");
        // The champion's cadence, in miniature: a decision every fourth tick, held in
        // between. Any dependence on the frame would show up as a different hash.
        let decide = |sim: &Sim| {
            let decided = sim.stats().frames / ACTION_REPEAT;
            ACTIONS[decided as usize % ACTIONS.len()]
        };
        let sixty = {
            let mut s = Session::new(&level, 3);
            for _ in 0..300 {
                s.advance_actions(1.0 / 60.0, decide);
            }
            s
        };
        let thirty = {
            let mut s = Session::new(&level, 3);
            for _ in 0..150 {
                s.advance_actions(1.0 / 30.0, decide);
            }
            s
        };
        assert_eq!(thirty.sim().stats().frames, sixty.sim().stats().frames);
        assert_eq!(thirty.sim().state_hash(), sixty.sim().state_hash());
    }

    /// Ticks between decisions in the two tests above, and the actions to cycle through
    /// there so the decision is not the constant one the playtest makes.
    const ACTION_REPEAT: u32 = 4;
    const ACTIONS: [AgentAction; 4] = [
        AgentAction::Run,
        AgentAction::RunJump,
        AgentAction::Walk,
        AgentAction::Run,
    ];

    fn shipped(slug: &str) -> Level {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"));
        Level::from_json_file(&path).unwrap_or_else(|e| panic!("{slug}: {e}"))
    }

    /// The desktop shell must mean the same thing by the keys as the terminal
    /// playtest: this is the one mapping both call.
    #[test]
    fn the_action_priority_is_the_same_for_both_ways_of_playing() {
        use sim::AgentAction::*;
        assert_eq!(action_for(false, false, false, false, false), Walk);
        assert_eq!(action_for(true, false, false, false, false), Retreat);
        assert_eq!(
            action_for(true, true, false, false, false),
            Run,
            "right beats left"
        );
        assert_eq!(action_for(false, false, false, false, true), Brake);
        assert_eq!(action_for(false, false, true, false, false), JumpInPlace);
        assert_eq!(action_for(false, true, true, false, false), RunJump);
        assert_eq!(
            action_for(true, false, true, false, false),
            RetreatJump,
            "a leftward jump is its own action now, not a neutral jump"
        );
        assert_eq!(
            action_for(true, true, true, true, true),
            SprintJump,
            "sprint and jump forward beats everything else held"
        );
        assert_eq!(
            action_for(false, true, false, true, false),
            Sprint,
            "sprint is a forward run, not a jump"
        );
        assert_eq!(
            action_for(false, false, false, true, false),
            Walk,
            "sprint with nothing to sprint in is no gear at all"
        );
    }

    #[test]
    fn the_sim_advances_by_wall_clock_in_fixed_ticks() {
        let level = shipped("l1-flat-run");
        let mut session = Session::new(&level, 1);
        // Half a second of play at 60 Hz is thirty ticks, whatever the frame rate.
        for _ in 0..30 {
            session.advance(1.0 / 60.0, Keys::default());
        }
        assert_eq!(session.sim().stats().frames, 30, "one tick per 1/60 s");

        // A frame that took 30 ms must not lose physics: the accumulator carries it.
        let mut session = Session::new(&level, 1);
        session.advance(0.03, Keys::default());
        assert_eq!(
            session.sim().stats().frames,
            1,
            "0.03 s is one tick, 0.018 left"
        );
        session.advance(0.03, Keys::default());
        assert_eq!(session.sim().stats().frames, 3, "the remainder was carried");

        // And a stall is clamped: a two-second frame must not teleport the player.
        let mut session = Session::new(&level, 1);
        session.advance(2.0, Keys::default());
        assert!(
            session.sim().stats().frames <= (MAX_CATCHUP / TICK) as u32 + 1,
            "a long hitch is clamped, got {}",
            session.sim().stats().frames
        );
    }

    #[test]
    fn holding_right_beats_the_opening_level_through_the_real_input_path() {
        let level = shipped("l1-flat-run");
        let mut session = Session::new(&level, 1);
        let keys = Keys {
            right: true,
            ..Keys::default()
        };
        for _ in 0..1200 {
            session.advance(1.0 / 60.0, keys);
            if session.sim().outcome().is_finished() {
                break;
            }
        }
        assert_eq!(
            session.sim().outcome(),
            sim::Outcome::Won,
            "{}",
            session.summary()
        );
    }

    #[test]
    fn a_restart_rewinds_the_episode_and_keeps_the_level() {
        let level = shipped("l2-first-gap");
        let mut session = Session::new(&level, 7);
        let keys = Keys {
            right: true,
            ..Keys::default()
        };
        for _ in 0..120 {
            session.advance(1.0 / 60.0, keys);
        }
        assert!(session.sim().stats().frames > 0);
        session.restart(&level);
        assert_eq!(session.sim().stats().frames, 0);
        assert_eq!(session.sim().outcome(), sim::Outcome::Running);
        // The same seed means the restart is the same episode, not a new one.
        assert_eq!(
            session.sim().state_hash(),
            Session::new(&level, 7).sim().state_hash()
        );
    }
}
