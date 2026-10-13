//! ASCII rendering of sim state.
//!
//! Glyph table (one character per tile, never more — terminals wrap otherwise):
//!
//! | glyph | meaning                                            |
//! |-------|----------------------------------------------------|
//! | `#`   | solid tile (also the invisible walls at the edges)  |
//! | `^`   | spike tile (not solid, lethal on overlap)           |
//! | `.`   | empty space                                         |
//! | `!`   | goal trigger box (drawn on top of empty tiles only) |
//! | `e`   | live walker                                         |
//! | `x`   | stomped/removed walker                              |
//! | `@`   | the player (drawn last, so it always wins)          |
//!
//! Overlay order matters and is fixed: goals, then dead walkers, then live
//! walkers, then the player. A walker standing in front of the goal must occlude
//! it, and the player must occlude everything.
//!
//! This module reads `&Sim` and nothing else. It performs no simulation.

use sim::level::{GLYPH_EMPTY, GLYPH_SOLID, GLYPH_SPIKE};
use sim::{Level, Sim};

/// Player glyph.
pub const GLYPH_PLAYER: char = '@';
/// Live walker glyph.
pub const GLYPH_WALKER_ALIVE: char = 'e';
/// Removed walker glyph.
pub const GLYPH_WALKER_DEAD: char = 'x';
/// Goal trigger glyph.
pub const GLYPH_GOAL: char = '!';

/// The full glyph table, for help text and legends.
pub const GLYPHS: [(char, &str); 7] = [
    (GLYPH_SOLID, "solid tile"),
    (GLYPH_SPIKE, "spike (lethal, not solid)"),
    (GLYPH_EMPTY, "empty space"),
    (GLYPH_GOAL, "goal flag"),
    (GLYPH_WALKER_ALIVE, "live walker enemy"),
    (GLYPH_WALKER_DEAD, "removed walker"),
    (GLYPH_PLAYER, "the player"),
];

/// A tile window into the level. Lets a small terminal show a large level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// First tile column shown.
    pub col0: i32,
    /// First tile row shown.
    pub row0: i32,
    /// Tile columns shown.
    pub cols: u16,
    /// Tile rows shown.
    pub rows: u16,
}

impl Viewport {
    /// The whole level.
    pub fn whole(level: &Level) -> Viewport {
        Viewport {
            col0: 0,
            row0: 0,
            cols: clamp_u16(level.width),
            rows: clamp_u16(level.height),
        }
    }

    /// A window centred on the player and clamped to the level bounds. When the
    /// level is narrower than the window, the camera parks at 0 so the level is
    /// not drawn off-centre for no reason.
    pub fn following(sim: &Sim, cols: u16, rows: u16) -> Viewport {
        let level = sim.level();
        let p = sim.player();
        let center_col = (p.x + p.w * 0.5).floor() as i32;
        let center_row = (p.y + p.h * 0.5).floor() as i32;
        Viewport {
            col0: camera(center_col, cols as i32, level.width),
            row0: camera(center_row, rows as i32, level.height),
            cols,
            rows,
        }
    }
}

fn clamp_u16(v: i32) -> u16 {
    v.clamp(0, u16::MAX as i32) as u16
}

fn camera(center: i32, window: i32, extent: i32) -> i32 {
    if window >= extent {
        return 0;
    }
    (center - window / 2).clamp(0, extent - window)
}

/// One frame of the whole level.
pub fn frame(sim: &Sim) -> String {
    frame_in(sim, Viewport::whole(sim.level()))
}

/// One frame, restricted to `view`.
pub fn frame_in(sim: &Sim, view: Viewport) -> String {
    let grid = render_grid(sim);
    let height = grid.len() as i32;
    let level_width = sim.level().width.max(0);
    let col0 = view.col0.max(0);
    let col_end = col0
        .saturating_add(view.cols as i32)
        .min(level_width)
        .max(col0);

    let mut lines: Vec<String> = Vec::with_capacity(view.rows as usize);
    for step in 0..view.rows as i32 {
        let ty = view.row0.saturating_add(step);
        if ty < 0 || ty >= height {
            continue;
        }
        let line: String = if col0 >= col_end {
            String::new()
        } else {
            grid[ty as usize][col0 as usize..col_end as usize]
                .iter()
                .collect()
        };
        lines.push(line);
    }
    lines.join("\n")
}

/// Build the character grid with every entity overlaid.
fn render_grid(sim: &Sim) -> Vec<Vec<char>> {
    let level = sim.level();
    let width = level.width.max(0) as usize;
    let mut grid: Vec<Vec<char>> = level
        .to_glyph_rows()
        .into_iter()
        .map(|row| row.chars().collect())
        .collect();
    // Defensive only: `to_glyph_rows` is already `height` rows of `width` chars.
    for row in grid.iter_mut() {
        row.resize(width, GLYPH_EMPTY);
    }

    // 1. goals, over empty tiles only, so a goal never erases terrain.
    for goal in &level.goals {
        let tx0 = goal.x.floor() as i32;
        let tx1 = (goal.right() - 1.0e-3).floor() as i32;
        let ty0 = goal.y.floor() as i32;
        let ty1 = (goal.bottom() - 1.0e-3).floor() as i32;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                put(&mut grid, tx, ty, GLYPH_GOAL, true);
            }
        }
    }

    // 2. walkers: dead ones first, so a live walker standing on a corpse wins.
    for walker in sim.walkers().iter().filter(|w| !w.alive) {
        let (cx, cy) = (walker.x + walker.w * 0.5, walker.y + walker.h * 0.5);
        put(&mut grid, Level::tile_x(cx), Level::tile_y(cy), GLYPH_WALKER_DEAD, false);
    }
    for walker in sim.walkers().iter().filter(|w| w.alive) {
        let (cx, cy) = (walker.x + walker.w * 0.5, walker.y + walker.h * 0.5);
        put(
            &mut grid,
            Level::tile_x(cx),
            Level::tile_y(cy),
            GLYPH_WALKER_ALIVE,
            false,
        );
    }

    // 3. the player, drawn last so it is never hidden.
    let p = sim.player();
    let (px, py) = (p.x + p.w * 0.5, p.y + p.h * 0.5);
    put(&mut grid, Level::tile_x(px), Level::tile_y(py), GLYPH_PLAYER, false);

    grid
}

fn put(grid: &mut [Vec<char>], tx: i32, ty: i32, glyph: char, only_over_empty: bool) {
    if tx < 0 || ty < 0 {
        return;
    }
    let (tx, ty) = (tx as usize, ty as usize);
    if ty >= grid.len() || tx >= grid[ty].len() {
        return;
    }
    if only_over_empty && grid[ty][tx] != GLYPH_EMPTY {
        return;
    }
    grid[ty][tx] = glyph;
}

/// Status line(s) for the top of the screen.
///
/// Returns lines joined by `\n` with **no** trailing newline.
pub fn header(sim: &Sim) -> String {
    let level = sim.level();
    let stats = sim.stats();
    let alive = sim.walkers().iter().filter(|w| w.alive).count();
    let total = sim.walkers().len();
    let progress_pct = if level.width_pixels() > 0.0 {
        100.0 * stats.progress_pixels / level.width_pixels()
    } else {
        0.0
    };

    let mut lines = Vec::with_capacity(2);
    lines.push(format!(
        "{name}  frame {frame}/{cap}  {outcome}  x {x:.2}  progress {px:.0}px ({pct:.0}%)  stomps {stomps}  walkers {alive}/{total}  state {hash:016x}",
        name = level.name.as_str(),
        frame = stats.frames,
        cap = sim.max_frames(),
        outcome = sim.outcome().as_str(),
        x = stats.best_x,
        px = stats.progress_pixels,
        pct = progress_pct,
        stomps = stats.stomps,
        alive = alive,
        total = total,
        hash = sim.state_hash(),
    ));

    match stats.death_pos {
        Some((x, y)) => lines.push(format!("death marker at x {x:.2} y {y:.2}")),
        None => lines.push(format!(
            "alive  vx {:+.3}  vy {:+.3}  {}",
            sim.player().vx,
            sim.player().vy,
            if sim.player().grounded { "grounded" } else { "airborne" }
        )),
    }
    lines.join("\n")
}

/// Glyph legend, for `train-cli` help output and the in-game help overlay.
pub fn legend() -> String {
    let mut out = String::from("glyphs:");
    for (glyph, meaning) in GLYPHS {
        out.push_str(&format!("\n  '{glyph}'  {meaning}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_parks_at_zero_when_the_level_fits() {
        assert_eq!(camera(5, 80, 60), 0);
        assert_eq!(camera(5, 60, 60), 0);
    }

    #[test]
    fn camera_clamps_to_the_level_bounds() {
        // Level 60 wide, window 20 wide: legal offsets are 0..=40.
        assert_eq!(camera(0, 20, 60), 0);
        assert_eq!(camera(100, 20, 60), 40);
        assert_eq!(camera(30, 20, 60), 20);
    }

    #[test]
    fn put_respects_bounds_and_the_empty_only_flag() {
        let mut grid = vec![vec!['.'; 3]; 2];
        put(&mut grid, 1, 1, 'e', true);
        assert_eq!(grid[1][1], 'e');
        put(&mut grid, 1, 1, '@', true);
        assert_eq!(grid[1][1], 'e', "empty-only put must not overwrite");
        put(&mut grid, 1, 1, '@', false);
        assert_eq!(grid[1][1], '@', "forced put overwrites");
        put(&mut grid, 9, 9, '@', false);
        put(&mut grid, -1, 0, '@', false);
        assert_eq!(grid[0][0], '.');
    }

    #[test]
    fn clamp_u16_never_wraps() {
        assert_eq!(clamp_u16(-3), 0);
        assert_eq!(clamp_u16(60), 60);
        assert_eq!(clamp_u16(i32::MAX), u16::MAX);
    }
}
