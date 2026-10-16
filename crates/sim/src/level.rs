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
pub const GLYPH_SAND: char = 'S';
pub const GLYPH_BRICK: char = 'B';
pub const GLYPH_PLANK: char = 'W';
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
    /// Soil of a different material: a beach, a dune, a desert floor.
    ///
    /// **Physics does not know about sand.** It is solid exactly as [`Tile::Solid`] is —
    /// `is_solid` says yes, the collision code never asks which material a tile is made of,
    /// and an agent's own senses report it as solid ([`crate::sensors`]), so a level painted
    /// with sand is not a new problem for the population: it is the same problem in a
    /// different colour. What the variant buys is authorship and art: a level file can say
    /// `S` where it means a beach, and the renderer draws the sand family over it, which is
    /// the whole difference between "ground" and "the ground here is sand".
    Sand = 4,
    /// Ancient stone masonry (`B`). Solid exactly as [`Tile::Solid`] and [`Tile::Sand`] are;
    /// drawn with the stone brick autotile family.
    Brick = 5,
    /// Timber scaffolding / wood planks (`W`). Solid exactly as [`Tile::Solid`] is;
    /// drawn with the wood plank autotile family.
    Plank = 6,
}

impl Tile {
    pub fn from_glyph(c: char) -> Option<Tile> {
        match c {
            GLYPH_EMPTY => Some(Tile::Empty),
            GLYPH_SOLID => Some(Tile::Solid),
            GLYPH_SAND => Some(Tile::Sand),
            GLYPH_BRICK => Some(Tile::Brick),
            GLYPH_PLANK => Some(Tile::Plank),
            GLYPH_SPIKE => Some(Tile::Spike),
            GLYPH_GOAL => Some(Tile::Goal),
            _ => None,
        }
    }

    pub fn glyph(self) -> char {
        match self {
            Tile::Empty => GLYPH_EMPTY,
            Tile::Solid => GLYPH_SOLID,
            Tile::Sand => GLYPH_SAND,
            Tile::Brick => GLYPH_BRICK,
            Tile::Plank => GLYPH_PLANK,
            Tile::Spike => GLYPH_SPIKE,
            Tile::Goal => GLYPH_GOAL,
        }
    }

    pub fn is_solid(self) -> bool {
        matches!(self, Tile::Solid | Tile::Sand | Tile::Brick | Tile::Plank)
    }

    /// Is this tile only a different *material* of solid ground?
    ///
    /// The one question the renderer asks about sand: which autotile family to draw it in.
    /// It is deliberately not a question about collision — nothing that decides where the
    /// player goes may care what the ground is made of.
    pub fn is_sand(self) -> bool {
        matches!(self, Tile::Sand)
    }

    pub fn is_brick(self) -> bool {
        matches!(self, Tile::Brick)
    }

    pub fn is_plank(self) -> bool {
        matches!(self, Tile::Plank)
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
    /// Carried only by a `scenery` entity, and omitted from every other entity's JSON so
    /// an existing file's shape is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sprite: Option<String>,
}

/// A placed piece of scenery: an atlas sprite name, standing in a tile cell.
///
/// Scenery is **not simulated**. The physics never sees it, an agent's sensors cannot
/// detect it, and no outcome depends on it. It exists so an author can *place* the same
/// bushes, trees, rocks, flowers and vines the view already scatters by hash, which turns
/// the dressing from a property of a tile into something a person composes. The sprite is
/// carried as a plain name, so `sim` still depends on no renderer; a name the tileset does
/// not carry is simply not drawn by the view rather than an error here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenery {
    /// The atlas sprite name to draw in the cell.
    pub sprite: String,
    /// The tile cell the sprite stands in.
    pub x: i32,
    pub y: i32,
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
    /// Placed scenery that fell outside the new grid. Decoration is disposable, so it is
    /// simply dropped (like a walker) rather than clamped back in (like the goal).
    pub dropped_scenery: usize,
    /// The spawn point fell outside the new grid and was pulled back in.
    pub spawn_moved: bool,
    /// The goal fell outside the new grid and was pulled back in.
    ///
    /// There is no `dropped_goals` beside this: a level has one flag, and the flag is
    /// never dropped by a resize — losing it would leave a document that cannot be saved
    /// or played, so the grid moves out from under it and the goal comes with it.
    pub goal_moved: bool,
}

impl ResizeReport {
    pub fn dropped_anything(&self) -> bool {
        self.dropped_walkers > 0 || self.dropped_scenery > 0
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
    /// The goal trigger box: **at most one**, and exactly one for a level a file on disk
    /// describes.
    ///
    /// A `Vec` rather than an `Option` only because every consumer walks it — the sensors,
    /// the oracle, the renderer — and a one-element walker list is less code than a maybe.
    /// The one-flag rule is kept by [`Level::set_goal_tile`], [`Level::remove_goal_at`],
    /// the loader and [`Level::validate`]; nothing else may push.
    pub goals: Vec<Rect>,
    /// Placed scenery, in draw order. Inert: not simulated, not hashed, not sensed.
    pub scenery: Vec<Scenery>,
    /// Lazily computed move-graph route table for this level, shared across clones via `Arc`.
    routes_cache: std::sync::OnceLock<std::sync::Arc<crate::reach::GoalRoutes>>,
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
            scenery: Vec::new(),
            routes_cache: std::sync::OnceLock::new(),
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
                "goal" => {
                    // One flag per level, refused here as well as in the editor: the oracle,
                    // the sensors and the score all answer about *the* goal, so a second one
                    // would not be "more goal", it would be an ambiguity.
                    if !level.goals.is_empty() {
                        return Err("level has more than one goal: \
                             a level has exactly one flag"
                            .into());
                    }
                    level.goals.push(goal_rect(e.x, e.y));
                }
                // Scenery carries its own sprite name; an unnamed one is a mistake in the
                // file rather than a piece of nothing, so it is refused like any bad entity.
                "scenery" => {
                    let sprite = e
                        .sprite
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or_else(|| "scenery entity has no sprite name".to_string())?;
                    level.scenery.push(Scenery {
                        sprite,
                        x: e.x,
                        y: e.y,
                    });
                }
                other => return Err(format!("unknown entity type {other:?}")),
            }
        }

        // A `G` tile is sugar for a goal entity; normalize it so there is exactly
        // one code path for goal triggers.
        for ty in 0..level.height {
            for tx in 0..level.width {
                if level.get(tx, ty) == Tile::Goal {
                    if !level.goals.is_empty() {
                        return Err("level has more than one goal (a 'G' tile and a \
                             goal entity): a level has exactly one flag"
                            .into());
                    }
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
            self.routes_cache = std::sync::OnceLock::new();
        }
    }

    /// Return the move-graph route table for this level, computing and caching it on first ask.
    pub fn routes(&self) -> &crate::reach::GoalRoutes {
        self.routes_cache
            .get_or_init(|| std::sync::Arc::new(crate::reach::goal_routes(self)))
    }

    pub fn is_solid(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty).is_solid()
    }

    pub fn is_spike(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty) == Tile::Spike
    }

    /// Can a body *stand* on this tile: solid ground, with room above it and no hazard in
    /// the room it needs?
    ///
    /// The hazard convention is that a floor spike occupies the row the body walks
    /// *through*, one above the row that supports it (`levels/l3-gauntlet.json` and every
    /// other shipped hazard level place them that way). So a supporting tile is standable
    /// unless the row above it is a spike — a spot where the body would be standing inside
    /// the obstacle — or is more ground, which is a buried tile inside a hill rather than a
    /// floor anyone can be on. The body is 0.9 tiles tall, so one clear tile above the
    /// support is exactly the headroom it needs.
    pub fn is_standable(&self, tx: i32, surface_row: i32) -> bool {
        self.is_solid(tx, surface_row)
            && !self.is_spike(tx, surface_row - 1)
            && !self.is_solid(tx, surface_row - 1)
    }

    /// The **landing zone** containing a standing tile: the run of standable tiles around
    /// `tx`, named by the column it starts at.
    ///
    /// A zone rather than a tile, because a landing is not a place you aim at exactly. A
    /// body that clears a hazard and touches down anywhere in the run beyond it has
    /// arrived; scoring the individual tile would pay for landing on the *first* safe tile
    /// and refuse the same crossing for landing a tile further on, which is the wrong
    /// question and would make the reward a function of landing precision rather than of
    /// the crossing.
    pub fn landing_zone(&self, tx: i32, surface_row: i32) -> i32 {
        self.landing_zone_bounds(tx, surface_row).0
    }

    /// Inclusive `(start, end)` column span of the contiguous standable zone containing `tx`
    /// on `surface_row`.
    pub fn landing_zone_bounds(&self, tx: i32, surface_row: i32) -> (i32, i32) {
        let mut start = tx;
        while start > 0 && self.is_standable(start - 1, surface_row) {
            start -= 1;
        }
        let mut end = tx;
        while end + 1 < self.width && self.is_standable(end + 1, surface_row) {
            end += 1;
        }
        (start, end)
    }

    /// Is the column `edge_col` just outside a landing zone a crossing (a spike at standing
    /// height, or a drop of at least two rows)?
    fn edge_is_crossing(&self, edge_col: i32, surface_row: i32) -> bool {
        if edge_col < 0 || edge_col >= self.width {
            return false;
        }
        if self.is_spike(edge_col, surface_row - 1) {
            return true;
        }
        !self.is_solid(edge_col, surface_row) && !self.is_solid(edge_col, surface_row + 1)
    }

    /// Is the landing zone beginning at `zone_start` one the body could only have arrived at
    /// by crossing something from the left?
    fn zone_is_checkpoint(&self, zone_start: i32, surface_row: i32) -> bool {
        debug_assert!(zone_start > 0, "callers exclude the left-edge zone");
        self.edge_is_crossing(zone_start - 1, surface_row)
    }

    /// If `(tx, surface_row)` is ground the body can stand on **and reaching it from the left
    /// meant crossing something**, the id of the checkpoint zone it belongs to; otherwise `None`.
    pub fn checkpoint_zone(&self, tx: i32, surface_row: i32) -> Option<i32> {
        if !self.is_standable(tx, surface_row) {
            return None;
        }
        let start = self.landing_zone(tx, surface_row);
        if start > 0 && self.zone_is_checkpoint(start, surface_row) {
            Some(start)
        } else {
            None
        }
    }

    /// Directional checkpoint query: given the column `(tx, surface_row)` the body is standing
    /// on and the column `from_col` it started or last left a landing zone from, returns the
    /// checkpoint zone id (`zone_start`) if arriving on this zone from `from_col` crossed a
    /// hazard or a drop on the approached side.
    ///
    /// Symmetric across left-to-right and right-to-left play:
    /// * approaching from the left (`from_col < start`) checks the zone's left edge (`start - 1`);
    /// * approaching from the right (`from_col > end`) checks the zone's right edge (`end + 1`);
    /// * staying inside `start..=end` on the same zone is not a crossing (`None`).
    pub fn checkpoint_zone_from(&self, tx: i32, surface_row: i32, from_col: i32) -> Option<i32> {
        if !self.is_standable(tx, surface_row) {
            return None;
        }
        let (start, end) = self.landing_zone_bounds(tx, surface_row);
        let crossed = if from_col < start {
            start > 0 && self.edge_is_crossing(start - 1, surface_row)
        } else if from_col > end {
            end + 1 < self.width && self.edge_is_crossing(end + 1, surface_row)
        } else {
            false
        };
        if crossed {
            Some(start)
        } else {
            None
        }
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
    /// Placed scenery is deliberately **not** hashed: it is inert, so a champion trained on
    /// a level and a champion trained on the same level with a tree added are the same
    /// champion. Excluding it keeps every hash this build produced before scenery existed
    /// valid, which keeps the champions on disk valid too.
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
            scenery: Vec::new(),
            routes_cache: std::sync::OnceLock::new(),
        };
        for tx in 0..width {
            level.set(tx, height - 1, Tile::Solid);
        }
        level.set_spawn_tile(1, height - 2);
        level.set_goal_tile(width - 2, height - 2);
        level
    }

    /// This level flipped left-to-right.
    ///
    /// The mirror of a level is the sharpest test a *directional* capability has. Every
    /// bug this crate's neighbours have had about direction — no jump-left, no sprint-left,
    /// a gap probe that looked the way the body happened to be drifting — is invisible on
    /// the shipped pack, which is all rightward, and fatal on its mirror: the mirror of
    /// `l8-long-jump` was *unbeatable in principle* while the level it mirrors is the one
    /// that teaches sprint. So this exists to be asserted on (`levels.rs` checks that every
    /// shipped level's mirror is still beatable, `evo`'s difficulty test that it is still
    /// *learned*), not as an editing convenience.
    ///
    /// Everything an entity carries moves: tiles, the spawn, the flag and the walkers.
    /// Scenery moves too — it is inert, but a flipped level that kept its bushes on the old
    /// side would look like a bug in everything that draws it.
    ///
    /// Written through the same mutators the editor uses ([`Self::set_tile`],
    /// [`Self::set_spawn_tile`], [`Self::set_goal_tile`]), so a mirror cannot be a level the
    /// rules would refuse to *make*: the spawn lands on the tile the mirrored one occupies,
    /// and the flag is moved rather than a second one appearing beside it.
    ///
    /// **One thing does not flip: a walker's patrol direction — and it is not a direction the
    /// file holds.** A walker entity carries a position, not a heading: the initial direction is
    /// drawn from the episode RNG
    /// ([`Walker::spawn_spread`](crate::physics::Walker::spawn_spread)), which is the sim's only
    /// gameplay randomness and the entire reason a walker level is stochastic. A heading field on
    /// the entity would make every trial of such a level the byte-identical episode, so there is
    /// nothing here for a mirror to flip — and a mirrored walker level is a reflection of the
    /// geometry and of the *distribution* of patrols rather than of any one patrol.
    ///
    /// **What a previous pass wrote here — that a mirrored walker level is measurably harder,
    /// 4/3/7/11 against 18/18/17/16 on `l8-long-jump` — does not reproduce, and that is worth
    /// recording rather than quietly deleting.** The same pair on this build, pop 200, seeds 1-4,
    /// `--stop-on-win`: **4 / 4 solved at generations 7, 3, 3, 4 against 3, 0, 5, 6** (means 4.2
    /// and 3.5), and with the walker deleted from both files 2, 2, 3, 6 against 3, 0, 3, 5
    /// (means 3.2 and 2.8) — the mirror is the *faster* of the two in both pairings. The model
    /// the old number rested on was wrong about the sim rather than about mirrors: walkers have
    /// never set off rightward, because the direction draw is in the repository's first commit.
    /// What was true is that `Walker::new`, which tooling uses for a deterministic pose, builds
    /// one facing right. See `REVIEW.md` §2.9.
    pub fn mirrored_horizontal(&self) -> Level {
        let mut out = Level::starter(
            format!("{} mirrored", self.name),
            self.width,
            self.height,
        );
        for ty in 0..self.height {
            for tx in 0..self.width {
                let tile = self.get(tx, ty);
                out.set_tile(self.width - 1 - tx, ty, tile);
            }
        }

        let (sx, sy) = self.spawn_tile();
        out.set_spawn_tile(self.width - 1 - sx, sy);

        match self.goal_tiles().first() {
            Some(&(gx, gy)) => {
                out.set_goal_tile(self.width - 1 - gx, gy);
            }
            // A document can hold no flag (the editor's eraser leaves none). The mirror of
            // no flag is no flag, so the starter's own placeholder comes off.
            None => {
                if let Some(&(gx, gy)) = out.goal_tiles().first() {
                    out.remove_goal_at(gx, gy);
                }
            }
        }

        // A box is mirrored by its *far* edge: `x' = width - (x + w)`, which is the whole
        // reason this is not just "negate x". Walkers are mirrored as raw boxes rather than
        // through `add_walker_tile` because a walker is stored as one, sub-tile jitter and
        // all, and re-snapping it to a lattice would move a body the physics had placed.
        let w = self.width as f32;
        out.walkers = self
            .walkers
            .iter()
            .map(|r| Rect::new(w - (r.x + r.w), r.y, r.w, r.h))
            .collect();
        out.scenery = self
            .scenery
            .iter()
            .map(|s| Scenery {
                sprite: s.sprite.clone(),
                x: self.width - 1 - s.x,
                y: s.y,
            })
            .collect();
        out
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
    /// dropped — walkers and scenery. The goal is **not** dropped: a level with no flag
    /// cannot be saved or played, so a grid that shrinks out from under the flag drags it
    /// back in rather than losing the document.
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
        self.routes_cache = std::sync::OnceLock::new();

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

        let before = self.scenery.len();
        self.scenery
            .retain(|s| !tx_out(s.x, s.y, width, height));
        report.dropped_scenery = before - self.scenery.len();

        let before = self.goals.len();
        let first_goal = self
            .goals
            .first()
            .map(|g| (tile_of_x(g.x, g.w), tile_of_bottom(g.bottom())));
        self.goals.retain(|g| {
            let (tx, ty) = (tile_of_x(g.x, g.w), tile_of_bottom(g.bottom()));
            !tx_out(tx, ty, width, height)
        });
        if self.goals.is_empty() && before > 0 {
            // Keep the level valid: the goal was pushed off the edge by a shrinking
            // grid, so pull it back in rather than deleting it.
            let (tx, ty) = first_goal.expect("before > 0 means a goal exists");
            self.set_goal_tile(tx.clamp(0, width - 1), ty.clamp(0, height - 1));
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

    /// The goal's tile position — one entry, or none in a document that has no flag yet.
    ///
    /// A goal's *anchor* tile: the cell its pole's base stands on. The marker is two cells
    /// tall and the flag hangs in the cell above ([`Self::goal_covers`]), so this is where a
    /// goal *is* while the cell above is still part of it.
    pub fn goal_tiles(&self) -> Vec<(i32, i32)> {
        self.goals.iter().map(goal_anchor).collect()
    }

    /// Whether a goal's *marker* covers tile `(tx, ty)` — a question about two cells, not one.
    ///
    /// The flag is drawn two tiles tall (`crate::view::goal_cells`): the pole's base sits on
    /// the goal's anchor and the cloth hangs in the cell above it. So the cell most people
    /// aim at is the flag, one tile *above* the anchor, and a rule that only knew the anchor
    /// would call that click "nothing here". Everything that has to agree with what the user
    /// can see — the eraser, the placement check, the drop ghost — asks this instead.
    pub fn goal_covers(&self, tx: i32, ty: i32) -> bool {
        self.goals.iter().any(|g| goal_marker_covers(g, tx, ty))
    }

    /// Whether the goal may be anchored at `(tx, ty)`: the tile has to be on the map, and
    /// neither of the two cells the marker would cover may already be the goal's.
    ///
    /// The editor asks this to colour the drop ghost, so the refusal is visible before the
    /// mouse button comes up; [`Self::set_goal_tile`] refuses the same placements, so the
    /// preview and the write cannot disagree.
    pub fn goal_placeable(&self, tx: i32, ty: i32) -> bool {
        !tx_out(tx, ty, self.width, self.height)
            && !self.goal_covers(tx, ty)
            && !self.goal_covers(tx, ty - 1)
    }

    /// Whether a walker placed at tile `(tx, ty)` would be standing on something.
    ///
    /// A placement is anchored by its feet: [`add_walker_tile`](Self::add_walker_tile) puts
    /// the box so its bottom rests on the row *below* `ty`, and a walker with nothing under
    /// it falls on the first tick — which the shipped pack's own test refuses. The editor
    /// asks this before it writes a walker, and that test asks it too, so the two cannot
    /// disagree about what a level is allowed to contain.
    pub fn walker_stands_on_ground(&self, tx: i32, ty: i32) -> bool {
        let x = tx as f32 + (1.0 - PHYSICS.walker_w) * 0.5;
        let (c0, c1) = crate::physics::tile_span(x, PHYSICS.walker_w);
        (c0..=c1).any(|c| self.is_solid(c, ty + 1))
    }

    /// Whether a spawn placed at tile `(tx, ty)` would stand on something.
    ///
    /// The spawn's own version of [`Self::walker_stands_on_ground`], and it refuses one
    /// thing more than the walker's rule does. A walker placed in a wall is an odd-looking
    /// level; a *spawn* placed in a wall is an episode that starts with the player embedded
    /// in the ground, and a spawn placed in mid-air is an episode that starts with the
    /// player falling — which is a level whose very first frame is already decided. So the
    /// cell has to be on the map, empty of tiles, and have solid ground under the player's
    /// own footprint rather than under one point of it.
    ///
    /// Ground, not *soil*: sand, brick and timber are solid in exactly the same way, and a
    /// start on a beach is a start ([`Tile::is_solid`]). The editor asks this before it
    /// writes a spawn, so the click that would drop the player is refused out loud instead
    /// of discovered on the first playtest.
    pub fn spawn_stands_on_ground(&self, tx: i32, ty: i32) -> bool {
        if tx_out(tx, ty, self.width, self.height) || self.is_solid(tx, ty) {
            return false;
        }
        let x = tx as f32 + (1.0 - PHYSICS.player_w) * 0.5;
        let (c0, c1) = crate::physics::tile_span(x, PHYSICS.player_w);
        (c0..=c1).any(|c| self.is_solid(c, ty + 1))
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

    /// Place **the** goal at a tile position, which also makes the level valid again if it
    /// had none.
    ///
    /// A level has exactly one flag, so this *moves* it: the flag already on the map
    /// relocates to `(tx, ty)` instead of a second one appearing beside it — the same shape
    /// as [`Self::set_spawn_tile`], for the same reason. Returns whether the document
    /// changed.
    ///
    /// Refuses a tile that is already part of the goal's own marker — the pole it stands on
    /// *or* the cell its cloth hangs in, plus the cell whose own flag would hang in that
    /// pole — so a click on the flag the user is looking at is a refusal that can be
    /// explained, not a silent one-cell shift of the marker out from under the pointer.
    /// Every rule is [`Self::goal_placeable`]'s.
    pub fn set_goal_tile(&mut self, tx: i32, ty: i32) -> bool {
        if !self.goal_placeable(tx, ty) {
            return false;
        }
        self.goals.clear();
        self.goals.push(goal_rect(tx, ty));
        self.routes_cache = std::sync::OnceLock::new();
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

    /// Remove the goal whose marker covers tile `(tx, ty)`, if any.
    ///
    /// Both of a goal's two cells address it ([`Self::goal_covers`]), so the eraser works
    /// whichever half of the marker the user clicked — the flag included.
    ///
    /// This **can** leave the document with no goal at all, and that is deliberate. The
    /// flag a new level starts with is the user's to delete — a locked flag is a flag you
    /// cannot put where you want it — so the document allows the one state the writer
    /// refuses, and says so out loud: [`Self::validate`] and [`Self::to_json_pretty`] report
    /// "no goal", the editor's own verdict reads "NOT beatable: the level has no goal
    /// flag", and placing a flag anywhere puts the level right again.
    pub fn remove_goal_at(&mut self, tx: i32, ty: i32) -> bool {
        let before = self.goals.len();
        self.goals.retain(|g| !goal_marker_covers(g, tx, ty));
        if self.goals.len() != before {
            self.routes_cache = std::sync::OnceLock::new();
            true
        } else {
            false
        }
    }

    /// Every placed scenery item, in draw order.
    pub fn scenery(&self) -> &[Scenery] {
        &self.scenery
    }

    /// Whether a scenery sprite may stand at tile `(tx, ty)`.
    ///
    /// The cell has to be **empty**: scenery drawn inside a wall is the same lie a prop
    /// inside one would be. A tree (`*_base`) is two cells of art plus a cell of canopy, so
    /// it also needs the cells above it clear, which is the rule [`crate::level::Level`]
    /// already applies when the view scatters one by hash.
    pub fn scenery_placeable(&self, sprite: &str, tx: i32, ty: i32) -> bool {
        if self.tile(tx, ty) != Some(Tile::Empty) {
            return false;
        }
        if sprite.ends_with("_base") {
            for dy in 1..=2 {
                if self.tile(tx, ty - dy) != Some(Tile::Empty) {
                    return false;
                }
            }
        }
        true
    }

    /// Place a scenery sprite at a tile. Returns whether it landed.
    ///
    /// Refuses a cell that already holds the same sprite, so a click is idempotent, and
    /// cites [`Self::scenery_placeable`] for the rest of its rules.
    pub fn add_scenery(&mut self, sprite: &str, tx: i32, ty: i32) -> bool {
        if !self.scenery_placeable(sprite, tx, ty) {
            return false;
        }
        if self
            .scenery
            .iter()
            .any(|s| s.sprite == sprite && s.x == tx && s.y == ty)
        {
            return false;
        }
        self.scenery.push(Scenery {
            sprite: sprite.to_string(),
            x: tx,
            y: ty,
        });
        true
    }

    /// The sprite placed in tile `(tx, ty)`, if any. The **topmost** one wins when several
    /// share a cell, so what the eraser removes is what the eye sees on top.
    pub fn scenery_at(&self, tx: i32, ty: i32) -> Option<&str> {
        self.scenery
            .iter()
            .rev()
            .find(|s| s.x == tx && s.y == ty)
            .map(|s| s.sprite.as_str())
    }

    /// Remove one piece of scenery at a tile, if any. Returns whether it removed one.
    ///
    /// A piece is addressed by **any cell of the art the user can see**, the rule the goal's
    /// flag already follows: a plain prop by its own cell, and a tree (`*_base`) by its trunk
    /// or by either cell of the canopy drawn above it, because the leaves are the part of a
    /// tree a person aims at. A cell holding a piece of its own wins over a canopy reaching
    /// over it, so the eraser still takes what is under the pointer.
    pub fn remove_scenery_at(&mut self, tx: i32, ty: i32) -> bool {
        let exact = self.scenery.iter().rposition(|s| s.x == tx && s.y == ty);
        let index =
            exact.or_else(|| self.scenery.iter().rposition(|s| scenery_reaches(s, tx, ty)));
        match index {
            Some(index) => {
                self.scenery.remove(index);
                true
            }
            None => false,
        }
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
        if self.goals.len() > 1 {
            return Err(format!(
                "level has {} goals: a level has exactly one flag",
                self.goals.len()
            ));
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
        for s in &self.scenery {
            if s.sprite.trim().is_empty() {
                return Err("a scenery entity has no sprite name".into());
            }
            if tx_out(s.x, s.y, self.width, self.height) {
                return Err(format!(
                    "scenery {:?} at tile ({}, {}) is outside the grid",
                    s.sprite, s.x, s.y
                ));
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
        let mut entities: Vec<RawEntity> =
            Vec::with_capacity(self.walkers.len() + self.goals.len() + self.scenery.len());
        for (tx, ty) in self.walker_tiles() {
            entities.push(RawEntity {
                kind: "walker".into(),
                x: tx,
                y: ty,
                sprite: None,
            });
        }
        for (tx, ty) in self.goal_tiles() {
            entities.push(RawEntity {
                kind: "goal".into(),
                x: tx,
                y: ty,
                sprite: None,
            });
        }
        for s in &self.scenery {
            entities.push(RawEntity {
                kind: "scenery".into(),
                x: s.x,
                y: s.y,
                sprite: Some(s.sprite.clone()),
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

/// How many cells above its trunk a `*_base` tree grows, and therefore how far a click on
/// its leaves is from the entity that holds it.
///
/// Two, and not a view constant: it is the number [`Level::scenery_placeable`] already makes
/// a tree prove clear before it will plant one, so the cells the art occupies and the cells
/// the rule reserves are the same two cells by construction.
const TREE_CANOPY_CELLS: i32 = 2;

/// Whether a scenery piece's art reaches tile `(tx, ty)`: its own cell, plus the canopy above
/// it when it is a tree.
fn scenery_reaches(s: &Scenery, tx: i32, ty: i32) -> bool {
    let up = if s.sprite.ends_with("_base") {
        TREE_CANOPY_CELLS
    } else {
        0
    };
    s.x == tx && (s.y - up..=s.y).contains(&ty)
}

/// The tile a goal entity is anchored at: the cell its pole's base stands on.
fn goal_anchor(g: &Rect) -> (i32, i32) {
    (tile_of_x(g.x, g.w), tile_of_bottom(g.bottom()))
}

/// Whether a goal's two-cell marker covers `(tx, ty)`: the pole's base cell, or the cell the
/// flag hangs in directly above it.
fn goal_marker_covers(g: &Rect, tx: i32, ty: i32) -> bool {
    let (ax, ay) = goal_anchor(g);
    (tx, ty) == (ax, ay) || (tx, ty) == (ax, ay - 1)
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

    const PACK: [&str; 10] = [
        "l1-flat-run",
        "l2-first-gap",
        "l3-gauntlet",
        "l4-rhythm",
        "l5-tightrope",
        "l6-marathon",
        "l7-ascent",
        "l7-wall-school",
        "l8-long-jump",
        "l9-wait-and-climb",
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

    /// Placed scenery round-trips through the writer, stays inside the grid, and does not
    /// change the level's identity.
    ///
    /// Scenery is deliberately *out* of the content hash — it is inert, so decorating a
    /// level must not invalidate a champion trained on it — which makes this test the one
    /// that holds the file format for it.
    #[test]
    fn scenery_round_trips_stays_inside_the_grid_and_is_not_hashed() {
        let mut level = Level::starter("scenery", 20, 10);
        let bare = Level::starter("scenery", 20, 10);
        assert!(level.add_scenery("bush_c", 4, 7));
        assert!(level.add_scenery("tree_a_base", 10, 7));
        assert_eq!(level.scenery_at(4, 7), Some("bush_c"));
        assert_eq!(
            level.content_hash(),
            bare.content_hash(),
            "decoration must not change the level's identity"
        );

        // Refusals: a duplicate, a solid cell, a tree with no room above it, off-grid.
        assert!(!level.add_scenery("bush_c", 4, 7), "a duplicate is refused");
        assert!(!level.add_scenery("bush_c", 4, 9), "that cell is solid floor");
        level.set_tile(3, 5, Tile::Solid);
        assert!(!level.add_scenery("tree_b_base", 3, 7), "no room for a tree");
        assert!(!level.add_scenery("bush_c", -1, 7), "off the grid");

        let json = level.to_json_pretty().expect("a decorated level is writable");
        let back = Level::from_json_str(&json).expect("and it loads back");
        assert_eq!(back.scenery(), level.scenery());
        assert_eq!(back.content_hash(), level.content_hash());

        // The eraser removes one sprite from a cell, then reports there is nothing left.
        assert!(level.remove_scenery_at(4, 7));
        assert_eq!(level.scenery_at(4, 7), None);
        assert!(!level.remove_scenery_at(4, 7), "nothing left to remove");

        // A shrinking grid drops the scenery outside it, and says so.
        let report = level.resize(6, 8, Tile::Empty);
        assert_eq!(report.dropped_scenery, 1);
        assert!(report.dropped_anything());
        assert!(level.validate().is_ok());
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
        for tile in [Tile::Empty, Tile::Solid, Tile::Sand, Tile::Spike] {
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

    /// The flag a new level starts with is the user's to delete — and there is only ever one.
    ///
    /// The rule this replaces was the wrong way round on both counts: the document refused to
    /// remove its last goal, so the flag `Level::starter` plants on a fresh level could not be
    /// got rid of, and it accepted a second flag beside it without complaint.
    #[test]
    fn the_goal_can_be_erased_away_and_there_is_never_more_than_one() {
        let mut level = Level::starter("one-goal", 20, 10);
        assert_eq!(level.goals.len(), 1);

        assert!(level.remove_goal_at(18, 8), "the starter's own flag comes off");
        assert!(level.goals.is_empty());
        assert!(!level.remove_goal_at(18, 8), "already gone");

        // A document with no flag cannot be saved, and says why. That is the guard rail;
        // the flag itself is not locked.
        let err = level.validate().expect_err("no flag, no valid level");
        assert!(err.contains("no goal"), "{err}");
        assert!(level.to_json_pretty().is_err());

        // Placing one anywhere puts it right again.
        assert!(level.set_goal_tile(2, 8));
        assert_eq!(level.goal_tiles(), vec![(2, 8)]);
        assert_eq!(level.goals.len(), 1, "and there is still only one");
        assert!(level.validate().is_ok());
        assert!(!level.remove_goal_at(19, 19), "nothing there to remove");
    }

    /// **A level has one flag**: placing a goal *moves* the one that is there, the way
    /// placing the spawn moves the spawn.
    ///
    /// What is left refused is the marker the user can see: the goal answers for both of its
    /// cells, so a click on its own pole or cloth does not shove the marker one cell up the
    /// screen, and the cell one row below the pole is refused because *its* flag would hang in
    /// that pole.
    #[test]
    fn placing_a_goal_moves_the_one_that_is_already_there() {
        let mut level = Level::starter("one-goal", 20, 10);
        assert_eq!(level.goal_tiles(), vec![(18, 8)]);

        assert!(level.set_goal_tile(10, 6));
        assert_eq!(level.goal_tiles(), vec![(10, 6)], "the flag moved");
        assert_eq!(level.goals.len(), 1, "and it did not multiply");
        assert!(!level.goal_covers(18, 8), "nothing was left behind");
        assert!(level.validate().is_ok());

        assert!(!level.set_goal_tile(10, 6), "the pole's own cell");
        assert!(!level.set_goal_tile(10, 5), "the cell its cloth hangs in");
        assert!(
            !level.set_goal_tile(10, 7),
            "the cell whose flag would hang in its pole"
        );
        assert_eq!(level.goal_tiles(), vec![(10, 6)], "all three left it alone");

        // Two cells up is a marker of its own: the cloth is one cell tall, so the cell
        // *above* the flag is free again.
        assert!(
            level.set_goal_tile(10, 4),
            "the cell above the flag is open air"
        );
        assert_eq!(level.goal_tiles(), vec![(10, 4)]);

        // Off the map refuses the *move*: the flag stays where it was rather than being
        // dropped over the edge.
        assert!(!level.set_goal_tile(-1, 4), "off the map");
        assert!(!level.set_goal_tile(20, 4), "past the right edge");
        assert_eq!(level.goal_tiles(), vec![(10, 4)], "the flag stayed put");

        // The preview and the write agree, which is what keeps the ghost honest.
        assert!(level.goal_placeable(14, 8));
        assert!(!level.goal_placeable(10, 4));
        assert!(!level.goal_placeable(10, 3));
        assert!(!level.goal_placeable(20, 8));
    }

    /// A goal answers to both cells of its two-tile marker.
    ///
    /// This is the bug a user found by hand: the flag is what you aim at, the flag is drawn
    /// one cell *above* the goal's own tile, and an eraser that only knew the tile below it
    /// reported "nothing here" for the most obvious click in the editor.
    #[test]
    fn a_goal_is_addressed_by_its_flag_as_well_as_its_pole() {
        let mut level = Level::starter("flag-cell", 20, 10);
        // The starter's goal is anchored at (18, 8) with its cloth in (18, 7).
        assert!(level.goal_covers(18, 8), "the pole's base cell");
        assert!(level.goal_covers(18, 7), "the cell the flag is drawn in");
        assert!(!level.goal_covers(18, 6), "one cell above the flag is open air");
        assert!(!level.goal_covers(17, 7), "one column left is open air");

        // Erased by its cloth — the only part of the marker with any colour in it.
        assert!(level.remove_goal_at(18, 7), "erased by the flag, not the pole");
        assert!(level.goals.is_empty());

        // And by the pole's own cell, once the flag is back where it was.
        assert!(level.set_goal_tile(18, 8));
        assert!(level.remove_goal_at(18, 8), "and by the pole itself");
        assert!(level.goals.is_empty());
    }

    /// A tree is addressed by its canopy as well as its trunk — the same rule the goal's flag
    /// follows, for the same reason: the leaves are the part of a tree a person clicks.
    #[test]
    fn a_tree_is_erased_by_its_canopy_as_well_as_its_trunk() {
        let mut level = Level::starter("canopy", 20, 10);
        assert!(level.add_scenery("tree_a_base", 12, 7));
        assert!(level.remove_scenery_at(12, 6), "the lower canopy cell");
        assert_eq!(level.scenery_at(12, 7), None);

        assert!(level.add_scenery("tree_b_base", 12, 7));
        assert!(level.remove_scenery_at(12, 5), "the upper canopy cell");
        assert_eq!(level.scenery_at(12, 7), None);

        // A bush is one cell, and the air above it is not the bush.
        assert!(level.add_scenery("bush_c", 12, 7));
        assert!(!level.remove_scenery_at(12, 6), "air above a bush is open air");
        assert_eq!(level.scenery_at(12, 7), Some("bush_c"));
        assert!(level.remove_scenery_at(12, 7));

        // A cell holding a piece of its own takes the pointer's target first, even when a
        // tree's canopy reaches over it: one click, the thing under the cursor.
        assert!(level.add_scenery("tree_a_base", 12, 7));
        assert!(level.add_scenery("bush_c", 12, 6), "in the tree's canopy cell");
        assert!(level.remove_scenery_at(12, 6));
        assert_eq!(level.scenery_at(12, 6), None, "the bush under the pointer");
        assert_eq!(
            level.scenery_at(12, 7),
            Some("tree_a_base"),
            "the tree it was standing in front of survives one click"
        );
        assert!(level.remove_scenery_at(12, 6), "the second click takes the tree");
        assert_eq!(level.scenery_at(12, 7), None);
    }

    /// The start point has to stand on ground, in an empty cell, on the map.
    ///
    /// The refusal the editor shows as a red ghost rather than as a broken first frame: a
    /// spawn one row higher than the floor falls, a spawn inside the floor is stuck, and a
    /// spawn off the grid is a level that kills the player before frame 0.
    #[test]
    fn a_spawn_needs_an_empty_cell_with_ground_under_it() {
        let level = Level::starter("start", 20, 10);
        // `starter` lays a floor on the bottom row and stands the spawn on it.
        let (sx, sy) = level.spawn_tile();
        assert!(
            level.spawn_stands_on_ground(sx, sy),
            "the start the constructor laid down must satisfy the rule"
        );
        // One row up is mid-air: the player would fall the moment the level opened.
        assert!(
            !level.spawn_stands_on_ground(sx, sy - 1),
            "mid-air is not a start"
        );
        assert!(!level.spawn_stands_on_ground(sx, sy - 4));
        // The floor itself is occupied by a tile, and a player cannot start inside it.
        assert!(
            !level.spawn_stands_on_ground(sx, sy + 1),
            "a solid cell is not a start"
        );
        // And off the grid, in all four directions, including below it: falling out is a
        // death, not a placement.
        assert!(!level.spawn_stands_on_ground(-1, sy));
        assert!(!level.spawn_stands_on_ground(level.width, sy));
        assert!(!level.spawn_stands_on_ground(sx, -1));
        assert!(!level.spawn_stands_on_ground(sx, level.height));

        // Every material is ground: the rule asks `is_solid`, not `== Tile::Solid`, so a
        // start on a beach, a cellar floor or a plank deck is a start.
        let mut level = Level::starter("start-materials", 20, 10);
        for (tx, material) in [(5, Tile::Sand), (7, Tile::Brick), (9, Tile::Plank)] {
            for x in 0..level.width {
                level.set_tile(x, level.height - 1, material);
            }
            assert!(
                level.spawn_stands_on_ground(tx, level.height - 2),
                "a start on {material:?} is a start"
            );
            level.set_tile(tx, level.height - 2, Tile::Empty);
        }

        // And the span is the player's, not one point: a body over a gap with a tile beside
        // it has ground under it somewhere. The pair is the rule's own asymmetry, so it is
        // asked both ways rather than assumed.
        let mut level = Level::starter("start-span", 20, 10);
        for x in 0..level.width {
            level.set_tile(x, level.height - 1, Tile::Empty);
        }
        level.set_tile(1, level.height - 1, Tile::Solid);
        assert!(
            level.spawn_stands_on_ground(1, level.height - 2),
            "a cell with ground under part of the footprint is a start"
        );
        assert!(
            !level.spawn_stands_on_ground(5, level.height - 2),
            "a cell over nothing at all is not"
        );
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
        // The flag moves where it is wanted — the same call the editor's goal brush makes.
        assert!(level.set_goal_tile(30, 10));
        assert_eq!(level.goal_tiles(), vec![(30, 10)]);

        let report = level.resize(10, 8, Tile::Empty);
        // Both entities were past column 10, so the walker goes. The goal is the one thing a
        // resize may not drop, whatever the new grid does to it.
        assert_eq!(report.dropped_walkers, 1, "the walker was at column 35");
        assert!(report.goal_moved, "the goal was at column 30");
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

        // Two flags: a level has exactly one, and pushing a second one into the field
        // directly is caught at write time rather than written out.
        let mut level = Level::starter("invalid", 20, 10);
        level.goals.push(goal_rect(4, 8));
        let err = level.to_json_pretty().expect_err("two goals, no file");
        assert!(err.contains("exactly one flag"), "{err}");

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
