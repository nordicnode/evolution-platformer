//! The level editor's document: what the user is drawing, and the tools to draw it.
//!
//! The document is a [`Level`] — the simulation's own type, not a parallel model.
//! That is the point of the `sim` editing API: what the palette paints is what the
//! physics will run, and what "save" writes is what the loader reads.
//!
//! Three rules the editor keeps for the user:
//!
//! * **Every stroke is one undo step.** Painting is continuous (the brush follows the
//!   pointer), so the snapshot is taken once when the stroke starts, not per tile.
//! * **A level has one flag.** [`Brush::Goal`] places *the* goal, so a later click
//!   *relocates* it instead of adding a rival — the same shape as the spawn, which is one
//!   point too — and the flag can be erased entirely, so the document says "no goal" rather
//!   than keeping a marker the user cannot get out of the way.
//! * **An unwritable document says so instead of writing.** The flag is the user's to
//!   delete as well as to place ([`Level::remove_goal_at`] will happily leave a document
//!   with no flag), so the level *can* stand unsaveable for a moment; [`Editor::save`]
//!   reports the loader's own error text rather than writing something it cannot read
//!   back, and [`Editor::verdict`] gives the same reason in the status bar first.
//! * **A drag is a shape as well as a material.** [`Brush`] says what is being painted and
//!   [`Shape`] says how the painting lies on the grid: freehand, a rectangle or a line. A
//!   rectangle or a line is redrawn whole from the tile the drag started on rather than
//!   accumulated tile by tile, so the box shrinks when the pointer comes back and the whole
//!   drag is one undo step either way.
//! * **The start and the enemies stand on ground.** A spawn in mid-air is an episode that
//!   is decided on its first frame, so [`Editor::can_place`] refuses one and the refusal is
//!   what the status bar says — the same bargain [`Level::walker_stands_on_ground`] makes
//!   for a walker, and the same predicate the pack's own test uses.

use std::path::{Path, PathBuf};

use sim::reach::ReachReport;
use sim::{Level, Tile};

/// The shipped level pack stems (`levels/l1-flat-run.json` through `l9-wait-and-climb.json`).
///
/// The editor opens on `l1-flat-run.json` by default. Without protection, drawing a custom
/// level on the default canvas and pressing Save (or Ctrl+S) silently overwrites the shipped
/// pack's opening course. Saving a shipped level redirects to a `custom-<stem>.json` copy in
/// the same directory so the user's creation is saved while the built-in pack stays intact.
pub const SHIPPED_LEVEL_STEMS: &[&str] = &[
    "l1-flat-run",
    "l2-first-gap",
    "l3-gauntlet",
    "l4-rhythm",
    "l4-tide-pools",
    "l5-tightrope",
    "l6-marathon",
    "l7-ascent",
    "l7-wall-school",
    "l8-long-jump",
    "l9-wait-and-climb",
];

/// If `path` points to one of the shipped level files inside a `levels` directory, return a
/// safe sibling path (`custom-<stem>.json`) so saving never clobbers the built-in pack.
pub fn protect_shipped_path(path: &Path) -> PathBuf {
    let is_levels_dir = path
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|dir| dir == "levels");
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if is_levels_dir && SHIPPED_LEVEL_STEMS.contains(&stem) {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        parent.join(format!("custom-{stem}.json"))
    } else {
        path.to_path_buf()
    }
}

/// The first free level name in a directory, starting from `want`: `want`, `want-2`, `want-3`, …
///
/// **A name is a file.** A level's name is what its file is called — [`Editor::save`] writes
/// `{slug(name)}.json` — so two levels both answering to `untitled` are two documents writing
/// to one file: the second Save replaced the first, with no warning and nothing on screen to
/// say a level had been eaten (AUDIT.md B2). Counting from the *directory* rather than from a
/// counter in memory, because the pack is what the name has to be free of: a window opened on
/// a directory that already holds `untitled.json` has to see it.
///
/// `want` is slugged first, because the file's stem is what has to be free and the two are the
/// same word everywhere a person reads it: the shelf card, the dock, the window title and the
/// stem a champion trained on the level wears. The numbering starts at the second — `name-1`
/// is a name no person would type, and a pack that reads `untitled, untitled-2` says what it
/// is, which is two of them.
pub fn free_level_name(dir: impl AsRef<Path>, want: &str) -> String {
    let dir = dir.as_ref();
    let base = crate::lab::slug(want);
    let taken = |name: &str| dir.join(format!("{name}.json")).exists();
    if !taken(&base) {
        return base;
    }
    for n in 2u32.. {
        let name = format!("{base}-{n}");
        if !taken(&name) {
            return name;
        }
    }
    unreachable!("an unbounded search for a free name ends at the first free name")
}

/// How deep undo goes. Snapshots are a level-sized vector of tile enums plus a
/// handful of rectangles, so a hundred of them is nothing.
const UNDO_LIMIT: usize = 128;

/// What the palette can place, and therefore what a drag can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brush {
    /// Solid terrain.
    Solid,
    /// Solid terrain of a different material: a beach, a dune, a desert floor.
    Sand,
    /// Solid terrain of a third material: a coping stone, a cellar wall, a ruin.
    Brick,
    /// Solid terrain of a fourth material: milled decking, scaffolding, a jetty.
    Plank,
    /// A lethal spike.
    Spike,
    /// Clears terrain.
    Empty,
    /// Moves the spawn point.
    Spawn,
    /// Places a goal flag.
    Goal,
    /// Places a patrolling walker.
    Walker,
    /// Places a piece of scenery (a bush, tree, rock, flower...): decoration the physics
    /// never sees. Which sprite is decided by [`Editor::scene_sprite`], because there are
    /// two dozen of them and two dozen palette buttons would be two dozen brushes.
    Scenery,
    /// Removes whatever is under the cursor: entity first, then terrain.
    Erase,
}

/// The scenery a person can place, curated from the shipped art.
///
/// Deliberately a *list* rather than every sprite in the atlas: the scattered dressing
/// already covers the ground in bushes and tufts, so a placement menu is for the props a
/// person reaches for on purpose — the trees, the rocks, the flowers, the things that say
/// what a place *is*. Sizes and the tree's canopy come from the view, so adding a name here
/// is all it takes to place it.
pub const SCENERY_SPRITES: &[&str] = &[
    "bush_c",
    "bush_e",
    "tuft_c",
    "tuft_e",
    "flowers",
    "flowers_pink",
    "flowers_white",
    "rock",
    "pebble",
    "mushroom",
    "mushroom_pair",
    "log",
    "stump",
    "reeds_a",
    "vine_a",
    "tree_a_base",
    "tree_b_base",
];

/// How a drag lays tiles down.
///
/// Not a *brush* — the brush is what is being painted — but the shape the painting takes:
/// freehand, an axis-aligned rectangle, or a straight line. It is the one thing about a
/// stroke that the material does not decide, and keeping it apart from [`Brush`] is what
/// lets a person draw a brick rectangle without a second row of rectangle-shaped buttons
/// for every material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The pointer's own path, with the gaps between frames filled in.
    Free,
    /// The rectangle between where the drag started and where the pointer is.
    Rect,
    /// The straight line between where the drag started and where the pointer is.
    Line,
}

impl Shape {
    pub const ALL: [Shape; 3] = [Shape::Free, Shape::Rect, Shape::Line];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Free => "brush",
            Shape::Rect => "rect",
            Shape::Line => "line",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Shape::Free => "Freehand: the tiles the pointer passes over.",
            Shape::Rect => "Rectangle: fills the box between the drag's start and the pointer.",
            Shape::Line => "Line: the straight run of tiles between the drag's two ends.",
        }
    }
}

impl Brush {
    pub const ALL: [Brush; 11] = [
        Brush::Solid,
        Brush::Sand,
        Brush::Brick,
        Brush::Plank,
        Brush::Spike,
        Brush::Scenery,
        Brush::Empty,
        Brush::Spawn,
        Brush::Goal,
        Brush::Walker,
        Brush::Erase,
    ];

    /// The atlas sprite that represents this brush in the palette.
    ///
    /// Every one of these has to be a name the shipped atlas actually carries, because the
    /// palette draws the icon by `atlas.uv(...)` and a name that is not there is a **blank
    /// button** rather than an error — the ground brush said `grass`, which no tileset has
    /// ever had a sprite called, and it had been an empty square in the palette ever since.
    /// `atlas::tests::every_brush_has_an_icon_in_the_atlas` is what says so now.
    pub fn sprite(self) -> &'static str {
        match self {
            Brush::Solid => "ground_1",
            Brush::Sand => "sand_1",
            Brush::Brick => "brick_1",
            Brush::Plank => "wood_1",
            Brush::Spike => "spike",
            Brush::Scenery => "bush_c",
            Brush::Empty => "empty",
            Brush::Spawn => "spawn_marker",
            Brush::Goal => "goal_top",
            Brush::Walker => "walker_a",
            Brush::Erase => "walker_dead",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Brush::Solid => "ground",
            Brush::Sand => "sand",
            Brush::Brick => "brick",
            Brush::Plank => "timber",
            Brush::Spike => "spikes",
            Brush::Scenery => "scenery",
            Brush::Empty => "clear",
            Brush::Spawn => "spawn",
            Brush::Goal => "goal",
            Brush::Walker => "walker",
            Brush::Erase => "erase",
        }
    }

    /// What this brush does, for the tooltip.
    pub fn about(self) -> &'static str {
        match self {
            Brush::Solid => "Solid tile: blocks movement from every side.",
            Brush::Sand => "Sand: solid tile, same physics, drawn as a beach.",
            Brush::Brick => "Brick: solid tile, same physics, drawn as mortared brickwork.",
            Brush::Plank => "Timber: solid tile, same physics, drawn as milled decking.",
            Brush::Spike => "Spike: not solid, lethal on contact.",
            Brush::Scenery => "Scenery: a bush, tree, rock... decoration the physics ignores.\n\
                Pick which one in the row under the palette.",
            Brush::Empty => "Empty space: clears a tile.",
            Brush::Spawn => "Where the player starts. Only one per level.",
            Brush::Goal => "The flag the population has to reach.\n\
                A level has one: placing it moves it.",
            Brush::Walker => "A patrol enemy. Stompable from above.",
            Brush::Erase => "Removes the entity under the cursor, then the tile.",
        }
    }
}

/// The editor document and its history.
pub struct Editor {
    pub level: Level,
    pub path: Option<PathBuf>,
    /// The active brush once something is picked, and what a drag carries.
    pub brush: Option<Brush>,
    /// Which sprite the scenery brush places, chosen in the palette row under the brushes.
    pub scene_sprite: &'static str,
    /// How a drag lays tiles down: freehand, a rectangle or a line.
    pub shape: Shape,
    /// Whether the *next* stroke is forced to a straight line whatever [`Self::shape`] says.
    ///
    /// Held by the app from the shift key and read by [`Editor::paint`]: shift is the shape a
    /// person wants right now rather than a mode to go and set, which is the whole reason the key
    /// is worth binding — a long straight floor is the one thing a steady hand cannot draw
    /// (AUDIT.md F4). A field rather than an argument to `paint` because `paint` is called from
    /// four places and the shape is a property of the gesture, not of the call.
    pub line: bool,
    /// Where the current shape gesture started, for [`Shape::Rect`] and [`Shape::Line`].
    ///
    /// The anchor rather than the previous tile, because a rectangle is not a path: every
    /// motion of the pointer has to redraw the *whole* box from the corner the drag started
    /// at, or the shape would only ever accumulate the tiles it had already been over.
    shape_anchor: Option<(i32, i32)>,
    /// The level as the reachability oracle sees it, refreshed after every change.
    pub report: ReachReport,
    /// Human-readable result of the last action, shown in the status bar.
    pub message: String,
    /// Set when the document differs from what is on disk.
    pub dirty: bool,

    undo: Vec<Level>,
    redo: Vec<Level>,
    /// A stroke is in progress: further tiles join the current undo step.
    stroke_open: bool,
    /// The level hash the current report describes, so the oracle is re-run when the
    /// document changes and not otherwise.
    report_hash: u64,
    /// Last tile painted, so a fast drag fills the gap it jumped over.
    last_paint: Option<(i32, i32)>,
}

impl Editor {
    pub fn new(level: Level, path: Option<PathBuf>) -> Editor {
        let report = sim::reach::report(&level);
        let report_hash = level.content_hash();
        Editor {
            level,
            path,
            brush: Some(Brush::Solid),
            scene_sprite: SCENERY_SPRITES[0],
            shape: Shape::Free,
            line: false,
            shape_anchor: None,
            report,
            message: String::new(),
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke_open: false,
            report_hash,
            last_paint: None,
        }
    }

    /// Load a level from disk, ready to edit.
    pub fn open(path: impl AsRef<Path>) -> Result<Editor, String> {
        let path = path.as_ref();
        let level = Level::from_json_file(path)?;
        Ok(Editor::new(level, Some(path.to_path_buf())))
    }

    /// A new document, already playable.
    pub fn fresh(name: &str, width: i32, height: i32) -> Editor {
        Editor::new(Level::starter(name, width, height), None)
    }

    /// The document's title: the file name, or "untitled" for a new level.
    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.json", self.level.name));
        if self.dirty {
            format!("{name} — edited")
        } else {
            name
        }
    }

    /// Start an undo step. Call once per pointer gesture; every paint inside it is
    /// one step in the history.
    pub fn begin_stroke(&mut self) {
        if self.stroke_open {
            return;
        }
        self.push_undo();
        self.stroke_open = true;
        self.last_paint = None;
    }

    /// End the current gesture.
    pub fn end_stroke(&mut self) {
        self.stroke_open = false;
        self.last_paint = None;
        // The anchor belongs to one gesture: a rectangle's first corner is where *this*
        // drag started, and keeping it after the button came up would make the next drag
        // draw its box from the last one's corner.
        self.shape_anchor = None;
    }

    fn push_undo(&mut self) {
        self.undo.push(self.level.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Whether the document's rules allow `brush` at `(tx, ty)`.
    ///
    /// Four brushes have a rule the document would otherwise accept quietly. A *walker* and
    /// a *start* both need ground under them, and without the check the document takes them
    /// and the pack's own test refuses the level afterwards. A *goal*'s refusal is not
    /// obvious from the pointer, because the marker is two cells tall: the tile it already
    /// owns is often *not* the one under the cursor but the flag's own cell a row above it.
    /// *Scenery* needs an empty cell, and a tree needs the room its canopy reaches into.
    ///
    /// Everything else is refused by its own writer — a tile outside the grid does not land,
    /// a goal needs a free tile — so this is the list of placements where the click and the
    /// write would otherwise disagree. The app asks it to colour the drop ghost, so a
    /// placement that will be refused looks refused before the mouse button comes up.
    pub fn can_place(&self, brush: Brush, tx: i32, ty: i32) -> bool {
        match brush {
            Brush::Spawn => self.level.spawn_stands_on_ground(tx, ty),
            Brush::Walker => self.level.walker_stands_on_ground(tx, ty),
            Brush::Scenery => self.level.scenery_placeable(self.scene_sprite, tx, ty),
            Brush::Goal => self.level.goal_placeable(tx, ty),
            _ => true,
        }
    }

    /// Apply the active brush at a tile. Returns whether the document changed.
    ///
    /// What the tiles *are* is a freehand stroke with the gaps filled in, so a fast drag
    /// does not leave a dotted line where the pointer skipped tiles between frames. When the
    /// shape is a rectangle or a line, though, the stroke is not a path at all: the shape is
    /// redrawn whole from the tile the drag started on, every time the pointer moves, over
    /// the level as it was when the gesture began — so the box can grow and shrink as the
    /// pointer comes back, and the drag is still one undo step, because the picture it is
    /// redrawn over *is* the gesture's own undo snapshot.
    pub fn paint(&mut self, tx: i32, ty: i32) -> bool {
        let Some(brush) = self.brush else {
            return false;
        };
        if !self.stroke_open {
            self.begin_stroke();
        }
        // The first tile painted in a gesture is the shape's anchor — where the drag started,
        // which is the corner a rectangle grows from and the end a line is drawn from. Set
        // here rather than in [`Editor::begin_stroke`] because the anchor is a *tile* and
        // the stroke is opened by the app from a pointer position that has not been turned
        // into one yet; set on `None` rather than on `!stroke_open` because a caller that
        // opens the stroke itself (`erase_at`, and every test that drives the editor
        // directly) still has to get an anchor.
        if self.shape_anchor.is_none() {
            self.shape_anchor = Some((tx, ty));
        }
        // A shape is redrawn from its anchor on every motion, over the level as it was
        // when the gesture started. That is the only way a rectangle can *shrink* as the
        // pointer comes back, and the snapshot the gesture already pushed for undo is
        // exactly the picture to redraw over — so a shape drag costs no second snapshot
        // and stays one undo step.
        // Shift forces the line (see [`Self::line`]): the shape row in the palette is where a
        // person *sets* a shape, and the key is where they ask for one for one stroke.
        if self.effective_shape() != Shape::Free {
            let anchor = self.shape_anchor.unwrap_or((tx, ty));
            let restore = if anchor == (tx, ty) {
                None
            } else {
                self.undo.last().cloned()
            };
            if let Some(before) = restore {
                self.level = before;
            }
            let changed = self.paint_shape(brush, anchor, (tx, ty));
            self.last_paint = Some((tx, ty));
            if changed {
                self.dirty = true;
            }
            return changed;
        }
        let mut changed = false;
        for (x, y) in self.stroke_path(tx, ty) {
            changed |= self.paint_one(brush, x, y);
        }
        self.last_paint = Some((tx, ty));
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// Apply `brush` over the whole shape from `anchor` to `(tx, ty)`.
    ///
    /// The tiles are painted in a row-major order rather than in the order the pointer
    /// reached them, so the same rectangle drawn twice is the same document: a level's
    /// content hash is what undo, the reachability oracle and the champion files all
    /// compare, and an order-dependent repaint would move the hash on every frame of a
    /// drag that nudged the pointer by nothing.
    fn paint_shape(&mut self, brush: Brush, anchor: (i32, i32), to: (i32, i32)) -> bool {
        let tiles = self.shape_tiles(anchor, to);
        let mut changed = false;
        for (x, y) in tiles {
            changed |= self.paint_one(brush, x, y);
        }
        changed
    }

    /// The tiles a shape covers, from its anchor: a box, or the run between two ends.
    /// The shape this stroke is actually drawn with: the palette's, or the line the shift key is
    /// asking for (see [`Self::line`]).
    ///
    /// One function rather than a `if` at each call site, because there are two of them — `paint`
    /// decides whether to take the shape path by it, and `shape_tiles` decides *what that path
    /// draws* by it. Two hand-written copies of this rule is how shift came to paint only the tile
    /// under the pointer and lose the anchor it was supposed to be drawing a line from.
    fn effective_shape(&self) -> Shape {
        if self.line {
            Shape::Line
        } else {
            self.shape
        }
    }

    fn shape_tiles(&self, anchor: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
        let (x0, y0) = anchor;
        let (x1, y1) = to;
        match self.effective_shape() {
            Shape::Free => vec![to],
            Shape::Rect => {
                let (left, right) = (x0.min(x1), x0.max(x1));
                let (top, bottom) = (y0.min(y1), y0.max(y1));
                (top..=bottom)
                    .flat_map(|y| (left..=right).map(move |x| (x, y)))
                    .collect()
            }
            Shape::Line => line_tiles(anchor, to),
        }
    }

    /// The tiles this paint step covers: the straight line from the last painted
    /// tile to this one, so a quick drag leaves a solid line rather than dots.
    fn stroke_path(&self, tx: i32, ty: i32) -> Vec<(i32, i32)> {
        let Some((px, py)) = self.last_paint else {
            return vec![(tx, ty)];
        };
        if (px, py) == (tx, ty) {
            return Vec::new();
        }
        let steps = (tx - px).abs().max((ty - py).abs()).max(1);
        (1..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                (
                    px + ((tx - px) as f32 * t).round() as i32,
                    py + ((ty - py) as f32 * t).round() as i32,
                )
            })
            .collect()
    }

    fn paint_one(&mut self, brush: Brush, tx: i32, ty: i32) -> bool {
        match brush {
            Brush::Solid => self.level.set_tile(tx, ty, Tile::Solid),
            Brush::Sand => self.level.set_tile(tx, ty, Tile::Sand),
            Brush::Brick => self.level.set_tile(tx, ty, Tile::Brick),
            Brush::Plank => self.level.set_tile(tx, ty, Tile::Plank),
            Brush::Spike => self.level.set_tile(tx, ty, Tile::Spike),
            Brush::Empty => self.level.set_tile(tx, ty, Tile::Empty),
            Brush::Spawn => {
                if self.level.spawn_tile() == (tx, ty) {
                    return false;
                }
                if !self.can_place(Brush::Spawn, tx, ty) {
                    // The same bargain the walker's refusal makes: a click that does nothing
                    // is a bug report, so the reason is said out loud instead of the level
                    // opening with the player in mid-air.
                    self.message = "the start needs an empty cell with ground under it"
                        .to_string();
                    return false;
                }
                self.level.set_spawn_tile(tx, ty);
                true
            }
            Brush::Goal => {
                if !self.can_place(Brush::Goal, tx, ty) {
                    // A goal refused off the map is not reachable from the UI; a goal refused
                    // for being on the marker that is already there is the click a user
                    // actually makes, and the reason is the whole point of saying anything.
                    self.message = if self.level.tile(tx, ty).is_none() {
                        "a goal has to stand on the map".to_string()
                    } else {
                        "the goal is already here".to_string()
                    };
                    return false;
                }
                // One flag per level, so this is a *move*. Worth saying out loud: the flag
                // disappears from one end of the level and appears at the other.
                let moved = self.level.set_goal_tile(tx, ty);
                if moved {
                    self.message = "moved the goal here".to_string();
                }
                moved
            }
            Brush::Walker => {
                if self.level.walker_tiles().contains(&(tx, ty)) {
                    return false;
                }
                if !self.can_place(Brush::Walker, tx, ty) {
                    // A click that does nothing is a bug report; the reason is the point.
                    self.message = "a walker needs ground under it".to_string();
                    return false;
                }
                self.level.add_walker_tile(tx, ty)
            }
            Brush::Scenery => {
                if !self.can_place(Brush::Scenery, tx, ty) {
                    self.message = "scenery needs an empty cell (and a tree needs room above)"
                        .to_string();
                    return false;
                }
                self.level.add_scenery(self.scene_sprite, tx, ty)
            }
            Brush::Erase => {
                // Entities before terrain, and the scenery the user can *see* before both:
                // one click on a decorated tile clears the decoration rather than the floor
                // under it, which is the edit the person aiming at the bush meant.
                if self.level.remove_scenery_at(tx, ty) {
                    return true;
                }
                if self.level.remove_walker_at(tx, ty) {
                    return true;
                }
                if self.level.remove_goal_at(tx, ty) {
                    return true;
                }
                self.level.set_tile(tx, ty, Tile::Empty)
            }
        }
    }

    /// Remove the entity (or tile) at a tile, without touching the active brush.
    pub fn erase_at(&mut self, tx: i32, ty: i32) -> bool {
        if !self.stroke_open {
            self.begin_stroke();
        }
        if self.shape_anchor.is_none() {
            self.shape_anchor = Some((tx, ty));
        }
        let changed = self.paint_one(Brush::Erase, tx, ty);
        self.last_paint = Some((tx, ty));
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// Take the brush from whatever is under `(tx, ty)`.
    ///
    /// The eyedropper: the one tool every drawing program has and this one did not. Without it,
    /// matching a material already on the level meant clicking the right one of four palette cells
    /// and hoping you had guessed which solid this patch was made of (AUDIT.md F4).
    ///
    /// **What the cell *is* decides the brush**, in the order a person would answer the question,
    /// because one tile can hold two of them: the walker standing on the floor is what the cell is
    /// (you cannot place two, and the terrain is under half the level), then a placed scenery
    /// sprite picks its own name out of [`SCENERY_SPRITES`], then the spawn, then the flag — which
    /// is two tiles of art for one entity, so either cell of it picks it — and last the material.
    ///
    /// Returns whether the brush changed. A pick that picks what is already in hand says
    /// nothing: the message line is for news.
    pub fn pick_at(&mut self, tx: i32, ty: i32) -> bool {
        let before = (self.brush, self.scene_sprite);
        let (sx, sy) = self.level.spawn_tile();
        let goal = self.level.goal_tiles().first().copied();
        let scenery = self
            .level
            .scenery
            .iter()
            .find(|s| (s.x, s.y) == (tx, ty));

        if self.level.walker_tiles().contains(&(tx, ty)) {
            self.brush = Some(Brush::Walker);
        } else if let Some(scenery) = scenery {
            // The sprite's own name, when the palette knows it: the scenery row is a curated list,
            // so a prop that is on the level but not in the list would silently pick the *first*
            // prop instead — and a brush that paints something else is worse than a refusal.
            match SCENERY_SPRITES.iter().find(|name| **name == scenery.sprite) {
                Some(name) => {
                    self.scene_sprite = name;
                    self.brush = Some(Brush::Scenery);
                }
                None => self.brush = Some(Brush::Scenery),
            }
        } else if goal.is_some_and(|(gx, gy)| gx == tx && (gy == ty || gy - 1 == ty)) {
            // The flag is the cell it stands in and the one its pole reaches into.
            self.brush = Some(Brush::Goal);
        } else if (tx, ty) == (sx, sy) {
            self.brush = Some(Brush::Spawn);
        } else {
            match self.level.tile(tx, ty) {
                Some(Tile::Solid) => self.brush = Some(Brush::Solid),
                Some(Tile::Sand) => self.brush = Some(Brush::Sand),
                Some(Tile::Brick) => self.brush = Some(Brush::Brick),
                Some(Tile::Plank) => self.brush = Some(Brush::Plank),
                Some(Tile::Spike) => self.brush = Some(Brush::Spike),
                // Air is a brush like any other — the clear one — and the flag's own tile is air
                // with an entity on it, which the branch above caught.
                Some(Tile::Empty | Tile::Goal) => self.brush = Some(Brush::Empty),
                None => return false,
            }
        }
        let now = (self.brush, self.scene_sprite);
        if now == before {
            return false;
        }
        let label = match self.brush {
            Some(brush) => brush.label(),
            None => "nothing",
        };
        self.message = format!("picked {label}");
        true
    }

    /// Undo one step, if there is one.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.level, previous));
        self.after_history_change();
        true
    }

    /// Redo one step, if there is one.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.level, next));
        self.after_history_change();
        true
    }

    fn after_history_change(&mut self) {
        self.stroke_open = false;
        self.last_paint = None;
        self.dirty = true;
        self.message = "history moved".to_string();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Resize the grid, reporting what had to move to fit.
    pub fn resize(&mut self, width: i32, height: i32, fill: Tile) {
        self.push_undo();
        let report = self.level.resize(width, height, fill);
        self.dirty = true;
        let mut parts = Vec::new();
        if report.dropped_walkers > 0 {
            parts.push(format!(
                "{} walker(s) fell off the map",
                report.dropped_walkers
            ));
        }
        if report.spawn_moved {
            parts.push("spawn moved".to_string());
        }
        if report.dropped_scenery > 0 {
            parts.push(format!("{} scenery dropped", report.dropped_scenery));
        }
        if report.goal_moved {
            parts.push("goal pulled back to the edge".to_string());
        }
        self.message = if parts.is_empty() {
            format!("resized to {}x{}", self.level.width, self.level.height)
        } else {
            format!(
                "resized to {}x{}: {}",
                self.level.width,
                self.level.height,
                parts.join(", ")
            )
        };
    }

    /// Flip the whole document left to right.
    ///
    /// Mirroring is the most useful verb a level designer has — it is how one stroke becomes a
    /// there-and-back course — and the core has held a tested implementation of it all along
    /// ([`Level::mirrored_horizontal`], exercised by the sim's own symmetry tests), while the
    /// editor had no way to ask for it: the palette has eleven brushes and not one of them acts
    /// on the document (AUDIT.md F3). This is the call site that was missing.
    ///
    /// Two things are the editor's business rather than the level's. The **name** is kept: `sim`
    /// marks its mirror `"X mirrored"` because a mirrored level is a *new level* there, whereas
    /// this is the same document with its terrain flipped, and taking the name would move the
    /// file it saves to. And the operation is **one undo step** like any other stroke, because a
    /// person who flips the wrong level wants the same key back that every other mistake takes.
    pub fn mirror_horizontal(&mut self) -> bool {
        self.push_undo();
        let name = self.level.name.clone();
        self.level = self.level.mirrored_horizontal();
        self.level.name = name;
        self.dirty = true;
        self.stroke_open = false;
        self.last_paint = None;
        self.shape_anchor = None;
        self.message = "mirrored left to right".to_string();
        true
    }

    /// Rename the document.
    ///
    /// The name is not decoration: it is the file a save writes ([`Editor::save`] slugs it),
    /// the word on the shelf card, and the stem any champion trained on this level will wear.
    /// Undoable like every other edit, because it *is* one — the alternative, a name that only
    /// the file carries, is how two levels came to share the one word `untitled`.
    ///
    /// Blank input is refused rather than slugged to `untitled`: an empty name is a name
    /// somebody is still typing, and taking it would rename the document under their cursor.
    /// A name that is already the document's own is not an edit and does not enter the
    /// history. Returns whether anything changed.
    pub fn rename(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || name == self.level.name {
            return false;
        }
        self.push_undo();
        self.level.name = name.to_string();
        self.dirty = true;
        self.message = format!("renamed to {name}");
        true
    }

    /// Re-run the reachability oracle when — and only when — the document changed.
    ///
    /// The change is detected by hashing the level rather than by setting a flag in
    /// every mutation, for two reasons. A flag has to be remembered by every writer,
    /// including the ones that reach the level directly (a test, a future tool, an
    /// undo step); a hash cannot forget. And hashing 900 tiles is far cheaper than the
    /// oracle, so the guard is worth more than it costs.
    ///
    /// Called once per frame rather than once per paint: a stroke can cover many
    /// tiles, and the oracle only needs to see the result.
    pub fn refresh_report(&mut self) {
        let hash = self.level.content_hash();
        if hash != self.report_hash {
            self.report = sim::reach::report(&self.level);
            self.report_hash = hash;
        }
    }

    /// Can this level be beaten in principle? The oracle's answer, not an opinion.
    pub fn beatable(&self) -> bool {
        self.report.goal_reached
    }

    /// A one-line summary of what the oracle thinks.
    pub fn verdict(&self) -> String {
        if self.report.goal_reached {
            format!(
                "beatable: flag reachable over {} platform(s)",
                self.report.reachable_platforms()
            )
        } else {
            match &self.report.diagnosis {
                Some(why) => format!("NOT beatable: {why}"),
                None => "NOT beatable: the flag cannot be reached".to_string(),
            }
        }
    }

    /// Save to the current path, or to `path` and remember it.
    pub fn save(&mut self, path: Option<PathBuf>) -> Result<PathBuf, String> {
        let raw_target = match path.or_else(|| self.path.clone()) {
            Some(p) => p,
            None => {
                return Err("no file name yet: use Save as, or open a level first".into());
            }
        };
        let target = protect_shipped_path(&raw_target);
        let redirected = target != raw_target;
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
        }
        // The document refuses to write a level the loader would reject, and its
        // error text is the message the user sees.
        self.level.to_json_file(&target)?;
        let name = self.level.name.clone();
        self.path = Some(target.clone());
        self.dirty = false;
        self.message = if redirected {
            format!(
                "saved {name} to {} (shipped level kept intact)",
                target.display()
            )
        } else {
            format!("saved {name} to {}", target.display())
        };
        Ok(target)
    }

    /// Every level in a directory, for the open menu.
    pub fn levels_in(dir: impl AsRef<Path>) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
            .collect();
        out.sort();
        out
    }
}

/// The tiles on the straight run from `from` to `to`, both ends included.
///
/// Walked by fraction of the way along the line rather than by Bresenham's integer form,
/// for the reason the shape tools exist: a one-tile diagonal and a forty-tile diagonal are
/// the same gesture, and the step count is the *longer* axis, so two tiles the walk puts
/// next to each other never differ by more than one on either axis. That is what makes a
/// drawn line a connected run rather than a dotted one.
fn line_tiles(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let ((x0, y0), (x1, y1)) = (from, to);
    let steps = (x1 - x0).abs().max((y1 - y0).abs());
    if steps == 0 {
        return vec![to];
    }
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            (
                x0 + ((x1 - x0) as f32 * t).round() as i32,
                y0 + ((y1 - y0) as f32 * t).round() as i32,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../levels")
    }

    /// A playable document: the editor's own starter, so these tests exercise what a
    /// user gets when they press New rather than a fixture only tests can build.
    fn doc() -> Editor {
        Editor::fresh("test-level", 40, 12)
    }

    #[test]
    fn a_drag_paints_one_undo_step_and_leaves_no_gaps() {
        let mut ed = doc();
        ed.brush = Some(Brush::Solid);
        ed.begin_stroke();
        assert!(ed.paint(2, 2));
        // A fast drag jumps tiles between frames: the stroke must fill the line.
        assert!(ed.paint(8, 2));
        ed.end_stroke();

        for tx in 2..=8 {
            assert_eq!(
                ed.level.tile(tx, 2),
                Some(Tile::Solid),
                "column {tx} was skipped by the stroke"
            );
        }
        assert!(ed.dirty);

        // One gesture, one undo step: the whole line goes back together.
        assert!(ed.undo());
        for tx in 2..=8 {
            assert_eq!(ed.level.tile(tx, 2), Some(Tile::Empty), "column {tx}");
        }
        assert!(ed.can_redo());
        assert!(ed.redo());
        assert_eq!(ed.level.tile(5, 2), Some(Tile::Solid));
    }

    /// A walker is refused where it would fall, and the refusal says why.
    ///
    /// This is the shipped pack's own rule (`sim`'s `walker_stands_on_ground`), applied when
    /// the click happens rather than by a test afterwards — the author's own level had five
    /// walkers in it, two of them hanging in the air, because nothing drew them and nothing
    /// said no.
    #[test]
    fn a_walker_is_refused_where_it_would_have_nothing_to_stand_on() {
        let mut ed = doc();
        ed.brush = Some(Brush::Walker);

        // The starter's floor is its bottom row, so the tile above it holds a walker...
        let floor_row = ed.level.height - 1;
        ed.begin_stroke();
        assert!(ed.paint(10, floor_row - 1));
        ed.end_stroke();
        assert_eq!(ed.level.walker_tiles(), vec![(10, floor_row - 1)]);

        // ...and the same column seven rows up does not.
        ed.begin_stroke();
        assert!(!ed.paint(10, floor_row - 8));
        ed.end_stroke();
        assert_eq!(
            ed.level.walker_tiles(),
            vec![(10, floor_row - 1)],
            "the mid-air walker was written anyway"
        );
        assert!(
            ed.message.contains("ground"),
            "the refusal should say why: {:?}",
            ed.message
        );

        // The preview and the write agree, which is what keeps the ghost honest.
        assert!(ed.can_place(Brush::Walker, 10, floor_row - 1));
        assert!(!ed.can_place(Brush::Walker, 10, floor_row - 8));

        // And a solid tile where the walker would hang makes it legal again.
        ed.brush = Some(Brush::Solid);
        ed.begin_stroke();
        ed.paint(10, floor_row - 7);
        ed.end_stroke();
        ed.brush = Some(Brush::Walker);
        ed.begin_stroke();
        assert!(ed.paint(10, floor_row - 8));
        ed.end_stroke();
    }

    /// The scenery brush places the sprite the picker chose, refuses a tree with no room,
    /// and the eraser takes the decoration before the floor under it.
    #[test]
    fn the_scenery_brush_places_its_sprite_and_the_eraser_takes_it_back() {
        let mut ed = doc();
        let floor_row = ed.level.height - 1;
        ed.brush = Some(Brush::Scenery);
        ed.scene_sprite = "bush_c";
        ed.begin_stroke();
        assert!(ed.paint(8, floor_row - 1));
        ed.end_stroke();
        assert_eq!(ed.level.scenery_at(8, floor_row - 1), Some("bush_c"));
        assert!(ed.dirty);

        // A tree needs three open cells; under a ceiling it is refused and says why.
        ed.scene_sprite = "tree_a_base";
        ed.level.set_tile(20, floor_row - 2, Tile::Solid);
        ed.begin_stroke();
        assert!(!ed.paint(20, floor_row - 1));
        ed.end_stroke();
        assert!(ed.message.contains("scenery"), "{}", ed.message);
        assert_eq!(ed.level.scenery_at(20, floor_row - 1), None);

        // The eraser clears decoration before terrain, so one click on a bush takes the
        // bush and leaves the floor it stands on.
        ed.brush = Some(Brush::Erase);
        ed.begin_stroke();
        ed.paint(8, floor_row - 1);
        ed.end_stroke();
        assert_eq!(ed.level.scenery_at(8, floor_row - 1), None);
        assert_eq!(
            ed.level.tile(8, floor_row - 1),
            Some(Tile::Empty),
            "the cell under the scenery must be untouched"
        );
    }

    #[test]
    fn undo_restores_entities_and_the_spawn_too() {
        let mut ed = doc();
        let before = ed.level.content_hash();
        ed.brush = Some(Brush::Walker);
        ed.begin_stroke();
        ed.paint(10, 10);
        ed.end_stroke();
        ed.brush = Some(Brush::Spawn);
        ed.begin_stroke();
        ed.paint(4, 10);
        ed.end_stroke();
        ed.brush = Some(Brush::Goal);
        ed.begin_stroke();
        ed.paint(20, 10);
        ed.end_stroke();
        assert_ne!(ed.level.content_hash(), before);
        assert_eq!(ed.level.walker_tiles(), vec![(10, 10)]);

        while ed.can_undo() {
            ed.undo();
        }
        assert_eq!(ed.level.content_hash(), before, "undo must reach the start");
    }

    #[test]
    fn the_eraser_takes_an_entity_before_the_terrain_under_it() {
        let mut ed = doc();
        ed.brush = Some(Brush::Walker);
        ed.begin_stroke();
        ed.paint(6, 10);
        ed.end_stroke();
        // Something solid under the walker, so the eraser has a choice to make.
        ed.level.set_tile(6, 10, Tile::Solid);

        ed.brush = Some(Brush::Erase);
        ed.begin_stroke();
        ed.paint(6, 10);
        ed.end_stroke();
        assert!(ed.level.walker_tiles().is_empty(), "the entity goes first");
        assert_eq!(
            ed.level.tile(6, 10),
            Some(Tile::Solid),
            "the terrain under it survives one erase"
        );

        ed.begin_stroke();
        ed.paint(6, 10);
        ed.end_stroke();
        assert_eq!(ed.level.tile(6, 10), Some(Tile::Empty));
    }

    /// The flag on a new level is the user's to delete — and the editor says what a level
    /// with no flag costs, rather than refusing the edit.
    ///
    /// The rule this replaces was the other way round: the document would not remove its last
    /// goal, so the flag [`Level::starter`] plants on a fresh level could not be cleared out
    /// of the way at all. With that gone, the document spends a moment with no flag, so the
    /// two things that depend on one — the save and the verdict — have to answer instead.
    #[test]
    fn the_goal_can_be_erased_away_and_the_editor_says_what_that_costs() {
        let mut ed = doc();
        let (gx, gy) = ed.level.goal_tiles()[0];
        ed.brush = Some(Brush::Erase);
        ed.begin_stroke();
        assert!(ed.paint(gx, gy), "a click on the pole erased nothing");
        ed.end_stroke();
        assert!(ed.level.goals.is_empty(), "the starter's flag comes off");

        // A document with no flag cannot be saved, and the refusal leaves no file behind.
        let path = std::env::temp_dir().join(format!("evo-flagless-{}.json", std::process::id()));
        let err = ed
            .save(Some(path.clone()))
            .expect_err("a flag-less level is not writable");
        assert!(err.contains("no goal"), "{err}");
        assert!(!path.exists(), "a refused save must not leave a file behind");

        // Nor can it be beaten, and the per-frame oracle run has to survive being asked.
        ed.refresh_report();
        assert!(!ed.beatable());
        assert_eq!(ed.verdict(), "NOT beatable: the level has no goal flag");

        // Putting a flag down anywhere makes it a level again.
        ed.brush = Some(Brush::Goal);
        ed.begin_stroke();
        assert!(ed.paint(6, 10), "a flag could not be placed");
        ed.end_stroke();
        assert_eq!(ed.level.goal_tiles(), vec![(6, 10)]);
        assert!(ed.level.to_json_pretty().is_ok());
        ed.refresh_report();
        assert!(ed.beatable(), "{}", ed.verdict());
    }

    /// The bug found by playing with it: a flag could be placed but the one already on the
    /// map could not be deleted.
    ///
    /// The flag is drawn one cell *above* the goal's own tile, so the click aimed at the flag
    /// — the only part of the marker with any colour in it — landed on a cell no goal
    /// claimed, and the erase fell through to a tile that was already empty. Both cells
    /// address the marker now (`Level::goal_covers`).
    #[test]
    fn the_eraser_takes_a_goal_by_the_flag_the_user_can_see() {
        let mut ed = doc();
        let (gx, gy) = ed.level.goal_tiles()[0];

        // Aim at the flag, one cell above the goal's own tile.
        ed.brush = Some(Brush::Erase);
        ed.begin_stroke();
        assert!(ed.paint(gx, gy - 1), "a click on the flag erased nothing");
        ed.end_stroke();
        assert!(ed.level.goals.is_empty());

        // And the flag can then be put down where the user wanted it instead.
        ed.brush = Some(Brush::Goal);
        ed.begin_stroke();
        assert!(ed.paint(4, 10));
        ed.end_stroke();
        assert_eq!(ed.level.goal_tiles(), vec![(4, 10)]);
        assert!(ed.level.to_json_pretty().is_ok());
    }

    /// **One flag per level**: a goal placed anywhere else *moves* the marker there, so a
    /// level cannot end up with two — the move is what the user sees, and the status line
    /// says it happened, because the flag leaves one end of the level for the other.
    #[test]
    fn placing_a_goal_elsewhere_moves_the_one_that_is_there() {
        let mut ed = doc();
        let (gx, gy) = ed.level.goal_tiles()[0];
        assert!(ed.can_place(Brush::Goal, 6, 10), "across the level is free");

        ed.brush = Some(Brush::Goal);
        ed.begin_stroke();
        assert!(ed.paint(6, 10));
        ed.end_stroke();
        assert_eq!(ed.level.goals.len(), 1, "a level has one flag, not two");
        assert_eq!(ed.level.goal_tiles(), vec![(6, 10)]);
        assert!(!ed.level.goal_covers(gx, gy), "nothing was left behind");
        assert!(ed.message.contains("goal"), "{}", ed.message);
        assert!(ed.level.to_json_pretty().is_ok());
    }

    /// The marker's own two cells are refused, and the ghost says so before the click — which
    /// matters because the cell that refuses it is usually the one *above* the cursor rather
    /// than the one under it.
    #[test]
    fn the_ghost_and_the_document_agree_that_the_goal_is_already_here() {
        let mut ed = doc();
        let (gx, gy) = ed.level.goal_tiles()[0];
        assert!(!ed.can_place(Brush::Goal, gx, gy), "the pole's own cell");
        assert!(
            !ed.can_place(Brush::Goal, gx, gy - 1),
            "the cell the flag hangs in"
        );
        assert!(ed.can_place(Brush::Goal, gx - 2, gy), "two columns left is free");

        ed.brush = Some(Brush::Goal);
        ed.begin_stroke();
        assert!(!ed.paint(gx, gy - 1), "the document refuses it too");
        ed.end_stroke();
        assert_eq!(ed.level.goals.len(), 1);
        assert_eq!(ed.level.goal_tiles(), vec![(gx, gy)], "the flag did not shift");
        assert!(
            ed.message.contains("goal"),
            "the refusal should say why: {:?}",
            ed.message
        );
    }

    /// A tree is addressed by its canopy as well as its trunk, so the eraser takes the tree
    /// when the click lands in its leaves — the same rule, for the same reason.
    #[test]
    fn the_eraser_takes_a_tree_by_its_canopy() {
        let mut ed = doc();
        let floor_row = ed.level.height - 1;
        ed.brush = Some(Brush::Scenery);
        ed.scene_sprite = "tree_a_base";
        ed.begin_stroke();
        assert!(ed.paint(15, floor_row - 1));
        ed.end_stroke();

        ed.brush = Some(Brush::Erase);
        ed.begin_stroke();
        assert!(
            ed.paint(15, floor_row - 3),
            "a click in the leaves erased nothing"
        );
        ed.end_stroke();
        assert_eq!(ed.level.scenery_at(15, floor_row - 1), None);
    }

    #[test]
    fn saving_writes_a_file_that_opens_back_identical() {
        let mut ed = doc();
        ed.brush = Some(Brush::Spike);
        ed.begin_stroke();
        ed.paint(12, 9);
        ed.paint(13, 9);
        ed.end_stroke();
        ed.brush = Some(Brush::Walker);
        ed.begin_stroke();
        ed.paint(20, 10);
        ed.end_stroke();

        let dir = std::env::temp_dir().join(format!("evo-editor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("saved.json");
        ed.save(Some(path.clone())).expect("save");
        assert!(!ed.dirty, "saving clears the edited flag");
        assert_eq!(ed.path.as_deref(), Some(path.as_path()));

        let reopened = Editor::open(&path).expect("open");
        assert_eq!(
            reopened.level.content_hash(),
            ed.level.content_hash(),
            "the file must describe the same level"
        );
        assert_eq!(reopened.level.walker_tiles(), vec![(20, 10)]);
        assert_eq!(reopened.level.goal_tiles(), ed.level.goal_tiles());
        assert!(!reopened.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_level_that_cannot_be_saved_says_so_instead_of_writing() {
        let mut ed = doc();
        ed.brush = Some(Brush::Solid);
        ed.begin_stroke();
        ed.paint(5, 5);
        ed.end_stroke();
        assert!(ed.dirty);

        ed.level.goals.clear();
        let path = std::env::temp_dir().join(format!("evo-refused-{}.json", std::process::id()));
        let err = ed
            .save(Some(path.clone()))
            .expect_err("a goal-less level is not writable");
        assert!(err.contains("no goal"), "{err}");
        assert!(
            !path.exists(),
            "a refused save must not leave a file behind"
        );
        assert!(ed.dirty, "a refused save leaves the document edited");
    }

    #[test]
    fn the_verdict_is_the_oracles_answer_not_an_opinion() {
        let mut ed = doc();
        assert!(ed.beatable(), "a fresh starter level is beatable");
        assert!(ed.verdict().starts_with("beatable"));

        // Wall the goal off with a spike curtain the jump cannot clear: the oracle
        // must notice, and the editor's verdict must follow it.
        let goal_x = ed.level.goal_tiles()[0].0;
        for ty in 0..ed.level.height {
            ed.level.set_tile(goal_x - 1, ty, Tile::Spike);
        }
        for tx in goal_x - 1..ed.level.width {
            ed.level.set_tile(tx, ed.level.height - 2, Tile::Spike);
        }
        ed.refresh_report();
        assert!(
            !ed.beatable(),
            "spikes across the whole approach are not beatable"
        );
        assert!(ed.verdict().starts_with("NOT beatable"));
    }

    #[test]
    fn resizing_reports_what_it_had_to_move() {
        let mut ed = doc();
        ed.resize(10, 8, Tile::Empty);
        assert!(
            (ed.level.width, ed.level.height) == (10, 8),
            "{:?}",
            ed.message
        );
        assert!(ed.message.contains("spawn moved"), "{}", ed.message);
        assert!(ed.level.validate().is_ok());
        // The pending-size pair the UI edits is deliberately not the document's.
        assert_eq!(ed.level.width, 10);
    }

    #[test]
    fn the_level_list_finds_the_shipped_pack() {
        let found = Editor::levels_in(levels_dir());
        assert!(found.len() >= 6, "found {} levels", found.len());
        assert!(found.iter().all(|p| p.extension().is_some()));
    }

    /// The two materials the pack can be made of and the palette could not paint.
    ///
    /// A brush that names a material and writes another is the class of bug the palette's
    /// icons exist to prevent, so each new one is asked for the tile it says it is.
    #[test]
    fn the_brick_and_timber_brushes_paint_their_own_tiles() {
        for (brush, tile) in [(Brush::Brick, Tile::Brick), (Brush::Plank, Tile::Plank)] {
            let mut ed = doc();
            ed.brush = Some(brush);
            ed.begin_stroke();
            assert!(ed.paint(6, 6), "{brush:?} painted nothing");
            ed.end_stroke();
            assert_eq!(ed.level.tile(6, 6), Some(tile), "{brush:?} wrote the wrong tile");
            // Both are solid, which is the whole of their physics: a level made of them is
            // not a new problem for the population.
            assert!(tile.is_solid());
            // And the palette can draw them, because a brush whose icon is missing is a
            // blank button rather than an error.
            assert!(!brush.sprite().is_empty());
        }
    }

    /// A rectangle drag is a shape, not a path: it fills the whole box, in both
    /// directions, and stays one undo step whatever the pointer did on the way.
    #[test]
    fn a_rect_drag_fills_the_box_and_is_one_undo_step() {
        let mut ed = doc();
        ed.brush = Some(Brush::Brick);
        ed.shape = Shape::Rect;
        let before = ed.level.content_hash();

        ed.begin_stroke();
        ed.paint(5, 3);
        // The pointer wanders, then comes back: a rectangle is redrawn from its corner each
        // time, so the tiles the wander passed over must not be left behind.
        ed.paint(9, 5);
        ed.paint(8, 4);
        ed.end_stroke();

        // The last frame was 5,3 -> 8,4: a filled four-by-two box.
        for ty in 3..=4 {
            for tx in 5..=8 {
                assert_eq!(
                    ed.level.tile(tx, ty),
                    Some(Tile::Brick),
                    "({tx}, {ty}) is not in the box"
                );
            }
        }
        // And nothing outside it: the corner the pointer had reached is air again.
        assert_eq!(ed.level.tile(9, 3), Some(Tile::Empty));
        assert_eq!(ed.level.tile(9, 5), Some(Tile::Empty));

        // One step back takes the whole gesture away, not the last frame of it.
        assert!(ed.undo());
        assert_eq!(ed.level.content_hash(), before, "undo left part of the drag");
    }

    /// A line drag draws a connected run between its two ends, both included.
    #[test]
    fn a_line_drag_leaves_a_connected_run() {
        let mut ed = doc();
        ed.brush = Some(Brush::Plank);
        ed.shape = Shape::Line;
        ed.begin_stroke();
        ed.paint(3, 2);
        ed.paint(18, 8);
        ed.end_stroke();

        let filled: Vec<(i32, i32)> = (0..ed.level.height)
            .flat_map(|ty| (0..ed.level.width).map(move |tx| (tx, ty)))
            .filter(|(tx, ty)| ed.level.tile(*tx, *ty) == Some(Tile::Plank))
            .collect();
        assert!(filled.len() >= 16, "a 15-tile line drew {} tiles", filled.len());
        assert!(filled.contains(&(3, 2)), "the line's start is missing");
        assert!(filled.contains(&(18, 8)), "the line's end is missing");

        // No holes: the sets of tiles has to be one piece. Walked with the paint's own
        // neighbours rather than by comparing consecutive pairs — a sorted list of tiles
        // says nothing about whether the run between them is whole.
        let set: std::collections::HashSet<(i32, i32)> = filled.iter().copied().collect();
        let mut seen: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::from([filled[0]]);
        while let Some((x, y)) = queue.pop_front() {
            if !seen.insert((x, y)) {
                continue;
            }
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let next = (x + dx, y + dy);
                    if set.contains(&next) && !seen.contains(&next) {
                        queue.push_back(next);
                    }
                }
            }
        }
        assert_eq!(
            seen.len(),
            set.len(),
            "the drawn line is in {} pieces",
            set.len() - seen.len() + 1
        );
    }

    /// A start in mid-air is refused out loud, like the walker's own refusal.
    #[test]
    fn the_spawn_refuses_a_cell_with_no_ground_under_it() {
        let mut ed = doc();
        ed.brush = Some(Brush::Spawn);
        let (sx, sy) = ed.level.spawn_tile();
        assert!(!ed.can_place(Brush::Spawn, sx, sy - 4), "mid-air");
        assert!(
            !ed.can_place(Brush::Spawn, sx, ed.level.height - 1),
            "inside the floor"
        );

        ed.begin_stroke();
        assert!(!ed.paint(sx, sy - 4), "a mid-air start must not be written");
        ed.end_stroke();
        assert_eq!(ed.level.spawn_tile(), (sx, sy), "the start moved anyway");
        assert!(
            ed.message.contains("ground"),
            "the refusal has to say why: {}",
            ed.message
        );

        // And a legal move is taken, on the floor further along.
        let (fx, fy) = (sx + 5, ed.level.height - 2);
        assert!(ed.can_place(Brush::Spawn, fx, fy), "the floor is a start");
        ed.begin_stroke();
        assert!(ed.paint(fx, fy));
        ed.end_stroke();
        assert_eq!(ed.level.spawn_tile(), (fx, fy));
    }

    /// A new document's own start already satisfies the rule the brush enforces.
    #[test]
    fn a_fresh_document_starts_on_ground() {
        for (w, h) in [(40, 12), (8, 5), (120, 30)] {
            let ed = Editor::fresh("start", w, h);
            let (sx, sy) = ed.level.spawn_tile();
            assert!(
                ed.can_place(Brush::Spawn, sx, sy),
                "a {w}x{h} starter has no legal start at ({sx}, {sy})"
            );
        }
    }

    /// Saving a shipped level redirects to `custom-<stem>.json` inside a `levels` directory
    /// so drawing on the default canvas and pressing Save never overwrites the built-in pack.
    #[test]
    fn saving_a_shipped_level_protects_the_built_in_file() {
        let dir = std::env::temp_dir()
            .join(format!("evo-protect-{}", std::process::id()))
            .join("levels");
        std::fs::create_dir_all(&dir).expect("temp levels dir");
        let shipped = dir.join("l1-flat-run.json");
        let original = Level::starter("L1-flat-run", 60, 15);
        original.to_json_file(&shipped).expect("write original");
        let orig_hash = original.content_hash();

        let mut ed = Editor::open(&shipped).expect("open shipped");
        ed.brush = Some(Brush::Spike);
        ed.begin_stroke();
        ed.paint(10, 13);
        ed.end_stroke();

        let saved_to = ed.save(None).expect("save redirects");
        assert_eq!(saved_to, dir.join("custom-l1-flat-run.json"));
        assert_eq!(ed.path.as_deref(), Some(saved_to.as_path()));
        assert!(ed.message.contains("shipped level kept intact"));

        let untouched = Level::from_json_file(&shipped).expect("read original");
        assert_eq!(untouched.content_hash(), orig_hash, "shipped file stayed intact");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// A new level takes a name the pack does not already hold.
    ///
    /// The bug this pins lost work silently: every **New** document was called `untitled`, the
    /// name *is* the file a save writes, and so the second Save wrote over the first — and the
    /// shelf then showed two cards with one name on them (AUDIT.md B2). The numbering is counted
    /// from the directory rather than from a counter in memory, so a name that exists only
    /// because an earlier window wrote it is still respected.
    #[test]
    fn a_new_level_takes_a_name_the_pack_does_not_already_hold() {
        let dir = std::env::temp_dir().join(format!("evo-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(free_level_name(&dir, "untitled"), "untitled");
        std::fs::write(dir.join("untitled.json"), "{}").unwrap();
        assert_eq!(free_level_name(&dir, "untitled"), "untitled-2");
        std::fs::write(dir.join("untitled-2.json"), "{}").unwrap();
        assert_eq!(
            free_level_name(&dir, "untitled"),
            "untitled-3",
            "the numbering walks until the pack has room"
        );

        // A document that already has a name keeps it while the name is free, and only a name
        // the pack holds is numbered: a save must not rename what the person called it.
        assert_eq!(free_level_name(&dir, "tide-pools"), "tide-pools");
        std::fs::write(dir.join("tide-pools.json"), "{}").unwrap();
        assert_eq!(free_level_name(&dir, "tide-pools"), "tide-pools-2");

        // The name offered is the *slug*, because the file it makes room for is the slug. The
        // dock, the shelf card, the window title and the file all have to be one word.
        assert_eq!(free_level_name(&dir, "My Level"), "my-level");
        std::fs::write(dir.join("my-level.json"), "{}").unwrap();
        assert_eq!(free_level_name(&dir, "My Level"), "my-level-2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A mirror flips the terrain, the entities and the scenery, and undo puts it all back.
    ///
    /// The bug this exists for is a capability the editor simply did not expose: `sim` has
    /// mirrored levels in its own tests for as long as levels have existed, and a person drawing
    /// a course had to retype the return trip by hand (AUDIT.md F3). Four facts, and the last two
    /// are the ones a wrong implementation gets wrong.
    #[test]
    fn a_mirror_flips_the_terrain_the_entities_and_the_name_survives_it() {
        let mut ed = doc();
        // Something asymmetric to look at, past the starter's own floor and spawn.
        ed.level.set_tile(3, ed.level.height - 3, Tile::Spike);
        ed.level.name = "asymmetric".to_string();
        let before = ed.level.clone();
        let width = ed.level.width;

        assert!(ed.mirror_horizontal());
        assert_eq!(
            ed.level.tile(width - 1 - 3, ed.level.height - 3),
            Some(Tile::Spike),
            "the hazard moved to the far side"
        );
        assert_eq!(ed.level.tile(3, ed.level.height - 3), Some(Tile::Empty));
        // The start and the flag are entities, and they swap sides with the terrain: the starter
        // puts its spawn on the second column and its flag two in from the far edge.
        let (sx, sy) = before.spawn_tile();
        assert_eq!(ed.level.spawn_tile(), (width - 1 - sx, sy));
        assert_eq!(ed.level.goal_tiles().len(), 1, "one flag, mirrored not copied");
        assert_ne!(
            ed.level.goal_tiles().first(),
            before.goal_tiles().first(),
            "the flag crossed over"
        );

        // The document keeps its *own* name: `sim` names a mirror "X mirrored" because there it
        // is a new level, and here the file it saves to must not move.
        assert_eq!(ed.level.name, "asymmetric");
        assert!(ed.dirty);

        // And the whole thing is one undo step.
        assert!(ed.undo());
        assert_eq!(ed.level.content_hash(), before.content_hash());
        assert_eq!(ed.level.name, "asymmetric");
    }

    /// The eyedropper takes the brush from the level, entity first and material second.
    ///
    /// The bug this pins is a missing *tool* rather than a wrong answer: matching a material
    /// already on the level meant guessing which of four palette cells the author had used, and
    /// there was no way at all to pick a prop out of the scenery row (AUDIT.md F4).
    #[test]
    fn picking_takes_the_brush_from_whatever_is_under_the_pointer() {
        let mut ed = doc();
        let floor = ed.level.height - 1;

        // A material picks its own brush — and every one of the four is a different brush.
        for (tx, brush) in [
            (Tile::Solid, Brush::Solid),
            (Tile::Sand, Brush::Sand),
            (Tile::Brick, Brush::Brick),
            (Tile::Plank, Brush::Plank),
        ] {
            ed.level.set_tile(5, floor, tx);
            ed.brush = Some(Brush::Erase);
            assert!(ed.pick_at(5, floor));
            assert_eq!(ed.brush, Some(brush), "{tx:?} picked the wrong brush");
        }
        ed.level.set_tile(5, floor, Tile::Spike);
        assert!(ed.pick_at(5, floor));
        assert_eq!(ed.brush, Some(Brush::Spike));

        // Air is the clear brush rather than a refusal: clearing is a brush, so picking air has
        // to hand it back.
        assert!(ed.pick_at(5, 0));
        assert_eq!(ed.brush, Some(Brush::Empty));

        // The spawn is an entity on a tile, and it wins over the floor under it.
        let (sx, sy) = ed.level.spawn_tile();
        assert!(ed.pick_at(sx, sy));
        assert_eq!(ed.brush, Some(Brush::Spawn));

        // A walker wins over the ground it stands on: you cannot place two in a cell, and the
        // terrain is under half the level.
        ed.level.set_tile(20, floor - 1, Tile::Solid);
        ed.level.set_tile(20, floor, Tile::Solid);
        ed.level.add_walker_tile(20, floor - 1);
        assert!(ed.pick_at(20, floor - 1));
        assert_eq!(ed.brush, Some(Brush::Walker));

        // An outside tile is not a pick, and says so rather than silently keeping the brush.
        ed.brush = Some(Brush::Solid);
        assert!(!ed.pick_at(-1, -1));
        assert_eq!(ed.brush, Some(Brush::Solid));

        // What is already in hand is not news: the same pick twice reports nothing the second
        // time, which is what keeps the status line from being chatter.
        assert!(ed.pick_at(20, floor - 1) == false || ed.brush == Some(Brush::Walker));
        assert!(!ed.pick_at(20, floor - 1));
    }

    /// Picking a placed prop takes the prop, not the first one in the list.
    #[test]
    fn picking_a_prop_takes_its_own_name_out_of_the_palette() {
        let mut ed = doc();
        let (tx, ty) = (10, ed.level.height - 2);
        ed.level.set_tile(tx, ty, Tile::Solid);
        let sprite = SCENERY_SPRITES[3];
        ed.level.add_scenery(sprite, tx, ty - 1);
        // Whatever was in hand, the pick hands back the sprite that is *on the level*.
        ed.scene_sprite = SCENERY_SPRITES[0];
        ed.brush = Some(Brush::Solid);
        assert!(ed.pick_at(tx, ty - 1));
        assert_eq!(ed.brush, Some(Brush::Scenery));
        assert_eq!(ed.scene_sprite, sprite);
    }

    /// Shift forces one stroke to be a straight line, whatever the palette's shape row says.
    ///
    /// The bug this pins is the same missing-tool story as the eyedropper: a long floor is the one
    /// thing a steady hand cannot draw, and asking for it meant changing the shape and remembering
    /// to change it back (AUDIT.md F4).
    #[test]
    fn shift_forces_the_stroke_to_a_straight_line() {
        let mut ed = doc();
        ed.brush = Some(Brush::Solid);
        assert_eq!(ed.shape, Shape::Free, "freehand is the default");
        ed.line = true;
        ed.begin_stroke();
        // A drag that wanders: a line lays down the straight run from the anchor to the pointer,
        // so the tile in the middle of the box is *not* painted and the ends are.
        ed.paint(2, 2);
        ed.paint(6, 8);
        ed.end_stroke();
        assert_eq!(ed.level.tile(2, 2), Some(Tile::Solid), "the anchor is painted");
        assert_eq!(ed.level.tile(6, 8), Some(Tile::Solid), "the pointer is painted");
        assert_eq!(
            ed.level.tile(2, 8),
            Some(Tile::Empty),
            "a line is not a rectangle: the corner is left alone"
        );

        // And with the key up the next stroke is freehand again, which is the whole point of it
        // being a key rather than a mode.
        ed.line = false;
        ed.begin_stroke();
        ed.paint(10, 4);
        ed.paint(12, 8);
        ed.end_stroke();
        assert_eq!(
            ed.level.tile(10, 8),
            Some(Tile::Empty),
            "freehand fills only the tiles the pointer crossed"
        );
        assert_eq!(
            ed.level.tile(12, 8),
            Some(Tile::Solid),
            "...and it does reach the pointer"
        );
    }

    /// A rename is an ordinary edit: it dirties the document and undo takes the whole name back.
    #[test]
    fn renaming_is_an_edit_that_undo_takes_back() {
        let mut ed = doc();
        assert!(!ed.dirty, "a fresh document is not edited");
        assert!(ed.rename("the tide pools"));
        assert_eq!(ed.level.name, "the tide pools");
        assert!(ed.dirty, "a renamed document is not on disk any more");

        // A blank name is refused rather than slugged to `untitled`: it is somebody still
        // typing, and taking it would rename the document under their cursor.
        assert!(!ed.rename("   "));
        assert_eq!(ed.level.name, "the tide pools");
        // The name it already has is not an edit, so it does not enter the history either.
        assert!(!ed.rename("the tide pools"));

        assert!(ed.undo());
        assert_eq!(ed.level.name, "test-level", "undo takes the name back");
    }
}
