//! Level format (§6): a human-writable character grid plus an entity list.
//!
//! ```jsonc
//! {
//!   "format": "evo-platformer-level",
//!   "level_format_version": 3,
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
use crate::reach::{CachedVerdict, GoalRoutes, ReachOptions, ReachReport};
use crate::route_cache::Slot;

use crate::PIXELS_PER_TILE;

pub const LEVEL_FORMAT: &str = "evo-platformer-level";
/// **Bumped to 2 when the trap tiles' generation added a tile and four entity kinds.**
///
/// The version is a promise about what a file *means*, and the promise is what lets an
/// older build refuse a document instead of misreading one. A v1 build meeting a `=` row
/// would call the level corrupt (its glyph table is shorter), and one meeting a `turret`
/// would refuse the entity — but both would be reading a file labelled with a version that
/// says they can. So a format that grew new glyphs and new entity types is version 2.
///
/// **A v2 build reads v1**, because v2 only *added*: every v1 glyph, entity and rule means
/// exactly what it meant, and a v1 file has no field this build reads differently. That is
/// the constraint that makes accepting it safe, and it is why the shipped pack did not have
/// to be migrated to open in this build. The writer always emits 2.
///
/// **Bumped to 3 when the elements arrived** — lava (`L`), water (`~`) and vines (`V`). Same
/// rule, same reason: a v2 build meeting a `~` calls the level corrupt (its glyph table is
/// shorter), so a file that may contain one cannot say "version 2". Three glyphs, three new
/// meanings for a cell, so version 3. **A v3 build reads v1 and v2**, because this pass only
/// *added*: every earlier glyph, entity and rule means exactly what it meant. The writer
/// always emits 3.
///
/// **Bumped to 4 when the rope arrived** — one new entity and no new glyph. The same rule
/// applies to entities as to tiles: a v3 build meeting `"type": "rope"` refuses the file with
/// "unknown entity type", which is the right *behaviour* from a build that has no rope, and the
/// wrong *promise* from a file that says version 3. So the writer emits 4, a v4 build still reads
/// 1 through 3 (every earlier meaning is unchanged), and a champion trained on a rope level
/// carries 4 in its provenance like every other version this format has had.
pub const LEVEL_FORMAT_VERSION: u32 = 4;

/// Smallest grid the editor will build. Not a loader rule — a level is legal from
/// 1x1 — but a painter with less room than this has nothing to paint with.
pub const MIN_LEVEL_W: i32 = 8;
pub const MIN_LEVEL_H: i32 = 5;

/// Grid characters. Anything else in a level file is a load error, not a shrug.
///
/// **The materials are uppercase and their cracked selves are lowercase.** `S`, `B` and `W`
/// are sand, brick and timber; `s`, `b` and `w` are the same three materials about to fall
/// apart. Soil has no uppercase letter to fold — it is `#`, the ground every level has had
/// since the first one — so its brittle twin is `d`, for *dirt*. One rule covers the four:
/// a cracked tile is written as the material, lowercased.
pub const GLYPH_EMPTY: char = '.';
pub const GLYPH_SOLID: char = '#';
pub const GLYPH_SAND: char = 'S';
pub const GLYPH_BRICK: char = 'B';
pub const GLYPH_PLANK: char = 'W';
pub const GLYPH_SPIKE: char = '^';
pub const GLYPH_GOAL: char = 'G';
/// Cracked soil: solid until a body stands on it (`d`, for dirt).
pub const GLYPH_CRACKED: char = 'd';
pub const GLYPH_CRACKED_SAND: char = 's';
pub const GLYPH_CRACKED_BRICK: char = 'b';
pub const GLYPH_CRACKED_PLANK: char = 'w';
/// A one-way platform (`=`): ground from above, air from below and from the side.
pub const GLYPH_ONEWAY: char = '=';
/// A spring pad (`v`). Not solid, and it throws a **falling** body upward.
///
/// The inverted spike, deliberately: `^` is the tile that ends an episode and `v` is the
/// tile that extends one, and the two are one character apart in shape as well as in the
/// legend. Every other unclaimed symbol in a level file is either a letter a material wants
/// or a character that does not read as a thing standing in a cell.
pub const GLYPH_SPRING: char = 'v';

/// Water (`~`): not solid, not lethal, and **it pulls a body down**.
///
/// The glyph is the wave, which is the one ASCII character that reads as a fluid rather than
/// as a thing standing in a cell. A body whose centre is in a water cell sinks at
/// [`crate::physics::Physics::water_sink`] until it presses jump, which swims it up at
/// [`crate::physics::Physics::water_swim`] — and near the surface the same press is a push
/// *off* the surface, so a pool with a lip can be left. See [`Tile::Water`].
pub const GLYPH_WATER: char = '~';

/// Lava (`L`): not solid, and lethal on contact like a spike.
///
/// A capital letter rather than a symbol because it is the one hazard that is a *material*
/// as well as a hazard, and because the surviving symbols in the legend are already doing
/// work (`^` spike, `v` spring, `=` shelf, `~` water). It is also the only tile that
/// **fires**: a lava cell with open air above it is a vent, and it spits a ball up on a
/// rhythm. See [`Tile::Lava`].
pub const GLYPH_LAVA: char = 'L';

/// A vine (`V`): not solid, not lethal, and **climbable**.
///
/// `V` beside the spring's lowercase `v` is the legend's own joke and its own warning: the
/// two are the same character at different heights, and the one that is a *capital* is the
/// one that holds a body up rather than throwing it. A body overlapping a vine climbs it
/// while jump is held and slides down it when it is not. See [`Tile::Vine`].
pub const GLYPH_VINE: char = 'V';

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
    /// Cracked soil (`d`): solid, and **brittle** — a body that stands on it lights a fuse,
    /// and when the fuse burns out the tile is gone.
    ///
    /// ## What brittle ground is, and what it is not
    ///
    /// It is not a material. The four cracked variants are the four materials of
    /// [`Tile::Solid`], [`Tile::Sand`], [`Tile::Brick`] and [`Tile::Plank`] with a *behaviour*
    /// attached: each is drawn with its material's own autotile family (plus a crack), and it
    /// holds a body up exactly as that material does — right up to the moment it does not.
    /// That is why [`Tile::is_solid`] answers **yes** for a cracked tile, and why nothing about
    /// the collision code had to change to add it: an intact tile is ground, and the state the
    /// sim keeps is not "a brittle tile" but "this tile is gone", which it expresses by
    /// setting the cell to [`Tile::Empty`].
    ///
    /// The fuse itself lives in the sim ([`crate::Sim`]), not in the tile, for the reason the
    /// spike's kill zone lives in the physics rather than in the art: a tile is a fact about
    /// the level, and the level file is the same document before and after a body has crossed
    /// it. See [`crate::physics::CRUMBLE_TICKS`].
    Cracked = 7,
    /// Cracked sand (`s`). Solid and brittle, drawn as a beach. See [`Tile::Cracked`].
    CrackedSand = 8,
    /// Cracked brick (`b`). Solid and brittle, drawn as mortared brickwork.
    CrackedBrick = 9,
    /// Cracked timber (`w`). Solid and brittle, drawn as milled decking.
    CrackedPlank = 10,
    /// A spring pad (`v`): not solid, and it launches a **descending** body upward.
    ///
    /// Launches, not teleports: the pad sets the body's vertical velocity and touches nothing
    /// else, so the horizontal velocity the body arrived with is still the horizontal velocity
    /// it leaves with — which is what "maintaining momentum" means in a game whose whole
    /// difficulty is a jump's carry (`Physics::max_jump_distance`). A pad that zeroed `vx`
    /// would be a pad that punishes arriving fast, and a pad that *added* to `vx` would be a
    /// speed the oracle cannot derive and the level author cannot aim.
    ///
    /// **Only a falling body is launched.** A body whose `vy > 0` is one that is on its way
    /// down onto the pad; a body rising through it from below, or walking across it on the
    /// ground, is not. That is one rule rather than two and it is what makes a pad something
    /// you *land* on: the alternative (any overlap launches) makes a pad on a flat floor a
    /// thing you cannot walk past, because the body crossing it would be thrown upward by
    /// contact rather than by arrival.
    ///
    /// The launch is a fixed velocity every time rather than a fraction of the fall that
    /// arrived, so the pad's output is a property of the level and not of the approach, and a
    /// level built on one is repeatable — the same derivation the jump ladder is built on
    /// (`Physics::spring_height`). A body that lands back on the pad is launched again, so a
    /// pad with a ceiling over it pogoes for as long as the episode's no-progress timer allows;
    /// that is the pad working, not a bug.
    Spring = 11,
    /// A one-way platform (`=`): **ground from above, air from below and from the side.**
    ///
    /// The tile that gives the format a vertical route. A body falling onto it lands and can
    /// jump from it; a body rising through it from below passes straight through, and a body
    /// walking into its edge is not stopped. That is what makes a shelf something you jump
    /// *up through* rather than a ledge you go around, and it is the one collision shape the
    /// level format could not express before this.
    OneWay = 12,
    /// Water (`~`): a cell that **carries a body down** instead of holding it up.
    ///
    /// The first tile that changes the *body* rather than the terrain. Water is not solid,
    /// not opaque and not lethal: a body falls into it, and from the moment its centre is
    /// inside, gravity is replaced by a slow sink. Holding jump swims up; a press near the
    /// surface is a push off it. Horizontal speed is capped at
    /// [`Physics::water_speed`](crate::physics::Physics::water_speed), so a pool is crossed
    /// slowly, and that slowness *is* the cost of the route.
    ///
    /// **Submersion is decided by the body's centre**, not by any overlap. A body at the
    /// waterline whose centre is still above it is a body in the air, which makes the surface
    /// a line the physics agrees with the art about rather than a band of half-states — and it
    /// is what makes the bobbing at a surface read as bobbing rather than as a body that
    /// cannot decide what it is in.
    Water = 13,
    /// Lava (`L`): not solid, **lethal on contact**, and a **vent** when open to the air.
    ///
    /// Two mechanics in one cell, and they are deliberately the same material: the pool is
    /// what ends an episode that steps in it, and the pool's own surface is what throws the
    /// balls that end the episodes above it. A body is killed by the *cell* rather than by a
    /// drawn box the way a spike is, because a lava tile is full: there is no transparent band
    /// at its top for a body to stand in, and `tools/art/terrain.py::lava` fills the cell.
    ///
    /// A lava cell is a vent when the cell above it is open — not solid, not lava, not water.
    /// The vents and their rhythm live in [`crate::Sim`] because the balls are state; the
    /// *positions* are a fact about the level, which is why [`Level::lava_vents`] derives them
    /// once rather than the sim remembering them.
    Lava = 14,
    /// A vine (`V`): not solid, and **climbable**.
    ///
    /// The vertical route that needs no jump: while the body's box overlaps a vine and the
    /// body is off the ground, holding jump climbs at
    /// [`Physics::vine_climb`](crate::physics::Physics::vine_climb) and releasing it slides
    /// down at [`Physics::vine_slide`](crate::physics::Physics::vine_slide). Gravity is the
    /// vine's to replace while the body is on it, so a climber hangs still at the top of a
    /// stroke rather than falling off it.
    ///
    /// **A vine is not ground.** A body standing on the floor at the foot of a vine is
    /// standing on the floor, with its jump intact: the vine only takes over once the body is
    /// airborne, which is what lets a body jump onto a vine, grab it and climb, instead of
    /// being caught by the first one it walks past.
    Vine = 15,
}

impl Tile {
    /// Every tile the format has, in glyph order — the argument to any question shaped "all tiles".
    ///
    /// One array rather than a hand-written list at each place that wants one. Three do: the glyph
    /// round-trip test, the terminal's legend, and `train-cli`'s lane-label alphabet, which must not
    /// collide with anything the scene draws. Every one of them has been wrong in exactly the same
    /// way — a new tile arrives and the list it is not on does not notice — and the lane labels are
    /// the one where it shows: five letters were still being offered as agent names after the trap
    /// tiles took them (`d`, `s`, `b`, `w`, `v`). So a tile is added **here** as well as to the enum,
    /// and every list that matters is derived from this.
    pub const ALL: [Tile; 16] = [
        Tile::Empty,
        Tile::Solid,
        Tile::Sand,
        Tile::Brick,
        Tile::Plank,
        Tile::Spike,
        Tile::Goal,
        Tile::Cracked,
        Tile::CrackedSand,
        Tile::CrackedBrick,
        Tile::CrackedPlank,
        Tile::Spring,
        Tile::OneWay,
        Tile::Water,
        Tile::Lava,
        Tile::Vine,
    ];

    pub fn from_glyph(c: char) -> Option<Tile> {
        match c {
            GLYPH_EMPTY => Some(Tile::Empty),
            GLYPH_SOLID => Some(Tile::Solid),
            GLYPH_SAND => Some(Tile::Sand),
            GLYPH_BRICK => Some(Tile::Brick),
            GLYPH_PLANK => Some(Tile::Plank),
            GLYPH_SPIKE => Some(Tile::Spike),
            GLYPH_GOAL => Some(Tile::Goal),
            GLYPH_CRACKED => Some(Tile::Cracked),
            GLYPH_CRACKED_SAND => Some(Tile::CrackedSand),
            GLYPH_CRACKED_BRICK => Some(Tile::CrackedBrick),
            GLYPH_CRACKED_PLANK => Some(Tile::CrackedPlank),
            GLYPH_SPRING => Some(Tile::Spring),
            GLYPH_ONEWAY => Some(Tile::OneWay),
            GLYPH_WATER => Some(Tile::Water),
            GLYPH_LAVA => Some(Tile::Lava),
            GLYPH_VINE => Some(Tile::Vine),
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
            Tile::Cracked => GLYPH_CRACKED,
            Tile::CrackedSand => GLYPH_CRACKED_SAND,
            Tile::CrackedBrick => GLYPH_CRACKED_BRICK,
            Tile::CrackedPlank => GLYPH_CRACKED_PLANK,
            Tile::Spring => GLYPH_SPRING,
            Tile::OneWay => GLYPH_ONEWAY,
            Tile::Water => GLYPH_WATER,
            Tile::Lava => GLYPH_LAVA,
            Tile::Vine => GLYPH_VINE,
        }
    }

    /// Is there ground here: does a body stand on this tile?
    ///
    /// **The question the rest of the crate means when it says "solid".** Standability,
    /// landing zones, walkers' floor probes, the route graph, the oracle's platform table and
    /// the sensors' patch all want this answer, and a one-way platform is ground to every one
    /// of them — which is why it is included here rather than given a parallel concept at
    /// twenty call sites.
    ///
    /// **Cracked tiles are solid**, and that is the whole of what makes a brittle bridge a
    /// bridge rather than a row of holes: a level paints one, the sim stands a body on it, and
    /// the tile leaves the level later — as [`Tile::Empty`], which is a state the collision code
    /// already understands. A cracked tile answers no here only once the sim has replaced it.
    pub fn is_solid(self) -> bool {
        matches!(
            self,
            Tile::Solid
                | Tile::Sand
                | Tile::Brick
                | Tile::Plank
                | Tile::Cracked
                | Tile::CrackedSand
                | Tile::CrackedBrick
                | Tile::CrackedPlank
                | Tile::OneWay
        )
    }

    /// Is this tile a one-way platform: ground from above and air from every other side?
    ///
    /// The whole of the rule, and it is deliberately one question rather than three. A body
    /// falling onto it stands; a body rising through it passes; a body walking into its side
    /// passes. That is what makes a one-way platform a *route* rather than a wall, and it is
    /// why the collision code asks [`Tile::is_opaque`] for its side and ceiling tests while
    /// everything that asks "is there ground here" keeps asking [`Tile::is_solid`].
    pub fn is_one_way(self) -> bool {
        matches!(self, Tile::OneWay)
    }

    /// Does this tile stop a body travelling sideways or upward: the wall/ceiling question.
    ///
    /// Every solid tile but the one-way platform. The player's x resolution and ceiling
    /// resolution ask this, as does the oracle's "is this take-off inside a wall" check — a
    /// body standing *under* a shelf is not inside anything solid.
    pub fn is_opaque(self) -> bool {
        self.is_solid() && !self.is_one_way()
    }

    pub fn is_cracked(self) -> bool {
        matches!(
            self,
            Tile::Cracked | Tile::CrackedSand | Tile::CrackedBrick | Tile::CrackedPlank
        )
    }

    /// Is this tile a spring pad?
    pub fn is_spring(self) -> bool {
        matches!(self, Tile::Spring)
    }

    pub fn is_water(self) -> bool {
        matches!(self, Tile::Water)
    }

    pub fn is_lava(self) -> bool {
        matches!(self, Tile::Lava)
    }

    pub fn is_vine(self) -> bool {
        matches!(self, Tile::Vine)
    }

    /// Does this cell end an episode on contact?
    ///
    /// Two tiles, and they differ in *where* the killing box is rather than in whether there
    /// is one: a spike's poison is the drawn sprite's box ([`crate::physics::spike_lethal_box`]),
    /// a lava cell's is the cell. The question is what a caller that only wants "is this place
    /// death" should ask, so it does not have to know the difference.
    pub fn is_lethal(self) -> bool {
        matches!(self, Tile::Spike | Tile::Lava)
    }

    /// The tile this one is a *material of*, with any brittleness stripped off.
    ///
    /// The renderer's seam and autotile lookups are all about what a tile is made of, and
    /// brittleness is not part of that answer: `S` and `s` are both beaches, and a cracked
    /// beach beside a lawn has the same seam a sound one has. Rather than teach five tables
    /// about a fourth axis, a cracked tile is *drawn as its material* and the crack is drawn
    /// over it, which is exactly how the two relate in the world.
    pub fn base_material(self) -> Tile {
        match self {
            Tile::Cracked => Tile::Solid,
            Tile::CrackedSand => Tile::Sand,
            Tile::CrackedBrick => Tile::Brick,
            Tile::CrackedPlank => Tile::Plank,
            other => other,
        }
    }

    /// Is this tile only a different *material* of solid ground?
    ///
    /// The one question the renderer asks about sand: which autotile family to draw it in.
    /// It is deliberately not a question about collision — nothing that decides where the
    /// player goes may care what the ground is made of.
    ///
    /// A cracked tile answers yes for the material it is made of. The question is what the
    /// tile *is*, and a cracked beach is a beach; the brittleness is the [`Tile::is_cracked`]
    /// question, asked separately by the one caller that draws the crack.
    pub fn is_sand(self) -> bool {
        matches!(self, Tile::Sand | Tile::CrackedSand)
    }

    pub fn is_brick(self) -> bool {
        matches!(self, Tile::Brick | Tile::CrackedBrick)
    }

    pub fn is_plank(self) -> bool {
        matches!(self, Tile::Plank | Tile::CrackedPlank)
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
    /// A flyer's travel or a platform's travel, in tiles. Omitted from every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    span: Option<i32>,
    /// A turret's firing direction (`+1` right, `-1` left).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dir: Option<i8>,
    /// A moving platform's width, in tiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tiles: Option<i32>,
    /// A moving platform's axis: `"x"` (default, a slider) or `"y"` (a lift).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    axis: Option<String>,
}

impl RawEntity {
    /// An entity of `kind` anchored at a tile, with every optional field left out.
    fn of(kind: &str, x: i32, y: i32) -> RawEntity {
        RawEntity {
            kind: kind.to_string(),
            x,
            y,
            sprite: None,
            span: None,
            dir: None,
            tiles: None,
            axis: None,
        }
    }
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

/// A **flyer**: an enemy that ignores gravity and patrols a horizontal band.
///
/// The first enemy in the pack that is not standing on something, and that is the whole of
/// what it adds as a decision: height stops being safety. A jump over a walker is a timing
/// problem on one row; a jump past a flyer is a timing problem on the row the body has to
/// pass *through*, which is the row a jump is spent in.
///
/// Its span is authored rather than derived from the ground, because there is no ground: the
/// entity carries the two ends of its patrol in tiles, and the sim keeps it between them and
/// inside any wall it meets. Deterministic like every other patrol — the phase comes from the
/// episode seed through the same per-entity draw the walkers use, so a flyer level is
/// stochastic in the same, single way a walker level is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlyerSpec {
    /// The authored box, with its bottom edge on the bottom of its tile row.
    pub rect: Rect,
    /// Left end of the patrol, in world tiles.
    pub min_x: f32,
    /// Right end of the patrol, in world tiles.
    pub max_x: f32,
}

/// A flyer's beat when the file does not say one, in tiles **of travel**.
///
/// Twelve, and it is the same number the editor's beat field starts at, because the two used to
/// be two literals in two files: a level authored in the window and a level written by hand
/// disagreed about what "a flyer" means, and only one of them was the one the palette promised.
/// Twelve tiles is a fifth of a short level and long enough that the flyer is a *band* to time a
/// crossing against rather than a nuisance guarding one doorway — which is the whole of what the
/// creature adds as a decision.
pub const DEFAULT_FLYER_SPAN: i32 = 12;

impl FlyerSpec {
    /// A flyer anchored in tile `(tx, ty)`, with a beat of `span` tiles **of travel**, centred
    /// on the cell.
    ///
    /// **`span` is the distance the box moves**, not the distance between two abstract ends. The
    /// two are not the same number: a box has width, so bounds that name its left edge at
    /// `centre ± span/2` move it `span - w`. That half-tile loss was invisible on a twelve-tile
    /// beat and enormous on a four-tile one ("my flyer only patrols three tiles"), and it is the
    /// sort of arithmetic a level author should never have to do. Here the box's own width is
    /// added to both ends, so the centre of the flyer goes exactly `span` tiles and the beat is
    /// symmetric about the cell that was clicked.
    pub fn at(tx: i32, ty: i32, span: i32) -> FlyerSpec {
        let span = span.max(1) as f32;
        let w = PHYSICS.flyer_w;
        let centre = tx as f32 + 0.5;
        FlyerSpec {
            rect: Rect::new(
                centre - w * 0.5,
                crate::physics::bottom_of(ty) - PHYSICS.flyer_h,
                w,
                PHYSICS.flyer_h,
            ),
            min_x: centre - span * 0.5 - w * 0.5,
            max_x: centre + span * 0.5 + w * 0.5,
        }
    }

    /// The beat's length, in tiles **of travel**: the distance the box moves.
    ///
    /// The inverse of [`FlyerSpec::at`], and the only correct way to read the span back out of a
    /// spec: the authored ends name the box's edges, so the distance between them is the travel
    /// *plus the box's own width*. Writing that number back to a file as the span is how a
    /// save/load round trip makes a flyer's beat grow by 0.7 tiles every time a level is saved.
    pub fn span(&self) -> f32 {
        (self.max_x - self.min_x - PHYSICS.flyer_w).max(0.0)
    }
}

/// A **turret**: a fixed enemy that fires a shot along the row it stands on.
///
/// The one enemy that attacks at a distance, so it is the first thing in the pack a body has
/// to cross *while* it is crossing something else. It does not move, does not chase and is
/// **solid** rather than stompable — a body stands on its lid and is stopped by its sides, and
/// the box itself never kills — which keeps it a level-design tool rather than a second game:
/// the shot is a period the author sets by where they put it, and the answer is always a route
/// or a jump over the muzzle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurretSpec {
    pub rect: Rect,
    /// `+1` fires right, `-1` fires left. The file carries it because a turret's whole
    /// meaning is which way its row is dangerous, and inferring it from a wall would make
    /// two visually different placements behave the same.
    pub dir: i8,
}

/// A **moving platform**: a shelf that slides along one axis on a fixed cycle.
///
/// Authored as a box plus the length of its travel; the position at a tick is a pure function
/// of the episode's frame count ([`crate::physics::Mover::rect_at`]), so two runs of one seed
/// agree and the state hash needs no extra state for it. It carries a body standing on it,
/// which is the reason it is worth the machinery: a platform that did not carry you would be
/// a decoration, and a lift that did not is a staircase.
///
/// **The oracle models it as a ride**, not as terrain: the phase of the turn is a fact about
/// the episode, so the sweep does not simulate the shelf's motion at all. What it asks instead
/// is the question an author's level actually depends on — *can a body stand on this shelf and
/// be carried to the far end of its travel?* — and a body that can reach the shelf at its home
/// can always wait for the cycle to bring it there. See [`crate::reach`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlatformSpec {
    /// The box where the shelf **starts**: `w` tiles wide and half a tile tall, its bottom edge
    /// on the bottom of row `y` (so the deck is mid-row). `dir` says which way it leaves it.
    pub rect: Rect,
    /// How far it travels, in tiles. Never negative: the *direction* is [`Self::dir`].
    pub span: f32,
    /// True for a lift (travels vertically), false for a slider (travels horizontally).
    pub vertical: bool,
    /// Which way it travels: `+1` right (or up, for a lift), `-1` left (or down).
    ///
    /// The file carries it for the same reason a turret carries its firing direction: a shelf
    /// whose only use is arriving from the left is a different level from one that goes right,
    /// and a sign on the span would make the two indistinguishable from the box alone. A
    /// missing one is `+1`, which is every shelf written before the field existed.
    pub dir: i8,
}

/// A platform's width and travel when the file does not say: three tiles wide, four of travel.
///
/// Shared with the editor for the same reason [`DEFAULT_FLYER_SPAN`] is — one default, in the
/// module that defines the entity, so a shelf placed in the window and one written by hand are
/// the same shelf.
pub const DEFAULT_PLATFORM_TILES: i32 = 3;
pub const DEFAULT_PLATFORM_SPAN: i32 = 4;

impl PlatformSpec {
    /// A shelf anchored at tile `(tx, ty)`, `tiles` wide, travelling `span` tiles towards `dir`
    /// (`+1` right, or up for a lift; `-1` left, or down).
    ///
    /// The one place the geometry of a shelf is stated. The loader and the editor's platform
    /// brush both come through here, because two copies of "half a tile tall, bottom edge on the
    /// row below the cell" is exactly the shape of bug that has the editor drawing a shelf where
    /// the sim will not put it.
    pub fn at(tx: i32, ty: i32, tiles: i32, span: i32, vertical: bool, dir: i8) -> PlatformSpec {
        PlatformSpec {
            rect: Rect::new(
                tx as f32,
                crate::physics::bottom_of(ty) - PHYSICS.platform_h,
                tiles.max(1) as f32,
                PHYSICS.platform_h,
            ),
            span: span.max(1) as f32,
            vertical,
            dir: if dir < 0 { -1 } else { 1 },
        }
    }
}

/// A **rope**: a length of line hanging from the underside of a tile that a body can grab in
/// the air and swing on.
///
/// The pack's third way across a gap, and the first that is neither a timed move nor a climb.
/// A vine is a vertical route the body *holds*: the brain decides to climb and the body goes up at
/// a constant rate. A rope is a route the body *rides*: once it is gripped, the body's motion is
/// the rope's, and the only way to change it is to pump the arc with a direction and to decide
/// when to let go. That is why it is worth the state — it is the one object in the pack whose
/// outcome is decided by momentum rather than by where the body is.
///
/// **Hangs from the bottom of row `row`, and its own cells are the rows below.** The anchor is a
/// point — the middle of the underside of the tile above `(col, row)` — rather than a box, because
/// a rope has no thickness: it is a line, its length is the entity's, and a body grabs it wherever
/// its hands happen to meet it. Those cells are not solid, not hazards and not tiles; they are
/// only where the rope is drawn and where the grip is looked for.
///
/// `length` is in tiles and is the full length of the line. A body that grabs near the anchor is
/// on a short pendulum and a body that grabs near the end is on a long one, which is the whole of
/// the skill the object asks for: the same rope is a different swing depending on where you catch
/// it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RopeSpec {
    /// Column the line hangs in.
    pub col: i32,
    /// Row the line's **topmost** cell occupies. The line is tied to the underside of the row
    /// above it — which is what [`Level::rope_placeable`] demands — and hangs `length` tiles down
    /// from there, so a rope's drawn cells and the segment it swings on are the same cells.
    pub row: i32,
    /// The line's full length, in tiles. Never less than one.
    pub length: f32,
}

/// A rope's length when the file does not say one: five tiles.
///
/// Shared with the editor for the reason every other default in this module is — a rope placed
/// in the window and a rope written by hand have to be the same rope. Five tiles is a little over
/// one full jump's height (2.5), so a body can grab it from a jump and still have an arc left
/// underneath: a one-jump rope is a vine with extra steps, and a fifteen-tile rope is a lift.
pub const DEFAULT_ROPE_TILES: i32 = 5;

impl RopeSpec {
    /// A rope hung at tile `(tx, ty)` with `length` tiles of line.
    ///
    /// The one place the geometry is stated: the anchor is the centre of the **top** edge of row
    /// `ty`, so a rope placed in the air under a ceiling is tied to that ceiling's underside and
    /// its first drawn cell is the first cell of the line. The alternative — tying it to the
    /// bottom of the row it is placed in — leaves a tile of air between the ceiling and the rope,
    /// which is a rope that looks like it is floating.
    pub fn at(tx: i32, ty: i32, length: i32) -> RopeSpec {
        RopeSpec {
            col: tx,
            row: ty,
            length: length.max(1) as f32,
        }
    }

    /// Where the line is tied: the centre of the row's top edge, in world tiles.
    pub fn anchor(&self) -> (f32, f32) {
        (self.col as f32 + 0.5, self.row as f32)
    }

    /// The lowest point of the line, where its end hangs at rest.
    pub fn end_y(&self) -> f32 {
        self.anchor().1 + self.length
    }
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
    /// Chasers, flyers and turrets the new grid had no room for.
    pub dropped_enemies: usize,
    /// Moving platforms the new grid had no room for.
    pub dropped_platforms: usize,
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
        self.dropped_walkers > 0
            || self.dropped_enemies > 0
            || self.dropped_platforms > 0
            || self.dropped_scenery > 0
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
    /// Chasers: walkers that hunt the body instead of patrolling a span. Simulated, hashed
    /// and sensed exactly as the walkers are, in their own list so that adding one cannot
    /// renumber a level's patrols (the enemy vector's tie-break is walker index).
    pub chasers: Vec<Rect>,
    /// Flyers, with the patrol each one walks.
    pub flyers: Vec<FlyerSpec>,
    /// Turrets, with the direction each one fires.
    pub turrets: Vec<TurretSpec>,
    /// Moving platforms, with the travel each one slides over.
    pub platforms: Vec<PlatformSpec>,
    /// Ropes, with the length each one hangs: a swing a body can grab in the air.
    pub ropes: Vec<RopeSpec>,
    /// Lazily computed move-graph route table for this level.
    ///
    /// **The slot is shared between clones, not copied by them.** The graph is a pure
    /// function of the terrain and costs a full sweep of every stand's canonical moves
    /// ([`crate::reach::goal_routes`] — 24 ms on a 60x15 level), so the editor's document,
    /// the run's copy of it, the lanes animated over it and every episode's own sim are all
    /// asking the same question about the same terrain and should be answered by one build.
    ///
    /// A bare `OnceLock` could not do that: `Clone` copies it, so a clone taken *before* the
    /// first build started with an empty slot and paid the whole sweep itself. That is what
    /// a generation full of lanes, and a replay full of ghosts, each charged once, looked
    /// like on screen: a 24 ms stall on the frame a lane first asked for a waypoint.
    ///
    /// A mutation replaces the `Arc` rather than filling it ([`Level::forget_routes`]), which
    /// keeps two things true at once: a level whose terrain has changed gets a graph built
    /// for the terrain it has now, and the clones that still hold the old terrain keep the
    /// graph they were built for.
    routes_cache: std::sync::Arc<std::sync::OnceLock<std::sync::Arc<GoalRoutes>>>,
    /// The route cache beside this level's file, for a level that came from one.
    ///
    /// Both things a level's terrain is asked for are already a function of the terrain — the
    /// route table above, and the oracle's verdict ([`crate::reach::report`]) — so both are
    /// kept beside the file the terrain came from, keyed by its content hash and the two
    /// behaviour versions of the oracle that produced them ([`crate::route_cache`]). Opening
    /// a level in a fresh process then costs a file read instead of a sweep of every canonical
    /// move on it. It can never *change* an answer: what comes back is what the same code
    /// would have computed for the same key.
    ///
    /// It is dropped by **every writer of the grid**: [`Level::forget_routes`] for an edit, and
    /// [`Level::burn_tile`] for an episode's crumbling ground. That is what keeps the key
    /// honest — while a slot is here, this document is still the document its file describes,
    /// so [`Level::content_hash_string`] is the hash its answers were computed for.
    cache: Option<Slot>,
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
        if !(1..=LEVEL_FORMAT_VERSION).contains(&raw.level_format_version) {
            return Err(format!(
                "level_format_version {} is not supported (this build reads 1..={})",
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
                let tile = Tile::from_glyph(ch)
                    .ok_or_else(|| format!("row {row} column {col}: unknown tile glyph {ch:?}"))?;
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
            chasers: Vec::new(),
            flyers: Vec::new(),
            turrets: Vec::new(),
            platforms: Vec::new(),
            ropes: Vec::new(),
            goals: Vec::new(),
            scenery: Vec::new(),
            routes_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
            cache: None,
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
                // A walker that hunts: same box, same stomp, different brain. It is a
                // separate entity kind rather than a flag on the walker so that a level file
                // reads as what it does, and so an older build refuses it by name instead of
                // silently dropping the hunt.
                "chaser" => level.chasers.push(Rect::new(
                    e.x as f32 + (1.0 - PHYSICS.walker_w) * 0.5,
                    crate::physics::bottom_of(e.y) - PHYSICS.walker_h,
                    PHYSICS.walker_w,
                    PHYSICS.walker_h,
                )),
                // A flyer's span is authored in tiles of travel, **centred on its cell**: the
                // file says where it beats and how far, not two absolute ends, so a moved level
                // keeps its patrol. [`FlyerSpec::at`] is the whole of that geometry, and
                // `DEFAULT_FLYER_SPAN` is the whole of the default.
                "flyer" => {
                    let span = e.span.unwrap_or(DEFAULT_FLYER_SPAN);
                    level.flyers.push(FlyerSpec::at(e.x, e.y, span));
                }
                "turret" => level.turrets.push(TurretSpec {
                    rect: Rect::new(
                        e.x as f32 + (1.0 - PHYSICS.turret_w) * 0.5,
                        crate::physics::bottom_of(e.y) - PHYSICS.turret_h,
                        PHYSICS.turret_w,
                        PHYSICS.turret_h,
                    ),
                    dir: if e.dir.unwrap_or(1) < 0 { -1 } else { 1 },
                }),
                // A platform's box is anchored like every other entity: its **bottom edge**
                // rests on the bottom of row `y`, so the shelf hangs in its own cell with its
                // deck half a tile above the row's bottom — a shelf authored in the row above
                // a floor sits half a tile over that floor, which is a hop up and a step off.
                // It is `tiles` wide and half a tile tall. The travel is upward for a lift and
                // rightward for a slider, which is why the axis is a name in the file rather
                // than a sign on the span. [`PlatformSpec::at`] owns the geometry so the
                // editor's brush and this reader cannot disagree about it.
                "platform" => {
                    let axis = e.axis.as_deref();
                    let vertical = matches!(axis, Some("y") | Some("up") | Some("down"));
                    // `dir` is the field a shelf shares with a turret, and it means the same
                    // thing: which way the thing goes. A file written before it existed has
                    // none, and reads as the `+1` every shelf used to be. `axis: "down"` is
                    // the one axis name that says the direction on its own, so it is honoured
                    // as one — a lift told to go down and then sent up is the kind of quiet
                    // disagreement between file and picture the editor exists to prevent.
                    let dir = e.dir.unwrap_or(if axis == Some("down") { -1 } else { 1 });
                    level.platforms.push(PlatformSpec::at(
                        e.x,
                        e.y,
                        e.tiles.unwrap_or(DEFAULT_PLATFORM_TILES),
                        e.span.unwrap_or(DEFAULT_PLATFORM_SPAN),
                        vertical,
                        dir,
                    ));
                }
                // A rope hangs from the **bottom of its own row**: the cell it is authored in
                // is the line's first cell, and the tile it is tied to is the one above it, so
                // the anchor is the underside of the ceiling an author would have clicked on.
                // The line's cells have to be inside the map — a rope that ends below the world
                // is a rope whose swing could carry a body out of the level without falling —
                // and that is refused here rather than clamped, because a file that asks for it
                // is a file whose author meant something else.
                "rope" => {
                    let length = e.tiles.unwrap_or(DEFAULT_ROPE_TILES).max(1);
                    if e.x < 0 || e.x >= level.width || e.y < 1 || e.y + length > level.height {
                        return Err(format!(
                            "rope at ({}, {}) of length {} does not hang inside a {}x{} level",
                            e.x, e.y, length, level.width, level.height
                        ));
                    }
                    level.ropes.push(RopeSpec::at(e.x, e.y, length));
                }
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
    ///
    /// This is the constructor that knows the level has a *file*, so it is the one that looks
    /// for — and later writes — the route cache beside it ([`crate::route_cache`]).
    pub fn from_json_file(path: impl AsRef<std::path::Path>) -> Result<Level, String> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut level =
            Level::from_json_str(&src).map_err(|e| format!("{}: {e}", path.display()))?;
        level.adopt_cache(path);
        Ok(level)
    }

    /// Take the answers kept beside `path`, when they are answers about this terrain.
    ///
    /// Adoption rather than computation is the whole point of the file: a graph that arrives
    /// this way is in `routes_cache` before anything asks for it, so the first ask is an
    /// atomic load, and a verdict that arrives this way is what [`crate::reach::report`]
    /// returns without sweeping.
    fn adopt_cache(&mut self, path: &std::path::Path) {
        let slot = Slot::new(crate::route_cache::sidecar_path(path));
        if let Some((routes, verdict)) = crate::route_cache::read(self, slot.path()) {
            if let Some(routes) = routes {
                self.routes_cache =
                    std::sync::Arc::new(std::sync::OnceLock::from(std::sync::Arc::new(routes)));
            }
            if let Some(verdict) = verdict {
                slot.remember(verdict);
            }
        }
        self.cache = Some(slot);
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
            self.forget_routes();
        }
    }

    /// Take a tile out of the ground during an episode — a brittle tile burning out —
    /// **without** dropping the route table.
    ///
    /// A route table ([`Level::routes`]) answers "which way is the flag" for the level as it
    /// was *authored*; an episode's own crumbling is not an edit to the level. The flag has
    /// not moved, so the compass the waypoint sensor steers by should not swing because a
    /// bridge gave way under one agent's feet — and the geodesic `hops_to_flag` the score
    /// reads stays a reading of the level's route rather than of one body's damage.
    ///
    /// The **physics is exact either way**: it reads the live grid ([`Level::get`]) on every
    /// tick, so the hole a body has just opened is a hole it falls through, and
    /// [`Level::tile`] shows the ground the episode actually has. What is shared is only the
    /// route table, which is why this is not [`Level::set_tile`].
    ///
    /// It is also what keeps a crumbling level affordable. The sweep behind a route table
    /// costs milliseconds, and an episode can break several tiles: rebuilding per cracked
    /// tile was the largest cost in training (and in *watching*) a level with brittle ground.
    ///
    /// The **file's** cache goes, though, which is the opposite rule for the opposite reason
    /// ([`Level::cache`]): a route table answers for the level as it was authored, but a cache
    /// beside a file has to key itself on the content it describes, and this body has just
    /// changed the content. A clone of the level that has broken a tile is no longer holding
    /// the terrain its file does, so it must not write that file's cache — the model level,
    /// which never burns anything, still does.
    pub fn burn_tile(&mut self, tx: i32, ty: i32) {
        if tx >= 0 && tx < self.width && ty >= 0 && ty < self.height {
            self.tiles[(ty * self.width + tx) as usize] = Tile::Empty;
            self.cache = None;
        }
    }

    /// Drop this level's move-graph route table, because the terrain it described has just
    /// changed.
    ///
    /// A fresh slot *for this level* rather than a clear of the shared one: the clones that
    /// are still holding the terrain the graph was built for keep it (see
    /// [`Level::routes_cache`]). Every writer of the grid or of the flag calls this —
    /// [`Level::set`] for a tile, [`Level::resize`] for the grid, and the goal edits — so the
    /// graph can never describe terrain the level no longer has.
    fn forget_routes(&mut self) {
        self.routes_cache = std::sync::Arc::new(std::sync::OnceLock::new());
        // An edit also gives up this level's claim to its file's cache: the document is no
        // longer the document the file describes, so it neither answers from it nor writes to
        // it ([`Level::cache`]).
        self.cache = None;
    }

    /// Return the move-graph route table for this level, computing and caching it on first ask.
    ///
    /// The first ask does the sweep; every later ask from this level or from any clone of it
    /// is an atomic load of the one shared answer — and for a level loaded from a file, the
    /// first ask is usually not even a sweep: the answer is there in the cache beside the file,
    /// adopted at load (`Level::adopt_cache`).
    ///
    /// A **build** here is the one moment this level writes that cache, because a build is the
    /// one proof that the file's copy could not answer — a matching key is adopted before this
    /// function is ever reached. A level that adopted its graph therefore leaves the file alone
    /// rather than rewriting what it just read.
    pub fn routes(&self) -> &GoalRoutes {
        let cold = self.routes_cache.get().is_none();
        let routes = self
            .routes_cache
            .get_or_init(|| std::sync::Arc::new(crate::reach::goal_routes(self)));
        if cold {
            self.write_cache(Some(&**routes), self.known_verdict());
        }
        routes
    }

    /// The oracle's verdict for this level, when its file has already been given one for
    /// exactly these abilities.
    ///
    /// `None` is the ordinary answer — a level with no file behind it, a document that has been
    /// edited since it was loaded, a cache written by another build, or a question about
    /// abilities the cache does not hold — and every one of them means "sweep it", never an
    /// error.
    pub(crate) fn cached_verdict(&self, options: ReachOptions) -> Option<ReachReport> {
        let cached = self.cache.as_ref()?.verdict()?;
        (cached.options == options).then(|| cached.report.clone())
    }

    /// Keep `report` as this level's verdict, and put it in the file's cache.
    ///
    /// Only the oracle's own question is kept — [`crate::reach::ReachOptions::default`]. A
    /// verdict for weakened abilities is a different claim about the same terrain, and the
    /// consumers that ask for one are tests checking that a level *needs* an ability
    /// (`crates/sim/tests/levels.rs`); giving each of those its own entry would rewrite one
    /// file per ability set and hand the editor a miss for the verdict it actually wants.
    pub(crate) fn remember_verdict(&self, options: ReachOptions, report: &ReachReport) {
        if options != ReachOptions::default() {
            return;
        }
        let Some(slot) = self.cache.as_ref() else {
            return;
        };
        slot.remember(CachedVerdict {
            options,
            report: report.clone(),
        });
        self.write_cache(self.routes_cache.get().map(|r| &**r), self.known_verdict());
    }

    /// The verdict this level's file has, as the slot holds it.
    fn known_verdict(&self) -> Option<&std::sync::Arc<CachedVerdict>> {
        self.cache.as_ref()?.verdict()
    }

    /// Write this level's cache beside its file: the graph and the verdict, whichever of the
    /// two this level has, and nothing at all for a level that has no file or has been edited
    /// since it was loaded.
    fn write_cache(
        &self,
        routes: Option<&GoalRoutes>,
        verdict: Option<&std::sync::Arc<CachedVerdict>>,
    ) {
        let Some(slot) = self.cache.as_ref() else {
            return;
        };
        crate::route_cache::write(self, slot, routes, verdict.map(|v| &**v));
    }

    pub fn is_solid(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty).is_solid()
    }

    pub fn is_spike(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty) == Tile::Spike
    }

    pub fn is_water(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty).is_water()
    }

    pub fn is_lava(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty).is_lava()
    }

    pub fn is_vine(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty).is_vine()
    }

    /// The top of the water column containing `(tx, ty)`: the surface a swimmer is measured
    /// against, or `None` when the cell is not water.
    ///
    /// A column, because a body swims up a *shaft* and the surface it has to reach is the top
    /// of that shaft, not the cell beside it. The scan is bounded by the grid's own height and
    /// walks up a run of water cells, so it costs what the pool is deep.
    pub fn water_top(&self, tx: i32, ty: i32) -> Option<f32> {
        if !self.is_water(tx, ty) {
            return None;
        }
        let mut top = ty;
        while top > 0 && self.is_water(tx, top - 1) {
            top -= 1;
        }
        Some(top as f32)
    }

    /// Every lava cell open to the air: the vents a lava ball is spat from.
    ///
    /// Row-major, so the order — and therefore the rhythm each vent fires on, which is derived
    /// from its index — is a property of the level rather than of a hash map's iteration order.
    /// "Open" is the cell above being neither solid nor another liquid: a lava cell under a
    /// ceiling, or under more lava, is not a surface and does not fire.
    pub fn lava_vents(&self) -> Vec<(i32, i32)> {
        let mut vents = Vec::new();
        for ty in 0..self.height {
            for tx in 0..self.width {
                if !self.is_lava(tx, ty) {
                    continue;
                }
                let above = self.get(tx, ty - 1);
                if !above.is_solid() && !above.is_lava() && !above.is_water() {
                    vents.push((tx, ty));
                }
            }
        }
        vents
    }

    /// Does this cell stop a body travelling sideways or upward?
    ///
    /// The wall/ceiling question, as opposed to [`Level::is_solid`]'s "is there ground here".
    /// A one-way platform is the difference between the two and the reason both exist.
    pub fn is_opaque(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty).is_opaque()
    }

    /// Is there a spring pad in this cell?
    pub fn is_spring(&self, tx: i32, ty: i32) -> bool {
        self.get(tx, ty).is_spring()
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
            && !self.is_lava(tx, surface_row - 1)
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
            .map(|ty| (0..self.width).map(|tx| self.get(tx, ty).glyph()).collect())
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
        self.hash_core(&mut h);
        h.finish()
    }

    /// The half of the hash that `content_hash` and [`Level::scene_hash`] share: the name, the
    /// grid, the spawn, the patrols and the goal.
    ///
    /// Split out so the two hashes cannot disagree about the terrain. They differ only in the
    /// entities appended after this, and appending nothing is exactly the old byte stream —
    /// which is what keeps every content hash this build has ever written still valid.
    fn hash_core(&self, h: &mut StateHasher) {
        for b in self.name.as_bytes() {
            h.u8(*b);
        }
        h.u32(self.width as u32);
        h.u32(self.height as u32);
        for t in &self.tiles {
            h.u8(*t as u8);
        }
        self.spawn.hash_into(h);
        h.u32(self.walkers.len() as u32);
        for w in &self.walkers {
            w.hash_into(h);
        }
        h.u32(self.goals.len() as u32);
        for g in &self.goals {
            g.hash_into(h);
        }
    }

    /// The plan's `"sha256:…"` metadata string, with the algorithm this build uses.
    pub fn content_hash_string(&self) -> String {
        format!("fnv1a64:{:016x}", self.content_hash())
    }

    /// A hash of **everything an episode can see**: [`Level::content_hash`] plus the entities it
    /// leaves out — chasers, flyers, turrets and moving platforms.
    ///
    /// This is the identity a *run* is kept against. A run is a claim about the episodes its
    /// population was scored on, and placing a chaser changes those episodes while leaving the
    /// terrain untouched; keying the run on [`Level::content_hash`] alone meant the window
    /// decided the run had not moved, kept its copy of the level, and drew the evolve view on a
    /// level that no longer had the chaser in it. The editor and the playtest read the document
    /// directly, so they showed it — the one view that did not was the one with a stale copy.
    ///
    /// Deliberately *not* the champion's [`Level::content_hash`]: a saved champion is a claim
    /// about the route it learned over the terrain, and an enemy added later does not make that
    /// route a claim about a different level. Keeping the two hashes separate keeps every
    /// champion and population file on disk loadable, while the run still restarts when an
    /// enemy moves.
    pub fn scene_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        self.hash_core(&mut h);
        h.u32(self.chasers.len() as u32);
        for c in &self.chasers {
            c.hash_into(&mut h);
        }
        h.u32(self.flyers.len() as u32);
        for f in &self.flyers {
            f.rect.hash_into(&mut h);
            h.f32(f.min_x);
            h.f32(f.max_x);
        }
        h.u32(self.turrets.len() as u32);
        for t in &self.turrets {
            t.rect.hash_into(&mut h);
            h.i8(t.dir);
        }
        h.u32(self.platforms.len() as u32);
        for p in &self.platforms {
            p.rect.hash_into(&mut h);
            h.f32(p.span);
            h.u8(p.vertical as u8);
            h.i8(p.dir);
        }
        h.finish()
    }

    /// [`Level::scene_hash`] in the metadata string form the run's packets carry.
    pub fn scene_hash_string(&self) -> String {
        format!("fnv1a64:{:016x}", self.scene_hash())
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
    /// Does this level draw a **patrol phase** per episode?
    ///
    /// Walkers, chasers and flyers are placed from the episode seed, so the same genome meets a
    /// different world on every seed. This is the one answer the trainer's auto-trial rule and
    /// the auto-shape scan share; they used to disagree — the scan counted chasers and flyers as
    /// stochastic, the trial rule counted only walkers — and a chaser level like
    /// `l11-the-elements` was therefore trained and saved on a single phase (`l11` logs `trials 1`
    /// despite `auto` naming `patrols`). The difference is how a champion is found that wins once
    /// and not again: the record was a maximum over one phase, not a policy that handles phases.
    pub fn has_patrols(&self) -> bool {
        !self.walkers.is_empty() || !self.chasers.is_empty() || !self.flyers.is_empty()
    }

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
            chasers: Vec::new(),
            flyers: Vec::new(),
            turrets: Vec::new(),
            platforms: Vec::new(),
            ropes: Vec::new(),
            goals: Vec::new(),
            scenery: Vec::new(),
            routes_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
            cache: None,
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
        let mut out = Level::starter(format!("{} mirrored", self.name), self.width, self.height);
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

        // The later enemies and the movers, mirrored by the same far-edge rule. Three of
        // them carry something that has to turn with them: a turret's **firing direction**
        // is a fact about which way its row is dangerous, so a mirrored turret fires left
        // from its new home; a flyer's beat is stored as two ends, which swap; and a
        // **slider's travel** is a direction in the mirrored axis, so a shelf that used to
        // go right now goes left from its mirrored home — otherwise the mirror of a level
        // whose only ride arrives from the left would be a level whose ride leaves.
        // A lift's up/down is not the mirrored axis and does not turn.
        out.chasers = self
            .chasers
            .iter()
            .map(|r| Rect::new(w - (r.x + r.w), r.y, r.w, r.h))
            .collect();
        // A rope is one column and one length, so the mirror is its column: the line stays
        // where it hangs and the anchor stays under the same ceiling.
        out.ropes = self
            .ropes
            .iter()
            .map(|r| RopeSpec {
                col: self.width - 1 - r.col,
                ..*r
            })
            .collect();
        out.flyers = self
            .flyers
            .iter()
            .map(|f| FlyerSpec {
                rect: Rect::new(w - (f.rect.x + f.rect.w), f.rect.y, f.rect.w, f.rect.h),
                min_x: w - f.max_x,
                max_x: w - f.min_x,
            })
            .collect();
        out.turrets = self
            .turrets
            .iter()
            .map(|t| TurretSpec {
                rect: Rect::new(w - (t.rect.x + t.rect.w), t.rect.y, t.rect.w, t.rect.h),
                dir: -t.dir,
            })
            .collect();
        out.platforms = self
            .platforms
            .iter()
            .map(|p| PlatformSpec {
                rect: Rect::new(w - (p.rect.x + p.rect.w), p.rect.y, p.rect.w, p.rect.h),
                span: p.span,
                vertical: p.vertical,
                dir: if p.vertical { p.dir } else { -p.dir },
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
        self.forget_routes();

        let mut report = ResizeReport::default();
        let (sx, sy) = self.spawn_tile();
        if tx_out(sx, sy, width, height) {
            self.set_spawn_tile(sx.clamp(0, width - 1), sy.clamp(0, height - 1));
            report.spawn_moved = true;
        }

        let before = self.walkers.len();
        self.walkers.retain(|w| {
            let (tx, ty) = (tile_of_x(w.x, PHYSICS.walker_w), tile_of_bottom(w.bottom()));
            !tx_out(tx, ty, width, height)
        });
        report.dropped_walkers = before - self.walkers.len();

        // The later enemies and the movers fall off a shrinking map like anything else, and
        // they have to be *dropped* rather than left outside the grid: a document with an
        // entity past its edge is one the writer refuses, so leaving them would only move
        // the failure from "a level that lost a flyer" to "a level that cannot be saved".
        let before = self.chasers.len() + self.flyers.len() + self.turrets.len();
        self.chasers.retain(|r| {
            !tx_out(
                tile_of_x(r.x, PHYSICS.walker_w),
                tile_of_bottom(r.bottom()),
                width,
                height,
            )
        });
        self.flyers.retain(|f| {
            !tx_out(
                tile_of_x(f.rect.x, PHYSICS.flyer_w),
                tile_of_bottom(f.rect.bottom()),
                width,
                height,
            )
        });
        self.turrets.retain(|t| {
            !tx_out(
                tile_of_x(t.rect.x, PHYSICS.turret_w),
                tile_of_bottom(t.rect.bottom()),
                width,
                height,
            )
        });
        report.dropped_enemies =
            before - (self.chasers.len() + self.flyers.len() + self.turrets.len());

        let before = self.platforms.len();
        self.platforms.retain(|p| {
            !tx_out(
                p.rect.x.round() as i32,
                tile_of_bottom(p.rect.bottom()),
                width,
                height,
            )
        });
        report.dropped_platforms = before - self.platforms.len();

        // A rope is dropped when its anchor column leaves the map *or* when the line would no
        // longer fit under the new bottom edge: half a rope is not a shorter rope, it is a
        // swing that lets go of a body over the void.
        self.ropes
            .retain(|r| r.col < width && r.row >= 1 && (r.row as f32 + r.length) <= height as f32);

        let before = self.scenery.len();
        self.scenery.retain(|s| !tx_out(s.x, s.y, width, height));
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
            .map(|w| (tile_of_x(w.x, PHYSICS.walker_w), tile_of_bottom(w.bottom())))
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

    // ---- the newer enemies and the moving platform --------------------------------------

    /// Tile positions of every chaser, in sim order (after the patrols).
    pub fn chaser_tiles(&self) -> Vec<(i32, i32)> {
        self.chasers
            .iter()
            .map(|c| (tile_of_x(c.x, PHYSICS.walker_w), tile_of_bottom(c.bottom())))
            .collect()
    }

    /// Tile positions of every flyer: the cell its body is authored in.
    pub fn flyer_tiles(&self) -> Vec<(i32, i32)> {
        self.flyers
            .iter()
            .map(|f| {
                (
                    tile_of_x(f.rect.x, PHYSICS.flyer_w),
                    tile_of_bottom(f.rect.bottom()),
                )
            })
            .collect()
    }

    /// Tile positions of every turret: the cell it stands in.
    pub fn turret_tiles(&self) -> Vec<(i32, i32)> {
        self.turrets
            .iter()
            .map(|t| {
                (
                    tile_of_x(t.rect.x, PHYSICS.turret_w),
                    tile_of_bottom(t.rect.bottom()),
                )
            })
            .collect()
    }

    /// Tile positions of every moving platform: the cell its left end rests on.
    pub fn platform_tiles(&self) -> Vec<(i32, i32)> {
        self.platforms
            .iter()
            .map(|p| (p.rect.x.round() as i32, tile_of_bottom(p.rect.bottom())))
            .collect()
    }

    /// Whether a chaser may be placed at `(tx, ty)`: on the map, standing on ground.
    ///
    /// A chaser walks, so it needs exactly what a walker needs — the same feet-on-the-row-
    /// below rule — and the editor colours its ghost from this answer.
    pub fn chaser_placeable(&self, tx: i32, ty: i32) -> bool {
        !tx_out(tx, ty, self.width, self.height) && self.walker_stands_on_ground(tx, ty)
    }

    /// Add a chaser at a tile if it may stand there. Returns whether it landed.
    pub fn add_chaser_tile(&mut self, tx: i32, ty: i32) -> bool {
        if !self.chaser_placeable(tx, ty) {
            return false;
        }
        self.chasers.push(Rect::new(
            tx as f32 + (1.0 - PHYSICS.walker_w) * 0.5,
            crate::physics::bottom_of(ty) - PHYSICS.walker_h,
            PHYSICS.walker_w,
            PHYSICS.walker_h,
        ));
        true
    }

    /// Whether a flyer may be placed at `(tx, ty)`: on the map, with a clear cell.
    ///
    /// A flyer hangs in the air, so the rule is the *scenery* rule rather than the walker's:
    /// the cell has to be empty. Nothing else is asked — the patrol's ends are clamped to the
    /// map and to whatever walls it meets when the sim runs it.
    pub fn flyer_placeable(&self, tx: i32, ty: i32) -> bool {
        !tx_out(tx, ty, self.width, self.height) && self.tile(tx, ty) == Some(Tile::Empty)
    }

    /// Add a flyer at a tile, with a beat `span` tiles long **centred on that tile**. A span
    /// under one tile is widened to one: a flyer that does not travel is a turret that cannot
    /// shoot.
    ///
    /// Centred on the *cell*, not on the box's left edge. The first version measured the beat
    /// from the box's left edge, which put the flyer half a tile off the click in one direction
    /// and made a "four-tile" beat three-and-change tiles of visible travel — the kind of
    /// arithmetic that makes a placement feel like it ignored the pointer.
    pub fn add_flyer_tile(&mut self, tx: i32, ty: i32, span: i32) -> bool {
        if !self.flyer_placeable(tx, ty) {
            return false;
        }
        self.flyers.push(FlyerSpec::at(tx, ty, span));
        true
    }

    /// Whether a turret may be placed at `(tx, ty)`: on the map, standing on ground.
    pub fn turret_placeable(&self, tx: i32, ty: i32) -> bool {
        !tx_out(tx, ty, self.width, self.height) && self.walker_stands_on_ground(tx, ty)
    }

    /// Add a turret at a tile, firing towards `dir` (`+1` right, `-1` left).
    pub fn add_turret_tile(&mut self, tx: i32, ty: i32, dir: i8) -> bool {
        if !self.turret_placeable(tx, ty) {
            return false;
        }
        self.turrets.push(TurretSpec {
            rect: Rect::new(
                tx as f32 + (1.0 - PHYSICS.turret_w) * 0.5,
                crate::physics::bottom_of(ty) - PHYSICS.turret_h,
                PHYSICS.turret_w,
                PHYSICS.turret_h,
            ),
            dir: if dir < 0 { -1 } else { 1 },
        });
        true
    }

    /// Whether a platform `tiles` wide may start at `(tx, ty)`: the whole low walk of its
    /// travel has to be on the map, and the shelf has to have room to be seen.
    ///
    /// Only the *authored* box is checked, not the far end of the travel: a slider whose far
    /// end runs into a wall is a shelf that stops early, which the sim already handles, and
    /// refusing it here would make the author hunt for the exact tile the wall is on.
    pub fn platform_placeable(&self, tx: i32, ty: i32, tiles: i32) -> bool {
        !tx_out(tx, ty, self.width, self.height) && tx + tiles.max(1) - 1 < self.width && ty >= 0
    }

    /// Add a moving platform anchored at a tile. `tiles` is its width, `span` its travel,
    /// `vertical` whether it is a lift rather than a slider, and `dir` which way it goes
    /// (`+1` right/up, `-1` left/down).
    pub fn add_platform_tile(
        &mut self,
        tx: i32,
        ty: i32,
        tiles: i32,
        span: i32,
        vertical: bool,
        dir: i8,
    ) -> bool {
        if !self.platform_placeable(tx, ty, tiles) {
            return false;
        }
        self.platforms
            .push(PlatformSpec::at(tx, ty, tiles, span, vertical, dir));
        true
    }

    // ---- the rope ----------------------------------------------------------------------

    /// Tile positions of every rope: the cell its line hangs from (its first cell).
    pub fn rope_tiles(&self) -> Vec<(i32, i32)> {
        self.ropes.iter().map(|r| (r.col, r.row)).collect()
    }

    /// Whether a rope may be hung at `(tx, ty)`: on the map, tied to something solid above it,
    /// with a cell of open air to hang through.
    ///
    /// **Tied to something**, unlike a flyer, because a rope that hangs from nothing is not a
    /// rope. The anchor is the underside of the tile above, so an author paints a ceiling and
    /// then hangs the line from it — the same click the vine tile's own art already teaches.
    /// The rest of the line is allowed to pass in front of a ledge: a rope is not solid, and a
    /// swing that brushes a wall is handled by the body's own collision.
    pub fn rope_placeable(&self, tx: i32, ty: i32) -> bool {
        !tx_out(tx, ty, self.width, self.height)
            && ty >= 1
            && self.is_opaque(tx, ty - 1)
            && self.tile(tx, ty) == Some(Tile::Empty)
    }

    /// Hang a rope at `(tx, ty)` with `length` tiles of line, shortened to the room under the
    /// new row so a brush stroke at the bottom of the map still hangs a whole rope.
    pub fn add_rope_tile(&mut self, tx: i32, ty: i32, length: i32) -> bool {
        if !self.rope_placeable(tx, ty) {
            return false;
        }
        let room = self.height - ty;
        self.ropes
            .push(RopeSpec::at(tx, ty, length.max(1).min(room.max(1))));
        true
    }

    /// The rope whose line passes through `(tx, ty)`, if any.
    ///
    /// A rope *occupies* the cells it hangs through, not just the one it is tied at. A person
    /// picks and erases the line where they can see it, which for anything longer than a couple
    /// of tiles is its middle — so the eyedropper and the eraser ask this rather than
    /// [`Self::rope_tiles`], which reports only the anchors.
    pub fn rope_at(&self, tx: i32, ty: i32) -> Option<&RopeSpec> {
        self.ropes.iter().find(|r| {
            let last = r.row + r.length.max(1.0).ceil() as i32 - 1;
            r.col == tx && (r.row..=last).contains(&ty)
        })
    }

    /// Whether a **cell** of rope may be painted at `(tx, ty)`.
    ///
    /// A rope is painted the way every other part of a level is: one cell at a time, so the brush
    /// asks this per tile rather than per line. A cell is legal when it is a line's first cell —
    /// empty, with something solid above to tie it to — or when it is the next cell below a line
    /// already hanging in that column, which is how a drag downwards grows one rope instead of
    /// starting a new one every row.
    pub fn rope_cell_placeable(&self, tx: i32, ty: i32) -> bool {
        if tx_out(tx, ty, self.width, self.height) || self.tile(tx, ty) != Some(Tile::Empty) {
            return false;
        }
        if ty >= 1 && self.is_opaque(tx, ty - 1) {
            return true;
        }
        self.ropes.iter().any(|r| {
            let end = r.row + r.length.max(1.0).ceil() as i32 - 1;
            r.col == tx && ty == end + 1
        })
    }

    /// Paint one cell of rope at `(tx, ty)`: start a line there, or grow the one above it by a cell.
    ///
    /// Returns whether the document changed. Growing rather than stacking one-tile ropes is what
    /// makes a drag down a ceiling's edge *one rope*: every row the pointer passes is the next cell
    /// of the line it started.
    pub fn add_rope_cell(&mut self, tx: i32, ty: i32) -> bool {
        if !self.rope_cell_placeable(tx, ty) {
            return false;
        }
        for r in &mut self.ropes {
            let end = r.row + r.length.max(1.0).ceil() as i32 - 1;
            if r.col == tx && ty == end + 1 {
                r.length = (ty - r.row + 1) as f32;
                self.forget_routes();
                return true;
            }
        }
        self.ropes.push(RopeSpec::at(tx, ty, 1));
        self.forget_routes();
        true
    }

    /// Remove the rope hanging from `(tx, ty)`, if any. Returns whether one went.
    ///
    /// `(tx, ty)` may be any cell of the line, for [`Self::rope_at`]'s reason.
    pub fn remove_rope_at(&mut self, tx: i32, ty: i32) -> bool {
        let before = self.ropes.len();
        self.ropes.retain(|r| {
            let last = r.row + r.length.max(1.0).ceil() as i32 - 1;
            !(r.col == tx && (r.row..=last).contains(&ty))
        });
        self.ropes.len() != before
    }

    /// The grip a body at `body` takes on the nearest rope, if its hands are on one.
    ///
    /// The hands are the **top centre of the body**, which is where a person's hands are when
    /// they reach up to catch a line, and the grab is "the line passes the hands": within
    /// [`Physics::rope_grab`] tiles horizontally of the rope's column and anywhere between the
    /// anchor and the rope's end vertically. A miss on either axis is a miss, so a body can jump
    /// past a rope without catching it — which is the decision the object adds.
    ///
    /// **The length is where the hands caught it**, not the rope's own: grabbing high on a long
    /// line is a short pendulum and grabbing low is a long one, so the same rope is a different
    /// swing from a different jump. Clamped to the rope's length, so a body that reaches past the
    /// end catches the end.
    pub fn rope_grab(&self, body: Rect, ph: &crate::Physics) -> Option<crate::physics::RopeGrip> {
        let (hx, hy) = (body.x + body.w * 0.5, body.y);
        let mut best: Option<crate::physics::RopeGrip> = None;
        for r in &self.ropes {
            let (ax, ay) = r.anchor();
            if (hx - ax).abs() > ph.rope_grab {
                continue;
            }
            if hy < ay - 0.01 || hy > ay + r.length {
                continue;
            }
            let dist = ((hx - ax).powi(2) + (hy - ay).powi(2)).sqrt();
            let grip = crate::physics::RopeGrip {
                anchor_x: ax,
                anchor_y: ay,
                length: dist.max(ph.rope_min_length).min(r.length),
                max_length: r.length,
            };
            // The lowest catch wins when two lines are within reach: a body is holding the
            // rope it is closest to meeting, not the one nearest its feet.
            if best.is_none_or(|b| grip.length > b.length) {
                best = Some(grip);
            }
        }
        best
    }

    /// The enemy whose cell is `(tx, ty)`, as its file kind, topmost of the kinds a click can
    /// mean. Used by the editor's eraser and the inspector.
    pub fn enemy_at(&self, tx: i32, ty: i32) -> Option<&'static str> {
        if self.turret_tiles().contains(&(tx, ty)) {
            Some("turret")
        } else if self.walker_tiles().contains(&(tx, ty)) {
            Some("walker")
        } else if self.chaser_tiles().contains(&(tx, ty)) {
            Some("chaser")
        } else if self.flyer_tiles().contains(&(tx, ty)) {
            Some("flyer")
        } else if self.platform_tiles().contains(&(tx, ty)) {
            Some("platform")
        } else if self.rope_at(tx, ty).is_some() {
            Some("rope")
        } else {
            None
        }
    }

    /// Remove whatever enemy or platform owns tile `(tx, ty)`. Returns whether one went.
    pub fn remove_enemy_at(&mut self, tx: i32, ty: i32) -> bool {
        let before = self.walkers.len()
            + self.chasers.len()
            + self.flyers.len()
            + self.turrets.len()
            + self.platforms.len()
            + self.ropes.len();
        self.walkers
            .retain(|w| (tile_of_x(w.x, PHYSICS.walker_w), tile_of_bottom(w.bottom())) != (tx, ty));
        self.chasers
            .retain(|c| (tile_of_x(c.x, PHYSICS.walker_w), tile_of_bottom(c.bottom())) != (tx, ty));
        self.flyers.retain(|f| {
            (
                tile_of_x(f.rect.x, PHYSICS.flyer_w),
                tile_of_bottom(f.rect.bottom()),
            ) != (tx, ty)
        });
        self.turrets.retain(|t| {
            (
                tile_of_x(t.rect.x, PHYSICS.turret_w),
                tile_of_bottom(t.rect.bottom()),
            ) != (tx, ty)
        });
        self.platforms
            .retain(|p| (p.rect.x.round() as i32, tile_of_bottom(p.rect.bottom())) != (tx, ty));
        self.ropes.retain(|r| {
            let last = r.row + r.length.max(1.0).ceil() as i32 - 1;
            !(r.col == tx && (r.row..=last).contains(&ty))
        });
        self.walkers.len()
            + self.chasers.len()
            + self.flyers.len()
            + self.turrets.len()
            + self.platforms.len()
            + self.ropes.len()
            != before
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
        self.forget_routes();
        true
    }

    /// Remove the walker standing in tile `(tx, ty)`, if any.
    pub fn remove_walker_at(&mut self, tx: i32, ty: i32) -> bool {
        let before = self.walkers.len();
        self.walkers
            .retain(|w| (tile_of_x(w.x, PHYSICS.walker_w), tile_of_bottom(w.bottom())) != (tx, ty));
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
            self.forget_routes();
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
        let index = exact.or_else(|| {
            self.scenery
                .iter()
                .rposition(|s| scenery_reaches(s, tx, ty))
        });
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
            return Err(format!(
                "level has non-positive size {}x{}",
                self.width, self.height
            ));
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
            return Err(
                "grid contains a 'G' tile: it is load-time sugar, and writing it \
                 would load back as a goal entity with different tiles"
                    .into(),
            );
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
            ("chaser", self.chaser_tiles()),
            ("flyer", self.flyer_tiles()),
            ("turret", self.turret_tiles()),
            ("platform", self.platform_tiles()),
            ("rope", self.rope_tiles()),
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
        let mut entities: Vec<RawEntity> = Vec::with_capacity(
            self.walkers.len()
                + self.chasers.len()
                + self.flyers.len()
                + self.turrets.len()
                + self.platforms.len()
                + self.ropes.len()
                + self.goals.len()
                + self.scenery.len(),
        );
        for (tx, ty) in self.walker_tiles() {
            entities.push(RawEntity::of("walker", tx, ty));
        }
        for r in &self.chasers {
            let (tx, ty) = (tile_of_x(r.x, PHYSICS.walker_w), tile_of_bottom(r.bottom()));
            entities.push(RawEntity::of("chaser", tx, ty));
        }
        for f in &self.flyers {
            entities.push(RawEntity {
                span: Some(f.span().round().max(1.0) as i32),
                ..RawEntity::of(
                    "flyer",
                    tile_of_x(f.rect.x, PHYSICS.flyer_w),
                    tile_of_bottom(f.rect.bottom()),
                )
            });
        }
        for t in &self.turrets {
            entities.push(RawEntity {
                dir: Some(t.dir),
                ..RawEntity::of(
                    "turret",
                    tile_of_x(t.rect.x, PHYSICS.turret_w),
                    tile_of_bottom(t.rect.bottom()),
                )
            });
        }
        for p in &self.platforms {
            entities.push(RawEntity {
                tiles: Some(p.rect.w.round().max(1.0) as i32),
                span: Some(p.span.round().max(1.0) as i32),
                axis: Some(if p.vertical { "y" } else { "x" }.to_string()),
                dir: Some(p.dir),
                ..RawEntity::of(
                    "platform",
                    p.rect.x.round() as i32,
                    tile_of_bottom(p.rect.bottom()),
                )
            });
        }
        for (tx, ty) in self.goal_tiles() {
            entities.push(RawEntity::of("goal", tx, ty));
        }
        for s in &self.scenery {
            entities.push(RawEntity {
                sprite: Some(s.sprite.clone()),
                ..RawEntity::of("scenery", s.x, s.y)
            });
        }
        for r in &self.ropes {
            entities.push(RawEntity {
                tiles: Some(r.length.round().max(1.0) as i32),
                ..RawEntity::of("rope", r.col, r.row)
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

    const PACK: [&str; 12] = [
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
        "l10-the-hunt",
        "l11-the-elements",
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
            assert_eq!(
                (back.width, back.height),
                (level.width, level.height),
                "{slug} size"
            );
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
            assert_eq!(
                level.to_json_pretty().unwrap(),
                json,
                "{slug} writer stability"
            );
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
        assert!(
            !level.add_scenery("bush_c", 4, 9),
            "that cell is solid floor"
        );
        level.set_tile(3, 5, Tile::Solid);
        assert!(
            !level.add_scenery("tree_b_base", 3, 7),
            "no room for a tree"
        );
        assert!(!level.add_scenery("bush_c", -1, 7), "off the grid");

        let json = level
            .to_json_pretty()
            .expect("a decorated level is writable");
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
        assert_ne!(
            level.content_hash(),
            before,
            "painting must change the level"
        );
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
            assert!(
                !level.set_tile(tx, ty, Tile::Solid),
                "({tx}, {ty}) must be refused"
            );
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

        assert!(
            level.remove_goal_at(18, 8),
            "the starter's own flag comes off"
        );
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
        assert!(
            !level.goal_covers(18, 6),
            "one cell above the flag is open air"
        );
        assert!(!level.goal_covers(17, 7), "one column left is open air");

        // Erased by its cloth — the only part of the marker with any colour in it.
        assert!(
            level.remove_goal_at(18, 7),
            "erased by the flag, not the pole"
        );
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
        assert!(
            !level.remove_scenery_at(12, 6),
            "air above a bush is open air"
        );
        assert_eq!(level.scenery_at(12, 7), Some("bush_c"));
        assert!(level.remove_scenery_at(12, 7));

        // A cell holding a piece of its own takes the pointer's target first, even when a
        // tree's canopy reaches over it: one click, the thing under the cursor.
        assert!(level.add_scenery("tree_a_base", 12, 7));
        assert!(
            level.add_scenery("bush_c", 12, 6),
            "in the tree's canopy cell"
        );
        assert!(level.remove_scenery_at(12, 6));
        assert_eq!(level.scenery_at(12, 6), None, "the bush under the pointer");
        assert_eq!(
            level.scenery_at(12, 7),
            Some("tree_a_base"),
            "the tree it was standing in front of survives one click"
        );
        assert!(
            level.remove_scenery_at(12, 6),
            "the second click takes the tree"
        );
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
        assert!(
            report.goal_moved,
            "the goal was dragged back to the grid edge"
        );
        assert!(level.validate().is_ok(), "{:?}", level.validate());
        assert_eq!(
            level.goal_tiles()[0],
            (MIN_LEVEL_W - 1, MIN_LEVEL_H - 1),
            "the last valid tile, not one inside the old goal's row"
        );
        assert!(level.to_json_pretty().is_ok());
    }

    /// **A resize drops what it cannot fit, and never leaves an entity outside the grid.**
    ///
    /// The failure this is written for: an entity past the new edge makes `validate` refuse
    /// the document, so a shrink would turn "a level that lost a flyer" into "a level that
    /// cannot be saved". Every kind is placed past column 10 and the grid is cut to it.
    #[test]
    fn resize_drops_the_later_entities_and_leaves_a_saveable_document() {
        let mut level = Level::starter("shrink-later", 40, 12);
        for tx in 0..40 {
            level.set_tile(tx, 10, Tile::Solid);
        }
        assert!(level.add_chaser_tile(30, 9));
        assert!(level.add_flyer_tile(32, 6, 2));
        assert!(level.add_turret_tile(34, 9, 1));
        assert!(level.add_platform_tile(36, 6, 2, 2, false, 1));
        assert_eq!(level.goal_tiles().len(), 1);

        let report = level.resize(10, 8, Tile::Empty);
        assert_eq!(report.dropped_enemies, 3, "a chaser, a flyer and a turret");
        assert_eq!(report.dropped_platforms, 1);
        assert!(level.chasers.is_empty() && level.flyers.is_empty() && level.turrets.is_empty());
        assert!(level.platforms.is_empty());
        assert!(level.validate().is_ok(), "{:?}", level.validate());
        assert!(level.to_json_pretty().is_ok(), "and it can still be saved");
    }

    /// **The mirror keeps every new entity, and is its own inverse.**
    ///
    /// The kind of omission a mirror test is for: the terrain is mirrored by a loop over
    /// tiles, so an entity family nobody remembered is simply *gone* from the reflection —
    /// and the editor's Mirror button is one click away from a level that lost its enemies.
    /// The turret's facing and the flyer's beat have to turn with the level; a slider's
    /// direction of travel does not, because the level flipped and the clock did not.
    #[test]
    fn the_mirror_keeps_the_later_entities_and_turns_what_faces() {
        let mut level = Level::starter("mirror-later", 40, 12);
        for tx in 0..40 {
            level.set_tile(tx, 10, Tile::Solid);
        }
        assert!(level.add_walker_tile(6, 9));
        assert!(level.add_chaser_tile(8, 9));
        assert!(level.add_flyer_tile(12, 6, 4));
        assert!(level.add_turret_tile(20, 9, -1));
        assert!(level.add_platform_tile(24, 6, 3, 4, true, 1));
        assert!(level.set_goal_tile(36, 9));

        let mirrored = level.mirrored_horizontal();
        assert_eq!(mirrored.chasers.len(), 1, "a chaser survived the mirror");
        assert_eq!(mirrored.flyers.len(), 1);
        assert_eq!(mirrored.turrets.len(), 1);
        assert_eq!(mirrored.platforms.len(), 1);
        assert_eq!(
            mirrored.turret_tiles(),
            vec![(40 - 1 - 20, 9)],
            "the turret moved to the reflected column"
        );
        assert_eq!(
            mirrored.turrets[0].dir, 1,
            "and now fires the other way: its row is dangerous from the other side"
        );
        assert!(
            mirrored.platforms[0].vertical,
            "a lift is still a lift after a mirror"
        );
        assert_eq!(mirrored.platforms[0].span, level.platforms[0].span);
        // The beat reflected: its two ends swap places.
        let stretch = |f: &FlyerSpec| (f.max_x - f.min_x, f.rect.w);
        assert_eq!(stretch(&mirrored.flyers[0]), stretch(&level.flyers[0]));

        // And the mirror of the mirror is the document again, entity for entity.
        let back = mirrored.mirrored_horizontal();
        assert_eq!(back.chaser_tiles(), level.chaser_tiles());
        assert_eq!(back.flyer_tiles(), level.flyer_tiles());
        assert_eq!(back.turret_tiles(), level.turret_tiles());
        assert_eq!(back.platform_tiles(), level.platform_tiles());
        assert_eq!(back.turrets[0].dir, level.turrets[0].dir);
        for (a, b) in back.flyers.iter().zip(level.flyers.iter()) {
            assert!((a.min_x - b.min_x).abs() < 1e-3 && (a.max_x - b.max_x).abs() < 1e-3);
        }
        assert!(back.validate().is_ok(), "{:?}", back.validate());
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
