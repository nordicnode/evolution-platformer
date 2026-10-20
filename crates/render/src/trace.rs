//! The champion path over time — the record a timeline scrubber walks (§10 Tier 2.7).
//!
//! Tier 1 answers "what is happening"; the death strip answers "where is it stuck";
//! this module answers **"what changed when it stopped being stuck"**. The plan's
//! phrasing is the spec: *record champion trajectory per generation, scrub back to
//! see the moment the jump was learned.* A path recorded every generation is a
//! path recorded mostly unchanged — a stalled run (finding 19) produces hundreds of
//! identical-looking traces — so [`TraceLog::observe`] keeps the ones that mark a
//! **new best**: the moments the champion's route actually changed.
//!
//! Like [`crate::chart`], this module depends on nothing but [`crate::ascii`]'s
//! `Body`: paths are numbers in, drawable markers out, so the scrubber's arithmetic
//! and its picture are testable without a terminal or a simulation.
//!
//! ## What a recorded trace is, and is not
//!
//! It is a *sample* of a real episode: one position every [`TRACE_SAMPLE_FRAMES`]
//! frames of the champion's own scored run, plus its exact final position. It is not
//! a replay — the sim does not store state history — so scrubbing shows the route
//! the champion took, not a re-animation of it. Recording the route is what makes
//! the arc of a learned jump visible; re-animating it is `replay`'s job, and
//! `watch --champion`'s.

use crate::ascii::Body;

/// Glyph for a cell the champion's path passed through.
///
/// ASCII printable on purpose: [`crate::ascii`] substitutes `?` for anything that
/// could not occupy exactly one terminal cell, and a trail of `?` would be worse
/// than no trail. `:` reads as a dotted line without competing with the level's own
/// glyphs (`#`, `^`, `!`, `.`).
pub const GLYPH_TRAIL: char = ':';

/// Frames between two recorded positions.
///
/// At 60 Hz this is 15 samples a second: fine enough that a jump arc — roughly half
/// a second, four tiles high — is drawn as an arc (~7 cells) rather than as a
/// straight line, coarse enough that a 5600-frame episode costs ~1400 samples.
pub const TRACE_SAMPLE_FRAMES: u32 = 4;

/// How much better the best must get before its path is worth keeping.
///
/// Relative, and small: a recorded trace is meant to mark *progress*, not noise. The
/// stall rule (finding 19) uses 5% because it is asking "has this stopped?", which
/// should not be answered by a walker-noise pixel; this asks "did the route change
/// for the better?", and 1% is comfortably above the frame-to-frame jitter an elite
/// genome shows while re-rolling its episode.
pub const TRACE_IMPROVEMENT: f32 = 0.01;

/// How many recorded improvements are kept.
///
/// Bounded because watch mode can run for hours: at ~17 KB a trace, this caps the
/// record at ~1 MB. When full the **oldest** trace is dropped, because the most
/// recent improvement is the one that explains what the population can do now; the
/// line under the chart reports the generation span the record covers, so a viewer
/// can see the window has moved rather than assuming it reaches back forever.
pub const TRACE_CAP: usize = 64;

/// One recorded position of a champion, in world tiles (top-left of its box).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TracePoint {
    pub frame: u32,
    pub x: f32,
    pub y: f32,
}

/// One generation's champion path, with the numbers that identify it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChampionTrace {
    pub generation: u32,
    /// The generation's best fitness — the number this route earned.
    pub max_fitness: f32,
    /// The route ended at the flag rather than in a death.
    pub won: bool,
    /// Sampled positions, oldest first, ending at the exact final position.
    pub points: Vec<TracePoint>,
}

impl ChampionTrace {
    /// Where the champion stopped: the flag, or the place it died.
    pub fn end(&self) -> Option<TracePoint> {
        self.points.last().copied()
    }

    /// How many frames the recorded route covers.
    pub fn frames(&self) -> u32 {
        self.end().map(|p| p.frame).unwrap_or(0)
    }

    /// The way the route ended, as a glyph.
    ///
    /// Deliberately the same vocabulary the lanes and the death strip already
    /// teach: `+` is a death marker there, and reaching the flag is `!` — so the
    /// scrubber needs no legend of its own.
    pub fn end_glyph(&self) -> char {
        if self.won {
            crate::ascii::GLYPH_GOAL
        } else {
            '+'
        }
    }

    /// The record as drawable markers: the end first, then the trail cells.
    ///
    /// The end marker is at index 0 because [`crate::ascii::Scene::primary`] is the
    /// lowest-index **live** body and is drawn last, on top of everything: where the
    /// route stopped is the one thing that must never be hidden. Trail cells are
    /// dead bodies, so they go underneath.
    ///
    /// Consecutive samples that land in the same tile collapse to one marker — the
    /// route is a path, not a smear — but a route that comes back through a cell it
    /// already visited is recorded twice, because that revisiting *is* the behaviour
    /// worth seeing.
    pub fn bodies(&self) -> Vec<Body> {
        let mut bodies = Vec::with_capacity(self.points.len() + 1);
        if let Some(end) = self.end() {
            bodies.push(marker(end, self.end_glyph(), true));
        }
        let mut last_cell: Option<(i32, i32)> = None;
        for p in &self.points {
            let cell = (p.x.floor() as i32, p.y.floor() as i32);
            if last_cell == Some(cell) {
                continue;
            }
            last_cell = Some(cell);
            bodies.push(marker(*p, GLYPH_TRAIL, false));
        }
        bodies
    }
}

/// A one-cell marker at a recorded position.
///
/// **Snapped to the tile corner**, which is what makes it exactly one cell: a 1×1
/// box at `x = 6.4` spans tiles 6 *and* 7 (the sim's span convention is
/// inclusive-of-interior), so an unsnapped marker would draw the trail two cells
/// thick — twice the width of the level it is drawn over. Snapping also puts the
/// end marker on the flag's own tile, which is where the eye looks for it.
fn marker(p: TracePoint, glyph: char, alive: bool) -> Body {
    Body::new(p.x.floor(), p.y.floor(), 1.0, 1.0, glyph, alive)
}

/// The improvements a scrubber can walk, oldest first.
#[derive(Clone, Debug, Default)]
pub struct TraceLog {
    entries: Vec<ChampionTrace>,
    best: f32,
}

impl TraceLog {
    pub fn new() -> TraceLog {
        TraceLog {
            entries: Vec::new(),
            best: f32::NEG_INFINITY,
        }
    }

    /// The fitness a new trace has to beat to be worth recording.
    ///
    /// Exposed so a caller can *ask before paying*: recording a trace costs one
    /// extra episode, and on a stalled run (the case this whole feature exists for)
    /// almost every generation is not an improvement.
    pub fn improvement_threshold(&self) -> f32 {
        self.best * (1.0 + TRACE_IMPROVEMENT)
    }

    /// Whether [`TraceLog::observe`] would keep a trace for this fitness.
    pub fn would_record(&self, max_fitness: f32) -> bool {
        // Finite only. A NaN would silently refuse every later trace (every
        // comparison against NaN is false), and an infinity would be recorded once
        // and then make the threshold infinite — freezing the record for the rest
        // of the run. Neither can be a real fitness, so neither is allowed in.
        max_fitness.is_finite() && max_fitness > self.improvement_threshold()
    }

    /// Keep `trace` if it beats the standing best. Returns whether it was kept.
    pub fn observe(&mut self, trace: ChampionTrace) -> bool {
        if !self.would_record(trace.max_fitness) {
            return false;
        }
        self.best = trace.max_fitness;
        self.entries.push(trace);
        if self.entries.len() > TRACE_CAP {
            self.entries.remove(0);
        }
        true
    }

    pub fn entries(&self) -> &[ChampionTrace] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The recorded trace for a generation, if it is still in the window.
    pub fn get(&self, generation: u32) -> Option<&ChampionTrace> {
        self.entries.iter().find(|t| t.generation == generation)
    }

    /// Position of a generation in the record, oldest first.
    pub fn index_of(&self, generation: u32) -> Option<usize> {
        self.entries.iter().position(|t| t.generation == generation)
    }

    /// The generation span the record covers, `(oldest, newest)`.
    ///
    /// Shown on screen because the record is a *window* onto a possibly much longer
    /// run, and a viewer reading "12 recorded" deserves to know from when.
    pub fn span(&self) -> Option<(u32, u32)> {
        match (self.entries.first(), self.entries.last()) {
            (Some(first), Some(last)) => Some((first.generation, last.generation)),
            _ => None,
        }
    }

    /// The generation one step *older* than `from`, or the newest when live.
    ///
    /// Navigation works in **generations, never indices**: the log drops its oldest
    /// entry when it fills, which shifts every index, so a cursor held as an index
    /// would silently start pointing at a different route than the one the viewer
    /// chose.
    pub fn step_back(&self, from: Option<u32>) -> Option<u32> {
        let last = self.entries.last()?;
        let Some(g) = from else {
            return Some(last.generation);
        };
        // The generation being shown is gone: the record drops its oldest entry
        // when it fills, and a cursor left pointing at a route that no longer
        // exists is better returned to live than silently shifted onto whichever
        // route took its place.
        self.index_of(g)?;
        // The oldest is a hard stop, not a wrap: staying put is what tells the
        // viewer they have reached the beginning.
        Some(
            self.entries
                .iter()
                .rev()
                .find(|t| t.generation < g)
                .map_or(g, |t| t.generation),
        )
    }

    /// The generation one step *newer* than `from`, or `None` to return to live.
    pub fn step_forward(&self, from: Option<u32>) -> Option<u32> {
        let g = from?;
        self.index_of(g)?;
        self.entries
            .iter()
            .find(|t| t.generation > g)
            .map(|t| t.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(generation: u32, max_fitness: f32) -> ChampionTrace {
        ChampionTrace {
            generation,
            max_fitness,
            won: false,
            points: vec![
                TracePoint {
                    frame: 0,
                    x: 2.0,
                    y: 12.0,
                },
                TracePoint {
                    frame: 4,
                    x: 6.0,
                    y: 12.0,
                },
                TracePoint {
                    frame: 8,
                    x: 10.0,
                    y: 9.0,
                },
            ],
        }
    }

    #[test]
    fn the_log_keeps_improvements_and_drops_the_rest() {
        let mut log = TraceLog::new();
        assert!(log.is_empty());
        // The first trace is always an improvement on nothing.
        assert!(log.observe(trace(0, 100.0)));
        // Half a percent better, and the same number again: neither is progress.
        assert!(!log.observe(trace(1, 100.5)));
        assert!(!log.observe(trace(2, 100.0)));
        // Two percent: recorded, and it becomes the number to beat.
        assert!(log.observe(trace(3, 102.0)));
        assert!(!log.observe(trace(4, 102.5)));
        assert!(log.observe(trace(5, 120.0)));

        assert_eq!(log.len(), 3);
        let generations: Vec<u32> = log.entries().iter().map(|t| t.generation).collect();
        assert_eq!(generations, vec![0, 3, 5]);
        assert_eq!(log.span(), Some((0, 5)));
        assert_eq!(log.get(3).map(|t| t.max_fitness), Some(102.0));
        assert_eq!(log.get(4), None);
        assert_eq!(
            log.improvement_threshold(),
            120.0 * (1.0 + TRACE_IMPROVEMENT)
        );
        assert!(log.would_record(121.5));
        assert!(!log.would_record(120.0));
    }

    #[test]
    fn a_non_finite_fitness_is_refused_without_poisoning_the_log() {
        let mut log = TraceLog::new();
        // A NaN would make every later comparison false, refusing all future
        // traces; an infinity would raise the threshold out of reach and freeze the
        // record for the rest of the run. Both are refused, and neither is fatal.
        assert!(!log.observe(trace(0, f32::NAN)));
        assert!(log.is_empty());
        assert!(log.observe(trace(1, 50.0)));
        assert!(!log.observe(trace(2, f32::INFINITY)));
        assert!(!log.observe(trace(3, f32::NEG_INFINITY)));
        assert_eq!(log.len(), 1);
        assert!(
            log.observe(trace(4, 100.0)),
            "the record still moves afterwards"
        );
        assert_eq!(log.span(), Some((1, 4)));
    }

    #[test]
    fn the_record_is_a_bounded_window_that_drops_its_oldest_entry() {
        let mut log = TraceLog::new();
        for i in 0..(TRACE_CAP as u32 + 10) {
            // Each generation is a clear improvement, so every one is recorded.
            assert!(log.observe(trace(i, 100.0 + i as f32 * 10.0)));
        }
        assert_eq!(log.len(), TRACE_CAP);
        let first = log.entries().first().expect("non-empty").generation;
        let last = log.entries().last().expect("non-empty").generation;
        assert_eq!(first, 10, "the oldest traces are the ones dropped");
        assert_eq!(last, TRACE_CAP as u32 + 9);
        assert_eq!(log.span(), Some((first, last)));
        // The dropped generation is gone, and the ones still in the window are not.
        assert_eq!(log.get(0), None);
        assert!(log.get(10).is_some());
    }

    #[test]
    fn scrubbing_moves_between_recorded_generations_and_stops_at_the_ends() {
        let mut log = TraceLog::new();
        // Note the gaps: 0, 40 and 97 were improvements; the generations between
        // them were not recorded, so navigation must step between entries rather
        // than count up by one.
        for (g, f) in [(0u32, 100.0f32), (40, 200.0), (97, 400.0)] {
            assert!(log.observe(trace(g, f)));
        }

        // Back from live lands on the newest, and each further press goes older.
        assert_eq!(log.step_back(None), Some(97));
        assert_eq!(log.step_back(Some(97)), Some(40));
        assert_eq!(log.step_back(Some(40)), Some(0));
        // The oldest is a hard stop, not a wrap: pressing back again stays there
        // (returning `None` would be indistinguishable from "go live").
        assert_eq!(log.step_back(Some(0)), Some(0));

        // Forward walks the other way, and off the end returns to live.
        assert_eq!(log.step_forward(Some(0)), Some(40));
        assert_eq!(log.step_forward(Some(40)), Some(97));
        assert_eq!(log.step_forward(Some(97)), None);
        // From live there is nothing newer.
        assert_eq!(log.step_forward(None), None);

        // An empty record has nowhere to go in either direction.
        let empty = TraceLog::new();
        assert_eq!(empty.step_back(None), None);
        assert_eq!(empty.step_forward(None), None);

        // A cursor left on a generation the window has since dropped returns to
        // live rather than jumping to a different route. Navigation itself can
        // never *produce* such a cursor (it only ever lands on recorded generations), so
        // this is a safety net, and it covers a generation that was never recorded
        // as well as one that has aged out.
        assert_eq!(log.step_back(Some(5)), None);
        assert_eq!(log.step_forward(Some(5)), None);
        let mut dropped = TraceLog::new();
        for i in 0..(TRACE_CAP as u32 + 1) {
            assert!(dropped.observe(trace(i, 100.0 + i as f32 * 10.0)));
        }
        assert!(
            dropped.get(0).is_none(),
            "generation 0 fell out of the window"
        );
        assert_eq!(dropped.step_back(Some(0)), None);
        assert_eq!(dropped.step_forward(Some(0)), None);
    }

    #[test]
    fn a_trace_draws_its_end_on_top_and_its_trail_once_per_cell() {
        let mut t = trace(7, 123.0);
        // Two samples inside one tile collapse; the route continues after them.
        t.points.insert(
            1,
            TracePoint {
                frame: 2,
                x: 6.4,
                y: 12.0,
            },
        );
        let bodies = t.bodies();
        assert_eq!(
            bodies.len(),
            4,
            "3 distinct cells of 4 samples, plus the end"
        );

        // The end marker is the primary body: first, and alive, so the renderer
        // keeps it out of the "dead bodies underneath" pass and draws it last.
        assert!(bodies[0].alive);
        assert_eq!(bodies[0].glyph, '+');
        assert_eq!(bodies[0].x, 10.0);
        assert_eq!(bodies[0].y, 9.0);
        assert!(bodies[1..].iter().all(|b| !b.alive));
        assert!(bodies[1..].iter().all(|b| b.glyph == GLYPH_TRAIL));
        // One cell each: a thicker marker would double the trail over the level.
        assert!(bodies
            .iter()
            .all(|b| b.tile_bounds() == Some((b.x as i32, b.x as i32, b.y as i32, b.y as i32))));

        // A winning route ends on the flag glyph the legend already teaches.
        let mut won = trace(8, 11100.0);
        won.won = true;
        assert_eq!(won.end_glyph(), crate::ascii::GLYPH_GOAL);
        assert_eq!(won.bodies()[0].glyph, crate::ascii::GLYPH_GOAL);
    }

    #[test]
    fn an_empty_trace_draws_nothing_rather_than_a_phantom_at_the_origin() {
        let t = ChampionTrace {
            generation: 3,
            max_fitness: 10.0,
            won: false,
            points: Vec::new(),
        };
        assert!(t.bodies().is_empty());
        assert_eq!(t.end(), None);
        assert_eq!(t.frames(), 0);
    }
}
