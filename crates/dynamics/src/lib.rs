//! `dynamics` — the game's own 2-D physics engine.
//!
//! The sim's body is a hand-written platformer: a rectangle, a tile grid, and a page of constants
//! that say how high a jump goes. That is the right shape for the thing the player *is*, and the
//! wrong shape for the things a level *contains* — a rope that swings, a crate that falls, a stack
//! that topples. Those are a solver's job, so this crate is the solver.
//!
//! # The model
//!
//! A world holds **bodies** (point masses, some of them pinned), **links** between them (exact
//! length, or a maximum length for a rope) and **grips** (a target on a point along a link, which
//! is how a pair of hands holds a line at an arc length without the line needing a hand-shaped
//! particle). A tick integrates every free body under gravity, projects the constraints and the
//! terrain a fixed number of times, and recovers each body's velocity from the distance it
//! actually moved.
//!
//! Everything is in tiles and ticks — the sim's own units — and everything is `f32`, walked in
//! index order, with no clock, no randomness and no hash map anywhere in a step. The same world
//! stepped twice is bit-identical; [`world`]'s docs say what that costs and what it buys.
//!
//! ```
//! use dynamics::{Open, Vec2, World};
//!
//! // A pendulum: a pinned pivot, a bob, and a rod between them.
//! let mut world = World::new(Vec2::new(0.0, 0.05));
//! let pivot = world.pin(Vec2::new(0.0, 0.0));
//! let bob = world.add_body(Vec2::new(1.0, 1.0), 1.0);
//! world.connect(pivot, bob, 2.0_f32.sqrt());
//! for _ in 0..60 {
//!     world.step(&Open);
//! }
//! assert!(world.body(bob).pos.y > 0.0, "the bob fell");
//! ```
//!
//! # The one consumer, so far
//!
//! The rope the game draws is this engine: [`World`] plus a chain of rigid links, a pin at the
//! tie, a grip at the body's hands, and [`terrain::Terrain`] implemented over a level's tiles. The
//! engine is a separate crate so that the next consumer is the sim rather than another copy of
//! this code — a crate that falls, a plank that tips, a boulder that rolls — and so that both can
//! be replayed from the same numbers.
//!
//! What is deliberately missing is written down in [`world`]: no sleeping, no stored rotation, no
//! continuous collision beyond the substep split, and restitution applied to velocity rather than
//! as an impulse. Each is a real limitation, stated rather than hidden.

pub mod terrain;
mod vec2;
pub mod world;

pub use terrain::{Grid, Open, Terrain};
pub use vec2::Vec2;
pub use world::{Body, Contact, Grip, Link, LinkKind, World};

/// The engine's unit of time: one tick, the sim's decision grain.
///
/// There is no other timestep. The engine advances a whole tick per [`World::step`] and a body's
/// velocity is in tiles per *tick*; a caller that runs at sixty frames a second and a caller that
/// replays at four times real time both step the same ticks, so they compute the same world.
pub const TICK: f32 = 1.0;
