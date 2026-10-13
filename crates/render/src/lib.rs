//! `render` — read-only views of sim state (development_plan.md §4.3).
//!
//! Boundary rule: **this crate only ever takes `&Sim`.** It never mutates sim
//! state, never advances an episode, and contains no simulation logic of its own.
//! If a view needs a number the sim does not expose, the fix belongs in `sim`,
//! not here.
//!
//! Despite the name, nothing here draws pixels. Both views are character grids:
//! one for a terminal status/observation view, one interactive keyboard playtest.
//! The pixel renderer (macroquad/egui) is an M3/M4 concern and will be a sibling
//! consumer of the same `&Sim`.

pub mod ascii;
pub mod play;

pub use ascii::{frame, frame_in, header, legend, Viewport, GLYPHS};
pub use play::{play, hint, PlaySummary, FRAME_TIME};
