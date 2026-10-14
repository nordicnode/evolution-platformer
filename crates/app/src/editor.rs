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

use std::path::{Path, PathBuf};

use sim::reach::ReachReport;
use sim::{Level, Tile};

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

impl Brush {
    pub const ALL: [Brush; 9] = [
        Brush::Solid,
        Brush::Sand,
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
    /// The one rule that needs deciding *before* the write is the walker's: everything else
    /// is refused by its own writer (a tile outside the grid does not land, a goal needs a
    /// free tile, the spawn is clamped into the grid), while a walker with no ground under
    /// it would be accepted by the document and refused by the pack's own test. The app
    /// asks this to colour the drop ghost, so a placement that will be refused looks
    /// refused before the mouse button comes up.
    ///
    /// A goal joins the walker here because its refusal is not obvious from the pointer: the
    /// marker is two cells tall, so the tile the marker already owns is often *not* the one
    /// under the cursor but the flag's own cell one above it.
    pub fn can_place(&self, brush: Brush, tx: i32, ty: i32) -> bool {
        match brush {
            Brush::Walker => self.level.walker_stands_on_ground(tx, ty),
            Brush::Scenery => self.level.scenery_placeable(self.scene_sprite, tx, ty),
            Brush::Goal => self.level.goal_placeable(tx, ty),
            _ => true,
        }
    }

    /// Apply the active brush at a tile. Returns whether the document changed.
    ///
    /// Extends the stroke from the previously painted tile, so a fast drag does not
    /// leave a dotted line where the pointer skipped tiles between frames.
    pub fn paint(&mut self, tx: i32, ty: i32) -> bool {
        let Some(brush) = self.brush else {
            return false;
        };
        if !self.stroke_open {
            self.begin_stroke();
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
            Brush::Spike => self.level.set_tile(tx, ty, Tile::Spike),
            Brush::Empty => self.level.set_tile(tx, ty, Tile::Empty),
            Brush::Spawn => {
                if self.level.spawn_tile() == (tx, ty) {
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
        let changed = self.paint_one(Brush::Erase, tx, ty);
        self.last_paint = Some((tx, ty));
        if changed {
            self.dirty = true;
        }
        changed
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
        let target = match path.or_else(|| self.path.clone()) {
            Some(p) => p,
            None => {
                return Err("no file name yet: use Save as, or open a level first".into());
            }
        };
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
        self.message = format!("saved {name} to {}", target.display());
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
}
