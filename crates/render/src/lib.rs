//! `render` — read-only views of sim state (development_plan.md §4.3).
//!
//! Boundary rule: **this crate only ever takes `&Sim`.** It never mutates sim
//! state, never advances an episode, and contains no simulation logic of its own.
//! If a view needs a number the sim does not expose, the fix belongs in `sim`,
//! not here.
//!
//! Despite the name, nothing here draws pixels. Every view is a character grid:
//! a status/observation view ([`ascii`]), an interactive keyboard playtest
//! ([`play`]), and the fitness chart and panels the watch mode is assembled from
//! ([`chart`]). The pixel renderer (macroquad/egui) is a later concern and will be
//! a sibling consumer of the same `&Sim`.
//!
//! One deliberate exception to the `&Sim` rule, and the reason for it: [`chart`]
//! depends on nothing at all -- not `sim`, not `evo`. Charts are pure numbers in,
//! text out, so the learning arc can be rendered, and unit-tested, by code that has
//! no opinion about physics. Watch mode is therefore a composition of a scene view
//! and a chart, not a fourth renderer.

pub mod ascii;
pub mod chart;
pub mod play;

pub use ascii::{
    frame, frame_in, header, legend, render_scene, Body, Scene, Viewport, GLYPHS, GLYPH_UNKNOWN,
};
pub use chart::{
    death_strip, fitness_chart, fitness_trend, format_fitness, panel, progress_bar, HistoryPoint,
    Trend,
};
pub use play::{play, hint, PlaySummary, FRAME_TIME};
