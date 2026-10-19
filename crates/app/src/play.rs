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

use std::cmp::Ordering;

use egui::{Context, Key};
use evo::{EpisodeRunner, EvalConfig, FastWin, Genome};
use sim::{AgentAction, Level, Outcome, Sim};

use crate::view::AgentAnim;

pub use render::play::action_for;

/// Physics ticks per second, from the sim's own timestep.
const TICK: f32 = sim::DT;

/// How much simulation a single frame may catch up on. A long stall (a breakpoint,
/// a slow redraw) must not teleport the player through a spike.
const MAX_CATCHUP: f32 = 0.25;

/// How one time stands against another, in words: `sweep` at 505 frames against 553 is
/// `"0.80s faster"`.
///
/// A hundredth of a second is the resolution [`sim::format_time`] writes below a minute, so it is
/// the resolution a gap is quoted in: two times a frame apart read as two times, and two times that
/// are the same read as the same rather than as `0.00s faster` — which would be a claim about a race
/// that did not happen.
fn gap(mine: u32, theirs: u32) -> String {
    match mine.cmp(&theirs) {
        Ordering::Equal => "the same time, to the frame".to_string(),
        Ordering::Less => format!("{:.2}s faster", seconds(theirs - mine)),
        Ordering::Greater => format!("{:.2}s slower", seconds(mine - theirs)),
    }
}

/// A count of frames as seconds. The sim's own timestep, named once here rather than spelled as
/// `60.0` at each arithmetic site.
fn seconds(frames: u32) -> f64 {
    frames as f64 * sim::DT as f64
}

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

    /// No key held at all: the state the clock waits in before the first input.
    pub fn is_idle(&self) -> bool {
        *self == Keys::default()
    }
}

/// How long a death is sat with before the session puts the agent back at the spawn.
///
/// A beat rather than an instant: the viewer has to *see* where they died, or every
/// death reads as a flicker and nothing is learned from it. One second is long enough
/// to read the spot and short enough that a retry never waits on the game. `R` still
/// restarts at once — this is the pause before the game does it for you.
pub const AUTO_RESTART_DELAY: f32 = 1.0;

/// The episode to race: whose it is, and the config it was scored with.
///
/// The window builds one out of the run's **quickest win** — the record the replay stars and
/// the dock quotes — so "the champion" would be the wrong word for it: on a run with two
/// winners the champion is a different agent ([`crate::replay::StarBasis`]). Nothing about
/// the field depends on which of the two records it is; everything about the *numbers* does,
/// because the ghost is raced against exactly the episode this spec names.
///
/// The genome and the seed are the episode; the eval config is its cadence
/// (`action_repeat`), without which the same network on the same seed is a different
/// run of it. Kept as the three parts rather than a built runner so a restart can
/// rebuild the ghost from scratch — a runner that had already reached the flag would
/// otherwise be a ghost that never moves again.
#[derive(Clone, Debug)]
pub struct GhostSpec {
    pub genome: Genome,
    pub eval: EvalConfig,
    pub seed: u64,
}

/// The record-holder's live episode, running on the same clock as the player's.
struct Ghost {
    runner: EpisodeRunner,
    spec: GhostSpec,
    anim: AgentAnim,
    visible: bool,
    /// The ghost's ears: it is heard (jump, stomp, the flag) at a lower gain
    /// than the player, because it is company in the race, not the racer.
    ear: crate::sound::Ear,
}

/// One playtest of one level.
pub struct Session {
    sim: Sim,
    seed: u64,
    accumulator: f32,
    /// Wall-clock seconds since the session started, for animation only.
    pub clock: f32,
    /// Whether the clock has started: false until the first input.
    ///
    /// The timer is a race timer, not a load timer — it must not bill the viewer for
    /// reading the level. Nothing steps until a key is held, so the walkers hold
    /// their patrol and the frame count stays at zero, and the first tick is the
    /// first thing the player asked for.
    started: bool,
    /// Wall-clock moment the episode ended, if it has. The sim's own frame count has
    /// stopped by then — a finished episode steps nowhere — so the pause before the
    /// automatic restart is measured on this clock instead ([`AUTO_RESTART_DELAY`]).
    finished_wall: Option<f32>,
    /// The record-holder's ghost, when the run has one to show.
    ghost: Option<Ghost>,
    /// The player's animation: the run cycle, the landing pose and the dust. Owned here
    /// because it is a fact about the *last few frames*, which a stateless view cannot
    /// recover from the sim alone.
    anim: AgentAnim,
    /// Where the body was at the start of the last simulated tick, and how far the frame is
    /// between that tick and the next — the two halves of drawing a 60 Hz simulation on a
    /// display that is not 60 Hz (see [`Session::body_at`]).
    prev_body: Option<(f32, f32)>,
    /// The ghost's position at the start of the last tick, for the same reason.
    prev_ghost: Option<(f32, f32)>,
    /// `accumulator / TICK`, in `0..1`: how far into the next tick this frame is.
    render_alpha: f32,
    /// The simulated clock the animation was last advanced to, in seconds: `frames * TICK` plus the
    /// fraction of a tick the frame is into. Its *difference* is what the run cycle, the dust and the
    /// squash are advanced by, so the legs move with the drawn body rather than with the frames that
    /// happen to carry a tick (see [`Session::advance_actions`]).
    anim_clock: f32,
    /// The player's ears: what each tick did, turned into sound
    /// (`crate::sound`). Silent when `audible` is false.
    ear: crate::sound::Ear,
    /// Whether this session makes sound at all. A playtest does; the title
    /// screen's demo session does not (a menu that plays the game at you is
    /// showing off, and it can stay quiet about it), so the splash builds its
    /// session and then mutes it with [`Session::set_audible`].
    audible: bool,
    /// The quickest *playthrough* that has touched the flag in this session, if any.
    ///
    /// The player's own record, kept for the reason the run keeps the AI's ([`FastWin`]): a
    /// stopwatch that only ever showed the time you just took would make every retry the first
    /// attempt again. It is a session's record rather than the level's — nothing here writes a
    /// file — and it survives a restart, because a restart is a retry rather than a new person.
    best: Option<u32>,
}

impl Session {
    pub fn new(level: &Level, seed: u64) -> Session {
        Session {
            sim: Sim::new(level, seed),
            seed,
            accumulator: 0.0,
            clock: 0.0,
            started: false,
            finished_wall: None,
            ghost: None,
            anim: AgentAnim::new(),
            prev_body: None,
            prev_ghost: None,
            render_alpha: 0.0,
            anim_clock: 0.0,
            ear: crate::sound::Ear::new(),
            audible: true,
            best: None,
        }
    }

    /// Whether this session makes sound. See the `audible` field: the one
    /// caller that wants silence is the title screen's demo.
    pub fn set_audible(&mut self, audible: bool) {
        self.audible = audible;
    }

    /// Where to **draw** the body this frame: between the last two physics ticks, by however
    /// much of a tick the frame is into.
    ///
    /// This is the whole of the playtest's smoothness. The simulator ticks at 60 Hz no matter
    /// what the display does, so on a 144 Hz screen two frames in three advance no tick at all
    /// and the third jumps a whole tick's movement — a 60 Hz stutter on a screen doing twice
    /// that, which is exactly what "jerky" looked like. Interpolating between the two positions
    /// the frames are between turns the same ticks into motion at the display's own rate.
    ///
    /// Linear rather than eased, on purpose: the physics already integrates acceleration, so
    /// easing the *render* would double-smooth the run and make a sprint feel like it was on
    /// ice. What is drawn is where the body was at a time between two states it really was in.
    pub fn body_at(&self) -> (f32, f32) {
        let p = self.sim.player();
        match self.prev_body {
            Some((px, py)) => (
                px + (p.x - px) * self.render_alpha,
                py + (p.y - py) * self.render_alpha,
            ),
            None => (p.x, p.y),
        }
    }

    /// Where to draw the **camera** this frame: the centre of the body as it is drawn, not as the
    /// simulator last left it.
    ///
    /// The camera has to be in on the interpolation rather than beside it. A camera that follows the
    /// sim's last tick while the body is drawn *between* two ticks holds the scene still while the
    /// body advances a third of a tick, then moves the whole scene a tick's travel at once when the
    /// tick lands — so the body's offset from the centre saws from zero to a full tick's motion and
    /// snaps back, sixty times a second, under a picture that is otherwise drawing at the display's
    /// rate. That is the stutter the interpolation was added to remove, still there in the camera.
    /// Put on the same clock, the body sits at one fixed point on screen and the world scrolls under
    /// it smoothly; what is left of the loop is the integer pixel the camera is snapped to, which is
    /// the pixel grid this renderer draws on and not a frame-rate artefact.
    pub fn camera_centre(&self) -> (f32, f32) {
        let p = self.sim.player();
        let (x, y) = self.body_at();
        (x + p.w * 0.5, y + p.h * 0.5)
    }

    /// The same for the ghost, so the race is drawn on one clock.
    pub fn ghost_at(&self) -> Option<(f32, f32)> {
        let g = self.ghost.as_ref()?;
        let p = g.runner.sim().player();
        Some(match self.prev_ghost {
            Some((px, py)) => (
                px + (p.x - px) * self.render_alpha,
                py + (p.y - py) * self.render_alpha,
            ),
            None => (p.x, p.y),
        })
    }

    /// Start over on the current level — after dying.
    ///
    /// The clock goes back to waiting for input rather than running: a restart that
    /// started the timer itself would bill the retry for the death pause, and a held
    /// key starts it again on the very next frame anyway. The ghost, if there is one,
    /// is rebuilt from its spec so the race restarts together.
    pub fn restart(&mut self, level: &Level) {
        let clock = self.clock;
        let best = self.best;
        let audible = self.audible;
        let spec = self.ghost.as_ref().map(|g| g.spec.clone());
        let visible = self.ghost.as_ref().map(|g| g.visible).unwrap_or(true);
        *self = Session::new(level, self.seed);
        self.clock = clock;
        self.best = best;
        self.audible = audible;
        if let Some(spec) = spec {
            self.attach_ghost(spec);
            if let Some(ghost) = &mut self.ghost {
                ghost.visible = visible;
            }
        }
    }

    /// Is an automatic restart due: the episode died more than [`AUTO_RESTART_DELAY`]
    /// of wall clock ago?
    ///
    /// Deaths only, not wins or time-outs: a win is stood behind to be looked at, and
    /// a time-out is already the whole allowance spent, while a death is a retry —
    /// which is why `R` still restarts at once from any of them.
    pub fn restart_due(&self) -> bool {
        self.sim.outcome() == Outcome::Died
            && self
                .finished_wall
                .is_some_and(|t| self.clock - t >= AUTO_RESTART_DELAY)
    }

    /// Attach the record-holder's ghost to this session, racing from the flag of the
    /// player's own first input: it steps on the player's ticks, so the two share
    /// a clock and the ghost is the time to beat made visible.
    pub fn attach_ghost(&mut self, spec: GhostSpec) {
        let runner =
            EpisodeRunner::new(self.sim.level(), spec.genome.clone(), &spec.eval, spec.seed);
        self.ghost = Some(Ghost {
            runner,
            spec,
            anim: AgentAnim::new(),
            visible: true,
            ear: crate::sound::Ear::new(),
        });
    }

    /// Is there a ghost to show at all (as opposed to whether it is shown)?
    pub fn has_ghost(&self) -> bool {
        self.ghost.is_some()
    }

    /// Flip whether the ghost is drawn. Returns the new state, or `None` when there
    /// is no ghost to show — so a key that does nothing also says nothing.
    pub fn toggle_ghost(&mut self) -> Option<bool> {
        let ghost = self.ghost.as_mut()?;
        ghost.visible = !ghost.visible;
        Some(ghost.visible)
    }

    /// The ghost's live episode, when there is one and it is shown.
    ///
    /// `None` covers both "nothing to race" and "hidden": the canvas draws
    /// nothing either way, and the status line names the state that holds.
    pub fn ghost_sim(&self) -> Option<&Sim> {
        self.ghost
            .as_ref()
            .filter(|g| g.visible)
            .map(|g| g.runner.sim())
    }

    /// The ghost's animation, alongside [`Session::ghost_sim`].
    pub fn ghost_anim(&self) -> Option<&AgentAnim> {
        self.ghost.as_ref().filter(|g| g.visible).map(|g| &g.anim)
    }

    /// This episode's clock, in sim frames — the unit the AI's episode is timed in, and the unit
    /// the run's own record is kept in.
    ///
    /// It is the sim's frame count and not [`Session::clock`], which is wall-clock seconds kept for
    /// the animation: the two differ by every frame the machine dropped, and a time that drifted
    /// with the frame rate would not be a time the AI could be raced against.
    pub fn frames(&self) -> u32 {
        self.sim.stats().frames
    }

    /// The quickest winning playthrough of this session, if the flag has been touched at all.
    pub fn best(&self) -> Option<u32> {
        self.best
    }

    /// The race, in one line: this playthrough's clock, where it has got to, and the AI's best time
    /// for the level beside it.
    ///
    /// `record` is the run's quickest win, which the window takes from the packet — the playtest
    /// races a *time*, so that is what this takes rather than the record that carries it. `None` is a
    /// window that has never trained on this level, and the line says so instead of leaving a blank
    /// where a time would be: "nothing to race" and "0.00s to beat" are very different claims, and
    /// only one of them is true.
    ///
    /// Three shapes, and the clock is in all of them: where the episode has got to (at the flag, or
    /// still going, or how it ended), and how that time compares with the AI's — faster, slower, or
    /// the same to the frame. Both times go through `sim::format_time`, so the number a person is
    /// racing is written exactly as the panel wrote it before they came here.
    pub fn race_line(&self, record: Option<FastWin>) -> String {
        let mine = self.frames();
        let where_it_got = if !self.started {
            "ready — the clock starts on your first input".to_string()
        } else {
            match self.sim.outcome() {
                Outcome::Running => "and running".to_string(),
                Outcome::Won => "at the flag".to_string(),
                Outcome::Died if self.restart_due() => "and died — restarting…".to_string(),
                Outcome::Died => "and died — restarting… (R now)".to_string(),
                other => format!("and {}", other.as_str()),
            }
        };
        // The comparison is only made once there is a clock to compare. At 0.00 s every gap is
        // "faster", so quoting one before the first input read `0.00s ready — the AI's best is
        // 8.42s, 8.42s faster`: a claim that the player was eight seconds ahead for standing
        // still. So the clock's own state replaces the gap rather than decorating it — the line
        // says what the AI did, and that nothing has been run against it yet.
        let against = match record {
            Some(fast) if self.started => format!(
                "the AI's best time is {}, {}",
                sim::format_time(fast.frames),
                gap(mine, fast.frames)
            ),
            Some(fast) => format!(
                "the AI's best time is {} — you have not run yet",
                sim::format_time(fast.frames)
            ),
            None => "the run has no win to race yet".to_string(),
        };
        format!("{} {where_it_got} — {against}", sim::format_time(mine))
    }

    /// Is this playthrough matching or beating `record`? True once the clock is running and its
    /// time is at or under the AI's.
    ///
    /// One rule for both states of the race, because they are the same claim at different moments:
    /// while the episode is running it means "still in front" — the AI got to the flag in
    /// `record.frames` and this body has not used that many ticks yet — and once the flag is touched
    /// it means the time was beaten. Reading it as one comparison is what stops the window having a
    /// lamp that means one thing while the agent runs and another the moment it stops.
    ///
    /// The clock is the sim's, so it is the same quantity [`crate::play::Session::race_line`] prints
    /// and the same one the episode will be scored on.
    pub fn beating(&self, record: FastWin) -> bool {
        // A race that has not started is not one you are winning. The bare comparison is
        // `0.00 s <= any record`, so it lit the lamp gold on a standing start — telling a
        // player they were ahead of an agent that had already finished, before they had
        // pressed anything. That is the same mistake [`Session::race_line`] refuses to make.
        self.started && self.frames() <= record.frames
    }

    /// Remember a win, if it beats this session's own record.
    ///
    /// Called after every batch of ticks rather than only on the frame the flag is touched: a frame
    /// can carry several ticks, so the tick the episode ended on is not the same thing as the frame
    /// the window noticed it on, and the time has to be the sim's. `min` makes it idempotent, so
    /// the frames after the win change nothing.
    fn note_win(&mut self) {
        if self.sim.outcome() == Outcome::Won {
            let frames = self.frames();
            self.best = Some(self.best.map_or(frames, |b| b.min(frames)));
        }
    }

    /// The player's animation for this frame.
    pub fn anim(&self) -> &AgentAnim {
        &self.anim
    }

    /// Advance by `dt` seconds of wall clock, in whole sim ticks, asking `decide` for the
    /// action of each tick as it is about to run.
    ///
    /// This is the seam an evolved agent comes in through. Its episode is a
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
        let audible = self.audible;
        self.accumulator += dt.min(MAX_CATCHUP);
        let mut stepped = 0u32;
        while self.accumulator >= TICK {
            self.accumulator -= TICK;
            // Remember where the body (and the ghost) were *before* this tick: the renderer
            // interpolates between this position and the one the step is about to produce.
            let p = self.sim.player();
            self.prev_body = Some((p.x, p.y));
            if let Some(ghost) = &self.ghost {
                let g = ghost.runner.sim().player();
                self.prev_ghost = Some((g.x, g.y));
            }
            let action = decide(&self.sim);
            self.sim.step(action);
            if audible {
                self.ear.observe(&self.sim, 1.0);
            }
            // The ghost runs on the player's ticks, not on its own clock: one AI
            // tick per player tick is what keeps the two episodes on the same time, at
            // any frame rate. A finished ghost holds its ground — it reached the flag
            // (or died) and stays there to be looked at.
            if let Some(ghost) = &mut self.ghost {
                if !ghost.runner.is_finished() {
                    ghost.runner.step();
                    if audible {
                        ghost.ear.observe(ghost.runner.sim(), 0.35);
                    }
                }
            }
            stepped += 1;
        }
        // How far this frame sits between the last tick and the next. Always updated, including
        // on a frame that ran no tick — that is the frame the interpolation exists for.
        self.render_alpha = (self.accumulator / TICK).clamp(0.0, 1.0);
        // The animation is advanced by the *simulated* time, not by the ticks this frame happened to
        // run: the drawn body moves a third of a tick on a frame that ran none, and a run cycle that
        // only moved on the tick frames would take its steps at 60 Hz under a body gliding at the
        // display's rate — the legs skating under a smooth sprite, which reads as a jerk however
        // smooth the position is. The clock is the sim's own (`frames * TICK`) plus the fraction of
        // the next tick the frame is into, so it is exactly the time the drawn body is at, and it is
        // still the simulation's time rather than the wall clock's: a stalled frame that ran no ticks
        // and is still stuck in the same tick advances the animation by the same fraction, never
        // further than the body moved.
        let simulated = self.sim.stats().frames as f32 * TICK + self.render_alpha * TICK;
        let anim_dt = (simulated - self.anim_clock).max(0.0);
        self.anim_clock = simulated;
        if anim_dt > 0.0 {
            self.anim.advance(&self.sim, anim_dt);
            if let Some(ghost) = &mut self.ghost {
                ghost.anim.advance(ghost.runner.sim(), anim_dt);
            }
        }
        if stepped > 0 {
            // And the stopwatch stops on the tick the episode ended on, whatever frame carried it.
            self.note_win();
            // The death pause starts the moment the outcome turns, measured in wall
            // clock because the sim's own count has stopped with the episode.
            if self.sim.outcome().is_finished() && self.finished_wall.is_none() {
                self.finished_wall = Some(self.clock);
            }
        }
    }

    /// Advance by `dt` seconds of wall clock with one key state held for all of it.
    ///
    /// Nothing moves until the first input: with no key held the call is a no-op, so
    /// the clock starts when the player does rather than when the level loads. The
    /// splash's scripted runner goes through [`Session::advance_actions`] instead and
    /// is unaffected — it is a recording, not a race.
    pub fn advance(&mut self, dt: f32, keys: Keys) {
        if !self.started && keys.is_idle() {
            return;
        }
        self.started = true;
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
        // the fitness every agent is scored on (see `EpisodeStats::progress_pixels`). "flag" is how far
        // the body has ever been from it, which is the number that means something on a level
        // whose goal is above rather than to the right; `x` stays because it is what a
        // side-scroller watcher is used to reading.
        let progress = if stats.goal_distance_pixels > 0.0 {
            100.0 * stats.progress_pixels / stats.goal_distance_pixels
        } else {
            100.0
        };
        // The clock is the frame count, written the one way this project writes a time, and the
        // budget beside it is the same cap the AI is scored against — so the line says how much of a
        // level's allowance this attempt has spent, which the frame count alone left the reader to
        // work out from `FRAMES_PER_TILE`.
        let best = match self.best() {
            Some(frames) => format!("  your best {}", sim::format_time(frames)),
            None => String::new(),
        };
        // The ghost's state in two words: which record it is shadowing is the dock's
        // business (the race line names the AI's time), while this bar only says whether
        // the thing to look at is on screen. Nothing when there is nothing to show.
        let ghost = match &self.ghost {
            Some(g) if g.visible => "  ghost on (G hides)",
            Some(_) => "  ghost hidden (G shows)",
            None => "",
        };
        format!(
            "{}  {} of {}  x {:.1}  flag {:.1} tiles  progress {:.0}%  stomps {}  jumps {}{best}{ghost}  {}",
            self.sim.outcome().as_str(),
            sim::format_time(stats.frames),
            sim::format_time(self.sim.max_frames()),
            stats.best_x,
            stats.closest_approach,
            progress,
            stats.stomps,
            stats.jumps,
            if !self.started {
                "press a move key — the clock starts on input"
            } else if self.sim.outcome() == Outcome::Died {
                "restarting… (R now)"
            } else if stats.frames == 0 {
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
    /// The per-tick form exists for an evolved agent, whose decisions do change in a frame;
    /// the playtest's does not, and this is the test that says so. A frame long enough to
    /// carry two ticks is part of the fixture on purpose: it is the case where "decide once
    /// per frame" and "decide once per tick" differ, and the one the AI's cadence
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
    /// The mirror of the test above, and the property the AI depends on: the same
    /// wall-clock, delivered in different frame sizes, is the same episode — because the
    /// decisions are made against the sim's tick count and never against the frame's.
    #[test]
    fn per_tick_decisions_are_the_same_episode_at_any_frame_rate() {
        let level = shipped("l2-first-gap");
        // The AI's cadence, in miniature: a decision every fourth tick, held in
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
        // A held key throughout: the clock runs on input, and this test is about the
        // tick discipline once it is running rather than about the starting gate.
        let go = Keys {
            right: true,
            ..Keys::default()
        };
        let mut session = Session::new(&level, 1);
        // Half a second of play at 60 Hz is thirty ticks, whatever the frame rate.
        for _ in 0..30 {
            session.advance(1.0 / 60.0, go);
        }
        assert_eq!(session.sim().stats().frames, 30, "one tick per 1/60 s");

        // A frame that took 30 ms must not lose physics: the accumulator carries it.
        let mut session = Session::new(&level, 1);
        session.advance(0.03, go);
        assert_eq!(
            session.sim().stats().frames,
            1,
            "0.03 s is one tick, 0.018 left"
        );
        session.advance(0.03, go);
        assert_eq!(session.sim().stats().frames, 3, "the remainder was carried");

        // And a stall is clamped: a two-second frame must not teleport the player.
        let mut session = Session::new(&level, 1);
        session.advance(2.0, go);
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

    /// A playthrough of the opening level that ends at the flag, and the frames it took.
    fn a_won_playthrough() -> (Session, u32) {
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
        let frames = session.frames();
        (session, frames)
    }

    /// The stopwatch is the sim's own frame count, and it is the same clock the AI is timed on.
    #[test]
    fn the_stopwatch_is_the_sims_frame_count_and_not_the_frames_drawn() {
        let level = shipped("l1-flat-run");
        let mut session = Session::new(&level, 1);
        // A held key: the clock runs on input, and this test is about what it counts
        // once it is running. Frames of very different lengths, one of which carries
        // no tick at all: the clock has to follow the physics and not the redraws, or
        // a busy machine would win the race.
        let go = Keys {
            right: true,
            ..Keys::default()
        };
        let mut elapsed = 0.0f32;
        for dt in [1.0 / 60.0, 1.0 / 240.0, 0.03, 1.0 / 240.0, 1.0 / 60.0] {
            elapsed += dt;
            session.advance(dt, go);
        }
        assert_eq!(
            session.frames(),
            session.sim().stats().frames,
            "the clock is read from the sim, not counted by the window"
        );
        assert_eq!(
            session.frames(),
            (elapsed / TICK).floor() as u32,
            "{elapsed} s of play is the ticks it holds, and no more"
        );
        assert!(
            session.frames() > 0,
            "and it is a running clock, not a stopped one"
        );
        assert!(
            session.best().is_none(),
            "nothing has been won yet, so there is no time to remember"
        );
    }

    /// The race line: the time, where it got to, and how it stands against the AI's best — including
    /// when there is no AI win to race at all.
    #[test]
    fn the_race_line_says_the_time_and_how_it_stands_against_the_ai() {
        let level = shipped("l1-flat-run");
        let budget = sim::frame_budget(&level);

        // Nothing won yet, and no run to have won it: the clock, and the fact that there is nothing
        // to race. `0.00s` would be a claim about a race that does not exist.
        let mut running = Session::new(&level, 3);
        // A held key: an idle session is "ready", not "running", and that state is
        // the next test's business rather than this one's.
        running.advance(
            1.0,
            Keys {
                right: true,
                ..Keys::default()
            },
        );
        let line = running.race_line(None);
        assert!(
            line.starts_with(&sim::format_time(running.frames())) && line.contains("and running"),
            "{line}"
        );
        assert!(line.contains("no win to race yet"), "{line}");

        // A win, timed against three records: slower, faster, and the same to the frame.
        let (won, frames) = a_won_playthrough();
        let record = |frames: u32| FastWin {
            generation: 4,
            genome_id: 12,
            frames,
            budget_frames: budget,
        };
        let slower = won.race_line(Some(record(frames - 30)));
        assert!(slower.contains("at the flag"), "{slower}");
        assert!(slower.contains("0.50s slower"), "{slower}");
        let faster = won.race_line(Some(record(frames + 30)));
        assert!(faster.contains("0.50s faster"), "{faster}");
        let tied = won.race_line(Some(record(frames)));
        assert!(tied.contains("the same time, to the frame"), "{tied}");
        // And both times are written by the same formatter, so the number being raced is the number
        // the panel showed before the viewer came here.
        assert!(faster.contains(&sim::format_time(frames + 30)), "{faster}");
        assert!(faster.starts_with(&sim::format_time(frames)), "{faster}");

        // An episode that ended short of the flag says how it ended rather than inventing a time at
        // the flag — the outcome's own word, which is the same word the status bar uses.
        let gap_level = shipped("l2-first-gap");
        let mut died = Session::new(&gap_level, 1);
        for _ in 0..1200 {
            died.advance(
                1.0 / 60.0,
                Keys {
                    right: true,
                    ..Keys::default()
                },
            );
            if died.sim().outcome().is_finished() {
                break;
            }
        }
        assert_eq!(
            died.sim().outcome(),
            sim::Outcome::Died,
            "holding right into the first gap has to end in the pit: {}",
            died.summary()
        );
        let line = died.race_line(Some(record(died.frames() + 300)));
        assert!(line.contains("and died"), "{line}");
        assert!(
            line.contains("5.00s faster"),
            "the AI's time on the *other* level is still the number it is raced against: {line}"
        );
    }

    /// Being in front of the AI is one comparison of two clocks, and it reads the same way while the
    /// episode is running and once it has finished.
    #[test]
    fn being_in_front_of_the_record_is_the_clock_against_the_clock() {
        let level = shipped("l1-flat-run");
        let record = |frames: u32| FastWin {
            generation: 4,
            genome_id: 12,
            frames,
            budget_frames: sim::frame_budget(&level),
        };

        // Running, level with the record's own time: still in front, because the flag is one tick
        // away and the AI's clock has already run out.
        let mut running = Session::new(&level, 1);
        // A held key: the clock runs on input, and the comparison below is about the
        // clock once it is running.
        let go = Keys {
            right: true,
            ..Keys::default()
        };
        for _ in 0..30 {
            running.advance(1.0 / 60.0, go);
        }
        assert_eq!(running.frames(), 30);
        assert!(running.beating(record(30)));
        assert!(running.beating(record(31)));
        assert!(!running.beating(record(29)), "the clock has passed it");

        // And the same rule at the flag: a win at or under the AI's time beats it, and the episode
        // being over does not change what the comparison means.
        let (won, frames) = a_won_playthrough();
        assert!(won.beating(record(frames)));
        assert!(won.beating(record(frames + 1)));
        assert!(!won.beating(record(frames - 1)));
    }

    /// The playthrough's own record: a win is remembered, a faster win replaces it, and a restart
    /// keeps it — a retry is a second attempt at the same level, not a new person at the machine.
    #[test]
    fn a_session_remembers_its_quickest_win_and_keeps_it_across_a_restart() {
        let level = shipped("l1-flat-run");
        let (mut won, frames) = a_won_playthrough();
        assert_eq!(won.best(), Some(frames), "the win is the session's record");
        assert!(
            won.summary()
                .contains(&format!("your best {}", sim::format_time(frames))),
            "{}",
            won.summary()
        );

        // Idempotent: the frames after the win are not a second, slower record.
        won.advance(1.0 / 60.0, Keys::default());
        won.advance(1.0 / 60.0, Keys::default());
        assert_eq!(won.best(), Some(frames));

        // A restart rewinds the clock to zero and leaves the record standing.
        won.restart(&level);
        assert_eq!(won.frames(), 0);
        assert_eq!(won.best(), Some(frames), "a retry is not a new session");

        // And the record is a *minimum*, not a last: the same win, with a quicker time already on
        // the books, does not replace it. The same seed and the same keys are the same episode, so
        // the frames are the frames either way.
        let mut again = Session::new(&level, 1);
        again.best = Some(frames - 1);
        for _ in 0..1200 {
            again.advance(
                1.0 / 60.0,
                Keys {
                    right: true,
                    ..Keys::default()
                },
            );
            if again.sim().outcome().is_finished() {
                break;
            }
        }
        assert_eq!(again.sim().outcome(), sim::Outcome::Won);
        assert_eq!(again.frames(), frames, "the same episode, watched twice");
        assert_eq!(again.best(), Some(frames - 1), "the quicker win stands");
    }

    /// The clock is a race timer: it waits for the first input rather than billing
    /// the viewer for loading the level.
    #[test]
    fn the_clock_waits_for_the_first_input() {
        let level = shipped("l1-flat-run");
        let mut session = Session::new(&level, 1);
        // Two seconds of wall clock with no key held: nothing steps, not even the
        // walkers — the first tick is the first thing the player asked for.
        for _ in 0..120 {
            session.advance(1.0 / 60.0, Keys::default());
        }
        assert_eq!(session.frames(), 0, "no input, no clock");
        assert!(
            session.summary().contains("the clock starts on input"),
            "{}",
            session.summary()
        );
        assert!(
            session
                .race_line(None)
                .contains("the clock starts on your first input"),
            "{}",
            session.race_line(None)
        );
        // And the race is not being won by a standing start: no lamp, no lead, no gap.
        let record = FastWin {
            generation: 1,
            genome_id: 2,
            frames: 505,
            budget_frames: sim::frame_budget(&level),
        };
        assert!(
            !session.beating(record),
            "a clock that has not started is not in front of anything"
        );
        let line = session.race_line(Some(record));
        assert!(line.contains("you have not run yet"), "{line}");
        assert!(
            !line.contains("faster") && !line.contains("slower"),
            "nothing has been run against the AI yet, so no gap can be quoted: {line}"
        );
        // And the first held key starts it: one frame of input is ticks on the board.
        session.advance(
            1.0 / 60.0,
            Keys {
                right: true,
                ..Keys::default()
            },
        );
        assert!(session.frames() > 0, "the first input starts the clock");
    }

    /// A death restarts itself after a second, and the pause is on the wall clock:
    /// the sim's own count has stopped with the episode.
    #[test]
    fn a_death_restarts_itself_after_a_second() {
        let level = shipped("l2-first-gap");
        let mut session = Session::new(&level, 1);
        let go = Keys {
            right: true,
            ..Keys::default()
        };
        for _ in 0..1200 {
            session.advance(1.0 / 60.0, go);
            if session.sim().outcome().is_finished() {
                break;
            }
        }
        assert_eq!(session.sim().outcome(), sim::Outcome::Died);
        assert!(
            !session.restart_due(),
            "the pause starts at the death, not before it"
        );
        // Half a second of sitting with the corpse: still dead, still waiting.
        for _ in 0..30 {
            session.advance(1.0 / 60.0, Keys::default());
        }
        assert_eq!(session.sim().outcome(), sim::Outcome::Died);
        assert!(!session.restart_due());
        assert!(
            session.summary().contains("restarting"),
            "{}",
            session.summary()
        );
        // Past the second: due. The window restarts on this signal.
        for _ in 0..40 {
            session.advance(1.0 / 60.0, Keys::default());
        }
        assert!(session.restart_due(), "a dead second is a retry");
        session.restart(&level);
        assert_eq!(session.sim().outcome(), sim::Outcome::Running);
        assert_eq!(session.frames(), 0);
        assert!(
            session.summary().contains("the clock starts on input"),
            "the retry waits for input rather than billing the death pause: {}",
            session.summary()
        );
    }

    /// **The ghost finishes on exactly the time the race is quoted against.**
    ///
    /// This is the timing bug the ghost was introduced with, and the test is written against the
    /// shape of it rather than the symptom. The window used to attach the run's *best-ever*
    /// genome while quoting the *fastest win* — two different agents the moment a run has two
    /// winners, and often not a winner at all — so the ghost on screen could die, or reach the
    /// flag seconds after the number beside it. The packet now carries the episode behind the
    /// time (`Packet::fastest_agent`), and this test drives the whole path a player drives:
    /// train until something wins, build the ghost from the packet's record, run it on the
    /// player's own ticks, and check the flag is touched on the recorded frame.
    #[test]
    fn the_ghost_finishes_on_the_time_the_run_quotes() {
        let level = shipped("l1-flat-run");
        let cfg = evo::GaConfig {
            pop_size: 24,
            ..evo::GaConfig::default()
        };
        let mut generator = crate::lab::Generator::new(
            level.clone(),
            cfg.clone(),
            crate::lab::DEFAULT_BEST_LANES,
            crate::lab::DEFAULT_RANDOM_LANES,
            crate::lab::RunPlan::default(),
            std::env::temp_dir().join("evo-play-ghost-record"),
            "l1-flat-run",
        )
        .expect("the generator starts");
        // A random population wins this level, so one generation is normally enough; the cap is
        // there so a change to the fixture fails as a clear assertion rather than a hang.
        let packet = (0..12)
            .map(|_| generator.next_packet())
            .find(|p| p.fastest.is_some())
            .expect("l1-flat-run is won by a random population");
        let record = packet.fastest.expect("just found");
        let agent = packet
            .fastest_agent
            .as_ref()
            .expect("the episode behind the time is on the packet");
        let mut session = Session::new(&level, 1);
        session.attach_ghost(GhostSpec {
            genome: agent.genome.clone(),
            eval: cfg.eval_config(),
            seed: agent.result.seed,
        });

        // Held input, so both clocks run: the ghost steps on the player's ticks.
        let go = Keys {
            right: true,
            ..Keys::default()
        };
        for _ in 0..record.frames + 120 {
            session.advance(1.0 / 60.0, go);
            if session
                .ghost_sim()
                .is_some_and(|s| s.outcome().is_finished())
            {
                break;
            }
        }
        let ghost = session.ghost_sim().expect("the ghost is shown");
        assert_eq!(
            ghost.outcome(),
            sim::Outcome::Won,
            "the ghost of the record must reach the flag"
        );
        assert_eq!(
            ghost.stats().frames,
            record.frames,
            "the ghost finished on {} frames while the dock quotes {} — a race against two \
             different records",
            ghost.stats().frames,
            record.frames
        );
    }

    /// The ghost runs on the player's ticks — one AI tick per player tick —
    /// and restarts with the player, so the race restarts together.
    #[test]
    fn the_ghost_runs_on_the_players_ticks_and_restarts_with_them() {
        let level = shipped("l1-flat-run");
        let packet = crate::lab::scored_packet(&level, "play-ghost");
        let best = packet.best.as_ref().expect("the fixture scores a best");
        let mut session = Session::new(&level, 1);
        assert!(!session.has_ghost(), "no record attached, no ghost");
        assert!(session.toggle_ghost().is_none(), "nothing to toggle");
        session.attach_ghost(GhostSpec {
            genome: best.genome.clone(),
            eval: EvalConfig::default(),
            seed: best.result.seed,
        });
        assert!(session.has_ghost());
        // Idle first: neither clock moves before the player's input, ghost included.
        session.advance(1.0, Keys::default());
        assert_eq!(session.frames(), 0);
        assert_eq!(session.ghost_sim().map(|s| s.stats().frames), Some(0));
        // Then thirty ticks of held input: the two episodes march together.
        let go = Keys {
            right: true,
            ..Keys::default()
        };
        for _ in 0..30 {
            session.advance(1.0 / 60.0, go);
        }
        assert_eq!(session.frames(), 30);
        assert_eq!(
            session.ghost_sim().map(|s| s.stats().frames),
            Some(30),
            "one AI tick per player tick"
        );
        assert!(session.ghost_anim().is_some());
        assert!(
            session.summary().contains("ghost on"),
            "{}",
            session.summary()
        );
        // Hidden: the episode still runs, only the drawing stops.
        assert_eq!(session.toggle_ghost(), Some(false));
        assert!(session.ghost_sim().is_none());
        assert_eq!(session.toggle_ghost(), Some(true));
        assert!(session.ghost_sim().is_some());
        // And a restart rewinds both to the spawn.
        session.restart(&level);
        assert_eq!(session.frames(), 0);
        assert_eq!(session.ghost_sim().map(|s| s.stats().frames), Some(0));
        assert!(session.has_ghost(), "the race restarts together");
    }

    /// **The playtest draws a body that moves on every frame, by the same amount.**
    ///
    /// The bug this pins is the one a 60 Hz simulation on a 144 Hz screen always has: two frames in
    /// three run no tick at all, so a view that draws the simulator's last position shows a body that
    /// stands still, stands still, then jumps a whole tick's travel. That is a 60 Hz stutter under a
    /// screen drawing three times as often, and it is what "jerky" looks like. Interpolating between
    /// the two ticks a frame is between makes the step per frame the accumulator's own share of a
    /// tick — equal on every frame, including the ones no tick landed on.
    ///
    /// Measured after the body has reached run speed, so the step is constant in the physics too and
    /// any variance is the view's.
    #[test]
    fn the_drawn_body_advances_on_every_frame_of_a_fast_display() {
        let level = shipped("l1-flat-run");
        let mut session = Session::new(&level, 1);
        let keys = Keys {
            right: true,
            ..Keys::default()
        };
        // Forty ticks at the simulation's own rate: long enough to be at the speed cap.
        for _ in 0..40 {
            session.advance(1.0 / 60.0, keys);
        }
        // Then a 144 Hz display, for three quarters of a second of running.
        let mut steps = Vec::new();
        let mut ticks = 0;
        let mut last = session.body_at().0;
        for _ in 0..120 {
            let before = session.frames();
            session.advance(1.0 / 144.0, keys);
            ticks += session.frames() - before;
            let x = session.body_at().0;
            steps.push(x - last);
            last = x;
        }
        assert!(
            ticks > 40 && ticks < 55,
            "120 frames at 144 Hz is about fifty ticks, got {ticks}"
        );
        let still = steps.iter().filter(|s| **s <= 0.0).count();
        assert_eq!(
            still,
            0,
            "{} of {} frames drew the body standing still; the drawn position is the tick's, not \
             a point between two ticks",
            still,
            steps.len()
        );
        let mean = steps.iter().sum::<f32>() / steps.len() as f32;
        let slowest = steps.iter().cloned().fold(f32::INFINITY, f32::min);
        let fastest = steps.iter().cloned().fold(0.0f32, f32::max);
        assert!(
            (slowest - mean).abs() <= mean * 0.25 && (fastest - mean).abs() <= mean * 0.25,
            "the step per frame should be the same on every frame: mean {mean:.4}, \
             min {slowest:.4}, max {fastest:.4}"
        );
        // And the camera is on the drawn body, so what actually moves on screen is the world: the
        // body's centre is the camera centre to within the f32 the lerp is computed in.
        let (cx, _) = session.camera_centre();
        assert!(
            (cx - (session.body_at().0 + session.sim().player().w * 0.5)).abs() < 1e-5,
            "the camera has to follow the body where it is drawn, not where the last tick left it"
        );
    }

    /// **The run cycle advances with the drawn body, not with the frames that carry a tick.**
    ///
    /// The second half of the same jerkiness: on a 144 Hz screen two frames in three run no tick, and
    /// an animator advanced by `ticks * TICK` takes its leg steps on the tick frames only — legs
    /// stepping at 60 Hz under a body gliding at 144, which reads as a skating sprite however smooth
    /// the position is. The animation clock is the *simulated* time the frame is at, fractional part
    /// included, so the cycle moves on the frames the body moves on.
    ///
    /// Counted through the drawn sprite rather than through a timer, because the sprite is what a
    /// player sees: the leg frame has to change on frames that ran no tick at all.
    #[test]
    fn the_run_cycle_advances_on_frames_that_run_no_tick() {
        let level = shipped("l1-flat-run");
        let mut session = Session::new(&level, 1);
        let keys = Keys {
            right: true,
            ..Keys::default()
        };
        for _ in 0..40 {
            session.advance(1.0 / 60.0, keys);
        }
        let mut last_sprite = crate::view::player_sprite(session.sim(), session.anim()).0;
        let mut changes_without_a_tick = 0;
        let mut changes = 0;
        for _ in 0..120 {
            let before = session.frames();
            session.advance(1.0 / 144.0, keys);
            let ran_tick = session.frames() != before;
            let sprite = crate::view::player_sprite(session.sim(), session.anim()).0;
            if sprite != last_sprite {
                changes += 1;
                if !ran_tick {
                    changes_without_a_tick += 1;
                }
                last_sprite = sprite;
            }
        }
        assert!(
            changes >= 6,
            "a 0.40 s run cycle over 0.83 s of running should step several times, got {changes}"
        );
        assert!(
            changes_without_a_tick > 0,
            "the leg frame changed only on frames that ran a tick, so the animation is running on the \
             simulation's 60 Hz rather than the display's clock"
        );
    }
}
