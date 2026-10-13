//! Level format (§6): a human-writable character grid plus an entity list.
//!
//! ```jsonc
//! {
//!   "format": "evo-platformer-level",
//!   "level_format_version": 1,
//!   "name": "L1-flat-run",
//!   "width": 60, "height": 15,
//!   "spawn": {"x": 2, "y": 12},
//!   "tiles": ["...", /* one string per row, top row first */],
//!   "entities": [{"type": "walker", "x": 30, "y": 12}]
//! }
//! ```
//!
//! Coordinates are **tile coordinates**. An entity at tile `(x, y)` stands with
//! its bottom edge on the bottom edge of row `y`, i.e. at world
//! `y_world = (y + 1) * TILE`. So an entity at `y = 12` resting on ground tiles in
//! row `13` is expressed as `y = 12` — matching the plan's example level.

use serde::{Deserialize, Serialize};

use crate::hash::StateHasher;
use crate::physics::{PlayerState, PHYSICS};

use crate::PIXELS_PER_TILE;

pub const LEVEL_FORMAT: &str = "evo-platformer-level";
pub const LEVEL_FORMAT_VERSION: u32 = 1;

/// Smallest grid the editor will build. Not a loader rule — a level is legal from
/// 1x1 — but a painter with less room than this has nothing to paint with.
pub const MIN_LEVEL_W: i32 = 8;
pub const MIN_LEVEL_H: i32 = 5;

/// Grid characters. Anything else in a level file is a load error, not a shrug.
pub const GLYPH_EMPTY: char = '.';
pub const GLYPH_SOLID: char = '#';
pub const GLYPH_SPIKE: char = '^';
pub const GLYPH_GOAL: char = 'G';

/// Tile ids (§6): small integers so the grid stays cheap to clone and hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Tile {
    Empty = 0,
    /// Blocks movement from every direction.
    Solid = 1,
    /// Not solid, lethal on overlap.
    Spike = 2,
    /// Reserved glyph for a goal *tile*; normalized into an entity at load time.
    Goal = 3,
}

impl Tile {
    pub fn from_glyph(c: char) -> Option<Tile> {
        match c {
            GLYPH_EMPTY => Some(Tile::Empty),
            GLYPH_SOLID => Some(Tile::Solid),
            GLYPH_SPIKE => Some(Tile::Spike),
            GLYPH_GOAL => Some(Tile::Goal),
            _ => None,
        }
    }

    pub fn glyph(self) -> char {
        match self {
            Tile::Empty => GLYPH_EMPTY,
            Tile::Solid => GLYPH_SOLID,
            Tile::Spike => GLYPH_SPIKE,
            Tile::Goal => GLYPH_GOAL,
        }
    }

    pub fn is_solid(self) -> bool {
        matches!(self, Tile::Solid)
    }
}

/// Axis-aligned box in world units, top-left origin, y grows downward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    /// Strict overlap. Touching edges do not count.
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    fn hash_into(&self, h: &mut StateHasher) {
        h.f32(self.x);
        h.f32(self.y);
        h.f32(self.w);
        h.f32(self.h);
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct TilePos {
    x: i32,
    y: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RawEntity {
    #[serde(rename = "type")]
    kind: String,
    x: i32,
    y: i32,
}

/// The on-disk shape, in write order. Serializing this rather than hand-building
/// JSON is what keeps the writer and the loader from ever disagreeing about keys:
/// they are the same struct.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct RawLevel {
    format: String,
    level_format_version: u32,
    name: String,
    width: i32,
    height: i32,
    spawn: TilePos,
    tiles: Vec<String>,
    #[serde(default)]
    entities: Vec<RawEntity>,
}

/// What a [`Level::resize`] had to drop, so the editor can say so out loud
/// instead of quietly losing the user's work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResizeReport {
    pub dropped_walkers: usize,
    pub dropped_goals: usize,
    /// The spawn point fell outside the new grid and was pulled back in.
    pub spawn_moved: bool,
    /// Every goal fell outside, so one was clamped back in: the grid has moved out
    /// from under the goal rather than the goal being deleted.
    pub goal_moved: bool,
}

impl ResizeReport {
    pub fn dropped_anything(&self) -> bool {
        self.dropped_walkers > 0 || self.dropped_goals > 0
    }
}

/// A parsed, validated level. Immutable during an episode.
#[derive(Clone, Debug)]
pub struct Level {
    pub name: String,
    pub width: i32,
    pub height: i32,
    /// Row-major, `width * height` entries, row 0 = top.
    tiles: Vec<Tile>,
    /// Player spawn AABB, already converted to world units.
    pub spawn: Rect,
    /// Initial walker AABBs.
    pub walkers: Vec<Rect>,
    /// Goal trigger boxes.
    pub goals: Vec<Rect>,
}

impl Level {
    /// Parse and validate a level from JSON text.
    pub fn from_json_str(src: &str) -> Result<Level, String> {
        let raw: RawLevel =
            serde_json::from_str(src).map_err(|e| format!("invalid level JSON: {e}"))?;

        if raw.format != LEVEL_FORMAT {
            return Err(format!(
                "unexpected format {:?} (expected {:?})",
                raw.format, LEVEL_FORMAT
            ));
        }
        if raw.level_format_version != LEVEL_FORMAT_VERSION {
            return Err(format!(
                "level_format_version {} is not supported (this build reads {})",
                raw.level_format_version, LEVEL_FORMAT_VERSION
            ));
        }
        if raw.width <= 0 || raw.height <= 0 {
            return Err(format!(
                "level has non-positive size {}x{}",
                raw.width, raw.height
            ));
        }
        if raw.tiles.len() != raw.height as usize {
            return Err(format!(
                "level declares height {} but has {} tile rows",
                raw.height,
                raw.tiles.len()
            ));
        }

        let mut tiles = Vec::with_capacity((raw.width * raw.height) as usize);
        for (row, line) in raw.tiles.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            if chars.len() != raw.width as usize {
                return Err(format!(
                    "row {row} has {} columns, expected {}",
                    chars.len(),
                    raw.width
                ));
            }
            for (col, ch) in chars.into_iter().enumerate() {
                let tile = Tile::from_glyph(ch).ok_or_else(|| {
                    format!("row {row} column {col}: unknown tile glyph {ch:?}")
                })?;
                tiles.push(tile);
            }
        }

        let mut level = Level {
            name: raw.name,
            width: raw.width,
            height: raw.height,
            tiles,
            spawn: Rect::new(0.0, 0.0, PHYSICS.player_w, PHYSICS.player_h),
            walkers: Vec::new(),
            goals: Vec::new(),
        };

        // Spawn: bottom edge on the bottom of the spawn tile's row.
        let (sx, sy) = (raw.spawn.x, raw.spawn.y);
        level.spawn = Rect::new(
            sx as f32 + (1.0 - PHYSICS.player_w) * 0.5,
            crate::physics::bottom_of(sy) - PHYSICS.player_h,
            PHYSICS.player_w,
            PHYSICS.player_h,
        );

        // Entities: walkers and goals, in file order (order is part of the sim).
        for e in &raw.entities {
            match e.kind.as_str() {
                "walker" => level.walkers.push(Rect::new(
                    e.x as f32 + (1.0 - PHYSICS.walker_w) * 0.5,
                    crate::physics::bottom_of(e.y) - PHYSICS.walker_h,
                    PHYSICS.walker_w,
                    PHYSICS.walker_h,
                )),
                "goal" => level.goals.push(goal_rect(e.x, e.y)),
                other => return Err(format!("unknown entity type {other:?}")),
            }
        }

        // A `G` tile is sugar for a goal entity; normalize it so there is exactly
        // one code path for goal triggers.
        for ty in 0..level.height {
            for tx in 0..level.width {
                if level.get(tx, ty) == Tile::Goal {
                    level.goals.push(goal_rect(tx, ty));
                    level.set(tx, ty, Tile::Empty);
                }
            }
        }

        if level.goals.is_empty() {
            return Err("level has no goal (add a \"goal\" entity or a 'G' tile)".into());
        }
        Ok(level)
    }

    /// Load a level from a file path.
    pub fn from_json_file(path: impl AsRef<std::path::Path>) -> Result<Level, String> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Level::from_json_str(&src).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Tile at `(tx, ty)`.
    ///
    /// Out-of-bounds semantics are part of the contract:
    /// * left/right of the grid → [`Tile::Solid`] (invisible level walls),
    /// * above the grid → [`Tile::Empty`] (you may jump off the top of the map),
    /// * below the grid → [`Tile::Empty`] (falling out is a pit death).
    pub fn get(&self, tx: i32, ty: i32) -> Tile {
        if ty < 0 || ty >= self.height {
            return Tile::Empty;
        }
        if tx < 0 || tx >= self.width {
            return Tile::Solid;
        }
        self.tiles[(ty * self.width + tx) as usize]
    }

    fn set(&mut self, tx: i32, ty: i32, tile: Tile) {
        if tx >= 0 && tx < self.width && ty >= 0 && ty < self.height {
            self.tiles[(ty * self.width + tx) as usize] = tile;
        }
    }

    pub fn is_solid(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty).is_solid()
    }

    pub fn is_spike(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty) == Tile::Spike
    }

    /// Tile column containing world x.
    pub fn tile_x(x: f32) -> i32 {
        x.floor() as i32
    }

    /// Tile row containing world y.
    pub fn tile_y(y: f32) -> i32 {
        y.floor() as i32
    }

    /// Convert to the character grid, for rendering and round-tripping.
    pub fn to_glyph_rows(&self) -> Vec<String> {
        (0..self.height)
            .map(|ty| {
                (0..self.width)
                    .map(|tx| self.get(tx, ty).glyph())
                    .collect()
            })
            .collect()
    }

    /// Content hash used for the champion file's `level_hash` (§7.6).
    ///
    /// The plan sketches a sha256; this build uses FNV-1a 64 instead so the sim
    /// crate stays dependency-light. It serves the same purpose: detecting a
    /// champion replayed against a level it was not trained on.
    pub fn content_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        for b in self.name.as_bytes() {
            h.u8(*b);
        }
        h.u32(self.width as u32);
        h.u32(self.height as u32);
        for t in &self.tiles {
            h.u8(*t as u8);
        }
        self.spawn.hash_into(&mut h);
        h.u32(self.walkers.len() as u32);
        for w in &self.walkers {
            w.hash_into(&mut h);
        }
        h.u32(self.goals.len() as u32);
        for g in &self.goals {
            g.hash_into(&mut h);
        }
        h.finish()
    }

    /// The plan's `"sha256:…"` metadata string, with the algorithm this build uses.
    pub fn content_hash_string(&self) -> String {
        format!("fnv1a64:{:016x}", self.content_hash())
    }

    /// Build the initial player state for this level.
    pub fn spawn_player(&self) -> PlayerState {
        PlayerState::spawn(self, &PHYSICS)
    }

    /// Rightmost `x` worth scoring, used to normalise progress displays.
    pub fn width_pixels(&self) -> f32 {
        self.width as f32 * PIXELS_PER_TILE
    }
}

/// Editing operations: what the level editor (M4) needs beyond the read-only view.
///
/// Two rules shape this API.
///
/// 1. **A level in memory is always one the loader would accept.** Nothing here can
///    produce a goal-less level, a grid containing a `G` tile, or an entity off the
///    map, and [`Level::to_json_pretty`] refuses to write one. The editor can hand
///    its document straight to the sim without a validation dance first.
/// 2. **Tile coordinates in, tile coordinates out.** The loader turns a tile
///    position into a world-space AABB; [`Level::spawn_tile`] and friends invert
///    exactly that math, and `every_shipped_level_round_trips_through_the_writer`
///    pins the inversion against the real pack rather than against a rearrangement
///    of the same formulas.
impl Level {
    /// A playable starting document: solid floor, spawn top-left, goal far right.
    ///
    /// Already valid rather than an empty grid, because every rule above is easier
    /// to trust when a brand-new document is a level you can press play on.
    pub fn starter(name: impl Into<String>, width: i32, height: i32) -> Level {
        let width = width.max(MIN_LEVEL_W);
        let height = height.max(MIN_LEVEL_H);
        let mut level = Level {
            name: name.into(),
            width,
            height,
            tiles: vec![Tile::Empty; (width * height) as usize],
            spawn: Rect::new(0.0, 0.0, PHYSICS.player_w, PHYSICS.player_h),
            walkers: Vec::new(),
            goals: Vec::new(),
        };
        for tx in 0..width {
            level.set(tx, height - 1, Tile::Solid);
        }
        level.set_spawn_tile(1, height - 2);
        level.add_goal_tile(width - 2, height - 2);
        level
    }

    /// In-bounds tile, or `None` outside the grid.
    ///
    /// [`Level::get`] reports the *collision* semantics of empty space instead
    /// (solid past the side walls, empty above and below). A painter needs the
    /// difference between "a tile here" and "no tile here", so it asks this.
    pub fn tile(&self, tx: i32, ty: i32) -> Option<Tile> {
        if tx < 0 || ty < 0 || tx >= self.width || ty >= self.height {
            return None;
        }
        Some(self.tiles[(ty * self.width + tx) as usize])
    }

    /// Paint one tile. Out-of-bounds coordinates are refused, never clamped: a
    /// brush that paints the edge when the cursor is past it is worse than one that
    /// draws nothing. Returns whether the paint landed.
    ///
    /// [`Tile::Goal`] is refused the same way, and deliberately without an assert:
    /// a `G` tile is load-time sugar that would come back as an entity, so the paint
    /// is a no-op rather than a crash in a tool the user is holding.
    /// [`Level::validate`] is the backstop if one ever reaches the grid.
    pub fn set_tile(&mut self, tx: i32, ty: i32, tile: Tile) -> bool {
        if tile == Tile::Goal || self.tile(tx, ty).is_none() {
            return false;
        }
        self.set(tx, ty, tile);
        true
    }

    /// Grow or shrink the grid, anchored at the top-left.
    ///
    /// The overlap is kept, new cells get `fill`, and content that falls outside is
    /// dropped — except the last goal, which is clamped back in, because a level
    /// with no goal cannot be saved or played and losing one to a resize would be
    /// losing the document.
    pub fn resize(&mut self, width: i32, height: i32, fill: Tile) -> ResizeReport {
        let width = width.max(MIN_LEVEL_W);
        let height = height.max(MIN_LEVEL_H);
        let mut tiles = vec![fill; (width * height) as usize];
        for ty in 0..height.min(self.height) {
            for tx in 0..width.min(self.width) {
                tiles[(ty * width + tx) as usize] = self.tiles[(ty * self.width + tx) as usize];
            }
        }
        self.tiles = tiles;
        self.width = width;
        self.height = height;

        let mut report = ResizeReport::default();
        let (sx, sy) = self.spawn_tile();
        if tx_out(sx, sy, width, height) {
            self.set_spawn_tile(sx.clamp(0, width - 1), sy.clamp(0, height - 1));
            report.spawn_moved = true;
        }

        let before = self.walkers.len();
        self.walkers.retain(|w| {
            let (tx, ty) = (
                tile_of_x(w.x, PHYSICS.walker_w),
                tile_of_bottom(w.bottom()),
            );
            !tx_out(tx, ty, width, height)
        });
        report.dropped_walkers = before - self.walkers.len();

        let before = self.goals.len();
        let first_goal = self
            .goals
            .first()
            .map(|g| (tile_of_x(g.x, g.w), tile_of_bottom(g.bottom())));
        self.goals.retain(|g| {
            let (tx, ty) = (tile_of_x(g.x, g.w), tile_of_bottom(g.bottom()));
            !tx_out(tx, ty, width, height)
        });
        report.dropped_goals = before - self.goals.len();
        if self.goals.is_empty() && before > 0 {
            // Keep the level valid: the goal was pushed off the edge by a shrinking
            // grid, so pull it back in rather than deleting it.
            let (tx, ty) = first_goal.expect("before > 0 means a goal exists");
            self.add_goal_tile(tx.clamp(0, width - 1), ty.clamp(0, height - 1));
            report.dropped_goals = report.dropped_goals.saturating_sub(1);
            report.goal_moved = true;
        }
        report
    }

    /// The spawn's tile position.
    pub fn spawn_tile(&self) -> (i32, i32) {
        (
            tile_of_x(self.spawn.x, PHYSICS.player_w),
            tile_of_bottom(self.spawn.bottom()),
        )
    }

    /// Move the spawn. Clamped into the grid, because a spawn outside the map is a
    /// level that kills the player on frame 0.
    pub fn set_spawn_tile(&mut self, tx: i32, ty: i32) {
        let tx = tx.clamp(0, self.width - 1);
        let ty = ty.clamp(0, self.height - 1);
        self.spawn = Rect::new(
            tx as f32 + (1.0 - PHYSICS.player_w) * 0.5,
            crate::physics::bottom_of(ty) - PHYSICS.player_h,
            PHYSICS.player_w,
            PHYSICS.player_h,
        );
    }

    /// Tile positions of every walker, in sim order.
    pub fn walker_tiles(&self) -> Vec<(i32, i32)> {
        self.walkers
            .iter()
            .map(|w| {
                (
                    tile_of_x(w.x, PHYSICS.walker_w),
                    tile_of_bottom(w.bottom()),
                )
            })
            .collect()
    }

    /// Tile positions of every goal, in sim order.
    pub fn goal_tiles(&self) -> Vec<(i32, i32)> {
        self.goals
            .iter()
            .map(|g| (tile_of_x(g.x, g.w), tile_of_bottom(g.bottom())))
            .collect()
    }

    /// Add a walker at a tile position if it is on the map. Returns whether it landed.
    pub fn add_walker_tile(&mut self, tx: i32, ty: i32) -> bool {
        if tx_out(tx, ty, self.width, self.height) {
            return false;
        }
        self.walkers.push(Rect::new(
            tx as f32 + (1.0 - PHYSICS.walker_w) * 0.5,
            crate::physics::bottom_of(ty) - PHYSICS.walker_h,
            PHYSICS.walker_w,
            PHYSICS.walker_h,
        ));
        true
    }

    /// Add a goal at a tile position, which also makes the level valid again if it
    /// had lost its last one.
    pub fn add_goal_tile(&mut self, tx: i32, ty: i32) -> bool {
        if tx_out(tx, ty, self.width, self.height) {
            return false;
        }
        self.goals.push(goal_rect(tx, ty));
        true
    }

    /// Remove the walker standing in tile `(tx, ty)`, if any.
    pub fn remove_walker_at(&mut self, tx: i32, ty: i32) -> bool {
        let before = self.walkers.len();
        self.walkers.retain(|w| {
            (
                tile_of_x(w.x, PHYSICS.walker_w),
                tile_of_bottom(w.bottom()),
            ) != (tx, ty)
        });
        self.walkers.len() != before
    }

    /// Remove the goal in tile `(tx, ty)`, if any.
    ///
    /// Refuses to remove the last one: that is the one edit that cannot be
    /// recovered by editing further, since every later save would be rejected.
    pub fn remove_goal_at(&mut self, tx: i32, ty: i32) -> bool {
        let matches = |g: &Rect| (tile_of_x(g.x, g.w), tile_of_bottom(g.bottom())) == (tx, ty);
        if self.goals.len() <= 1 && self.goals.iter().any(matches) {
            return false;
        }
        let before = self.goals.len();
        self.goals.retain(|g| !matches(g));
        self.goals.len() != before
    }

    /// Is this document one the loader would accept?
    ///
    /// The writer runs this first, so a rejected level is a bug in the editor rather
    /// than a corrupt file on disk. It checks exactly the loader's rules plus the one
    /// invariant the loader hides by normalizing: a `G` tile in the grid would come
    /// back as a goal *entity*, so it is not writable.
    pub fn validate(&self) -> Result<(), String> {
        if self.width <= 0 || self.height <= 0 {
            return Err(format!("level has non-positive size {}x{}", self.width, self.height));
        }
        if self.tiles.len() != (self.width * self.height) as usize {
            return Err(format!(
                "grid holds {} tiles, expected {}x{} = {}",
                self.tiles.len(),
                self.width,
                self.height,
                self.width * self.height
            ));
        }
        if self.tiles.contains(&Tile::Goal) {
            return Err("grid contains a 'G' tile: it is load-time sugar, and writing it \
                 would load back as a goal entity with different tiles"
                .into());
        }
        if self.goals.is_empty() {
            return Err("level has no goal (place a goal entity or drop a 'G' tile)".into());
        }
        if !(self.spawn.x.is_finite() && self.spawn.y.is_finite()) {
            return Err("spawn position is not finite".into());
        }
        let (sx, sy) = self.spawn_tile();
        if tx_out(sx, sy, self.width, self.height) {
            return Err(format!("spawn tile ({sx}, {sy}) is outside the grid"));
        }
        for (what, tiles) in [
            ("walker", self.walker_tiles()),
            ("goal", self.goal_tiles()),
        ] {
            for (tx, ty) in tiles {
                if tx_out(tx, ty, self.width, self.height) {
                    return Err(format!("{what} at tile ({tx}, {ty}) is outside the grid"));
                }
            }
        }
        Ok(())
    }

    /// Serialize to the format `from_json_str` reads, pretty-printed.
    ///
    /// Refuses an invalid document rather than writing one: every file this crate
    /// produces must be a file this crate can load.
    pub fn to_json_pretty(&self) -> Result<String, String> {
        self.validate()?;
        let mut entities: Vec<RawEntity> = Vec::with_capacity(self.walkers.len() + self.goals.len());
        for (tx, ty) in self.walker_tiles() {
            entities.push(RawEntity {
                kind: "walker".into(),
                x: tx,
                y: ty,
            });
        }
        for (tx, ty) in self.goal_tiles() {
            entities.push(RawEntity {
                kind: "goal".into(),
                x: tx,
                y: ty,
            });
        }
        let (sx, sy) = self.spawn_tile();
        let raw = RawLevel {
            format: LEVEL_FORMAT.to_string(),
            level_format_version: LEVEL_FORMAT_VERSION,
            name: self.name.clone(),
            width: self.width,
            height: self.height,
            spawn: TilePos { x: sx, y: sy },
            tiles: self.to_glyph_rows(),
            entities,
        };
        serde_json::to_string_pretty(&raw)
            .map(|mut s| {
                s.push('\n');
                s
            })
            .map_err(|e| format!("cannot serialize level: {e}"))
    }

    /// Write the level to a file, creating it if needed.
    pub fn to_json_file(&self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        let path = path.as_ref();
        let json = self.to_json_pretty()?;
        std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

/// Tile column containing the left edge of a body placed by the loader.
fn tile_of_x(rect_x: f32, w: f32) -> i32 {
    (rect_x - (1.0 - w) * 0.5).round() as i32
}

/// Tile row whose bottom edge a body's bottom edge rests on.
fn tile_of_bottom(rect_bottom: f32) -> i32 {
    (rect_bottom - 1.0).round() as i32
}

fn tx_out(tx: i32, ty: i32, width: i32, height: i32) -> bool {
    tx < 0 || ty < 0 || tx >= width || ty >= height
}

fn goal_rect(tx: i32, ty: i32) -> Rect {
    let bottom = crate::physics::bottom_of(ty);
    Rect::new(tx as f32, bottom - 2.0, 1.0, 2.0)
}

#[cfg(test)]
mod editing_tests {
    use super::*;

    const PACK: [&str; 6] = [
        "l1-flat-run",
        "l2-first-gap",
        "l3-gauntlet",
        "l4-rhythm",
        "l5-tightrope",
        "l6-marathon",
    ];

    fn shipped(slug: &str) -> Level {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"));
        match Level::from_json_file(&path) {
            Ok(level) => level,
            Err(err) => panic!("shipped level {slug} failed to load: {err}"),
        }
    }

    /// The load -> write -> load loop over the real pack.
    ///
    /// This is the test the whole writing API exists to survive. The content hash
    /// covers the name, the grid, the spawn and every entity AABB, so a single tile
    /// column rounding the wrong way in `tile_of_x` shows up here as a changed hash
    /// rather than as a level that quietly plays differently.
    #[test]
    fn every_shipped_level_round_trips_through_the_writer_with_the_same_hash() {
        for slug in PACK {
            let level = shipped(slug);
            let json = level.to_json_pretty().expect("the pack must be writable");
            let back = Level::from_json_str(&json)
                .unwrap_or_else(|e| panic!("{slug} did not survive a round trip: {e}\n{json}"));

            assert_eq!(back.content_hash(), level.content_hash(), "{slug} hash");
            assert_eq!(back.content_hash_string(), level.content_hash_string());
            assert_eq!(back.name, level.name, "{slug} name");
            assert_eq!((back.width, back.height), (level.width, level.height), "{slug} size");
            assert_eq!(back.to_glyph_rows(), level.to_glyph_rows(), "{slug} grid");
            assert_eq!(back.spawn, level.spawn, "{slug} spawn");
            assert_eq!(back.walkers, level.walkers, "{slug} walkers");
            assert_eq!(back.goals, level.goals, "{slug} goals");
            assert_eq!(
                back.walker_tiles(),
                level.walker_tiles(),
                "{slug} walker tiles"
            );
            assert_eq!(back.goal_tiles(), level.goal_tiles(), "{slug} goal tiles");
            assert_eq!(back.spawn_tile(), level.spawn_tile(), "{slug} spawn tile");

            // Writing is deterministic: the same document gives the same bytes, so a
            // save that changed nothing does not produce a diff.
            assert_eq!(level.to_json_pretty().unwrap(), json, "{slug} writer stability");
            // And the written file is a level document, not just valid JSON.
            assert!(json.contains(LEVEL_FORMAT), "{slug} carries the format tag");
            assert!(json.ends_with('\n'), "{slug} ends with a newline");
        }
    }

    #[test]
    fn entity_and_spawn_tiles_invert_the_placement_math() {
        let mut level = Level::starter("tiles", 20, 10);
        assert_eq!(level.spawn_tile(), (1, 8));
        assert_eq!(level.goal_tiles(), vec![(18, 8)]);

        level.add_walker_tile(7, 8);
        assert_eq!(level.walker_tiles(), vec![(7, 8)]);
        // The AABB the loader would have derived from that tile, exactly.
        let w = level.walkers[0];
        assert_eq!(w.x, 7.0 + (1.0 - PHYSICS.walker_w) * 0.5);
        assert_eq!(w.bottom(), crate::physics::bottom_of(8));

        // Round-tripping through the writer keeps the tile positions, which is the
        // only form a human can edit.
        let back = Level::from_json_str(&level.to_json_pretty().unwrap()).expect("parses");
        assert_eq!(back.walker_tiles(), level.walker_tiles());
        assert_eq!(back.goal_tiles(), level.goal_tiles());
        assert_eq!(back.spawn_tile(), level.spawn_tile());
    }

    #[test]
    fn moving_the_spawn_moves_the_player() {
        let mut level = Level::starter("spawn", 20, 10);
        level.set_spawn_tile(6, 3);
        assert_eq!(level.spawn_tile(), (6, 3));
        let player = level.spawn_player();
        assert_eq!(player.x, 6.0 + (1.0 - PHYSICS.player_w) * 0.5);
        assert_eq!(player.y + player.h, crate::physics::bottom_of(3));

        // A spawn past the edge is clamped, never stored outside the map.
        level.set_spawn_tile(999, -5);
        assert_eq!(level.spawn_tile(), (19, 0));
        assert!(level.validate().is_ok());
    }

    #[test]
    fn starter_level_is_valid_with_a_floor_spawn_and_goal() {
        let level = Level::starter("fresh", 20, 10);
        assert!(level.validate().is_ok(), "{:?}", level.validate());
        assert!(level.to_json_pretty().is_ok());
        assert_eq!(level.width, 20);
        assert_eq!(level.height, 10);
        // A floor to stand on, in the bottom row only.
        assert!((0..20).all(|tx| level.is_solid(tx, 9)));
        assert!((0..9).all(|ty| (0..20).all(|tx| !level.is_solid(tx, ty))));
        // Spawn and goal both stand on that floor, which is what makes it playable.
        assert_eq!(level.spawn_tile(), (1, 8));
        assert_eq!(level.goal_tiles(), vec![(18, 8)]);
        assert_eq!(level.goals.len(), 1);
        // Tiny requests come back as the minimum grid rather than something unusable.
        let small = Level::starter("small", 1, 1);
        assert_eq!((small.width, small.height), (MIN_LEVEL_W, MIN_LEVEL_H));
    }

    #[test]
    fn painting_a_tile_and_erasing_it_leaves_the_document_identical() {
        let mut level = shipped("l2-first-gap");
        let before = level.content_hash();
        let original = level.tile(10, 12);

        assert!(level.set_tile(10, 12, Tile::Solid));
        assert_ne!(level.content_hash(), before, "painting must change the level");
        assert!(level.set_tile(10, 12, original.expect("in bounds")));
        assert_eq!(level.content_hash(), before, "erasing must give it back");

        // Painting over every tile type is allowed; the grid is the editor's.
        for tile in [Tile::Empty, Tile::Solid, Tile::Spike] {
            assert!(level.set_tile(3, 3, tile));
            assert_eq!(level.tile(3, 3), Some(tile));
        }
    }

    #[test]
    fn set_tile_refuses_out_of_bounds_and_goal_glyphs() {
        let mut level = Level::starter("edges", 20, 10);
        let before = level.content_hash();
        for (tx, ty) in [(-1, 5), (20, 5), (5, -1), (5, 10), (99, 99)] {
            assert!(!level.set_tile(tx, ty, Tile::Solid), "({tx}, {ty}) must be refused");
            assert_eq!(level.tile(tx, ty), None, "no tile exists at ({tx}, {ty})");
        }
        // A `G` tile is load-time sugar and would load back as an entity.
        assert!(!level.set_tile(4, 4, Tile::Goal));
        assert_eq!(level.content_hash(), before);
        // The whole-grid view still reports empty above the floor and walls at the
        // sides, because `get` answers a collision question, not an editing one.
        assert_eq!(level.get(5, 5), Tile::Empty);
        assert_eq!(level.get(-1, 5), Tile::Solid);
    }

    #[test]
    fn remove_refuses_to_delete_the_last_goal() {
        let mut level = Level::starter("one-goal", 20, 10);
        assert!(!level.remove_goal_at(18, 8), "the last goal is the document");
        assert_eq!(level.goals.len(), 1);
        assert!(level.validate().is_ok());

        // With two, both can go in either order and the level stays valid.
        assert!(level.add_goal_tile(2, 8));
        assert!(level.remove_goal_at(18, 8));
        assert_eq!(level.goal_tiles(), vec![(2, 8)]);
        assert!(!level.remove_goal_at(2, 8));
        assert!(level.remove_goal_at(19, 19).eq(&false), "nothing there to remove");
    }

    #[test]
    fn adding_and_removing_walkers_is_symmetric() {
        let mut level = Level::starter("enemies", 20, 10);
        assert!(!level.add_walker_tile(20, 5), "off the map");
        assert!(!level.add_walker_tile(-1, 5));
        assert!(level.add_walker_tile(8, 8));
        assert_eq!(level.walker_tiles(), vec![(8, 8)]);
        assert!(!level.remove_walker_at(7, 8), "one tile left of it");
        assert!(level.remove_walker_at(8, 8));
        assert!(level.walkers.is_empty());
        assert!(!level.remove_walker_at(8, 8), "already gone");
    }

    #[test]
    fn resize_keeps_the_overlap_and_reports_what_it_dropped() {
        let mut level = shipped("l3-gauntlet");
        let before = level.clone();
        // The level is 60x15 with a walker; grow it and nothing should move.
        let report = level.resize(70, 20, Tile::Empty);
        assert_eq!(report, ResizeReport::default());
        assert_eq!((level.width, level.height), (70, 20));
        assert_eq!((before.width, before.height), (60, 15));
        for ty in 0..15 {
            for tx in 0..60 {
                assert_eq!(level.tile(tx, ty), before.tile(tx, ty), "tile ({tx}, {ty})");
            }
        }
        assert_eq!(level.walker_tiles(), before.walker_tiles());
        assert_eq!(level.spawn_tile(), before.spawn_tile());
        // New cells got the fill.
        assert_eq!(level.tile(69, 19), Some(Tile::Empty));

        // Shrinking to a corner that excludes the goal drags the goal back in, and
        // the level stays writable.
        let report = level.resize(MIN_LEVEL_W, MIN_LEVEL_H, Tile::Empty);
        assert!(report.spawn_moved, "the spawn was 40 tiles to the right");
        assert!(report.goal_moved, "the goal was dragged back to the grid edge");
        assert!(level.validate().is_ok(), "{:?}", level.validate());
        assert_eq!(
            level.goal_tiles()[0],
            (MIN_LEVEL_W - 1, MIN_LEVEL_H - 1),
            "the last valid tile, not one inside the old goal's row"
        );
        assert!(level.to_json_pretty().is_ok());
    }

    #[test]
    fn resize_to_a_smaller_grid_clamps_the_spawn_and_drops_dead_entities() {
        let mut level = Level::starter("shrink", 40, 12);
        level.add_walker_tile(35, 10);
        level.add_goal_tile(30, 10);
        assert_eq!(level.goal_tiles(), vec![(38, 10), (30, 10)]);

        let report = level.resize(10, 8, Tile::Empty);
        // Both entities were past column 10, so the walker goes and one of the two
        // goals does: the last one is never sacrificed, it is pulled back in.
        assert_eq!(report.dropped_walkers, 1, "the walker was at column 35");
        assert_eq!(report.dropped_goals, 1, "the second goal was at column 30");
        assert!(report.goal_moved);
        assert!(report.dropped_anything());
        assert_eq!(level.walker_tiles(), Vec::<(i32, i32)>::new());
        assert_eq!(level.goal_tiles(), vec![(9, 7)], "clamped to the grid edge");
        assert_eq!(level.spawn_tile(), (1, 7), "the spawn row was 10");
        assert!(report.spawn_moved);
        assert!(level.validate().is_ok(), "{:?}", level.validate());
    }

    #[test]
    fn the_writer_refuses_a_document_the_loader_would_reject() {
        let mut level = Level::starter("invalid", 20, 10);
        level.goals.clear();
        let err = level.to_json_pretty().expect_err("no goal, no file");
        assert!(err.contains("no goal"), "{err}");

        // A spawn dragged outside the grid is refused rather than written out.
        let mut level = Level::starter("invalid", 20, 10);
        level.spawn.x = 999.0;
        let err = level.to_json_pretty().expect_err("spawn off the map");
        assert!(err.contains("spawn"), "{err}");

        // And a `G` tile smuggled into the grid: writing it would change the grid on
        // the way back in.
        let mut level = Level::starter("invalid", 20, 10);
        level.set(4, 4, Tile::Goal);
        let err = level.to_json_pretty().expect_err("'G' tile in the grid");
        assert!(err.contains("'G' tile"), "{err}");
    }

    #[test]
    fn to_json_file_writes_a_level_the_loader_reads_back() {
        let level = shipped("l5-tightrope");
        let mut path = std::env::temp_dir();
        path.push(format!("evo-level-write-{}.json", std::process::id()));
        level.to_json_file(&path).expect("write");
        let back = Level::from_json_file(&path).expect("read back");
        assert_eq!(back.content_hash(), level.content_hash());
        let _ = std::fs::remove_file(&path);

        // A path that cannot be written is an error, not a panic.
        let mut bad = std::env::temp_dir();
        bad.push("evo-level-write-missing-dir");
        bad.push("nope.json");
        assert!(level.to_json_file(&bad).is_err());
    }
}
