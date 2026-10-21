//! ASCII rendering of a **scene**.
//!
//! A [`Scene`] is a level plus zero or more [`Body`]s. Rendering is a pure
//! function of those two things, so the same routine draws a live [`Sim`], a
//! replayed historical position, or eight agents racing the same level at once.
//! That single-routine property is the reason this module takes a scene rather
//! than a `&Sim`: once watch mode renders several agents side by side, two
//! separate drawing paths would inevitably disagree about what a body looks like
//! or which tile it is standing on.
//!
//! This module reads `&Sim` (through [`Scene::from_sim`]) and nothing else. It
//! performs no simulation and mutates no sim state.
//!
//! ## Glyph table
//!
//! One character per tile, never more — terminals wrap otherwise.
//!
//! | glyph | meaning                                             |
//! |-------|-----------------------------------------------------|
//! | `#`   | solid tile (also the invisible walls at the edges)  |
//! | `S`   | sand tile: solid exactly as `#` is, another material |
//! | `B`   | brick tile: solid, drawn as mortared masonry         |
//! | `W`   | plank tile: solid, drawn as milled timber            |
//! | `d`   | cracked soil: solid, and it breaks away underfoot    |
//! | `s`   | cracked sand: the same, on a beach                   |
//! | `b`   | cracked brick: the same, in masonry                  |
//! | `w`   | cracked timber: the same, in milled decking          |
//! | `^`   | spike tile (not solid, lethal on overlap)            |
//! | `v`   | spring pad (not solid, throws a falling body upward) |
//! | `.`   | empty space                                          |
//! | `!`   | goal trigger box (drawn on top of empty tiles only)  |
//! | `e`   | live walker                                          |
//! | `x`   | stomped/removed walker                               |
//! | `@`   | the primary body (drawn last, so it always wins)     |
//!
//! The last handful arrived with the traps: a **cracked** tile is the lowercase of the
//! material it falls apart from (`S`→`s`, `B`→`b`, `W`→`w`, and `#`→`d` for *dirt*, the one
//! material with no letter to fold), and it is drawn exactly as its material is until a body has
//! stood on it. The **spring** is the spike inverted — the one glyph pair here whose shapes say
//! what they do, `^` ending an episode and `v` throwing a body back up it.
//!
//! [`GLYPHS`] is the legend for those defaults, **not a whitelist**: watch mode
//! gives each agent its own letter, and any printable non-space ASCII character
//! is drawn happily. Only a character that would break the one-tile-one-character
//! rule (whitespace, control characters, anything non-ASCII) is replaced, with
//! [`GLYPH_UNKNOWN`].
//!
//! ## Overlay order
//!
//! Fixed, and asserted by tests:
//!
//! 1. goals, over empty tiles only, so a goal never erases terrain;
//! 2. dead bodies, in scene order, so a live body standing on a corpse wins;
//! 3. live bodies, in scene order, **except** the primary;
//! 4. the primary — the lowest-index live body — last, so the leading agent is
//!    never hidden.
//!
//! Within a tile the last body drawn wins. Scene order therefore *is* the
//! z-order, and [`Scene::from_sim`] places the player at index 0 so a single-agent
//! view is unchanged from before this module took scenes.

use sim::level::{
    GLYPH_BRICK, GLYPH_CRACKED, GLYPH_CRACKED_BRICK, GLYPH_CRACKED_PLANK, GLYPH_CRACKED_SAND,
    GLYPH_EMPTY, GLYPH_LAVA, GLYPH_PLANK, GLYPH_SAND, GLYPH_SOLID, GLYPH_SPIKE, GLYPH_SPRING,
    GLYPH_VINE, GLYPH_WATER,
};
use sim::{Level, Sim};

/// Player glyph.
pub const GLYPH_PLAYER: char = '@';
/// Live walker glyph.
pub const GLYPH_WALKER_ALIVE: char = 'e';
/// Removed walker glyph.
pub const GLYPH_WALKER_DEAD: char = 'x';
/// Live chaser glyph. A different letter from a patroller's, because the two ask for
/// different answers and a terminal frame is where a reader checks its own reasoning.
pub const GLYPH_CHASER_ALIVE: char = 'h';
/// Removed chaser glyph.
pub const GLYPH_CHASER_DEAD: char = 'x';
/// Flyer glyph.
pub const GLYPH_FLYER: char = 'F';
/// Turret glyph.
pub const GLYPH_TURRET: char = 'T';
/// A shot in flight.
pub const GLYPH_SHOT: char = 'o';
/// A moving platform.
pub const GLYPH_MOVER: char = 'M';
/// A rope's line, one glyph per cell it hangs through.
///
/// A vertical bar rather than a letter: every other object in this view is a body with a name,
/// and a rope is the one thing in a level that is a *line* — the drawing that says "a cell you can
/// hang in" is the one a person would draw.
pub const GLYPH_ROPE: char = '|';
/// A ball of lava in flight.
///
/// A star rather than a letter, for the same reason a shot is `o`: it is the one body in the
/// frame that is not a creature, and a terminal frame is where a reader checks its own reading
/// of a level. Water, lava and vines are *tiles*, so they carry their own glyphs (`~`, `L`, `V`)
/// and need no body of their own.
pub const GLYPH_LAVA_BALL: char = '*';
/// Goal trigger glyph.
pub const GLYPH_GOAL: char = '!';
/// Substitute for a glyph that cannot be drawn in one cell.
pub const GLYPH_UNKNOWN: char = '?';

/// The full glyph table, for help text and legends.
///
/// A legend, not a whitelist — see the module docs. Watch mode draws each agent
/// with its own letter and those letters are not listed here.
pub const GLYPHS: [(char, &str); 27] = [
    (GLYPH_SOLID, "solid tile"),
    (GLYPH_SAND, "sand tile (solid, a beach)"),
    (GLYPH_BRICK, "stone brick tile (solid masonry)"),
    (GLYPH_PLANK, "wood plank tile (solid timber)"),
    // The brittle four sit with the materials they are, because that is the whole of what they
    // are: the table keeps one line per material and then one per material's cracked self, so a
    // reader looking up `s` finds it where `S` is rather than at the end of the list.
    (
        GLYPH_CRACKED,
        "cracked ground (solid, breaks away underfoot)",
    ),
    (
        GLYPH_CRACKED_SAND,
        "cracked sand (solid, breaks away underfoot)",
    ),
    (
        GLYPH_CRACKED_BRICK,
        "cracked brick (solid, breaks away underfoot)",
    ),
    (
        GLYPH_CRACKED_PLANK,
        "cracked timber (solid, breaks away underfoot)",
    ),
    (
        sim::level::GLYPH_ONEWAY,
        "one-way platform (ground from above, air from below)",
    ),
    (GLYPH_SPIKE, "spike (lethal, not solid)"),
    (
        GLYPH_SPRING,
        "spring pad (not solid, throws a falling body up)",
    ),
    (
        GLYPH_WATER,
        "water (not solid: a body sinks in it and swims out)",
    ),
    (
        GLYPH_LAVA,
        "lava (not solid, lethal, and a vent when open to the air)",
    ),
    (GLYPH_VINE, "vine (not solid, climbable)"),
    (GLYPH_EMPTY, "empty space"),
    (GLYPH_GOAL, "goal flag"),
    (GLYPH_WALKER_ALIVE, "live walker enemy"),
    (GLYPH_WALKER_DEAD, "removed walker"),
    (GLYPH_CHASER_ALIVE, "live chaser (hunts the player)"),
    (GLYPH_CHASER_DEAD, "removed chaser"),
    (GLYPH_FLYER, "flyer (holds its height, beats across a span)"),
    (GLYPH_TURRET, "turret (fires along its row)"),
    (GLYPH_SHOT, "a turret's shot in flight"),
    (GLYPH_LAVA_BALL, "a lava ball in flight"),
    (GLYPH_MOVER, "a moving platform"),
    (GLYPH_ROPE, "a rope (grab it in the air and swing)"),
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

    /// A window centred on an arbitrary body rather than a sim's player, so watch
    /// mode can follow one of several simultaneously drawn agents.
    ///
    /// Still clamped to the level, and still parks at 0 when the level fits.
    pub fn following_body(
        level: &Level,
        center_x: f32,
        center_y: f32,
        cols: u16,
        rows: u16,
    ) -> Viewport {
        Viewport {
            col0: camera(center_x.floor() as i32, cols as i32, level.width),
            row0: camera(center_y.floor() as i32, rows as i32, level.height),
            cols,
            rows,
        }
    }

    /// The first visible tile column, never negative.
    pub fn vis_col0(&self) -> i32 {
        self.col0.max(0)
    }

    /// The first visible tile row, never negative.
    pub fn vis_row0(&self) -> i32 {
        self.row0.max(0)
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

/// One body to draw, with the glyph that identifies it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// World-space top-left corner, in tiles.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub glyph: char,
    /// Draw order only — a dead body is still drawn (as a corpse marker), it just
    /// goes underneath the living.
    pub alive: bool,
}

impl Body {
    /// Build a body, clamping a glyph that could not be drawn in one cell.
    ///
    /// The fields are public so a caller can bypass this; [`render_scene`]
    /// sanitizes again at draw time, so the one-character-per-tile rule holds
    /// however a `Body` was constructed.
    pub fn new(x: f32, y: f32, w: f32, h: f32, glyph: char, alive: bool) -> Body {
        Body {
            x,
            y,
            w: w.max(0.0),
            h: h.max(0.0),
            glyph: sanitize_glyph(glyph),
            alive,
        }
    }

    /// A body from an AABB in the shape `sim` hands out.
    pub fn from_rect(rect: sim::Rect, glyph: char, alive: bool) -> Body {
        Body::new(rect.x, rect.y, rect.w, rect.h, glyph, alive)
    }

    /// True when the body can be drawn at all.
    ///
    /// A non-finite coordinate is a bug upstream; it is skipped rather than
    /// silently drawn at the origin, where it would look like a real agent.
    pub fn is_drawable(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.w.is_finite() && self.h.is_finite()
    }

    /// Inclusive tile bounds, `(tx0, tx1, ty0, ty1)`, or `None` when undrawable.
    pub fn tile_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        if !self.is_drawable() {
            return None;
        }
        let (tx0, tx1) = tile_span(self.x, self.w);
        let (ty0, ty1) = tile_span(self.y, self.h);
        Some((tx0, tx1, ty0, ty1))
    }
}

/// The level's static tiles, its goals, and any number of bodies.
#[derive(Clone, Debug)]
pub struct Scene<'a> {
    pub level: &'a Level,
    /// Draw order is z-order: later bodies are drawn on top. The primary agent
    /// (the one that must never be hidden) is the lowest-index live body.
    pub bodies: Vec<Body>,
}

impl<'a> Scene<'a> {
    pub fn new(level: &'a Level, bodies: Vec<Body>) -> Scene<'a> {
        Scene { level, bodies }
    }

    /// The single-agent view: the player as `@`, live walkers `e`, dead walkers `x`.
    ///
    /// The player goes at index 0 so it is the primary body and therefore drawn
    /// last, exactly as this module drew it before it took scenes.
    pub fn from_sim(sim: &Sim) -> Scene<'_> {
        let mut bodies = Vec::with_capacity(
            sim.walkers().len() + sim.flyers().len() + sim.turrets().len() + sim.shots().len() + 6,
        );
        let p = sim.player();
        bodies.push(Body::from_rect(p.aabb(), GLYPH_PLAYER, true));
        for w in sim.walkers() {
            let (live, dead) = match w.mode {
                sim::WalkerMode::Chase => (GLYPH_CHASER_ALIVE, GLYPH_CHASER_DEAD),
                sim::WalkerMode::Patrol => (GLYPH_WALKER_ALIVE, GLYPH_WALKER_DEAD),
            };
            bodies.push(Body::from_rect(
                w.aabb(&sim::PHYSICS),
                if w.alive { live } else { dead },
                w.alive,
            ));
        }
        // The air and the floor of the new mechanics: a flyer and a turret are bodies, so
        // they take a cell; a shot is one too; and a mover is drawn as the shelf it is.
        for f in sim.flyers() {
            bodies.push(Body::from_rect(f.aabb(), GLYPH_FLYER, f.alive));
        }
        for t in sim.turrets() {
            bodies.push(Body::from_rect(t.aabb(), GLYPH_TURRET, t.alive));
        }
        for s in sim.shots().iter().filter(|s| s.alive) {
            bodies.push(Body::from_rect(s.aabb(), GLYPH_SHOT, true));
        }
        // A lava ball is a body in the air, so it takes a cell like the shot it shares a
        // pass with: a scene that left them out would draw a level whose fountains are
        // invisible and whose deaths are unexplained.
        for b in sim.lava_balls().iter().filter(|b| b.alive) {
            bodies.push(Body::from_rect(b.aabb(), GLYPH_LAVA_BALL, true));
        }
        for shelf in sim.platforms() {
            bodies.push(Body::new(
                shelf.x,
                shelf.y,
                shelf.w,
                shelf.h,
                GLYPH_MOVER,
                true,
            ));
        }
        // A rope is drawn as the cells it hangs through, one glyph each, because that is what a
        // terminal can say about a line. It is not a body (nothing collides with it) but it takes
        // cells, and a scene that left it out would show a swing the reader could not explain.
        for rope in &sim.level().ropes {
            for i in 0..rope.length.max(1.0) as i32 {
                bodies.push(Body::new(
                    rope.col as f32,
                    (rope.row + i) as f32,
                    1.0,
                    1.0,
                    GLYPH_ROPE,
                    true,
                ));
            }
        }
        Scene::new(sim.level(), bodies)
    }

    /// Index of the body that must survive every overlap: the lowest-index live
    /// one.
    pub fn primary(&self) -> Option<usize> {
        self.bodies.iter().position(|b| b.alive)
    }
}

/// One frame of the whole level.
pub fn frame(sim: &Sim) -> String {
    frame_in(sim, Viewport::whole(sim.level()))
}

/// One frame of a sim, restricted to `view`.
///
/// A thin wrapper over [`render_scene`], so there is exactly one drawing routine.
pub fn frame_in(sim: &Sim, view: Viewport) -> String {
    render_scene(&Scene::from_sim(sim), view)
}

/// Render a scene through a viewport.
///
/// Rows are joined by `\n` with **no** trailing newline. Rows outside the level
/// are omitted rather than padded, so a viewport taller than the level yields
/// exactly the level's rows.
pub fn render_scene(scene: &Scene, view: Viewport) -> String {
    let grid = build_grid(scene, view);
    let height = grid.len() as i32;
    let level_width = scene.level.width.max(0);
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
        let row = &grid[ty as usize];
        let end = col_end.min(row.len() as i32).max(col0);
        let line: String = if col0 >= end {
            String::new()
        } else {
            row[col0 as usize..end as usize].iter().collect()
        };
        lines.push(line);
    }
    lines.join("\n")
}

/// Build the character grid with goals and every body overlaid.
///
/// Bodies outside `view` are culled before any tile is touched, so this costs
/// O(bodies x tiles-covered) and never walks the level area once per body.
fn build_grid(scene: &Scene, view: Viewport) -> Vec<Vec<char>> {
    let level = scene.level;
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
        let (tx0, tx1) = tile_span(goal.x, goal.w);
        let (ty0, ty1) = tile_span(goal.y, goal.h);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                put(&mut grid, tx, ty, GLYPH_GOAL, true);
            }
        }
    }

    let visible = VisibleTiles::of(view);
    // 2. dead bodies first, so a live body standing on a corpse wins.
    for body in scene.bodies.iter().filter(|b| !b.alive) {
        draw_body(&mut grid, body, &visible);
    }
    // 3. live bodies, in scene order, except the primary.
    let primary = scene.primary();
    for (index, body) in scene.bodies.iter().enumerate() {
        if body.alive && Some(index) != primary {
            draw_body(&mut grid, body, &visible);
        }
    }
    // 4. the primary last, so the leading agent is never hidden.
    if let Some(index) = primary {
        draw_body(&mut grid, &scene.bodies[index], &visible);
    }

    grid
}

fn draw_body(grid: &mut [Vec<char>], body: &Body, visible: &VisibleTiles) {
    let Some((tx0, tx1, ty0, ty1)) = body.tile_bounds() else {
        return;
    };
    if !visible.intersects(tx0, tx1, ty0, ty1) {
        return;
    }
    let glyph = sanitize_glyph(body.glyph);
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            put(grid, tx, ty, glyph, false);
        }
    }
}

/// The tile rectangle a viewport actually shows, for cheap body culling.
struct VisibleTiles {
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
}

impl VisibleTiles {
    fn of(view: Viewport) -> VisibleTiles {
        let x0 = view.col0.max(0);
        let y0 = view.row0.max(0);
        VisibleTiles {
            x0,
            x1: x0.saturating_add(view.cols as i32).saturating_sub(1),
            y0,
            y1: y0.saturating_add(view.rows as i32).saturating_sub(1),
        }
    }

    fn intersects(&self, tx0: i32, tx1: i32, ty0: i32, ty1: i32) -> bool {
        tx1 >= self.x0 && tx0 <= self.x1 && ty1 >= self.y0 && ty0 <= self.y1
    }
}

/// Inclusive tile index range covered by `[min, min + size)`.
///
/// Delegates to the sim's own collision helper rather than maintaining a second
/// opinion: a body drawn one tile wider than the collision code thinks it is would
/// hide a physics bug instead of making it obvious. Sim only ever passes real body
/// sizes, but a `Scene` can be assembled by any tool, so the degenerate cases are
/// guarded here rather than there.
#[inline]
fn tile_span(min: f32, size: f32) -> (i32, i32) {
    if !min.is_finite() {
        return (0, 0);
    }
    let lo = min.floor() as i32;
    if !size.is_finite() || size <= 0.0 {
        // A degenerate span would otherwise invert (lo > hi) and draw nothing.
        return (lo, lo);
    }
    sim::physics::tile_span(min, size)
}

/// Force a glyph into something that occupies exactly one terminal cell.
fn sanitize_glyph(glyph: char) -> char {
    if glyph.is_ascii_graphic() {
        glyph
    } else {
        GLYPH_UNKNOWN
    }
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
    // The fraction of the *way to the flag*, not of the level's width. The objective is the
    // distance closed on the goal, so a tower has to read 0 → 100 the same way a corridor
    // does; dividing by the width made every vertical level look like it was at 0% while the
    // body was climbing it. A spawn on top of the flag has nothing to close, hence 100.
    let progress_pct = if stats.goal_distance_pixels > 0.0 {
        100.0 * stats.progress_pixels / stats.goal_distance_pixels
    } else {
        100.0
    };

    let mut lines = Vec::with_capacity(2);
    lines.push(format!(
        "{name}  frame {frame}/{cap}  {outcome}  x {x:.2}  progress {px:.0}px ({pct:.0}%)  stomps {stomps}  jumps {jumps}  zones {checkpoints}  walkers {alive}/{total}  state {hash:016x}",
        name = level.name.as_str(),
        frame = stats.frames,
        cap = sim.max_frames(),
        outcome = sim.outcome().as_str(),
        x = stats.best_x,
        px = stats.progress_pixels,
        pct = progress_pct,
        stomps = stats.stomps,
        // Reported next to the stomps because it is the statistic that makes "this agent
        // bounces everywhere" visible rather than a feeling, and because `evo`'s fitness
        // charges for it (fitness v2).
        jumps = stats.jumps,
        // Safe landing zones reached past a hazard. The companion to `jumps` on a hazard
        // level: `jumps` is the body trying, this is what trying bought, and the two
        // disagreeing (many jumps, no zones) is the shape of an agent bouncing into the
        // same spike over and over — which is exactly what the owner's report described.
        checkpoints = stats.checkpoints,
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
            if sim.player().grounded {
                "grounded"
            } else {
                "airborne"
            }
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

    /// Build a level from ASCII art. Escaped-quote JSON rather than a raw string:
    /// a run of `#` inside `r#"..."#` collides with the raw-string delimiter.
    fn fixture(rows: &[&str], spawn: (i32, i32), entities: &[(&str, i32, i32)]) -> Level {
        let width = rows[0].len();
        assert!(
            rows.iter().all(|r| r.len() == width),
            "ragged test fixture rows"
        );
        let tiles = rows
            .iter()
            .map(|r| format!("    \"{r}\""))
            .collect::<Vec<_>>()
            .join(",\n");
        let ents = entities
            .iter()
            .map(|(k, x, y)| format!("{{\"type\":\"{k}\",\"x\":{x},\"y\":{y}}}"))
            .collect::<Vec<_>>()
            .join(",");
        let src = format!(
            "{{\"format\":\"evo-platformer-level\",\"level_format_version\":1,\"name\":\"fixture\",\
             \"width\":{width},\"height\":{},\"spawn\":{{\"x\":{},\"y\":{}}},\
             \"tiles\":[\n{tiles}\n],\"entities\":[{ents}]}}",
            rows.len(),
            spawn.0,
            spawn.1
        );
        Level::from_json_str(&src).expect("fixture parses")
    }

    /// Flat 12x5 room, ground in row 4, spawn in row 3, goal at column 10.
    fn flat() -> Level {
        fixture(
            &[
                "............",
                "............",
                "............",
                "............",
                "############",
            ],
            (1, 3),
            &[("goal", 10, 3)],
        )
    }

    /// Flat room with a wall in column 5, rows 1..=3.
    fn walled() -> Level {
        fixture(
            &[
                "............",
                ".....#......",
                ".....#......",
                ".....#......",
                "############",
            ],
            (1, 3),
            &[("goal", 10, 3)],
        )
    }

    fn rows_of(frame: &str) -> Vec<String> {
        frame.split('\n').map(|s| s.to_string()).collect()
    }

    /// Body standing on the ground of the fixtures: bottom flush with row 4.
    fn standing(x: f32, glyph: char, alive: bool) -> Body {
        Body::new(x, 3.1, 0.6, 0.9, glyph, alive)
    }

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

    #[test]
    fn tile_span_matches_the_sim_epsilon_convention() {
        // Same cases sim's own collision helper is pinned to.
        assert_eq!(tile_span(5.0, 0.6), (5, 5));
        assert_eq!(tile_span(5.4, 0.6), (5, 5));
        assert_eq!(tile_span(5.4001, 0.6), (5, 6));
        assert_eq!(tile_span(-0.6, 0.6), (-1, -1));
        // Degenerate and non-finite spans stay well formed instead of inverting.
        assert_eq!(tile_span(3.0, 0.0), (3, 3));
        assert_eq!(tile_span(3.0, -1.0), (3, 3));
        assert_eq!(tile_span(f32::NAN, 0.6), (0, 0));
    }

    #[test]
    fn a_scene_draws_each_bodys_glyph_in_its_own_column() {
        let level = flat();
        let scene = Scene::new(
            &level,
            vec![
                standing(1.2, '@', true),
                standing(4.2, 'a', true),
                standing(7.2, 'b', true),
            ],
        );
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows.len(), 5, "one line per in-range row");
        // Row 3 is the row a body standing on row 4 occupies.
        assert_eq!(rows[3].chars().nth(1), Some('@'));
        assert_eq!(rows[3].chars().nth(4), Some('a'));
        assert_eq!(rows[3].chars().nth(7), Some('b'));
        // Ground and goal survive.
        assert_eq!(rows[4], "############");
        assert_eq!(rows[3].chars().nth(10), Some(GLYPH_GOAL));
    }

    #[test]
    fn the_primary_body_is_drawn_last_and_wins_a_shared_tile() {
        let level = flat();
        // Both in column 1: the lowest-index live body must end up visible.
        let scene = Scene::new(
            &level,
            vec![standing(1.2, '@', true), standing(1.2, 'a', true)],
        );
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(
            rows[3].chars().nth(1),
            Some('@'),
            "the primary body must not be hidden by a later live body in the same tile"
        );

        // A live body beats a corpse in the same tile, whichever order they are in.
        let scene = Scene::new(
            &level,
            vec![standing(1.2, 'x', false), standing(1.2, 'e', true)],
        );
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows[3].chars().nth(1), Some('e'));

        let scene = Scene::new(
            &level,
            vec![standing(1.2, 'e', true), standing(1.2, 'x', false)],
        );
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows[3].chars().nth(1), Some('e'));
    }

    #[test]
    fn the_primary_is_the_lowest_index_live_body_even_past_a_corpse() {
        let level = flat();
        let scene = Scene::new(
            &level,
            vec![
                standing(1.2, 'x', false),
                standing(1.2, 'a', true),
                standing(1.2, 'b', true),
            ],
        );
        assert_eq!(scene.primary(), Some(1));
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows[3].chars().nth(1), Some('a'));
    }

    #[test]
    fn bodies_outside_the_viewport_are_clipped_without_panic() {
        let level = flat();
        let baseline = render_scene(&Scene::new(&level, vec![]), Viewport::whole(&level));

        let scene = Scene::new(
            &level,
            vec![
                Body::new(-50.0, 3.1, 0.6, 0.9, 'L', true),
                Body::new(999.0, 3.1, 0.6, 0.9, 'R', true),
                Body::new(-0.0, -80.0, 0.6, 0.9, 'U', true),
                Body::new(0.0, 900.0, 0.6, 0.9, 'D', true),
            ],
        );
        let frame = render_scene(&scene, Viewport::whole(&level));
        assert_eq!(
            frame, baseline,
            "a body entirely off-screen must leave the frame untouched"
        );

        // A body straddling the left edge is clipped, not dropped and not wrapped.
        let scene = Scene::new(&level, vec![Body::new(-0.4, 3.1, 0.9, 0.9, 'C', true)]);
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows[3].chars().next(), Some('C'));

        // A viewport scrolled past the level yields no rows, and no panic.
        let scrolled = render_scene(
            &scene,
            Viewport {
                col0: 500,
                row0: 500,
                cols: 4,
                rows: 4,
            },
        );
        assert_eq!(scrolled.matches('\n').count() + 1, 1);
        assert!(scrolled.is_empty() || scrolled.chars().all(|c| c == '\n'));
    }

    #[test]
    fn a_body_flush_against_a_wall_is_not_drawn_inside_it() {
        let level = walled();
        // Right edge exactly on the wall's left face at x = 5.0.
        let scene = Scene::new(&level, vec![Body::new(4.4, 3.1, 0.6, 0.9, '@', true)]);
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows[3].chars().nth(4), Some('@'), "body occupies column 4");
        assert_eq!(
            rows[3].chars().nth(5),
            Some(GLYPH_SOLID),
            "a body flush against a wall must not be drawn inside it"
        );
        assert_eq!(rows[2].chars().nth(5), Some(GLYPH_SOLID));
    }

    #[test]
    fn a_body_that_straddles_a_tile_boundary_covers_both_tiles() {
        // Bodies cover every tile their AABB overlaps, because that is exactly what
        // the collision code tests against. A body whose span crosses a boundary
        // must therefore look two tiles wide, or the picture would lie about what
        // could be hit.
        let level = flat();
        let scene = Scene::new(&level, vec![Body::new(4.5, 3.1, 0.6, 0.9, '@', true)]);
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows[3].chars().nth(4), Some('@'));
        assert_eq!(rows[3].chars().nth(5), Some('@'));
    }

    #[test]
    fn a_goal_never_erases_terrain() {
        // The flag stands with its pole's foot in the floor row, so the two cells its marker
        // covers are the floor itself and the air above it. Goals are drawn over *empty*
        // tiles only, so the floor keeps its glyph and the flag appears above it.
        //
        // This fixture used to carry a `G` tile *and* a goal entity, which is two goals — a
        // level has one — so the sugar and the entity are the same case now, and the overlap
        // with the terrain is what the test is actually about.
        let level = fixture(&["......", "......", "######"], (1, 1), &[("goal", 2, 2)]);
        let rows = rows_of(&render_scene(
            &Scene::new(&level, vec![]),
            Viewport::whole(&level),
        ));
        assert_eq!(rows[2], "######", "the floor under the flag survives");
        assert!(
            rows[1].contains(GLYPH_GOAL),
            "and the flag is drawn in the air above it: {:?}",
            rows[1]
        );
    }

    #[test]
    fn glyphs_outside_the_legend_are_still_drawn() {
        let level = flat();
        // Watch mode gives each agent its own letter; the legend is not a filter.
        for glyph in ['Q', 'z', '7', '+'] {
            let scene = Scene::new(&level, vec![standing(1.2, glyph, true)]);
            let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
            assert_eq!(rows[3].chars().nth(1), Some(glyph), "glyph {glyph:?}");
        }
    }

    #[test]
    fn a_glyph_that_would_wrap_a_line_is_replaced() {
        let level = flat();
        for bad in ['\n', '\t', ' ', '\u{1F600}', '\u{0}'] {
            // Both routes in must be covered: the constructor's clamp and the
            // draw-time sanitize that a struct literal would bypass.
            let sanitized = Body::new(1.2, 3.1, 0.6, 0.9, bad, true);
            assert_eq!(sanitized.glyph, GLYPH_UNKNOWN);

            let bypassed = Body {
                x: 1.2,
                y: 3.1,
                w: 0.6,
                h: 0.9,
                glyph: bad,
                alive: true,
            };
            let scene = Scene::new(&level, vec![bypassed]);
            let frame = render_scene(&scene, Viewport::whole(&level));
            assert_eq!(
                rows_of(&frame).len(),
                5,
                "a bad glyph must not add or lose a line"
            );
            assert_eq!(
                rows_of(&frame)[3].chars().nth(1),
                Some(GLYPH_UNKNOWN),
                "glyph {bad:?} must be replaced"
            );
        }
    }

    #[test]
    fn non_finite_bodies_are_skipped_rather_than_drawn_at_the_origin() {
        let level = flat();
        let baseline = render_scene(&Scene::new(&level, vec![]), Viewport::whole(&level));
        for nan_body in [
            Body::new(f32::NAN, 3.1, 0.6, 0.9, '@', true),
            Body::new(1.2, f32::INFINITY, 0.6, 0.9, '@', true),
            Body::new(f32::NEG_INFINITY, 3.1, f32::NAN, 0.9, '@', true),
        ] {
            assert!(!nan_body.is_drawable());
            assert_eq!(nan_body.tile_bounds(), None);
            let frame = render_scene(&Scene::new(&level, vec![nan_body]), Viewport::whole(&level));
            assert_eq!(
                frame, baseline,
                "an undrawable body must not appear at the origin"
            );
        }
    }

    #[test]
    fn a_zero_size_body_draws_one_tile_instead_of_inverting_its_span() {
        let level = flat();
        let scene = Scene::new(&level, vec![Body::new(3.0, 2.0, 0.0, 0.0, '@', true)]);
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows[2].chars().nth(3), Some('@'));
    }

    #[test]
    fn render_scene_from_sim_is_exactly_frame_in() {
        // The regression test for the "one drawing routine" property: the
        // single-agent path must not diverge from the scene path.
        let level = walled();
        let sim = Sim::new(&level, 7);
        for view in [
            Viewport::whole(&level),
            Viewport::following(&sim, 6, 3),
            Viewport {
                col0: 2,
                row0: 1,
                cols: 4,
                rows: 2,
            },
            Viewport {
                col0: 0,
                row0: 0,
                cols: 200,
                rows: 200,
            },
        ] {
            assert_eq!(
                render_scene(&Scene::from_sim(&sim), view),
                frame_in(&sim, view),
                "mismatch at view {view:?}"
            );
        }
    }

    /// **The map and the swing agree about where the line is.** The `|` glyphs must occupy
    /// exactly the cells between the rope's anchor and its end, or a reader is shown a line one
    /// tile away from the line a body actually grabs — which is what the first cut did, when the
    /// anchor was tied to the bottom of its own cell.
    #[test]
    fn a_rope_draws_every_cell_of_the_line_it_hangs_on() {
        let rows = [
            "............",
            "............",
            "####........",
            "............",
            "............",
            "............",
            "############",
        ];
        let tiles = rows
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect::<Vec<_>>()
            .join(",");
        let src = format!(
            "{{\"format\":\"evo-platformer-level\",\"level_format_version\":4,\"name\":\"rope\",\
             \"width\":12,\"height\":7,\"spawn\":{{\"x\":1,\"y\":5}},\"tiles\":[{tiles}],\
             \"entities\":[{{\"type\":\"rope\",\"x\":2,\"y\":3,\"tiles\":3}},\
             {{\"type\":\"goal\",\"x\":10,\"y\":5}}]}}"
        );
        let level = Level::from_json_str(&src).expect("the rope level parses");
        let rope = level.ropes[0];
        assert_eq!(rope.anchor(), (2.5, 3.0), "tied to the underside of row 2");
        assert_eq!(
            rope.end_y(),
            6.0,
            "three tiles of line, ending on the floor"
        );

        let frame = render_scene(
            &Scene::from_sim(&Sim::new(&level, 1)),
            Viewport::whole(&level),
        );
        let rows = rows_of(&frame);
        let drawn: Vec<i32> = (0..level.height)
            .filter(|row| rows[*row as usize].chars().nth(2) == Some(GLYPH_ROPE))
            .collect();
        assert_eq!(
            drawn,
            vec![3, 4, 5],
            "one glyph per cell of the line, from the ceiling down:\n{frame}"
        );
    }

    #[test]
    fn frame_and_frame_in_agree_on_the_whole_level() {
        let level = flat();
        let sim = Sim::new(&level, 3);
        assert_eq!(frame(&sim), frame_in(&sim, Viewport::whole(&level)));
        assert_eq!(rows_of(&frame(&sim)).len(), level.height as usize);
    }

    #[test]
    fn from_sim_puts_the_player_first_so_it_is_the_primary() {
        let level = fixture(
            &[
                "............",
                "............",
                "............",
                "############",
            ],
            (1, 2),
            &[("walker", 6, 2), ("goal", 10, 2)],
        );
        let sim = Sim::new(&level, 1);
        let scene = Scene::from_sim(&sim);
        assert_eq!(scene.bodies.len(), sim.walkers().len() + 1);
        assert_eq!(scene.bodies[0].glyph, GLYPH_PLAYER);
        assert_eq!(scene.primary(), Some(0));
        assert!(scene.bodies[1..]
            .iter()
            .all(|b| b.glyph == GLYPH_WALKER_ALIVE || b.glyph == GLYPH_WALKER_DEAD));
    }

    #[test]
    fn following_body_is_clamped_to_the_level() {
        let level = flat();
        let v = Viewport::following_body(&level, -100.0, -100.0, 4, 2);
        assert_eq!((v.col0, v.row0), (0, 0));
        let v = Viewport::following_body(&level, 999.0, 999.0, 4, 2);
        assert_eq!((v.col0, v.row0), (8, 3));
        // A window at least as large as the level parks at the origin.
        let v = Viewport::following_body(&level, 999.0, 999.0, 40, 40);
        assert_eq!((v.col0, v.row0), (0, 0));
        assert_eq!(v.vis_col0(), 0);
    }

    #[test]
    fn a_body_at_a_negative_tile_with_a_positive_span_draws_its_visible_part() {
        // Regression for the culling check: intersecting the view is the test, not
        // starting inside it, or a body half off the left edge would vanish.
        let level = flat();
        let scene = Scene::new(&level, vec![Body::new(-0.5, 3.1, 1.2, 0.9, 'L', true)]);
        let rows = rows_of(&render_scene(&scene, Viewport::whole(&level)));
        assert_eq!(rows[3].chars().next(), Some('L'));
    }
}
