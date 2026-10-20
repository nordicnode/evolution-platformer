//! Pure text charts for the watch mode (development_plan.md §10 Tier 1.2 and §10
//! Tier 2.6).
//!
//! Everything here is data in, text out. No `sim`, no `evo`, no terminal, no I/O,
//! no colour escapes. That is deliberate on two counts: it keeps `render`'s
//! boundary rule intact (the crate reads sim state, it does not drive anything),
//! and it means every chart can be unit-tested to the exact character with no TTY.
//!
//! ## Reading the fitness chart
//!
//! The chart is the thing that answers "is this population learning?", and the
//! plan gives four readings:
//!
//! | what you see | what it means |
//! |---|---|
//! | max and avg both rising | learning |
//! | max rising, avg flat | a few good agents, the rest floundering |
//! | both flat | converged |
//! | max falling | degenerating |
//! | no new best for [`STALL_GENERATIONS`] | **stalled** |
//!
//! [`fitness_trend`] classifies those in code so the watch mode can label the
//! chart instead of leaving the player to eyeball it. The last row is the one the
//! plan's four readings cannot express: "both flat" describes the *shape* of the
//! last few generations, while a stall is a question about time — how long since
//! the population found anything genuinely better — and that is the reading which
//! decides whether a session is still worth watching. [`fitness_stall`] answers it,
//! and the watch view reports it together with the place agents keep dying,
//! because "no new best for 27 generations, deaths cluster at x 33" is a statement
//! about the *level*: §13 asks for stalling to be framed as difficulty information,
//! and §12 M5 is the stall UX.
//!
//! ## The one hard case: max and avg are orders of magnitude apart
//!
//! A champion at 10 926 next to a population averaging 348 is the *normal* state
//! of this game, not an edge case, and it is exactly what the chart must
//! communicate. On a single linear scale spanning the union of both series the
//! average line sits one row above the axis floor, where it is indistinguishable
//! from a flat line — the ambiguity the chart exists to remove.
//!
//! `fitness_chart` therefore does all three of:
//!
//! 1. Uses one shared linear scale over the union of both plotted series, so `*`
//!    and `o` are directly comparable rather than on two secretly different axes.
//! 2. Labels that scale at both ends in the left margin, so the reader can see
//!    how large the gap is.
//! 3. Prints the average series' **own** peak in the bottom margin on the axis
//!    row, prefixed with `o` to tie it to the `o` glyph. A reader who sees the
//!    scale running `30.0` to `10.9k` with the average line on the floor and
//!    `o348.4` in the corner knows immediately that the line is low because the
//!    champion is 30x the population, not because the population is stuck.
//!
//! Rescaling the average series to its own axis was the alternative and is
//! rejected: two series on two silent scales make "both rising" unreadable, which
//! is the single judgement this chart has to support.
//!
//! Every returned line is exactly the requested width, and every function clamps
//! or truncates rather than panicking, because a terminal resized mid-frame must
//! not take the watch mode down with it.

/// Characters reserved on the left for y-axis labels: an 8-character
/// right-aligned label plus one space of gutter before the plot area.
pub const MARGIN: usize = 9;

/// Narrowest chart that can still show a labelled plot. Below this,
/// [`fitness_chart`] returns a placeholder of the requested size.
pub const MIN_CHART_WIDTH: usize = MARGIN + 7;

/// Shortest chart that can still show two y labels, an interior row and an axis
/// row. Below this, [`fitness_chart`] returns a placeholder.
pub const MIN_CHART_HEIGHT: usize = 4;

/// How many trailing generations [`fitness_trend`] looks at. Long enough to see a
/// direction, short enough that a solved-then-stalled run does not read as
/// "learning" forever.
pub const TREND_TAIL: usize = 10;

/// Relative movement across the tail required to call a series rising or falling.
///
/// Relative rather than absolute on purpose: fitness spans about -120 to +11 000
/// in this game, so a fixed epsilon would call a 30-point climb "flat" at the
/// bottom of the range and a 30-point slide "flat" at the top.
pub const TREND_TOLERANCE: f32 = 0.05;

/// Plot glyph for the max-fitness series.
pub const MAX_GLYPH: char = '*';
/// Plot glyph for the average-fitness series.
pub const AVG_GLYPH: char = 'o';

/// One generation of the fitness chart.
///
/// A plain-data mirror of the trainer's `GenRow`, kept local so this module does
/// not have to depend on `evo`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HistoryPoint {
    /// Generation index this point describes.
    pub generation: u32,
    /// Best fitness in the generation.
    pub max_fitness: f32,
    /// Mean fitness across the population.
    pub avg_fitness: f32,
    /// How many agents in this generation reached the flag.
    pub wins: usize,
}

/// How the population looks to be doing, in the plan's own vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trend {
    /// Not enough history to say anything yet.
    WarmingUp,
    /// Max and average both rising.
    Learning,
    /// Max rising while the average stays flat — the plan's "few good agents".
    FewGoodAgents,
    /// Both flat.
    Converged,
    /// The best agent is getting worse.
    Degenerating,
    /// No new best for [`STALL_GENERATIONS`]: the run has stopped finding anything
    /// better, whatever the last few generations happen to look like.
    Stalled,
    /// An agent reached the flag. Outranks every other reading.
    Solved,
}

impl Trend {
    /// A short human sentence for the chart's caption.
    pub fn label(self) -> &'static str {
        match self {
            Trend::WarmingUp => "warming up: not enough generations yet",
            Trend::Learning => "learning: max and avg both rising",
            Trend::FewGoodAgents => "few good agents: max rising, average flat",
            Trend::Converged => "converged: fitness flat",
            Trend::Degenerating => "degenerating: max falling",
            Trend::Stalled => "stalled: no new best in a long time",
            Trend::Solved => "solved: an agent reached the flag",
        }
    }
}

/// Which way a series is going over the window it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Rising,
    Flat,
    Falling,
}

/// Least-squares slope of `values` against their index.
fn slope(values: &[f32]) -> f32 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    let mean_x = (n as f32 - 1.0) * 0.5;
    let mean_y = values.iter().sum::<f32>() / n as f32;
    let mut num = 0.0f32;
    let mut den = 0.0f32;
    for (i, y) in values.iter().enumerate() {
        let dx = i as f32 - mean_x;
        num += dx * (y - mean_y);
        den += dx * dx;
    }
    if den == 0.0 {
        0.0
    } else {
        num / den
    }
}

/// Classify one series.
///
/// Compares the slope's *total* predicted change across the window against the
/// series' own magnitude, so the threshold is a percentage of the values being
/// plotted rather than a number that only makes sense in one part of the fitness
/// range. A least-squares slope is used instead of "last minus first" so a single
/// noisy generation cannot flip the reading.
fn direction(values: &[f32]) -> Direction {
    if values.len() < 2 {
        return Direction::Flat;
    }
    let total = slope(values) * (values.len() as f32 - 1.0);
    let magnitude = values
        .iter()
        .fold(
            0.0f32,
            |m, v| if v.is_finite() { m.max(v.abs()) } else { m },
        )
        .max(1.0);
    let relative = total / magnitude;
    if relative > TREND_TOLERANCE {
        Direction::Rising
    } else if relative < -TREND_TOLERANCE {
        Direction::Falling
    } else {
        Direction::Flat
    }
}

/// Classify a history using only its most recent [`TREND_TAIL`] points.
pub fn fitness_trend(history: &[HistoryPoint]) -> Trend {
    if history.len() < 2 {
        return Trend::WarmingUp;
    }
    let start = history.len().saturating_sub(TREND_TAIL);
    let tail = &history[start..];

    // A win is terminal information: once someone has touched the flag, no other
    // reading of the same chart is more useful.
    if tail[tail.len() - 1].wins > 0 {
        return Trend::Solved;
    }

    // A stall outranks the shape of the tail. Both can be true at once — a run that
    // improved once, long ago, can still show a rising maximum *inside* the last ten
    // generations while never coming close to its own best — and in that case the
    // shape of the tail is the misleading half: "few good agents: max rising" beside
    // "nothing better for 40 generations" reads as an invitation to keep waiting.
    if fitness_stall(history).is_some() {
        return Trend::Stalled;
    }

    let maxes: Vec<f32> = tail.iter().map(|p| p.max_fitness).collect();
    let avgs: Vec<f32> = tail.iter().map(|p| p.avg_fitness).collect();
    let max_dir = direction(&maxes);
    let avg_dir = direction(&avgs);

    // Enumerated as nested matches so every combination is visibly handled.
    match max_dir {
        Direction::Rising => match avg_dir {
            // The plan's headline reading.
            Direction::Rising => Trend::Learning,
            // "Max-only rising = few good agents": the champion is improving while
            // most of the population is not.
            Direction::Flat => Trend::FewGoodAgents,
            Direction::Falling => Trend::FewGoodAgents,
        },
        Direction::Flat => match avg_dir {
            // The population as a whole is catching up to a flat ceiling.
            Direction::Rising => Trend::Learning,
            Direction::Flat => Trend::Converged,
            Direction::Falling => Trend::Degenerating,
        },
        Direction::Falling => Trend::Degenerating,
    }
}

/// How many generations without a meaningful improvement to the best fitness make
/// a run *stalled*.
///
/// A number had to be picked, so it is picked from the measured ladder rather than
/// from taste: the slowest shipped level solves at a median of 20 generations (L3,
/// FINDINGS.md, the difficulty ladder), so 25 flat generations is longer than the *median*
/// solve on every level in the pack — a run that goes that long without improving is
/// not merely between improvements. It is long enough that ordinary noise — a
/// walker's patrol phase re-rolled for an elite, so the generation's max can fall on
/// a level that is still improving (finding 14) — cannot trigger it, and short enough
/// to fire inside the pack's real stalls: L7 seed 12 stands still for 89 generations
/// before its wall climb is found, and this line reports the first 64 of them.
pub const STALL_GENERATIONS: u32 = 25;

/// A run whose best fitness has stopped moving.
///
/// Plain data, no text: the watch view composes the line, and tests can assert on
/// the numbers instead of on a phrase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stall {
    /// Generations since the best last improved, counted from the newest point.
    pub generations: u32,
    /// Generation that set the standing best.
    pub since_generation: u32,
    /// The standing best max-fitness.
    pub ceiling: f32,
}

impl Stall {
    /// The stall's figures as one phrase, for the watch view's status area.
    ///
    /// Deliberately without a "stalled:" prefix: [`Trend::Stalled`] already says
    /// that on the line above, and repeating it wastes the width a terminal does not
    /// have. This half carries the numbers the reading cannot — how long, and how
    /// good the best that nobody can beat is.
    pub fn label(&self) -> String {
        format!(
            "no new best in {} generations (best {} set at gen {})",
            self.generations,
            format_fitness(self.ceiling),
            self.since_generation
        )
    }
}

/// Detect a stall, or answer `None` while the run is still finding improvements.
///
/// "No new best" is judged with [`TREND_TOLERANCE`] *relative to the standing best*,
/// the same rule [`direction`] uses to call a series flat: a run that wobbles by a
/// pixel a generation while going nowhere is stalled, and a run that gains a percent
/// of its own best is not. Relative on purpose, for the reason given on
/// [`TREND_TOLERANCE`] — fitness here spans -120 to +11 000, so one absolute
/// threshold cannot mean the same thing at both ends.
///
/// It looks at the **whole** history rather than the [`TREND_TAIL`] window, which is
/// the point: the tail answers "what shape is the last stretch", and a stall is a
/// question about how long ago the population last found something better, which is
/// invisible in any fixed window once the answer exceeds it.
///
/// A win is never a stall. Once an agent has reached the flag the job is done, and a
/// flat stretch *after* a win on a walker level is the elite re-rolling its episode
/// (finding 14), not a population that cannot get further.
pub fn fitness_stall(history: &[HistoryPoint]) -> Option<Stall> {
    let first = history.first()?;
    let newest = history.last()?;
    if newest.wins > 0 {
        return None;
    }

    let mut best = first.max_fitness;
    let mut best_generation = first.generation;
    for point in history.iter().skip(1) {
        let threshold = best.abs().max(1.0) * TREND_TOLERANCE;
        if point.max_fitness > best + threshold {
            best = point.max_fitness;
            best_generation = point.generation;
        }
    }

    let generations = newest.generation.saturating_sub(best_generation);
    if generations < STALL_GENERATIONS {
        return None;
    }
    Some(Stall {
        generations,
        since_generation: best_generation,
        ceiling: best,
    })
}

/// The level tile where deaths are piling up, with how many fell there.
///
/// [`death_strip`] already shows this as intensity glyphs, and this is the same data
/// as a number: "deaths cluster at x 33" names the obstacle a viewer could also
/// squint at a row of `#` to find, and it is what a level designer needs in order to
/// do something about it. Ties go to the leftmost column, so the answer is stable.
///
/// Marks outside the level (or with a non-finite x) are ignored exactly as in
/// [`death_strip`], so the two always agree about where the population died.
pub fn death_cluster(marks: &[(f32, f32, u32)], level_width_tiles: i32) -> Option<(i32, usize)> {
    if level_width_tiles <= 0 {
        return None;
    }
    let mut counts = vec![0usize; level_width_tiles as usize];
    for (x, _y, _generation) in marks {
        if !x.is_finite() || *x < 0.0 || *x >= level_width_tiles as f32 {
            continue;
        }
        counts[*x as usize] += 1;
    }
    let mut busiest: Option<(i32, usize)> = None;
    for (tile, count) in counts.iter().enumerate() {
        if *count == 0 {
            continue;
        }
        // Strictly greater, scanning left to right, so a tie keeps the leftmost
        // obstacle: the one the population met first is the one to fix first.
        let better = match busiest {
            Some((_, n)) => *count > n,
            None => true,
        };
        if better {
            busiest = Some((tile as i32, *count));
        }
    }
    busiest
}

/// Which row of a `rows`-tall plot a value belongs on, with row 0 at the top.
/// Returns `None` for a non-finite value so it is simply not drawn.
fn row_of(v: f32, lo: f32, span: f32, rows: usize) -> Option<usize> {
    if rows == 0 || !v.is_finite() || !span.is_finite() || span <= 0.0 {
        return None;
    }
    let t = ((v - lo) / span).clamp(0.0, 1.0);
    let r = ((1.0 - t) * (rows as f32 - 1.0)).round();
    Some((r.max(0.0) as usize).min(rows - 1))
}

/// Right-align `text` inside [`MARGIN`] characters, leaving one space of gutter
/// before the plot. Long text is truncated from the right, which keeps the
/// leading digits of a negative or large number.
fn margin_label(text: &str) -> String {
    let inner = MARGIN - 1;
    let taken: String = text.chars().take(inner).collect();
    let mut out = String::with_capacity(MARGIN);
    for _ in taken.chars().count()..inner {
        out.push(' ');
    }
    out.push_str(&taken);
    out.push(' ');
    out
}

/// Pad or truncate `s` to exactly `width` characters, counting characters rather
/// than bytes so a multi-byte glyph cannot split and panic.
fn fit(s: &str, width: usize) -> String {
    let mut out: String = s.chars().take(width).collect();
    while out.chars().count() < width {
        out.push(' ');
    }
    out
}

/// A placeholder of exactly `height` lines of exactly `width` characters.
fn placeholder(width: usize, height: usize, msg: &str) -> Vec<String> {
    (0..height)
        .map(|i| {
            if i == height / 2 {
                fit(msg, width)
            } else {
                fit("", width)
            }
        })
        .collect()
}

/// Two-series fitness chart: `*` is max fitness, `o` is average fitness.
///
/// Returns exactly `height` lines, each exactly `width` characters. The last line
/// is the axis rule; the row above it is the bottom of the scale (and carries the
/// low label in the margin), and the first row carries the high label. The axis
/// row's margin carries the average series' own peak, prefixed with `o` — see the
/// module doc comment for why that matters.
///
/// Degenerate sizes (`width` below [`MIN_CHART_WIDTH`], `height` below
/// [`MIN_CHART_HEIGHT`], or an empty history) return a sized placeholder rather
/// than panicking, because a terminal can be resized at any moment.
pub fn fitness_chart(history: &[HistoryPoint], width: usize, height: usize) -> Vec<String> {
    if height == 0 {
        return Vec::new();
    }
    if width < MIN_CHART_WIDTH || height < MIN_CHART_HEIGHT {
        return placeholder(width, height, "(chart too small)");
    }
    if history.is_empty() {
        return placeholder(width, height, "(no history yet)");
    }

    let plot_w = width - MARGIN;
    let plot_rows = height - 1; // the final row is the axis rule
    let start = history.len().saturating_sub(plot_w);
    let points = &history[start..];

    // Shared scale over the union of both series.
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for p in points {
        for v in [p.max_fitness, p.avg_fitness] {
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    if hi - lo < 1.0e-3 {
        // A constant series still deserves a band it can be drawn inside.
        let c = (hi + lo) * 0.5;
        lo = c - 1.0;
        hi = c + 1.0;
    }
    let span = hi - lo;

    let mut cells = vec![vec!['.'; plot_w]; plot_rows];
    let mut avg_peak = f32::NEG_INFINITY;
    for (col, p) in points.iter().enumerate() {
        if p.avg_fitness.is_finite() {
            avg_peak = avg_peak.max(p.avg_fitness);
        }
        if let Some(r) = row_of(p.max_fitness, lo, span, plot_rows) {
            cells[r][col] = MAX_GLYPH;
        }
        if let Some(r) = row_of(p.avg_fitness, lo, span, plot_rows) {
            // Max wins a shared cell: the champion's position is the more
            // important thing to preserve, and the average is always in the
            // margin anyway.
            if cells[r][col] != MAX_GLYPH {
                cells[r][col] = AVG_GLYPH;
            }
        }
    }

    let mut out = Vec::with_capacity(height);
    for (r, row) in cells.iter().enumerate().take(plot_rows) {
        let label = if r == 0 {
            format_fitness(hi)
        } else if r == plot_rows - 1 {
            format_fitness(lo)
        } else {
            String::new()
        };
        let mut line = margin_label(&label);
        line.extend(row.iter());
        out.push(line);
    }

    // The `o` prefix ties this number to the `o` glyph, so a squeezed average
    // series reads as "the average line is low because its own peak is 315.0",
    // not as "the population is stuck".
    let peak = if avg_peak.is_finite() { avg_peak } else { lo };
    let mut axis = margin_label(&format!("{}{}", AVG_GLYPH, format_fitness(peak)));
    axis.push('+');
    for _ in 1..plot_w {
        axis.push('-');
    }
    out.push(axis);

    debug_assert_eq!(out.len(), height);
    out
}

/// A bracketed bar of exactly `width` characters, e.g. `[======>-----]` at 47 %.
///
/// Returns the bar **only** — the caller appends its own percentage, because the
/// caller is the one that knows whether it wants "47%" or "47 / 200 genomes",
/// and because baking a suffix in would make `width` mean two different things.
/// `fraction` is clamped to `0.0..=1.0` and NaN reads as `0.0`.
pub fn progress_bar(fraction: f32, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "=".to_string();
    }
    let f = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    };
    let inner = width - 2;
    let filled = ((f * inner as f32).round() as usize).min(inner);

    let mut out = String::with_capacity(width);
    out.push('[');
    for i in 0..inner {
        out.push(if i < filled {
            '='
        } else if i == filled && filled > 0 && filled < inner {
            '>'
        } else {
            '-'
        });
    }
    out.push(']');
    out
}

/// A `+--+` rule of exactly `width` characters.
fn rule(width: usize, left: char, fill: char, right: char) -> String {
    let mut s = String::with_capacity(width);
    if width == 0 {
        return s;
    }
    s.push(left);
    for _ in 0..width.saturating_sub(2) {
        s.push(fill);
    }
    if width >= 2 {
        s.push(right);
    }
    s
}

/// A boxed panel: a border, the title, a separator, the body lines, a border.
///
/// Every returned line is exactly `width` characters. Widths below 4 collapse to
/// truncated empty boxes rather than panicking.
pub fn panel(title: &str, lines: &[String], width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    // Lay the box out at a workable size, then trim to whatever was asked for, so
    // a mid-frame shrink cannot underflow the arithmetic below.
    let w = width.max(4);
    let inner = w - 4;

    let mut out = Vec::with_capacity(lines.len() + 3);
    out.push(rule(w, '+', '-', '+'));
    out.push(format!("| {} |", fit(title, inner)));
    out.push(rule(w, '+', '-', '+'));
    for line in lines {
        out.push(format!("| {} |", fit(line, inner)));
    }
    out.push(rule(w, '+', '-', '+'));

    out.iter().map(|l| fit(l, width)).collect()
}

/// Compact fitness for a narrow column, never longer than 7 characters.
///
/// `10926.9 -> "10.9k"`, `348.44 -> "348.4"`, `-120.0 -> "-120.0"`, `NaN -> "nan"`.
pub fn format_fitness(v: f32) -> String {
    if v.is_nan() {
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    let a = v.abs();
    let s = if a >= 1_000_000_000.0 {
        // Scientific, because a fitness in the billions is a bug or a runaway
        // mutation, and "3.4e38" is both shorter and more useful than "3402823".
        format!("{:.1e}", v)
    } else if a >= 1_000_000.0 {
        format!("{:.1}M", v / 1_000_000.0)
    } else if a >= 10_000.0 {
        format!("{:.1}k", v / 1_000.0)
    } else if a >= 1_000.0 {
        format!("{:.2}k", v / 1_000.0)
    } else if a >= 100.0 {
        format!("{v:.1}")
    } else if a >= 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.3}")
    };
    // The branches above are sized to stay within the bound; this is the guard
    // that makes the documented contract unconditional for any input.
    if s.chars().count() > 7 {
        s.chars().take(7).collect()
    } else {
        s
    }
}

/// Where agents have been dying, as a one-line strip across the level.
///
/// `marks` are `(x in world tiles, y in world tiles, generation)`. Marks outside
/// `0.0..level_width_tiles` (or with a non-finite x) are ignored. Intensity is the
/// number of deaths that fall in a column, so a later mark can only ever raise a
/// column's intensity, never lower it. Returns exactly `width` characters.
pub fn death_strip(marks: &[(f32, f32, u32)], level_width_tiles: i32, width: usize) -> String {
    let mut out = String::with_capacity(width);
    if width == 0 {
        return out;
    }
    if level_width_tiles <= 0 {
        return ".".repeat(width);
    }

    let mut counts = vec![0u32; width];
    let level_width = level_width_tiles as f32;
    for (x, _y, _generation) in marks {
        if !x.is_finite() || *x < 0.0 || *x >= level_width {
            continue;
        }
        let col = ((*x / level_width) * width as f32).floor();
        if col < 0.0 {
            continue;
        }
        let col = col as usize;
        if col < width {
            counts[col] = counts[col].saturating_add(1);
        }
    }

    for c in counts {
        out.push(match c {
            0 => '.',
            1 => ':',
            2..=3 => '+',
            _ => '#',
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A history where max and avg both climb steadily.
    fn learning() -> Vec<HistoryPoint> {
        (0..12)
            .map(|i| HistoryPoint {
                generation: i,
                max_fitness: 30.0 + i as f32 * 100.0,
                avg_fitness: 25.0 + i as f32 * 60.0,
                wins: 0,
            })
            .collect()
    }

    fn flat(count: u32) -> Vec<HistoryPoint> {
        (0..count)
            .map(|i| HistoryPoint {
                generation: i,
                max_fitness: 500.0,
                avg_fitness: 200.0,
                wins: 0,
            })
            .collect()
    }

    #[test]
    fn trend_treats_a_short_history_as_warming_up() {
        assert_eq!(fitness_trend(&[]), Trend::WarmingUp);
        assert_eq!(
            fitness_trend(&[HistoryPoint {
                generation: 0,
                max_fitness: 1.0,
                avg_fitness: 1.0,
                wins: 0,
            }]),
            Trend::WarmingUp
        );
    }

    #[test]
    fn trend_sees_max_and_avg_both_rising_as_learning() {
        assert_eq!(fitness_trend(&learning()), Trend::Learning);
    }

    #[test]
    fn trend_sees_max_rising_over_a_flat_average_as_few_good_agents() {
        // The plan's "max-only rising = few good agents" reading.
        let history: Vec<HistoryPoint> = (0..12)
            .map(|i| HistoryPoint {
                generation: i,
                max_fitness: 30.0 + i as f32 * 100.0,
                avg_fitness: 25.0,
                wins: 0,
            })
            .collect();
        assert_eq!(fitness_trend(&history), Trend::FewGoodAgents);
    }

    #[test]
    fn trend_sees_a_flat_pair_as_converged_and_a_falling_max_as_degenerating() {
        assert_eq!(fitness_trend(&flat(12)), Trend::Converged);

        let falling: Vec<HistoryPoint> = (0..12)
            .map(|i| HistoryPoint {
                generation: i,
                max_fitness: 2000.0 - i as f32 * 100.0,
                avg_fitness: 500.0 - i as f32 * 20.0,
                wins: 0,
            })
            .collect();
        assert_eq!(fitness_trend(&falling), Trend::Degenerating);
    }

    #[test]
    fn a_win_outranks_every_other_reading() {
        // Take a history that would otherwise read as degenerating and add a win
        // to the newest point.
        let mut history: Vec<HistoryPoint> = (0..12)
            .map(|i| HistoryPoint {
                generation: i,
                max_fitness: 2000.0 - i as f32 * 100.0,
                avg_fitness: 500.0 - i as f32 * 20.0,
                wins: 0,
            })
            .collect();
        assert_eq!(fitness_trend(&history), Trend::Degenerating);
        history.last_mut().expect("non-empty").wins = 1;
        assert_eq!(fitness_trend(&history), Trend::Solved);
    }

    #[test]
    fn trend_tolerance_absorbs_a_single_noisy_generation() {
        // A converged run with one wobble on the last point must not flip to
        // rising: that is what the tolerance and the least-squares slope are for.
        let mut history = flat(12);
        if let Some(last) = history.last_mut() {
            last.max_fitness = 503.0;
        }
        assert_eq!(
            fitness_trend(&history),
            Trend::Converged,
            "a 0.6% wobble must not read as learning"
        );
    }

    #[test]
    fn chart_has_exactly_the_requested_shape() {
        let history = learning();
        for (w, h) in [(16usize, 4usize), (40, 10), (80, 20), (120, 6)] {
            let lines = fitness_chart(&history, w, h);
            assert_eq!(lines.len(), h, "height {h} at width {w}");
            for (i, line) in lines.iter().enumerate() {
                assert_eq!(
                    line.chars().count(),
                    w,
                    "line {i} of the {w}x{h} chart is {} chars: {line:?}",
                    line.chars().count()
                );
            }
        }
    }

    #[test]
    fn chart_survives_degenerate_sizes_without_panicking() {
        let history = learning();
        for w in [0usize, 1, 2, 8, 15] {
            for h in [0usize, 1, 2, 3] {
                let lines = fitness_chart(&history, w, h);
                assert_eq!(lines.len(), h, "{w}x{h}");
                for line in &lines {
                    assert_eq!(line.chars().count(), w, "{w}x{h}: {line:?}");
                }
            }
        }
    }

    #[test]
    fn chart_placeholder_is_sized_correctly_for_an_empty_history() {
        let lines = fitness_chart(&[], 40, 6);
        assert_eq!(lines.len(), 6);
        assert!(lines.iter().all(|l| l.chars().count() == 40));
        assert!(
            lines.iter().any(|l| l.contains("no history")),
            "an empty history must say so: {lines:?}"
        );
    }

    #[test]
    fn chart_draws_both_series_in_one_plot() {
        let history = learning();
        let lines = fitness_chart(&history, 48, 8);
        let body = lines.join("\n");
        assert!(
            body.contains(MAX_GLYPH),
            "the max series must be drawn:\n{body}"
        );
        assert!(
            body.contains(AVG_GLYPH),
            "the avg series must be drawn:\n{body}"
        );
    }

    #[test]
    fn a_squeezed_average_series_is_still_drawn_and_its_own_peak_is_reported() {
        // The normal state of this game: a champion 30x the population average.
        let history: Vec<HistoryPoint> = (0..20)
            .map(|i| HistoryPoint {
                generation: i,
                max_fitness: 100.0 + i as f32 * 500.0,
                avg_fitness: 30.0 + i as f32 * 15.0,
                wins: 0,
            })
            .collect();
        let lines = fitness_chart(&history, 60, 10);
        let body = lines.join("\n");
        assert!(
            body.contains(AVG_GLYPH),
            "avg must survive the squeeze:\n{body}"
        );

        let axis = lines.last().expect("axis row");
        let margin: String = axis.chars().take(MARGIN).collect();
        assert!(
            margin.contains(AVG_GLYPH),
            "the axis margin must report the avg series' own peak, got {margin:?}"
        );
        // 30 + 19*15 = 315, which compacts to "315.0".
        assert!(
            margin.contains("315.0"),
            "expected the avg peak in {margin:?}"
        );
        // And the shared scale's top must be the champion-sized number, so the
        // reader can see *why* the average line is low.
        let top_margin: String = lines[0].chars().take(MARGIN).collect();
        assert!(
            top_margin.contains("k"),
            "the top label should be a champion-sized value, got {top_margin:?}"
        );
    }

    #[test]
    fn chart_ignores_non_finite_values_instead_of_panicking() {
        let mut history = learning();
        history.push(HistoryPoint {
            generation: 99,
            max_fitness: f32::NAN,
            avg_fitness: f32::INFINITY,
            wins: 0,
        });
        let lines = fitness_chart(&history, 50, 8);
        assert_eq!(lines.len(), 8);
        assert!(lines.iter().all(|l| l.chars().count() == 50));
    }

    #[test]
    fn chart_shows_the_recent_arc_not_the_beginning() {
        // Long history, sharply rising at the end: the newest point must be the
        // topmost plotted row.
        let mut history = flat(200);
        for i in 0..10 {
            history.push(HistoryPoint {
                generation: 200 + i,
                max_fitness: 500.0 + (i as f32 + 1.0) * 500.0,
                avg_fitness: 200.0,
                wins: 0,
            });
        }
        let lines = fitness_chart(&history, 40, 8);
        let first_body_row_margin: String = lines[0].chars().take(MARGIN).collect();
        // 500 + 10*500 = 5500 -> "5.50k"
        assert!(
            first_body_row_margin.contains("5.50k"),
            "the top of the scale should be the newest point, got {first_body_row_margin:?}"
        );
    }

    #[test]
    fn progress_bar_covers_both_ends_and_clamps() {
        assert_eq!(progress_bar(0.0, 12), "[----------]");
        assert_eq!(progress_bar(1.0, 12), "[==========]");
        // Inner width is 10, so 0.5 fills exactly 5 cells, then the arrow head.
        assert_eq!(progress_bar(0.5, 12), "[=====>----]");
        assert_eq!(progress_bar(0.47, 14), "[======>-----]");
        // Out of range clamps rather than wrapping or panicking.
        assert_eq!(progress_bar(-5.0, 12), "[----------]");
        assert_eq!(progress_bar(5.0, 12), "[==========]");
        assert_eq!(progress_bar(f32::NAN, 12), "[----------]");
    }

    #[test]
    fn progress_bar_is_exactly_the_requested_width() {
        for w in [2usize, 3, 12, 40, 80] {
            for f in [0.0f32, 0.33, 0.5, 0.99, 1.0] {
                let bar = progress_bar(f, w);
                assert_eq!(bar.chars().count(), w, "width {w} at {f}");
            }
        }
        assert_eq!(progress_bar(0.5, 0), "");
        assert_eq!(progress_bar(0.5, 1), "=");
    }

    #[test]
    fn format_fitness_stays_short_and_readable_across_the_whole_range() {
        for v in [
            0.0f32,
            0.5,
            -0.5,
            1.0,
            9.99,
            99.9,
            348.44,
            -120.0,
            999.9,
            1_234.5,
            -1_234.5,
            9_999.0,
            10_926.9,
            -10_926.9,
            999_999.0,
            1_234_567.0,
            -1_234_567.0,
            f32::MAX,
            f32::MIN,
        ] {
            let s = format_fitness(v);
            assert!(
                s.chars().count() <= 7,
                "format_fitness({v}) = {s:?} exceeds 7 chars"
            );
            assert!(!s.trim().is_empty(), "format_fitness({v}) is blank");
        }
        assert_eq!(format_fitness(10_926.9), "10.9k");
        assert_eq!(format_fitness(348.44), "348.4");
        assert_eq!(format_fitness(-120.0), "-120.0");
        assert_eq!(format_fitness(f32::NAN), "nan");
        assert_eq!(format_fitness(f32::INFINITY), "inf");
        assert_eq!(format_fitness(f32::NEG_INFINITY), "-inf");
    }

    #[test]
    fn panel_lines_are_all_exactly_the_requested_width() {
        let body = vec![
            "first".to_string(),
            String::new(),
            "a much longer line than fits".into(),
        ];
        for width in [4usize, 10, 20, 40] {
            let lines = panel("title", &body, width);
            assert_eq!(lines.len(), body.len() + 4, "width {width}");
            for line in &lines {
                assert_eq!(line.chars().count(), width, "width {width}: {line:?}");
            }
        }
        // Narrow-but-possible widths must not panic or lose the shape of the result.
        for width in [1usize, 2, 3] {
            let lines = panel("t", &body, width);
            assert_eq!(lines.len(), body.len() + 4, "width {width}");
            for line in &lines {
                assert_eq!(line.chars().count(), width, "width {width}: {line:?}");
            }
        }
        // A zero-width panel has no shape to preserve: it is nothing, not four
        // empty rows pretending to be a box. Asserted separately because it is a
        // different contract from "narrow", which is why it is not in the loop
        // above.
        assert_eq!(
            panel("t", &body, 0).len(),
            0,
            "a zero-width panel is nothing"
        );
    }

    #[test]
    fn death_strip_intensity_climbs_with_the_number_of_deaths() {
        // One level tile per column: x=0.5 lands in column 0, x=2.5 in column 2.
        assert_eq!(death_strip(&[(0.5, 0.0, 1)], 4, 4), ":...");
        assert_eq!(death_strip(&[(0.5, 0.0, 1), (0.5, 0.0, 2)], 4, 4), "+...");
        assert_eq!(
            death_strip(&[(0.5, 0.0, 1), (0.5, 0.0, 2), (0.5, 0.0, 3)], 4, 4),
            "+..."
        );
        let four: Vec<(f32, f32, u32)> = (0..4).map(|g| (0.5, 0.0, g)).collect();
        assert_eq!(death_strip(&four, 4, 4), "#...");
    }

    #[test]
    fn death_strip_ignores_marks_outside_the_level_and_degenerate_widths() {
        assert_eq!(
            death_strip(&[(-1.0, 0.0, 1), (9.0, 0.0, 1), (f32::NAN, 0.0, 1)], 4, 4),
            "....",
            "out-of-range and non-finite x must be ignored"
        );
        assert_eq!(death_strip(&[(1.0, 0.0, 1)], 4, 0), "");
        assert_eq!(
            death_strip(&[(1.0, 0.0, 1)], 0, 4),
            "....",
            "a level with no width has no columns to attribute deaths to"
        );
        // The strip scales with the requested width.
        let strip = death_strip(&[(2.0, 0.0, 1)], 8, 16);
        assert_eq!(strip.chars().count(), 16);
        assert_eq!(strip.chars().filter(|c| *c != '.').count(), 1);
    }

    #[test]
    fn labels_are_human_readable_for_every_trend() {
        for trend in [
            Trend::WarmingUp,
            Trend::Learning,
            Trend::FewGoodAgents,
            Trend::Converged,
            Trend::Degenerating,
            Trend::Stalled,
            Trend::Solved,
        ] {
            assert!(!trend.label().is_empty());
            assert!(trend.label().chars().count() <= 48, "{:?}", trend);
        }
        assert_ne!(Trend::Learning.label(), Trend::Converged.label());
        assert_ne!(Trend::Stalled.label(), Trend::Converged.label());
    }

    /// A run whose best was reached at generation 1 and has not moved since, with
    /// exactly `stalled` generations after that.
    ///
    /// The generation arithmetic lives in here so the tests below read as statements
    /// about the cutoff rather than about indices.
    fn stalled_for(best: f32, stalled: u32) -> Vec<HistoryPoint> {
        let mut h = vec![HistoryPoint {
            generation: 0,
            max_fitness: best * 0.5,
            avg_fitness: best * 0.2,
            wins: 0,
        }];
        for i in 1..=stalled + 1 {
            h.push(HistoryPoint {
                generation: i,
                max_fitness: best,
                avg_fitness: best * 0.3,
                wins: 0,
            });
        }
        h
    }

    #[test]
    fn a_stall_starts_exactly_at_the_cutoff() {
        assert_eq!(
            fitness_stall(&stalled_for(1000.0, STALL_GENERATIONS - 1)),
            None,
            "one generation short of the cutoff is not a stall"
        );
        let stall = fitness_stall(&stalled_for(1000.0, STALL_GENERATIONS)).expect("stalled");
        assert_eq!(stall.generations, STALL_GENERATIONS);
        assert_eq!(stall.ceiling, 1000.0);
        assert_eq!(
            stall.since_generation, 1,
            "the best was set by generation 1; the flat run is counted from there"
        );
        let label = stall.label();
        assert!(
            label.contains("no new best in 25 generations"),
            "the label has to carry the number the viewer is watching for: {label}"
        );
        assert!(
            label.contains("1.00k") && label.contains("gen 1"),
            "and where the best was set, in the chart's own number format: {label}"
        );
        // A history too short to hold a stall cannot report one whatever it says.
        assert_eq!(fitness_stall(&[]), None);
        assert_eq!(fitness_stall(&stalled_for(1000.0, 1)), None);
        let single = vec![HistoryPoint {
            generation: 7,
            max_fitness: 42.0,
            avg_fitness: 1.0,
            wins: 0,
        }];
        assert_eq!(fitness_stall(&single), None);
    }

    #[test]
    fn noise_does_not_end_a_stall_but_a_real_improvement_does() {
        // A ceiling of 1000 with TREND_TOLERANCE 0.05: a full 5% is an improvement, a
        // pixel is not. Both are asserted, because that tolerance is the only thing
        // stopping walker noise from resetting the clock forever (finding 14).
        let mut noisy = stalled_for(1000.0, STALL_GENERATIONS);
        let last = noisy.len() - 1;
        noisy[last].max_fitness = 1002.0;
        assert!(
            fitness_stall(&noisy).is_some(),
            "two points better than the standing best is a wobble, not progress"
        );

        let mut h = stalled_for(1000.0, STALL_GENERATIONS);
        let end = h.len() as u32;
        h.push(HistoryPoint {
            generation: end,
            max_fitness: 1100.0,
            avg_fitness: 300.0,
            wins: 0,
        });
        assert_eq!(
            fitness_stall(&h),
            None,
            "10% better than the standing best ends the stall"
        );

        // And a fresh stall is measured from the improvement, not from generation 0.
        for i in 1..=STALL_GENERATIONS {
            h.push(HistoryPoint {
                generation: end + i,
                max_fitness: 1100.0,
                avg_fitness: 300.0,
                wins: 0,
            });
        }
        let again = fitness_stall(&h).expect("stalled again");
        assert_eq!(again.since_generation, end);
        assert_eq!(again.generations, STALL_GENERATIONS);
        assert_eq!(again.ceiling, 1100.0);
    }

    #[test]
    fn a_stall_outranks_a_rising_tail_and_a_win_outranks_both() {
        // The awkward case the extra reading exists for: the best is ancient, the
        // last ten generations are climbing, and nothing has come near the best.
        let mut h = vec![HistoryPoint {
            generation: 0,
            max_fitness: 2000.0,
            avg_fitness: 400.0,
            wins: 0,
        }];
        for i in 1..=STALL_GENERATIONS {
            h.push(HistoryPoint {
                generation: i,
                max_fitness: 500.0 + i as f32 * 40.0,
                avg_fitness: 300.0,
                wins: 0,
            });
        }
        assert!(
            direction(&h[1..].iter().map(|p| p.max_fitness).collect::<Vec<_>>())
                == Direction::Rising,
            "the tail really is rising, or this test proves nothing"
        );
        assert_eq!(
            fitness_trend(&h),
            Trend::Stalled,
            "a rising tail under an out-of-reach best must not read as learning"
        );

        // A win beats everything, including a stall: the job is done.
        h.last_mut().expect("non-empty").wins = 1;
        assert_eq!(fitness_trend(&h), Trend::Solved);
        assert_eq!(
            fitness_stall(&h),
            None,
            "a winning run is not a stalled one"
        );
    }

    #[test]
    fn death_cluster_names_the_busiest_tile() {
        let marks = [
            (10.0, 12.0, 1u32),
            (10.4, 12.0, 2),
            (10.9, 12.0, 3),
            (44.0, 12.0, 4),
            (44.2, 12.0, 5),
        ];
        assert_eq!(
            death_cluster(&marks, 60),
            Some((10, 3)),
            "x = 10.0, 10.4 and 10.9 are one tile-column cluster"
        );
        // Ties go to the leftmost obstacle, which is the one met first.
        assert_eq!(
            death_cluster(&[(5.0, 1.0, 1), (30.0, 1.0, 2)], 60),
            Some((5, 1))
        );
        // Outside the level, non-finite, or a level with no width: not attributable.
        assert_eq!(
            death_cluster(&[(-1.0, 1.0, 1), (99.0, 1.0, 2), (f32::NAN, 1.0, 3)], 60),
            None
        );
        assert_eq!(death_cluster(&[], 60), None);
        assert_eq!(death_cluster(&[(1.0, 1.0, 1)], 0), None);
        // The last column is inside the level; the strip and this agree about it.
        assert_eq!(death_cluster(&[(59.5, 1.0, 1)], 60), Some((59, 1)));
    }
}
