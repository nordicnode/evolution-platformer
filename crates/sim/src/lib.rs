//! `sim` — the deterministic core (development_plan.md §4, §5, §6).
//!
//! Hard boundary rules, enforced by review rather than by the type system:
//!
//! 1. No `std::time`, no threads, no `println!` inside [`Sim::step`], no `HashMap`
//!    in hot paths (SipHash has a per-process random seed → nondeterminism).
//! 2. Everything the sim needs crosses the boundary as plain data:
//!    `&Level`, `AgentAction`, `seed: u64`.
//! 3. Rendering only ever borrows `&Sim`.
//!
//! Version trio (§8): [`SIM_VERSION`], [`sensors::VERSION`], and
//! `evo::fitness::VERSION`. All three are stamped into every champion file.

pub mod hash;
pub mod level;
pub mod physics;
pub mod reach;
pub mod rng;
pub mod sensors;

pub use crate::hash::StateHasher;
pub use crate::level::{Level, Rect, Scenery, Tile};
pub use crate::physics::{AgentAction, Physics, PlayerState, Walker, PHYSICS};
pub use crate::rng::{episode_seed, rng_from_seed, Rng};

/// Sim behaviour version. Bump on *any* change to physics, tile semantics, or
/// episode termination — champions carrying an older value must be refused.
pub const SIM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Fixed timestep (§5.1). Headless training steps in a plain loop; only visible
/// realtime play needs an accumulator.
pub const DT: f32 = 1.0 / 60.0;

/// One tile is one world unit. Keeps the physics bible readable.
pub const TILE: f32 = 1.0;

/// Physics ticks between network queries: the **granularity of every decision the agent
/// makes**.
///
/// The sim owns this number rather than the trainer, because it is a physical fact about the
/// agent rather than a training knob. One action is held for this many ticks, so a block is
/// the atom of the agent's control — the shortest thing it can ask for, and the unit in which
/// it asks for everything longer. That is why [`physics::Physics::min_jump_hold`] is exactly
/// one of these: a jump held for a single block is the shortest jump the sim has, and each
/// further block of hold adds height, so jump *length* is a ladder with a rung per decision
/// spent. `evo` re-exports the value as its `--action-repeat` default, and
/// [`sensors::VERSION`] covers a change to it (§8).
pub const DECISION_TICKS: u32 = 4;

/// Fitness is scored per "pixel" of new rightmost ground covered (§7.4.1).
pub const PIXELS_PER_TILE: f32 = 16.0;

/// Episodes are declared stuck after this many frames with no new rightmost x
/// (§7.4.5). Without it, campers burn the whole compute budget standing still.
pub const NO_PROGRESS_FRAMES: u32 = 600;

/// Episode cap = `level.width * FRAMES_PER_TILE` (§7.4.6).
pub const FRAMES_PER_TILE: u32 = 40;

/// The episode cap for `level`: `max(width, height) × FRAMES_PER_TILE` (§7.4.6).
///
/// The *longer* side, not the width. A budget proportional to width alone quietly assumed
/// every level is a side-scroller: a 10x22 tower got 400 frames to climb 18 rows, which is
/// less than the 600-frame anti-camper timer and about seven jumps' worth of airtime. Every
/// shipped level is wider than it is tall, so this leaves all of them exactly as they were.
///
/// One function rather than the expression written twice, because two consumers now need the
/// same number: the sim stops an episode at it, and the scoring measures how much of it a
/// winning episode left unspent ([`EpisodeStats::budget_frames`]). A cap the two disagreed
/// about would grade a fast win as a slow one, and the disagreement would be invisible in
/// both.
pub fn frame_budget(level: &Level) -> u32 {
    level.width.max(level.height).max(1) as u32 * FRAMES_PER_TILE
}

/// A count of frames written as a **time**, the one way this project writes one.
///
/// The unit is the frame, because that is the unit everything else counts in: [`DT`] of a
/// second each, so `frames` is the whole of what a clock is and no caller has to know about a
/// wall-clock. Both front-ends and the champion metadata go through here rather than through
/// their own arithmetic, for the reason [`frame_budget`] is one function: a window that said
/// `8.4s` beside a terminal that said `504 frames` for the same episode would be two answers to
/// one question, and the number a viewer races has to be the number the AI was scored on.
///
/// Two shapes, deliberately. Under a minute it is hundredths of a second — `0.98s`, `8.42s` —
/// which is the resolution a race needs, since one frame is 0.017 s and two times that differ by
/// a frame have to be tellable apart. At a minute and up it is `m:ss.hh`, because `97.35s` is a
/// number a reader has to convert; no shipped level is anywhere near it (a full-width level's
/// budget is 40 s), so the second shape exists for the level someone draws tomorrow rather than
/// for one of ours.
pub fn format_time(frames: u32) -> String {
    let seconds = frames as f64 * DT as f64;
    if seconds < 60.0 {
        format!("{seconds:.2}s")
    } else {
        let minutes = (seconds / 60.0).floor();
        format!("{}:{:05.2}", minutes as u64, seconds - minutes * 60.0)
    }
}

/// How an episode finished. `Running` means "not finished yet".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Running,
    /// Touched the goal flag.
    Won,
    /// Spike contact, walker side-hit, or fell into a pit.
    Died,
    /// Hit the frame cap set by [`FRAMES_PER_TILE`].
    TimedOut,
    /// Hit [`NO_PROGRESS_FRAMES`] without advancing.
    Stuck,
}

impl Outcome {
    pub fn is_finished(self) -> bool {
        self != Outcome::Running
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Running => "running",
            Outcome::Won => "won",
            Outcome::Died => "died",
            Outcome::TimedOut => "timed-out",
            Outcome::Stuck => "stuck",
        }
    }
}

/// Raw, unweighted episode measurements. Scoring lives in `evo::fitness` so the
/// fitness function can be versioned independently of the sim.
#[derive(Clone, Debug, PartialEq)]
pub struct EpisodeStats {
    /// Frames in which the episode was actually simulated.
    pub frames: u32,
    /// The frame cap this episode ran under ([`frame_budget`] for the level it was played on).
    ///
    /// Constant for the episode, and carried here rather than re-derived by whoever reads the
    /// stats because it is the *denominator* of the one reading that needs it: how much of the
    /// budget the body left unspent, which is what finishing quickly means in a game whose
    /// clock is its tick counter (`evo::fitness::SPEED_BONUS`). A consumer that recomputed the
    /// cap itself could disagree with the sim about when an episode ends, and a fast win would
    /// score as a slow one.
    pub budget_frames: u32,
    /// How much closer to the flag the body has ever been than it started, in pixels
    /// (§7.4.1).
    ///
    /// **Direction-agnostic since 0.5.0**, and that was a bug fix rather than a tidy-up.
    /// This used to be "new rightmost x", which quietly defined the game as a left-to-right
    /// side-scroller: on a level whose goal is *up* — a tower, a shaft, a climb — walking
    /// along the bottom scored better than climbing, and the episode was declared stuck
    /// while the body was on its way up. Distance-to-the-flag works in every direction,
    /// including diagonally, and on a rightward level it is the same signal it always was.
    pub progress_pixels: f32,
    /// The smallest distance to the flag the body has ever reached, in tiles. The running
    /// minimum that makes [`EpisodeStats::progress_pixels`] monotone.
    pub closest_approach: f32,
    /// Distance from the spawn to the flag, in pixels: the denominator that turns
    /// [`EpisodeStats::progress_pixels`] into a fraction of the level.
    ///
    /// Infinite for a level with no flag at all — the state a document is in between the
    /// editor's eraser and the next placement. Nothing divides by it without checking, so
    /// the arithmetic stays finite; the readouts that turn it into a percentage simply show
    /// no progress, which is what a level with nothing to reach is worth.
    pub goal_distance_pixels: f32,
    /// Rightmost x ever reached, in world units.
    ///
    /// Reported, not scored, since 0.5.0: it is what a watcher reads to see how far along a
    /// side-scroller the body got, but it is no longer the objective — `progress_pixels` is.
    pub best_x: f32,
    /// Consecutive frames with no new rightmost x.
    pub no_progress_frames: u32,
    /// Walkers stomped.
    pub stomps: u32,
    /// Jumps taken: ground jumps, buffered ones and wall jumps alike.
    ///
    /// Reported because it is the statistic that makes "the agent jumps endlessly"
    /// visible rather than a feeling, and scored (lightly, negatively) because a hop that
    /// buys no progress should not be free. See `evo::fitness::JUMP_PENALTY`.
    pub jumps: u32,
    /// Distinct safe landing zones the body has **stood on** that it could only have
    /// reached by crossing a hazard or a hole (scored in fitness v4, see
    /// `evo::fitness::CHECKPOINT_BONUS`).
    ///
    /// The companion to `jumps`: `jumps` is whether the body is trying, this is what
    /// trying has bought. Counted per *zone* ([`Level::landing_zone`]) rather than per
    /// tile, and only while the body is on the ground — so it cannot be collected by
    /// sailing over a hazard and dying beyond it, and a single zone pays once however many
    /// times the body walks back onto it.
    pub checkpoints: u32,
    /// Route crossings closed from the spawn stand toward the flag, measured only when
    /// grounded on a route stand (`spawn_hops - best_hops_reached`, in `0..=spawn_hops`).
    ///
    /// **Monotone along zig-zag routes where straight-line Euclidean distance reverses.**
    /// Every canonical crossing onto a stand fewer hops from the flag increments this
    /// count, giving evolution a true geodesic gradient up winding towers.
    pub route_hops_closed: u32,
    /// Best (deepest along the route) stand index the body has **stood on** during the
    /// episode, or `None` if it has not yet stood on a route stand.
    pub best_stand: Option<usize>,
    /// Where the agent died, if it did.
    pub death_pos: Option<(f32, f32)>,
}

impl EpisodeStats {
    fn new(player: &PlayerState, level: &Level) -> Self {
        let approach = goal_approach(player, level);
        let spawn_stand = spawn_route_stand(level);
        EpisodeStats {
            frames: 0,
            budget_frames: frame_budget(level),
            progress_pixels: 0.0,
            closest_approach: approach,
            goal_distance_pixels: approach * PIXELS_PER_TILE,
            best_x: player.x,
            no_progress_frames: 0,
            stomps: 0,
            jumps: 0,
            checkpoints: 0,
            route_hops_closed: 0,
            best_stand: spawn_stand,
            death_pos: None,
        }
    }
}

/// The route stand index of the level's spawn point, when the spawn rests on a standable zone.
fn spawn_route_stand(level: &Level) -> Option<usize> {
    let surface = Level::tile_y(level.spawn.y + level.spawn.h + 0.05);
    let col = Level::tile_x(level.spawn.x + level.spawn.w * 0.5);
    level
        .routes()
        .zone_at(col, surface)
        .or_else(|| level.routes().zone_at(col - 1, surface))
        .or_else(|| level.routes().zone_at(col + 1, surface))
}

/// Distance from a body to the level's goal, in tiles: zero when they overlap.
///
/// An AABB-to-AABB distance rather than a centre-to-centre one, so a wide flag on a ledge
/// does not read as "further away" than a narrow one at the same spot. A level has one flag
/// (`Level::goals`), and a level with *none* — a document mid-edit, not something a file can
/// describe — has no distance to close: the fold's `INFINITY` identity comes back, and
/// [`EpisodeStats::goal_distance_pixels`] documents what the readers do with it.
fn goal_approach(player: &PlayerState, level: &Level) -> f32 {
    let a = player.aabb();
    level
        .goals
        .iter()
        .map(|g| {
            let dx = (g.x - a.right()).max(a.x - g.right()).max(0.0);
            let dy = (g.y - a.bottom()).max(a.y - g.bottom()).max(0.0);
            (dx * dx + dy * dy).sqrt()
        })
        .fold(f32::INFINITY, f32::min)
}

/// One deterministically simulated episode.
///
/// Construct with [`Sim::new`], then drive with [`Sim::step`]. The full state
/// hashes to a `u64` via [`Sim::state_hash`]; the determinism test pins that.
#[derive(Clone, Debug)]
pub struct Sim {
    level: Level,
    player: PlayerState,
    walkers: Vec<Walker>,
    stats: EpisodeStats,
    outcome: Outcome,
    rng: Rng,
    /// Checkpoint zones already credited, as `(surface_row, zone_start)`. A linear scan of
    /// a list whose length is the level's hazard count, so a Vec is the right shape here —
    /// the hot path is the `last_stand` guard below, not this.
    credited: Vec<(i32, i32)>,
    /// The standing tile the last credit check looked at. A body resting on one tile is the
    /// common case by far, and this is what keeps it from re-deriving its landing zone
    /// (a scan back to the start of the run) on every one of those frames.
    last_stand: (i32, i32),
    /// The route stand index the body is currently standing on (or last stood on), used by
    /// the waypoint sensor (`sensors::observe_from_stand`) and geodesic progress tracking.
    current_route_stand: Option<usize>,
    /// Route hops from the spawn stand to the flag, if the spawn is on a route to the flag.
    spawn_hops: Option<u32>,
    /// Minimum `hops_to_flag` reached on a grounded stand so far in this episode.
    min_hops_reached: Option<u32>,
}

impl Sim {
    /// Start an episode. Same `level` + `seed` ⇒ identical trajectory.
    ///
    /// The episode RNG is consumed here, once per walker, to draw that walker's
    /// patrol phase ([`Walker::spawn_spread`]). Drawing at construction rather
    /// than during `step` keeps the RNG stream position a pure function of the
    /// walker count, so the state hash stays comparable across frames.
    pub fn new(level: &Level, seed: u64) -> Self {
        let mut rng = rng_from_seed(seed);
        let walkers = level
            .walkers
            .iter()
            .enumerate()
            .map(|(i, r)| Walker::spawn_spread(i as u32, *r, level, &PHYSICS, &mut rng))
            .collect();
        let spawn_player = level.spawn_player();
        let current_route_stand = spawn_route_stand(level);
        let spawn_hops = current_route_stand.and_then(|s| level.routes().hops_to_flag(s));
        Sim {
            player: spawn_player,
            walkers,
            stats: EpisodeStats::new(&spawn_player, level),
            level: level.clone(),
            outcome: Outcome::Running,
            rng,
            credited: Vec::new(),
            // No tile is ever at a negative row, so this cannot match a real standing tile
            // and the first grounded frame always takes the slow path once.
            last_stand: (i32::MIN, i32::MIN),
            current_route_stand,
            spawn_hops,
            min_hops_reached: spawn_hops,
        }
    }

    pub fn level(&self) -> &Level {
        &self.level
    }

    pub fn player(&self) -> &PlayerState {
        &self.player
    }

    pub fn walkers(&self) -> &[Walker] {
        &self.walkers
    }

    pub fn stats(&self) -> &EpisodeStats {
        &self.stats
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// Advance exactly one physics tick (1/[`DT`] of a second).
    pub fn step(&mut self, action: AgentAction) {
        if self.outcome.is_finished() {
            return;
        }
        let ph = &PHYSICS;
        self.stats.frames += 1;

        // Walker patrols advance whatever the player does: the level is alive.
        for w in self.walkers.iter_mut() {
            w.tick(&self.level, ph);
        }

        let prev_bottom = self.player.y + self.player.h;
        let touched = self.player.tick(&self.level, ph, action);
        if touched.jumped {
            self.stats.jumps += 1;
        }

        // ---- hazards ------------------------------------------------------
        if touched.touched_spike {
            self.die();
            return;
        }

        // ---- walkers: stomp vs side-hit ----------------------------------
        let mut stomped = false;
        for w in self.walkers.iter_mut() {
            if !w.alive || !self.player.aabb().overlaps(&w.aabb(ph)) {
                continue;
            }
            let descending = self.player.vy > 0.0;
            let above = prev_bottom <= w.y + ph.enemy_stomp_tol;
            if descending && above {
                w.alive = false;
                self.player.vy = -ph.jump_v0 * ph.stomp_bounce;
                self.player.grounded = false;
                stomped = true;
            } else {
                self.die();
                return;
            }
        }
        if stomped {
            self.stats.stomps += 1;
        }

        // ---- goal --------------------------------------------------------
        if self
            .level
            .goals
            .iter()
            .any(|g| self.player.aabb().overlaps(g))
        {
            self.outcome = Outcome::Won;
            self.finish();
            return;
        }

        // ---- pit ---------------------------------------------------------
        if self.player.y > self.level.height as f32 {
            self.die();
            return;
        }

        // ---- checkpoints & geodesic route progress -----------------------
        // Credit the safe landing zones the body has walked onto. Grounded-only on
        // purpose: the term exists to pay for *arriving* somewhere, so a body that sails
        // over a hazard and dies in the air beyond it keeps the frontier it covered and
        // earns nothing here. See `Level::checkpoint_zone_from`.
        let mut route_advanced = false;
        if self.player.grounded {
            let surface = Level::tile_y(self.player.y + self.player.h + 0.05);
            let col = Level::tile_x(self.player.x + self.player.w * 0.5);
            if (surface, col) != self.last_stand {
                let from_col = if self.last_stand.0 == i32::MIN {
                    Level::tile_x(self.level.spawn.x + self.level.spawn.w * 0.5)
                } else {
                    self.last_stand.1
                };
                self.last_stand = (surface, col);
                if let Some(zone) = self.level.checkpoint_zone_from(col, surface, from_col) {
                    let id = (surface, zone);
                    if !self.credited.contains(&id) {
                        self.credited.push(id);
                        self.stats.checkpoints = self.credited.len() as u32;
                    }
                }
                let routes = self.level.routes();
                let stand = routes
                    .zone_at(col, surface)
                    .or_else(|| routes.zone_at(col - 1, surface))
                    .or_else(|| routes.zone_at(col + 1, surface));
                if let Some(s) = stand {
                    self.current_route_stand = Some(s);
                    if let Some(hops) = routes.hops_to_flag(s) {
                        if self.min_hops_reached.is_none_or(|m| hops < m) {
                            self.min_hops_reached = Some(hops);
                            self.stats.best_stand = Some(s);
                            if let Some(sh) = self.spawn_hops {
                                let closed = sh.saturating_sub(hops);
                                if closed > self.stats.route_hops_closed {
                                    self.stats.route_hops_closed = closed;
                                    route_advanced = true;
                                }
                            }
                        } else if self.stats.best_stand.is_none() {
                            self.stats.best_stand = Some(s);
                        }
                    } else if self.stats.best_stand.is_none() {
                        self.stats.best_stand = Some(s);
                    }
                }
            }
        }

        // ---- progress / termination --------------------------------------
        // Progress is *how much closer to the flag than the spawn the body has ever been*,
        // so a level never starts "in credit" and the objective has no preferred direction: a
        // tower is climbed, a corridor is run, and neither is a special case (see
        // `EpisodeStats::progress_pixels` for what this replaced and why). Recomputing the
        // total from the running minimum rather than accumulating a delta keeps it exactly
        // monotone in floating point, so a route that leaves the flag and comes back cannot
        // lose ground already banked.
        let approach = goal_approach(&self.player, &self.level);
        if approach < self.stats.closest_approach {
            self.stats.closest_approach = approach;
            self.stats.progress_pixels =
                self.stats.goal_distance_pixels - approach * PIXELS_PER_TILE;
            self.stats.no_progress_frames = 0;
        } else if route_advanced {
            self.stats.no_progress_frames = 0;
        } else {
            self.stats.no_progress_frames += 1;
        }
        if self.player.x > self.stats.best_x {
            self.stats.best_x = self.player.x;
        }

        if self.stats.no_progress_frames >= NO_PROGRESS_FRAMES {
            self.outcome = Outcome::Stuck;
            self.finish();
        } else if self.stats.frames >= self.max_frames() {
            self.outcome = Outcome::TimedOut;
            self.finish();
        }
    }

    /// Frame cap for this level (§7.4.6), i.e. [`frame_budget`] of its terrain.
    pub fn max_frames(&self) -> u32 {
        frame_budget(&self.level)
    }

    fn die(&mut self) {
        self.outcome = Outcome::Died;
        self.stats.death_pos = Some((self.player.x, self.player.y));
        self.finish();
    }

    fn finish(&mut self) {
        debug_assert!(self.outcome.is_finished());
    }

    /// How close the body is to the flag *right now*, in tiles ([`goal_approach`]).
    ///
    /// The current reading, as opposed to [`EpisodeStats::closest_approach`] — the running
    /// minimum over the whole episode, which includes positions the body only passed through
    /// and never kept (the apex of a leap it then fell back from). Ask this one when the
    /// question is "where has the body actually *stood*": `evo::curriculum::stuck_point`
    /// tracks it over grounded frames to find the spot a stalled run should practice from.
    pub fn goal_approach(&self) -> f32 {
        goal_approach(&self.player, &self.level)
    }

    /// Observation vector for the agent policy (§7.2). See [`sensors`].
    pub fn observation(&self) -> [f32; sensors::N_INPUTS] {
        sensors::observe_from_stand(
            &self.level,
            &self.player,
            &self.walkers,
            &PHYSICS,
            self.current_route_stand,
        )
    }

    /// Canonical hash of the complete simulated state (§5.2, §5.3).
    ///
    /// Floats are hashed by their IEEE-754 bit pattern after canonicalizing NaN.
    /// Rust does not contract `f32` arithmetic into FMA without fast-math, and
    /// the sim uses no libm transcendentals, so this is stable across native
    /// builds on the same architecture.
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.u32(self.stats.frames);
        h.f32(self.stats.progress_pixels);
        h.f32(self.stats.closest_approach);
        h.f32(self.stats.best_x);
        h.u32(self.stats.no_progress_frames);
        h.u32(self.stats.stomps);
        h.u32(self.stats.jumps);
        h.u32(self.stats.checkpoints);
        h.u32(self.stats.route_hops_closed);
        match self.stats.best_stand {
            Some(s) => {
                h.u8(1);
                h.u32(s as u32);
            }
            None => h.u8(0),
        }
        // And the zone ids themselves, so a change to *which* zones qualify moves this hash
        // even when an episode happens to credit the same number of them.
        h.u32(self.credited.len() as u32);
        for (row, zone) in &self.credited {
            h.u32(*row as u32);
            h.u32(*zone as u32);
        }
        match self.stats.death_pos {
            Some((x, y)) => {
                h.u8(1);
                h.f32(x);
                h.f32(y);
            }
            None => h.u8(0),
        }
        h.u8(self.outcome as u8);
        self.player.hash_into(&mut h);
        h.u32(self.walkers.len() as u32);
        for w in &self.walkers {
            w.hash_into(&mut h);
        }
        h.u64(self.rng_probe());
        h.finish()
    }

    /// Deterministic probe of the RNG's internal state.
    ///
    /// `Xoshiro256PlusPlus` does not expose its state words, so hashing a clone's
    /// next output is the cheapest way to fold "where the RNG stream is" into the
    /// state hash. Two sims at the same point in the same stream always agree.
    ///
    /// Note the walkers' spawn draw already moved this stream, so two sims built
    /// from different seeds have different probes even before a single `step`.
    /// That is the intended signal: the seeds really do produce different
    /// episodes on any level with walkers.
    pub fn rng_probe(&self) -> u64 {
        use rand_xoshiro::rand_core::Rng as _;
        let mut probe = self.rng.clone();
        probe.next_u64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A time is written one way, and the two shapes meet exactly where they say they do.
    #[test]
    fn a_time_in_frames_is_written_short_and_in_order() {
        assert_eq!(format_time(0), "0.00s");
        assert_eq!(format_time(1), "0.02s", "one frame is 1/60 s");
        assert_eq!(format_time(505), "8.42s");
        assert_eq!(format_time(2400), "40.00s", "a full-width level's budget");
        // The seam: 3599 frames is 59.98 s and stays in hundredths, 3600 is a minute and changes
        // shape. A reader never sees both shapes for one time, and a time is never written two
        // ways, which is the whole point of there being one function.
        assert_eq!(format_time(3599), "59.98s");
        assert_eq!(format_time(3600), "1:00.00");
        assert_eq!(format_time(3600 + 150), "1:02.50");

        // And it is a *clock*: reading a time back gives the frame count it was made from, so a
        // column of them is ordered by frames whatever shape each one is in. Sixty frames to the
        // hundredth is exactly why hundredths are the right resolution — a frame is 0.0167 s, so
        // two episodes a frame apart still read as two times.
        for frames in [0u32, 1, 2, 59, 60, 505, 3598, 3599, 3600, 3725, 20_000, 727_000] {
            assert_eq!(
                frames_of(&format_time(frames)),
                frames as i64,
                "{} did not survive the round trip",
                format_time(frames)
            );
        }
    }

    /// Frames a written time stands for, to the nearest frame. The inverse of [`format_time`] for
    /// both of its shapes, used only by the test above — which is what makes "a slower episode
    /// never reads as a faster one" a fact rather than an impression.
    fn frames_of(written: &str) -> i64 {
        let (minutes, rest) = match written.split_once(':') {
            Some((m, rest)) => (m.parse::<f64>().expect("minutes"), rest),
            None => (0.0, written),
        };
        let seconds = rest.trim_end_matches('s').parse::<f64>().expect("seconds");
        let total = minutes * 60.0 + seconds;
        (total / DT as f64).round() as i64
    }
}
