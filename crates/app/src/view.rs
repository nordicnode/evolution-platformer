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

use egui::{Color32, Mesh, Painter, Pos2, Rect, TextureId, Vec2};
use sim::reach::ReachReport;
use sim::{Level, Sim, Tile};

use crate::atlas::Atlas;

/// Sky, top to bottom of the window. Sampled into a two-triangle gradient.
const SKY_TOP: Color32 = Color32::from_rgb(86, 160, 232);
const SKY_BOTTOM: Color32 = Color32::from_rgb(178, 214, 240);
/// The old-school checker behind the level, so the play area is obvious.
const BACKDROP_DARK: Color32 = Color32::from_rgb(196, 220, 238);
const GRID_LINE: Color32 = Color32::from_rgba_premultiplied(24, 30, 40, 26);
const BOUNDS_LINE: Color32 = Color32::from_rgb(40, 52, 68);
const SPAWN_LINE: Color32 = Color32::from_rgb(120, 92, 30);
const UNREACHED_TINT: Color32 = Color32::from_rgba_premultiplied(90, 20, 24, 96);
const CULPRIT_LINE: Color32 = Color32::from_rgb(226, 154, 40);
const FRONTIER_LINE: Color32 = Color32::from_rgb(210, 90, 60);
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
}

impl RenderOpts {
    /// The editor's view: grid, spawn marker, bounds.
    pub fn editing() -> RenderOpts {
        RenderOpts {
            grid: true,
            spawn_marker: true,
            bounds: true,
        }
    }

    /// The play view: no grid, no editing furniture.
    pub fn playing() -> RenderOpts {
        RenderOpts {
            grid: false,
            spawn_marker: false,
            bounds: true,
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
    pub fn fit(level: &Level, viewport: Rect, margin: f32) -> Camera {
        let avail = Vec2::new(
            (viewport.width() - margin * 2.0).max(1.0),
            (viewport.height() - margin * 2.0).max(1.0),
        );
        let zoom = (avail.x / level.width.max(1) as f32)
            .min(avail.y / level.height.max(1) as f32)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        Camera {
            centre: Vec2::new(level.width as f32 * 0.5, level.height as f32 * 0.5),
            zoom,
        }
    }

    /// Top-left corner of tile `(tx, ty)` in screen space.
    pub fn to_screen(self, tx: f32, ty: f32, viewport: Rect) -> Pos2 {
        let c = viewport.center();
        Pos2::new(
            c.x + (tx - self.centre.x) * self.zoom,
            c.y + (ty - self.centre.y) * self.zoom,
        )
    }

    /// Tile-space position of a screen point. The exact inverse of
    /// [`Camera::to_screen`]; fractional inside a tile.
    pub fn to_tile(self, screen: Pos2, viewport: Rect) -> Vec2 {
        let c = viewport.center();
        Vec2::new(
            (screen.x - c.x) / self.zoom + self.centre.x,
            (screen.y - c.y) / self.zoom + self.centre.y,
        )
    }

    /// Screen rectangle of one tile, snapped to whole points.
    pub fn tile_rect(&self, tx: i32, ty: i32, viewport: Rect) -> Rect {
        let min = self.to_screen(tx as f32, ty as f32, viewport);
        let size = self.zoom.max(1.0);
        Rect::from_min_size(
            Pos2::new(min.x.round(), min.y.round()),
            Vec2::splat(size.ceil()),
        )
    }

    /// Screen rectangle of a world-space box given in tiles (`sim`'s units).
    pub fn world_rect(&self, x: f32, y: f32, w: f32, h: f32, viewport: Rect) -> Rect {
        let min = self.to_screen(x, y, viewport);
        Rect::from_min_max(
            Pos2::new(min.x.round(), min.y.round()),
            Pos2::new(
                (min.x + w * self.zoom).round(),
                (min.y + h * self.zoom).round(),
            ),
        )
    }

    /// Zoom about a screen point, keeping the tile under it in place.
    pub fn zoom_at(&mut self, factor: f32, anchor: Pos2, viewport: Rect) {
        let before = self.to_tile(anchor, viewport);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_tile(anchor, viewport);
        self.centre += before - after;
    }

    /// Keep the camera centre inside the level, allowing a margin so a level edge is
    /// reachable but the view cannot be lost in empty space.
    pub fn clamp_to(&mut self, level: &Level, keep: f32) {
        self.centre.x = self
            .centre
            .x
            .clamp(-keep, level.width as f32 + keep);
        self.centre.y = self
            .centre
            .y
            .clamp(-keep, level.height as f32 + keep);
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

/// The atlas rectangles the renderer needs, resolved once per frame instead of once
/// per tile.
#[derive(Clone, Copy, Debug)]
pub struct Sprites {
    pub grass: Rect,
    pub grass_left: Rect,
    pub grass_right: Rect,
    pub dirt: Rect,
    pub dirt_left: Rect,
    pub dirt_right: Rect,
    pub spike: Rect,
    pub player_stand: Rect,
    pub player_run_a: Rect,
    pub player_run_b: Rect,
    pub player_jump: Rect,
    pub walker_a: Rect,
    pub walker_b: Rect,
    pub walker_dead: Rect,
    pub goal_top: Rect,
    pub goal_bottom: Rect,
    pub spawn_marker: Rect,
}

impl Sprites {
    /// Resolve every name from the atlas, or say which are missing.
    pub fn load(atlas: &Atlas) -> Result<Sprites, String> {
        let missing = atlas.missing_required();
        if !missing.is_empty() {
            return Err(format!(
                "{} is missing: {}",
                atlas.source().display(),
                missing.join(", ")
            ));
        }
        Ok(Sprites {
            grass: atlas.uv("grass")?,
            grass_left: atlas.uv("grass_left")?,
            grass_right: atlas.uv("grass_right")?,
            dirt: atlas.uv("dirt")?,
            dirt_left: atlas.uv("dirt_left")?,
            dirt_right: atlas.uv("dirt_right")?,
            spike: atlas.uv("spike")?,
            player_stand: atlas.uv("player_stand")?,
            player_run_a: atlas.uv("player_run_a")?,
            player_run_b: atlas.uv("player_run_b")?,
            player_jump: atlas.uv("player_jump")?,
            walker_a: atlas.uv("walker_a")?,
            walker_b: atlas.uv("walker_b")?,
            walker_dead: atlas.uv("walker_dead")?,
            goal_top: atlas.uv("goal_top")?,
            goal_bottom: atlas.uv("goal_bottom")?,
            spawn_marker: atlas.uv("spawn_marker")?,
        })
    }
}

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
}

/// Is a *real* solid tile here? Not [`Level::is_solid`], which reports the invisible
/// side walls: an art decision wants the level's edge to look like open air.
pub fn solid_at(level: &Level, tx: i32, ty: i32) -> bool {
    level.tile(tx, ty) == Some(Tile::Solid)
}

/// Which sprite draws the solid tile at `(tx, ty)`.
///
/// Terrain is picked by what surrounds it, so a level needs no per-tile art
/// decisions: grass where the sky shows, dirt where it is buried, and an edge
/// variant where a side is exposed.
pub fn tile_sprite(sprites: &Sprites, level: &Level, tx: i32, ty: i32) -> Rect {
    let open_above = !solid_at(level, tx, ty - 1);
    let open_left = !solid_at(level, tx - 1, ty);
    let open_right = !solid_at(level, tx + 1, ty);
    match (open_above, open_left, open_right) {
        (true, true, _) => sprites.grass_left,
        (true, _, true) => sprites.grass_right,
        (true, _, _) => sprites.grass,
        (false, true, _) => sprites.dirt_left,
        (false, _, true) => sprites.dirt_right,
        (false, _, _) => sprites.dirt,
    }
}

/// The two tiles a goal box covers, upper (flag) and lower (pole).
pub fn goal_cells(goal: sim::Rect) -> ((i32, i32), (i32, i32)) {
    let top = (goal.x.floor() as i32, goal.y.floor() as i32);
    (top, (top.0, top.1 + 1))
}

/// Sky gradient plus a checkerboard, so the level's own shape is legible.
pub fn draw_background(painter: &Painter, viewport: Rect, camera: &Camera, level: &Level) {
    let mut mesh = Mesh::default();
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(viewport.left_top(), SKY_TOP);
    mesh.colored_vertex(viewport.right_top(), SKY_TOP);
    mesh.colored_vertex(viewport.right_bottom(), SKY_BOTTOM);
    mesh.colored_vertex(viewport.left_bottom(), SKY_BOTTOM);
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
    painter.add(mesh);

    // Checker cells the size of two tiles, drawn over the sky where the level is not.
    let size = (camera.zoom * 2.0).max(8.0_f32);
    let (tx0, _tx1, ty0, _ty1) = camera.visible_tiles(level, viewport);
    let start = camera.to_screen(tx0 as f32, ty0 as f32, viewport);
    let mut y = start.y - (start.y - viewport.top()) % size - size;
    while y < viewport.bottom() {
        let mut x = start.x - (start.x - viewport.left()) % size - size;
        while x < viewport.right() {
            let cell = Rect::from_min_size(Pos2::new(x, y), Vec2::splat(size));
            let even = (((x / size).round() as i64 + (y / size).round() as i64) % 2) == 0;
            if even {
                painter.rect_filled(cell, egui::CornerRadius::ZERO, BACKDROP_DARK);
            }
            x += size;
        }
        y += size;
    }
}

/// Draw the level's tiles, its goal flags, and (optionally) its grid and bounds.
///
/// `reach` is an index into [`ReachReport::reached`] when the caller wants the
/// validation overlay; pass `None` for a plain view.
pub fn draw_level(
    sheet: Sheet<'_>,
    level: &Level,
    opts: RenderOpts,
    reach: Option<&ReachReport>,
) {
    let painter = sheet.painter;
    let (tx0, tx1, ty0, ty1) = sheet.camera.visible_tiles(level, sheet.viewport);

    for ty in ty0..ty1 {
        for tx in tx0..tx1 {
            let rect = sheet.tile(tx, ty);
            match level.tile(tx, ty) {
                Some(Tile::Solid) => {
                    sheet.image(rect, tile_sprite(sheet.sprites, level, tx, ty));
                }
                Some(Tile::Spike) => {
                    sheet.image(rect, sheet.sprites.spike);
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

    // Goals: the flag above the pole, in the two tiles the box covers.
    for goal in &level.goals {
        let (top, bottom) = goal_cells(*goal);
        sheet.image(sheet.tile(top.0, top.1), sheet.sprites.goal_top);
        sheet.image(sheet.tile(bottom.0, bottom.1), sheet.sprites.goal_bottom);
    }

    if opts.spawn_marker {
        let (sx, sy) = level.spawn_tile();
        let rect = sheet.tile(sx, sy);
        sheet.image(rect, sheet.sprites.spawn_marker);
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
            sheet
                .camera
                .to_screen(platform.x0 as f32, platform.surface_row as f32, sheet.viewport),
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

/// Draw a live episode: the level, the player with the pose its state implies, and
/// every walker, alive or squashed.
pub fn draw_sim(sheet: Sheet<'_>, sim: &Sim, opts: RenderOpts, clock: f32) {
    draw_level(sheet, sim.level(), opts, None);
    draw_agents(sheet, sim, clock, Color32::WHITE);
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
/// art. `WHITE` is the plain, fully opaque sprite.
pub fn draw_agents(sheet: Sheet<'_>, sim: &Sim, clock: f32, tint: Color32) {
    let sprites = sheet.sprites;

    for walker in sim.walkers() {
        let uv = if !walker.alive {
            sprites.walker_dead
        } else if walker_frame(clock) == 0 {
            sprites.walker_a
        } else {
            sprites.walker_b
        };
        sheet.image_tinted(sheet.world(walker.x, walker.y, walker.w, walker.h), uv, tint);
    }

    let p = sim.player();
    let uv = if !p.grounded {
        sprites.player_jump
    } else if p.vx.abs() > 0.01 {
        if player_frame(clock) == 0 {
            sprites.player_run_a
        } else {
            sprites.player_run_b
        }
    } else {
        sprites.player_stand
    };
    sheet.image_tinted(sheet.world(p.x, p.y, p.w, p.h), uv, tint);
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

/// Which of two walker frames is showing. Slow enough to read as walking, and a pure
/// function of the clock so a replay of the same episode draws the same frame.
fn walker_frame(clock: f32) -> i64 {
    ((clock * 3.0) as i64).rem_euclid(2)
}

/// Which of the two run frames is showing.
fn player_frame(clock: f32) -> i64 {
    ((clock * 8.0) as i64).rem_euclid(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};
    use render::trace::{ChampionTrace, TracePoint};

    fn assets_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    fn shipped(slug: &str) -> Level {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"));
        Level::from_json_file(&path).unwrap_or_else(|e| panic!("{slug}: {e}"))
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
                        draw_level(sheet, level, opts, None);
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
                        draw_level(sheet, level, RenderOpts::playing(), None);
                        if layers {
                            let sim = Sim::new(level, 1);
                            draw_agents(sheet, &sim, 0.0, Color32::from_white_alpha(120));
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

        // L1 has no walkers, so one lane is one agent: exactly four sprite vertices. A
        // lane that drew the whole population, or none of it, would show up here.
        assert_eq!(
            images,
            base_images + 4,
            "one agent sprite per lane: {base_images} -> {images}"
        );
        // The tag, the deaths and the route are drawn shapes rather than sprites, so they
        // are counted in the frame's total.
        assert!(
            total > base_total,
            "the lane tag, the death map and the route should reach the frame: \
             {base_total} -> {total}"
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
        let trail: Vec<(i32, i32)> = cells
            .iter()
            .filter(|c| !c.2)
            .map(|c| (c.0, c.1))
            .collect();
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
}
