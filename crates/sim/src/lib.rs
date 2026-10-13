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
pub use crate::level::{Level, Rect, Tile};
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

/// Fitness is scored per "pixel" of new rightmost ground covered (§7.4.1).
pub const PIXELS_PER_TILE: f32 = 16.0;

/// Episodes are declared stuck after this many frames with no new rightmost x
/// (§7.4.5). Without it, campers burn the whole compute budget standing still.
pub const NO_PROGRESS_FRAMES: u32 = 600;

/// Episode cap = `level.width * FRAMES_PER_TILE` (§7.4.6).
pub const FRAMES_PER_TILE: u32 = 40;

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
    /// New rightmost ground covered, measured in pixels (§7.4.1).
    pub progress_pixels: f32,
    /// Rightmost x ever reached, in world units.
    pub best_x: f32,
    /// Consecutive frames with no new rightmost x.
    pub no_progress_frames: u32,
    /// Walkers stomped.
    pub stomps: u32,
    /// Where the agent died, if it did.
    pub death_pos: Option<(f32, f32)>,
}

impl EpisodeStats {
    fn new(spawn_x: f32) -> Self {
        EpisodeStats {
            frames: 0,
            progress_pixels: 0.0,
            best_x: spawn_x,
            no_progress_frames: 0,
            stomps: 0,
            death_pos: None,
        }
    }
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
        Sim {
            player: level.spawn_player(),
            walkers,
            stats: EpisodeStats::new(level.spawn.x),
            level: level.clone(),
            outcome: Outcome::Running,
            rng,
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

        // ---- progress / termination --------------------------------------
        // Progress is measured from the spawn x so a level never starts "in credit".
        if self.player.x > self.stats.best_x {
            let gained = (self.player.x - self.stats.best_x) * PIXELS_PER_TILE;
            self.stats.progress_pixels += gained;
            self.stats.best_x = self.player.x;
            self.stats.no_progress_frames = 0;
        } else {
            self.stats.no_progress_frames += 1;
        }

        if self.stats.no_progress_frames >= NO_PROGRESS_FRAMES {
            self.outcome = Outcome::Stuck;
            self.finish();
        } else if self.stats.frames >= self.max_frames() {
            self.outcome = Outcome::TimedOut;
            self.finish();
        }
    }

    /// Frame cap for this level (§7.4.6).
    pub fn max_frames(&self) -> u32 {
        (self.level.width.max(1) as u32) * FRAMES_PER_TILE
    }

    fn die(&mut self) {
        self.outcome = Outcome::Died;
        self.stats.death_pos = Some((self.player.x, self.player.y));
        self.finish();
    }

    fn finish(&mut self) {
        debug_assert!(self.outcome.is_finished());
    }

    /// Observation vector for the agent policy (§7.2). See [`sensors`].
    pub fn observation(&self) -> [f32; sensors::N_INPUTS] {
        sensors::observe(&self.level, &self.player, &self.walkers, &PHYSICS)
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
        h.f32(self.stats.best_x);
        h.u32(self.stats.no_progress_frames);
        h.u32(self.stats.stomps);
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
