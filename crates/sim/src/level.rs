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

use serde::Deserialize;

use crate::hash::StateHasher;
use crate::physics::{PlayerState, PHYSICS};

use crate::PIXELS_PER_TILE;

pub const LEVEL_FORMAT: &str = "evo-platformer-level";
pub const LEVEL_FORMAT_VERSION: u32 = 1;

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

#[derive(Clone, Copy, Debug, Deserialize)]
struct TilePos {
    x: i32,
    y: i32,
}

#[derive(Clone, Debug, Deserialize)]
struct RawEntity {
    #[serde(rename = "type")]
    kind: String,
    x: i32,
    y: i32,
}

#[derive(Clone, Debug, Deserialize)]
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

fn goal_rect(tx: i32, ty: i32) -> Rect {
    let bottom = crate::physics::bottom_of(ty);
    Rect::new(tx as f32, bottom - 2.0, 1.0, 2.0)
}
