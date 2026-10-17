//! The pixel-art view: a camera, the tile painter, and the live-sim painter.
//!
//! The boundary rule is the `render` crate's: **this module only draws.** It
//! simulates nothing, mutates nothing, and takes `&Level`, `&Sim` or `&Scene`.
//!
//! Everything here works in two spaces and never confuses them:
//!
//! * **tile space** — one unit per tile, `(0, 0)` top-left, which is the space the
//!   level file, the editor and `sim` all speak;
//! * **screen space** — egui points, after [`Camera::to_screen`].
//!
//! Screen rectangles are rounded to whole points before drawing. That is what keeps
//! pixel art crisp at fractional camera positions: a tile drawn from 10.3 points
//! would have its rows sampled at half-pixels and shimmer as the camera moves.

use std::collections::BTreeMap;

use egui::{Color32, Mesh, Painter, Pos2, Rect, TextureId, Vec2};
use sim::reach::ReachReport;
use sim::{Level, Outcome, Sim, Tile, Walker};

use crate::atlas::Atlas;

/// Sky, top to bottom of the window. Sampled into a two-triangle gradient.
const SKY_TOP: Color32 = Color32::from_rgb(86, 160, 232);
const SKY_BOTTOM: Color32 = Color32::from_rgb(178, 214, 240);
/// The two parallax hill bands: far first, so a level with a pit in it shows distance
/// rather than a hole with more sky behind it.
const HILL_FAR: Color32 = Color32::from_rgb(126, 168, 186);
const HILL_NEAR: Color32 = Color32::from_rgb(96, 146, 132);
/// The editor's own surface: the flat blue a document being *edited* is set on.
///
/// A third material beside the window's cream chrome and the sky it plays under — neither of them,
/// and deliberately between them in weight, so the level's own art (green, brown, terracotta) is the
/// brightest thing on it. See [`draw_background`] for why it is flat.
const EDITOR_SURFACE: Color32 = Color32::from_rgb(150, 186, 218);

/// The colours the sky-and-hills backdrop is painted with.
///
/// Asked for, rather than written down again, by the art test that holds the player's
/// palette to "not the colour of the backdrop": a character is chosen against *these*
/// pixels, and a test carrying its own copy of them would be asserting the copy. This is
/// the rule that let a cyan runner and a blue sky ship together for as long as it did.
///
/// Test-only because the renderer does not need it: it paints the sky mesh, the two hill
/// bands, the sun and the tree-line with the constants directly. The sun and the pines are
/// in the list because they are *also* things a runner can be drawn in front of — a pale
/// disc and a hazy silhouette are two very different backgrounds, and the rule is about
/// every one of them rather than about the sky it happens to start from.
#[cfg(test)]
pub fn backdrop_colours() -> [Color32; 13] {
    [
        SKY_TOP,
        SKY_BOTTOM,
        HILL_FAR,
        HILL_NEAR,
        SUN_FACE,
        PINE_LIGHT,
        PINE,
        PINE_DARK,
        PINE_SHADE,
        PINE_FAR_LIGHT,
        PINE_FAR,
        PINE_NEAR_LIGHT,
        PINE_NEAR,
    ]
}
const GRID_LINE: Color32 = Color32::from_rgba_premultiplied(24, 30, 40, 26);
const BOUNDS_LINE: Color32 = Color32::from_rgb(40, 52, 68);
const SPAWN_LINE: Color32 = Color32::from_rgb(120, 92, 30);
const UNREACHED_TINT: Color32 = Color32::from_rgba_premultiplied(90, 20, 24, 96);
const CULPRIT_LINE: Color32 = Color32::from_rgb(226, 154, 40);
const FRONTIER_LINE: Color32 = Color32::from_rgb(210, 90, 60);
/// The sun: one pale disc, far enough away that it is a light rather than a place.
///
/// The same colour `tools/art/common.py` calls `SUN`, and the only thing in the backdrop
/// that was ever *drawn* from the palette without anything using it: it sat unused in the
/// art vocabulary from the first commit until the sky grew a sun.
const SUN_FACE: Color32 = Color32::from_rgb(255, 240, 190);
/// The far tree-line: the hazy greens of `common.py`'s four `PINE_*` tones, which the
/// pines' own pixels are drawn in. They live here because a runner can be drawn in front of
/// one, so the art rule has to know the colours — the sprites themselves come from the
/// atlas.
///
/// All of them, not one: the rule is about every colour a character can be drawn against, and
/// the tree-line is the one backdrop thing whose tones are far enough apart to matter. They
/// are the conifer green mixed most of the way to [`HILL_FAR`], which is what makes a tree
/// standing on a faded hill read as being *on* it rather than in front of it — and the rank
/// behind is a further step into the same haze, so the two ranks are two distances rather than
/// two sizes of the same one.
///
/// Lightened in the parallax pass, and greened with it: the tones this replaces
/// (117/106/99/92) were a dark blue-grey whose darkest member sat at a luminance near
/// 120, and a pine's tiers alternate the body with the shadow, so half of every crown
/// was that darkest tone — a row of trees that read as one dark mass against the hill
/// rather than as trees *on* it. Every tone is roughly twenty points brighter now
/// and green is the strongest channel in each, so the crown is a pale hazy green and
/// only the trunk keeps the old depth. The mirrors in `tools/art/common.py` say the
/// same thing, because the PNG is drawn from them and this list is drawn from the PNG.
#[cfg(test)]
const PINE_LIGHT: Color32 = Color32::from_rgb(142, 180, 168);
#[cfg(test)]
const PINE: Color32 = Color32::from_rgb(128, 172, 158);
#[cfg(test)]
const PINE_DARK: Color32 = Color32::from_rgb(116, 160, 148);
#[cfg(test)]
const PINE_SHADE: Color32 = Color32::from_rgb(104, 148, 138);
#[cfg(test)]
const PINE_FAR_LIGHT: Color32 = Color32::from_rgb(140, 180, 182);
#[cfg(test)]
const PINE_FAR: Color32 = Color32::from_rgb(130, 170, 174);
/// The near hills' tree-line: the same conifer one band closer, in `common.py`'s
/// `PINE_NEAR_*` tones — the saturated green hazed half way to the near hills'
/// own colour, so the rank sits in the backdrop's distance rather than standing
/// out of it, while staying darker than its hill and stronger than the far rank:
/// that ordering is the depth cue the fade test holds. Only the lit tone and the
/// body are listed: those are the two a runner can fill a frame against.
#[cfg(test)]
const PINE_NEAR_LIGHT: Color32 = Color32::from_rgb(100, 161, 118);
#[cfg(test)]
const PINE_NEAR: Color32 = Color32::from_rgb(90, 152, 110);
/// Tiles between one dome and the next in a band.
///
/// Five, and every band's shortest dome is at least half of that, so two neighbours always
/// meet: a pair of hills that does not touch leaves the range behind them showing in the gap
/// between them, which is a hole in a hillside however it is painted. The near band used to
/// break that rule — its shortest dome was under half the spacing — and the flat base line
/// showing through between its mounds is what it looked like.
const BAND_SPACING: f32 = 5.0;
/// The far band's domes: how far above the horizon its shortest and its tallest stand, in
/// tiles. Taller than the near band's, because a range further away stands higher on the
/// screen — that difference is what makes the two bands read as two distances rather than as
/// one line of hills drawn twice.
const FAR_DOMES: (f32, f32) = (3.4, 4.8);
/// The near band's. Shorter than the far band's, so the range behind shows over this one,
/// and never shorter than [`BAND_SPACING`]'s half, so its own domes always meet.
const NEAR_DOMES: (f32, f32) = (2.5, 3.2);
/// The lowest and highest the *ground between* a band's domes stands above its horizon, in
/// tiles, and how many tiles one crest of that wave is from the next.
///
/// The band's own ground never touches its horizon line, and that is the whole point of it:
/// this is the floor of every saddle in the range, and a saddle that reached the horizon
/// would be a flat stripe of the band's colour running from one hill to the next with the
/// range behind it above — a gap, which is exactly what the hills used to have. It waves so
/// that no two saddles are the same depth.
const SWELL: (f32, f32) = (0.35, 1.1);
const SWELL_TILES: f32 = 11.0;
/// Tiles a band is drawn *past* the window on each side.
///
/// Things are drawn in front of it that stand outside the frame: the tree-line plants its
/// clumps two of its own spacings beyond the edge, and a clump is a tree and a half wide. A
/// band that stopped at the window's edge leaves those trees standing in the air.
const SKY_PAD: f32 = 8.0;
/// How high the sun stands above the level's ground line, in tiles.
///
/// An altitude above the *ground*, not a fraction of the window: the sky belongs to the
/// world, so a sun a tenth of the way down the window is a sun that slides relative to
/// the level every time the camera changes what it is looking at or how far away it is.
///
/// It has to fit in the sky a *follow* camera has above the player, which is half a
/// window at 28 points a tile — about twelve tiles — and above the hills, which stand
/// five of them, and above the clouds, whose own altitudes are ranges rather than rows
/// (see [`CloudLayer`]). The sun is above the highest cloud; a camera zoomed in past
/// that leaves the sun out of the frame, which is what looking at the ground does.
const SUN_ALTITUDE: f32 = 10.5;
/// The sun's parallax: the smallest in the scene, because it is the furthest thing in it.
///
/// Every backdrop layer's factor lives in one place now — this, the two
/// [`CloudLayer`]s and the two hill bands — and they are in strictly increasing
/// order back to front, which is the whole of what "parallax" means here: a layer
/// that is drawn behind another also *moves* less than it. The clouds are the
/// cautionary tale: they used to be called with 0.30 and 0.55, in front of nothing
/// but moving faster than both hill bands in front of them, so the sky slid past
/// the hills it was supposed to be behind — a backdrop where the distance cues
/// disagreed with each other, and the eye believed the motion.
const SUN_PARALLAX: f32 = 0.04;
/// The far hills' parallax, shared with the tree-line that stands on their ridge.
///
/// Shared because a tree planted on a ridge has to move with the ridge: at its own parallax
/// it slides along the silhouette and its foot climbs and falls with the domes it passes,
/// which is a fence of bobbing trees rather than a hillside. Two things that stand on each
/// other are one layer.
const FAR_HILLS_PARALLAX: f32 = 0.20;
/// The near hills' parallax, shared with the tree-line that stands on *their* ridge.
/// They are in front of the far band, and in front of its trees.
const NEAR_HILLS_PARALLAX: f32 = 0.38;
/// Where the population keeps dying. Warm, so a corpse never reads as terrain.
const DEATH_MARK: [u8; 3] = [235, 70, 60];
/// A recorded champion route: its trail, and the two ways it can end.
const ROUTE_TRAIL: Color32 = Color32::from_rgba_premultiplied(34, 28, 56, 150);
const ROUTE_FLAG: Color32 = Color32::from_rgb(110, 220, 130);
const ROUTE_GRAVE: Color32 = Color32::from_rgb(235, 70, 60);

/// Zoom limits, in points per tile. Below the minimum the grid is unreadable, above
/// the maximum a single tile fills the window.
pub const MIN_ZOOM: f32 = 6.0;
pub const MAX_ZOOM: f32 = 96.0;
pub const DEFAULT_ZOOM: f32 = 28.0;

/// What to draw over the level.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderOpts {
    /// Faint tile grid, for editing.
    pub grid: bool,
    /// The spawn marker: a plain rectangle in play, a labelled box in the editor.
    pub spawn_marker: bool,
    /// Outline the level's own bounds.
    pub bounds: bool,
    /// The level's own entity placement — its walkers — drawn where the *file* puts them.
    ///
    /// The editor needs this and a live view must not have it. A `Sim` marches its walkers
    /// away from the tiles they were placed on and draws them itself through
    /// [`draw_agents`], so a static pass underneath that would leave a copy of every walker
    /// standing where it started, and two of everything is worse than none of anything.
    pub entities: bool,
    /// Ambient life: the butterflies drifting above the level ([`draw_ambient`]).
    ///
    /// Off in the editor, and the reason is the same one the sky is off there: a document
    /// view is about what the *file* says, and what the file says about a butterfly is
    /// nothing at all. A painter also wants the tile under the cursor to stay put while
    /// they draw it, and a moving thing on the canvas is the one layer that cannot be
    /// read off the level.
    pub ambient: bool,
}

impl RenderOpts {
    /// The editor's view: grid, spawn marker, bounds.
    pub fn editing() -> RenderOpts {
        RenderOpts {
            grid: true,
            spawn_marker: true,
            bounds: true,
            entities: true,
            ambient: false,
        }
    }

    /// The play view: no grid, no editing furniture, and the level is inhabited.
    pub fn playing() -> RenderOpts {
        RenderOpts {
            grid: false,
            spawn_marker: false,
            bounds: true,
            entities: false,
            ambient: true,
        }
    }
}

/// A window onto the level: where it is centred and how big a tile is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Tile coordinate at the centre of the viewport.
    pub centre: Vec2,
    /// Points per tile.
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            centre: Vec2::ZERO,
            zoom: DEFAULT_ZOOM,
        }
    }
}

impl Camera {
    /// A camera showing a whole level inside `viewport` with a margin.
    ///
    /// The fitted zoom is **floored** to whole points so the whole level still fits at the
    /// integer scale the renderer uses ([`Camera::scale`]): rounding up would clip a sliver
    /// off the far edge, which is the one thing this camera exists to avoid.
    pub fn fit(level: &Level, viewport: Rect, margin: f32) -> Camera {
        let avail = Vec2::new(
            (viewport.width() - margin * 2.0).max(1.0),
            (viewport.height() - margin * 2.0).max(1.0),
        );
        let zoom = (avail.x / level.width.max(1) as f32)
            .min(avail.y / level.height.max(1) as f32)
            .floor()
            .clamp(MIN_ZOOM, MAX_ZOOM);
        Camera {
            centre: Vec2::new(level.width as f32 * 0.5, level.height as f32 * 0.5),
            zoom,
        }
    }

    /// Points per tile as the renderer uses them: whole points, never fewer than one.
    ///
    /// Pixel art has exactly one correct scale and it is an integer. At 17.2 points a tile
    /// a sixteen-pixel sprite spans seventeen pixels on one row and eighteen on the next,
    /// and a tile boundary lands halfway inside a pixel — which is the shimmer, the uneven
    /// sprite widths and the one-pixel seams between two floor tiles. Every sprite, tile
    /// and effect is drawn through this, so the whole frame shares one integer scale.
    pub fn scale(&self) -> f32 {
        self.zoom.round().clamp(1.0, MAX_ZOOM)
    }

    /// Top-left corner of tile `(tx, ty)` in screen space, on a whole pixel.
    ///
    /// The origin is snapped once, here, rather than per rectangle: `origin + tile * scale`
    /// with both terms whole makes every tile boundary a whole pixel **and** makes two
    /// neighbouring tiles share an edge exactly. Rounding each rectangle on its own — which
    /// is what this used to do — is what leaves a one-pixel seam between one of them and
    /// the next.
    pub fn to_screen(self, tx: f32, ty: f32, viewport: Rect) -> Pos2 {
        let c = viewport.center();
        let scale = self.scale();
        Pos2::new(
            (c.x - self.centre.x * scale).round() + tx * scale,
            (c.y - self.centre.y * scale).round() + ty * scale,
        )
    }

    /// Tile-space position of a screen point. The exact inverse of
    /// [`Camera::to_screen`]; fractional inside a tile.
    pub fn to_tile(self, screen: Pos2, viewport: Rect) -> Vec2 {
        let c = viewport.center();
        let scale = self.scale();
        let origin = Vec2::new(
            (c.x - self.centre.x * scale).round(),
            (c.y - self.centre.y * scale).round(),
        );
        Vec2::new((screen.x - origin.x) / scale, (screen.y - origin.y) / scale)
    }

    /// Screen rectangle of one tile: a whole pixel, exactly `scale()` on a side.
    pub fn tile_rect(&self, tx: i32, ty: i32, viewport: Rect) -> Rect {
        Rect::from_min_size(
            self.to_screen(tx as f32, ty as f32, viewport),
            Vec2::splat(self.scale()),
        )
    }

    /// Screen rectangle of a world-space box given in tiles (`sim`'s units).
    ///
    /// The *size* is rounded rather than the corners: an agent's body is the same size in
    /// every frame unless the body itself changed, so a sprite does not gain and lose a
    /// pixel of width as its fractional position crosses a boundary.
    pub fn world_rect(&self, x: f32, y: f32, w: f32, h: f32, viewport: Rect) -> Rect {
        let scale = self.scale();
        let min = self.to_screen(x, y, viewport);
        Rect::from_min_size(
            Pos2::new(min.x.round(), min.y.round()),
            Vec2::new((w * scale).round().max(1.0), (h * scale).round().max(1.0)),
        )
    }

    /// Zoom about a screen point, keeping the tile under it in place.
    pub fn zoom_at(&mut self, factor: f32, anchor: Pos2, viewport: Rect) {
        let before = self.to_tile(anchor, viewport);
        // Snapped to a whole point here rather than only at draw time, so the zoom the
        // panel prints is the zoom the frame is drawn at.
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM).round();
        let after = self.to_tile(anchor, viewport);
        self.centre += before - after;
    }

    /// Keep the camera centre inside the level, allowing a margin so a level edge is
    /// reachable but the view cannot be lost in empty space.
    pub fn clamp_to(&mut self, level: &Level, keep: f32) {
        self.centre.x = self.centre.x.clamp(-keep, level.width as f32 + keep);
        self.centre.y = self.centre.y.clamp(-keep, level.height as f32 + keep);
    }

    /// Inclusive tile bounds visible in `viewport`, clamped to the level.
    pub fn visible_tiles(&self, level: &Level, viewport: Rect) -> (i32, i32, i32, i32) {
        let a = self.to_tile(viewport.min, viewport);
        let b = self.to_tile(viewport.max, viewport);
        (
            (a.x.floor() as i32).max(0),
            (b.x.ceil() as i32 + 1).min(level.width),
            (a.y.floor() as i32).max(0),
            (b.y.ceil() as i32 + 1).min(level.height),
        )
    }
}

/// The sprite rectangles the renderer needs, resolved once at startup.
///
/// A map keyed by the manifest's own names rather than one field per sprite: the atlas
/// is seventy cells and grows every time the art does, and a struct with seventy fields
/// is a struct every new sprite has to be threaded through in three places. Names are
/// what the manifest already speaks, so the call site asks for `"ground_1"` and the
/// manifest decides where that is.
pub struct Sprites {
    rects: BTreeMap<String, Rect>,
}

impl Sprites {
    /// Resolve every sprite the game requires, or say which are missing.
    ///
    /// A missing required sprite is a startup error listing the names — the same
    /// contract the atlas has always had. Optional art (the tile variants) is simply
    /// absent from the map, and [`Sprites::try_get`] answers `None` for it.
    pub fn load(atlas: &Atlas) -> Result<Sprites, String> {
        let missing = atlas.missing_required();
        if !missing.is_empty() {
            return Err(format!(
                "{} is missing: {}",
                atlas.source().display(),
                missing.join(", ")
            ));
        }
        let mut rects = BTreeMap::new();
        for name in atlas.names() {
            if let Ok(uv) = atlas.uv(name) {
                rects.insert(name.to_string(), uv);
            }
        }
        Ok(Sprites { rects })
    }

    /// A sprite the game requires.
    ///
    /// Panics on anything else, which can only be a typo at a call site: every name this
    /// is called with is in [`crate::atlas::REQUIRED`], and a tileset missing one of
    /// those fails at startup rather than here.
    pub fn get(&self, name: &str) -> Rect {
        *self
            .rects
            .get(name)
            .unwrap_or_else(|| panic!("sprite {name:?} is not in REQUIRED and is not in the atlas"))
    }

    /// A sprite the game can do without, e.g. one of the tile variants.
    pub fn try_get(&self, name: &str) -> Option<Rect> {
        self.rects.get(name).copied()
    }

    /// The first of `names` this tileset actually has.
    ///
    /// This is how a *variant* is chosen: the caller hands over the plain name and its
    /// decorated alternatives, and a tileset made of plain tiles answers with the plain
    /// one instead of failing.
    pub fn first_of(&self, names: &[&str]) -> Rect {
        names
            .iter()
            .find_map(|name| self.try_get(name))
            .unwrap_or_else(|| panic!("none of {names:?} is in the atlas"))
    }
}

/// How much bigger an agent's *picture* is than its physics box, measured on the
/// box's height. See [`Sheet::actor`]: the box is a collision shape the sim owns,
/// and at 1.0 the player — the one sprite that is always on screen — was smaller
/// than the bushes it ran past. 1.3 puts the runner at about 1.17 tiles against a
/// walker's 0.78, a bush's 0.75 and a foreground tree's 3.0: present without
/// towering, and the same factor for every agent, so the player-to-walker ratio
/// the physics describes (0.9 against 0.6) is the ratio the eye sees.
const ACTOR_DRAW_SCALE: f32 = 1.3;

/// Everything a draw call needs that is not the thing being drawn.
///
/// One value rather than eight arguments, and the texture id and sprite rectangles
/// resolve once per frame instead of once per call site.
#[derive(Clone, Copy)]
pub struct Sheet<'a> {
    pub painter: &'a Painter,
    pub texture: TextureId,
    pub sprites: &'a Sprites,
    pub camera: &'a Camera,
    pub viewport: Rect,
}

impl<'a> Sheet<'a> {
    pub fn new(
        painter: &'a Painter,
        atlas: &Atlas,
        sprites: &'a Sprites,
        camera: &'a Camera,
        viewport: Rect,
    ) -> Sheet<'a> {
        Sheet {
            painter,
            texture: atlas.texture_id(),
            sprites,
            camera,
            viewport,
        }
    }

    /// Screen rectangle of one tile.
    pub fn tile(&self, tx: i32, ty: i32) -> Rect {
        self.camera.tile_rect(tx, ty, self.viewport)
    }

    /// Screen rectangle of a world-space box measured in tiles, as `sim` reports it.
    pub fn world(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        self.camera.world_rect(x, y, w, h, self.viewport)
    }

    /// Draw one sprite cell.
    pub fn image(&self, rect: Rect, uv: Rect) {
        self.image_tinted(rect, uv, Color32::WHITE);
    }

    /// Draw one sprite cell multiplied by `tint`.
    ///
    /// This is how the evolution view shows eight agents at once: the champion is drawn
    /// opaque, the rest translucent, and the eye can still tell them apart where they
    /// overlap. A tint of `WHITE` is the plain sprite.
    pub fn image_tinted(&self, rect: Rect, uv: Rect, tint: Color32) {
        self.painter.image(self.texture, rect, uv, tint);
    }

    /// Draw one sprite cell, mirrored left-to-right when `flip` is set.
    ///
    /// Mirroring is the atlas rectangle with its x corners swapped, which is free and
    /// exact: an agent facing the way it is travelling is worth a second set of sprites
    /// to a hand-drawn game, and worth nothing at all to a texture sampler.
    pub fn image_facing(&self, rect: Rect, uv: Rect, flip: bool, tint: Color32) {
        let uv = if flip {
            Rect::from_min_max(Pos2::new(uv.max.x, uv.min.y), Pos2::new(uv.min.x, uv.max.y))
        } else {
            uv
        };
        self.painter.image(self.texture, rect, uv, tint);
    }

    /// Draw an agent sprite in a physics box, keeping the *sprite's* proportions.
    ///
    /// The player's box is 0.6 tiles wide and 0.9 tall, so stretching a 16x16 cell into
    /// it squeezes the art to two thirds of its width: a circle becomes an ellipse and a
    /// round head becomes a narrow one. Fitting the larger square that goes inside the
    /// box instead — centred horizontally, standing on the box's floor — costs nothing
    /// and makes every character keep the proportions it was drawn with.
    ///
    /// The square is sized off the box's *height*, enlarged by [`ACTOR_DRAW_SCALE`] —
    /// and both halves of that sentence fix a bug apiece. This used to fit the square
    /// to the *smaller* side, `w.min(h)`, which threw the player's height away: a
    /// 0.6-by-0.9 box was drawn as a 0.6-tile square, exactly the walker's size, even
    /// though the walker it runs into is a 0.6-by-0.6 box — the one sprite that is
    /// always on screen was the smallest thing in the scene, smaller than a bush
    /// (0.75 tiles) standing beside it. And a sprite drawn at exactly its physics
    /// height reads small next to everything else in the vocabulary anyway, because
    /// the rest of the world is drawn generously: foreground trees at a tile and a
    /// half per cell, the goal two tiles tall. So the art overflows its box by the
    /// scale below — centred on the box, feet on the box's floor — the way a
    /// platformer character is always bigger than its collision. The physics does
    /// not change: the box is the sim's, and this only decides how much picture is
    /// painted over it. At the scale below the player stands about 1.17 tiles tall
    /// and the walker about 0.78, so the runner is taller than the thing it stomps
    /// and shorter than the flag it runs to, which is the ordering the eye expects.
    ///
    /// The argument list is a box, its art, its mirror and its tint — the whole of what an
    /// agent sprite *is*, and a struct for it would only be unpacked again at the two call
    /// sites that exist.
    #[allow(clippy::too_many_arguments)]
    pub fn actor_squashed(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        squash_x: f32,
        squash_y: f32,
        uv: Rect,
        flip: bool,
        tint: Color32,
    ) {
        let base_side = h * ACTOR_DRAW_SCALE;
        let width_tiles = base_side * squash_x;
        let height_tiles = base_side * squash_y;
        let rect = self.camera.world_rect(
            x + w * 0.5 - width_tiles * 0.5,
            y + h - height_tiles,
            width_tiles,
            height_tiles,
            self.viewport,
        );
        self.image_facing(rect, uv, flip, tint);
    }

    /// An agent at its resting shape: the box, its art, its mirror and its tint, with no
    /// squash. The narrowing the callers that do not animate want, and the reason
    /// [`Sheet::actor_squashed`] can carry the full animator's argument list.
    #[allow(clippy::too_many_arguments)]
    pub fn actor(&self, x: f32, y: f32, w: f32, h: f32, uv: Rect, flip: bool, tint: Color32) {
        self.actor_squashed(x, y, w, h, 1.0, 1.0, uv, flip, tint);
    }
}

/// Is a *real* solid tile here? Not [`Level::is_solid`], which reports the invisible
/// side walls: an art decision wants the level's edge to look like open air.
pub fn solid_at(level: &Level, tx: i32, ty: i32) -> bool {
    // `is_solid`, not `== Tile::Solid`: the material a solid tile is made of is a drawing
    // decision, and every rule in this file that asks "is there ground here" — the
    // autotile mask, the scenery, the vines — has to answer the same for sand as for soil.
    // Asked the other way round, a level made of sand has no ground in it at all: no edge
    // masks, no props standing on it, and a tree refusing to be planted on a beach.
    level.tile(tx, ty).is_some_and(Tile::is_solid)
}

/// The 4-bit "open to the air on this side" mask of the solid tile at `(tx, ty)`.
///
/// bit 0 = open above, bit 1 = open left, bit 2 = open right, bit 3 = open below. This
/// is a frozen contract with the art: `assets/tileset.json` carries one `ground_N`
/// sprite per mask, and `tools/art/terrain.py` says the same thing about the same bits.
/// Sixteen hand-drawn edge sprites was already more than the old art could keep
/// consistent, so the mask decides the tile and nothing else does.
pub fn ground_mask(level: &Level, tx: i32, ty: i32) -> u8 {
    let mut mask = 0u8;
    if !solid_at(level, tx, ty - 1) {
        mask |= 0b0001;
    }
    if !solid_at(level, tx - 1, ty) {
        mask |= 0b0010;
    }
    if !solid_at(level, tx + 1, ty) {
        mask |= 0b0100;
    }
    if !solid_at(level, tx, ty + 1) {
        mask |= 0b1000;
    }
    mask
}

/// Solid tiles stacked above a tile before it stops being soil and becomes rock.
pub const ROCK_DEPTH: i32 = 3;

/// How deep under a surface `(tx, ty)` is, capped at [`ROCK_DEPTH`].
fn depth_under_surface(level: &Level, tx: i32, ty: i32) -> i32 {
    let mut depth = 0;
    while depth < ROCK_DEPTH && solid_at(level, tx, ty - depth - 1) {
        depth += 1;
    }
    depth
}

/// The plain autotile for each mask, and the rock that replaces it underground.
///
/// Static tables rather than a `format!` per tile per frame: a sixty-wide level asks for
/// this a few hundred times every frame, and a level is a *level* — the same mask is
/// drawn thousands of times, so the name may as well be looked up.
const GROUND: [&str; 16] = [
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
];
/// Rock exists only for masks with no sky above (bit 0 clear), which is why the odd
/// entries are empty and why the lookup asserts before it indexes.
const ROCK: [&str; 16] = [
    "deep_0", "", "deep_2", "", "deep_4", "", "deep_6", "", "deep_8", "", "deep_10", "", "deep_12",
    "", "deep_14", "",
];

/// The same sixteen masks in sand, and the rock under it in sandstone.
///
/// A second material rather than a tint: a beach's crown is its own drawing — sand piled by
/// wind, no grass on it — so `sand_5` is not `ground_5` in yellow, it is a bank of sand with
/// the same silhouette contract. The deep half exists for the same reason rock does: three
/// tiles into a dune the loose sand is compressed, and drawing it as the grey rock under a
/// *hill* would put a stripe of somebody else's material inside a beach.
const SAND: [&str; 16] = [
    "sand_0", "sand_1", "sand_2", "sand_3", "sand_4", "sand_5", "sand_6", "sand_7", "sand_8",
    "sand_9", "sand_10", "sand_11", "sand_12", "sand_13", "sand_14", "sand_15",
];
const SANDSTONE: [&str; 16] = [
    "sandstone_0",
    "",
    "sandstone_2",
    "",
    "sandstone_4",
    "",
    "sandstone_6",
    "",
    "sandstone_8",
    "",
    "sandstone_10",
    "",
    "sandstone_12",
    "",
    "sandstone_14",
    "",
];

const BRICK: [&str; 16] = [
    "brick_0", "brick_1", "brick_2", "brick_3", "brick_4", "brick_5", "brick_6", "brick_7",
    "brick_8", "brick_9", "brick_10", "brick_11", "brick_12", "brick_13", "brick_14", "brick_15",
];
const MASONRY: [&str; 16] = [
    "masonry_0",
    "",
    "masonry_2",
    "",
    "masonry_4",
    "",
    "masonry_6",
    "",
    "masonry_8",
    "",
    "masonry_10",
    "",
    "masonry_12",
    "",
    "masonry_14",
    "",
];

const WOOD: [&str; 16] = [
    "wood_0", "wood_1", "wood_2", "wood_3", "wood_4", "wood_5", "wood_6", "wood_7", "wood_8",
    "wood_9", "wood_10", "wood_11", "wood_12", "wood_13", "wood_14", "wood_15",
];
const TIMBER: [&str; 16] = [
    "timber_0",
    "",
    "timber_2",
    "",
    "timber_4",
    "",
    "timber_6",
    "",
    "timber_8",
    "",
    "timber_10",
    "",
    "timber_12",
    "",
    "timber_14",
    "",
];

/// The decorated alternatives to a plain tile, in the order the variant index picks them.
///
/// They are optional art: a tileset without them draws the plain tile, which is why this
/// is a lookup and not a second required name per mask.
const VARIANTS: [(&str, [&str; 2]); 5] = [
    ("ground_1", ["ground_1_b", "ground_1_c"]),
    ("deep_0", ["deep_0_b", "deep_0_c"]),
    ("sand_1", ["sand_1_b", "sand_1_c"]),
    ("brick_1", ["brick_1_b", "brick_1_c"]),
    ("wood_1", ["wood_1_b", "wood_1_c"]),
];

/// The rock under a tile, or the soil on top of it: the family that is *not* optional.
///
/// Split out because sand is optional art and soil is not, so the two answers have to be
/// able to disagree. `mask` is 4 bits, so indexing is total; the empty entries are the
/// masks that cannot be buried, and the assert says so rather than returning one.
fn soil_name(mask: u8, deep: bool) -> &'static str {
    let name = if deep {
        ROCK[mask as usize]
    } else {
        GROUND[mask as usize]
    };
    debug_assert!(
        !name.is_empty(),
        "mask {mask} has no soil sprite at depth {deep}"
    );
    name
}

/// What a solid tile *looks* like: which material it is made of, and how that material is
/// drawn where the tile sits.
///
/// Six looks, because a material has three appearances — a crown where the sky touches it,
/// a body a few tiles down, and rock under that — and the game has five materials: soil,
/// sand, stone, brick and timber. The [`Family`] is the part that decides whether two tiles
/// blend: a lawn's grass and the dirt under it are one material at two depths and already join
/// without a seam (the crown's last row is drawn ragged against the body on purpose), while
/// sand beside dirt is two materials and is the seam the blends exist for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    Grass,
    Dirt,
    Stone,
    Beach,
    Packed,
    Sandstone,
    Brick,
    Masonry,
    Wood,
    Timber,
}

/// The material a look is made of. Two looks of one family never blend.
///
/// A family is one substance at two depths — grass over dirt, brick coping over masonry,
/// decking over structural beams — and the crown's ragged last row already joins the two, so
/// the family is what says *do not draw a seam here*. Brick beside soil is two families and is
/// exactly the seam the bands exist for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Soil,
    Sand,
    Stone,
    Sandstone,
    Brick,
    Timber,
}

impl Look {
    fn family(self) -> Family {
        match self {
            Look::Grass | Look::Dirt => Family::Soil,
            Look::Beach | Look::Packed => Family::Sand,
            Look::Stone => Family::Stone,
            Look::Sandstone => Family::Sandstone,
            Look::Brick | Look::Masonry => Family::Brick,
            Look::Wood | Look::Timber => Family::Timber,
        }
    }

    /// The row of [`BLEND`] that carries this look across a neighbouring tile's edge.
    ///
    /// One row per *look* rather than per family, because the band is cut from the look's own
    /// cell: the green crown of a lawn and the brown body under it are one material and two
    /// pictures, and a seam beside a surface line has to carry the surface's colours.
    fn index(self) -> usize {
        match self {
            Look::Grass => 0,
            Look::Dirt => 1,
            Look::Stone => 2,
            Look::Beach => 3,
            Look::Packed => 4,
            Look::Sandstone => 5,
            Look::Brick => 6,
            Look::Masonry => 7,
            Look::Wood => 8,
            Look::Timber => 9,
        }
    }
}

/// A solid tile's sprite, the soil sprite it falls back to, and what it looks like.
#[derive(Clone, Copy, Debug)]
struct Tiling {
    /// The name the atlas is asked for.
    name: &'static str,
    /// The same tile in soil, for a tileset that has no sand.
    soil: &'static str,
    /// What the tile is made of, for the seams around it.
    look: Look,
}

/// A solid tile's own autotile, the soil one it falls back to, and its material.
///
/// `name` and `soil` differ only for a sand tile, and only because the sand family is
/// optional art — a tileset drawn before sand existed has no `sand_*`, and a beach in it is
/// drawn as soil rather than refusing to start. That is the same bargain the tile variants
/// and the props past `pebble` make, and the reason this carries both names rather than one:
/// the caller that has an atlas can use the first and the caller that must not fail uses the
/// second.
///
/// The look is decided here rather than in a second function on purpose. Whether a tile is
/// drawn as a crown or as a buried body is the same question as which sprite it gets, and two
/// answers to it in two places is how a seam ends up blending with a material the tile beside
/// it is not drawn as.
fn autotile(level: &Level, tx: i32, ty: i32) -> Tiling {
    let mask = ground_mask(level, tx, ty);
    let surface = mask & 0b0001 != 0;
    let deep = !surface && depth_under_surface(level, tx, ty) >= ROCK_DEPTH;
    let soil = soil_name(mask, deep);
    // Brittleness is stripped off before anything below asks a question: every row of this match
    // is about what a tile is *made of*, and a cracked beach is a beach. The one caller that cares
    // that it is cracked draws the crack over this result.
    let tile = level.tile(tx, ty).unwrap_or(Tile::Solid).base_material();
    let (look, name) = match (tile, surface, deep) {
        (Tile::Sand, true, _) => (Look::Beach, SAND[mask as usize]),
        (Tile::Sand, false, true) => (Look::Sandstone, SANDSTONE[mask as usize]),
        (Tile::Sand, false, false) => (Look::Packed, SAND[mask as usize]),
        (Tile::Brick, true, _) => (Look::Brick, BRICK[mask as usize]),
        (Tile::Brick, false, true) => (Look::Masonry, MASONRY[mask as usize]),
        (Tile::Brick, false, false) => (Look::Brick, BRICK[mask as usize]),
        (Tile::Plank, true, _) => (Look::Wood, WOOD[mask as usize]),
        (Tile::Plank, false, true) => (Look::Timber, TIMBER[mask as usize]),
        (Tile::Plank, false, false) => (Look::Wood, WOOD[mask as usize]),
        (_, true, _) => (Look::Grass, soil),
        (_, false, true) => (Look::Stone, soil),
        (_, false, false) => (Look::Dirt, soil),
    };
    Tiling { name, soil, look }
}

/// The look of the solid tile at `(tx, ty)`, or `None` where there is no ground.
fn look_at(level: &Level, tx: i32, ty: i32) -> Option<Look> {
    solid_at(level, tx, ty).then(|| autotile(level, tx, ty).look)
}

/// The sides of a tile, in the order the blend names list them: left, right, above, below,
/// each with the offset that reaches the tile on that side.
const SIDES: [(char, i32, i32); 4] = [('l', -1, 0), ('r', 1, 0), ('t', 0, -1), ('b', 0, 1)];

/// The seam sprite for each look and side: the band of *that* material which the view draws
/// over the tile next to it, so that each material creeps a few pixels into the other at a
/// seam.
///
/// Two entries are empty, and no level can ask for them: a surface tile has sky above it, so
/// no solid tile ever has a crown *below* it — which is `blend_grass_b` and `blend_beach_b`,
/// the two bands the atlas does not ship. Empty rather than absent so the lookup stays one
/// index, and the assertion below is what keeps the gaps to exactly those two.
///
/// Brick and wood are crowns and still need their `_b` band, because a look is a *depth*: a
/// tile two courses under a brick coping is drawn as `brick` too, so it can have another
/// material below it. Grass and dirt are two looks for exactly that reason.
const BLEND: [[&str; 4]; 10] = [
    // left, right, above, below
    ["blend_grass_l", "blend_grass_r", "blend_grass_t", ""],
    [
        "blend_dirt_l",
        "blend_dirt_r",
        "blend_dirt_t",
        "blend_dirt_b",
    ],
    [
        "blend_stone_l",
        "blend_stone_r",
        "blend_stone_t",
        "blend_stone_b",
    ],
    ["blend_beach_l", "blend_beach_r", "blend_beach_t", ""],
    [
        "blend_packed_l",
        "blend_packed_r",
        "blend_packed_t",
        "blend_packed_b",
    ],
    [
        "blend_sandstone_l",
        "blend_sandstone_r",
        "blend_sandstone_t",
        "blend_sandstone_b",
    ],
    [
        "blend_brick_l",
        "blend_brick_r",
        "blend_brick_t",
        "blend_brick_b",
    ],
    [
        "blend_masonry_l",
        "blend_masonry_r",
        "blend_masonry_t",
        "blend_masonry_b",
    ],
    [
        "blend_wood_l",
        "blend_wood_r",
        "blend_wood_t",
        "blend_wood_b",
    ],
    [
        "blend_timber_l",
        "blend_timber_r",
        "blend_timber_t",
        "blend_timber_b",
    ],
];

/// The plain name for a solid tile: what the level says it is made of, the mask, and how far
/// under the surface it sits.
///
/// Test-only because a frame does not ask one tile at a time: the tile pass computes the
/// [`Tiling`] once per tile and asks the *neighbours* about their material as well, and the
/// tests want the same answer asked tile by tile. Both go through [`autotile`], so the two
/// cannot drift.
#[cfg(test)]
pub fn ground_name(level: &Level, tx: i32, ty: i32) -> &'static str {
    autotile(level, tx, ty).name
}

/// Which decoration a tile gets: 0 for the plain tile, 1 and 2 for its two variants.
///
/// Two tiles in three stay plain on purpose. A level where every second floor tile has
/// flowers on it reads as wallpaper; the point of a variant is that the eye notices it,
/// which it cannot do if it is everywhere. The salt keeps this hash from correlating with
/// the one that scatters scenery over the same tiles.
fn variant_index(tx: i32, ty: i32) -> usize {
    (tile_hash(tx, ty, 5) % 3) as usize
}

/// A stable hash of a tile's coordinates, for everything the view scatters.
///
/// The view draws a level identically every frame and after every regeneration, so
/// "randomly placed" scenery cannot come from an RNG the renderer owns: it comes from
/// this, which is a pure function of where the tile is.
pub fn tile_hash(tx: i32, ty: i32, salt: u32) -> u32 {
    let mut n = (tx as u32).wrapping_mul(0x9E37_79B9)
        ^ (ty as u32).wrapping_mul(0x85EB_CA6B)
        ^ salt.wrapping_mul(0xC2B2_AE35);
    n ^= n >> 15;
    n = n.wrapping_mul(0x2545_F491);
    n ^= n >> 13;
    n
}

/// A hash as a fraction in [0, 1), taken from the hash's *high* slice.
///
/// The same lesson [`scenery_sprite`] learned about `% 100`: the low bits of
/// [`tile_hash`] move in step along a run of neighbouring indices, so a fraction
/// built from them (`hash % 1000 / 1000`) clusters — neighbouring clouds came out
/// the same size and the same height because their indices are neighbours. The
/// slice above bit 8 is the mixed part of the hash, so consecutive indices give
/// unrelated fractions, which is what a *continuous* placement (a cloud's size,
/// its altitude, its jitter along the sky) needs.
fn hash_frac(tx: i32, ty: i32, salt: u32) -> f32 {
    ((tile_hash(tx, ty, salt) >> 8) % 1000) as f32 / 1000.0
}

/// The sprite for a solid tile: autotile, rock or soil or sand, and its variant if there is
/// one.
///
/// A material this tileset does not draw falls back to soil before the variant is chosen, so
/// a beach in a tileset without sand is a plain floor rather than a missing-sprite panic.
///
/// Test-only for the same reason as [`ground_name`].
#[cfg(test)]
pub fn ground_sprite(sprites: &Sprites, level: &Level, tx: i32, ty: i32) -> Rect {
    tile_sprite(sprites, autotile(level, tx, ty), tx, ty)
}

/// The sprite for a tile whose tiling is already known, so a frame that needs both the art
/// and the material around it computes the mask once.
fn tile_sprite(sprites: &Sprites, tiling: Tiling, tx: i32, ty: i32) -> Rect {
    let plain = if sprites.try_get(tiling.name).is_some() {
        tiling.name
    } else {
        // Only a material this tileset does not draw gets this far, and only once per
        // tile per frame: the soil tile with the same mask and the same depth.
        tiling.soil
    };
    let options = VARIANTS
        .iter()
        .find(|(base, _)| *base == plain)
        .map(|(_, options)| *options);
    match (variant_index(tx, ty), options) {
        (0, _) | (_, None) => sprites.get(plain),
        (n, Some(options)) => sprites.first_of(&[options[n - 1], plain]),
    }
}

/// The crack drawn over a brittle tile: its material's overlay, or `None` for a sound tile.
///
/// **Optional art, on the same terms as the sand family and the tile variants.** A tileset drawn
/// before cracked ground existed draws a brittle tile as the material it is made of, and the only
/// thing lost is the warning that it is about to go — which is a real loss, and still not a reason
/// for somebody else's atlas to stop starting. The palette's cracked brushes lean on the same
/// four sprites, so a tileset without them paints a cracked floor the colour of its material.
///
/// One overlay per **material** rather than one crack for all four, because a crack is the
/// material's own darkest tone: a grey fissure across a beach is the kind of detail that makes the
/// eye stop reading the tile as sand, and the layers here already exist for exactly this reason
/// (`_with_detail(ground(1), _crack)` in `tools/art/terrain.py`).
fn cracked_overlay(sprites: &Sprites, tile: Tile) -> Option<Rect> {
    let name = match tile {
        Tile::Cracked => "cracked_ground",
        Tile::CrackedSand => "cracked_sand",
        Tile::CrackedBrick => "cracked_brick",
        Tile::CrackedPlank => "cracked_wood",
        _ => return None,
    };
    sprites.try_get(name)
}

/// Draw the seams where this tile's material meets another one.
///
/// For each of the tile's four sides, the **neighbouring** material's band is drawn over
/// this tile: a beach beside a lawn puts sand on the lawn's edge and grass on the beach's,
/// and the two crossing a few pixels into each other is the transition. Nothing here knows
/// which pair of materials it is looking at — that is what makes a material per band and
/// not a band per pair, and why a fifth material costs four sprites and no new code.
///
/// Two rules are worth stating because they are the whole of the behaviour:
///
/// * a band is drawn only when the neighbour is a *different family*. Grass over the dirt
///   beneath it is one material at two depths, and the crown's ragged last row already
///   joins them; blending there would smear a surface line that is deliberately exact.
/// * the bands lie in the tile's own cell, so a seam can never bleed into a tile that is
///   not part of the seam, and a tile with no different neighbour costs nothing.
fn draw_seams(sheet: Sheet<'_>, level: &Level, tx: i32, ty: i32, rect: Rect, mine: Look) {
    for (index, (_side, dx, dy)) in SIDES.iter().enumerate() {
        let Some(theirs) = look_at(level, tx + dx, ty + dy) else {
            continue;
        };
        if theirs.family() == mine.family() || theirs.index() >= BLEND.len() {
            continue;
        }
        let name = BLEND[theirs.index()][index];
        if name.is_empty() {
            continue;
        }
        // Optional art, like the tile variants: a tileset drawn before the seams existed
        // has no bands and draws a hard edge, which is what every tileset did until now.
        if let Some(uv) = sheet.sprites.try_get(name) {
            sheet.image(rect, uv);
        }
    }
}

/// A prop to draw: which sprite, and the cell it goes in.
///
/// The two are one decision, so they travel together. A prop is drawn in the **open** tile
/// above the ground it is planted on, and that is not a matter of taste: a solid tile is
/// opaque by contract (`tools/art/terrain.py` refuses a transparent pixel in one), so a
/// prop drawn in the ground's own cell is painted over by the terrain pass a few lines
/// later. Every bush, rock, mushroom and flower in the game was invisible until this cell
/// was named — the art was right, and the tests all asked *which* sprite a tile wants,
/// which is a question that cannot see where the answer lands.
pub struct Prop {
    /// The cell to draw in: the open tile above the surface tile this prop belongs to, or
    /// — for [`hanging_sprite`] — the open tile below the one it hangs from.
    pub cell: (i32, i32),
    /// The sprite, in the atlas.
    pub sprite: Rect,
    /// The sprite for the cell **above** `cell`, for a prop that is two tiles tall.
    ///
    /// One optional upper half rather than a list of parts, because exactly one thing in
    /// the game is taller than a tile and it is the tree. A general stack would be a data
    /// structure with one user and a second cell to get wrong at every call site, and the
    /// day three-tile trees arrive the field that has to change is this one, three lines
    /// from where the tree is built.
    pub crown: Option<Rect>,
    /// How big the prop is drawn, in tiles: one cell of art, drawn this many tiles tall,
    /// standing on the bottom of [`Prop::cell`].
    ///
    /// The cell is a *slot* rather than a size, and the number belongs to the kind of prop
    /// ([`prop_size`]). Every prop used to be drawn at exactly one cell — a cell is as tall
    /// as an agent — so a patch of daisies was as wide as a tree's canopy and a mushroom
    /// stood as tall as a stump: the whole vocabulary was one size, and nothing on a lawn
    /// was bigger than anything else on it. A tree is two cells drawn at [`TREE_SIZE`], so
    /// it stands three tiles and a bit with a canopy a tile and a half across, and the
    /// daisies beside it are a third of a tile wide.
    pub size: f32,
}

/// The scenery for a surface tile: its sprite, and the cell above the tile it stands on.
///
/// Scenery is dressing: the level format has no concept of a bush, and the view must
/// therefore never put one anywhere the physics disagrees with — every prop here is drawn
/// on a tile whose *own* surface is solid, standing in the open air above it, where an
/// agent running past will pass in front of it.
pub fn scenery_sprite(
    sprites: &Sprites,
    level: &Level,
    tx: i32,
    ty: i32,
    clock: f32,
) -> Option<Prop> {
    if !solid_at(level, tx, ty) || !ground_mask(level, tx, ty) & 0b0001 != 0 {
        return None;
    }
    // A prop needs room to stand up in: a surface tile with a solid tile above it is a
    // slot in a wall rather than a surface, and a surface tile on the level's own top row
    // has no cell above it to stand in.
    if ty == 0 || solid_at(level, tx, ty - 1) {
        return None;
    }
    let sway = ((clock * 1.6) as i32).rem_euclid(2) == 1;
    // The roll is the **top** slice of the hash rather than the whole of it mod a hundred.
    // A floor is a horizontal run of tiles at one `ty`, and the low bits of this hash move
    // in step along a run: `% 100` of the whole value never once lands in 15..=17 anywhere
    // in the shipped pack, and 15..=17 is the flowers' band — three hundred and fifty-three
    // surface tiles, and not one daisy, whatever the table said. The high bits
    // are the mixed ones (measured over the same pack: the low slice reaches 59 of the
    // hundred buckets and this slice 68), so a range of the table is a range of the level.
    let roll = (tile_hash(tx, ty, 23) >> 18) % 100;
    // Each prop names what to draw *instead* when this tileset does not have it: the
    // props past `pebble` are optional art, like the tile variants, and a fallback is the
    // same kind of thing — a log a tileset lacks is a rock, reeds are a tuft, the white
    // daisies are the gold ones — so a tileset drawn before they existed still dresses a
    // floor rather than refusing to start.
    //
    // The table is the *rhythm* of a dressed floor, and it is a table of ranges for that
    // reason: two in a hundred tiles get a berry bush, five get the plain one, and the
    // eye finds a lawn rather than a prop list. Every shape added to the vocabulary takes
    // its slots from the same hundred, so adding a bush does not add a bush to every
    // level — it makes the same number of bushes more various.
    let (name, instead) = match roll {
        0..=1 => two_frames(sway, "bush_b", "bush_a"),
        2..=3 => two_frames(sway, "bush_d", "bush_c"),
        4..=5 => two_frames(sway, "bush_f", "bush_e"),
        6..=9 => two_frames(sway, "tuft_b", "tuft_a"),
        10..=12 => two_frames(sway, "tuft_d", "tuft_c"),
        13..=14 => two_frames(sway, "tuft_f", "tuft_e"),
        // Three slots, one per colour rather than three for the group: a flower patch is
        // the one prop whose *kind* is picked by a second hash, and a band of two slots
        // split three ways left the white daisies off a level entirely.
        15..=17 => match tile_hash(tx, ty, 29) % 3 {
            0 => ("flowers", "flowers"),
            1 => ("flowers_pink", "flowers"),
            _ => ("flowers_white", "flowers"),
        },
        18 => ("rock", "rock"),
        19 => ("mushroom", "mushroom"),
        20..=21 => ("pebble", "pebble"),
        22..=23 => ("log", "rock"),
        24..=25 => ("stump", "rock"),
        26..=27 => {
            if sway {
                ("reeds_b", "tuft_b")
            } else {
                ("reeds_a", "tuft_a")
            }
        }
        28..=29 => ("mushroom_pair", "mushroom"),
        // The rare one, and the only prop that is not one cell: see `tree_prop`.
        30..=32 => return tree_prop(sprites, level, tx, ty),
        _ => return None,
    };
    Some(Prop {
        cell: (tx, ty - 1),
        sprite: sprites.first_of(&[name, fallback_for(name, instead)]),
        crown: None,
        size: prop_size(name),
    })
}

/// How big a tree is drawn, in tiles per cell of its art.
///
/// Above one on purpose, and it is the only prop that is: a tree two cells tall at one
/// tile each is a shrub on a stick next to a daisy patch, and the picture has to say which
/// of the two things on a lawn is a tree. See [`prop_size`].
const TREE_SIZE: f32 = 1.5;

/// How many cells of air above the ground a tree needs before it is a tree rather than a bush.
///
/// Its own height in cells: a tree is two cells of art at [`TREE_SIZE`] each, so it stands
/// three whole tiles and reaches the top of the third cell above the ground it is planted on.
/// Two cells was the right answer while a tree was two tiles tall, and the rule that keeps it
/// from being drawn through a ceiling is the same rule at a new number.
const TREE_CELLS: i32 = 3;

/// How big a piece of scenery is drawn, in tiles, measured up from the bottom of its cell.
///
/// One table rather than a number at every call site, because the sizes are a *scale* and a
/// scale is only readable next to its neighbours: what makes a lawn look inhabited is the
/// ratios — a tree four times a tuft of grass, a daisy patch half a bush — and those are
/// invisible if each prop carries its own number.
///
/// The kinds are named by the first word of the sprite, so the family grows without this:
/// any new colour of daisy is `flowers_*` and is drawn like the others.
fn prop_size(name: &str) -> f32 {
    match name.split('_').next().unwrap_or(name) {
        // The one prop that is taller than a tile, and the reason [`Prop::crown`] and this
        // field exist. Drawn at more than a cell each half is a compromise worth naming: a
        // tree a real 2x2 cells of art would need a redraw of the atlas, and the alternative
        // — a tree one tile wide and two tall — is a shrub on a stick, which is what this
        // used to be. So the two cells are drawn a cell and a half tall each, which is the
        // same enlargement the pines have always had (a 16px silhouette drawn up to 1.6
        // tiles) and the same pixel size a distant tree gets.
        "tree" => TREE_SIZE,
        // A shrub is waist-high on an agent: a bush that filled its cell was a tree with no
        // trunk, and there is no reading of the picture in which that is what it is.
        "bush" => 0.75,
        // Grass, flowers, mushrooms, pebbles: the dressing you are meant to walk past. Half a
        // tile is the size at which a tuft reads as a tuft rather than as a small bush.
        "tuft" => 0.7,
        "flowers" => 0.6,
        "mushroom" => 0.7,
        "pebble" => 0.75,
        // The heavier dressing: a rock and a fallen log are objects, not decoration, but they
        // are still not trees.
        "rock" => 0.85,
        "log" | "stump" | "crate" | "crystal" => 0.9,
        "lantern" | "signpost" => 1.0,
        "banner" => 1.15,
        // A reed bed and a vine both want their full cell: one is a stand of stems and the
        // other hangs from a ceiling to the frame below it, and a half-size vine would leave
        // a gap under the tile it is supposed to be growing from.
        _ => 1.0,
    }
}

/// The sprite and its substitute for this frame of a two-frame prop.
///
/// `b` is the second frame, `a` the first: the frame *letters* are the sway, and `a` is
/// also what a tileset without the shape at all falls back to — a bush is a bush whichever
/// way the air is moving.
fn two_frames(sway: bool, b: &'static str, a: &'static str) -> (&'static str, &'static str) {
    if sway {
        (b, a)
    } else {
        (a, a)
    }
}

/// What a prop falls back to when this tileset does not have it: its own family's first
/// shape, then nothing.
///
/// The mapping is deliberately by *family* rather than per name: every bush shape falls
/// back to the plain bush, every grass shape to the plain tuft, and a tileset drawn before
/// the third bush existed still draws a bush. Anything with no plainer relative (the
/// pines, the clouds, the butterflies) names itself, and a tileset without it simply has
/// no prop there.
fn fallback_for(name: &'static str, instead: &'static str) -> &'static str {
    match name {
        "bush_c" | "bush_d" | "bush_e" | "bush_f" => "bush_a",
        "tuft_c" | "tuft_d" | "tuft_e" | "tuft_f" => "tuft_a",
        _ => instead,
    }
}

/// A tree: two cells, or a bush when the level has no room for one.
///
/// The one prop in the game that is bigger than a tile, and the reason [`Prop::crown`]
/// exists. A bush needs one open cell above the ground it stands on; a tree needs two, and
/// the second one has to be *empty* rather than merely not solid — `tile` rather than
/// `is_solid`, the same care the vines take, because a tree growing through a spike is a
/// decoration lying about a hazard. Where there is no room the place gets a bush instead:
/// what must never happen is the trunk drawn anyway with its canopy inside the ceiling
/// above it, which is a tree growing through rock.
fn tree_prop(sprites: &Sprites, level: &Level, tx: i32, ty: i32) -> Option<Prop> {
    let bush = || Prop {
        cell: (tx, ty - 1),
        sprite: sprites.first_of(&["bush_c", "bush_a"]),
        crown: None,
        size: prop_size("bush"),
    };
    for above in 2..=TREE_CELLS {
        if level.tile(tx, ty - above) != Some(Tile::Empty) {
            return Some(bush());
        }
    }
    let (base, top) = if tile_hash(tx, ty, 31) % 2 == 0 {
        ("tree_a_base", "tree_a_top")
    } else {
        ("tree_b_base", "tree_b_top")
    };
    match (sprites.try_get(base), sprites.try_get(top)) {
        // Both halves or neither: a trunk with no canopy is not a tree a tileset is
        // missing the top of, it is a mistake, and a bush is what belongs there.
        (Some(lower), Some(upper)) => Some(Prop {
            cell: (tx, ty - 1),
            sprite: lower,
            crown: Some(upper),
            size: TREE_SIZE,
        }),
        _ => Some(bush()),
    }
}

/// The scenery that hangs: a vine in the open tile under a solid one.
///
/// The ceiling half of [`scenery_sprite`], and the same care about *where* it may be
/// drawn, read upside down: the tile above has to be the thing it hangs from, and this
/// cell has to be empty *and in the level* — which is what keeps a vine from dangling
/// under the bottom edge of the world. One column in six gets one: a vine on every column
/// of an overhang is a curtain rather than vegetation.
pub fn hanging_sprite(
    sprites: &Sprites,
    level: &Level,
    tx: i32,
    ty: i32,
    clock: f32,
) -> Option<Prop> {
    // [`Level::tile`] rather than `is_solid`: this cell has to be *empty*, which is a
    // stronger claim than "nothing solid here". A vine through a spike would be a
    // decoration that lies about a hazard.
    if level.tile(tx, ty) != Some(Tile::Empty) || !solid_at(level, tx, ty - 1) {
        return None;
    }
    if tile_hash(tx, ty, 13) % 6 != 0 {
        return None;
    }
    let sway = ((clock * 1.6) as i32).rem_euclid(2) == 1;
    // No vines at all in this tileset means no hanging scenery, rather than a substitute
    // hanging off nothing. The fallback is only for the *frame* being missing.
    let (first, second) = if sway {
        ("vine_b", "vine_a")
    } else {
        ("vine_a", "vine_b")
    };
    let sprite = sprites.try_get(first).or_else(|| sprites.try_get(second))?;
    Some(Prop {
        cell: (tx, ty),
        sprite,
        // A vine is one cell: nothing hangs off the bottom of one.
        crown: None,
        size: 1.0,
    })
}

/// The prop for a **placed** piece of scenery: the sprite the author named, at the size
/// that sprite's family is drawn at.
///
/// The one difference from [`scenery_sprite`] is that a `*_base` sprite also carries its
/// canopy, so a placed tree stands as tall as a scattered one. A name the tileset does not
/// have draws nothing rather than a substitute: a placed sprite is a decision the author
/// made, and quietly drawing a different plant would be the editor lying about what it
/// saved.
fn placed_prop(sprites: &Sprites, s: &sim::Scenery) -> Option<Prop> {
    let sprite = sprites.try_get(&s.sprite)?;
    let crown = s
        .sprite
        .strip_suffix("_base")
        .and_then(|stem| sprites.try_get(&format!("{stem}_top")));
    Some(Prop {
        cell: (s.x, s.y),
        sprite,
        crown,
        size: prop_size(&s.sprite),
    })
}

/// One prop, drawn at its own size with its feet on the bottom of its own cell.
///
/// `size` rather than the cell is what the art is measured in, and the bottom of the cell is
/// the ground line: a prop stands on the tile it belongs to with air above it, whatever its
/// picture is worth. So a third-of-a-tile daisy patch and a three-tile tree are placed by
/// the same rule, and nothing here has to know which one it is holding.
///
/// The canopy is the trunk's own rectangle lifted by exactly the height the trunk was drawn
/// at, and not the cell above: a tree at [`TREE_SIZE`] grows past the cell it stands in, and
/// the two halves only meet if the one is placed off the other. Cell-relative halves were
/// right while every prop was one cell and are the first thing to break when one is not.
fn draw_prop(sheet: Sheet<'_>, prop: &Prop) {
    let cell = sheet.tile(prop.cell.0, prop.cell.1);
    // Whole points, like every other sprite in the view: a prop drawn at 22.4 points a side
    // has rows of three pixels and rows of two, which shimmers as the camera moves.
    let side = (cell.height() * prop.size).round().max(2.0);
    let foot = |bottom: f32| {
        Rect::from_min_size(
            Pos2::new((cell.center().x - side * 0.5).round(), bottom - side),
            Vec2::splat(side),
        )
    };
    sheet.image(foot(cell.bottom()), prop.sprite);
    if let Some(crown) = prop.crown {
        sheet.image(foot(cell.bottom() - side), crown);
    }
}

/// One butterfly per this many tiles of *standing room*, on average.
///
/// Sparse on purpose. The pool is not the level's air any more but the open tiles with
/// ground directly under them — the strip the grass line is at, which is sixty-odd tiles in
/// a level of nine hundred — so the rate is much higher and the count is the one it always
/// was: three or four animals in a level, twenty over the pack, one or two in a follow
/// frame. Enough that a level is inhabited, few enough that each one is a thing you notice.
const BUTTERFLY_PER_GROUND: u32 = 15;

/// Wings per second. Fast, because it is small: a butterfly's beat is a blur at arm's
/// length and a bird's is not, which is half of what tells the two apart at a glance.
const BUTTERFLY_HZ: f32 = 7.0;

/// How far a butterfly wanders from the tile it belongs to, how big it is drawn, and how far
/// above the ground it flies — all in tiles.
///
/// A butterfly is a fifth of a tile across and it lives in the air just over the grass: the
/// picture that made these read as birds was a *whole tile* wide wandering a third of a tile
/// around the middle of any empty tile in the level, which put them ten tiles up in the sky
/// at the size of a shrub. The drift is small enough that the rect cannot reach out of its
/// own cell and through the ground it is standing on.
const BUTTERFLY_DRIFT: f32 = 0.16;
const BUTTERFLY_SIZE: f32 = 0.45;
const BUTTERFLY_LIFT: f32 = 0.10;

/// The ambient layer: the butterflies in a level's air just above its ground.
///
/// The one thing in the view that is not *placed*. A bush belongs to the tile it stands
/// on and is asked for by that tile; a butterfly belongs to nothing, which is why this is
/// a layer rather than a branch of [`scenery_sprite`], and why the three rules it does have
/// are about air rather than about a surface: it is drawn only in a tile the level calls
/// **empty** (a creature inside a wall is the same lie a prop inside one would be), only
/// where the tile above is not solid, so a butterfly is not squeezed into a one-tile gap
/// between two platforms, and only where the tile **below** is solid — which is what keeps
/// them at the grass line instead of all over the sky, the way they were drawn when any
/// empty tile would do.
///
/// The motion is a pure function of the clock, the tile and the tile's hash: an ellipse
/// around the tile's centre, and the wingbeat off the same clock. Nothing here is random,
/// so a paused frame is always the same frame and a replay at a given timestamp has the
/// same butterflies in it as the recording did.
pub fn draw_ambient(sheet: Sheet<'_>, level: &Level, clock: f32) {
    let (tx0, tx1, ty0, ty1) = sheet.camera.visible_tiles(level, sheet.viewport);
    let scale = sheet.camera.scale();
    let frame = ((clock * BUTTERFLY_HZ) as i32).rem_euclid(2);
    let side = (scale * BUTTERFLY_SIZE).round().max(2.0);
    for ty in ty0..ty1 {
        for tx in tx0..tx1 {
            if level.tile(tx, ty) != Some(Tile::Empty) || solid_at(level, tx, ty - 1) {
                continue;
            }
            if !solid_at(level, tx, ty + 1) {
                continue;
            }
            if tile_hash(tx, ty, 47) % BUTTERFLY_PER_GROUND != 0 {
                continue;
            }
            let shape = tile_hash(tx, ty, 59) % 2;
            let name = match (shape, frame) {
                (0, 0) => "butterfly_a_1",
                (0, _) => "butterfly_a_2",
                (1, 0) => "butterfly_b_1",
                _ => "butterfly_b_2",
            };
            // Optional art: a tileset drawn before the butterflies existed has no ambient
            // life, which is a quiet level rather than a panic.
            let Some(sprite) = sheet.sprites.try_get(name) else {
                continue;
            };
            // A slow ellipse just above the ground, out of phase per tile, so a frame with
            // three butterflies in it does not look like one butterfly three times. The
            // vertical part is smaller than the horizontal one: a butterfly hovers, and the
            // rect is small enough now that a bob of the old amplitude would put it through
            // the ground it is flying over.
            let phase = (tile_hash(tx, ty, 61) % 1024) as f32 / 1024.0;
            let angle = clock * 0.9 + phase * std::f32::consts::TAU;
            let cell = sheet.tile(tx, ty);
            let drift = Vec2::new(
                angle.cos() * scale * BUTTERFLY_DRIFT,
                (angle * 1.7).sin() * scale * BUTTERFLY_DRIFT * 0.5,
            );
            // Bottom-anchored like a prop, because that is what it does: the feet of the
            // rect are where the creature is, lifted a hair off the ground.
            let foot = cell.bottom() - scale * BUTTERFLY_LIFT + drift.y;
            sheet.image(
                Rect::from_min_size(
                    Pos2::new(
                        (cell.center().x - side * 0.5 + drift.x).round(),
                        (foot - side).round(),
                    ),
                    Vec2::splat(side),
                ),
                sprite,
            );
        }
    }
}

/// The two tiles a goal box covers, upper (flag) and lower (pole).
pub fn goal_cells(goal: sim::Rect) -> ((i32, i32), (i32, i32)) {
    let top = (goal.x.floor() as i32, goal.y.floor() as i32);
    (top, (top.0, top.1 + 1))
}

/// The background behind a level: the editor's graph paper, or the game's sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Backdrop {
    /// A checkerboard two tiles wide, so a level being *drawn* has something to read
    /// its scale against.
    #[default]
    Checker,
    /// Sky, hills and clouds: what the game looks like when nobody is editing.
    Sky,
}

/// The world line the sky's horizon sits on: the level's own ground line.
///
/// The top of the deepest run of solid rows — the surface the level is walked on, which is
/// neither the bottom row's own top nor the level's bottom edge. A level whose floor is two
/// rows thick has its ground line at the top of them, and that is the line the ground plane
/// beside the level has to meet, or the level reads as a plateau standing on the world
/// instead of as part of it. A level with no solid tile in it at all puts the line at its
/// floor, which is the only place left.
///
/// **The whole sky is placed off this line, and none of it off the window.** Every layer used
/// to be a fraction of the viewport's height — a horizon at 0.62 of it, the sun at 0.10 —
/// which bakes in one camera looking at one level and moves everything the moment either
/// changes: a longer level, a taller window, a closer zoom, or a camera following an agent
/// that has fallen, and the ground plane no longer meets the ground, the sun is behind a hill
/// or under the floor, and the level floats in a picture of a landscape painted for another
/// frame. Anchored to the world they sit where they are whatever the camera is doing, and
/// what shows above the ground line is the sky of the level rather than of the window.
fn sky_ground(level: &Level) -> f32 {
    let solid_row = |y: i32| (0..level.width).any(|x| solid_at(level, x, y));
    let mut y = level.height - 1;
    while y >= 0 && !solid_row(y) {
        y -= 1;
    }
    if y < 0 {
        return level.height as f32;
    }
    while y > 0 && solid_row(y - 1) {
        y -= 1;
    }
    y as f32
}

/// Screen y of the level's ground line: what every layer of the sky is placed off.
fn sky_line(camera: &Camera, level: &Level, viewport: Rect) -> f32 {
    camera.to_screen(0.0, sky_ground(level), viewport).y
}

/// Sky gradient, then either the checkerboard or the parallax landscape.
pub fn draw_background(sheet: Sheet<'_>, level: &Level, backdrop: Backdrop, clock: f32) {
    let (painter, viewport, camera) = (sheet.painter, sheet.viewport, sheet.camera);

    match backdrop {
        // The editor's ground: one flat surface behind the whole document, with the tile grid drawn
        // on it a tile at a time (`draw_level`). It used to be a two-tile checkerboard over the sky
        // gradient, and that is exactly the wrong picture — a checkerboard is what an image tool
        // paints where there is *nothing*, and every cell of it was shouting "nothing here" about
        // the one surface in the window that is full of a person's work. What actually tells an eye
        // where the tiles are is the grid's own lines, which were being drawn all along; and a flat
        // ground hands the level's art the whole of the contrast, which is what a canvas is for.
        Backdrop::Checker => {
            painter.rect_filled(viewport, egui::CornerRadius::ZERO, EDITOR_SURFACE);
        }
        Backdrop::Sky => {
            // Sky, top to bottom of the window. Sampled into a two-triangle gradient.
            let mut mesh = Mesh::default();
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(viewport.left_top(), SKY_TOP);
            mesh.colored_vertex(viewport.right_top(), SKY_TOP);
            mesh.colored_vertex(viewport.right_bottom(), SKY_BOTTOM);
            mesh.colored_vertex(viewport.left_bottom(), SKY_BOTTOM);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
            painter.add(mesh);
            // Furthest first: the sun, the two cloud layers, the far hills, the
            // tree-line standing on their ridge, then the near hills and *their*
            // tree-line in front of all of it. The order is the depth order, and
            // the parallax factors increase in exactly this order — sun 0.04,
            // far clouds 0.08, near clouds 0.17, far hills 0.20, near hills
            // 0.38, the level itself 1.0 — which is what makes the layers read
            // as distance rather than as six pictures stacked up. A layer that
            // is drawn behind another and moves *faster* than it (which is what
            // the clouds used to do, at 0.30 and 0.55 against the hills' 0.20
            // and 0.38) tells the eye two opposite stories at once.
            //
            // The clouds moved in front of the hills to behind them when they were given an
            // altitude: hung off the top of the window they never overlapped a hill, and at
            // their own height in the sky they do, and a cloud a hill can stand in front of
            // is a cloud that is further away than the hill rather than a cut-out pasted
            // over it.
            draw_sun(painter, viewport, camera, level);
            for layer in CLOUD_LAYERS {
                draw_clouds(sheet, level, clock, layer);
            }
            // A tree-line is handed the band it stands on rather than the numbers to
            // rebuild it: a tree planted on a ridge rides that ridge, and two copies of a
            // hill's geometry is one copy after the first edit. Both bands get one:
            // the near hills stood bare in front of a forested range for as long as
            // only the far band had a line, which read as a nearer hill that was
            // somehow balder than the one behind it.
            let far = HillBand::new(camera, level, viewport, FAR_HILLS_PARALLAX, FAR_DOMES, 11);
            far.draw(painter, HILL_FAR);
            draw_treeline(sheet, &far, &RANKS);
            let near = HillBand::new(camera, level, viewport, NEAR_HILLS_PARALLAX, NEAR_DOMES, 17);
            near.draw(painter, HILL_NEAR);
            draw_treeline(sheet, &near, &NEAR_RANKS);
            draw_sunbeams(painter, viewport, camera, level, clock);
        }
    }
}

/// Diagonal volumetric sunbeams cast across the sky from the sun's position.
fn draw_sunbeams(painter: &Painter, viewport: Rect, camera: &Camera, level: &Level, clock: f32) {
    let shift = camera.centre.x * SUN_PARALLAX * camera.scale();
    let sun_pos = Pos2::new(
        viewport.left() + viewport.width() * 0.76 - shift,
        sky_line(camera, level, viewport) - SUN_ALTITUDE * camera.scale(),
    );
    let mut mesh = Mesh::default();
    let rays = [
        (-0.55, 0.10, 14_u8),
        (-0.28, 0.14, 18),
        (0.02, 0.09, 12),
    ];
    let length = viewport.height() * 1.15;
    for (i, (angle_offset, spread, base_alpha)) in rays.iter().enumerate() {
        let sway = (clock * 0.4 + i as f32 * 1.7).sin() * 0.03;
        let a0 = std::f32::consts::FRAC_PI_2 + 0.32 + angle_offset + sway;
        let a1 = a0 + spread;
        let p1 = Pos2::new(sun_pos.x + a0.cos() * length, sun_pos.y + a0.sin() * length);
        let p2 = Pos2::new(sun_pos.x + a1.cos() * length, sun_pos.y + a1.sin() * length);
        let top_col = Color32::from_rgba_unmultiplied(255, 248, 210, *base_alpha);
        let bot_col = Color32::from_rgba_unmultiplied(255, 248, 210, 0);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(sun_pos, top_col);
        mesh.colored_vertex(p1, bot_col);
        mesh.colored_vertex(p2, bot_col);
        mesh.add_triangle(base, base + 1, base + 2);
    }
    painter.add(mesh);
}

/// Subtle edge vignette and warm top-light wash for live play/evolve/replay views.
pub fn draw_atmosphere(painter: &Painter, viewport: Rect) {
    let band = (viewport.height() * 0.08).clamp(18.0, 48.0);
    let mut mesh = Mesh::default();
    let top_a = Color32::from_rgba_unmultiplied(12, 16, 28, 38);
    let clear = Color32::from_rgba_unmultiplied(12, 16, 28, 0);
    // Top subtle shadow
    let b0 = mesh.vertices.len() as u32;
    mesh.colored_vertex(viewport.left_top(), top_a);
    mesh.colored_vertex(viewport.right_top(), top_a);
    mesh.colored_vertex(Pos2::new(viewport.right(), viewport.top() + band), clear);
    mesh.colored_vertex(Pos2::new(viewport.left(), viewport.top() + band), clear);
    mesh.add_triangle(b0, b0 + 1, b0 + 2);
    mesh.add_triangle(b0, b0 + 2, b0 + 3);
    // Bottom subtle vignette
    let b1 = mesh.vertices.len() as u32;
    mesh.colored_vertex(Pos2::new(viewport.left(), viewport.bottom() - band), clear);
    mesh.colored_vertex(Pos2::new(viewport.right(), viewport.bottom() - band), clear);
    mesh.colored_vertex(viewport.right_bottom(), top_a);
    mesh.colored_vertex(viewport.left_bottom(), top_a);
    mesh.add_triangle(b1, b1 + 1, b1 + 2);
    mesh.add_triangle(b1, b1 + 2, b1 + 3);
    painter.add(mesh);
}

/// The sun: a disc with two dimmer rings around it, high and as far away as anything gets.
///
/// A shape rather than a sprite, because a sun is the one thing in the frame that *should*
/// be soft: it is a light, and a hard-edged circle on a gradient sky reads as a sticker. Its
/// parallax is the smallest in the scene (0.04), which is what makes it read as the furthest
/// thing a level has — half a screen of walking moves it four pixels.
///
/// Its *height* is the level's, not the window's: [`SUN_ALTITUDE`] tiles above the ground
/// line, so it is a light in the sky of this level, and it rises and falls with the ground as
/// the camera moves. A sun placed at a tenth of the viewport used to be a sun that hung in
/// front of the level at one framing and sank behind its own hills at the next.
fn draw_sun(painter: &Painter, viewport: Rect, camera: &Camera, level: &Level) {
    let shift = camera.centre.x * SUN_PARALLAX * camera.scale();
    let centre = Pos2::new(
        viewport.left() + viewport.width() * 0.76 - shift,
        sky_line(camera, level, viewport) - SUN_ALTITUDE * camera.scale(),
    );
    let radius = (camera.scale() * 1.5).clamp(9.0, 34.0);
    for (scale, alpha) in [(2.1, 22_u8), (1.5, 40), (1.0, 200)] {
        painter.circle_filled(
            centre,
            radius * scale,
            Color32::from_rgba_unmultiplied(SUN_FACE.r(), SUN_FACE.g(), SUN_FACE.b(), alpha),
        );
    }
}

/// One rank of a tree-line: the same construction at any of its distances.
#[derive(Clone, Copy)]
struct Rank {
    /// Tiles between the clumps this rank plants.
    spacing: f32,
    /// One position in this many is left as open sky.
    gap: u32,
    /// The two silhouettes it draws from.
    shapes: [&'static str; 2],
    /// How many trees stand in one clump: at least `least`, at most `most`.
    least: u32,
    most: u32,
    /// Height range for a tree in this rank, in tiles.
    short: f32,
    tall: f32,
    /// How far below the ridge the feet are buried, in tiles.
    sunk: f32,
    /// Which hashes it draws with, so no two ranks are the same line of trees twice.
    salt: u32,
}

/// How far apart two trees of one clump stand, as a fraction of a tree's own width.
///
/// Less than one, so neighbours touch and a clump is a single mass with a ragged top — but
/// only just less. A pine's crown is opaque across about three quarters of its cell, so at
/// the 0.5 this used to be, two neighbours overlapped by a third of a crown each: the left
/// tree's lit edge and the right tree's shadow rows landed on the same pixels, their tiers
/// interleaved at two different heights, and a clump read as two trees printed on top of
/// each other rather than as two trees growing together. At 0.62 the crowns meet at their
/// ragged edges and barely cross — a mass, not a double exposure — while the centres stay
/// well inside the rank's own spacing, which is what the bunching test measures. The first
/// version packed a clump to the *rank's* spacing instead, which put two trees twenty
/// points apart at a size of twenty-five: three-quarters of a tree of open sky between two
/// trees that are supposed to be growing out of the same stump, which reads as the even row
/// of singles the bunching was meant to replace.
const CLUMP_PACK: f32 = 0.62;

/// The two ranks of the far tree-line, farthest first.
///
/// A forest edge is bunched and two deep: one row of trees at even spacing, which is how this
/// started, reads as a fence with gaps in it — the eye finds the rhythm in three trees and
/// never stops seeing it. So each position plants a clump of two or three growing out of one
/// another, a third of the positions are left as sky (clearings, and what keeps a forest edge
/// from becoming a hedge), and a second rank stands behind the first — denser than the rank in
/// front, a size smaller and a haze paler, so the clearings in the front rank show a band of
/// trees further off rather than an empty horizon.
///
/// The height ranges are narrow, and that is part of the same overlap fix as
/// [`CLUMP_PACK`]: at the old 1.0-to-1.6 spread, two trees of one clump could differ
/// by sixty per cent, so the shorter one's crown ended halfway up the taller one's
/// tiers — two silhouettes crossing mid-crown, which is the strange overlap a
/// clump showed. A stand of one species on one ridge varies by a third, not by
/// two thirds, so the ranges below span about that.
const RANKS: [Rank; 2] = [
    // Behind: the hazier rank, in the `pine_c`/`pine_d` sprites' two-tone palette. Planted at
    // nearly every position — a clearing in the rank behind is a clearing with sky in it, which
    // is exactly what the front rank's clearings must not show.
    Rank {
        spacing: 2.2,
        gap: 4,
        shapes: ["pine_c", "pine_d"],
        least: 1,
        most: 3,
        short: 0.85,
        tall: 1.15,
        sunk: 0.35,
        salt: 171,
    },
    // In front: the rank standing on the ridge, a tile and a fraction tall.
    Rank {
        spacing: 3.0,
        gap: 3,
        shapes: ["pine_a", "pine_b"],
        least: 2,
        most: 3,
        short: 1.05,
        tall: 1.45,
        sunk: 0.0,
        salt: 71,
    },
];

/// The near hills' tree-line: one rank, in the `pine_near_*` sprites' hazed green.
///
/// The near band stood bare while the range behind it was forested, which is
/// backwards — a nearer hillside shows *more* of its trees, not fewer. One rank
/// rather than two: this band is already in front of a two-deep line, and a second
/// rank behind it would be hidden by the first at this size. The trees are the
/// biggest in the backdrop (a tile and a half to two tiles: closer is bigger),
/// planted sparser than the far line and in clumps of at most two, because a near
/// tree drawn at this size in a tight clump of three is a foreground object
/// competing with the level rather than a backdrop. A quarter of its positions
/// are clearings, and through them the far line shows, which is what ties the
/// two bands into one landscape.
///
/// The sprites are optional art, in the way the butterflies are: a tileset drawn
/// before the near band grew a line simply has bare near hills, and [`draw_rank`]
/// skips a tree whose sprite the atlas does not have rather than panicking.
const NEAR_RANKS: [Rank; 1] = [Rank {
    spacing: 3.4,
    gap: 4,
    shapes: ["pine_near_a", "pine_near_b"],
    least: 1,
    most: 2,
    short: 1.5,
    tall: 2.1,
    sunk: 0.0,
    salt: 271,
}];

/// A forest edge of conifers along a hill band's ridge, in ranks.
///
/// The cheapest depth cue in the sky and the one it was missing: silhouettes planted along
/// the ridge of the band behind them, and only two shapes per rank because at this size a
/// tree is a silhouette with a ragged edge rather than a species.
///
/// **Small, because it is far.** A tree on a ridge three hills away is a tenth of the height
/// of one you could walk to, and this line was drawn at nearly half the height of the hills it
/// stands on, which reads as a row of pines planted just behind the level's back wall. A tree
/// on the far band is a tile and a fraction tall on screen at every zoom, against domes three
/// and a half to five tiles across, and the rank behind it is smaller again; the near band's
/// trees are the largest of the three ranks and still under half their domes' height.
///
/// Each tree stands on [`HillBand::ridge`] at its own centre column, and the band it is given
/// is the very object that drew the hills, so the two cannot disagree about where the ground
/// is. Planting on the horizon *line* instead — which is where this used to put them — leaves
/// every tree thirty to eighty points below the ridge it should stand on, inside the hill's flat
/// fill: a pine hovering in front of a hill, its trunk ending in mid air, with the nearest thing
/// below it the level's floor two tiles further down.
fn draw_treeline(sheet: Sheet<'_>, band: &HillBand, ranks: &[Rank]) {
    // Farther rank first, so the rank in front draws over it: that overlap is the whole of what
    // makes the second rank read as *behind* rather than beside.
    for rank in ranks {
        draw_rank(sheet, band, *rank);
    }
}

/// Draw one rank of a tree-line: clumps of trees, planted on the band's ridge.
///
/// A clump is drawn tallest first, so the shorter trees of a stand stand *in front
/// of* the taller ones: drawn in planting order (left to right) instead, which
/// tree of a clump occluded its neighbour depended on which side of the clump it
/// happened to be planted on, and a tall crown could be cut across by a short
/// neighbour on one side and cut across that neighbour on the other. Back-to-front
/// by height is the order a painter would use, and it makes every clump read the
/// same way.
fn draw_rank(sheet: Sheet<'_>, band: &HillBand, rank: Rank) {
    let (painter, viewport, camera, sprites) =
        (sheet.painter, sheet.viewport, sheet.camera, sheet.sprites);
    let scale = camera.scale();
    let spacing = scale * rank.spacing;
    // The band's own shift rather than one of its own: same layer, same motion.
    let shift = viewport.center().x - band.origin;
    let first = ((shift - viewport.width() * 0.5) / spacing).floor() as i32 - 2;
    let last = ((shift + viewport.width() * 1.5) / spacing).ceil() as i32 + 2;
    for i in first..=last {
        if tile_hash(i, 29, rank.salt) % rank.gap == 0 {
            continue;
        }
        let count = rank.least + tile_hash(i, 43, rank.salt + 1) % (rank.most - rank.least + 1);
        // Resolve the whole clump before drawing any of it, so it can be painted
        // tallest first (above) rather than in planting order. Each entry is
        // `(size, x, foot, sprite)`: the foot is resolved here, with the tree,
        // because it is a property of where the tree stands.
        let mut clump: Vec<(f32, f32, f32, Rect)> = Vec::new();
        for k in 0..count {
            let seed = i * 4 + k as i32;
            // Optional art is skipped, not panicked on: the near rank's sprites are
            // newer than the far ranks', and a tileset without them has a bare near
            // band rather than no game (see [`NEAR_RANKS`]).
            let Some(uv) =
                sprites.try_get(rank.shapes[(tile_hash(seed, 31, rank.salt + 2) % 2) as usize])
            else {
                continue;
            };
            // Whole pixels and a whole-pixel size, like everything else: a horizon of trees is
            // the one place where forty half-pixel sprites would shimmer together.
            let height = rank.short
                + (tile_hash(seed, 37, rank.salt + 3) % 100) as f32 / 100.0
                    * (rank.tall - rank.short);
            let size = (scale * height).round().max(scale);
            // Where in the clump this one stands — packed to *its own* width, so a clump is one
            // mass — and a nudge, so clumps of the same size are not the same arrangement twice
            // and a clump of three is not three trees in a column.
            let across = (k as f32 - (count - 1) as f32 * 0.5) * size * CLUMP_PACK;
            let nudge = ((tile_hash(seed, 41, rank.salt + 4) % 100) as f32 / 100.0 - 0.5)
                * size
                * CLUMP_PACK
                * 0.5;
            let x = (viewport.center().x + i as f32 * spacing - shift + across + nudge).round();
            // The foot is the ridge under the tree's own centre, so the trunk's last row lands
            // on the hill's silhouette instead of in the middle of its fill. Rounded up, so a
            // tree is planted *on* the line rather than a hair above it — and the rank behind
            // is buried by a third of a tile, which is what a tree behind a ridge looks like
            // when the hill was painted before it: the trunk crosses the skyline and its base
            // is on the far slope.
            let foot = (band.ridge(x + size * 0.5) + rank.sunk * scale).ceil();
            clump.push((size, x, foot, uv));
        }
        clump.sort_by(|a, b| b.0.partial_cmp(&a.0).expect("no NaN size"));
        for (size, x, foot, uv) in clump {
            painter.image(
                sheet.texture,
                Rect::from_min_size(Pos2::new(x, foot - size), Vec2::splat(size)),
                uv,
                Color32::WHITE,
            );
        }
    }
}

/// One band of rolling hills: a landmass whose top edge is the line its domes rise from.
///
/// The band is an object rather than a function because two things have to agree about where
/// it is — the painter, and whatever is planted on it — and it is **one closed silhouette**
/// rather than a rectangle with fans sitting on it, which is what it used to be. A base with
/// domes on top leaves the base's own top edge showing wherever the hash skips a dome or two
/// neighbours stand too far apart to touch: a flat stripe of the band's colour running from
/// one hill to the next with the range behind it above and open sky beside it, which reads as
/// a gap between two hills rather than as a hillside. It was one, too — the base is a flat
/// line and a dome that is not there is not covered by one. Sampling the ridge into the
/// polygon's own top edge means the drawn edge *is* the line [`HillBand::ridge`] answers with,
/// so the only thing between two domes is the band's own ground, and [`SWELL`] keeps that
/// ground off the horizon everywhere, so the silhouette has no flat floor to read as a gap
/// either.
#[derive(Clone)]
struct HillBand {
    /// Screen x of dome zero, after the camera's shift: `centre(0)`.
    origin: f32,
    spacing: f32,
    /// Screen y of the band's horizon: the level's own ground line, in this camera.
    horizon_y: f32,
    /// The bottom of the window, so the band reaches past everything drawn on top of it.
    bottom: f32,
    /// The span the silhouette is drawn across: the window, a point wider on each side.
    left: f32,
    right: f32,
    /// The domes over that span, as the screen x of each and its radius in points.
    ///
    /// Resolved once when the band is built rather than per column: the silhouette asks every
    /// dome in the band about itself a thousand times a frame.
    domes: Vec<(f32, f32)>,
    /// How low and how high a dome stands above the horizon, in tiles.
    short: f32,
    tall: f32,
    scale: f32,
    salt: u32,
}

impl HillBand {
    /// `parallax` is how much of the camera's own motion the band follows: the far band
    /// barely moves and the near band moves more, which is the whole trick. The domes are
    /// hash-placed from the band's own integer grid, so the hills never swim between frames
    /// and a level's skyline is the same skyline every time it is opened.
    ///
    /// The horizon is the level's ground line and the domes are sized in tiles, so the band is
    /// a picture of a hillside at the level's scale: every zoom shows the same hills at `zoom`
    /// points a tile, standing on the ground the level is walked on, rather than a hillside
    /// that has to be re-tuned for each framing.
    fn new(
        camera: &Camera,
        level: &Level,
        viewport: Rect,
        parallax: f32,
        domes: (f32, f32),
        salt: u32,
    ) -> HillBand {
        let scale = camera.scale();
        let spacing = (scale * BAND_SPACING).max(32.0);
        let shift = camera.centre.x * parallax * scale;
        let (short, tall) = domes;
        let pad = scale * SKY_PAD;
        let mut band = HillBand {
            origin: viewport.center().x - shift,
            spacing,
            horizon_y: sky_line(camera, level, viewport),
            bottom: viewport.bottom() + 1.0,
            left: viewport.left() - pad,
            right: viewport.right() + pad,
            domes: Vec::new(),
            short,
            tall,
            scale,
            salt,
        };
        // Only the domes that can be seen: one whose radius does not reach the drawn span
        // cannot lift the silhouette inside it, and the span runs past the window.
        let reach = scale * tall;
        let first = ((band.left - reach - band.origin) / band.spacing).floor() as i32;
        let last = ((band.right + reach - band.origin) / band.spacing).ceil() as i32;
        for i in first..=last {
            if let Some(r) = band.dome(i) {
                band.domes.push((band.centre(i), r));
            }
        }
        band
    }

    /// Screen x of the centre of dome `i`.
    fn centre(&self, i: i32) -> f32 {
        self.origin + i as f32 * self.spacing
    }

    /// Dome `i`'s radius in points, or `None` where the hash left it out.
    fn dome(&self, i: i32) -> Option<f32> {
        let roll = tile_hash(i, 3, self.salt) % 5;
        if roll == 0 {
            return None;
        }
        // The hash's four surviving rolls spread over the band's range and never reach the
        // bottom of it: the shortest dome in a band is the one that has to touch its
        // neighbours, so it keeps a quarter of the range in hand.
        let t = roll as f32 / 4.0;
        Some(self.scale * (self.short + t * (self.tall - self.short)))
    }

    /// How far the band's own ground stands above its horizon at screen `x`, in points.
    ///
    /// A slow wave rather than a constant, so that no two saddles in the range are the same
    /// depth — a line of hills whose valleys are all one height is a fence with round tops.
    /// It is a function of the band's own coordinate rather than of the window, so it rides
    /// the band's parallax with everything else in it.
    fn swell(&self, x: f32) -> f32 {
        let period = self.scale * SWELL_TILES;
        let t = (x - self.origin) / period;
        let i = t.floor();
        let f = t - i;
        let at = |k: i32| (tile_hash(k, 0, self.salt + 5) % 1000) as f32 / 1000.0;
        // Smoothstep, because a corner in the ground line is a joint between two pictures.
        let s = f * f * (3.0 - 2.0 * f);
        let (low, high) = SWELL;
        self.scale * (low + (high - low) * (at(i as i32) + (at(i as i32 + 1) - at(i as i32)) * s))
    }

    /// The band's silhouette at screen `x`: the top edge of the ground it draws, above the
    /// horizon by the swell and by the tallest dome standing over the column.
    ///
    /// **This is the line something stands on**, and it is the line the painter draws: the
    /// silhouette is this function sampled across the span. The line is left fractional rather
    /// than rounded to a whole point like the rest of the frame, because a flat fill's edge is
    /// the one place a fractional coordinate costs nothing and a whole-point one has to cut a
    /// corner: the ridge is a curve, and a staircase of whole points would put a tree planted
    /// on it beside the edge it was planted on.
    fn ridge(&self, x: f32) -> f32 {
        let mut rise = self.swell(x);
        for &(centre, radius) in &self.domes {
            let away = (x - centre).abs();
            if away < radius {
                // The circle's own height over this column, so a tree planted on the ridge is
                // planted on the dome it can see rather than on a fan that approximates it.
                rise = rise.max((radius * radius - away * away).sqrt());
            }
        }
        self.horizon_y - rise
    }

    /// Draw the band: one closed silhouette across the window, its top edge the ridge.
    ///
    /// One mesh rather than a rectangle and a `circle_filled` per dome, because the ridge has
    /// to be *this* geometry. egui feathers what it paints — it fills half a point inside the
    /// shape it was asked for — and at the rim of a dome half a point along the normal is two
    /// and a half points straight down, which is a sliver of sky under a tree whose foot was
    /// measured against the circle. A polygon sampled off the ridge has no such gap, closes the
    /// sky out of every saddle, and a hillside drawn for a pixel-art game wants a hard edge
    /// anyway.
    fn draw(&self, painter: &Painter, colour: Color32) {
        let mut mesh = Mesh::default();
        let mut below = None;
        // A point a sample along, and the polygon's straight edge cuts the arc it is drawn from
        // by the sagitta of a one-point chord: a hundredth of a point on the biggest dome, and
        // nothing on the swell. So the silhouette is the ridge function to well inside a pixel,
        // which is what lets a tree be planted on one and stand on the other.
        let mut x = self.left.ceil();
        while x <= self.right {
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(Pos2::new(x, self.ridge(x)), colour);
            mesh.colored_vertex(Pos2::new(x, self.bottom), colour);
            if let Some(previous) = below {
                mesh.add_triangle(previous, base, base + 1);
                mesh.add_triangle(previous, base + 1, previous + 1);
            }
            below = Some(base);
            x += 1.0;
        }
        painter.add(mesh);
    }
}

/// One layer of cloud drift: every property that makes it a *distance*.
///
/// The sky has two of these, and they are layers in the same sense as the hill
/// bands — everything about a layer agrees: the farther one is smaller, higher,
/// paler (lower alpha), slower to drift and slower under the camera (a smaller
/// parallax, and crucially a smaller one than either hill band's: see
/// [`SUN_PARALLAX`]). A layer is a value rather than eight more arguments
/// because the numbers are only readable next to the other layer's.
#[derive(Clone, Copy, Debug)]
struct CloudLayer {
    /// How much of the camera's motion this layer follows. Behind both hill
    /// bands, ahead of only the sun.
    parallax: f32,
    /// Drift speed, in tiles per second, on top of the parallax: the sky moves
    /// even when nobody is walking, and the nearer layer drifts faster.
    drift: f32,
    /// Tiles between one cloud slot and the next, before the per-cloud jitter.
    spacing: f32,
    /// Cloud widths in tiles: smallest to largest in this layer. A *range*,
    /// sampled per cloud — see [`draw_clouds`].
    widths: (f32, f32),
    /// Altitudes in tiles above the ground line, measured to the cloud's top:
    /// lowest to highest in this layer, also sampled per cloud.
    altitudes: (f32, f32),
    /// Alpha the layer is drawn at: the farther layer is the paler one.
    alpha: u8,
    /// Which hashes this layer draws with, so the two layers are not the same
    /// sky twice at two speeds — which they were, when both calls shared one
    /// set of salts and differed only in parallax and a clock offset.
    salt: u32,
}

/// The two cloud layers, farthest first.
///
/// The far layer's altitude range sits above the near layer's centre, and its
/// widths sit under the near layer's: higher and smaller is farther, in a sky
/// as much as on a hillside. Both ranges keep a cloud's top above the far
/// range's tallest dome ([`FAR_DOMES`]), because a cloud is behind the hills
/// and only the part of it above their ridge is ever seen.
const CLOUD_LAYERS: [CloudLayer; 2] = [
    CloudLayer {
        parallax: 0.08,
        drift: 0.020,
        spacing: 11.0,
        widths: (2.4, 3.9),
        altitudes: (7.0, 9.2),
        alpha: 150,
        salt: 101,
    },
    CloudLayer {
        parallax: 0.17,
        drift: 0.045,
        spacing: 14.0,
        widths: (3.6, 5.8),
        altitudes: (5.6, 8.0),
        alpha: 205,
        salt: 211,
    },
];

/// How tall a cloud is drawn for how wide it is.
///
/// The sprites' own proportions, roughly: a cumulus cell is opaque across about
/// fourteen of its sixteen columns and ten of its rows, so a cloud drawn square
/// (which this used to do, one 4-tile square for every cloud in the sky) is a
/// cloud stretched half again too tall — and, being the same square every time,
/// the same cloud every time.
const CLOUD_ASPECT: f32 = 0.72;

/// Clouds drifting at their own speed, in three shapes and every size in a range.
///
/// They are sprites rather than shapes because a cloud needs soft edges and a light top,
/// and they carry their own motion in the background: a still sky behind a moving level
/// looks like a painted wall.
///
/// A cloud's height is a world altitude like the sun's and its x is the camera's, because
/// a cloud is not *at* a place in the level: it is far enough away to be in the same
/// direction from anywhere. What that buys is that the cloud the window showed at one
/// framing is the same cloud at the next, higher or lower in the sky as the ground is.
///
/// **No two clouds in a layer are the same size, the same height or the same slot
/// apart**, and that is a change, not a hope: this used to place every cloud in one
/// of three altitude rows, draw every one at exactly 4.0 tiles square, and space
/// them on an unjittered 13-tile grid. Three rows at one size on a rigid grid is
/// a frieze, and the sky read as one cloud stamped across it at one height —
/// the shapes differed and nothing else did. Now the row is a range sampled per
/// cloud from the hash's high slice ([`hash_frac`], because the low bits of
/// neighbouring indices move together and the frieze would survive in miniature),
/// the width is a range in the same way, and each cloud is jittered off its slot
/// by up to a third of the spacing, so the eye gets a sky rather than a pattern.
fn draw_clouds(sheet: Sheet<'_>, level: &Level, clock: f32, layer: CloudLayer) {
    let (painter, viewport, camera, sprites) =
        (sheet.painter, sheet.viewport, sheet.camera, sheet.sprites);
    let scale = camera.scale();
    let spacing = scale * layer.spacing;
    // The drift is a slow time term on top of the camera's parallax.
    let shift = camera.centre.x * layer.parallax * scale + clock * scale * layer.drift;
    let ground = sky_line(camera, level, viewport);
    let first = ((shift - viewport.width() * 0.5) / spacing).floor() as i32 - 2;
    let last = ((shift + viewport.width() * 1.5) / spacing).ceil() as i32 + 2;
    for i in first..=last {
        if tile_hash(i, 5, layer.salt) % 3 != 0 {
            continue;
        }
        let shape = tile_hash(i, 7, layer.salt + 1) % 3;
        let uv = sprites.get(match shape {
            0 => "cloud_a",
            1 => "cloud_b",
            _ => "cloud_c",
        });
        // Width and altitude are independent samples — different salts, so a big
        // cloud is not automatically a low one — and each is a range, not a row.
        let width =
            layer.widths.0 + hash_frac(i, 13, layer.salt + 2) * (layer.widths.1 - layer.widths.0);
        let altitude = layer.altitudes.0
            + hash_frac(i, 17, layer.salt + 3) * (layer.altitudes.1 - layer.altitudes.0);
        let jitter = (hash_frac(i, 19, layer.salt + 4) - 0.5) * spacing * 0.66;
        let size = Vec2::new(scale * width, scale * width * CLOUD_ASPECT);
        let x = viewport.center().x + i as f32 * spacing - shift + jitter;
        let y = ground - altitude * scale;
        // Drawn at a lower alpha than the level: a cloud is behind everything, and a
        // full-strength white blob at the top of the frame fights the panel for attention.
        painter.image(
            // The sheet's own texture, so a cloud is the same art as everything else.
            sheet.texture,
            Rect::from_min_size(Pos2::new(x, y), size),
            uv,
            Color32::from_white_alpha(layer.alpha),
        );
    }
}

/// The four frames of the goal flag's wave, walked by the clock.
const FLAG_FRAMES: [&str; 4] = ["goal_flag_1", "goal_flag_2", "goal_flag_3", "goal_flag_4"];
/// Waves per second: slow enough that the flag reads as cloth, fast enough to catch the
/// eye from across the frame.
const FLAG_HZ: f32 = 3.0;

/// Draw the level: its scenery, its tiles, its goal flag, and (optionally) its grid and
/// bounds.
///
/// `clock` is what animates the things the level itself owns — the waving flag, the
/// swaying grass — and it is an argument rather than a reading of egui's own time because
/// the view is a pure function of it: two draws at the same clock draw the same picture,
/// which is what makes this testable at all. `reach` is the validation overlay; pass
/// `None` for a plain view.
pub fn draw_level(
    sheet: Sheet<'_>,
    level: &Level,
    opts: RenderOpts,
    reach: Option<&ReachReport>,
    clock: f32,
) {
    let painter = sheet.painter;
    let (tx0, tx1, ty0, ty1) = sheet.camera.visible_tiles(level, sheet.viewport);

    // Scenery first, and in the cell the prop itself names: the open tile above the ground
    // it stands on ([`Prop`]). Drawn in the ground's *own* cell — which is where this used
    // to put it — every prop in the game is painted over by the terrain below and nobody
    // ever sees one.
    for ty in ty0..ty1 {
        for tx in tx0..tx1 {
            let prop = scenery_sprite(sheet.sprites, level, tx, ty, clock)
                .or_else(|| hanging_sprite(sheet.sprites, level, tx, ty, clock));
            if let Some(prop) = prop {
                draw_prop(sheet, &prop);
            }
        }
    }

    for ty in ty0..ty1 {
        for tx in tx0..tx1 {
            let rect = sheet.tile(tx, ty);
            let tile = level.tile(tx, ty);
            match tile {
                // Sand, brick and wood are drawn by the same call: which material a solid
                // tile is made of is one lookup inside it, and nothing here has to know.
                // A *cracked* tile is the same call again, and the crack is drawn over it:
                // brittleness is a second thing about a tile, not a fifth material, so the
                // material is drawn exactly as it would be if the tile were sound
                // ([`crate::view::cracked_overlay`]).
                Some(
                    Tile::Solid
                    | Tile::Sand
                    | Tile::Brick
                    | Tile::Plank
                    | Tile::Cracked
                    | Tile::CrackedSand
                    | Tile::CrackedBrick
                    | Tile::CrackedPlank,
                ) => {
                    let tiling = autotile(level, tx, ty);
                    sheet.image(rect, tile_sprite(sheet.sprites, tiling, tx, ty));
                    draw_seams(sheet, level, tx, ty, rect, tiling.look);
                    if let Some(crack) = tile.and_then(|t| cracked_overlay(sheet.sprites, t)) {
                        sheet.image(rect, crack);
                    }
                }
                Some(Tile::Spike) => {
                    sheet.image(rect, sheet.sprites.get("spike"));
                }
                // A spring pad fills its cell the way a spike does, and for the same reason:
                // it is a thing standing in the tile a body passes through rather than a
                // material the level is made of, so there is no autotile family to pick from.
                Some(Tile::Spring) => {
                    sheet.image(rect, sheet.sprites.get("spring"));
                }
                Some(Tile::Empty) | Some(Tile::Goal) | None => {}
            }
            if opts.grid {
                painter.rect_stroke(
                    rect,
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(1.0_f32, GRID_LINE),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    // Placed scenery, over the terrain: a person put it there, so it is drawn where they
    // can see it rather than behind the ground the way the hashed dressing would be. It is
    // the same [`Prop`] the scattered props build, so a placed bush and a blurred one are
    // the same bush.
    for s in level.scenery() {
        if let Some(prop) = placed_prop(sheet.sprites, s) {
            if s.sprite == "lantern" || s.sprite == "crystal" {
                let cell = sheet.tile(prop.cell.0, prop.cell.1);
                let pulse = 0.85 + 0.15 * (clock * 3.5 + s.x as f32).sin();
                let glow_colour = if s.sprite == "lantern" {
                    Color32::from_rgba_unmultiplied(255, 210, 110, (36.0 * pulse) as u8)
                } else {
                    Color32::from_rgba_unmultiplied(110, 230, 255, (32.0 * pulse) as u8)
                };
                painter.circle_filled(cell.center(), cell.width() * 0.95 * pulse, glow_colour);
            }
            draw_prop(sheet, &prop);
        }
    }

    // Ambient life, over the terrain and under everything that moves on its own: the
    // butterflies are the only layer of the level that is in the air, and the last thing
    // a runner should be hidden by is a butterfly.
    if opts.ambient {
        draw_ambient(sheet, level, clock);
    }

    // The level's own walkers, for a view with no simulation in it. The editor is a
    // document about *intent*: what the file says is where the walker is drawn, even
    // though the moment the level is played it starts marching away from this tile. The
    // frame comes from the same `walker_sprite` the live lanes use, so the creature on the
    // editor's canvas and the creature patrolling a run are the same one.
    if opts.entities {
        for walker in &level.walkers {
            // A placement faces right, which is what the loader gives the walker it
            // builds from this tile: an authored level has no direction to draw.
            sheet.actor(
                walker.x,
                walker.y,
                walker.w,
                walker.h,
                sheet.sprites.get(walker_frame(clock)),
                false,
                Color32::WHITE,
            );
        }
    }

    // The goal: pole, finial, and the flag that waves off the right of the pole. The pole
    // is two sprites because the flag hangs in the upper tile, and the finial has to stay
    // put while the cloth moves. In live views (`opts.ambient`), a warm golden beacon aura
    // pulses around the finial so the target is unmistakable from across the level.
    let flag = FLAG_FRAMES[((clock * FLAG_HZ) as i64).rem_euclid(4) as usize];
    for goal in &level.goals {
        let (top, bottom) = goal_cells(*goal);
        let top_rect = sheet.tile(top.0, top.1);
        if opts.ambient {
            let pulse = 0.85 + 0.15 * (clock * 2.8).sin();
            let centre = Pos2::new(
                top_rect.left() + top_rect.width() * 0.35,
                top_rect.top() + top_rect.height() * 0.28,
            );
            painter.circle_filled(
                centre,
                top_rect.width() * 1.15 * pulse,
                Color32::from_rgba_unmultiplied(255, 224, 120, 22),
            );
            painter.circle_filled(
                centre,
                top_rect.width() * 0.65 * pulse,
                Color32::from_rgba_unmultiplied(255, 236, 150, 42),
            );
        }
        sheet.image(
            sheet.tile(bottom.0, bottom.1),
            sheet.sprites.get("goal_bottom"),
        );
        sheet.image(top_rect, sheet.sprites.get("goal_top"));
        sheet.image(top_rect, sheet.sprites.get(flag));
    }

    if opts.spawn_marker {
        let (sx, sy) = level.spawn_tile();
        let rect = sheet.tile(sx, sy);
        sheet.image(rect, sheet.sprites.get("spawn_marker"));
        painter.rect_stroke(
            rect,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.5_f32, SPAWN_LINE),
            egui::StrokeKind::Inside,
        );
    }

    if opts.bounds {
        let rect = Rect::from_min_max(
            sheet.camera.to_screen(0.0, 0.0, sheet.viewport),
            sheet
                .camera
                .to_screen(level.width as f32, level.height as f32, sheet.viewport),
        );
        painter.rect_stroke(
            rect,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(2.0_f32, BOUNDS_LINE),
            egui::StrokeKind::Inside,
        );
    }

    if let Some(report) = reach {
        draw_reach_overlay(sheet, level, report);
    }
}

/// Validation overlay (§9): where the spawn can and cannot get to.
pub fn draw_reach_overlay(sheet: Sheet<'_>, level: &Level, report: &ReachReport) {
    let painter = sheet.painter;
    for platform in &report.platforms {
        if report.reached.get(platform.id).copied().unwrap_or(false) {
            continue;
        }
        let rect = Rect::from_min_max(
            sheet.camera.to_screen(
                platform.x0 as f32,
                platform.surface_row as f32,
                sheet.viewport,
            ),
            sheet.camera.to_screen(
                (platform.x1 + 1) as f32,
                (platform.surface_row + 1) as f32,
                sheet.viewport,
            ),
        );
        painter.rect_filled(rect, egui::CornerRadius::ZERO, UNREACHED_TINT);
    }

    if let Some(culprit) = &report.culprit {
        // The jump that could not be made: mark the tile it takes off from, on the
        // platform it takes off from.
        let row = report
            .platforms
            .get(culprit.from)
            .map(|p| p.surface_row)
            .unwrap_or(0);
        let rect = sheet.tile(culprit.takeoff_x, row);
        painter.rect_stroke(
            rect,
            egui::CornerRadius::same(2),
            egui::Stroke::new(2.0_f32, CULPRIT_LINE),
            egui::StrokeKind::Outside,
        );
    }

    if let Some(frontier) = report.frontier_x {
        let top = sheet.camera.to_screen(frontier as f32, 0.0, sheet.viewport);
        let bottom = sheet
            .camera
            .to_screen(frontier as f32, level.height as f32, sheet.viewport);
        painter.line_segment([top, bottom], egui::Stroke::new(2.0_f32, FRONTIER_LINE));
    }
}

/// Draw a live episode: the level, and every agent in it with the pose its state implies.
pub fn draw_sim(sheet: Sheet<'_>, sim: &Sim, opts: RenderOpts, clock: f32, anim: &AgentAnim) {
    draw_level(sheet, sim.level(), opts, None, clock);
    draw_agents(sheet, sim, clock, Color32::WHITE, anim, true);
}

/// `sim`'s full running speed in tiles per tick.
///
/// The view needs a *speed* to decide whether an agent is running or standing, and it
/// could not read the physics' own `run_speed` without the app handing it a `Physics`.
/// This is that default, and it is a threshold in three places that only ever compare
/// against it: a change there would make the run cycle start a little early or late,
/// which is a cosmetic difference rather than a wrong game.
const RUN_SPEED: f32 = 0.10;
/// Seconds of full-speed running for one turn of the six-frame run cycle.
///
/// The cycle is driven by *distance*, not by time, so an agent creeping at half speed
/// gets half the frame rate: a run cycle that plays at a fixed rate while the legs move
/// at another is the single clearest tell of a game that is not paying attention.
const RUN_CYCLE: f32 = 0.40;
/// How long the landing pose holds after a touchdown.
const LAND_POSE: f32 = 0.12;
/// How long one puff of dust stays up, and how often a runner kicks one up.
const DUST_LIFE: f32 = 0.22;
const DUST_PERIOD: f32 = 0.28;

/// The six frames of the run cycle, in order.
const RUN_FRAMES: [&str; 6] = [
    "player_run_1",
    "player_run_2",
    "player_run_3",
    "player_run_4",
    "player_run_5",
    "player_run_6",
];
/// The four frames of a walker's march.
const WALKER_FRAMES: [&str; 4] = ["walker_a", "walker_b", "walker_c", "walker_d"];

/// What a single frame of a live episode cannot tell you about an agent.
///
/// The renderer owns no state — it takes a `&Sim` and draws — so anything that is a
/// *transition* rather than a state has to live with whoever keeps the clock. There are
/// three such things, and each one is a visible bug without it: a landing pose needs to
/// know that the agent was airborne an instant ago, a puff of dust needs to know how long
/// ago the last one was, and a run cycle needs to accumulate how far the agent has run
/// rather than restarting every time the sim happens to be sampled.
#[derive(Clone, Copy, Debug)]
pub struct TrailDot {
    pub x: f32,
    pub y: f32,
    pub age: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct BurstParticle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub life: f32,
    pub max_life: f32,
    pub colour: [u8; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct AgentAnim {
    /// Distance run so far, in seconds-at-full-speed, wrapped to one run cycle.
    stride: f32,
    /// Seconds left in the landing pose.
    land: f32,
    /// Seconds left on the puff under the feet, if one is showing.
    dust: Option<f32>,
    /// Seconds until the next puff is allowed.
    dust_timer: f32,
    /// Seconds standing still, which drives the two idle frames.
    idle: f32,
    /// Whether the agent was off the ground at the last advance.
    airborne: bool,
    /// The way it was last heading. A standing agent keeps facing the way it ran, which is
    /// both truer and steadier than snapping to the right the moment it stops.
    pub facing_left: bool,
    /// Set for the one frame on which a fall ended.
    pub landed: bool,
    /// Dynamic squash-and-stretch factor on width (1.0 at rest).
    pub squash_x: f32,
    /// Dynamic squash-and-stretch factor on height (1.0 at rest).
    pub squash_y: f32,
    /// Recent positions for high-speed sprint/jump motion trail.
    trail: [TrailDot; 6],
    trail_len: usize,
    trail_timer: f32,
    /// Active particle bursts (jump dust, landing puff, stomp/win sparks).
    particles: [BurstParticle; 12],
    particle_len: usize,
    prev_outcome: Option<Outcome>,
    prev_stomps: u32,
}

impl Default for AgentAnim {
    fn default() -> Self {
        AgentAnim {
            stride: 0.0,
            land: 0.0,
            dust: None,
            dust_timer: 0.0,
            idle: 0.0,
            airborne: false,
            facing_left: false,
            landed: false,
            squash_x: 1.0,
            squash_y: 1.0,
            trail: [TrailDot {
                x: 0.0,
                y: 0.0,
                age: 1.0,
            }; 6],
            trail_len: 0,
            trail_timer: 0.0,
            particles: [BurstParticle {
                x: 0.0,
                y: 0.0,
                vx: 0.0,
                vy: 0.0,
                life: 0.0,
                max_life: 0.3,
                colour: [255, 255, 255],
            }; 12],
            particle_len: 0,
            prev_outcome: None,
            prev_stomps: 0,
        }
    }
}

impl AgentAnim {
    pub fn new() -> AgentAnim {
        AgentAnim::default()
    }

    /// Advance by `dt` seconds from what `sim` reports right now.
    ///
    /// Called once per drawn frame by whoever owns a live episode. Every field here is
    /// derived: the sim is still the only truth about where the player is, and this
    /// remembers only the transitions between two frames.
    pub fn advance(&mut self, sim: &Sim, dt: f32) {
        let p = sim.player();
        let grounded = p.grounded;

        self.landed = false;
        let jumped = !self.airborne && !grounded && p.vy < -0.05;
        if self.airborne && grounded {
            self.land = LAND_POSE;
            self.landed = true;
            self.squash_x = 1.18;
            self.squash_y = 0.84;
            self.spawn_burst(
                p.x + p.w * 0.5,
                p.y + p.h,
                [236, 224, 202],
                4,
                1.8,
                -1.2,
            );
        } else if jumped {
            self.squash_x = 0.86;
            self.squash_y = 1.16;
            self.spawn_burst(
                p.x + p.w * 0.5,
                p.y + p.h,
                [220, 210, 190],
                3,
                1.4,
                -0.8,
            );
        }
        self.airborne = !grounded;

        // Smoothly relax squash & stretch back to (1.0, 1.0).
        let relax = (dt * 14.0).clamp(0.0, 1.0);
        self.squash_x += (1.0 - self.squash_x) * relax;
        self.squash_y += (1.0 - self.squash_y) * relax;

        // Stomp or outcome events trigger burst particles.
        let stomps = sim.stats().stomps;
        if stomps > self.prev_stomps {
            self.prev_stomps = stomps;
            self.squash_x = 1.15;
            self.squash_y = 0.86;
            self.spawn_burst(
                p.x + p.w * 0.5,
                p.y + p.h,
                [222, 168, 244],
                6,
                2.8,
                -2.2,
            );
        }
        let outcome = sim.outcome();
        if self.prev_outcome != Some(outcome) {
            self.prev_outcome = Some(outcome);
            match outcome {
                Outcome::Died => {
                    self.spawn_burst(
                        p.x + p.w * 0.5,
                        p.y + p.h * 0.5,
                        [244, 128, 52],
                        8,
                        3.4,
                        -2.6,
                    );
                }
                Outcome::Won => {
                    self.spawn_burst(
                        p.x + p.w * 0.5,
                        p.y + p.h * 0.3,
                        [255, 214, 102],
                        10,
                        3.6,
                        -3.2,
                    );
                }
                _ => {}
            }
        }

        // Advance active burst particles.
        let mut keep_p = 0;
        for i in 0..self.particle_len {
            let mut pt = self.particles[i];
            pt.life -= dt;
            if pt.life > 0.0 {
                pt.x += pt.vx * dt;
                pt.y += pt.vy * dt;
                pt.vy += 6.5 * dt;
                self.particles[keep_p] = pt;
                keep_p += 1;
            }
        }
        self.particle_len = keep_p;

        // Advance and sample motion trail when sprinting or airborne at high speed.
        let mut keep_t = 0;
        for i in 0..self.trail_len {
            let mut dot = self.trail[i];
            dot.age += dt;
            if dot.age < 0.18 {
                self.trail[keep_t] = dot;
                keep_t += 1;
            }
        }
        self.trail_len = keep_t;
        self.trail_timer = (self.trail_timer - dt).max(0.0);
        if (p.vx.abs() > RUN_SPEED * 1.08 || !grounded)
            && self.trail_timer <= 0.0
            && self.trail_len < self.trail.len()
        {
            self.trail[self.trail_len] = TrailDot {
                x: p.x + p.w * 0.5,
                y: p.y + p.h * 0.55,
                age: 0.0,
            };
            self.trail_len += 1;
            self.trail_timer = 0.028;
        }

        if p.vx.abs() > RUN_SPEED * 0.05 {
            self.facing_left = p.vx < 0.0;
        }

        let speed = (p.vx.abs() / RUN_SPEED).clamp(0.0, 1.0);
        if grounded && speed > 0.2 {
            self.stride = (self.stride + dt * speed).rem_euclid(RUN_CYCLE);
            self.idle = 0.0;
        } else if grounded {
            self.stride = 0.0;
            self.idle += dt;
        } else {
            self.idle = 0.0;
        }

        self.land = (self.land - dt).max(0.0);
        self.dust = self.dust.map(|left| left - dt).filter(|left| *left > 0.0);
        self.dust_timer = (self.dust_timer - dt).max(0.0);
        if grounded && speed > 0.35 && self.dust.is_none() && self.dust_timer <= 0.0 {
            self.dust = Some(DUST_LIFE);
            self.dust_timer = DUST_PERIOD;
        }
    }

    /// Which of the six run frames is up.
    fn run_frame(&self) -> usize {
        ((self.stride / (RUN_CYCLE / 6.0)) as usize).min(RUN_FRAMES.len() - 1)
    }

    /// Whether the idle cycle is on its second frame.
    fn idle_frame(&self) -> bool {
        ((self.idle * 2.2) as i64).rem_euclid(2) == 1
    }

    fn spawn_burst(
        &mut self,
        x: f32,
        y: f32,
        colour: [u8; 3],
        count: usize,
        spread_vx: f32,
        up_vy: f32,
    ) {
        for k in 0..count {
            if self.particle_len >= self.particles.len() {
                break;
            }
            let t = if count > 1 {
                k as f32 / (count - 1) as f32 - 0.5
            } else {
                0.0
            };
            let vx = t * 2.0 * spread_vx;
            let vy = up_vy * (1.0 - 0.35 * t.abs());
            self.particles[self.particle_len] = BurstParticle {
                x,
                y,
                vx,
                vy,
                life: 0.26,
                max_life: 0.26,
                colour,
            };
            self.particle_len += 1;
        }
    }
}

/// The player's sprite for this frame, and whether it is mirrored.
///
/// The order of these tests *is* the priority: a dead agent is dead whatever it was doing,
/// and an airborne one is jumping even if it left the ground a single frame ago. Every
/// name returned is in [`crate::atlas::REQUIRED`], so the atlas has already refused to
/// start without it.
pub fn player_sprite(sim: &Sim, anim: &AgentAnim) -> (&'static str, bool) {
    if sim.outcome() == Outcome::Died {
        return ("player_hurt", anim.facing_left);
    }
    let p = sim.player();
    let flip = anim.facing_left;
    if !p.grounded {
        if p.vy < 0.0 {
            ("player_jump", flip)
        } else {
            ("player_fall", flip)
        }
    } else if anim.land > 0.0 {
        ("player_land", flip)
    } else if p.vx.abs() > RUN_SPEED * 0.2 {
        (RUN_FRAMES[anim.run_frame()], flip)
    } else if anim.idle_frame() {
        ("player_idle_b", flip)
    } else {
        ("player_idle_a", flip)
    }
}

/// Which frame of the march is showing, six frames a second.
///
/// A pure function of the clock, so a replay of one episode draws the same frames it drew
/// the first time — and so the editor, which has no simulation, animates a placed walker
/// with the same legs the live one marches on.
pub fn walker_frame(clock: f32) -> &'static str {
    let frame = ((clock * 6.0) as i64).rem_euclid(WALKER_FRAMES.len() as i64) as usize;
    WALKER_FRAMES[frame]
}

/// The walker's sprite for this frame: the march mirrored when it heads left, and the
/// flattened pose once it has been stomped.
pub fn walker_sprite(walker: &Walker, clock: f32) -> (&'static str, bool) {
    if !walker.alive {
        return ("walker_dead", false);
    }
    (walker_frame(clock), walker.dir < 0)
}

/// Draw just an episode's agents — the player and every walker — leaving the terrain
/// alone.
///
/// Split from [`draw_sim`] for the evolution view, which draws the level **once** and
/// then eight agents from eight different episodes over it: eight copies of the terrain
/// would be eight times the draw calls for one identical picture, and their tile edges
/// would shimmer against each other.
///
/// `tint` multiplies every sprite, which is how a lane is faded without a second set of
/// art. `WHITE` is the plain, fully opaque sprite. `effects` turns the dust, the landing
/// puff and the death burst on: they belong to the one agent being watched, and eight
/// piles of dust at once is noise rather than feedback.
pub fn draw_agents(
    sheet: Sheet<'_>,
    sim: &Sim,
    clock: f32,
    tint: Color32,
    anim: &AgentAnim,
    effects: bool,
) {
    let sprites = sheet.sprites;

    for walker in sim.walkers() {
        let (name, flip) = walker_sprite(walker, clock);
        sheet.actor(
            walker.x,
            walker.y,
            walker.w,
            walker.h,
            sprites.get(name),
            flip,
            tint,
        );
    }

    // Effects go under the agent, before it is drawn: dust at the feet that painted over
    // the legs would look like the agent standing in a cloud.
    if effects {
        let p = sim.player();
        if let Some(left) = anim.dust {
            let name = if left > DUST_LIFE * 0.55 {
                "dust_a"
            } else {
                "dust_b"
            };
            // The dust cell is drawn *below* the feet: its art sits in the bottom rows of
            // the cell, so bottom-aligning it with the player's feet puts the puff on the
            // ground the agent is running over.
            let feet = sheet.world(p.x + p.w * 0.5 - 0.5, p.y + p.h - 1.0, 1.0, 1.0);
            sheet.image_tinted(feet, sprites.get(name), Color32::from_white_alpha(210));
        }
        if anim.landed {
            let feet = sheet.world(p.x + p.w * 0.5 - 0.5, p.y + p.h - 1.0, 1.0, 1.0);
            sheet.image_tinted(feet, sprites.get("puff"), Color32::from_white_alpha(150));
        }

        // High-speed motion trail dots (drawn as non-textured circles so sprite quad counts stay exact).
        let scale = sheet.camera.scale();
        for i in 0..anim.trail_len {
            let dot = anim.trail[i];
            let frac = (1.0 - dot.age / 0.18).clamp(0.0, 1.0);
            let alpha = (frac * 95.0) as u8;
            let pos = sheet.camera.to_screen(dot.x, dot.y, sheet.viewport);
            sheet.painter.circle_filled(
                pos,
                (scale * 0.18 * frac).max(1.5),
                Color32::from_rgba_unmultiplied(255, 186, 116, alpha),
            );
        }

        // Dynamic particle bursts (dust, stomp, death, goal).
        for i in 0..anim.particle_len {
            let pt = anim.particles[i];
            let frac = (pt.life / pt.max_life.max(0.01)).clamp(0.0, 1.0);
            let alpha = (frac * 215.0) as u8;
            let pos = sheet.camera.to_screen(pt.x, pt.y, sheet.viewport);
            let [r, g, b] = pt.colour;
            sheet.painter.circle_filled(
                pos,
                (scale * 0.11 * (0.5 + 0.5 * frac)).max(1.5),
                Color32::from_rgba_unmultiplied(r, g, b, alpha),
            );
        }
    }

    let p = sim.player();
    draw_agent_body(sheet, sim, tint, anim);

    if effects {
        match sim.outcome() {
            // Where it died, not where it is drawn: the burst marks the tile the run
            // failed on, which is the thing worth looking at.
            Outcome::Died => {
                let (x, y) = sim
                    .stats()
                    .death_pos
                    .unwrap_or((p.x + p.w * 0.5, p.y + p.h * 0.5));
                let cell = sheet.world(x - 0.5, y - 0.5, 1.0, 1.0);
                sheet.image_tinted(cell, sprites.get("puff"), Color32::from_white_alpha(120));
            }
            // A win is the one moment the game is allowed to glitter: the two sparkle
            // frames alternate on the clock, over the flag the agent reached.
            Outcome::Won => {
                let spark = if ((clock * 6.0) as i64).rem_euclid(2) == 0 {
                    "spark_a"
                } else {
                    "spark_b"
                };
                for goal in &sim.level().goals {
                    let (top, bottom) = goal_cells(*goal);
                    let uv = sprites.get(spark);
                    sheet.image_tinted(sheet.tile(top.0, top.1), uv, Color32::WHITE);
                    let corner = sheet.tile(bottom.0 - 1, bottom.1);
                    sheet.image_tinted(corner, uv, Color32::from_white_alpha(170));
                }
            }
            Outcome::Running | Outcome::TimedOut | Outcome::Stuck => {}
        }
    }
}

/// Draw one episode's body — its player, and nothing else.
///
/// Split from [`draw_agents`] for the replay's ghosts, which share the level with one
/// champion: a hundred bodies is the picture worth having, but a hundred copies of that
/// level's walkers marching a hundred different distances is a curtain, and the walkers
/// of the episode the viewer is actually watching are already drawn. The dust and the
/// death burst belong to that same one agent, so they are not here either.
pub fn draw_agent_body(sheet: Sheet<'_>, sim: &Sim, tint: Color32, anim: &AgentAnim) {
    let (name, flip) = player_sprite(sim, anim);
    let p = sim.player();
    sheet.actor_squashed(
        p.x,
        p.y,
        p.w,
        p.h,
        anim.squash_x,
        anim.squash_y,
        sheet.sprites.get(name),
        flip,
        tint,
    );
}

/// The hue of each rank band of the ghost field, best band first.
///
/// The four are chosen against the scene rather than in the abstract: the backdrop is a
/// light blue sky over green hills and brown ground, so a pale blue-white agent — the first
/// thing a "cool" palette suggests — is the one colour that disappears against it. Cyan,
/// violet, amber and salmon are all hues the terrain does not use, and they are far enough
/// apart from each other to be told apart on a sprite 24 pixels tall.
const COHORT_HUES: [[u8; 3]; crate::replay::GHOST_COHORTS] = [
    [140, 225, 245],
    [185, 172, 255],
    [245, 205, 110],
    [240, 145, 135],
];

/// How solid each band is drawn: the better the band, the more present it is.
///
/// Every band stays translucent — a ghost that hides the star is worse than no ghost —
/// and the fall is monotone, so the field is an order even where a viewer cannot separate
/// two hues on a moving sprite. It is the property the panel's legend leans on: the bands
/// are listed best first, and the field fades the same way.
const COHORT_ALPHA: [u8; crate::replay::GHOST_COHORTS] = [115, 95, 80, 68];

/// The tint one rank band of the ghost field is drawn with.
pub fn cohort_tint(cohort: usize) -> Color32 {
    let band = cohort.min(crate::replay::GHOST_COHORTS - 1);
    let [r, g, b] = COHORT_HUES[band];
    Color32::from_rgba_unmultiplied(r, g, b, COHORT_ALPHA[band])
}

/// The same band's hue at full strength, for a legend swatch.
///
/// A legend swatch is read against the panel rather than over a level, so a 40%-opaque
/// colour would be a smudge; the hue is what a viewer is matching, and the fall in alpha is
/// described in words beside it.
pub fn cohort_swatch(cohort: usize) -> Color32 {
    let [r, g, b] = COHORT_HUES[cohort.min(crate::replay::GHOST_COHORTS - 1)];
    Color32::from_rgb(r, g, b)
}

/// Draw a replay: the ghosts as bodies in rank bands, then the star complete.
///
/// The order is the point, twice over. The bands go worst first, so the agents nearest the
/// star's quality are drawn on top of the field rather than buried in it — the viewer's
/// eye should land on the agents that are nearly as good, not on the ones that never left
/// the first platform. And the star goes last and with everything on — its walkers, its
/// dust, its flag — because it is what the replay exists for, and two hundred bodies
/// landing on the same tile must not be able to hide it.
pub fn draw_replay(sheet: Sheet<'_>, replay: &crate::replay::Replay) {
    if replay.ghosts {
        for cohort in (0..crate::replay::GHOST_COHORTS).rev() {
            let tint = cohort_tint(cohort);
            for i in 0..replay.ghost_count() {
                if replay.ghost_cohort(i) == cohort {
                    draw_agent_body(sheet, replay.ghost_sim(i), tint, replay.ghost_anim(i));
                }
            }
        }
    }
    draw_agents(
        sheet,
        replay.star_sim(),
        replay.clock(),
        Color32::WHITE,
        replay.star_anim(),
        true,
    );
}

/// A small bar under an agent's feet in the lane's own colour.
///
/// The panel lists the lanes with their colours, and eight translucent agents on one
/// level are otherwise hard to tell apart where they overlap. A tag at the feet rather
/// than an outline around the body keeps the sprite itself readable.
pub fn draw_lane_tag(sheet: Sheet<'_>, sim: &Sim, colour: Color32) {
    let p = sim.player();
    let feet = sheet.world(p.x, p.y + p.h, p.w.max(0.5), 0.0);
    let bar = Rect::from_min_size(
        Pos2::new(feet.left(), feet.top()),
        Vec2::new(feet.width(), (sheet.camera.zoom * 0.12).clamp(1.5, 4.0)),
    );
    sheet
        .painter
        .rect_filled(bar, egui::CornerRadius::ZERO, colour);
}

/// How opaque a death marker is, by how many generations ago it happened.
///
/// Recent corpses are the ones that say where the population is stuck *now*, so they are
/// near-opaque and the older ones fade most of the way out inside the window the map
/// keeps: a forty-generation-old death is history, not a diagnosis.
pub fn death_alpha(age: u32, window: u32) -> u8 {
    if window == 0 {
        return 255;
    }
    let t = age.min(window) as f32 / window as f32;
    (240.0 * (1.0 - 0.75 * t)).round() as u8
}

/// Mark where the population has been dying, on the terrain it died on.
///
/// This is the plan's death map (§10 Tier 1.6) in the medium the window has: the
/// terminal prints a strip of intensity glyphs under the level, and here the same marks
/// land on the tile itself — which is what a level designer actually needs, since the
/// answer to "where do they die" is "at that ledge", not "at column 33".
///
/// `newest` is the generation being drawn, so the fade is measured in generations and
/// stays honest while a run is paused.
pub fn draw_deaths(sheet: Sheet<'_>, marks: &[(f32, f32, u32)], newest: u32, window: u32) {
    let painter = sheet.painter;
    let radius = (sheet.camera.zoom * 0.16).clamp(1.5, 6.0);
    for (x, y, generation) in marks {
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        let alpha = death_alpha(newest.saturating_sub(*generation), window);
        let centre = sheet.camera.to_screen(x + 0.5, y + 0.5, sheet.viewport);
        painter.circle_filled(
            centre,
            radius,
            Color32::from_rgba_unmultiplied(DEATH_MARK[0], DEATH_MARK[1], DEATH_MARK[2], alpha),
        );
    }
}

/// The recorded route as drawable cells, `(tile_x, tile_y, is_end)`.
///
/// The folding rule is `render`'s, taken from [`ChampionTrace::bodies`] rather than
/// re-derived: consecutive samples that land in one tile are one marker, because a route
/// is a path and not a smear, while coming back through a tile *is* recorded twice. That
/// method also puts the end marker first, which is what makes the end the last thing
/// drawn here — where the route stopped is the one thing that must never be hidden.
pub fn route_cells(trace: &render::trace::ChampionTrace) -> Vec<(i32, i32, bool)> {
    trace
        .bodies()
        .iter()
        .filter_map(|body| {
            body.tile_bounds()
                .map(|(tx0, _tx1, ty0, _ty1)| (tx0, ty0, body.alive))
        })
        .collect()
}

/// Draw a recorded champion route over the level.
///
/// A trail of dim marks and one bright end: green at the flag, red in a grave. The
/// vocabulary is the view's own rather than the terminal's glyphs, but it says the same
/// thing the scrubber's `!` and `+` do, so the two views read alike.
pub fn draw_route(sheet: Sheet<'_>, trace: &render::trace::ChampionTrace) {
    let painter = sheet.painter;
    let mut cells = route_cells(trace);
    // The end is first in `bodies()`; draw it last, on top of the trail it ends.
    cells.sort_by_key(|(_, _, is_end)| *is_end);
    for (tx, ty, is_end) in cells {
        let rect = sheet.tile(tx, ty);
        if is_end {
            let colour = if trace.won { ROUTE_FLAG } else { ROUTE_GRAVE };
            painter.rect_filled(
                rect.shrink(rect.width() * 0.16),
                egui::CornerRadius::same(2),
                colour,
            );
        } else {
            painter.rect_filled(
                rect.shrink(rect.width() * 0.34),
                egui::CornerRadius::ZERO,
                ROUTE_TRAIL,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};
    use render::trace::{ChampionTrace, TracePoint};
    use sim::AgentAction;

    fn assets_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    /// The shipped tileset, loaded once per call for the tests that need real sprite
    /// rectangles rather than a hand-built map.
    fn atlas() -> Atlas {
        Atlas::load(&egui::Context::default(), assets_dir()).expect("the shipped tileset loads")
    }

    /// A flat runway this test owns.
    ///
    /// The pose and run-cycle tests are about the animation, not about a level's shape, and
    /// the shipped levels are documents the author draws in: a test that needs an open road
    /// to the right would secretly be asserting something about L1's floor, and a mound
    /// drawn into that floor would fail it for a reason that has nothing to do with poses.
    fn runway() -> Level {
        Level::starter("pose-probe", 30, 8)
    }

    fn shipped(slug: &str) -> Level {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"));
        Level::from_json_file(&path).unwrap_or_else(|e| panic!("{slug}: {e}"))
    }

    /// A level taller than it is wide is framed by the same camera, and this is the property
    /// the first vertical level needed: the fit takes the **smaller** of the two ratios, so a
    /// 10×22 tower is zoomed out until its *height* fits rather than its width, and the whole
    /// grid is on screen in one frame.
    ///
    /// The honest limit, asserted rather than implied: `fit` cannot go below `MIN_ZOOM`, so a
    /// level taller than `viewport_height / MIN_ZOOM` rows is bigger than one screen whatever
    /// the camera does. The window's answer to that is the follow camera panning up the shaft
    /// (`clamp_to` bounds both axes, which is the second half of this test) rather than a fit
    /// that claims more than it can do.
    #[test]
    fn a_tall_level_is_fitted_by_its_height_not_its_width() {
        let tower = shipped("scratch-tower");
        assert!(
            tower.height > tower.width,
            "the fixture is not a vertical level: {}×{}",
            tower.width,
            tower.height
        );
        let viewport = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(917.0, 613.0));

        let camera = Camera::fit(&tower, viewport, 24.0);
        let top_left = camera.to_screen(0.0, 0.0, viewport);
        let bottom_right = camera.to_screen(tower.width as f32, tower.height as f32, viewport);
        for (what, p) in [("top-left", top_left), ("bottom-right", bottom_right)] {
            assert!(
                viewport.contains(p),
                "the {what} corner of the tower is off-screen at {p:?}: it is not fitted"
            );
        }
        // The height is the binding constraint: at the width-only zoom the tower would be
        // taller than the window, so the fitted zoom has to be strictly smaller than it.
        let width_only = ((viewport.width() - 48.0) / tower.width as f32).floor();
        assert!(
            camera.zoom < width_only,
            "the fit used the width ({width_only}) and not the height ({} zoom)",
            camera.zoom
        );
        // So no row of the tower is off-screen.
        let (_, _, ty0, ty1) = camera.visible_tiles(&tower, viewport);
        assert_eq!(
            (ty0, ty1),
            (0, tower.height),
            "the fitted camera does not cover every row of the level"
        );

        // And when the camera *is* following a body, the clamp bounds the vertical axis too:
        // without that, a run up a shaft could follow the climber right off the top of the
        // world and frame empty sky.
        let mut follow = Camera::fit(&tower, viewport, 24.0);
        follow.centre = Vec2::new(5.0, -50.0);
        follow.clamp_to(&tower, 2.0);
        assert!(
            follow.centre.y >= -2.0 && follow.centre.y <= tower.height as f32 + 2.0,
            "clamp_to left the camera at y {} on a {} row level",
            follow.centre.y,
            tower.height
        );
    }

    /// The camera is a whole-pixel grid: every tile boundary lands on a whole point, two
    /// neighbouring tiles share an edge exactly, a tile is exactly `scale()` across, and
    /// `to_tile` inverts `to_screen`.
    ///
    /// This is what stops nearest-neighbour pixel art crawling. A fractional scale makes a
    /// sprite span seventeen pixels on one row and eighteen on the next; rounding each
    /// rectangle on its own leaves a seam between one floor tile and the next. Both are
    /// invisible in a still frame and unmissable in motion, which is exactly why the
    /// arithmetic is pinned here rather than judged by eye in a screenshot.
    #[test]
    fn the_camera_lands_on_whole_pixels() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(917.0, 613.0));
        let mut cameras = vec![Camera::default(), Camera::fit(&level, viewport, 24.0)];
        for (cx, cy) in [(3.25, 7.5), (11.0, 9.0), (17.7, 2.3)] {
            for zoom in [6.0, 17.0, 28.0, 33.0, 96.0] {
                cameras.push(Camera {
                    centre: Vec2::new(cx, cy),
                    zoom,
                });
            }
        }
        // A zoom that is not whole points is drawn at the whole scale nearest it.
        cameras.push(Camera {
            centre: Vec2::new(4.3, 5.1),
            zoom: 17.4,
        });

        for camera in &cameras {
            let scale = camera.scale();
            assert_eq!(
                scale,
                scale.round(),
                "the drawn scale {scale} is not whole points per tile"
            );
            for (tx, ty) in [(0, 0), (5, 9), (23, 12), (59, 14)] {
                let tile = camera.tile_rect(tx, ty, viewport);
                assert_eq!(
                    tile.min,
                    tile.min.round(),
                    "tile ({tx},{ty}) does not start on a whole pixel: {} at zoom {}",
                    tile.min,
                    camera.zoom
                );
                assert_eq!(
                    tile.width(),
                    scale,
                    "tile ({tx},{ty}) is not exactly {scale} across"
                );
                // No seam and no overlap: the next tile starts exactly where this one ends.
                assert_eq!(
                    camera.tile_rect(tx + 1, ty, viewport).min.x,
                    tile.max.x,
                    "tile ({tx},{ty}) and its right neighbour do not share an edge"
                );
                assert_eq!(
                    camera.tile_rect(tx, ty + 1, viewport).min.y,
                    tile.max.y,
                    "tile ({tx},{ty}) and the tile below it do not share an edge"
                );
            }
            // The inverse property the editor's brush and the status bar's tile readout
            // both depend on: a screen point survives a round trip to within the pixel the
            // camera snapped it to.
            for screen in [
                Pos2::new(0.5, 0.5),
                Pos2::new(300.25, 401.75),
                Pos2::new(916.0, 612.0),
            ] {
                let tile = camera.to_tile(screen, viewport);
                let back = camera.to_screen(tile.x, tile.y, viewport);
                assert!(
                    (back.x - screen.x).abs() <= 1.0 && (back.y - screen.y).abs() <= 1.0,
                    "to_tile/to_screen do not invert at {screen}: {back} at zoom {}",
                    camera.zoom
                );
            }
        }
    }

    /// One drawn frame, as the renderer left it.
    ///
    /// `sprites` is one entry per sprite drawn — its screen rectangle and the atlas
    /// rectangle it sampled — and `colours` is every vertex colour in the frame, which is how
    /// a *shape* (the sky gradient, the hills, the sun, the grid) can be found when there is
    /// no rectangle to look for. A sprite is a quad on the atlas texture; egui appends one
    /// quad per image call, so four vertices in a row are one sprite.
    struct Frame {
        sprites: Vec<(Rect, Rect)>,
        colours: Vec<Color32>,
        /// One entry per triangle of a *shape*: the sky, the hill bands, the sun, the grid. This
        /// is the only record of the backdrop's geometry, and so the only thing a sprite planted
        /// on a band can be measured against without asking the code that planted it.
        triangles: Vec<Triangle>,
    }

    /// Where one triangle of a drawn shape is, and what colour it was painted.
    ///
    /// A tessellated frame has no shapes in it, only triangles: everything sharing a texture is
    /// merged into one mesh, and a circle arrives as a fan of them. So the backdrop's geometry
    /// is read back the way a pixel would see it — which triangles cover this point — rather
    /// than by guessing which run of vertices is a dome. That matters: a fitted circle was half
    /// a point out, which is five points wrong where a dome meets the base.
    type Triangle = (Color32, [Pos2; 3]);

    /// The top of the triangle's cross-section at column `x`, or `None` where it does not reach
    /// that column at all.
    fn column_top(tri: &[Pos2; 3], x: f32) -> Option<f32> {
        let mut top = f32::INFINITY;
        for edge in 0..3 {
            let (a, b) = (tri[edge], tri[(edge + 1) % 3]);
            if x < a.x.min(b.x) || x > a.x.max(b.x) {
                continue;
            }
            if a.x == b.x {
                top = top.min(a.y.min(b.y));
            } else {
                let t = (x - a.x) / (b.x - a.x);
                top = top.min(a.y + (b.y - a.y) * t);
            }
        }
        (top.is_finite()).then_some(top)
    }

    /// Is `p` inside the triangle? Three edges, each of which has the point on the same side.
    fn inside(tri: &[Pos2; 3], p: Pos2) -> bool {
        let side = |a: Pos2, b: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
        let (s0, s1, s2) = (
            side(tri[0], tri[1]),
            side(tri[1], tri[2]),
            side(tri[2], tri[0]),
        );
        (s0 >= -0.01 && s1 >= -0.01 && s2 >= -0.01) || (s0 <= 0.01 && s1 <= 0.01 && s2 <= 0.01)
    }

    impl Frame {
        /// The tiles of one colour that were drawn: the shapes the backdrop is built from.
        fn tiles(&self, colour: Color32) -> impl Iterator<Item = &Triangle> {
            self.triangles.iter().filter(move |(c, _)| *c == colour)
        }

        /// The silhouette of the band of `colour` at screen `x`: the highest edge over that
        /// column, read back from the geometry itself rather than from the code that placed it.
        fn ridge_at(&self, colour: Color32, x: f32) -> f32 {
            self.tiles(colour)
                .filter_map(|(_, tri)| column_top(tri, x))
                .fold(f32::INFINITY, f32::min)
        }

        /// Is this point inside the band of `colour` as it was drawn?
        fn covers(&self, colour: Color32, p: Pos2) -> bool {
            self.tiles(colour).any(|(_, tri)| inside(tri, p))
        }

        /// Was this atlas rectangle drawn? Compared with a tolerance, because the sampler is
        /// handed `col / cols` in f32 and a corner is not exact.
        fn draws(&self, uv: Rect) -> bool {
            let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
            self.sprites.iter().any(|(_, drawn)| {
                near(drawn.min.x, uv.min.x)
                    && near(drawn.min.y, uv.min.y)
                    && near(drawn.max.x, uv.max.x)
                    && near(drawn.max.y, uv.max.y)
            })
        }

        /// Was any vertex in this frame multiplied by `colour`?
        fn used(&self, colour: Color32) -> bool {
            self.colours.contains(&colour)
        }
    }

    fn headless_frame(
        level: &Level,
        size: Vec2,
        backdrop: Backdrop,
        clock: f32,
        opts: RenderOpts,
    ) -> Frame {
        let viewport = Rect::from_min_size(Pos2::ZERO, size);
        headless_frame_with(
            level,
            Camera::fit(level, viewport, 24.0),
            size,
            backdrop,
            clock,
            opts,
        )
    }

    /// A frame drawn through a camera of the test's own choosing.
    ///
    /// The camera is what a backdrop can be wrong about without anything else noticing: the
    /// fitted whole-level camera is one framing out of the many the app draws in, and a layer
    /// placed as a fraction of the window is correct in exactly one of them.
    fn headless_frame_with(
        level: &Level,
        camera: Camera,
        size: Vec2,
        backdrop: Backdrop,
        clock: f32,
        opts: RenderOpts,
    ) -> Frame {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let viewport = Rect::from_min_size(Pos2::ZERO, size);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let rect = ui.available_rect_before_wrap();
                        let painter = ui.painter_at(rect);
                        let sheet = Sheet::new(&painter, &atlas, &sprites, &camera, rect);
                        draw_background(sheet, level, backdrop, clock);
                        draw_level(sheet, level, opts, None, clock);
                    });
            },
        );
        let mut frame = Frame {
            sprites: Vec::new(),
            colours: Vec::new(),
            triangles: Vec::new(),
        };
        for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            frame.colours.extend(mesh.vertices.iter().map(|v| v.color));
            // The font texture is where shapes live; the atlas is where sprites do.
            if mesh.texture_id == egui::TextureId::default() {
                for tri in mesh.indices.chunks(3) {
                    let points = [
                        mesh.vertices[tri[0] as usize],
                        mesh.vertices[tri[1] as usize],
                        mesh.vertices[tri[2] as usize],
                    ];
                    // One triangle of one shape: three vertices of one colour. The font's own
                    // textured quads and the atlas's are not shapes and are left out here.
                    if points.iter().all(|v| v.color == points[0].color) {
                        frame.triangles.push((
                            points[0].color,
                            [points[0].pos, points[1].pos, points[2].pos],
                        ));
                    }
                }
                continue;
            }
            for quad in mesh.vertices.chunks(4) {
                if quad.len() < 4 {
                    continue;
                }
                let (mut screen, mut uv) = (Rect::NOTHING, Rect::NOTHING);
                for v in quad {
                    screen = screen.union(Rect::from_min_size(v.pos, Vec2::ZERO));
                    uv = uv.union(Rect::from_min_size(v.uv, Vec2::ZERO));
                }
                frame.sprites.push((screen, uv));
            }
        }
        frame
    }

    /// The hills are one landmass, not a row of mounds with gaps between them.
    ///
    /// Three things are asserted here and every one of them has been wrong. The band reaches
    /// its own horizon and the level's ground under it: a wall drawn *per dome* left a whole
    /// column of open sky in the one place in five where the hash skipped a dome, and a
    /// tree-line planted on that horizon could stand in one. The band's silhouette is above its
    /// horizon *everywhere*, because a base with domes on top of it leaves the base's own flat
    /// top edge showing in every saddle — a stripe of hill colour with the range behind it above
    /// and the sky beside it, one saddle at a time, which is the gap the near band had. And both
    /// bands are checked, because the same construction draws both and the far one is the ground
    /// the pines stand on.
    #[test]
    fn the_hill_bands_have_no_hole_in_them() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let camera = Camera::fit(&level, viewport, 24.0);
        let frame = headless_frame_with(
            &level,
            camera,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let line = sky_line(&camera, &level, viewport);
        let scale = camera.scale();

        for colour in [HILL_FAR, HILL_NEAR] {
            // The samples sit a point inside the band rather than on the line, because a point
            // on it is a point on a triangle's edge, and that is the one place where "inside"
            // is arithmetic rather than geometry.
            for step in 0..=180 {
                let x = viewport.left() + step as f32 * 5.0;
                let ridge = frame.ridge_at(colour, x);
                assert!(ridge.is_finite(), "{colour:?} has nothing drawn at x={x}");
                // A saddle is the band's own ground, never the horizon line: a flat edge between
                // two hills is what a gap in a range is made of.
                assert!(
                    ridge <= line - scale * SWELL.0 + 0.5,
                    "{colour:?} drops to {ridge} against a horizon at {line}: {:.2} tiles of \
                     flat ground at {scale} points a tile is a gap, not a valley",
                    (line - ridge) / scale
                );
                for depth in [1.0, 3.0, 40.0, 120.0] {
                    assert!(
                        frame.covers(colour, Pos2::new(x, line + depth)),
                        "{colour:?} is open sky at x={x}, {depth} points under the ground line"
                    );
                }
            }
        }
    }

    /// The hills stand on the level's own ground line, in every camera.
    ///
    /// Every layer of the sky was placed as a fraction of the viewport's height — the hills at
    /// 0.62 and 0.78 of it, the sun at 0.10 — which is one camera looking at one level, written
    /// down as three numbers. It stops being true the moment anything else is: a camera
    /// following an agent puts the level's floor somewhere else on the screen, a taller level's
    /// floor is somewhere else again, and a backdrop that is *behind* the level and belongs to
    /// the world ends up hanging in front of nothing — a ground plane crossing the level's own
    /// air, hills standing in the air with it, and sky where the ground is. So each band is
    /// asked for its height over the ground line in three cameras, and in all of them the foot
    /// of the range is the level's ground line and the range rises out of it by its own height
    /// in tiles.
    #[test]
    fn the_hills_stand_on_the_levels_own_ground() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let ground = sky_ground(&level);
        let cameras = [
            ("the whole level", Camera::fit(&level, viewport, 24.0)),
            (
                "following an agent on the floor",
                Camera {
                    centre: Vec2::new(12.0, ground - 0.5),
                    zoom: 28.0,
                },
            ),
            (
                "zoomed in at the far end",
                Camera {
                    centre: Vec2::new(level.width as f32 - 4.0, ground - 0.5),
                    zoom: MAX_ZOOM * 0.75,
                },
            ),
        ];

        for (what, camera) in cameras {
            let frame = headless_frame_with(
                &level,
                camera,
                viewport.size(),
                Backdrop::Sky,
                0.0,
                RenderOpts::playing(),
            );
            let line = camera.to_screen(0.0, ground, viewport).y;
            let scale = camera.scale();
            for (colour, (_, tall)) in [(HILL_FAR, FAR_DOMES), (HILL_NEAR, NEAR_DOMES)] {
                for step in 0..=60 {
                    let x = viewport.left() + step as f32 * 15.0;
                    let ridge = frame.ridge_at(colour, x);
                    assert!(
                        ridge.is_finite(),
                        "{what}: {colour:?} draws nothing at x={x}"
                    );
                    // Out of the ground by the band's own height in tiles: the same hills at
                    // every zoom, rather than hills that grow and shrink with the window.
                    let height = (line - ridge) / scale;
                    assert!(
                        height > SWELL.0 - 0.5 / scale && height <= tall + 0.5 / scale,
                        "{what}: {colour:?} stands {height:.2} tiles over the level's ground \
                         line, and this band is a {tall}-tile range"
                    );
                    // And below the line: the plane the level's own floor is the top of, so a
                    // pit in the level shows hill rather than sky.
                    assert!(
                        frame.covers(colour, Pos2::new(x, line + 1.0)),
                        "{what}: {colour:?} is not under the ground line at x={x}"
                    );
                }
            }
        }
    }

    /// The ground line is the top of the level's floor, whatever shape the floor is.
    ///
    /// The sky's horizon is the level's own ground, so the line is asked for four shapes rather
    /// than one: a floor of one row, of two (which is what the shipped pack draws), a floor with
    /// a pit in it — a pit is a hole *in* the ground and the level's ground line does not move
    /// for it — and a level with no ground at all, where there is no line to find and the
    /// level's own bottom edge has to do. The pack's own floors are checked the other way round:
    /// as a property, row by row, rather than against a number copied out of the file.
    #[test]
    fn the_ground_line_is_the_top_of_the_levels_floor() {
        let mut two = Level::starter("two-row", 20, 10);
        for tx in 0..20 {
            two.set_tile(tx, 8, Tile::Solid);
        }
        let mut pitted = Level::starter("pitted", 20, 10);
        pitted.set_tile(9, 9, Tile::Empty);
        let mut sky = Level::starter("sky-only", 20, 10);
        for tx in 0..20 {
            sky.set_tile(tx, 9, Tile::Empty);
        }
        let shapes: [(&str, Level, f32); 4] = [
            ("one row", Level::starter("one-row", 20, 10), 9.0),
            ("two rows", two, 8.0),
            ("a floor with a pit in it", pitted, 9.0),
            ("a level with no ground", sky, 10.0),
        ];
        for (what, level, line) in &shapes {
            assert_eq!(sky_ground(level), *line, "{what}");
        }

        for slug in [
            "l1-flat-run",
            "l2-first-gap",
            "l3-gauntlet",
            "l4-rhythm",
            "l5-tightrope",
            "l6-marathon",
        ] {
            let level = shipped(slug);
            let line = sky_ground(&level) as i32;
            let mut on_ground = false;
            for tx in 0..level.width {
                if solid_at(&level, tx, line) {
                    on_ground = true;
                    assert!(
                        !solid_at(&level, tx, line - 1),
                        "{slug}: the ground line at row {line} has ground above it"
                    );
                }
            }
            assert!(on_ground, "{slug}: the ground line is not on the ground");
        }
    }

    /// The sun and the clouds hang in the level's sky, at their own height over its ground.
    ///
    /// Both used to be fractions of the window — the sun a tenth of the way down it, the clouds
    /// between five and nineteen hundredths — which is a sky painted on the glass rather than on
    /// the world: the level's floor moves with the camera and the sky does not. So the height is
    /// asked for twice: against the ground line in one camera, where the altitude is the level's,
    /// and across two cameras five tiles apart, which is where a sky on the glass would stay put.
    #[test]
    fn the_sun_and_the_clouds_are_in_the_levels_sky() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let ground = sky_ground(&level);
        let here = Camera {
            centre: Vec2::new(20.0, ground - 0.5),
            zoom: 28.0,
        };
        let higher = Camera {
            centre: Vec2::new(20.0, ground - 5.5),
            zoom: 28.0,
        };
        let draw = |camera: Camera| {
            headless_frame_with(
                &level,
                camera,
                viewport.size(),
                Backdrop::Sky,
                0.0,
                RenderOpts::playing(),
            )
        };
        let (above, below) = (draw(here), draw(higher));
        let scale = here.scale();
        let line = here.to_screen(0.0, ground, viewport).y;

        let sun = drawn_sun(&above).expect("the sun is drawn");
        assert!(
            (sun.y - (line - SUN_ALTITUDE * scale)).abs() <= 1.0,
            "the sun is at {}: {SUN_ALTITUDE} tiles over a ground line at {line} is {}",
            sun.y,
            line - SUN_ALTITUDE * scale
        );
        assert!(
            sun.y > viewport.top(),
            "the sun is off the top of the frame at {} in a window {viewport:?}",
            sun.y
        );
        // Five tiles of camera up, five tiles of sun down the screen. A sun painted on the
        // glass is the same height in both frames.
        let sun_below = drawn_sun(&below).expect("the sun is drawn from the higher camera too");
        assert!(
            (sun_below.y - sun.y - 5.0 * scale).abs() <= 1.0,
            "the sun stayed with the window: five tiles of camera moved it {:.1} points",
            sun_below.y - sun.y
        );

        // The clouds by their own rectangles, so this is the placement and not a count.
        let clouds = |frame: &Frame| {
            let mut rects = drawn_sprites(frame, &["cloud_a", "cloud_b", "cloud_c"]);
            rects.sort_by(|a, b| a.min.x.partial_cmp(&b.min.x).expect("no NaN x"));
            rects
        };
        let (high, low) = (clouds(&above), clouds(&below));
        assert!(high.len() >= 3, "the sky has {} cloud(s) in it", high.len());
        assert_eq!(high.len(), low.len(), "the same sky, one window lower");
        // The altitudes the two layers between them can place a cloud at: a range
        // per layer now, not the three rows this used to assert membership of —
        // a cloud at a row's *exact* height is the frieze the ranges replaced.
        let (lowest, highest) = (CLOUD_LAYERS[1].altitudes.0, CLOUD_LAYERS[0].altitudes.1);
        for (i, cloud) in high.iter().enumerate() {
            let altitude = (line - cloud.min.y) / scale;
            assert!(
                altitude >= lowest - 0.01 && altitude <= highest + 0.01,
                "a cloud is {altitude:.2} tiles over the ground line, and the layers \
                 place clouds between {lowest} and {highest} tiles"
            );
            assert!(
                altitude > FAR_DOMES.1,
                "a cloud at {altitude:.2} tiles is inside the {}-tile range rather than over it",
                FAR_DOMES.1
            );
            assert!(
                (cloud.height() / cloud.width() - CLOUD_ASPECT).abs() < 0.03,
                "a cloud is {} points wide and {} tall: not the sprites' proportions",
                cloud.width(),
                cloud.height()
            );
            assert!(
                (low[i].min.y - cloud.min.y - 5.0 * scale).abs() <= 1.0,
                "a cloud stayed with the window: five tiles of camera moved it {:.1} points",
                low[i].min.y - cloud.min.y
            );
        }
    }

    /// The centre of the sun as it was drawn: the middle of the brightest of its three rings,
    /// which is a colour nothing else in the scenery uses.
    fn drawn_sun(frame: &Frame) -> Option<Pos2> {
        let face = Color32::from_rgba_unmultiplied(SUN_FACE.r(), SUN_FACE.g(), SUN_FACE.b(), 200);
        let points: Vec<Pos2> = frame.tiles(face).flat_map(|(_, tri)| *tri).collect();
        let (mut min, mut max) = (*points.first()?, *points.first()?);
        for p in &points {
            min = min.min(*p);
            max = max.max(*p);
        }
        Some(Pos2::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5))
    }

    /// Every pine stands on the ridge of the hills it is drawn in front of.
    ///
    /// A background tree has nothing in front of it to hide its feet and no floor reaching up
    /// to it, so the only thing that can make it read as standing rather than hovering is its
    /// foot landing on the silhouette of the band behind it. Measured against the geometry that
    /// was drawn — the base's top edge and the domes' arcs, read back out of the frame — and
    /// not against the function that placed them, which is how the bug this pins survived the
    /// test that only checked a pine was drawn at all.
    #[test]
    fn every_pine_stands_on_the_hills_ridge() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let scale = Camera::fit(&level, viewport, 24.0).scale();
        let pines = drawn_sprites(&frame, &["pine_a", "pine_b"]);
        assert!(
            pines.len() >= 4,
            "the tree-line has shrunk to {} pine(s)",
            pines.len()
        );

        // The slack is the sprite's own foot and the rounding of it up to a whole point: the
        // foot is `ridge` rounded *up*, so the trunk's last row overlaps the hillside it is
        // planted on by up to a pixel. The band draws the very geometry `ridge` measures, so
        // those two are all the difference there is. A tree planted on the horizon line this
        // used to use sits thirty to eighty points inside a dome's fill — not like this at all.
        const SLACK: f32 = 1.5;
        // And small, because it is far. A pine is a tile to a tile and a half tall on
        // screen whatever the zoom; at the size this line was first drawn — nearly half the
        // height of the hills it stands on — it read as a row of trees just behind the
        // level's back wall rather than a ridge three hills away. The range is the front
        // far rank's own ([`RANKS`]): 1.05 to 1.45 tiles, floored at a tile by the draw.
        for pine in &pines {
            let (shortest, tallest) = (scale, scale * 1.45 + 1.0);
            assert!(
                pine.height() >= shortest && pine.height() <= tallest,
                "a pine is {:.0} points tall at {scale} points a tile: {shortest} to {tallest} \
                 is what a tree that far away looks like",
                pine.height()
            );
            let x = pine.center().x;
            let ridge = frame.ridge_at(HILL_FAR, x);
            assert!(
                (pine.max.y - ridge).abs() <= SLACK,
                "a pine at x={x} has its foot at {} while the hills' ridge is at {ridge}",
                pine.max.y
            );
            // The same statement, the other way round: the hills are under it, so what lies
            // below the trunk is the far band rather than sky. Two points down, because the
            // foot itself is *on* the silhouette, and a point on a triangle's edge is the one
            // place where "inside" is arithmetic. A tree floating in front of a dome — which is
            // what these were — has sky under its foot either way.
            assert!(
                frame.covers(HILL_FAR, Pos2::new(x, pine.max.y + 2.0)),
                "a pine at x={x} is standing in the air: nothing under its foot"
            );
        }
    }

    /// The near hills carry a tree-line of their own, standing on *their* ridge.
    ///
    /// The far band's line is pinned by `every_pine_stands_on_the_hills_ridge`; this
    /// is the same claim for the band in front of it, which stood bare while the
    /// range behind it was forested. Measured the same way — feet read back against
    /// the near band's drawn silhouette — plus the two things that make this rank
    /// a *nearer* one rather than a copy: its trees are bigger than the far rank's,
    /// and they are drawn after it (in front of it), because draw order is depth.
    #[test]
    fn the_near_hills_have_a_treeline_of_their_own() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let scale = Camera::fit(&level, viewport, 24.0).scale();
        let pines = drawn_sprites(&frame, &["pine_near_a", "pine_near_b"]);
        assert!(
            pines.len() >= 6,
            "the near tree-line has shrunk to {} pine(s)",
            pines.len()
        );
        const SLACK: f32 = 1.5;
        for pine in &pines {
            // The near rank's own range: a tile and a half to two tiles and a bit —
            // bigger than the far rank's 1.05–1.45, because it is nearer.
            let (shortest, tallest) = (scale * 1.5 - 1.0, scale * 2.1 + 1.0);
            assert!(
                pine.height() >= shortest && pine.height() <= tallest,
                "a near pine is {:.0} points tall at {scale} points a tile: {shortest} to \
                 {tallest} is the near rank's range",
                pine.height()
            );
            let x = pine.center().x;
            let ridge = frame.ridge_at(HILL_NEAR, x);
            assert!(
                (pine.max.y - ridge).abs() <= SLACK,
                "a near pine at x={x} has its foot at {} while the near hills' ridge is \
                 at {ridge}",
                pine.max.y
            );
            assert!(
                frame.covers(HILL_NEAR, Pos2::new(x, pine.max.y + 2.0)),
                "a near pine at x={x} is standing in the air: nothing under its foot"
            );
        }
        // In front: every near tree reaches the frame after every far one.
        let atlas = atlas();
        let index_of = |name: &str| {
            let uv = atlas.uv(name).expect("the pine is in the atlas");
            frame
                .sprites
                .iter()
                .position(|(_, drawn)| *drawn == uv)
                .expect("the pine is drawn")
        };
        let first_near = ["pine_near_a", "pine_near_b"]
            .iter()
            .map(|name| index_of(name))
            .min()
            .expect("a near pine is drawn");
        let last_far = ["pine_a", "pine_b", "pine_c", "pine_d"]
            .iter()
            .filter_map(|name| {
                let uv = atlas.uv(name).expect("the pine is in the atlas");
                frame.sprites.iter().rposition(|(_, drawn)| *drawn == uv)
            })
            .max()
            .expect("a far pine is drawn");
        assert!(
            last_far < first_near,
            "the near tree-line is drawn at {first_near}, before the far line is \
             finished at {last_far}: the nearer trees are behind the farther ones"
        );
    }

    /// Draw one frame of a live episode (backdrop, level and agents) and report
    /// its sprite quads, in the shape of [`headless_frame_with`] but through
    /// [`draw_sim`]: the agents only exist in a `Sim`, so a level-only frame
    /// cannot say anything about how big the player is drawn.
    fn headless_sim_sprites(level: &Level, camera: Camera, size: Vec2) -> Vec<(Rect, Rect)> {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let viewport = Rect::from_min_size(Pos2::ZERO, size);
        let sim = Sim::new(level, 1);
        let anim = AgentAnim::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let rect = ui.available_rect_before_wrap();
                        let painter = ui.painter_at(rect);
                        let sheet = Sheet::new(&painter, &atlas, &sprites, &camera, rect);
                        draw_background(sheet, level, Backdrop::Sky, 0.0);
                        draw_sim(sheet, &sim, RenderOpts::playing(), 0.0, &anim);
                    });
            },
        );
        let texture = atlas.texture_id();
        let mut quads = Vec::new();
        for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            if mesh.texture_id != texture {
                continue;
            }
            for chunk in mesh.vertices.chunks(4) {
                if chunk.len() < 4 {
                    continue;
                }
                let (mut screen, mut uv) = (Rect::NOTHING, Rect::NOTHING);
                for v in chunk {
                    screen = screen.union(Rect::from_min_size(v.pos, Vec2::ZERO));
                    uv = uv.union(Rect::from_min_size(v.uv, Vec2::ZERO));
                }
                quads.push((screen, uv));
            }
        }
        quads
    }

    /// The player is drawn bigger than its box — and bigger than a walker.
    ///
    /// The regression this pins is the actor fitting its square to the box's
    /// *smaller* side: the 0.6-by-0.9 player came out as a 0.6-tile sprite,
    /// the same size as the 0.6-by-0.6 walker and smaller than a bush. Read
    /// off the frame: the player's rectangle is square (the art's proportions),
    /// its height is the box's height times [`ACTOR_DRAW_SCALE`], its feet are
    /// on the box's floor (an enlarged sprite that floated or sank would trade
    /// one bug for another), and it is plainly taller than the walker in the
    /// same frame — the ordering the physics describes, 0.9 against 0.6.
    #[test]
    fn the_player_is_drawn_bigger_than_its_box_and_its_walkers() {
        let level = shipped("l3-gauntlet");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let camera = Camera::fit(&level, viewport, 24.0);
        let scale = camera.scale();
        let quads = headless_sim_sprites(&level, camera, viewport.size());
        let atlas = atlas();
        let find = |name: &str| -> Vec<Rect> {
            let uv = atlas.uv(name).expect("the sprite is in the atlas");
            quads
                .iter()
                .filter(|(_, drawn)| *drawn == uv)
                .map(|(screen, _)| *screen)
                .collect()
        };
        let players = find("player_idle_a");
        assert_eq!(players.len(), 1, "one standing player in the frame");
        let player = players[0];
        let sim = Sim::new(&level, 1);
        let p = sim.player();
        // Square, and the box's height at the draw scale — not its width.
        assert!(
            (player.width() - player.height()).abs() <= 1.0,
            "the player is {} by {} points: its proportions were squeezed",
            player.width(),
            player.height()
        );
        let expected = p.h * ACTOR_DRAW_SCALE * scale;
        assert!(
            (player.height() - expected).abs() <= 1.5,
            "the player is {:.0} points tall; its {:.2}-tile box at {} points a \
             tile and a draw scale of {ACTOR_DRAW_SCALE} is {expected:.0}",
            player.height(),
            p.h,
            scale
        );
        assert!(
            player.height() > scale,
            "the player is {:.0} points tall at {scale} points a tile: under a \
             tile, which is smaller than the scenery it runs past",
            player.height()
        );
        // Feet on the box's floor, centred on the box.
        let floor = camera.to_screen(0.0, p.y + p.h, viewport).y;
        assert!(
            (player.max.y - floor).abs() <= 1.5,
            "the player's feet are at {}, and its box's floor is at {floor}",
            player.max.y
        );
        let centre = camera.to_screen(p.x + p.w * 0.5, 0.0, viewport).x;
        assert!(
            (player.center().x - centre).abs() <= 1.5,
            "the player is centred at {}, and its box is centred at {centre}",
            player.center().x
        );
        // Taller than a walker, by roughly the boxes' own ratio.
        let walkers: Vec<Rect> = ["walker_a", "walker_b", "walker_c", "walker_d"]
            .iter()
            .flat_map(|name| find(name))
            .collect();
        assert!(
            !walkers.is_empty(),
            "the gauntlet has a walker in the frame"
        );
        for walker in &walkers {
            assert!(
                player.height() > walker.height() * 1.3,
                "the player ({:.0} points) is barely taller than a walker \
                 ({:.0}): the two boxes are 0.9 and 0.6 tiles",
                player.height(),
                walker.height()
            );
        }
    }

    /// No two clouds are the same size, and they are not all at the same height.
    ///
    /// The regression this pins is a sky of one cloud: every cloud drawn at
    /// exactly 4.0 tiles square, in one of three altitude rows, on a 13-tile
    /// grid. Read back off a wide frame so the sample is a sky and not three
    /// clouds: the widths must span a real range (the two layers between them
    /// span 2.4 to 5.8 tiles), the altitudes must span a real range (5.6 to
    /// 9.2), and the widths must actually be *various* — several distinct
    /// sizes, not one size with a single outlier doing the spanning.
    #[test]
    fn the_clouds_are_not_all_the_same_size_or_height() {
        let level = shipped("l1-flat-run");
        let size = Vec2::new(2600.0, 700.0);
        let viewport = Rect::from_min_size(Pos2::ZERO, size);
        let ground = sky_ground(&level);
        let camera = Camera {
            centre: Vec2::new(30.0, ground - 2.0),
            zoom: 28.0,
        };
        let frame = headless_frame_with(
            &level,
            camera,
            size,
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let scale = camera.scale();
        let line = sky_line(&camera, &level, viewport);
        let clouds = drawn_sprites(&frame, &["cloud_a", "cloud_b", "cloud_c"]);
        assert!(
            clouds.len() >= 6,
            "a wide sky holds {} cloud(s): too few to measure a sky by",
            clouds.len()
        );
        let widths: Vec<f32> = clouds.iter().map(|c| c.width() / scale).collect();
        let altitudes: Vec<f32> = clouds.iter().map(|c| (line - c.min.y) / scale).collect();
        let span = |v: &[f32]| {
            v.iter().fold(f32::MIN, |a, b| a.max(*b)) - v.iter().fold(f32::MAX, |a, b| a.min(*b))
        };
        assert!(
            span(&widths) >= 1.2,
            "the clouds span {:.2} tiles of width ({widths:.1?}): one size stamped \
             across the sky is the bug this is for",
            span(&widths)
        );
        assert!(
            span(&altitudes) >= 1.2,
            "the clouds span {:.2} tiles of altitude ({altitudes:.1?}): one height \
             across the sky is the bug this is for",
            span(&altitudes)
        );
        let mut distinct: Vec<i32> = widths.iter().map(|w| (w * scale) as i32).collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(
            distinct.len() >= 4,
            "the sky has {} distinct cloud width(s) in it: {widths:.1?}",
            distinct.len()
        );
    }

    /// The backdrop's layers move at the speed of their depth, in order.
    ///
    /// Parallax is a *relation* between layers, so it is asserted as one: the
    /// factors strictly increase in draw order (sun, far clouds, near clouds,
    /// far hills, near hills), every one of them is slower than the level
    /// itself, and the two hill bands really do translate by their own factor
    /// when the camera moves — measured on the bands' origins, which are the
    /// geometry both the hills and their tree-lines are placed from, so a band
    /// and its trees cannot pass this separately and disagree on screen. The
    /// cloud layers' own agreement is asserted with them: farther is smaller,
    /// higher, paler and slower-drifting.
    #[test]
    fn the_backdrop_layers_move_at_the_speed_of_their_depth() {
        let factors = [
            SUN_PARALLAX,
            CLOUD_LAYERS[0].parallax,
            CLOUD_LAYERS[1].parallax,
            FAR_HILLS_PARALLAX,
            NEAR_HILLS_PARALLAX,
        ];
        for pair in factors.windows(2) {
            assert!(
                pair[0] < pair[1],
                "the parallax factors are not in depth order: {factors:?}"
            );
        }
        assert!(
            factors.iter().all(|f| *f > 0.0 && *f < 1.0),
            "a backdrop layer moves with the level or against it: {factors:?}"
        );
        let (far, near) = (CLOUD_LAYERS[0], CLOUD_LAYERS[1]);
        assert!(
            far.widths.1 <= near.widths.0 + 1.5
                && far.altitudes.0 >= near.altitudes.0
                && far.alpha < near.alpha
                && far.drift < near.drift,
            "the far cloud layer is not the smaller, higher, paler, slower one: \
             {far:?} against {near:?}"
        );

        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let ground = sky_ground(&level);
        let at = |x: f32| Camera {
            centre: Vec2::new(x, ground - 0.5),
            zoom: 28.0,
        };
        let (a, b) = (at(20.0), at(30.0));
        let scale = a.scale();
        for (parallax, domes, salt) in [
            (FAR_HILLS_PARALLAX, FAR_DOMES, 11),
            (NEAR_HILLS_PARALLAX, NEAR_DOMES, 17),
        ] {
            let origin_a = HillBand::new(&a, &level, viewport, parallax, domes, salt).origin;
            let origin_b = HillBand::new(&b, &level, viewport, parallax, domes, salt).origin;
            let moved = origin_a - origin_b;
            let expected = 10.0 * parallax * scale;
            assert!(
                (moved - expected).abs() <= 0.01,
                "a band at parallax {parallax} moved {moved} points for ten tiles of \
                 camera, not {expected}: it is not at its own depth"
            );
        }
    }

    /// The screen rectangles of the sprites a frame drew, for the sprite names given.
    fn drawn_sprites(frame: &Frame, names: &[&str]) -> Vec<Rect> {
        let atlas = atlas();
        names
            .iter()
            .flat_map(|name| {
                let uv = atlas.uv(name).expect("the sprite is in the atlas");
                frame
                    .sprites
                    .iter()
                    .filter(move |(_, drawn)| *drawn == uv)
                    .map(|(screen, _)| *screen)
                    .collect::<Vec<Rect>>()
            })
            .collect()
    }

    /// The rank behind the tree-line stands behind the *ridge*, and reads as behind it.
    ///
    /// Two things make a second rank read as farther away rather than as more trees: it is
    /// smaller and paler, and it is cut off by the hill in front of it. A tree behind a ridge is
    /// hidden by it, so what an eye gets is a crown above the line and a trunk on the hill's far
    /// slope — and since the hills are painted before the trees, being hidden is drawn by burying
    /// the foot and letting the trunk cross the ridge. Buried by a third of a tile, never more:
    /// a trunk that ends deep inside a flat fill with the whole tree under the skyline is the
    /// blob-on-a-hill this started as.
    #[test]
    fn the_rank_behind_stands_behind_the_ridge() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let scale = Camera::fit(&level, viewport, 24.0).scale();
        let behind = drawn_sprites(&frame, &["pine_c", "pine_d"]);
        assert!(
            behind.len() >= 4,
            "the rank behind the tree-line has shrunk to {} tree(s)",
            behind.len()
        );

        for pine in &behind {
            let x = pine.center().x;
            let ridge = frame.ridge_at(HILL_FAR, x);
            let depth = pine.max.y - ridge;
            assert!(
                depth > 0.0 && depth <= scale * 0.5,
                "a pine behind the ridge at x={x} sits {depth} points under it, which is either \
                 floating or buried in the hill"
            );
            // The cell has empty rows above the crown, so a crown that clears the ridge by a
            // quarter of the sprite clears it visibly.
            let clearance = ridge - pine.min.y;
            assert!(
                clearance > pine.height() * 0.25,
                "a pine behind the ridge at x={x} clears it by only {clearance} points of its \
                 own {}, so nothing of it crosses the skyline",
                pine.height()
            );
            let (shortest, tallest) = (scale * 0.8, scale * 1.2 + 1.0);
            assert!(
                pine.height() >= shortest && pine.height() <= tallest,
                "a pine in the rank behind is {:.0} points tall at {scale} points a tile: \
                 {shortest} to {tallest} is the smaller rank",
                pine.height()
            );
        }
    }

    /// The line is bunched, and the rank behind fills what the bunching leaves.
    ///
    /// Every round of "the tree-line still looks wrong" before this one was about tone or size —
    /// distance properties, each a fraction in one place, each tuned by eye. The *shape* of the
    /// line was never measured and it was the thing wrong the whole time: every position planted
    /// its clump at the rank's own spacing apart, so two trees called a clump stood a tree and a
    /// half away from each other and the eye found an even rhythm to lock onto, whatever the
    /// tone. What makes a line of trees read as an edge is that some of them are growing out of
    /// one another, some of the ridge is bare, and what shows through the bare parts is more
    /// trees — so all three are asserted here rather than eyeballed.
    #[test]
    fn the_tree_line_is_bunched_with_clearings_between() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let scale = Camera::fit(&level, viewport, 24.0).scale();
        let mut front: Vec<Rect> = drawn_sprites(&frame, &["pine_a", "pine_b"]);
        let behind = drawn_sprites(&frame, &["pine_c", "pine_d"]);
        front.sort_by(|a, b| a.center().x.partial_cmp(&b.center().x).expect("no NaN x"));
        assert!(
            front.len() >= 8 && behind.len() >= 8,
            "the tree-line is down to {} trees in front and {} behind",
            front.len(),
            behind.len()
        );

        // Bunched: two trees of one clump stand nearer than the rank plants its clumps, by
        // enough that it cannot be an accident of spacing. The threshold is forty per cent of
        // the rank's own interval, which no pair of neighbouring clumps can fall under, so what
        // it counts is the pairs *inside* a clump — and **most** pairs in the line have to be
        // those. The number is not picked by eye: the line as it was measures eleven of thirty-
        // four (a third), which this test was written after seeing, and the clumps this asks for
        // measure twenty-three of thirty-eight (three fifths). Most is what sits between them.
        let pitch = RANKS[1].spacing * scale;
        let distances: Vec<f32> = front
            .windows(2)
            .map(|w| w[1].center().x - w[0].center().x)
            .collect();
        let packed = distances.iter().filter(|d| **d < pitch * 0.4).count();
        assert!(
            packed * 2 >= distances.len(),
            "{} of {} neighbouring pairs in the line are a clump rather than a space: a forest \
             edge is bunched more often than it is spaced, which is a third of the pairs as a row \
             of singles and three fifths as clumps",
            packed,
            distances.len()
        );

        // Clearings: the merged spans of the front rank leave bare stretches of ridge at least
        // two tiles wide. Clumps *and* sky — a line that covers everything is a hedge.
        let mut runs: Vec<(f32, f32)> = Vec::new();
        for pine in &front {
            match runs.last_mut() {
                Some(run) if pine.min.x <= run.1 => run.1 = run.1.max(pine.max.x),
                _ => runs.push((pine.min.x, pine.max.x)),
            }
        }
        let clearings: Vec<(f32, f32)> = runs.windows(2).map(|w| (w[0].1, w[1].0)).collect();
        let widest = clearings.iter().map(|(a, b)| b - a).fold(0.0, f32::max);
        assert!(
            widest >= scale * 2.0,
            "the widest clearing in the line is {widest:.0} points wide at {scale} points a \
             tile: the tree-line has closed up into a wall"
        );

        // And the clearings are not holes: most of them show the rank behind, which is the whole
        // reason it is there. A clearing with no tree in it anywhere behind is a gap in a fence.
        let filled = clearings
            .iter()
            .filter(|(a, b)| {
                behind
                    .iter()
                    .any(|p| p.center().x > *a && p.center().x < *b)
            })
            .count();
        assert!(
            filled * 2 >= clearings.len(),
            "{} of the line's {} clearings have nothing behind them at all",
            clearings.len() - filled,
            clearings.len()
        );

        // Two deep, and the deeper one is behind: every tree of the far rank reaches the frame
        // before the first tree of the front rank does. Draw order is the depth, so a rank that
        // has drifted in front of the one it stands behind is a rank whose trunks cross it.
        let atlas = atlas();
        let uv_of = |names: &[&str]| -> Vec<Rect> {
            names
                .iter()
                .map(|name| atlas.uv(name).expect("the pines are in the atlas"))
                .collect()
        };
        let indices = |uvs: &[Rect]| -> Vec<usize> {
            frame
                .sprites
                .iter()
                .enumerate()
                .filter(|(_, (_, drawn))| uvs.contains(drawn))
                .map(|(i, _)| i)
                .collect()
        };
        let (far, near) = (
            indices(&uv_of(&["pine_c", "pine_d"])),
            indices(&uv_of(&["pine_a", "pine_b"])),
        );
        assert!(
            !far.is_empty() && !near.is_empty() && far.iter().max() < near.iter().min(),
            "the rank behind is drawn over the rank in front: the farthest tree that reaches \
             the frame is drawn at {} and the first one in front at {:?}",
            far.iter().max().map(|i| i.to_string()).unwrap_or_default(),
            near.iter().min()
        );
    }

    /// A backdrop layer that is farther away is paler, and a tree is darker than the hills it
    /// stands on.
    ///
    /// This is the invariant behind three rounds of "the tree-line looks too close": the pines
    /// were painted in the darkest colours in the scene at nearly half the height of the hills
    /// they stood on, and nothing in the project said they should not be. Tone and size are both
    /// distance cues; this is the tone half of it, read off the same constants the renderer
    /// paints with, so an art change that puts a distant layer in foreground colours fails here
    /// instead of waiting to be noticed.
    #[test]
    fn the_backdrop_fades_with_distance() {
        /// How far a colour is from the sky it is drawn against: the sky's own midpoint, which
        /// is what everything distant fades towards.
        fn to_sky(colour: Color32) -> f32 {
            let mid = [SKY_TOP, SKY_BOTTOM].iter().fold([0.0f32; 3], |sum, sky| {
                [
                    sum[0] + sky.r() as f32 * 0.5,
                    sum[1] + sky.g() as f32 * 0.5,
                    sum[2] + sky.b() as f32 * 0.5,
                ]
            });
            let (r, g, b) = (
                colour.r() as f32 - mid[0],
                colour.g() as f32 - mid[1],
                colour.b() as f32 - mid[2],
            );
            (r * r + g * g + b * b).sqrt()
        }

        // Far before near: each pair is the same kind of thing one band or one rank apart.
        for (far, near, what) in [
            (HILL_FAR, HILL_NEAR, "the two hill bands"),
            (PINE_FAR, PINE, "the two ranks of the far tree-line"),
            (PINE_FAR_LIGHT, PINE_LIGHT, "the lit tones of the far ranks"),
            (PINE, PINE_NEAR, "the far rank and the near hills' rank"),
            (
                PINE_LIGHT,
                PINE_NEAR_LIGHT,
                "the lit tones of the far and near ranks",
            ),
        ] {
            assert!(
                to_sky(far) < to_sky(near),
                "{what}: the far one ({far:?}) is not the paler one ({near:?})"
            );
        }
        // And every rank is darker than the hills it stands on, or it would vanish
        // into them rather than standing on them.
        for (tone, rank, hill) in [
            (PINE, "the far rank in front", HILL_FAR),
            (PINE_FAR, "the far rank behind", HILL_FAR),
            (PINE_NEAR, "the near hills' rank", HILL_NEAR),
        ] {
            assert!(
                to_sky(tone) > to_sky(hill),
                "{rank} ({tone:?}) is not darker than the hills it stands on ({hill:?})"
            );
        }
        // And no rank is *too* dark: the far ranks' tones all sit above the
        // luminance the old palette's darkest tone had, which is the reading
        // that shipped as a dark mass against the hill (see the `PINE_*`
        // constants). Luminance, not distance-to-sky, because "too dark" is a
        // claim about light, and a saturated green can be far from the sky
        // while still being light.
        fn luminance(colour: Color32) -> f32 {
            0.2126 * f32::from(colour.r())
                + 0.7152 * f32::from(colour.g())
                + 0.0722 * f32::from(colour.b())
        }
        for (tone, what) in [
            (PINE_LIGHT, "the far rank's lit tone"),
            (PINE, "the far rank's body"),
            (PINE_DARK, "the far rank's shadow"),
            (PINE_SHADE, "the far rank's trunk"),
            (PINE_FAR, "the rank behind's body"),
            (PINE_NEAR, "the near rank's body"),
        ] {
            assert!(
                luminance(tone) > 130.0,
                "{what} ({tone:?}) has a luminance of {:.0}: a backdrop tree tone \
                 that dark is the mass this palette replaced",
                luminance(tone)
            );
        }
    }

    /// The sky has a sun and a tree-line, and the editor's backdrop has neither.
    ///
    /// Both are new *layers* rather than new tints: the sun is a shape at the smallest
    /// parallax in the scene, and the pines are sprites standing on the ridges of
    /// both hill bands, each in its band's own layer because that is what they are
    /// standing on. The editor keeps its
    /// graph paper, because a level being drawn is measured against a grid rather than admired.
    #[test]
    fn the_sky_has_a_sun_and_a_treeline_and_the_editor_does_not() {
        let level = shipped("l1-flat-run");
        let viewport = Vec2::new(900.0, 600.0);
        let atlas = atlas();
        let sky = headless_frame(&level, viewport, Backdrop::Sky, 0.0, RenderOpts::editing());
        let checker = headless_frame(
            &level,
            viewport,
            Backdrop::Checker,
            0.0,
            RenderOpts::editing(),
        );

        for name in [
            "pine_a",
            "pine_b",
            "pine_c",
            "pine_d",
            "pine_near_a",
            "pine_near_b",
        ] {
            let uv = atlas.uv(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(sky.draws(uv), "{name} is never drawn in the sky");
            assert!(!checker.draws(uv), "the editor's backdrop grew a tree-line");
        }
        // The sun is a shape, so it is found by its colour rather than by a rectangle: the
        // brightest of its three rings is this colour at this alpha, and nothing else in the
        // scenery is.
        let face = Color32::from_rgba_unmultiplied(SUN_FACE.r(), SUN_FACE.g(), SUN_FACE.b(), 200);
        assert!(sky.used(face), "no sun in the sky");
        assert!(!checker.used(face), "the editor's backdrop has a sun");
    }

    /// Draw a level into a headless egui context and report what came out: how many
    /// vertices landed on the atlas texture (i.e. actual sprites), and where.
    fn headless_draw(level: &Level, size: Vec2) -> (usize, Rect) {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let viewport = Rect::from_min_size(Pos2::ZERO, size);
        let camera = Camera::fit(level, viewport, 24.0);
        let opts = RenderOpts::editing();

        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let rect = ui.available_rect_before_wrap();
                        let painter = ui.painter_at(rect);
                        let sheet = Sheet::new(&painter, &atlas, &sprites, &camera, rect);
                        draw_level(sheet, level, opts, None, 0.0);
                    });
            },
        );

        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let font = egui::TextureId::default();
        let mut vertices = 0usize;
        let mut bounds = Rect::NOTHING;
        for primitive in primitives {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                if mesh.texture_id == font {
                    continue;
                }
                vertices += mesh.vertices.len();
                for v in &mesh.vertices {
                    bounds = bounds.union(Rect::from_min_size(v.pos, Vec2::ZERO));
                }
            }
        }
        (vertices, bounds)
    }

    /// Draw one headless frame of a level and return every sprite quad in it, as `(where
    /// it was drawn, which atlas cell it came from)`.
    ///
    /// Chunking the mesh's vertices in fours is exact here: one `Painter::image` call is
    /// one quad, and egui merges quads that share a texture into one mesh without
    /// splitting them.
    fn headless_sprite_quads(level: &Level, opts: RenderOpts) -> Vec<(Rect, Rect)> {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let camera = Camera::fit(level, viewport, 24.0);

        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let rect = ui.available_rect_before_wrap();
                        let painter = ui.painter_at(rect);
                        let sheet = Sheet::new(&painter, &atlas, &sprites, &camera, rect);
                        draw_level(sheet, level, opts, None, 0.0);
                    });
            },
        );

        let texture = atlas.texture_id();
        let mut quads = Vec::new();
        for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            if mesh.texture_id != texture {
                continue;
            }
            for chunk in mesh.vertices.chunks(4) {
                if chunk.len() < 4 {
                    continue;
                }
                let (mut min, mut max) = (
                    Pos2::new(f32::INFINITY, f32::INFINITY),
                    Pos2::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
                );
                let (mut uv_min, mut uv_max) = (
                    Pos2::new(f32::INFINITY, f32::INFINITY),
                    Pos2::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
                );
                for v in chunk {
                    min = Pos2::new(min.x.min(v.pos.x), min.y.min(v.pos.y));
                    max = Pos2::new(max.x.max(v.pos.x), max.y.max(v.pos.y));
                    uv_min = Pos2::new(uv_min.x.min(v.uv.x), uv_min.y.min(v.uv.y));
                    uv_max = Pos2::new(uv_max.x.max(v.uv.x), uv_max.y.max(v.uv.y));
                }
                quads.push((
                    Rect::from_min_max(min, max),
                    Rect::from_min_max(uv_min, uv_max),
                ));
            }
        }
        quads
    }

    /// Draw one replay over its level into a headless frame and report how many sprite
    /// quads came out.
    ///
    /// The level is drawn by the caller's own path too ([`headless_sprite_quads`]), so the
    /// difference between the two counts is the agents and nothing else — which is the
    /// question a test of a *drawing* rule has to ask: not "does the roster say there are
    /// ghosts" but "did a body for each of them land in the frame".
    fn headless_replay_quads(replay: &crate::replay::Replay) -> usize {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let camera = Camera::fit(replay.level(), viewport, 24.0);

        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let rect = ui.available_rect_before_wrap();
                        let painter = ui.painter_at(rect);
                        let sheet = Sheet::new(&painter, &atlas, &sprites, &camera, rect);
                        draw_level(sheet, replay.level(), RenderOpts::playing(), None, 0.0);
                        draw_replay(sheet, replay);
                    });
            },
        );

        let texture = atlas.texture_id();
        let mut quads = 0usize;
        for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                if mesh.texture_id == texture {
                    quads += mesh.vertices.len() / 4;
                }
            }
        }
        quads
    }

    /// Draw one headless frame — the level, and a replay over it when one is given — and
    /// report the sprite tints in it in the order they first reach a vertex.
    ///
    /// `Painter` emits its shapes in the order they were asked for and egui merges them
    /// into one mesh without reordering, so a tint's first appearance is a fact about what
    /// was drawn first. That is what makes the *layering* of the field assertable rather
    /// than just the presence of its colours: which band went under which.
    fn headless_frame_tints(level: &Level, replay: Option<&crate::replay::Replay>) -> Vec<Color32> {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let camera = Camera::fit(level, viewport, 24.0);

        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let rect = ui.available_rect_before_wrap();
                        let painter = ui.painter_at(rect);
                        let sheet = Sheet::new(&painter, &atlas, &sprites, &camera, rect);
                        draw_level(sheet, level, RenderOpts::playing(), None, 0.0);
                        if let Some(replay) = replay {
                            draw_replay(sheet, replay);
                        }
                    });
            },
        );

        let texture = atlas.texture_id();
        let mut tints: Vec<Color32> = Vec::new();
        for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            if mesh.texture_id != texture {
                continue;
            }
            for vertex in &mesh.vertices {
                if !tints.contains(&vertex.color) {
                    tints.push(vertex.color);
                }
            }
        }
        tints
    }

    /// The field is drawn in rank bands, and the layering is the band order: the stragglers
    /// go down first and the best of the population sits on top of them.
    #[test]
    fn the_rank_bands_are_drawn_in_their_own_tints_from_the_worst_up() {
        // The palette is an order rather than four decorations: distinct bands, every one of
        // them translucent, and monotone fading as rank falls away.
        let tints: Vec<Color32> = (0..crate::replay::GHOST_COHORTS).map(cohort_tint).collect();
        for pair in tints.windows(2) {
            assert_ne!(pair[0], pair[1], "two rank bands share a tint");
            assert!(
                pair[0].a() > pair[1].a(),
                "a worse band is the more solid one: {pair:?}"
            );
        }
        assert!(
            tints.iter().all(|tint| tint.a() < 255),
            "a ghost tint is opaque: {tints:?}"
        );
        for band in 0..crate::replay::GHOST_COHORTS - 1 {
            assert_eq!(
                cohort_swatch(band).a(),
                255,
                "a legend swatch is translucent"
            );
            assert_ne!(
                cohort_swatch(band),
                cohort_swatch(band + 1),
                "two bands share a hue"
            );
        }
        // A band number past the palette is clamped rather than a panic: a caller that
        // somehow asks for a fifth band gets the last colour, not a crash in a draw call.
        let last = crate::replay::GHOST_COHORTS - 1;
        assert_eq!(
            cohort_swatch(crate::replay::GHOST_COHORTS),
            cohort_swatch(last)
        );
        assert_eq!(cohort_tint(crate::replay::GHOST_COHORTS), cohort_tint(last));

        // The frame itself: the level's own tints, then the bands worst first, then the
        // star — which is drawn last, opaque, whatever the field did.
        let level = Level::starter("replay-cohorts", 40, 8);
        let packet = crate::lab::scored_packet(&level, "view-cohorts");
        let replay = crate::replay::Replay::new(&packet, &level, &evo::EvalConfig::default())
            .expect("a scored generation has a best agent");
        let bands = replay.cohorts();
        assert!(
            bands.len() >= 2,
            "the fixture wears too few bands to show an order: {bands:?}"
        );

        let level_only = headless_frame_tints(&level, None);
        let mut expected = level_only.clone();
        for (band, _) in bands.iter().rev() {
            expected.push(cohort_tint(*band));
        }
        assert_eq!(headless_frame_tints(&level, Some(&replay)), expected);
        assert_eq!(
            *expected.last().expect("a tint"),
            cohort_tint(bands.first().expect("a band").0),
            "the best band was not drawn last among the ghosts"
        );
    }

    /// The replay draws the field it says it is drawing: one body per ghost, and the
    /// star whatever the toggle says.
    ///
    /// The bug this is written against is the one the editor's own walkers had — a
    /// document that is right and a canvas that is wrong — so the assertion is made on the
    /// frame rather than on the roster. A walker-free level and the first tick of an
    /// episode keep the count exact: at tick zero nobody has landed, nobody is dusty and
    /// nothing has won, so every agent quad in the difference is a body.
    #[test]
    fn the_replay_draws_one_body_per_ghost_and_one_for_the_star() {
        let level = Level::starter("replay-draw", 40, 8);
        let packet = crate::lab::scored_packet(&level, "view-draw");
        let mut replay = crate::replay::Replay::new(&packet, &level, &evo::EvalConfig::default())
            .expect("a scored generation has a best agent");
        assert!(
            replay.ghost_count() >= 3,
            "the fixture drew too few ghosts to count: {}",
            replay.ghost_count()
        );

        let level_only = headless_sprite_quads(&level, RenderOpts::playing()).len();
        let shown = headless_replay_quads(&replay) - level_only;
        assert_eq!(
            shown,
            1 + replay.ghost_count(),
            "the frame does not hold the star plus every ghost"
        );

        // And the toggle is a drawing decision end to end: with the ghosts hidden the
        // star is still there, alone.
        replay.ghosts = false;
        let hidden = headless_replay_quads(&replay) - level_only;
        assert_eq!(hidden, 1, "hiding the ghosts hid more than the ghosts");
    }

    /// Draw the backdrop and the level into a headless frame, and report the sprite
    /// vertices (the atlas texture) and all of them.
    fn headless_backdrop_vertices(level: &Level, size: Vec2, backdrop: Backdrop) -> (usize, usize) {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let viewport = Rect::from_min_size(Pos2::ZERO, size);
        let camera = Camera::fit(level, viewport, 24.0);

        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let rect = ui.available_rect_before_wrap();
                        let painter = ui.painter_at(rect);
                        let sheet = Sheet::new(&painter, &atlas, &sprites, &camera, rect);
                        draw_background(sheet, level, backdrop, 0.0);
                        draw_level(sheet, level, RenderOpts::playing(), None, 0.0);
                    });
            },
        );

        let font = egui::TextureId::default();
        let (mut images, mut total) = (0usize, 0usize);
        for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                total += mesh.vertices.len();
                if mesh.texture_id != font {
                    images += mesh.vertices.len();
                }
            }
        }
        (images, total)
    }

    /// Draw a level, optionally with the evolution view's own layers, and report how many
    /// vertices reached the frame: the image-textured ones (sprites) and all of them.
    ///
    /// Both numbers are needed. The agents are sprites, but the lane tag, the death map
    /// and a recorded route are drawn shapes — they land in egui's non-textured mesh, so a
    /// counter that only looked at sprite vertices would call a frame with three of the
    /// four layers missing a correct one.
    fn headless_evolve_vertices(level: &Level, size: Vec2, layers: bool) -> (usize, usize) {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let viewport = Rect::from_min_size(Pos2::ZERO, size);
        let camera = Camera::fit(level, viewport, 24.0);
        let route = ChampionTrace {
            generation: 4,
            max_fitness: 900.0,
            won: false,
            points: vec![
                TracePoint {
                    frame: 0,
                    x: 1.0,
                    y: 6.0,
                },
                TracePoint {
                    frame: 60,
                    x: 8.0,
                    y: 6.0,
                },
            ],
        };

        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let rect = ui.available_rect_before_wrap();
                        let painter = ui.painter_at(rect);
                        let sheet = Sheet::new(&painter, &atlas, &sprites, &camera, rect);
                        draw_level(sheet, level, RenderOpts::playing(), None, 0.0);
                        if layers {
                            let sim = Sim::new(level, 1);
                            draw_agents(
                                sheet,
                                &sim,
                                0.0,
                                Color32::from_white_alpha(120),
                                &AgentAnim::new(),
                                false,
                            );
                            draw_lane_tag(sheet, &sim, Color32::from_rgb(250, 214, 102));
                            draw_deaths(sheet, &[(3.0, 6.0, 4), (8.0, 6.0, 1)], 4, 25);
                            draw_route(sheet, &route);
                        }
                    });
            },
        );

        let font = egui::TextureId::default();
        let (mut images, mut total) = (0usize, 0usize);
        for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                total += mesh.vertices.len();
                if mesh.texture_id != font {
                    images += mesh.vertices.len();
                }
            }
        }
        (images, total)
    }

    /// The evolution layers have to reach the tessellator, not merely compile: a lane's
    /// agent, its tag, the death map and a recorded route all draw over the terrain.
    #[test]
    fn the_evolution_layers_draw_over_the_level() {
        let level = shipped("l1-flat-run");
        let size = Vec2::new(900.0, 600.0);
        let (base_images, base_total) = headless_evolve_vertices(&level, size, false);
        let (images, total) = headless_evolve_vertices(&level, size, true);

        // One lane is one agent *plus* the walkers of its own episode: four sprite
        // vertices each. A lane that drew the whole population, or none of it, would show
        // up here — and the number of walkers is the sim's to report rather than the
        // test's to assume, because the level is a document the author edits.
        let walkers = Sim::new(&level, 1).walkers().len();
        let per_lane = 4 * (1 + walkers);
        assert_eq!(
            images,
            base_images + per_lane,
            "one agent and {walkers} walker(s) per lane: {base_images} -> {images}"
        );
        // The tag, the deaths and the route are drawn shapes rather than sprites, so they
        // are counted in the frame's total.
        assert!(
            total > base_total,
            "the lane tag, the death map and the route should reach the frame: \
             {base_total} -> {total}"
        );
    }

    /// A walker the file places has to be *on the screen* in the editor.
    ///
    /// It arrived in the document long before it arrived in a frame: the only thing that
    /// ever drew a walker was `draw_agents`, which draws the walkers of a live `Sim`, and
    /// the editor has no simulation — so a placed walker was saved, counted in the status
    /// line, and invisible.
    #[test]
    fn the_editor_draws_the_walkers_the_file_placed() {
        let level = shipped("l3-gauntlet");
        assert_eq!(level.walkers.len(), 1, "L3 ships one walker");

        let walker_uv = atlas().uv("walker_a").expect("walker_a is in the atlas");
        let quads: Vec<(Rect, Rect)> = headless_sprite_quads(&level, RenderOpts::editing())
            .into_iter()
            .filter(|(_, uv)| *uv == walker_uv)
            .collect();
        assert_eq!(
            quads.len(),
            level.walkers.len(),
            "one walker sprite per placed walker"
        );

        // And it is drawn where it stands: the sprite's feet on the floor of the box the
        // loader derived for that tile, centred on the same column.
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let camera = Camera::fit(&level, viewport, 24.0);
        let walker = level.walkers[0];
        let feet = camera.world_rect(
            walker.x + walker.w * 0.5,
            walker.y + walker.h,
            0.0,
            0.0,
            viewport,
        );
        let (rect, _) = quads[0];
        assert!(
            (rect.center().x - feet.center().x).abs() <= 1.0,
            "the walker is drawn off its column: {:?} vs {:?}",
            rect.center(),
            feet.center()
        );
        assert!(
            (rect.bottom() - feet.bottom()).abs() <= 1.0,
            "the walker's feet are not on its tile's floor: {:?} vs {:?}",
            rect.bottom(),
            feet.bottom()
        );
    }

    /// The game's backdrop draws clouds that the editor's does not, and both draw the
    /// level: the sky is not a second renderer, it is the same one with weather in it.
    ///
    /// Only the sky path emits *sprites* for the background, so the count of atlas-textured
    /// vertices is what separates the two there. The hills and the gradient are shapes instead,
    /// and a count of shape vertices separates nothing — the editor's grid is a shape too, and a
    /// denser one than two hill bands. So the shapes are compared by *which* colours are in the
    /// frame, which is the question the count was standing in for.
    #[test]
    fn the_sky_backdrop_draws_clouds_over_the_same_level() {
        let level = shipped("l1-flat-run");
        let size = Vec2::new(900.0, 600.0);
        let (checker_images, _) = headless_backdrop_vertices(&level, size, Backdrop::Checker);
        let (sky_images, _) = headless_backdrop_vertices(&level, size, Backdrop::Sky);

        assert!(
            sky_images > checker_images,
            "the sky should add cloud sprites to the frame: {checker_images} -> {sky_images}"
        );
        let sky = headless_frame(&level, size, Backdrop::Sky, 0.0, RenderOpts::playing());
        let checker = headless_frame(&level, size, Backdrop::Checker, 0.0, RenderOpts::playing());
        for colour in [HILL_FAR, HILL_NEAR] {
            assert!(
                sky.tiles(colour).count() > 0,
                "no {colour:?} band in the sky"
            );
            assert_eq!(
                checker.tiles(colour).count(),
                0,
                "the editor's backdrop grew hills"
            );
        }
        assert!(
            checker.tiles(EDITOR_SURFACE).count() > 0,
            "the editor lost the surface it draws the document on"
        );
        // Both draw the level itself, so neither frame is a background and nothing else.
        assert!(
            checker_images >= 4 * 120,
            "the editor's frame should still draw the terrain: {checker_images}"
        );
    }

    /// A route's shape is `render`'s, so the window and the terminal cannot disagree
    /// about it: consecutive samples in one tile are one marker, coming back through a
    /// tile is recorded twice, and exactly one marker is the end.
    #[test]
    fn a_route_folds_into_one_marker_per_tile() {
        let trace = ChampionTrace {
            generation: 3,
            max_fitness: 12.5,
            won: false,
            points: vec![
                TracePoint {
                    frame: 0,
                    x: 1.0,
                    y: 4.0,
                },
                TracePoint {
                    frame: 6,
                    x: 1.4,
                    y: 4.4,
                },
                TracePoint {
                    frame: 12,
                    x: 2.0,
                    y: 4.0,
                },
                TracePoint {
                    frame: 18,
                    x: 3.0,
                    y: 4.0,
                },
                TracePoint {
                    frame: 24,
                    x: 2.4,
                    y: 4.4,
                },
            ],
        };
        let cells = route_cells(&trace);
        let trail: Vec<(i32, i32)> = cells.iter().filter(|c| !c.2).map(|c| (c.0, c.1)).collect();
        assert_eq!(trail, vec![(1, 4), (2, 4), (3, 4), (2, 4)]);
        assert_eq!(
            cells.iter().filter(|c| c.2).count(),
            1,
            "exactly one end marker"
        );

        // A route that never moved still has its end. `bodies` puts the end first and
        // does not fold the trail against it, which is why this is two markers, not one:
        // "where it ended" and "where it went" are different facts even in one tile.
        let still = ChampionTrace {
            generation: 0,
            max_fitness: 0.0,
            won: false,
            points: vec![TracePoint {
                frame: 0,
                x: 2.0,
                y: 3.0,
            }],
        };
        let cells = route_cells(&still);
        assert_eq!(cells.iter().filter(|c| c.2).count(), 1);
        assert!(cells.iter().any(|c| !c.2));
    }

    /// The mask is the contract with the art, so it is checked against a real level rather
    /// than only against a hand-built one — and it is checked *per tile*, because a level
    /// the author has drawn in may have no row that is clean from end to end.
    #[test]
    fn a_surface_tile_asks_for_the_mask_its_neighbours_imply() {
        let level = shipped("l1-flat-run");
        let mut surfaces = 0;
        for ty in 1..level.height {
            for tx in 0..level.width {
                if !solid_at(&level, tx, ty) || solid_at(&level, tx, ty - 1) {
                    continue;
                }
                surfaces += 1;
                assert_eq!(
                    ground_mask(&level, tx, ty) & 0b0001,
                    0b0001,
                    "({tx},{ty}) has sky above it, so its mask says so"
                );
                // Sky above means soil, never rock: rock is what the tiles with something
                // overhead are made of, and this is the same rule read through the name.
                assert!(
                    ground_name(&level, tx, ty).starts_with("ground_"),
                    "({tx},{ty}) has open sky above it and is drawn as rock"
                );
            }
        }
        assert!(
            surfaces > 10,
            "the opening level should have a floor: {surfaces} surface tiles"
        );

        // And its underside is a different sprite from its crown: a floor seen from below
        // has no grass on it.
        let underside = (1..level.height).flat_map(|ty| (0..level.width).map(move |tx| (tx, ty)));
        let undersides = underside.filter(|(tx, ty)| {
            solid_at(&level, *tx, *ty)
                && solid_at(&level, *tx, ty - 1)
                && !solid_at(&level, *tx, ty + 1)
        });
        let mut counted = 0;
        for (tx, ty) in undersides {
            counted += 1;
            assert_eq!(
                ground_mask(&level, tx, ty) & 0b0001,
                0,
                "({tx},{ty}) has ground above it, so it is not a surface"
            );
            assert_eq!(
                ground_mask(&level, tx, ty) & 0b1000,
                0b1000,
                "({tx},{ty}) has air under it, so its mask says so"
            );
        }
        assert!(counted > 0, "the floor has a bottom somewhere");
    }

    /// Rock is only for tiles with no sky anywhere above them, and only in a mass thick
    /// enough for it: a two-tile platform has soil with a shaded underside all the way
    /// through, which is why L5 has no rock in it at all and L3 does.
    #[test]
    fn rock_only_appears_under_a_thick_mass_of_ground() {
        let mut levels_with_rock = 0;
        for slug in ["l1-flat-run", "l3-gauntlet", "l5-tightrope"] {
            let level = shipped(slug);
            let mut soil = 0;
            let mut rock = 0;
            for ty in 0..level.height {
                for tx in 0..level.width {
                    if !solid_at(&level, tx, ty) {
                        continue;
                    }
                    if ground_name(&level, tx, ty).starts_with("deep_") {
                        rock += 1;
                        assert_eq!(
                            ground_mask(&level, tx, ty) & 0b0001,
                            0,
                            "{slug}: ({tx},{ty}) is rock with open sky above it"
                        );
                    } else {
                        soil += 1;
                    }
                }
            }
            assert!(soil > 20, "{slug} is mostly soil: {soil}");
            if rock > 0 {
                levels_with_rock += 1;
            }
        }
        assert!(
            levels_with_rock > 0,
            "some shipped level should be thick enough for rock"
        );
    }

    /// A tile with a wall beside it is not the same art as one in the open, and the tile
    /// the mask describes is the tile the atlas draws.
    ///
    /// The world here is small and shaped on purpose: a floor with a four-tall column
    /// standing on it, so one test covers the surface (a crown), a wall (no crown), the
    /// open bottom of a floor tile, and the row where soil becomes rock.
    #[test]
    fn the_mask_tells_apart_a_floor_from_a_wall_from_a_corner() {
        let mut level = Level::starter("mask-probe", 8, 12);
        for ty in 0..level.height {
            for tx in 0..level.width {
                level.set_tile(tx, ty, Tile::Empty);
            }
        }
        for tx in 0..6 {
            level.set_tile(tx, 9, Tile::Solid);
        }
        for ty in 5..9 {
            level.set_tile(2, ty, Tile::Solid);
        }

        // A floor tile with air above and below it.
        assert_eq!(
            ground_mask(&level, 3, 9),
            0b1001,
            "open above and open below"
        );
        assert_eq!(ground_name(&level, 3, 9), "ground_9");
        assert_eq!(
            ground_mask(&level, 2, 9),
            0b1000,
            "the column's foot: only under it is air"
        );
        assert_eq!(
            ground_name(&level, 2, 9),
            "deep_8",
            "three rows of soil overhead and no sky anywhere near it: this is rock"
        );
        // A middle row of the column: soil, open on both sides, and still not the surface.
        assert_eq!(ground_mask(&level, 2, 6), 0b0110);
        assert_eq!(ground_name(&level, 2, 6), "ground_6");
        assert_eq!(ground_mask(&level, 2, 8), 0b0110);
        assert_eq!(ground_name(&level, 2, 8), "deep_6");
        // The column's top: the only row of it that gets a crown.
        assert_eq!(ground_mask(&level, 2, 5), 0b0111);
        assert_eq!(ground_name(&level, 2, 5), "ground_7");

        // The level's own edge is air to the art even though the physics treats it as a
        // wall. A leftmost floor tile would draw a sealed block if the two were confused.
        assert!(
            level.is_solid(-1, 9),
            "the sim's invisible side wall is solid to the physics"
        );
        assert!(
            !solid_at(&level, -1, 9),
            "...and open air to the art, which is the whole point of `solid_at`"
        );
        assert_eq!(
            ground_mask(&level, 0, 9),
            0b1011,
            "open left, above and below"
        );
    }

    /// The variant is a pure function of the tile, so a level draws the same flowers in
    /// the same places on every frame and after every reload — and it is not the plain
    /// tile everywhere, or the variants would be dead art.
    #[test]
    fn tile_decorations_are_stable_and_not_uniform() {
        let plain = (0..40).filter(|tx| variant_index(*tx, 9) == 0).count();
        assert!(plain > 10, "most tiles stay plain: {plain}/40");
        for tx in -20..20 {
            assert_eq!(
                variant_index(tx, 9),
                variant_index(tx, 9),
                "the same tile always gets the same decoration"
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for tx in -20..20 {
            seen.insert(variant_index(tx, 9));
        }
        assert!(
            seen.contains(&0) && seen.contains(&1),
            "all three states occur"
        );
    }

    /// Scenery is dressing, and these two rules are what keep it from lying: a prop only
    /// stands on a tile that is *solid with sky over it*, and it never moves.
    #[test]
    fn scenery_only_stands_on_open_surfaces() {
        let level = shipped("l3-gauntlet");
        let mut props = 0;
        for ty in 0..level.height {
            for tx in 0..level.width {
                let Some(prop) =
                    scenery_sprite(&Sprites::load(&atlas()).unwrap(), &level, tx, ty, 0.0)
                else {
                    continue;
                };
                props += 1;
                assert!(
                    solid_at(&level, tx, ty),
                    "a prop at ({tx},{ty}) is not standing on ground"
                );
                assert!(
                    !solid_at(&level, tx, ty - 1),
                    "a prop at ({tx},{ty}) is drawn inside the terrain"
                );
                // And it stands in that open tile rather than in the ground's own cell,
                // which is the whole reason it can be seen at all.
                assert_eq!(
                    prop.cell,
                    (tx, ty - 1),
                    "a prop at ({tx},{ty}) is not drawn in the open cell above its ground"
                );
            }
        }
        assert!(
            props > 4,
            "a shipped level should have some scenery: {props}"
        );
    }

    /// The scenery sways, because two frames of it exist; the sway is the only reason the
    /// clock reaches the terrain pass at all.
    /// Placed scenery reaches the frame: one quad for a one-cell prop, two for a tree.
    ///
    /// This is finding 30's question asked of the new layer — "is the thing I placed on
    /// the screen?" — and it is asked of the *frame* rather than of the document, because
    /// the document was always right. A prop the renderer forgets emits zero quads, which
    /// is exactly what a count of the difference catches.
    #[test]
    fn placed_scenery_is_drawn_in_the_frame() {
        let mut level = Level::starter("scenery-frame", 30, 12);
        let opts = RenderOpts::editing();
        let bare = headless_sprite_quads(&level, opts).len();

        // An open cell above the starter's floor: a bush is one quad...
        let air = level.height - 2;
        assert_eq!(level.tile(5, air), Some(Tile::Empty));
        assert!(level.add_scenery("bush_c", 5, air));
        let one = headless_sprite_quads(&level, opts).len();
        assert_eq!(one, bare + 1, "a placed bush must add exactly one quad");

        // ...and a tree is two: its trunk and the canopy above it.
        assert!(level.add_scenery("tree_a_base", 15, air));
        let two = headless_sprite_quads(&level, opts).len();
        assert_eq!(
            two,
            one + 2,
            "a placed tree must add its trunk and its canopy"
        );

        // And clearing the cells gives the quads back: the picture follows the document.
        assert!(level.remove_scenery_at(5, air));
        assert!(level.remove_scenery_at(15, air));
        assert_eq!(headless_sprite_quads(&level, opts).len(), bare);
    }

    #[test]
    fn scenery_sways_with_the_clock() {
        let level = shipped("l1-flat-run");
        let sprites = Sprites::load(&atlas()).unwrap();
        let mut swaying = 0;
        let mut props = 0;
        for ty in 0..level.height {
            for tx in 0..level.width {
                let first = scenery_sprite(&sprites, &level, tx, ty, 0.0).map(|p| p.sprite);
                let later = scenery_sprite(&sprites, &level, tx, ty, 1.0).map(|p| p.sprite);
                if first.is_none() {
                    continue;
                }
                props += 1;
                if first != later {
                    swaying += 1;
                }
            }
        }
        assert!(props > 2, "L1's surface should have some scenery: {props}");
        assert!(
            swaying > 0,
            "the swaying props should be showing a different frame by then: {swaying}/{props}"
        );
    }

    /// Every prop the scatter can *name* has to be reachable on this art, or it is dead
    /// art: a name that never resolves to its own sprite draws its fallback for ever and
    /// nobody can see the difference. The props past `pebble` are optional art with
    /// substitutes, which is exactly the arrangement in which that can happen quietly — so
    /// this asks the shipped PNG, over a long synthetic floor rather than over somebody's
    /// level, because reachability is a question about the hash and not about the pack.
    #[test]
    fn every_prop_the_scatter_can_name_is_drawn_somewhere() {
        let atlas = atlas();
        let sprites = Sprites::load(&atlas).unwrap();
        let floor = Level::starter("prop-probe", 400, 8);
        let mut drawn: Vec<Rect> = Vec::new();
        for ty in 0..floor.height {
            for tx in 0..floor.width {
                if let Some(prop) = scenery_sprite(&sprites, &floor, tx, ty, 0.0) {
                    if !drawn.contains(&prop.sprite) {
                        drawn.push(prop.sprite);
                    }
                }
            }
        }
        for name in [
            "bush_a",
            "tuft_a",
            "flowers",
            "flowers_pink",
            "flowers_white",
            "rock",
            "mushroom",
            "pebble",
            "log",
            "stump",
            "reeds_a",
            "mushroom_pair",
        ] {
            let rect = atlas.uv(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                drawn.contains(&rect),
                "{name} is never drawn on a {}-tile floor: dead art",
                floor.width
            );
        }
    }

    /// The scatter is a dressing, not a wallpaper: over the whole shipped pack, the share
    /// of eligible surface tiles that gets something on it has to stay in a band. A bucket
    /// typo that carpeted every tile, or a `_` arm that swallowed the lot, fails here.
    #[test]
    fn the_scatter_dresses_the_floor_without_paving_it() {
        let atlas = atlas();
        let sprites = Sprites::load(&atlas).unwrap();
        let (mut surfaces, mut props) = (0usize, 0usize);
        for slug in [
            "l1-flat-run",
            "l2-first-gap",
            "l3-gauntlet",
            "l4-rhythm",
            "l5-tightrope",
            "l6-marathon",
        ] {
            let level = shipped(slug);
            for ty in 0..level.height {
                for tx in 0..level.width {
                    // The view's own condition for a tile a prop may stand on.
                    if !solid_at(&level, tx, ty)
                        || ground_mask(&level, tx, ty) & 1 == 0
                        || ty == 0
                        || solid_at(&level, tx, ty - 1)
                    {
                        continue;
                    }
                    surfaces += 1;
                    if scenery_sprite(&sprites, &level, tx, ty, 0.0).is_some() {
                        props += 1;
                    }
                }
            }
        }
        let rate = props as f32 / surfaces as f32;
        assert!(
            (0.15..=0.40).contains(&rate),
            "{props} props over {surfaces} surface tiles is {rate:.2}: either invisible or \
             wallpaper"
        );
    }

    /// A vine hangs from a platform's underside and nowhere else: not under the world's own
    /// edge, not in a cell that is not empty, and never on every column of an overhang at
    /// once — a vine per column is a curtain, not vegetation.
    #[test]
    fn vines_hang_under_platforms_and_only_there() {
        let atlas = atlas();
        let sprites = Sprites::load(&atlas).unwrap();
        // The platform's row is part of the fixture rather than a detail: which columns
        // get a vine is a hash of the cell, so the underside has to be a row this hash
        // actually drapes. Row 6 of a 40-wide probe gets five, two of them under the
        // overhang below — enough that the assertions are about the rule rather than
        // about one lucky tile.
        let mut level = Level::starter("vine-probe", 40, 10);
        for tx in 6..24 {
            assert!(
                level.set_tile(tx, 5, Tile::Solid),
                "the test laid its own platform"
            );
        }
        let vines = [
            atlas.uv("vine_a").expect("vine_a ships"),
            atlas.uv("vine_b").expect("vine_b ships"),
        ];

        let mut hanging = Vec::new();
        for ty in 0..level.height {
            for tx in 0..level.width {
                let Some(prop) = hanging_sprite(&sprites, &level, tx, ty, 0.0) else {
                    continue;
                };
                hanging.push((tx, ty));
                assert_eq!(
                    prop.cell,
                    (tx, ty),
                    "a vine is drawn in another tile's cell"
                );
                assert_eq!(
                    level.tile(tx, ty),
                    Some(Tile::Empty),
                    "a vine at ({tx},{ty}) is drawn in a cell that is not empty"
                );
                assert!(
                    solid_at(&level, tx, ty - 1),
                    "a vine at ({tx},{ty}) hangs from nothing"
                );
                assert!(
                    vines.contains(&prop.sprite),
                    "the hanging scenery at ({tx},{ty}) is not a vine sprite"
                );
            }
        }
        assert!(
            !hanging.is_empty(),
            "an eighteen-tile overhang should have some vines on it"
        );
        assert!(
            hanging.iter().all(|(tx, _)| (6..24).contains(tx)),
            "vines outside the overhang: {hanging:?}"
        );
        assert!(
            hanging.len() < 18,
            "all {} columns of the overhang have a vine: that is a curtain",
            hanging.len()
        );
    }

    /// The player's pose is decided by the sim and the animation together: airborne is
    /// the sim's, landing is the animation's, and the run frame is the distance run.
    #[test]
    fn the_player_pose_follows_the_state_and_the_animation() {
        let level = runway();
        let mut sim = Sim::new(&level, 7);
        let mut anim = AgentAnim::new();

        // Standing at spawn: idle, and not mirrored.
        anim.advance(&sim, 0.016);
        assert_eq!(player_sprite(&sim, &anim).0, "player_idle_a");
        assert!(!player_sprite(&sim, &anim).1);

        // A jump: rising, then falling.
        sim.step(AgentAction::RunJump);
        anim.advance(&sim, sim::DT);
        assert_eq!(player_sprite(&sim, &anim).0, "player_jump");
        for _ in 0..26 {
            sim.step(AgentAction::RunJump);
            anim.advance(&sim, sim::DT);
        }
        assert!(
            sim.player().vy > 0.0 && !sim.player().grounded,
            "by the back half of a 40-tick jump the agent is falling"
        );
        assert_eq!(player_sprite(&sim, &anim).0, "player_fall");

        // The landing: the pose comes from the *transition*, which is why the animation
        // has to be told about the frames the renderer skipped.
        while !sim.player().grounded {
            sim.step(AgentAction::Run);
            anim.advance(&sim, sim::DT);
        }
        assert_eq!(player_sprite(&sim, &anim).0, "player_land");
        assert!(
            anim.landed,
            "the frame the fall ended is the one that puffs"
        );

        // Running: one of the six run frames, and the run cycle is the distance run.
        for _ in 0..20 {
            sim.step(AgentAction::Run);
            anim.advance(&sim, sim::DT);
        }
        let (name, flip) = player_sprite(&sim, &anim);
        assert!(name.starts_with("player_run_"), "running: {name}");
        assert!(!flip, "running right is not mirrored");
        let frame = anim.run_frame();
        for _ in 0..8 {
            sim.step(AgentAction::Run);
            anim.advance(&sim, sim::DT);
        }
        assert_ne!(anim.run_frame(), frame, "the run cycle keeps moving");

        // Turning around mirrors the sprite, and the mirror outlives the step that made
        // it: a standing agent keeps facing the way it was running.
        for _ in 0..30 {
            sim.step(AgentAction::Retreat);
            anim.advance(&sim, sim::DT);
        }
        assert!(player_sprite(&sim, &anim).1, "running left is mirrored");
        for _ in 0..40 {
            sim.step(AgentAction::Brake);
            anim.advance(&sim, sim::DT);
        }
        assert!(
            player_sprite(&sim, &anim).1,
            "and stopping does not turn it"
        );
    }

    /// The run cycle only turns while the agent is on the ground and moving, and the dust
    /// pulses behind it instead of being painted on every frame.
    #[test]
    fn the_run_cycle_and_the_dust_follow_the_ground() {
        let level = runway();
        let mut sim = Sim::new(&level, 3);
        let mut anim = AgentAnim::new();

        let mut puffs = 0;
        for _ in 0..40 {
            sim.step(AgentAction::Run);
            anim.advance(&sim, sim::DT);
            if anim.dust.is_some() {
                puffs += 1;
            }
        }
        assert!(puffs > 0, "a runner should kick up dust");
        assert!(puffs < 40, "and not on every single frame: {puffs}/40");

        // Standing still: the cycle resets to its first frame and the dust stops.
        for _ in 0..40 {
            sim.step(AgentAction::Brake);
            anim.advance(&sim, sim::DT);
        }
        assert_eq!(anim.run_frame(), 0, "a standing agent is on frame one");
        assert!(anim.dust.is_none(), "and is not kicking up dust");

        // A jump: no dust in the air either.
        let mut airborne = AgentAnim::new();
        sim.step(AgentAction::RunJump);
        airborne.advance(&sim, sim::DT);
        for _ in 0..20 {
            sim.step(AgentAction::RunJump);
            airborne.advance(&sim, sim::DT);
            assert!(
                airborne.dust.is_none(),
                "dust belongs on the ground, not under a jumping agent"
            );
        }
    }

    /// A stomped walker is drawn flattened, without a mirror, whatever it was doing.
    #[test]
    fn a_stomped_walker_is_drawn_dead() {
        let level = shipped("l3-gauntlet");
        let sim = Sim::new(&level, 1);
        let Some(walker) = sim.walkers().first() else {
            panic!("the gauntlet has walkers");
        };
        let (name, flip) = walker_sprite(walker, 0.0);
        assert!(name.starts_with("walker_"), "{name}");
        assert_eq!(flip, walker.dir < 0, "a walker faces the way it marches");

        let mut dead = *walker;
        dead.alive = false;
        assert_eq!(walker_sprite(&dead, 0.0), ("walker_dead", false));
    }

    /// The flag waves: the four frames are walked by the clock, and the pole does not
    /// move while they are.
    #[test]
    fn the_goal_flag_waves_on_the_clock() {
        let frame = |clock: f32| FLAG_FRAMES[((clock * FLAG_HZ) as i64).rem_euclid(4) as usize];
        assert_eq!(frame(0.0), "goal_flag_1");
        let seen: std::collections::BTreeSet<_> =
            (0..40).map(|step| frame(step as f32 * 0.05)).collect();
        assert_eq!(seen.len(), 4, "every frame of the wave is shown: {seen:?}");
        // The two tiles a goal covers: the flag rides the upper one, and the lower one is
        // the pole's foot.
        let level = shipped("l1-flat-run");
        let (top, bottom) = goal_cells(level.goals[0]);
        assert_eq!(bottom, (top.0, top.1 + 1));
    }

    #[test]
    fn death_markers_fade_out_over_the_window() {
        assert_eq!(death_alpha(0, 25), 240, "the newest death is fully drawn");
        assert!(death_alpha(12, 25) < death_alpha(0, 25));
        assert!(death_alpha(25, 25) < death_alpha(12, 25));
        assert_eq!(
            death_alpha(400, 25),
            death_alpha(25, 25),
            "older than the window is just the oldest"
        );
        // A zero window is nonsense input, not a panic.
        assert_eq!(death_alpha(3, 0), 255);
    }

    #[test]
    fn a_shipped_level_draws_sprites_where_the_camera_says_it_should() {
        let level = shipped("l1-flat-run");
        let size = Vec2::new(900.0, 600.0);
        let camera = Camera::fit(&level, Rect::from_min_size(Pos2::ZERO, size), 24.0);
        let (vertices, bounds) = headless_draw(&level, size);
        eprintln!(
            "zoom {:.2} centre {:?} -> {vertices} sprite vertices, bounds {bounds:?}",
            camera.zoom, camera.centre
        );
        // L1's floor is two full rows (120 tiles) plus a flag and a pole.
        assert!(
            vertices >= 4 * 122,
            "a 60x15 level should emit one quad per drawn cell, got {vertices} vertices"
        );
        // And they must land on the level, not somewhere off-screen: the camera puts
        // the floor's last row on screen, near the bottom of the window.
        assert!(
            bounds.min.x > 0.0 && bounds.max.x < size.x,
            "sprites should be inside the viewport: {bounds:?}"
        );
        assert!(
            bounds.max.y > size.y * 0.5,
            "a flat level's floor should sit in the lower half: {bounds:?}"
        );
    }

    /// Every butterfly sprite, so no test has to spell the four out.
    const BUTTERFLIES: [&str; 4] = [
        "butterfly_a_1",
        "butterfly_a_2",
        "butterfly_b_1",
        "butterfly_b_2",
    ];

    /// A level whose every solid tile is made of `material`.
    ///
    /// Painted rather than loaded, because the point is the *material* and the shipped
    /// pack is soil: the first level with a beach in it is the one this test builds.
    fn a_level_of(material: Tile) -> Level {
        let mut level = Level::starter("material-probe", 30, 10);
        // Two rows thick, like the shipped pack. The upper row is the one the mask calls
        // `_1` — open above and nothing else — and the lower is the floor of the world,
        // which is open *below* as well because there is no world under it.
        for ty in [level.height - 2, level.height - 1] {
            for tx in 0..level.width {
                level.set_tile(tx, ty, material);
            }
        }
        level
    }

    /// The tile in the middle of a two-row floor: the one a level's mask calls `_1`.
    fn a_plain_floor_tile(level: &Level) -> (i32, i32) {
        (15, level.height - 2)
    }

    /// How many sprites from a family a frame drew.
    fn drawn_family(frame: &Frame, prefixes: &[&str]) -> usize {
        let atlas = atlas();
        let wanted: Vec<Rect> = atlas
            .names()
            .filter(|name| prefixes.iter().any(|prefix| name.starts_with(prefix)))
            .filter_map(|name| atlas.uv(name).ok())
            .collect();
        frame
            .sprites
            .iter()
            .filter(|(_, uv)| wanted.contains(uv))
            .count()
    }

    /// A level is made of a *material*, and the view draws the material the level names.
    ///
    /// The half of `Tile::Sand` that the sim cannot see. `sim` has its own test that a
    /// beach behaves exactly as a floor does — same state hash, same observation, every
    /// tick — so what is left is the other promise: that the level file can *say* a beach
    /// and the renderer believes it. A level painted with one brush is the whole of it:
    /// every solid tile sand, and the frame must contain sand tiles and no soil at all.
    #[test]
    fn a_level_made_of_sand_is_drawn_in_sand() {
        let level = a_level_of(Tile::Sand);
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        assert!(
            drawn_family(&frame, &["sand_", "sandstone_"]) > 0,
            "a level made of sand drew no sand"
        );
        assert_eq!(
            drawn_family(&frame, &["ground_", "deep_"]),
            0,
            "a level made entirely of sand still drew soil"
        );

        // And the mask picks the sprite in sand exactly as it does in soil: the middle of
        // a flat floor asks for `sand_1`, which is the beach the palette draws.
        let (tx, ty) = a_plain_floor_tile(&level);
        assert_eq!(ground_mask(&level, tx, ty), 0b0001, "a flat floor's mask");
        assert_eq!(ground_name(&level, tx, ty), "sand_1");
        assert_eq!(ground_name(&a_level_of(Tile::Solid), tx, ty), "ground_1");
    }

    /// Sand is optional art: a tileset drawn before the material existed paints a beach as
    /// soil rather than refusing to start.
    ///
    /// The same bargain the tile variants and the later props make, and the reason the
    /// sand family is not in `atlas::REQUIRED` — a tileset is somebody's art, and a new
    /// *drawing* of the ground must not be a new requirement on it. The test strips the
    /// sand sprites out of the shipped atlas by hand, which is the only way to check a
    /// fallback that by definition never fires on this project's own assets.
    #[test]
    fn a_beach_in_a_tileset_without_sand_is_drawn_as_soil() {
        let dir = std::env::temp_dir().join(format!("evo-no-sand-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let src = assets_dir();
        std::fs::copy(src.join("tileset.png"), dir.join("tileset.png")).expect("copy png");
        let mut doc: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(src.join("tileset.json")).expect("manifest"),
        )
        .expect("manifest json");
        let kept: Vec<serde_json::Value> = doc["sprites"]
            .as_array()
            .expect("a sprite list")
            .iter()
            .filter(|sprite| !sprite["name"].as_str().expect("a name").starts_with("sand"))
            .cloned()
            .collect();
        doc["sprites"] = serde_json::json!(kept);
        std::fs::write(
            dir.join("tileset.json"),
            serde_json::to_string_pretty(&doc).expect("json"),
        )
        .expect("write manifest");

        let ctx = egui::Context::default();
        let stripped = Atlas::load(&ctx, &dir).expect("a tileset without sand still loads");
        let sprites = Sprites::load(&stripped).expect("and still has everything required");
        let level = a_level_of(Tile::Sand);
        let (tx, ty) = a_plain_floor_tile(&level);

        // The name is still sand's — the view asks for the material the level names —
        // and the *sprite* is the soil tile with the same mask and the same depth.
        assert_eq!(ground_name(&level, tx, ty), "sand_1");
        assert_eq!(
            ground_sprite(&sprites, &level, tx, ty),
            sprites.get("ground_1"),
            "a beach should be drawn as soil when the tileset has no sand"
        );
        // And with the shipped atlas it is sand, or one of its two variants.
        let shipped = Sprites::load(&atlas()).expect("the shipped tileset loads");
        let drawn = ground_sprite(&shipped, &level, tx, ty);
        assert!(
            ["sand_1", "sand_1_b", "sand_1_c"]
                .iter()
                .any(|name| shipped.get(name) == drawn),
            "a sand tile should ask for the sand family, not {drawn:?}"
        );
    }

    /// A tree is the one prop bigger than a tile, and both of its halves are drawn and meet.
    ///
    /// The claim has three parts and two of them are what a single-sprite test cannot make.
    /// The canopy is not merely drawn: it is drawn **directly on top of** the trunk it belongs
    /// to — a tree whose top half landed one cell to the side would be two sprites in the right
    /// frame and no tree in the picture — and the two together are **taller than two tiles**,
    /// which is the size at which a tree reads as a tree rather than as a shrub on a stick
    /// beside a patch of daisies.
    #[test]
    fn a_tree_is_planted_as_two_cells_that_meet() {
        let level = Level::starter("wood", 60, 12);
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let scale = Camera::fit(&level, viewport, 24.0).scale();
        let bases = drawn_sprites(&frame, &["tree_a_base", "tree_b_base"]);
        let tops = drawn_sprites(&frame, &["tree_a_top", "tree_b_top"]);
        assert!(
            !bases.is_empty(),
            "a sixty-tile floor grew no trees at all, so this test proves nothing"
        );
        assert_eq!(
            bases.len(),
            tops.len(),
            "{} trunk(s) and {} canopy/canopies: a tree drew half of itself",
            bases.len(),
            tops.len()
        );
        for base in &bases {
            let Some(top) = tops
                .iter()
                .find(|top| (top.min.x - base.min.x).abs() <= 1.0)
            else {
                panic!("a trunk has no canopy over it at all: trunk {base:?}, canopies {tops:?}");
            };
            assert!(
                (top.max.y - base.min.y).abs() <= 1.0,
                "a tree has a gap through it: trunk {base:?} ends at {} and its canopy {top:?} \
                 starts at {}",
                base.max.y,
                top.min.y
            );
            assert!(
                base.height() + top.height() >= scale * 3.0,
                "a tree stands {} points at {scale} points a tile: three tiles is the height at \
                 which a tree is not a shrub",
                base.height() + top.height()
            );
        }
    }

    /// The flag is painted in the cell *above* the goal's own tile — and that cell is one of
    /// the two the eraser answers for.
    ///
    /// The bug this pins was found by playing with the editor: a goal could be placed but not
    /// deleted, because the click everyone aims at is the flag and the flag is drawn one tile
    /// above the goal's anchor, on a cell no goal claimed. The claim here is a
    /// *correspondence* between two halves that live in different crates — the cells
    /// `Level::goal_covers` accepts and the cells the picture actually fills — so it cannot
    /// hold if either half moves: draw the flag one cell lower and the tile read back changes,
    /// narrow `goal_covers` to the anchor and the flag's own cell stops answering.
    #[test]
    fn the_flag_is_drawn_in_the_cell_the_eraser_addresses() {
        let level = Level::starter("one-flag", 40, 12);
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let camera = Camera::fit(&level, viewport, 24.0);
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );

        let flags = drawn_sprites(&frame, &FLAG_FRAMES);
        let poles = drawn_sprites(&frame, &["goal_bottom"]);
        assert_eq!(flags.len(), 1, "the starter level has exactly one flag");
        assert_eq!(poles.len(), 1, "and exactly one pole to hang it on");
        assert!(
            flags[0].center().y < poles[0].center().y,
            "the flag is drawn above the pole's foot: flag {:?}, pole {:?}",
            flags[0],
            poles[0]
        );

        // The cells the picture fills, read back through the camera it was drawn with...
        let cells = [flags[0], poles[0]].map(|rect| {
            let t = camera.to_tile(rect.center(), viewport);
            (t.x.floor() as i32, t.y.floor() as i32)
        });
        // ...are cells the document answers a click on, which is what makes the flag deletable.
        for (tx, ty) in cells {
            assert!(
                level.goal_covers(tx, ty),
                "the goal art in ({tx}, {ty}) is a cell no click can reach"
            );
        }
        let (ax, ay) = level.goal_tiles()[0];
        assert_eq!(cells[1], (ax, ay), "the pole's foot stands on the anchor");
        assert_eq!(cells[0], (ax, ay - 1), "and the cloth hangs one cell above it");
        assert!(
            !level.goal_covers(ax, ay - 2),
            "a third cell up is open air, not the goal"
        );
    }

    /// The scenery is drawn at its own size, and the sizes are in the right order.
    ///
    /// The complaint this exists to prevent is a picture in which nothing on a lawn is any
    /// bigger than anything else on it — every prop was drawn at exactly one cell, so a patch of
    /// daisies was as wide as a tree's canopy and a mushroom stood as tall as a stump. So the
    /// test does not check the numbers, it checks the *order* and the gaps: a tree is more than a
    /// tile, every bush, tuft and flower is less than one, a flower is the smallest thing on the
    /// lawn, and a tree is at least twice a bush. That is the comparison an eye makes, and it is
    /// the one that was wrong.
    #[test]
    fn the_scenery_is_drawn_at_the_size_its_kind_is() {
        let size = |name: &str| prop_size(name);
        assert_eq!(
            (2.0 * TREE_SIZE).ceil() as i32,
            TREE_CELLS,
            "a tree is drawn {} tiles tall and asks the level for room for {TREE_CELLS} cells",
            2.0 * TREE_SIZE
        );
        assert!(
            size("tree_a") > 1.0 && size("tree_b_base") > 1.0,
            "a tree is a cell of art worth {} tiles, and a tile is a shrub",
            size("tree_a")
        );
        for small in [
            "bush_c",
            "tuft_a",
            "flowers_pink",
            "mushroom_pair",
            "pebble",
        ] {
            assert!(
                size(small) < 1.0,
                "{small} is drawn at {} tiles: the small dressing has to be smaller than the \
                 tile it stands in",
                size(small)
            );
        }
        assert!(
            size("flowers") < size("bush_a") && size("flowers") <= size("tuft_a"),
            "a patch of daisies ({}) is the smallest thing on the lawn, and a bush ({}) is not",
            size("flowers"),
            size("bush_a")
        );
        assert!(
            size("tree_a") >= size("bush_a") * 2.0,
            "a tree ({}) is not twice a bush ({}): the two are what an eye compares",
            size("tree_a"),
            size("bush_a")
        );
        // And every family answers for itself: a new colour of daisy is a daisy.
        for name in ["flowers", "flowers_pink", "flowers_white"] {
            assert_eq!(
                size(name),
                size("flowers"),
                "{name} is not drawn as a flower"
            );
        }
        for name in ["bush_a", "bush_c", "bush_e"] {
            assert_eq!(size(name), size("bush_a"), "{name} is not drawn as a bush");
        }
    }

    /// A prop is drawn on the ground, not in the middle of its cell.
    ///
    /// The size rule above is only half of what makes a small prop small: a daisy patch drawn
    /// at a third of a tile *floating* in the middle of its cell would hover half a tile over the
    /// lawn it is growing out of. So the bottom of the rect is pinned to the bottom of the cell
    /// and the art is measured up from it, whatever the size — the same rule for a tuft and for a
    /// tree, which is what lets one of them overflow the cell it was placed in.
    #[test]
    fn scenery_stands_on_the_bottom_of_its_cell() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let camera = Camera::fit(&level, viewport, 24.0);
        let scale = camera.scale();
        let sprites = Sprites::load(&atlas()).expect("the shipped tileset loads");
        let mut seen = 0;
        for ty in 0..level.height {
            for tx in 0..level.width {
                let Some(prop) = scenery_sprite(&sprites, &level, tx, ty, 0.0) else {
                    continue;
                };
                // The rects this prop could be: the right size, in the right column. Read off
                // the frame rather than rebuilt from the cell, so the test cannot agree with
                // the drawing code by repeating its arithmetic.
                let cell = camera.to_screen(tx as f32, (ty - 1) as f32, viewport);
                let side = (scale * prop.size).round().max(2.0);
                let foot = cell.y + scale;
                let mine: Vec<Rect> = frame
                    .sprites
                    .iter()
                    .map(|(screen, _)| *screen)
                    .filter(|rect| {
                        (rect.height() - side).abs() <= 0.5
                            && (rect.center().x - (cell.x + scale * 0.5)).abs() <= 1.0
                    })
                    .collect();
                assert!(
                    !mine.is_empty(),
                    "a prop at ({tx},{ty}) worth {} tiles is not in the frame at all",
                    prop.size
                );
                seen += 1;
                assert!(
                    mine.iter().any(|rect| (rect.max.y - foot).abs() <= 1.0),
                    "a prop at ({tx},{ty}) is drawn standing {:.0} points off the ground line: \
                     {mine:?} against {foot}",
                    mine[0].max.y - foot
                );
                if prop.crown.is_some() {
                    assert!(
                        mine.iter()
                            .any(|rect| (rect.max.y - (foot - side)).abs() <= 1.0),
                        "the canopy of the tree at ({tx},{ty}) does not start where its trunk ends"
                    );
                }
                if prop.size > 1.0 {
                    assert!(
                        mine.iter().any(|rect| rect.min.y < cell.y),
                        "a prop worth {} tiles is drawn entirely inside the cell it stands in",
                        prop.size
                    );
                }
            }
        }
        assert!(seen > 2, "no props were measured at all: {seen}");
    }

    /// A tree needs two cells of air; where the level does not have them, it is a bush.
    ///
    /// The rule is about *room*, so it is checked cell by cell: the same floor under ceilings one
    /// tile higher each time, and trees appear only at the height a tree is actually drawn at.
    /// What must never happen is the trunk planted anyway with its canopy inside the ceiling
    /// above it, which is a tree growing through rock — and the room a tree needs is not a
    /// number of its own, it is the height it is drawn at, so a ceiling that has always been
    /// fine for a two-tile tree stops being fine the moment the tree is three tiles.
    #[test]
    fn a_tree_is_a_bush_where_there_is_no_room_for_a_canopy() {
        let sprites = Sprites::load(&atlas()).expect("the shipped tileset loads");
        // A floor with a ceiling `air` tiles above it, and how much of it grew trees.
        let count = |air: i32| -> (usize, usize) {
            let mut level = Level::starter("roof", 200, 14);
            let floor = level.height - 1;
            let ceiling = floor - air - 1;
            for tx in 0..level.width {
                level.set_tile(tx, ceiling, Tile::Solid);
            }
            let (mut props, mut trees) = (0, 0);
            for tx in 0..level.width {
                if let Some(prop) = scenery_sprite(&sprites, &level, tx, floor, 0.0) {
                    props += 1;
                    if prop.crown.is_some() {
                        trees += 1;
                    }
                }
            }
            (props, trees)
        };

        // One cell of air: a corridor, and only bushes in it.
        let (props, trees) = count(1);
        assert!(
            props > 40,
            "the corridor should still be dressed: {props} props"
        );
        assert_eq!(
            trees, 0,
            "a tree was planted with its canopy inside the ceiling"
        );

        // Two cells: still a slot rather than a surface. This is the case that changed when the
        // tree did — a two-tile tree fitted under this roof and a three-tile one does not, and
        // the canopy that would be drawn through the ceiling is what says so.
        let (_, trees) = count(2);
        assert_eq!(
            trees, 0,
            "a tree was planted under a ceiling too low for the size it is drawn at"
        );

        // And at the height a tree really is, the same floor grows trees again: the rule is
        // about the room and not about the tile.
        let (_, trees) = count(3);
        assert!(
            trees > 0,
            "no tree in two hundred tiles with the room for one"
        );
    }

    /// The butterflies are ambient life: in the air, in empty tiles, and moving.
    ///
    /// Every rule here is a rule the *other* props do not have, which is the whole reason
    /// the layer exists. A bush belongs to a surface tile; a butterfly belongs to no tile,
    /// so what it has instead is that it may only be drawn where the level is empty — a
    /// creature inside a wall is the same lie a prop inside one would be — and that it is
    /// nowhere in the editor, because a document says nothing about a butterfly.
    #[test]
    fn the_butterflies_drift_only_where_the_air_is() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let camera = Camera::fit(&level, viewport, 24.0);
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let flies = drawn_sprites(&frame, &BUTTERFLIES);
        assert!(
            !flies.is_empty(),
            "no butterflies in a level of open sky: the ambient layer drew nothing"
        );
        for fly in &flies {
            let at = camera.to_tile(fly.center(), viewport);
            let (tx, ty) = (at.x.floor() as i32, at.y.floor() as i32);
            assert_eq!(
                level.tile(tx, ty),
                Some(Tile::Empty),
                "a butterfly is drawn at ({tx}, {ty}), which is not empty air"
            );
            assert!(
                !solid_at(&level, tx, ty - 1),
                "a butterfly is drawn at ({tx}, {ty}), where the ceiling is on top of it"
            );
            // And it is over ground rather than up in the level's air: the layer used to
            // draw in any empty tile with open sky above it, which put them ten tiles up as
            // readily as one, and that is what made them read as birds.
            assert!(
                solid_at(&level, tx, ty + 1),
                "a butterfly is drawn at ({tx}, {ty}) with nothing under it: they live just \
                 above a surface"
            );
            // Small, too: a fifth of a tile against the cell-and-a-bit it used to fill.
            assert!(
                fly.width() <= camera.scale() * BUTTERFLY_SIZE + 1.0,
                "a butterfly is drawn {:.0} points across at {} points a tile",
                fly.width(),
                camera.scale()
            );
        }

        // A wingbeat is the clock's, and every wing beats together: at rest the first
        // frame is up, and a quarter of a beat later the second one is.
        let atlas = atlas();
        assert!(
            frame.draws(atlas.uv("butterfly_a_1").expect("in the atlas")),
            "the first frame of the wingbeat is missing"
        );
        assert!(
            !frame.draws(atlas.uv("butterfly_a_2").expect("in the atlas")),
            "both wingbeats are drawn at once"
        );
        let later = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.25,
            RenderOpts::playing(),
        );
        assert!(
            later.draws(atlas.uv("butterfly_a_2").expect("in the atlas")),
            "the wingbeat never reaches its second frame"
        );

        // And they have drifted: the same butterflies, in the same order, not in the same
        // place. A creature that only flapped would be a sprite on a tile after all. Sampled
        // two seconds apart rather than two wingbeats, because the wander is a hover now: at
        // a fifth of a tile over the ground, a quarter of a beat moves one by less than the
        // pixel the drawing is rounded to, and a test that asked for that would be asking the
        // drift to be big rather than asking it to be there.
        let drifted = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            2.0,
            RenderOpts::playing(),
        );
        let moved = drawn_sprites(&drifted, &BUTTERFLIES);
        assert_eq!(
            flies.len(),
            moved.len(),
            "the clock changed how many there are"
        );
        assert!(
            flies
                .iter()
                .zip(&moved)
                .any(|(a, b)| (a.min.x - b.min.x).abs() > 0.5 || (a.min.y - b.min.y).abs() > 0.5),
            "nothing moved over two seconds of clock"
        );

        // The editor is a document, and a document has no butterflies in it.
        let editor = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Checker,
            0.0,
            RenderOpts::editing(),
        );
        assert!(
            drawn_sprites(&editor, &BUTTERFLIES).is_empty(),
            "the editor grew ambient life"
        );
    }

    /// Every drawn quad that is one of the material seams, as `(name, screen rect)`.
    fn drawn_seams(frame: &Frame) -> Vec<(String, Rect)> {
        let atlas = atlas();
        let mut out = Vec::new();
        for (screen, drawn) in &frame.sprites {
            for name in atlas.names() {
                if !name.starts_with("blend_") {
                    continue;
                }
                if atlas.uv(name).map(|uv| uv == *drawn).unwrap_or(false) {
                    out.push((name.to_string(), *screen));
                }
            }
        }
        out
    }

    /// The look and side a seam name spells, read out of the renderer's own table.
    fn seam_of(name: &str) -> (Look, usize) {
        for look in [
            Look::Grass,
            Look::Dirt,
            Look::Stone,
            Look::Beach,
            Look::Packed,
            Look::Sandstone,
            Look::Brick,
            Look::Masonry,
            Look::Wood,
            Look::Timber,
        ] {
            for (index, (_side, _dx, _dy)) in SIDES.iter().enumerate() {
                if BLEND[look.index()][index] == name {
                    return (look, index);
                }
            }
        }
        panic!("{name} is not a seam the view can draw");
    }

    /// A floor built to contain every seam the level format can make.
    ///
    /// Six rows deep (8..=13) in fourteen column ranges, and the ranges exist to put the
    /// *vertical* seams in it as well as the horizontal ones: a sand surface lying on soil, soil
    /// lying on sand, and rock over rock of the other material — which is the only arrangement
    /// the rock looks' `_t` bands can be reached from at all, since a rock tile three rows down
    /// has rock under it unless the material changes. Rows 0..8 stay empty so the surface looks
    /// are crowns.
    ///
    /// Brick and plank arrive with their own materials, so the floor carries their stacks too:
    /// a brick wall beside a bank of soil reaches the brick and masonry bands, and a deck of
    /// planks beside it reaches the wood and timber ones. The four columns that mix a crown into
    /// a stack are the only way to reach a *crown* material's `_t` band — a brick tile at the
    /// surface with soil under it — and a buried tile of the other material the only way to reach
    /// the deep looks' `_b` bands.
    fn seam_probe() -> Level {
        let mut level = Level::starter("seam-probe", 70, 14);
        let ranges: [(i32, i32, [Tile; 6]); 14] = [
            // Rows 8..=13, top to bottom.
            (0, 4, [Tile::Solid; 6]),
            (5, 9, [Tile::Sand; 6]),
            (
                10,
                14,
                [
                    Tile::Sand,
                    Tile::Sand,
                    Tile::Sand,
                    Tile::Sand,
                    Tile::Solid,
                    Tile::Solid,
                ],
            ),
            (
                15,
                19,
                [
                    Tile::Solid,
                    Tile::Solid,
                    Tile::Sand,
                    Tile::Sand,
                    Tile::Solid,
                    Tile::Solid,
                ],
            ),
            (
                20,
                24,
                [
                    Tile::Sand,
                    Tile::Solid,
                    Tile::Solid,
                    Tile::Solid,
                    Tile::Solid,
                    Tile::Solid,
                ],
            ),
            (
                25,
                29,
                [
                    Tile::Solid,
                    Tile::Sand,
                    Tile::Sand,
                    Tile::Sand,
                    Tile::Sand,
                    Tile::Sand,
                ],
            ),
            (
                30,
                34,
                [
                    Tile::Solid,
                    Tile::Solid,
                    Tile::Solid,
                    Tile::Solid,
                    Tile::Solid,
                    Tile::Sand,
                ],
            ),
            // A brick stack on its own, then a bank of soil beside it: brick/masonry `_l` and
            // `_r` on both sides of the two seams, at the crown and at depth.
            (35, 39, [Tile::Brick; 6]),
            (40, 44, [Tile::Solid; 6]),
            // A brick crown over soil: the soil tile below it carries `blend_brick_t`, and the
            // buried soil tile below *that* carries `blend_masonry_t` and `blend_masonry_b`.
            (
                45,
                49,
                [
                    Tile::Brick,
                    Tile::Solid,
                    Tile::Brick,
                    Tile::Brick,
                    Tile::Solid,
                    Tile::Brick,
                ],
            ),
            // The same three arrangements in planks: a plank deck beside soil for the wood and
            // timber verticals, and the mixed stack for the crown's `_t` and the deep `_b`.
            (50, 54, [Tile::Plank; 6]),
            (55, 59, [Tile::Sand; 6]),
            (
                60,
                64,
                [
                    Tile::Plank,
                    Tile::Solid,
                    Tile::Plank,
                    Tile::Plank,
                    Tile::Solid,
                    Tile::Plank,
                ],
            ),
            // And the two materials beside each other, so a brick wall abutting a deck has a seam
            // between two built materials rather than only against soil.
            (65, 69, [Tile::Brick; 6]),
        ];
        for (first, last, rows) in ranges {
            for tx in first..=last {
                for (offset, material) in rows.iter().enumerate() {
                    level.set_tile(tx, 8 + offset as i32, *material);
                }
            }
        }
        level
    }

    /// Two materials meeting: each one is drawn a few pixels into the other's tile.
    ///
    /// The seam is the whole point of the bands, so the assertion is about *where* they land
    /// rather than about how they look: a lawn with a beach in the middle of it has four tiles
    /// along the two vertical seams, and each of them carries the neighbouring material across
    /// exactly the edge it meets it on — the surface tiles carrying the two crowns and the
    /// buried tiles the two bodies, which is the depth rule showing through the seams as well as
    /// through the tiles.
    #[test]
    fn a_ground_seam_blends_both_ways() {
        let mut level = Level::starter("seam", 30, 10);
        for ty in [level.height - 2, level.height - 1] {
            for tx in 0..level.width {
                let material = if (10..20).contains(&tx) {
                    Tile::Sand
                } else {
                    Tile::Solid
                };
                level.set_tile(tx, ty, material);
            }
        }
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );

        // The beach is two rows tall, so each seam edge is two tiles and carries two bands — one
        // drawn on the lawn, one on the beach — and the surface pair is the two crowns while the
        // buried pair is the two bodies. The two seams face opposite ways, so each of the eight
        // names appears exactly once: eight bands, which is the number of material edges here.
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (name, _) in drawn_seams(&frame) {
            *counts.entry(name).or_default() += 1;
        }
        let expected: BTreeMap<String, usize> = [
            ("blend_beach_l", 1),
            ("blend_beach_r", 1),
            ("blend_grass_l", 1),
            ("blend_grass_r", 1),
            ("blend_packed_l", 1),
            ("blend_packed_r", 1),
            ("blend_dirt_l", 1),
            ("blend_dirt_r", 1),
        ]
        .into_iter()
        .map(|(name, n)| (name.to_string(), n))
        .collect();
        assert_eq!(
            counts, expected,
            "the seams of a two-row beach in a lawn are not the ones that were drawn"
        );

        // And nothing crept in from above or below: grass over the dirt under it is one
        // material, and the crown's own ragged last row is what joins them.
        for (name, _) in drawn_seams(&frame) {
            assert!(
                name.ends_with("_l") || name.ends_with("_r"),
                "{name} is a vertical seam, and this level has no vertical one"
            );
        }
    }

    /// Each band is drawn in the tile on the *near* side of its seam, and carries the material
    /// that is really across it.
    ///
    /// The invariant a band can break that a sprite cannot: the atlas is a picture, and only the
    /// call site decides which tile it lands in. Read back from the frame — the drawn rectangle,
    /// the camera's inverse, the level's own tiles — so the test cannot agree with the code by
    /// sharing its arithmetic.
    #[test]
    fn a_seam_carries_the_material_across_the_edge_it_lies_on() {
        let level = seam_probe();
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let camera = Camera::fit(&level, viewport, 24.0);
        let seams = drawn_seams(&frame);
        assert!(!seams.is_empty(), "the probe floor drew no seams at all");

        for (name, rect) in &seams {
            let (look, index) = seam_of(name);
            let (side, dx, dy) = SIDES[index];
            let at = camera.to_tile(rect.min, viewport);
            let (tx, ty) = (at.x.floor() as i32, at.y.floor() as i32);
            // The tile the band is drawn in must not be made of the material it carries.
            let mine = look_at(&level, tx, ty).expect("a band is drawn over ground");
            assert_ne!(
                mine.family(),
                look.family(),
                "{name} is drawn over a {mine:?} tile, which is its own material"
            );
            // And the tile across the edge it lies on must be.
            assert_eq!(
                look_at(&level, tx + dx, ty + dy),
                Some(look),
                "{name} lies on the {side} edge of ({tx}, {ty}), where there is no {look:?}"
            );
        }

        // Nothing was drawn twice for one seam and nothing was missed: the count is the number
        // of tile edges in the level that join two materials, computed here from the tiles.
        let mut expected = 0;
        for ty in 0..level.height {
            for tx in 0..level.width {
                let Some(mine) = look_at(&level, tx, ty) else {
                    continue;
                };
                for (_side, dx, dy) in SIDES {
                    let theirs = look_at(&level, tx + dx, ty + dy);
                    if theirs.is_some_and(|theirs| theirs.family() != mine.family()) {
                        expected += 1;
                    }
                }
            }
        }
        assert_eq!(
            seams.len(),
            expected,
            "{expected} material edges, {} bands drawn",
            seams.len()
        );
    }

    /// Every band the atlas ships is asked for by some level: art with no job is art nobody can
    /// tell is wrong.
    #[test]
    fn every_seam_the_atlas_ships_is_drawn_by_a_level() {
        let atlas = atlas();
        let level = seam_probe();
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let drawn: std::collections::BTreeSet<String> =
            drawn_seams(&frame).into_iter().map(|(n, _)| n).collect();
        let shipped: Vec<String> = atlas
            .names()
            .filter(|name| name.starts_with("blend_"))
            .map(|name| name.to_string())
            .collect();
        assert_eq!(shipped.len(), 38, "the atlas ships 38 seams");
        let missed: Vec<&String> = shipped.iter().filter(|n| !drawn.contains(*n)).collect();
        assert!(
            missed.is_empty(),
            "no level in this test asks for {missed:?}: dead seam art"
        );

        // And the two bands that cannot exist are the two the table leaves empty, so the gap in
        // the atlas and the gap in the renderer are the same gap.
        let looks = [
            Look::Grass,
            Look::Dirt,
            Look::Stone,
            Look::Beach,
            Look::Packed,
            Look::Sandstone,
            Look::Brick,
            Look::Masonry,
            Look::Wood,
            Look::Timber,
        ];
        for look in looks {
            for (index, (_side, _dx, _dy)) in SIDES.iter().enumerate() {
                let name = BLEND[look.index()][index];
                let in_atlas = !name.is_empty() && shipped.iter().any(|s| s == name);
                assert_eq!(
                    name.is_empty(),
                    !in_atlas,
                    "{look:?} on side {index} disagrees with the shipped atlas"
                );
            }
        }
    }

    /// One material in a level means no seams in the frame: a floor that is all soil travels with
    /// four bands' worth of atlas it never draws.
    #[test]
    fn a_floor_of_one_material_has_no_seams() {
        let level = shipped("l1-flat-run");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
        let frame = headless_frame(
            &level,
            viewport.size(),
            Backdrop::Sky,
            0.0,
            RenderOpts::playing(),
        );
        let seams = drawn_seams(&frame);
        assert!(
            seams.is_empty(),
            "L1 is one material: {:?}",
            seams.into_iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
    }

    /// Every prop *family* the table can name is really dressed onto the shipped pack.
    ///
    /// The probe floor above asks whether a name can be *reached*; this asks whether it is.
    /// The difference is the whole of finding 36. The roll used to be `tile_hash(..) % 100`,
    /// and the low bits of that hash move in step along a horizontal run of floor tiles: over
    /// the pack's hundred distinct surface tiles, `% 100` never once landed in 15..=17, which
    /// is the flowers' band. Six levels, eight daisy patches paid for out of the table, and
    /// not one flower drawn anywhere in the game — and neither the art suite nor the probe
    /// floor could see it, because both of those ask about a hash rather than about a level.
    ///
    /// Families rather than names on purpose: which *shape* of bush a tile wants is the sway
    /// hash's business and asking for each of the six would be asking for the table's ranges
    /// again. What must hold is that a viewer of the shipped pack sees every kind of thing the
    /// scatter can draw.
    #[test]
    fn every_prop_family_is_dressed_onto_the_shipped_pack() {
        let atlas = atlas();
        let sprites = Sprites::load(&atlas).unwrap();
        let name_of = |rect: Rect| -> String {
            atlas
                .names()
                .find(|name| atlas.uv(name).map(|uv| uv == rect).unwrap_or(false))
                .unwrap_or("<none>")
                .to_string()
        };
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        let mut drawn = 0usize;
        for slug in [
            "l1-flat-run",
            "l2-first-gap",
            "l3-gauntlet",
            "l4-rhythm",
            "l5-tightrope",
            "l6-marathon",
        ] {
            let level = shipped(slug);
            for ty in 0..level.height {
                for tx in 0..level.width {
                    if !solid_at(&level, tx, ty)
                        || ground_mask(&level, tx, ty) & 1 == 0
                        || ty == 0
                        || solid_at(&level, tx, ty - 1)
                    {
                        continue;
                    }
                    if let Some(prop) = scenery_sprite(&sprites, &level, tx, ty, 0.0) {
                        drawn += 1;
                        let name = name_of(prop.sprite);
                        // `tree_a_base` is the family `tree`; everything else is a family
                        // all by itself.
                        let family = name.split('_').next().unwrap_or("<none>").to_string();
                        seen.insert(family);
                    }
                }
            }
        }
        assert!(drawn > 50, "the pack is barely dressed: {drawn} props");
        for family in [
            "bush", "tuft", "flowers", "tree", "rock", "mushroom", "pebble", "log", "stump",
            "reeds",
        ] {
            assert!(
                seen.contains(family),
                "no {family} is drawn anywhere in the shipped levels: the table pays for \
                 {family} and the level never sees one. Dressed families: {seen:?}"
            );
        }
    }
}
