//! The level shelf: the pack, as pictures rather than as file names.
//!
//! "Open" used to be a menu of `l1-flat-run` and friends, which says nothing about what
//! any of them *is*. This module is the answer: a card per level, with a thumbnail drawn
//! from the level's own tiles, what it is made of, how big it is, whether it has a start
//! and a flag — and what the loader thought of it, because a level the pack refuses to
//! load is exactly the card worth showing greyed out with its reason.
//!
//! **The thumbnail is the terrain, not the art.** It is drawn from `sim::Tile` and the
//! theme's material colours ([`theme::material_colour`]), not from the atlas, for two
//! reasons: a level the window has not opened has no artist drawn on it yet, and a
//! ninety-tile level in a 160-pixel card is a map rather than a picture either way. What
//! it has to get right is *shape* — where the floor is, where the pits are, whether the
//! ground climbs — and that is a fact about the tile grid.
//!
//! **Nothing here reads the disk on a frame.** [`scan`] walks the directory and parses
//! every level once; the app holds the result and refreshes it when the shelf opens or a
//! document is saved. Parsing a pack per frame would be a file system call per card per
//! frame, which is the sort of thing that makes a window feel heavy for no visible
//! reason.

use std::path::{Path, PathBuf};

use egui::{Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, Vec2};
use sim::Level;

use crate::theme;

/// How wide a thumbnail is drawn, in points, and the tallest it may be.
///
/// A fixed width rather than an aspect-correct box: every card is the same size and the
/// thumbnails are compared *down* a column, which is what makes a shelf scannable. The
/// height follows the level's own aspect ratio, capped, so a tall level does not make its
/// card taller than the window.
pub const THUMB_WIDTH: f32 = 168.0;
/// The tallest thumbnail, in points. A level taller than this is drawn squashed.
pub const THUMB_MAX_HEIGHT: f32 = 84.0;

/// One card on the shelf: everything the shelf says about one file.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    /// The file's stem: the level's name on the shelf, and what a save writes back to.
    pub name: String,
    /// The width the level loads at, and the height. Zero when it did not load.
    pub size: (i32, i32),
    /// The materials the level is made of, in the order the palette lists them, with the
    /// hazard. Never empty for a level that loaded: bare air is still a level.
    pub materials: Vec<&'static str>,
    /// Whether the level has a start point and a flag. A level without them is one the
    /// pack's own tests refuse, so the shelf says so before it is opened.
    pub has_spawn: bool,
    pub has_goal: bool,
    /// Patrol walkers, which are the difference between a course and an obstacle.
    pub walkers: usize,
    /// The tiles, downsampled to the thumbnail's grid: one `Option<Color32>` per cell,
    /// `None` for air. Empty when the level did not load.
    pub thumb: Vec<Option<Color32>>,
    /// The thumbnail's own grid: columns then rows.
    pub thumb_size: (usize, usize),
    /// Why the level could not be opened, if it could not.
    pub error: Option<String>,
    /// The champion file this level has won, when one is in the champions directory.
    ///
    /// The link between the two halves of this game, and it was missing: a person could train a
    /// population on a level, save the champion, and the shelf would say nothing about it — the
    /// pack and the run had no visible connection at all (AUDIT.md V3). The *file name* is the
    /// link, because that is how the file was written: a run names its artifacts after the
    /// level's own stem ([`crate::lab::save_champion`]), so `l3-gauntlet.json` in the champions
    /// directory is a champion of `l3-gauntlet.json` in the pack.
    pub champion: Option<PathBuf>,
}

impl Entry {
    /// Whether this entry can be opened at all.
    pub fn loadable(&self) -> bool {
        self.error.is_none()
    }

    /// One line of facts, the way the shelf prints them under the thumbnail.
    pub fn summary(&self) -> String {
        if let Some(error) = &self.error {
            return format!("will not load: {error}");
        }
        let mut parts = vec![format!("{}x{}", self.size.0, self.size.1)];
        parts.push(if self.walkers == 0 {
            "no enemies".to_string()
        } else {
            format!("{} enemy(ies)", self.walkers)
        });
        if !self.has_spawn {
            parts.push("no start".to_string());
        }
        if !self.has_goal {
            parts.push("no flag".to_string());
        }
        parts.join("   ")
    }
}

/// Read every `*.json` level in `dir`, sorted by name.
///
/// The *list* of files is [`crate::editor::Editor::levels_in`]'s, not a second copy of the
/// rule: which files in a directory are levels is a question the editor already answers for
/// its own list of them, and two answers to it is how the shelf and the editor end up
/// disagreeing about what is in the pack. Sorted rather than in directory order — which
/// `levels_in` already does — so the shelf does not reshuffle under the pointer between two
/// frames on a machine whose readdir order differs. A file that is not a level is a card
/// with a reason, not a hole in the list: a broken file in the pack is exactly the thing a
/// person opens the shelf to find.
pub fn scan(dir: &Path, champions: Option<&Path>) -> Vec<Entry> {
    crate::editor::Editor::levels_in(dir)
        .iter()
        .map(|path| read_with(path, champions))
        .collect()
}

/// Read one level file into a card, real or refused, looking for a champion in `champions`.
///
/// `champions` is optional rather than a path with a default, because "there is no directory to
/// look in" and "the directory is empty" are different facts: a caller with nowhere to look must
/// not have a card claim there is no champion when it never asked.
pub fn read_with(path: &Path, champions: Option<&Path>) -> Entry {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    // A champion is a file *named* after the level, in the champions directory the run writes to.
    // Asked of the disk here rather than remembered by the app, because the shelf is a view of
    // what is on disk: a champion saved from the terminal while the window is open is on the next
    // card the shelf draws, and one deleted is off it.
    let champion = champions
        .map(|dir| dir.join(format!("{name}.json")))
        .filter(|p| p.is_file());
    match Level::from_json_file(path) {
        Ok(level) => {
            let (columns, rows) = thumbnail_grid(level.width, level.height);
            Entry {
                path: path.to_path_buf(),
                name,
                size: (level.width, level.height),
                materials: materials_of(&level),
                has_spawn: level.spawn_tile().0 >= 0,
                has_goal: !level.goal_tiles().is_empty(),
                walkers: level.walkers.len(),
                thumb: thumbnail(&level, columns, rows),
                thumb_size: (columns, rows),
                error: None,
                champion,
            }
        }
        Err(error) => Entry {
            path: path.to_path_buf(),
            name,
            size: (0, 0),
            materials: Vec::new(),
            has_spawn: false,
            has_goal: false,
            walkers: 0,
            thumb: Vec::new(),
            thumb_size: (0, 0),
            error: Some(error),
            champion: None,
        },
    }
}

/// Which materials the level is made of, in palette order, plus the hazard.
pub fn materials_of(level: &Level) -> Vec<&'static str> {
    use sim::Tile;
    let mut found = [false; 5];
    for ty in 0..level.height {
        for tx in 0..level.width {
            match level.tile(tx, ty) {
                Some(Tile::Solid) => found[0] = true,
                Some(Tile::Sand) => found[1] = true,
                Some(Tile::Brick) => found[2] = true,
                Some(Tile::Plank) => found[3] = true,
                Some(Tile::Spike) => found[4] = true,
                _ => {}
            }
        }
    }
    let names = ["soil", "sand", "brick", "timber", "spikes"];
    names
        .iter()
        .zip(found)
        .filter(|(_, seen)| *seen)
        .map(|(name, _)| *name)
        .collect()
}

/// The thumbnail's grid for a level of this size: at most [`MAX_THUMB_CELLS`] columns.
///
/// A cell covers a block of tiles rather than a fraction of one, so a thumbnail can only
/// ever *under*-report: a one-tile pillar may fall between two cells and not be drawn. That
/// is the right way round for a map of a level — the alternative is a thumbnail whose tiny
/// features depend on the window's pixel ratio — and the shape a person reads (the floor,
/// the gap, the climb) is always several cells wide.
pub const MAX_THUMB_CELLS: usize = 84;

pub fn thumbnail_grid(width: i32, height: i32) -> (usize, usize) {
    let width = width.max(1) as usize;
    let height = height.max(1) as usize;
    let columns = width.min(MAX_THUMB_CELLS);
    let scale = width.div_ceil(columns);
    let rows = height.div_ceil(scale);
    (columns, rows)
}

/// Downsample a level into thumbnail cells: the material of the *topmost* solid tile in
/// each block, or the hazard if a spike is in it.
///
/// Topmost rather than "any" because the thumbnail's job is the surface line: a column of
/// soil with a beach on top is a beach, and a block whose top tile is air but whose bottom
/// tile is floor has to draw the floor. Hazard and entities win over terrain so a level's
/// danger is visible at a glance.
pub fn thumbnail(level: &Level, columns: usize, rows: usize) -> Vec<Option<Color32>> {
    let mut out = vec![None; columns * rows];
    let scale_x = (level.width.max(1) as f32 / columns as f32).max(1.0);
    let scale_y = (level.height.max(1) as f32 / rows as f32).max(1.0);
    for row in 0..rows {
        for column in 0..columns {
            let tx0 = (column as f32 * scale_x).floor() as i32;
            let tx1 = (((column + 1) as f32 * scale_x).ceil() as i32).min(level.width);
            let ty0 = (row as f32 * scale_y).floor() as i32;
            let ty1 = (((row + 1) as f32 * scale_y).ceil() as i32).min(level.height);
            let mut seen: Option<Color32> = None;
            for ty in ty0..ty1.max(ty0 + 1) {
                for tx in tx0..tx1.max(tx0 + 1) {
                    if let Some(tile) = level.tile(tx, ty) {
                        if let Some(colour) = theme::material_colour(tile) {
                            // Topmost wins for terrain; anything wins over air. A spike is
                            // not terrain at all, so it is kept whatever it is above.
                            if seen.is_none() || tile == sim::Tile::Spike {
                                seen = Some(colour);
                            }
                            if tile == sim::Tile::Spike {
                                break;
                            }
                        }
                    }
                }
                // The first row of the block that has anything in it is the surface.
                if seen.is_some() {
                    break;
                }
            }
            out[row * columns + column] = seen;
        }
    }
    out
}

/// What a click on the shelf asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open this level in the editor.
    Open(PathBuf),
    /// Open it and start a playtest on it.
    Play(PathBuf),
    /// Note: nothing was clicked.
    None,
}

/// Draw the shelf and report what was clicked.
///
/// `current` is the path the window has open, drawn as the selected card so the shelf
/// answers "where am I?" as well as "what is there?".
pub fn show(ui: &mut Ui, entries: &[Entry], current: Option<&Path>) -> Action {
    let mut action = Action::None;
    if entries.is_empty() {
        ui.label(
            egui::RichText::new("no levels here yet — draw one and press Save")
                .color(theme::TEXT_DIM),
        );
        return action;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for entry in entries {
                let selected = current == Some(entry.path.as_path());
                let response = card(ui, entry, selected);
                if response.clicked() {
                    action = if entry.loadable() {
                        Action::Open(entry.path.clone())
                    } else {
                        Action::None
                    };
                }
                if response.double_clicked() && entry.loadable() {
                    action = Action::Play(entry.path.clone());
                }
                response.on_hover_text(if entry.loadable() {
                    format!(
                        "{}\n{}\nclick to open, double-click to playtest",
                        entry.path.display(),
                        entry.summary()
                    )
                } else {
                    format!("{}\n{}", entry.path.display(), entry.summary())
                });
            }
        });
    action
}

/// One card: the thumbnail, the name, the facts, and a Play button.
fn card(ui: &mut Ui, entry: &Entry, selected: bool) -> Response {
    let mut clicked = false;
    let frame = if selected {
        theme::card_frame().stroke(Stroke::new(1.0_f32, theme::ACCENT))
    } else {
        theme::card_frame()
    };
    let response = frame
        .show(ui, |ui| {
            ui.set_width(THUMB_WIDTH + 20.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&entry.name)
                            .color(if entry.loadable() {
                                theme::TEXT
                            } else {
                                theme::TEXT_FAINT
                            })
                            .monospace()
                            .strong(),
                    );
                    if selected {
                        ui.label(egui::RichText::new("open").small().color(theme::ACCENT));
                    }
                });
                let thumb = draw_thumbnail(ui, entry);
                if thumb.clicked() {
                    clicked = true;
                }
                ui.label(
                    egui::RichText::new(entry.summary())
                        .small()
                        .color(if entry.loadable() {
                            theme::TEXT_DIM
                        } else {
                            theme::BAD
                        }),
                );
                if !entry.materials.is_empty() {
                    ui.label(
                        egui::RichText::new(entry.materials.join(" · "))
                            .small()
                            .color(theme::TEXT_FAINT),
                    );
                }
                // The one line a card gets that is about the *run* rather than the level: this is
                // what says the pack and the training are two halves of one game (AUDIT.md V3).
                // The accent ink rather than a badge sprite, because the shelf draws no art — its
                // thumbnails are maps, not pictures — and a word is legible at this size.
                if entry.champion.is_some() {
                    ui.label(
                        egui::RichText::new("beaten — a champion is saved for this level")
                            .small()
                            .color(theme::GOOD_INK),
                    );
                }
            });
        })
        .response;
    if clicked {
        // A click anywhere on the card is a click on the card: the thumbnail is the biggest
        // thing on it and aiming at the 20-pixel label above it would be the sort of
        // precision a shelf exists to avoid.
        return ui.interact(
            response.rect,
            ui.id().with(("shelf-card", entry.name.as_str())),
            Sense::click(),
        );
    }
    response
}

/// The thumbnail itself: one filled rectangle per cell, on the sunken colour.
fn draw_thumbnail(ui: &mut Ui, entry: &Entry) -> Response {
    let (columns, rows) = entry.thumb_size;
    let height = if columns == 0 {
        THUMB_MAX_HEIGHT * 0.4
    } else {
        (THUMB_WIDTH * rows as f32 / columns as f32).clamp(24.0, THUMB_MAX_HEIGHT)
    };
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(THUMB_WIDTH, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), theme::SUNKEN);
    if columns > 0 && rows > 0 {
        let cw = rect.width() / columns as f32;
        let ch = rect.height() / rows as f32;
        for row in 0..rows {
            for column in 0..columns {
                let Some(colour) = entry.thumb[row * columns + column] else {
                    continue;
                };
                let cell = Rect::from_min_size(
                    egui::pos2(
                        rect.left() + column as f32 * cw,
                        rect.top() + row as f32 * ch,
                    ),
                    Vec2::new(cw + 0.5, ch + 0.5),
                );
                painter.rect_filled(cell, CornerRadius::ZERO, colour);
            }
        }
    } else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "unreadable",
            egui::FontId::monospace(11.0),
            theme::BAD,
        );
    }
    // The frame on top, so neighbouring cards do not read as one picture.
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS),
        Stroke::new(1.0_f32, theme::EDGE),
        egui::StrokeKind::Inside,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped(slug: &str) -> Level {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"));
        Level::from_json_file(&path).unwrap_or_else(|e| panic!("{slug}: {e}"))
    }

    fn levels_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../levels")
    }

    /// The shelf shows the pack, and every card is a level the window can actually open.
    ///
    /// This is the property the shelf exists for: the list of names in the Open menu and
    /// the list of files on disk have to be the same list, and every one of them has to
    /// load — a card that cannot be opened is a bug report, not a feature.
    #[test]
    fn the_shelf_lists_every_shipped_level_and_each_one_loads() {
        let entries = scan(&levels_dir(), None);
        assert!(
            entries.len() >= 6,
            "the pack has six levels, found {}",
            entries.len()
        );
        for entry in &entries {
            assert!(entry.loadable(), "{} did not load", entry.name);
            assert_eq!(entry.error, None);
            assert!(
                entry.has_spawn && entry.has_goal,
                "{} has no start or no flag",
                entry.name
            );
            assert!(entry.size.0 > 0 && entry.size.1 > 0);
            assert!(!entry.thumb.is_empty(), "{} has no thumbnail", entry.name);
        }
    }

    /// A card's facts are read off the level rather than guessed.
    #[test]
    fn a_card_reports_the_levels_own_materials_and_size() {
        let level = shipped("l1-flat-run");
        let entry = read_with(&levels_dir().join("l1-flat-run.json"), None);
        assert_eq!(entry.size, (level.width, level.height));
        assert!(
            entry.materials.contains(&"soil"),
            "the opening level is soil: {:?}",
            entry.materials
        );
        assert_eq!(entry.materials, materials_of(&level));

        // And a material the level really has shows up: the beach level is sand.
        let beach = read_with(&levels_dir().join("l4-tide-pools.json"), None);
        if beach.loadable() {
            assert!(
                beach.materials.contains(&"sand"),
                "a beach level that does not list sand: {:?}",
                beach.materials
            );
        }
    }

    /// A file that is not a level is a card with a reason, not a missing card.
    #[test]
    fn a_broken_level_is_a_card_that_says_why() {
        let dir = std::env::temp_dir().join("evo-shelf-broken");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("not-a-level.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        let entry = read_with(&path, None);
        assert!(!entry.loadable());
        assert!(entry.error.is_some());
        assert!(entry.summary().starts_with("will not load:"));
        assert!(entry.thumb.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The thumbnail is a map of the level: air stays air, and the floor is drawn.
    #[test]
    fn the_thumbnail_draws_the_floor_and_not_the_sky() {
        let level = shipped("l1-flat-run");
        let (columns, rows) = thumbnail_grid(level.width, level.height);
        let thumb = thumbnail(&level, columns, rows);
        assert_eq!(thumb.len(), columns * rows);
        let air = thumb.iter().filter(|c| c.is_none()).count();
        assert!(air > 0, "a flat run has sky above it");
        assert!(
            thumb.iter().any(|c| c.is_some()),
            "and ground under it: {thumb:?}"
        );
        // The bottom row is the level's floor, so it must be drawn across the width.
        let bottom = &thumb[(rows - 1) * columns..];
        assert!(
            bottom.iter().all(|c| c.is_some()),
            "the floor row has a hole in it: {bottom:?}"
        );
    }

    /// A big level is downsampled to a bounded grid rather than drawn tile for tile.
    #[test]
    fn a_long_level_is_downsampled_to_the_thumbnails_grid() {
        let level = shipped("l6-marathon");
        let (columns, rows) = thumbnail_grid(level.width, level.height);
        assert!(columns <= MAX_THUMB_CELLS, "{columns} columns");
        // The grid keeps the level's aspect ratio to within one cell per axis.
        let level_aspect = level.width as f32 / level.height as f32;
        let thumb_aspect = columns as f32 / rows as f32;
        assert!(
            (level_aspect - thumb_aspect).abs() < 1.5,
            "a {level_aspect:.1} level is drawn at {thumb_aspect:.1}"
        );
    }

    /// A card says when the level has been beaten, and says nothing when it has not.
    ///
    /// The bug this pins is the pack and the run having no visible connection: a champion saved
    /// from this very window changed nothing on the shelf, so a person could not see which levels
    /// had been solved and which were still open (AUDIT.md V3). The link is the *file name* a run
    /// writes its artifacts under, so the test writes one there and asks the shelf.
    #[test]
    fn a_card_reports_a_champion_saved_for_that_level() {
        let root = std::env::temp_dir().join(format!("evo-shelf-champs-{}", std::process::id()));
        let champions = root.join("champions");
        let levels = root.join("levels");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&champions).unwrap();
        std::fs::create_dir_all(&levels).unwrap();
        let level = shipped("l1-flat-run");
        level.to_json_file(&levels.join("l1-flat-run.json")).unwrap();

        // Nothing saved yet, and no champions directory at all: no card claims a win.
        assert!(scan(&levels, Some(&champions))[0].champion.is_none());
        assert!(scan(&levels, None)[0].champion.is_none());

        // The run's own naming — `{stem}.json` — beside the level's own file.
        std::fs::write(champions.join("l1-flat-run.json"), "{}").unwrap();
        let checked = scan(&levels, Some(&champions));
        let champion = checked[0].champion.as_ref().expect("the champion was found");
        assert!(
            champion.ends_with("l1-flat-run.json"),
            "{}",
            champion.display()
        );

        // Only that level: another level's champion is another level's, and a *directory* named
        // like a champion file is not one.
        std::fs::write(champions.join("l2-first-gap.json"), "{}").unwrap();
        assert!(scan(&levels, Some(&champions))[0].champion.is_some());
        std::fs::remove_file(champions.join("l1-flat-run.json")).unwrap();
        std::fs::create_dir(champions.join("l1-flat-run.json")).unwrap();
        assert!(
            scan(&levels, Some(&champions))[0].champion.is_none(),
            "a directory is not a champion"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A level with a hazard says so, and one without does not.
    #[test]
    fn the_materials_list_the_hazard() {
        let mut level = shipped("l1-flat-run");
        assert!(!materials_of(&level).contains(&"spikes"));
        level.set_tile(10, level.height - 2, sim::Tile::Spike);
        assert!(materials_of(&level).contains(&"spikes"));
    }
}
