//! The art atlas: `assets/tileset.png` plus the manifest that names its cells.
//!
//! The manifest is the contract, not the pixel coordinates. A sprite is looked up
//! by name, so a replacement tileset only has to list the names this crate asks for
//! — and [`Atlas::require`] fails loudly, listing what the file *does* contain,
//! rather than drawing a mystery tile.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::{ColorImage, Context, Pos2, Rect, TextureHandle, TextureOptions};

/// The names the renderer and the editor ask for. A tileset missing any of these
/// cannot draw a level, and [`Sprites::load`] says so at startup rather than at the
/// first frame that happens to need the sprite.
///
/// Four names are deliberately *not* here: the tile variants (`ground_1_b`,
/// `ground_1_c`, `deep_0_b`, `deep_0_c`). They are texture rather than structure, so a
/// replacement tileset that omits them draws the plain tile instead of refusing to
/// start.
///
/// The eight props from `flowers_white` on are optional for the same reason, and each
/// has a fallback the view falls back *to* rather than a name it panics on
/// (`crate::view::scenery_sprite`): a tileset drawn before the dressing grew still runs,
/// it is simply dressed with the props it has.
///
/// The material seams (`blend_*`) are optional the same way, and more so than the rest:
/// a tileset without them draws a hard edge between two materials, which is what every
/// tileset drew before they existed.
pub const REQUIRED: [&str; 81] = [
    "empty",
    // The ground: one sprite per 4-bit open-side mask, and the rock the same way for
    // every mask with no sky above it.
    "ground_0",
    "ground_1",
    "ground_2",
    "ground_3",
    "ground_4",
    "ground_5",
    "ground_6",
    "ground_7",
    "ground_8",
    "ground_9",
    "ground_10",
    "ground_11",
    "ground_12",
    "ground_13",
    "ground_14",
    "ground_15",
    "deep_0",
    "deep_2",
    "deep_4",
    "deep_6",
    "deep_8",
    "deep_10",
    "deep_12",
    "deep_14",
    "spike",
    // The two tiles a level can put *behaviour* in, rather than material: the pad a body is
    // thrown by. A brittle tile needs nothing required of its own — it is drawn as the material
    // it is made of plus one of the optional `cracked_*` overlays — but a pad is an object the
    // game has no fallback picture for, exactly as a spike is, so a tileset without it is refused
    // at startup rather than drawn as an empty cell a body bounces off for no visible reason.
    "spring",
    // The elements: the fluid bodies and their beds, the two animated surfaces each, the ball
    // a vent throws, and the vine. All required, because each of the three tiles has
    // *behaviour* the game cannot draw a fallback picture for — an invisible pool is a body
    // sinking for no visible reason, which is exactly the failure `spring`'s note describes.
    // The bed is required for a subtler reason: it is the sprite that lets the body be
    // seamless, so a tileset with the bodies and the surfaces but no beds would draw a pool's
    // depth cue nowhere at all rather than draw it twice.
    "water",
    "water_top_a",
    "water_top_b",
    "lava",
    "lava_top_a",
    "lava_top_b",
    "lava_ball",
    "vine",
    // The spiked ball is the same kind of required picture the lava ball is: a hazard whose art
    // is the hitbox the player has to see. A tileset without it would draw a body dying to
    // nothing, so the load is refused instead.
    "spike_ball",
    // The player's animation set: two idle frames, a six-frame run, one pose per
    // airborne state, and the pose a dead agent is left in.
    "player_idle_a",
    "player_idle_b",
    "player_run_1",
    "player_run_2",
    "player_run_3",
    "player_run_4",
    "player_run_5",
    "player_run_6",
    "player_jump",
    "player_fall",
    "player_land",
    "player_hurt",
    // The walker patrol, and what is left of it after a stomp.
    "walker_a",
    "walker_b",
    "walker_c",
    "walker_d",
    "walker_dead",
    // The goal: a pole, and the flag that waves on it.
    "goal_top",
    "goal_bottom",
    "goal_flag_1",
    "goal_flag_2",
    "goal_flag_3",
    "goal_flag_4",
    "spawn_marker",
    // Scenery, scattered along a level's surfaces by the view.
    "bush_a",
    "bush_b",
    "tuft_a",
    "tuft_b",
    "flowers",
    "flowers_pink",
    "rock",
    "pebble",
    "mushroom",
    "cloud_a",
    "cloud_b",
    "cloud_c",
    // The far tree-line: the rank standing on the ridge, and the hazier rank behind it.
    "pine_a",
    "pine_b",
    "pine_c",
    "pine_d",
    // Effects: a runner's dust, a death and a win.
    "dust_a",
    "dust_b",
    "puff",
    "spark_a",
    "spark_b",
];

/// Where a sprite sits in the atlas grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sprite {
    pub col: i32,
    pub row: i32,
}

/// A loaded tileset: the pixels on the GPU, plus the grid that describes them.
pub struct Atlas {
    texture: TextureHandle,
    cols: i32,
    rows: i32,
    sprites: BTreeMap<String, Sprite>,
    source: PathBuf,
}

impl Atlas {
    /// Load `tileset.json` and the PNG it names, relative to `assets_dir`.
    ///
    /// Errors are strings rather than panics because the failure a user will
    /// actually hit is "I pointed the game at a directory with no tileset in it".
    pub fn load(ctx: &Context, assets_dir: impl AsRef<Path>) -> Result<Atlas, String> {
        let dir = assets_dir.as_ref();
        let manifest_path = dir.join("tileset.json");
        let src = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?;
        let doc: serde_json::Value = serde_json::from_str(&src)
            .map_err(|e| format!("{} is not valid JSON: {e}", manifest_path.display()))?;

        if doc.get("format").and_then(|v| v.as_str()) != Some("evo-platformer-tileset") {
            return Err(format!(
                "{} is not a tileset manifest (format tag missing)",
                manifest_path.display()
            ));
        }
        let cell = doc
            .get("cell")
            .and_then(|v| v.as_i64())
            .filter(|c| *c > 0)
            .ok_or_else(|| format!("{} has no usable \"cell\" size", manifest_path.display()))?
            as i32;
        let cols = doc.get("cols").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let rows = doc.get("rows").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let image_name = doc
            .get("image")
            .and_then(|v| v.as_str())
            .unwrap_or("tileset.png");

        let mut sprites = BTreeMap::new();
        for entry in doc
            .get("sprites")
            .and_then(|v| v.as_array())
            .unwrap_or(&Vec::new())
        {
            let (Some(name), Some(col), Some(row)) = (
                entry.get("name").and_then(|v| v.as_str()),
                entry.get("col").and_then(|v| v.as_i64()),
                entry.get("row").and_then(|v| v.as_i64()),
            ) else {
                continue;
            };
            sprites.insert(
                name.to_string(),
                Sprite {
                    col: col as i32,
                    row: row as i32,
                },
            );
        }

        let image_path = dir.join(image_name);
        let bytes = std::fs::read(&image_path)
            .map_err(|e| format!("cannot read {}: {e}", image_path.display()))?;
        let decoded = image::load_from_memory(&bytes)
            .map_err(|e| format!("{} is not a readable image: {e}", image_path.display()))?;
        let rgba = decoded.to_rgba8();
        let (width, height) = (rgba.width() as i32, rgba.height() as i32);

        // The grid has to describe the pixels, or every sprite would be drawn from
        // the wrong place and the art would look like garbage rather than like a bug.
        if cols <= 0 || rows <= 0 {
            return Err(format!(
                "{} declares a {cols}x{rows} grid",
                manifest_path.display()
            ));
        }
        if width != cols * cell || height != rows * cell {
            return Err(format!(
                "{} is {width}x{height}px but the manifest describes a {cols}x{rows} \
                 grid of {cell}px cells ({}x{}px)",
                image_path.display(),
                cols * cell,
                rows * cell
            ));
        }

        let texture = ctx.load_texture(
            "tileset",
            ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            ),
            // Nearest neighbours: pixel art, scaled by the camera, must stay crisp.
            TextureOptions::NEAREST,
        );

        Ok(Atlas {
            texture,
            cols,
            rows,
            sprites,
            source: manifest_path,
        })
    }

    /// The sprite called `name`, or `None` if this tileset does not have it.
    pub fn sprite(&self, name: &str) -> Option<Sprite> {
        self.sprites.get(name).copied()
    }

    /// The sprite called `name`, or an error naming both what was wanted and what
    /// the loaded tileset offers.
    pub fn require(&self, name: &str) -> Result<Sprite, String> {
        self.sprite(name).ok_or_else(|| {
            format!(
                "{} has no sprite {name:?}; it offers: {}",
                self.source.display(),
                self.names().collect::<Vec<_>>().join(", ")
            )
        })
    }

    /// Every sprite name in the loaded tileset, in sorted order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.sprites.keys().map(|s| s.as_str())
    }

    /// Every name the renderer needs that this tileset is missing.
    pub fn missing_required(&self) -> Vec<&'static str> {
        REQUIRED
            .iter()
            .copied()
            .filter(|name| !self.sprites.contains_key(*name))
            .collect()
    }

    /// The atlas grid, in cells. The UV math is derived from it, so a test checks the
    /// derivation against the grid the file actually declares.
    #[cfg(test)]
    pub fn grid(&self) -> (i32, i32) {
        (self.cols, self.rows)
    }

    pub fn texture_id(&self) -> egui::TextureId {
        self.texture.id()
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    /// The atlas rectangle for a sprite, in UV (0..1) space.
    ///
    /// One cell is `1/cols` of the width — a *fraction of the image*, not of the
    /// grid. (Dividing the pixel size by the column count gives a number in the
    /// thousands-thousands of units and samples outside the texture entirely, which
    /// renders as nothing at all rather than as an obvious mistake. The unit-square
    /// test on every sprite in the shipped atlas is there because that happened.)
    pub fn uv(&self, name: &str) -> Result<Rect, String> {
        let sprite = self.require(name)?;
        // Both corners come from *dividing* a cell coordinate by the grid rather than from
        // multiplying it by a precomputed `1.0 / n`. The two agree to within a rounding error
        // everywhere except at the far edge, where the reciprocal form lands just outside the
        // texture — in f32, `14 * (1/15) + 1/15` is `1.0000001` — and the unit-square test refuses it,
        // which is the test doing its job: a sprite sampled a hair off the edge of the atlas is a
        // sprite drawing its neighbour's last column. Dividing puts the last cell's far edge at
        // exactly 1.0, where "the last cell ends where the image ends" is true by construction. It
        // went unnoticed for as long as the grid was one whose reciprocals happened to cancel.
        let min = Pos2::new(
            sprite.col as f32 / self.cols as f32,
            sprite.row as f32 / self.rows as f32,
        );
        let max = Pos2::new(
            (sprite.col + 1) as f32 / self.cols as f32,
            (sprite.row + 1) as f32 / self.rows as f32,
        );
        Ok(Rect::from_min_max(min, max))
    }
}

/// The window's own icon, cut out of the game's art.
///
/// A program with no icon wears the platform's default, so a row of this game's windows and a
/// row of every other program's looked the same on a taskbar — and the square beside a window's
/// name is the first thing most people ever see of it (AUDIT.md F5). The sprite is the
/// **runner**, the pose the title screen opens on, which is the one picture in the atlas that
/// says whose window this is.
///
/// Scaled by a whole number, nearest-neighbour, for the same reason the canvas camera rounds its
/// zoom to whole points ([`crate::view::Camera::scale`]): pixel art has exactly one correct
/// scale and it is an integer, and a smoothly-resampled 16-pixel sprite is a smudge at every
/// size. `scale` is asked for explicitly rather than picked here, because how big a taskbar's
/// icon slot is is the window's business.
///
/// Read from the PNG instead of taken from the loaded texture because the icon is wanted
/// *before* the window exists — `with_icon` is a viewport option and the atlas is loaded inside
/// the window's own creation callback — and one extra decode at startup is cheaper than holding
/// a second copy of the atlas for the life of the session. Every failure is a `None`: a missing
/// assets directory is a window without an icon, not a window that will not open, which is the
/// same bargain the splash's missing champion makes.
pub fn window_icon(
    assets_dir: impl AsRef<Path>,
    sprite: &str,
    scale: u32,
) -> Option<egui::IconData> {
    let dir = assets_dir.as_ref();
    let src = std::fs::read_to_string(dir.join("tileset.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&src).ok()?;
    let cell = doc.get("cell")?.as_u64()? as u32;
    let entry = doc
        .get("sprites")?
        .as_array()?
        .iter()
        .find(|e| e.get("name").and_then(|v| v.as_str()) == Some(sprite))?;
    let col = entry.get("col")?.as_u64()? as u32;
    let row = entry.get("row")?.as_u64()? as u32;
    let image_name = doc.get("image")?.as_str()?;
    let bytes = std::fs::read(dir.join(image_name)).ok()?;
    let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();

    let scale = scale.max(1);
    let size = cell * scale;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // Checked rather than indexed: a manifest whose sprite sits outside the image it
            // names is a broken tileset, and a broken tileset must not panic inside the window
            // builder before a window exists to report it in.
            let pixel = rgba.get_pixel_checked(col * cell + x / scale, row * cell + y / scale)?;
            out.extend_from_slice(&pixel.0);
        }
    }
    Some(egui::IconData {
        rgba: out,
        width: size,
        height: size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    /// The window's icon is a whole-number crop of the runner, and it is not blank.
    ///
    /// The bug this pins is a window with no icon at all: on a taskbar, every window of this game
    /// looked like every other program's (AUDIT.md F5). A test that only asked for the right *size*
    /// would pass on a blank square, which is the platform default with extra steps — so this asks
    /// for the runner's own colours as well, and for the sprite argument to actually choose the
    /// cell: `empty` is a transparent tile, and cropping it must produce nothing.
    #[test]
    fn the_window_icon_is_the_runner_at_a_whole_number_scale() {
        let icon = window_icon(assets_dir(), "player_idle_a", 4).expect("an icon from the atlas");
        let cell = 16u32;
        assert_eq!((icon.width, icon.height), (cell * 4, cell * 4));
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);

        let opaque = icon.rgba.chunks(4).filter(|p| p[3] > 0).count();
        assert!(opaque > 0, "a blank icon is not an icon");

        // The runner's own palette, as `art/common.py` names it: the dark ink of the outline and
        // the vermilion of the body. *Present* is the claim — the crop is of the sprite that was
        // asked for — and it is the one thing a size assertion cannot see.
        let has = |rgb: [u8; 3]| icon.rgba.chunks(4).any(|p| p[..3] == rgb && p[3] > 0);
        assert!(
            has([44, 24, 36]),
            "the runner's outline ink is not in the icon"
        );
        assert!(has([186, 74, 30]), "the runner's body is not in the icon");

        // And the name picks the cell: a transparent tile scales up to a transparent icon.
        let empty = window_icon(assets_dir(), "empty", 4).expect("an icon from the atlas");
        assert!(
            empty.rgba.chunks(4).all(|p| p[3] == 0),
            "the icon ignored which sprite it was given"
        );

        // A directory with no tileset in it is *no icon* rather than no window: the window opens
        // and the atlas load itself reports the real problem.
        let missing = std::env::temp_dir().join("evo-no-assets-at-all");
        assert!(window_icon(&missing, "player_idle_a", 4).is_none());
    }

    /// Every sprite must map inside the texture, sized to exactly one cell.
    ///
    /// The bug this pins: a UV rect computed from the *grid* instead of from the
    /// image put every sprite far outside 0..1, so a level drew nothing at all while
    /// the code that resolved sprites, chose animation frames and counted tiles looked
    /// perfectly healthy. Nothing else in the workspace can see that.
    #[test]
    fn every_shipped_sprite_maps_inside_the_image_at_one_cell() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let (cols, rows) = atlas.grid();
        // One cell, as a width and a height rather than as a vector: the far edge of the last cell
        // has to land on **exactly** 1.0, and a vector is a place where a rounding error can hide
        // behind `Debug` formatting that rounds to two places.
        let (expected_w, expected_h) = (1.0 / cols as f32, 1.0 / rows as f32);
        for name in atlas.names() {
            let uv = atlas.uv(name).expect("every listed sprite resolves");
            assert!(
                uv.min.x >= 0.0 && uv.min.y >= 0.0 && uv.max.x <= 1.0 && uv.max.y <= 1.0,
                "{name} maps outside the texture: {uv:?}"
            );
            assert!(
                (uv.width() - expected_w).abs() < 1e-6 && (uv.height() - expected_h).abs() < 1e-6,
                "{name} is not one cell ({expected_w} x {expected_h}): {uv:?}"
            );
        }
    }

    #[test]
    fn the_shipped_atlas_satisfies_every_name_the_renderer_asks_for() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        assert_eq!(atlas.missing_required(), Vec::<&str>::new());
    }

    /// Every brush in the editor's palette has an icon in the shipped atlas.
    ///
    /// The palette draws a brush with `atlas.uv(brush.sprite())` and simply skips a name it
    /// cannot resolve, so a brush pointing at a sprite that does not exist is a **blank
    /// button** and no error anywhere — which is what the ground brush had been, asking for
    /// a sprite called `grass` that no tileset in this project has ever had. A missing icon
    /// is exactly the kind of thing that is invisible until somebody paints with the wrong
    /// brush three times, so it is a test now rather than a convention.
    #[test]
    fn every_brush_has_an_icon_in_the_atlas() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        for brush in crate::editor::Brush::ALL {
            assert!(
                atlas.uv(brush.sprite()).is_ok(),
                "the {:?} brush would draw {:?}, which is not in the atlas",
                brush,
                brush.sprite()
            );
            // And the layer *under* the icon, for the brushes that have one: a cracked brush draws
            // its material first and its own transparent crack over it, so a base sprite the atlas
            // does not have is a crack drawn on a bare plate — the same blank-button failure, one
            // layer down, and invisible unless somebody knows what the cell should look like.
            if let Some(base) = brush.base_sprite() {
                assert!(
                    atlas.uv(base).is_ok(),
                    "the {:?} brush would draw {base:?} under its own icon, and the atlas does not \
                     have it",
                    brush
                );
            }
        }
    }

    /// Every glyph the window's own controls draw is in the shipped atlas.
    ///
    /// The same guard as `every_brush_has_an_icon_in_the_atlas`, for the other half of the window:
    /// `ui::icon_button` falls back to drawing a control's *name* when its sprite is missing, so a
    /// glyph renamed in the art and not in the app is a tool bar of small words — no error, no blank,
    /// nothing anyone could call a bug, and exactly the drift that left the ground brush pointing at a
    /// sprite called `grass` for months. The `ui_*` glyphs are deliberately **not** in [`REQUIRED`]
    /// (a tileset drawn before them still runs), which is precisely why nothing else would notice.
    #[test]
    fn every_glyph_the_window_draws_is_in_the_atlas() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        for name in crate::ui::GLYPHS {
            assert!(
                atlas.uv(name).is_ok(),
                "the window draws {name:?}, which the atlas does not have — so every plate that \
                 asks for it would quietly draw its own name instead"
            );
        }
    }

    #[test]
    fn a_missing_tileset_is_an_error_naming_the_path() {
        let ctx = egui::Context::default();
        let err = match Atlas::load(&ctx, assets_dir().join("nope")) {
            Ok(_) => panic!("a missing tileset must be an error"),
            Err(err) => err,
        };
        assert!(err.contains("tileset.json"), "{err}");
    }

    #[test]
    fn a_manifest_that_disagrees_with_the_image_is_refused() {
        // The grid in the manifest is what the UVs are computed from, so a mismatch
        // between it and the pixels must fail loudly rather than draw confetti.
        let dir = std::env::temp_dir().join(format!("evo-atlas-mismatch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let src = assets_dir();
        std::fs::copy(src.join("tileset.png"), dir.join("tileset.png")).expect("copy png");
        let mut doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(src.join("tileset.json")).unwrap())
                .expect("manifest json");
        doc["cols"] = serde_json::json!(9);
        std::fs::write(
            dir.join("tileset.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .expect("write manifest");

        let ctx = egui::Context::default();
        let err = match Atlas::load(&ctx, &dir) {
            Ok(_) => panic!("a 9-column grid cannot describe a 256px atlas"),
            Err(err) => err,
        };
        assert!(err.contains("grid"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The assets README is the manifest in words, and it has been wrong before: it still
    /// described an 8x3 atlas of `grass`/`dirt`/`player_stand` long after the tileset had
    /// grown to seventy sprites, because prose about a generated file is prose nothing
    /// checks. The names are the part worth pinning — every sprite the manifest ships has
    /// to be named in `assets/README.md`.
    #[test]
    fn the_assets_readme_lists_every_shipped_sprite() {
        let readme = std::fs::read_to_string(assets_dir().join("README.md"))
            .expect("the assets README is part of the repository");
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        // Backticks on both sides, so `ground_1` cannot be "found" inside `ground_1_b`.
        let missing: Vec<String> = atlas
            .names()
            .filter(|name| !readme.contains(&format!("`{name}`")))
            .map(|name| name.to_string())
            .collect();
        assert!(
            missing.is_empty(),
            "assets/README.md does not name {} shipped sprite(s): {missing:?}",
            missing.len()
        );
    }

    /// The shipped PNG, pixel by pixel, with the manifest's own coordinates.
    ///
    /// Every test below is about what the *art* looks like rather than about what the
    /// code says, so they read the actual pixels. The manifest is loaded through
    /// [`Atlas::load`], which means these also fail if the manifest stops describing the
    /// image.
    struct Pixels {
        atlas: Atlas,
        rgba: Vec<[u8; 4]>,
        width: i32,
        cell: i32,
    }

    impl Pixels {
        fn load() -> Pixels {
            let dir = assets_dir();
            let ctx = egui::Context::default();
            let atlas = Atlas::load(&ctx, &dir).expect("the shipped tileset loads");
            let (cols, _rows) = atlas.grid();
            let cell = {
                let doc: serde_json::Value = serde_json::from_str(
                    &std::fs::read_to_string(dir.join("tileset.json")).expect("manifest"),
                )
                .expect("manifest json");
                doc["cell"].as_i64().expect("cell size") as i32
            };
            let image = image::load_from_memory(
                &std::fs::read(dir.join("tileset.png")).expect("the shipped image"),
            )
            .expect("readable png")
            .to_rgba8();
            let width = cols * cell;
            assert_eq!(image.width() as i32, width, "the grid describes the image");
            Pixels {
                atlas,
                rgba: image.pixels().map(|p| p.0).collect(),
                width,
                cell,
            }
        }

        /// One sprite's pixels, row-major from its top-left corner.
        fn sprite(&self, name: &str) -> Sprite2 {
            let sprite = self.atlas.require(name).unwrap_or_else(|e| panic!("{e}"));
            let mut px = Vec::with_capacity((self.cell * self.cell) as usize);
            for y in 0..self.cell {
                for x in 0..self.cell {
                    let px_x = sprite.col * self.cell + x;
                    let px_y = sprite.row * self.cell + y;
                    px.push(self.rgba[(px_y * self.width + px_x) as usize]);
                }
            }
            Sprite2 {
                px,
                cell: self.cell,
            }
        }

        fn names(&self) -> Vec<String> {
            self.atlas.names().map(|n| n.to_string()).collect()
        }
    }

    /// A decoded cell, with the few questions the art tests ask of a sprite.
    /// The eight neighbours of a pixel, for the art rules that ask about a pixel's
    /// surroundings rather than about the pixel itself.
    const NEIGHBOURS: [(i32, i32); 8] = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ];

    struct Sprite2 {
        px: Vec<[u8; 4]>,
        cell: i32,
    }

    impl Sprite2 {
        fn at(&self, x: i32, y: i32) -> [u8; 4] {
            self.px[(y * self.cell + x) as usize]
        }

        /// Every pixel, as colours, in row-major order.
        fn colours(&self) -> Vec<[u8; 4]> {
            self.px.clone()
        }

        /// Is this pixel part of the figure's edge — opaque, with a transparent
        /// neighbour inside the cell?
        ///
        /// Which is the same question the art's outline pass asks
        /// (`characters._bump`), so the two agree by construction rather than by
        /// coincidence.
        fn on_the_edge(&self, x: i32, y: i32) -> bool {
            if self.at(x, y)[3] == 0 {
                return false;
            }
            NEIGHBOURS.iter().any(|(dx, dy)| {
                let (nx, ny) = (x + dx, y + dy);
                nx >= 0 && ny >= 0 && nx < self.cell && ny < self.cell && self.at(nx, ny)[3] == 0
            })
        }

        /// The colours along the figure's edge, one per edge pixel.
        fn edge_colours(&self) -> Vec<[u8; 4]> {
            let mut out = Vec::new();
            for y in 0..self.cell {
                for x in 0..self.cell {
                    if self.on_the_edge(x, y) {
                        out.push(self.at(x, y));
                    }
                }
            }
            out
        }

        /// The colour the figure is mostly made of *inside* its edge: its body.
        ///
        /// The modal interior colour, so "the player's colour" is a fact about the
        /// shipped pixels rather than a number copied out of the art module. The edge is
        /// excluded on purpose: an outline is by definition the most common colour along
        /// a silhouette, and asking about the body means asking about what the rim
        /// surrounds.
        fn modal_interior_colour(&self) -> [u8; 4] {
            let mut counts: Vec<([u8; 4], usize)> = Vec::new();
            for y in 0..self.cell {
                for x in 0..self.cell {
                    if self.at(x, y)[3] == 0 || self.on_the_edge(x, y) {
                        continue;
                    }
                    let pixel = self.at(x, y);
                    match counts.iter_mut().find(|(colour, _)| *colour == pixel) {
                        Some((_, n)) => *n += 1,
                        None => counts.push((pixel, 1)),
                    }
                }
            }
            counts
                .into_iter()
                .fold(None, |best: Option<([u8; 4], usize)>, entry| match best {
                    Some(best) if best.1 >= entry.1 => Some(best),
                    _ => Some(entry),
                })
                .expect("a character has an inside")
                .0
        }

        fn opaque_rows(&self) -> Vec<i32> {
            (0..self.cell)
                .filter(|y| (0..self.cell).any(|x| self.at(x, *y)[3] > 0))
                .collect()
        }

        fn fully_opaque(&self) -> bool {
            self.px.iter().all(|p| p[3] == 255)
        }

        fn has_transparency(&self) -> bool {
            self.px.iter().any(|p| p[3] == 0)
        }

        /// The rows of the cell, as sets of colours, for the band checks.
        fn row_colours(&self, y: i32) -> Vec<[u8; 4]> {
            (0..self.cell).map(|x| self.at(x, y)).collect()
        }
    }

    /// A hole in the world: a solid tile is never allowed to be transparent, because the
    /// sky behind it would read as a gap in the terrain an agent could fall through.
    #[test]
    fn every_solid_tile_is_opaque() {
        let pixels = Pixels::load();
        let mut tiles = 0;
        for name in pixels.names() {
            if ![
                "ground_",
                "deep_",
                "sand_",
                "sandstone_",
                "brick_",
                "masonry_",
                "wood_",
                "timber_",
            ]
            .iter()
            .any(|family| name.starts_with(family))
            {
                continue;
            }
            tiles += 1;
            let sprite = pixels.sprite(&name);
            assert!(
                sprite.fully_opaque(),
                "{name} has transparent pixels: {} of 256",
                sprite.px.iter().filter(|p| p[3] != 255).count()
            );
        }
        assert_eq!(
            tiles, 106,
            "four materials of 16 masks and 8 buried masks each, and ten decorated variants"
        );
    }

    /// The sand material is complete, and its crown is one flat colour like the grass one.
    ///
    /// A beach mixed into a level's floor has to meet the lawn beside it: the surface line
    /// of a level is the sum of its tiles' top rows, so a sand tile whose crown started a
    /// row lower would put a step in a flat floor exactly as surely as a grass one would.
    #[test]
    fn the_sand_material_has_a_crown_at_the_same_height_as_the_grass() {
        let pixels = Pixels::load();
        let mut surfaced = 0;
        let mut crown: Option<[u8; 4]> = None;
        for mask in (0..16).step_by(2).map(|m| m | 1) {
            let name = format!("sand_{mask}");
            assert!(
                pixels.names().contains(&name),
                "the sand family has no sprite for mask {mask}, which has sky above it"
            );
            let sprite = pixels.sprite(&name);
            // Columns 1..14: the two outer columns are the tile's own edge shading, which
            // is darker on one side and is not part of the surface band.
            let band: Vec<[u8; 4]> = (1..15).map(|x| sprite.at(x, 0)).collect();
            let first = band[0];
            assert!(
                band.iter().all(|c| *c == first),
                "{name}'s top row is not a flat band: {band:?}"
            );
            match crown {
                None => crown = Some(first),
                Some(crown) => assert_eq!(first, crown, "{name}'s sand is a different colour"),
            }
            surfaced += 1;
        }
        assert_eq!(surfaced, 8, "eight masks have sky over them");

        // And the buried half exists for the same eight masks rock does, in sandstone: a
        // dune three tiles deep is packed sand, not somebody else's grey rock.
        for mask in (0..16).filter(|m| m & 1 == 0) {
            let name = format!("sandstone_{mask}");
            assert!(
                pixels.names().contains(&name),
                "the sandstone family has no sprite for mask {mask}"
            );
        }
    }

    /// The two halves of a tree meet: the base's top row and the top's bottom row are the
    /// same row of the same drawing.
    ///
    /// The one thing a two-cell prop can get wrong that a one-cell prop cannot. The sprite
    /// is drawn whole and cut in half, so this holds by construction — which is exactly why
    /// it is worth asserting, because the next tree might not be drawn that way. A seam that
    /// does not line up is a stripe of sky through the trunk of every tree in the game.
    #[test]
    fn the_two_halves_of_a_tree_meet_at_the_seam() {
        let pixels = Pixels::load();
        for shape in ["a", "b"] {
            let base = pixels.sprite(&format!("tree_{shape}_base"));
            let top = pixels.sprite(&format!("tree_{shape}_top"));
            // Every column the canopy reaches in the upper half's last row is canopy in
            // the lower half's first row too. Not equality: a canopy tapers *upwards*, so
            // the lower half is allowed to be wider at the join — what it may not be is
            // narrower, which is a hole a pixel tall through the tree at every boundary.
            let seam = |sprite: &Sprite2, row: i32| -> Vec<bool> {
                (0..sprite.cell)
                    .map(|x| sprite.at(x, row)[3] > 0)
                    .collect::<Vec<_>>()
            };
            let (above, below) = (seam(&top, base.cell - 1), seam(&base, 0));
            let holes: Vec<i32> = (0..base.cell)
                .filter(|x| above[*x as usize] && !below[*x as usize])
                .collect();
            assert!(
                holes.is_empty(),
                "tree_{shape}'s halves do not meet: the columns {holes:?} are canopy in the \
                 upper half's last row and sky in the lower half's first"
            );
            // And the trunk reaches the ground: a tree that stopped a pixel short of its
            // cell's floor is a tree standing on air.
            let lowest = *base.opaque_rows().last().expect("a tree has pixels");
            assert!(
                lowest >= base.cell - 1,
                "tree_{shape}_base reaches row {lowest} only: it floats above its tile"
            );
        }
    }

    /// A butterfly is the one sprite in the game that is not standing on anything.
    ///
    /// Every other prop's art reaches its cell's last row, because the view plants it on
    /// the surface tile below and a prop that stopped short would stand a pixel of air
    /// above the ground ([`props_stand_on_the_ground_and_clouds_float`]). A butterfly
    /// drifts in the *middle* of its tile, so the rule is inverted here on purpose, and
    /// what it pins is that the creature is drawn around the centre of its cell: pinned to
    /// row 15 it would be a moth on the ground.
    #[test]
    fn butterflies_are_drawn_in_the_air_not_on_the_ground() {
        let pixels = Pixels::load();
        for name in [
            "butterfly_a_1",
            "butterfly_a_2",
            "butterfly_b_1",
            "butterfly_b_2",
        ] {
            let sprite = pixels.sprite(name);
            let rows = sprite.opaque_rows();
            let (top, bottom) = (rows[0], *rows.last().expect("a butterfly has pixels"));
            assert!(
                bottom <= sprite.cell - 3,
                "{name} reaches row {bottom}: a butterfly is not on the ground"
            );
            assert!(
                top >= 3,
                "{name} starts at row {top}: it should sit in the middle of its cell"
            );
            assert!(
                !sprite.fully_opaque(),
                "{name} fills its whole cell, which is a tile and not a creature"
            );
        }
    }

    /// The crown lives at a fixed height and is one colour across every tile that has sky
    /// above it. That is what makes a floor read as one continuous surface instead of a
    /// row of separate blocks — the property the whole autotiling scheme exists for.
    #[test]
    fn the_crown_is_a_flat_band_of_one_colour() {
        let pixels = Pixels::load();
        let mut crown: Option<[u8; 4]> = None;
        let mut surfaced = 0;
        for mask in 0..16 {
            if mask & 0b0001 == 0 {
                continue;
            }
            surfaced += 1;
            let name = format!("ground_{mask}");
            let sprite = pixels.sprite(&name);
            let row = sprite.row_colours(0);
            // The columns inside the side edges: a tile whose side is open shades that
            // one column, and that is the corner of the silhouette rather than the band.
            let band = &row[1..15];
            let first = band[0];
            assert!(
                band.iter().all(|c| *c == first),
                "{name}'s crown is not a flat band: {band:?}"
            );
            match crown {
                None => crown = Some(first),
                Some(crown) => assert_eq!(
                    first, crown,
                    "{name}'s crown is a different colour from the other tiles'"
                ),
            }
        }
        assert_eq!(surfaced, 8, "eight masks have sky over them");
        let crown = crown.expect("some tile has a crown");

        // And a tile with no sky above it has no crown at all: buried soil is dirt, and a
        // wall beside a pit is rock or dirt, all the way to its top row.
        for mask in (0..16).step_by(2) {
            let name = format!("ground_{mask}");
            let sprite = pixels.sprite(&name);
            assert!(
                !sprite.colours().contains(&crown),
                "{name} is underground but is painted as grass"
            );
        }
    }

    /// The decorations a level repeats most are the plain tile *plus detail*: a variant
    /// may put flowers, a pebble or a crack on a tile, and it may not move the tile it
    /// varies. The surface line of a whole level is the sum of its tiles' top rows, so a
    /// variant that shifted one would put a step in a flat floor.
    #[test]
    fn a_variant_is_its_base_tile_plus_detail() {
        let pixels = Pixels::load();

        // The soil variants decorate the crown and leave the body under it alone: rows
        // four and below are the dirt a stacked tile sits in.
        for variant in ["ground_1_b", "ground_1_c"] {
            let base = pixels.sprite("ground_1");
            let sprite = pixels.sprite(variant);
            assert_ne!(
                base.colours(),
                sprite.colours(),
                "{variant} is an exact copy of ground_1, which is dead art"
            );
            for y in 4..sprite.cell {
                assert_eq!(
                    base.row_colours(y),
                    sprite.row_colours(y),
                    "{variant} changes row {y}, which is not its own to change"
                );
            }
        }

        // The sand variants are crown variants like the soil ones, and their crown is
        // four rows rather than five for the same reason: row 4 is the ragged boundary
        // between the beach and the body under it, which is a silhouette and not a colour.
        for variant in ["sand_1_b", "sand_1_c"] {
            let base = pixels.sprite("sand_1");
            let sprite = pixels.sprite(variant);
            assert_ne!(
                base.colours(),
                sprite.colours(),
                "{variant} is an exact copy of sand_1, which is dead art"
            );
            for y in 4..sprite.cell {
                assert_eq!(
                    base.row_colours(y),
                    sprite.row_colours(y),
                    "{variant} changes row {y}, which is not its own to change"
                );
            }
        }

        // The rock variants have no crown to decorate, so they crack the body instead —
        // and the row that touches the sky and the row that meets the shadow below it are
        // what they must not move.
        for variant in ["deep_0_b", "deep_0_c"] {
            let base = pixels.sprite("deep_0");
            let sprite = pixels.sprite(variant);
            assert_ne!(
                base.colours(),
                sprite.colours(),
                "{variant} is an exact copy of deep_0, which is dead art"
            );
            assert_eq!(
                base.row_colours(0),
                sprite.row_colours(0),
                "{variant} moves its top row"
            );
            assert_eq!(
                base.row_colours(sprite.cell - 1),
                sprite.row_colours(sprite.cell - 1),
                "{variant} moves its bottom row"
            );
        }
    }

    /// Rock is never the surface, so no rock tile may contain the crown's colour.
    #[test]
    fn rock_is_never_painted_as_grass() {
        let pixels = Pixels::load();
        let crown = pixels.sprite("ground_1").row_colours(0)[7];
        for mask in (0..16).step_by(2) {
            let name = format!("deep_{mask}");
            let sprite = pixels.sprite(&name);
            assert!(!sprite.colours().contains(&crown), "{name} has grass on it");
        }
    }

    /// A hazard has to look like a hazard: the spike's top row is left transparent so a
    /// spike tile can never appear to be part of the ceiling it hangs under.
    #[test]
    fn the_spikes_leave_the_ceiling_open() {
        let pixels = Pixels::load();
        let spike = pixels.sprite("spike");
        assert!(
            (0..spike.cell).all(|x| spike.at(x, 0)[3] == 0),
            "the spike's top row should be empty"
        );
        assert!(
            spike.opaque_rows().len() > 8,
            "and the spike itself should fill the rest of the cell"
        );
    }

    /// **The pad is drawn over exactly the band the sim leaves unlaunchable.**
    ///
    /// `physics::SPRING_TOP_CLEARANCE` is four of sixteen cell rows, and the sprite has to leave
    /// those four empty and start drawing on the fifth — the drawing half of one number, checked
    /// against the arithmetic half rather than against a copied-in 4. The spike taught this lesson
    /// expensively (REVIEW.md §2.12): its kill box was the whole tile while its art stopped three
    /// rows short, and the invisible band that mismatch left above every spike was lethal. The pad's
    /// version of the same defect is the opposite way round and just as wrong: a body lands on the
    /// pad it can see and is not thrown by it, because the launch zone starts *below* the drawing.
    #[test]
    fn the_spring_leaves_the_band_the_sim_will_not_launch_in_empty() {
        let pixels = Pixels::load();
        let spring = pixels.sprite("spring");
        let clearance = (sim::physics::SPRING_TOP_CLEARANCE * spring.cell as f32) as i32;
        assert_eq!(
            clearance, 4,
            "the sim's clearance is four of sixteen rows; if that constant moved, so must the art"
        );
        for y in 0..clearance {
            assert!(
                (0..spring.cell).all(|x| spring.at(x, y)[3] == 0),
                "the pad draws in row {y}, which is inside the band a landing body is not \
                 launched in — the art and the launch zone have drifted"
            );
        }
        assert_eq!(
            spring.opaque_rows().first().copied(),
            Some(clearance),
            "the pad's first drawn row is where the launch zone starts"
        );
        assert!(
            spring.opaque_rows().len() >= 8,
            "and the pad itself should fill the rest of the cell"
        );
    }

    /// A crack is an **overlay**: ink and transparency, drawn over a material's own autotile, and
    /// one per material rather than one picture reused four times.
    ///
    /// Both the renderer and the editor's palette depend on that (`view::cracked_overlay` lays it
    /// over the tile, `Brush::base_sprite` puts the material under the palette's cell). The failure
    /// it guards is a crack drawn as a *whole tile*: every brittle tile in every level would lose
    /// its crown, its edges and its seams with the ground beside it — a floor of cracked tiles would
    /// read as a row of loose blocks, which is exactly the reason the cracked tile is a variant plus
    /// a mark rather than a fifth set of sixteen autotiles.
    #[test]
    fn a_crack_is_a_mark_on_its_material_and_not_a_tile() {
        let pixels = Pixels::load();
        let names = [
            "cracked_ground",
            "cracked_sand",
            "cracked_brick",
            "cracked_wood",
        ];
        let mut art = Vec::new();
        for name in names {
            let crack = pixels.sprite(name);
            let drawn = crack.px.iter().filter(|p| p[3] > 0).count();
            assert!(
                drawn > 8,
                "{name} draws {drawn} pixels, which is not a crack"
            );
            assert!(
                drawn < 64,
                "{name} draws {drawn} of {} pixels: a mark that thick is the tile, not a crack \
                 drawn on one",
                crack.px.len()
            );
            assert!(
                crack.has_transparency(),
                "{name} has no transparency: it would cover the material instead of marking it"
            );
            art.push(crack.colours());
        }
        for (i, one) in art.iter().enumerate() {
            for other in &art[i + 1..] {
                assert_ne!(
                    one, other,
                    "two of the four cracks are the same picture: a crack is its material's own \
                     darkest tone, not one generic scratch"
                );
            }
        }
    }

    /// A character stands on the bottom of its cell and is not a filled square: that is
    /// what puts its feet on the tile the physics says it is standing on.
    #[test]
    fn every_character_stands_on_its_own_floor() {
        let pixels = Pixels::load();
        // On its feet: the sprite reaches the bottom row of its cell, which is where the
        // physics box's floor is.
        for name in [
            "player_idle_a",
            "player_idle_b",
            "player_run_1",
            "player_run_4",
            "player_land",
            "player_hurt",
            "walker_a",
            "walker_c",
            "walker_dead",
        ] {
            let sprite = pixels.sprite(name);
            let lowest = *sprite.opaque_rows().last().expect("a character has pixels");
            assert!(
                lowest >= sprite.cell - 2,
                "{name}'s lowest pixel is row {lowest}: it is floating"
            );
            assert!(
                sprite.has_transparency(),
                "{name} is a filled square, not a character"
            );
        }

        // Airborne, and allowed to tuck its legs up — a jump pose that still touched the
        // floor would read as standing rather than jumping. Both are still in the lower
        // half of the cell, so the agent does not appear to fly.
        for name in ["player_jump", "player_fall"] {
            let sprite = pixels.sprite(name);
            let lowest = *sprite.opaque_rows().last().expect("a character has pixels");
            assert!(
                lowest >= sprite.cell / 2,
                "{name} reaches row {lowest}, which is not the lower half"
            );
            assert!(
                sprite.has_transparency(),
                "{name} is a filled square, not a character"
            );
        }
    }

    /// The sprite that is on screen in *every* mode has to be visible against the
    /// backdrop it is drawn on.
    ///
    /// The character this replaced was a cyan runner on a blue sky — a body a few shades
    /// off the thing behind it, which is a colour scheme that reads as "I cannot find my
    /// own player". The two halves of the assertion come from the two files that have to
    /// agree about it: the body colour is the modal colour of the shipped PNG, and the
    /// backdrop is the renderer's own list,
    /// asked for rather than copied, so neither side can drift without this failing. (The
    /// old palette measures 3 176 against the sky's top: comfortably inside the 14 400
    /// this refuses.)
    #[test]
    fn the_player_is_not_the_colour_of_the_sky_it_runs_in_front_of() {
        const FAR_APART: i64 = 120 * 120;
        let pixels = Pixels::load();
        let backdrop = crate::view::backdrop_colours();
        for name in [
            "player_idle_a",
            "player_idle_b",
            "player_run_1",
            "player_run_2",
            "player_run_3",
            "player_run_4",
            "player_run_5",
            "player_run_6",
            "player_jump",
            "player_fall",
            "player_land",
            "player_hurt",
        ] {
            let body = pixels.sprite(name).modal_interior_colour();
            for colour in backdrop {
                let dx = i64::from(body[0]) - i64::from(colour.r());
                let dy = i64::from(body[1]) - i64::from(colour.g());
                let dz = i64::from(body[2]) - i64::from(colour.b());
                let distance = dx * dx + dy * dy + dz * dz;
                assert!(
                    distance >= FAR_APART,
                    "{name}'s body {body:?} is only {distance} from the backdrop colour \
                     {:?}: the character is camouflaged",
                    colour.to_array()
                );
            }
        }

        // And the specific mistake the assertion is for, kept so that the check cannot
        // quietly stop being able to see it: the palette this replaced, against the
        // closest colour in the sky it used to run in front of.
        let replaced = [92u8, 216, 234];
        let closest = backdrop
            .iter()
            .map(|colour| {
                let dx = i64::from(replaced[0]) - i64::from(colour.r());
                let dy = i64::from(replaced[1]) - i64::from(colour.g());
                let dz = i64::from(replaced[2]) - i64::from(colour.b());
                dx * dx + dy * dy + dz * dz
            })
            .min()
            .expect("a backdrop has colours");
        assert!(
            closest < FAR_APART,
            "the cyan runner this replaced is {closest} from the sky, which this test \
             would now accept: the rule has stopped being able to see the bug"
        );
    }

    /// Every player frame wears one dark rim, and it is dark *because* it is the rim.
    ///
    /// This is the pixel-level reading of the art's own rule (`characters._bump` wraps the
    /// finished silhouette in one pixel of outline), and it is what keeps the character
    /// separable from any backdrop rather than only from the ones its hue happens to suit:
    /// the edge of the figure is its own colour, not a shade of whatever is behind it. The
    /// three claims are that there *is* an edge, that it is one colour rather than a
    /// ragged fringe, and that it reads as an edge — the rim of a figure is the darkest
    /// part of it. The cell's own boundary is exempt by definition, which is why the feet
    /// can still stand on row 15.
    #[test]
    fn every_player_frame_wears_one_dark_rim() {
        fn luminance(colour: [u8; 4]) -> f32 {
            0.2126 * f32::from(colour[0])
                + 0.7152 * f32::from(colour[1])
                + 0.0722 * f32::from(colour[2])
        }

        let pixels = Pixels::load();
        for name in [
            "player_idle_a",
            "player_idle_b",
            "player_run_1",
            "player_run_2",
            "player_run_3",
            "player_run_4",
            "player_run_5",
            "player_run_6",
            "player_jump",
            "player_fall",
            "player_land",
            "player_hurt",
        ] {
            let sprite = pixels.sprite(name);
            let edge = sprite.edge_colours();
            assert!(
                edge.len() > 20,
                "{name} has an edge of {} pixels: it is barely a character",
                edge.len()
            );
            let rim = edge[0];
            assert!(
                edge.iter().all(|colour| *colour == rim),
                "{name}'s edge is more than one colour: {:?}",
                edge.iter().filter(|c| **c != rim).collect::<Vec<_>>()
            );
            let body = sprite.modal_interior_colour();
            assert!(
                luminance(rim) * 2.0 < luminance(body),
                "{name}'s rim {rim:?} is not darker than its body {body:?}: an edge has to \
                 read as an edge"
            );
        }
    }

    /// Scenery sits on the ground it is drawn on, and a cloud does not: the two have
    /// different anchors, and a bush drawn floating would undo the point of the terrain.
    ///
    /// The view leans on this: a surface prop is drawn in the *open* tile above the ground
    /// (`view::Prop`), so a prop whose art stopped at row 14 would stand one pixel of air
    /// above the tile it belongs to, on every tile of every level.
    #[test]
    fn props_stand_on_the_ground_and_clouds_float() {
        let pixels = Pixels::load();
        for name in [
            "bush_a",
            "bush_c",
            "bush_e",
            "tuft_a",
            "tuft_c",
            "tuft_e",
            "flowers",
            "flowers_white",
            "rock",
            "pebble",
            "mushroom",
            "mushroom_pair",
            "lantern",
            "crystal",
            "signpost",
            "crate",
            "banner_a",
            "banner_b",
            "log",
            "stump",
            "reeds_a",
            "reeds_b",
            // Both halves of both trees: the base has to reach the cell's floor, and the
            // top has to reach the *bottom* of its own cell, which is the seam it shares
            // with the base. A tree whose upper half stopped short is a tree cut in two.
            "tree_a_base",
            "tree_a_top",
            "tree_b_base",
            "tree_b_top",
            // The tree-line is planted on the sky's ridge rather than on a level's floor,
            // and the anchor is the same one: a pine whose trunk stopped short would float
            // a pixel above whatever it is standing on, wherever that is, and all three
            // ranks stand on something.
            "pine_a",
            "pine_b",
            "pine_c",
            "pine_d",
            "pine_near_a",
            "pine_near_b",
        ] {
            let sprite = pixels.sprite(name);
            let lowest = *sprite.opaque_rows().last().expect("a prop has pixels");
            assert!(
                lowest >= sprite.cell - 1,
                "{name}'s base is row {lowest}, so it floats above its tile"
            );
        }
        for name in ["cloud_a", "cloud_b", "cloud_c"] {
            let sprite = pixels.sprite(name);
            let lowest = *sprite.opaque_rows().last().expect("a cloud has pixels");
            assert!(
                lowest <= sprite.cell - 4,
                "{name} reaches row {lowest}: a cloud should not sit on the ground"
            );
        }
        // The dust anchor, which the view relies on when it puts a puff under a runner.
        for name in ["dust_a", "dust_b"] {
            let sprite = pixels.sprite(name);
            let lowest = *sprite.opaque_rows().last().expect("dust has pixels");
            assert!(
                lowest >= sprite.cell - 2,
                "{name} is not at the cell's floor"
            );
        }
    }

    /// A near pine's trunk is faded bark-brown, not a shade of its own foliage,
    /// and not full-strength foreground bark either.
    ///
    /// The far ranks are silhouettes and their trunks are foliage tones on
    /// purpose — three rows of haze cannot be anything else. The near rank
    /// stands up to two tiles tall on the near hills, its trunk is a visible
    /// piece of the picture, and the first version of it drew that trunk in
    /// the foliage's darkest green: a green pole with a tree on it, in the
    /// one rank where a trunk can be seen. The second version swung to the
    /// other fault and used the foreground trees' `BARK` at full strength,
    /// which stood out of the hazed backdrop with the rest of that palette.
    /// So the shipped pixels are asked, not the palette: the two centre
    /// columns at the cell's floor — the part of the trunk below the lowest
    /// skirt, which is the part that shows — are `BARK` faded towards its
    /// own grey (108, 88, 72: red over green over blue, at about half the
    /// saturation), because a bark seen a hillside away is a muted bark, not
    /// a green one and not a near one.
    #[test]
    fn the_near_pines_have_brown_trunks() {
        const FADED_BARK: [u8; 4] = [108, 88, 72, 255];
        let pixels = Pixels::load();
        for name in ["pine_near_a", "pine_near_b"] {
            let sprite = pixels.sprite(name);
            for y in [14, 15] {
                for x in [7, 8] {
                    let pixel = sprite.at(x, y);
                    assert_eq!(
                        pixel, FADED_BARK,
                        "{name}'s trunk at ({x}, {y}) is {pixel:?}, not faded bark: a \
                         near pine's trunk is the part of it that is not foliage"
                    );
                    assert!(
                        pixel[0] > pixel[1] && pixel[1] > pixel[2],
                        "{name}'s trunk at ({x}, {y}) is {pixel:?}: brown is red over \
                         green over blue, and a green trunk is the bug this is for"
                    );
                }
            }
        }
    }

    /// The vines hang from the tile above rather than standing on the one below, so they
    /// are the mirror image of the rule above: opaque in the cell's *first* row, where the
    /// platform they grow out of touches them, and reaching well past halfway so the cell
    /// reads as something hanging rather than as something growing.
    #[test]
    fn hanging_props_grow_out_of_the_ceiling() {
        let pixels = Pixels::load();
        for name in ["vine_a", "vine_b"] {
            let sprite = pixels.sprite(name);
            assert!(
                (0..sprite.cell).any(|x| sprite.at(x, 0)[3] > 0),
                "{name} does not touch the top of its cell: it is floating under the \
                 platform rather than growing out of it"
            );
            let lowest = *sprite.opaque_rows().last().expect("a vine has pixels");
            assert!(
                lowest >= sprite.cell / 2,
                "{name} stops at row {lowest}: a vine that short is a leaf on the ceiling"
            );
            assert!(
                sprite.has_transparency(),
                "{name} is a filled square, not a hanging plant"
            );
        }
    }

    /// The view hangs every vine from a shelf's *deck*, and the deck is the top rows of the
    /// shelf's own cell: the two numbers are one decision about one drawing, so they are
    /// pinned together the way the fluid block's size is. If the plank were drawn deeper, a
    /// vine would be hung from the middle of the wood; if it were shallower, the stem would
    /// still float under it.
    #[test]
    fn a_one_way_shelf_deck_is_the_top_rows_of_its_cell() {
        let pixels = Pixels::load();
        let deck = (crate::view::ONEWAY_PLANK * pixels.cell as f32).round() as i32;
        assert!(
            deck > 0 && deck < pixels.cell,
            "a deck of {deck} rows in a {} row cell",
            pixels.cell
        );
        for name in ["oneway_l", "oneway_m", "oneway_r"] {
            let sprite = pixels.sprite(name);
            for y in 0..deck {
                let width = (0..sprite.cell).filter(|x| sprite.at(*x, y)[3] > 0).count() as i32;
                assert_eq!(
                    width, sprite.cell,
                    "{name} row {y} is not deck: the plank does not fill its cell"
                );
            }
            let brackets = (0..sprite.cell)
                .filter(|x| sprite.at(*x, deck)[3] > 0)
                .count() as i32;
            assert!(
                brackets < sprite.cell,
                "{name}: row {deck} still fills the cell, so ONEWAY_PLANK stops short of \
                 the wood's underside"
            );
        }
    }

    /// Every animation is an animation: its frames differ from each other, pixel for
    /// pixel. A cycle whose frames are the same drawing is the most invisible kind of
    /// broken art there is, and no drawing code can notice it.
    #[test]
    fn every_animation_actually_animates() {
        let pixels = Pixels::load();
        let cycles: [&[&str]; 10] = [
            &["player_idle_a", "player_idle_b"],
            &[
                "player_run_1",
                "player_run_2",
                "player_run_3",
                "player_run_4",
                "player_run_5",
                "player_run_6",
            ],
            &["walker_a", "walker_b", "walker_c", "walker_d"],
            &["goal_flag_1", "goal_flag_2", "goal_flag_3", "goal_flag_4"],
            &["bush_a", "bush_b"],
            &["tuft_a", "tuft_b"],
            &["dust_a", "dust_b"],
            &["spark_a", "spark_b"],
            &["reeds_a", "reeds_b"],
            &["vine_a", "vine_b"],
        ];
        for cycle in cycles {
            for (i, first) in cycle.iter().enumerate() {
                for second in &cycle[i + 1..] {
                    assert_ne!(
                        pixels.sprite(first).colours(),
                        pixels.sprite(second).colours(),
                        "{first} and {second} are the same drawing"
                    );
                }
            }
        }
        // The airborne poses are not a cycle, but they are three different states and
        // showing the same drawing for all three would be just as wrong.
        assert_ne!(
            pixels.sprite("player_jump").colours(),
            pixels.sprite("player_fall").colours()
        );
        assert_ne!(
            pixels.sprite("player_fall").colours(),
            pixels.sprite("player_land").colours()
        );
        assert_ne!(
            pixels.sprite("player_land").colours(),
            pixels.sprite("player_hurt").colours()
        );
    }

    /// The flag hangs off the pole: its pixels start to the right of the pole's columns,
    /// and the pole's own sprites are what the flag is drawn beside.
    #[test]
    fn the_flag_hangs_off_the_right_of_the_pole() {
        let pixels = Pixels::load();
        let pole = pixels.sprite("goal_top");
        let flag = pixels.sprite("goal_flag_1");
        let pole_columns: Vec<i32> = (0..pole.cell)
            .filter(|x| (0..pole.cell).any(|y| pole.at(*x, y)[3] > 0))
            .collect();
        assert!(
            pole_columns.iter().all(|x| *x >= 3 && *x <= 6),
            "the pole is not in the left part of its cell: {pole_columns:?}"
        );
        let flag_columns: Vec<i32> = (0..flag.cell)
            .filter(|x| (0..flag.cell).any(|y| flag.at(*x, y)[3] > 0))
            .collect();
        assert!(
            flag_columns.iter().all(|x| *x >= 6),
            "the flag overlaps the pole's own columns: {flag_columns:?}"
        );
        assert!(
            flag_columns.iter().any(|x| *x >= 12),
            "the flag does not reach far enough to read as a flag"
        );
    }

    /// A seam's material and side, from its name: `blend_{look}_{l|r|t|b}`.
    fn seam_name(name: &str) -> Option<(&str, char)> {
        let rest = name.strip_prefix("blend_")?;
        let (look, side) = rest.rsplit_once('_')?;
        let side = side.chars().next()?;
        Some((look, side))
    }

    /// Where a pixel sits relative to a side: how far in from that edge, and how far along it.
    fn depth_along(side: char, x: i32, y: i32) -> (i32, i32) {
        match side {
            'l' => (x, y),
            'r' => (15 - x, y),
            't' => (y, x),
            'b' => (15 - y, x),
            _ => panic!("unknown side {side:?}"),
        }
    }

    /// The pixel on the far edge of a side's band, `along` from the start of the edge.
    fn on_edge(side: char, along: i32) -> (i32, i32) {
        match side {
            'l' => (0, along),
            'r' => (15, along),
            't' => (along, 0),
            'b' => (along, 15),
            _ => panic!("unknown side {side:?}"),
        }
    }

    /// How far each look is allowed to creep, in pixels. The renderer's table (`view::BLEND`)
    /// names the bands and `tools/art/terrain.py`'s `SPILL` decides this; the numbers are here
    /// so a band that reached half way across a tile fails a test rather than being noticed.
    fn spill(look: &str) -> i32 {
        match look {
            "grass" | "dirt" => 3,
            "stone" => 2,
            "beach" | "packed" => 5,
            "sandstone" => 3,
            "brick" | "masonry" => 2,
            "wood" | "timber" => 3,
            other => panic!("unknown look {other:?}"),
        }
    }

    /// The seam bands are overlays, so each of them has to *be* one.
    ///
    /// Four properties, each of them a way a band can be wrong that no eye would separate from
    /// "the blending looks a bit off". A band that covered its cell would replace one material
    /// with the other rather than mixing them. A band that reached across the cell would paint a
    /// material onto the far side of the tile it belongs on. A band that covered its **own edge**
    /// solidly — which is what these were first drawn as, on the theory that the two materials
    /// have to meet — stacks against the band on the other side of the seam into two hard parallel
    /// stripes; sand against soil, whose tones are five steps apart, is then a bright line with a
    /// dark one beside it for the whole depth of the ground, and that is the seam this whole
    /// family of sprites exists to remove. A band whose grains did not thin out with depth would
    /// be a wedge of colour rather than a scatter of material. Nothing can leak sky at a seam
    /// either way: a band is drawn over the tile's own opaque cell, so every pixel of a seam
    /// belongs to one material or the other and the question is only which one owns it.
    ///
    /// The loosest material is also required to spill furthest, because that is what makes the
    /// depth a property of the material rather than a number per band.
    #[test]
    fn every_seam_is_a_band_along_its_own_side() {
        let pixels = Pixels::load();
        let mut deepest: Vec<(String, i32)> = Vec::new();
        let mut checked = 0;
        for name in pixels.names() {
            let Some((look, side)) = seam_name(&name) else {
                continue;
            };
            checked += 1;
            let sprite = pixels.sprite(&name);
            let band: Vec<i32> = (0..16)
                .flat_map(|y| (0..16).map(move |x| (x, y)))
                .filter(|(x, y)| sprite.at(*x, *y)[3] > 0)
                .map(|(x, y)| depth_along(side, x, y).0)
                .collect();
            assert!(!band.is_empty(), "{name} is empty");
            let share = band.len() * 100 / 256;
            assert!(
                share <= 40,
                "{name} is a stamp rather than a band: {share}% of its cell is opaque"
            );
            // The edge the material crosses carries the material on most of its length — it is
            // a band and not a fringe — and gives way on some of it, which is what lets the two
            // bands interlock instead of stacking. Both halves are the point, so both are asked
            // for: all 16 covered is the stripe this replaced, and a handful of pixels is a band
            // that is not really crossing the edge it lies on at all. The loosest material
            // crosses on five rows in six and the tightest on two in three, so seven is the floor.
            let covered = (0..16)
                .filter(|along| {
                    let (x, y) = on_edge(side, *along);
                    sprite.at(x, y)[3] > 0
                })
                .count();
            assert!(
                (7..16).contains(&covered),
                "{name} carries its material on {covered} of the 16 pixels of its own edge: a seam \
                 band has to cross the edge it lies on without ruling a solid stripe along it"
            );
            // And it thins inward: grains at the edge, strays further in.
            let at = |d: i32| band.iter().filter(|depth| **depth == d).count();
            let tail = band.iter().copied().max().expect("a band has pixels");
            assert!(
                at(0) > at(tail),
                "{name} has {} grain(s) on its edge and {} at its deepest: a band that does not \
                 thin out with depth is a wedge of colour",
                at(0),
                at(tail)
            );
            let reach = band.iter().copied().max().expect("a band has pixels");
            assert!(
                reach <= spill(look) + 1,
                "{name} reaches {reach} pixels into the tile, and {look} spills {}",
                spill(look)
            );
            deepest.push((look.to_string(), reach));
        }
        assert_eq!(checked, 38, "the atlas ships 38 seams");

        let furthest = |looks: &[&str]| -> i32 {
            deepest
                .iter()
                .filter(|(look, _)| looks.contains(&look.as_str()))
                .map(|(_, reach)| *reach)
                .max()
                .unwrap_or_else(|| panic!("no band for {looks:?}"))
        };
        let (sand, soil, stone) = (
            furthest(&["beach", "packed"]),
            furthest(&["grass", "dirt"]),
            furthest(&["stone"]),
        );
        assert!(
            sand > soil && soil > stone,
            "sand spills {sand}, soil {soil}, stone {stone}: the looser material should reach \
             further into the tile beside it"
        );
    }

    /// A band is cut out of the material's own cell, so its pixels *are* the material's pixels.
    ///
    /// This is what makes a seam impossible to get subtly wrong: there is no second palette in
    /// which to choose a nearly-right yellow. Every opaque pixel of `blend_{look}_*` has to be a
    /// colour of the plain tile that look is drawn from — the crown and body for the surface
    /// looks, the body for the buried ones — so the way a material looks at a seam cannot drift
    /// away from the way it looks everywhere else, and a band cannot be hand-painted at all.
    #[test]
    fn a_seam_is_made_of_the_materials_own_pixels() {
        let pixels = Pixels::load();
        let plain = [
            ("grass", "ground_1"),
            ("dirt", "ground_0"),
            ("stone", "deep_0"),
            ("beach", "sand_1"),
            ("packed", "sand_0"),
            ("sandstone", "sandstone_0"),
            ("brick", "brick_1"),
            ("masonry", "masonry_0"),
            ("wood", "wood_1"),
            ("timber", "timber_0"),
        ];
        for (look, tile) in plain {
            let allowed = pixels.sprite(tile).colours();
            for side in ['l', 'r', 't', 'b'] {
                let name = format!("blend_{look}_{side}");
                if !pixels.names().contains(&name) {
                    continue;
                }
                let sprite = pixels.sprite(&name);
                for y in 0..16 {
                    for x in 0..16 {
                        let pixel = sprite.at(x, y);
                        if pixel[3] == 0 {
                            continue;
                        }
                        assert!(
                            allowed.contains(&pixel),
                            "{name} pixel ({x}, {y}) is {pixel:?}, which is not a colour of \
                             {tile}: a band must be cut from the material it carries"
                        );
                    }
                }
            }
        }
    }
}
