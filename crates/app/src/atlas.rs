//! The art atlas: `assets/tileset.png` plus the manifest that names its cells.
//!
//! The manifest is the contract, not the pixel coordinates. A sprite is looked up
//! by name, so a replacement tileset only has to list the names this crate asks for
//! — and [`Atlas::require`] fails loudly, listing what the file *does* contain,
//! rather than drawing a mystery tile.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::{ColorImage, Context, Pos2, Rect, TextureHandle, TextureOptions, Vec2};

/// The names the renderer and editor ask for. A tileset missing any of these
/// cannot draw a level.
pub const REQUIRED: [&str; 17] = [
    "empty",
    "grass",
    "grass_left",
    "grass_right",
    "dirt",
    "dirt_left",
    "dirt_right",
    "spike",
    "player_stand",
    "player_run_a",
    "player_run_b",
    "player_jump",
    "walker_a",
    "walker_b",
    "walker_dead",
    "goal_top",
    "goal_bottom",
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
            ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()),
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
        let (cw, ch) = (1.0 / self.cols as f32, 1.0 / self.rows as f32);
        let min = Pos2::new(sprite.col as f32 * cw, sprite.row as f32 * ch);
        Ok(Rect::from_min_size(min, Vec2::new(cw, ch)))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
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
        let expected = Vec2::new(1.0 / cols as f32, 1.0 / rows as f32);
        for name in atlas.names() {
            let uv = atlas.uv(name).expect("every listed sprite resolves");
            assert!(
                uv.min.x >= 0.0 && uv.min.y >= 0.0 && uv.max.x <= 1.0 && uv.max.y <= 1.0,
                "{name} maps outside the texture: {uv:?}"
            );
            assert!(
                (uv.width() - expected.x).abs() < 1e-6
                    && (uv.height() - expected.y).abs() < 1e-6,
                "{name} is not one cell ({expected:?}): {uv:?}"
            );
        }
    }

    #[test]
    fn the_shipped_atlas_satisfies_every_name_the_renderer_asks_for() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        assert_eq!(atlas.missing_required(), Vec::<&str>::new());
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
            Ok(_) => panic!("a 9-column grid cannot describe a 128px atlas"),
            Err(err) => err,
        };
        assert!(err.contains("grid"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
