//! The window: panels, the canvas, and the two things you do on it.
//!
//! Layout, top to bottom and left to right:
//!
//! ```text
//! +---------------------------------------------------------------+
//! | level name   New  Open  Save | Undo Redo | Edit/Play | Fit     |
//! +---------+-------------------------------------------+---------+
//! | palette |                                           | verdict |
//! | (drag   |              the level                    | physics |
//! |  these) |                                           | counts  |
//! | resize  |                                           |         |
//! +---------+-------------------------------------------+---------+
//! | status: message, tile under the cursor, zoom                   |
//! +---------------------------------------------------------------+
//! ```
//!
//! Drag and drop is implemented directly on top of egui's pointer rather than with
//! its payload API, for one reason: the drop has to be *placed*, and a brush that
//! lands on the wrong tile is worse than no brush at all. Holding the drag state
//! here means the canvas can draw the ghost sprite at the exact tile the release
//! will commit to, so what the user sees is what they get.

use std::path::PathBuf;

use egui::{Color32, Context, Pos2, Rect, Sense, Stroke, Vec2};
use sim::Tile;

use crate::atlas::Atlas;
use crate::editor::{Brush, Editor};
use crate::play;
use crate::view::{self, Camera, RenderOpts, Sheet, Sprites};

/// Which half of the tool the canvas is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Play,
}

/// Open the window and run until the user closes it.
pub fn run(level: PathBuf, assets: PathBuf, screenshot: Option<PathBuf>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 800.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title("evolution platformer"),
        ..Default::default()
    };
    let shot = screenshot.clone();
    eframe::run_native(
        "evolution platformer",
        options,
        Box::new(move |cc| {
            let app = EvoApp::new(cc, level, assets, screenshot)?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| format!("cannot open a window: {e}"))
    .map(|_| {
        let _ = shot;
    })
}

struct EvoApp {
    atlas: Atlas,
    sprites: Sprites,
    camera: Camera,
    editor: Editor,
    mode: Mode,
    session: Option<play::Session>,
    levels_dir: PathBuf,
    /// Pending grid size, applied by the Resize button.
    pending_size: (i32, i32),
    grid: bool,
    /// Brush being dragged out of the palette, if any.
    dragging: Option<Brush>,
    /// Pointer position from the previous frame, for panning.
    last_pointer: Option<Pos2>,
    /// A loaded atlas problem, shown once instead of panicking mid-frame.
    warning: Option<String>,
    /// Render one frame, save it, and leave. See `--screenshot`.
    screenshot: Option<PathBuf>,
    shot_requested: bool,
    frames: u32,
    fit_pending: bool,
}

impl EvoApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        level_path: PathBuf,
        assets: PathBuf,
        screenshot: Option<PathBuf>,
    ) -> Result<EvoApp, String> {
        let ctx = &cc.egui_ctx;
        ctx.set_visuals(egui::Visuals::dark());
        let atlas = Atlas::load(ctx, &assets)?;
        let sprites = Sprites::load(&atlas)?;

        let levels_dir = level_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("levels"));
        let editor = match Editor::open(&level_path) {
            Ok(editor) => editor,
            Err(err) => {
                // Opening a bad file must not lose the window: start from a fresh
                // level and say what happened.
                let mut editor = Editor::fresh("untitled", 60, 15);
                editor.message = format!("could not open {}: {err}", level_path.display());
                editor
            }
        };
        let pending_size = (editor.level.width, editor.level.height);

        Ok(EvoApp {
            atlas,
            sprites,
            camera: Camera::default(),
            editor,
            mode: Mode::Edit,
            session: None,
            levels_dir,
            pending_size,
            grid: true,
            dragging: None,
            last_pointer: None,
            warning: None,
            screenshot,
            shot_requested: false,
            frames: 0,
            fit_pending: true,
        })
    }

    fn opts(&self) -> RenderOpts {
        match self.mode {
            Mode::Edit => RenderOpts {
                grid: self.grid,
                ..RenderOpts::editing()
            },
            Mode::Play => RenderOpts::playing(),
        }
    }

    fn enter_play(&mut self) {
        self.session = Some(play::Session::new(&self.editor.level, 1));
        self.mode = Mode::Play;
    }

    fn leave_play(&mut self) {
        self.mode = Mode::Edit;
        self.session = None;
    }

    /// Act on the keys that are not part of playing: undo, save, mode switching.
    fn handle_shortcuts(&mut self, ctx: &Context) {
        let (undo, redo, grid, play, fit, save, restart) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Z) && i.modifiers.command,
                (i.key_pressed(egui::Key::Y) && i.modifiers.command)
                    || (i.key_pressed(egui::Key::Z) && i.modifiers.command && i.modifiers.shift),
                i.key_pressed(egui::Key::G),
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::S) && i.modifiers.command,
                i.key_pressed(egui::Key::R),
            )
        });
        if undo && self.editor.undo() {
            self.editor.message = "undo".into();
        }
        if redo && self.editor.redo() {
            self.editor.message = "redo".into();
        }
        if grid {
            self.grid = !self.grid;
        }
        if fit {
            self.fit_pending = true;
        }
        if save {
            self.save(None);
        }
        match self.mode {
            Mode::Edit => {
                if play {
                    self.enter_play();
                }
            }
            Mode::Play => {
                if play || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.leave_play();
                }
                if restart {
                    if let Some(session) = &mut self.session {
                        session.restart(&self.editor.level);
                    }
                }
            }
        }
    }

    fn save(&mut self, path: Option<PathBuf>) {
        // An untitled level saves by its own name, into the levels directory.
        let target = path.or_else(|| {
            self.editor.path.clone().or_else(|| {
                Some(self.levels_dir.join(format!("{}.json", slug(&self.editor.level.name))))
            })
        });
        if let Err(err) = self.editor.save(target) {
            self.editor.message = format!("not saved: {err}");
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(self.editor.title());
            ui.separator();
            if ui.button("New").clicked() {
                self.editor = Editor::fresh("untitled", 60, 15);
                self.pending_size = (60, 15);
                self.fit_pending = true;
            }
            let levels = Editor::levels_in(&self.levels_dir);
            ui.menu_button("Open", |ui| {
                if levels.is_empty() {
                    ui.label(format!("no levels in {}", self.levels_dir.display()));
                }
                for path in levels {
                    let label = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let current = self.editor.path.as_deref() == Some(path.as_path());
                    if ui.selectable_label(current, label).clicked() {
                        match Editor::open(&path) {
                            Ok(editor) => {
                                self.editor = editor;
                                self.pending_size = (self.editor.level.width, self.editor.level.height);
                                self.fit_pending = true;
                                self.session = None;
                            }
                            Err(err) => self.editor.message = err,
                        }
                        ui.close();
                    }
                }
            });
            if ui.button("Save").clicked() {
                self.save(None);
            }
            ui.separator();
            ui.add_enabled_ui(self.editor.can_undo(), |ui| {
                if ui.button("Undo").clicked() && self.editor.undo() {
                    self.editor.message = "undo".into();
                }
            });
            ui.add_enabled_ui(self.editor.can_redo(), |ui| {
                if ui.button("Redo").clicked() && self.editor.redo() {
                    self.editor.message = "redo".into();
                }
            });
            ui.separator();

            let mut mode = self.mode;
            ui.selectable_value(&mut mode, Mode::Edit, "Edit");
            ui.selectable_value(&mut mode, Mode::Play, "Play");
            if mode != self.mode {
                match mode {
                    Mode::Play => self.enter_play(),
                    Mode::Edit => self.leave_play(),
                }
            }
            if self.mode == Mode::Play && ui.button("Restart (R)").clicked() {
                if let Some(session) = &mut self.session {
                    session.restart(&self.editor.level);
                }
            }
            if ui.button("Fit (F)").clicked() {
                self.fit_pending = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.editor.dirty {
                    ui.colored_label(Color32::from_rgb(240, 190, 90), "unsaved");
                }
            });
        });
    }

    fn palette(&mut self, ui: &mut egui::Ui) {
        ui.label("Palette — drag onto the level, or click to select");
        ui.add_space(4.0);
        let cell = 40.0;
        for brush in Brush::ALL {
            let selected = self.editor.brush == Some(brush);
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(cell, cell), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            let frame = if selected {
                Stroke::new(2.0_f32, Color32::from_rgb(250, 214, 102))
            } else {
                Stroke::new(1.0_f32, Color32::from_gray(90))
            };
            painter.rect_filled(rect, egui::CornerRadius::same(4), Color32::from_gray(38));
            if let Ok(uv) = self.atlas.uv(brush.sprite()) {
                painter.image(
                    self.atlas.texture_id(),
                    rect.shrink(5.0),
                    uv,
                    Color32::WHITE,
                );
            }
            painter.rect_stroke(
                rect,
                egui::CornerRadius::same(4),
                frame,
                egui::StrokeKind::Inside,
            );
            let response = response.on_hover_text(format!(
                "{}: {}\nDrag onto the level to place it.",
                brush.label(),
                brush.about()
            ));
            if response.clicked() {
                self.editor.brush = Some(brush);
                self.editor.message = format!("{} brush", brush.label());
            }
            if response.drag_started() {
                self.dragging = Some(brush);
                self.editor.brush = Some(brush);
            }
            ui.label(brush.label());
            ui.add_space(2.0);
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.label("Validation");
        let verdict = self.editor.verdict();
        let colour = if self.editor.beatable() {
            Color32::from_rgb(120, 216, 120)
        } else {
            Color32::from_rgb(240, 120, 110)
        };
        ui.colored_label(colour, verdict);
        if let Some(culprit) = &self.editor.report.culprit {
            ui.small(format!(
                "stuck at platform {} take-off x {}{}",
                culprit.from,
                culprit.takeoff_x,
                culprit
                    .to
                    .map(|t| format!(" -> platform {t}"))
                    .unwrap_or_default()
            ));
        }
        if !self.editor.beatable() {
            ui.small("Reddish tint = the spawn cannot get there.");
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label("Physics (from sim::PHYSICS)");
        let physics = &sim::PHYSICS;
        ui.small(format!(
            "jump {:.2} tiles high, {:.2} tiles far",
            physics.max_jump_height(),
            physics.max_jump_distance()
        ));
        ui.small(format!(
            "run speed {:.2}/tick, gravity {:.4}",
            physics.run_speed, physics.gravity
        ));

        ui.add_space(8.0);
        ui.separator();
        ui.label("Level");
        let level = &self.editor.level;
        ui.small(format!("grid {}x{}", level.width, level.height));
        ui.small(format!("spawn {:?}", level.spawn_tile()));
        ui.small(format!(
            "{} goal(s), {} walker(s)",
            level.goals.len(),
            level.walkers.len()
        ));
        let (mut solid, mut spike) = (0, 0);
        for ty in 0..level.height {
            for tx in 0..level.width {
                match level.tile(tx, ty) {
                    Some(Tile::Solid) => solid += 1,
                    Some(Tile::Spike) => spike += 1,
                    _ => {}
                }
            }
        }
        ui.small(format!("{solid} solid tiles, {spike} spikes"));
        ui.small(level.content_hash_string());

        ui.add_space(8.0);
        ui.separator();
        ui.label("Resize the grid");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.pending_size.0).range(8..=400));
            ui.label("x");
            ui.add(egui::DragValue::new(&mut self.pending_size.1).range(5..=200));
        });
        if ui.button("Apply size").clicked() {
            let fill = Tile::Empty;
            self.editor
                .resize(self.pending_size.0, self.pending_size.1, fill);
            self.fit_pending = true;
        }
        if let Some(path) = &self.editor.path {
            ui.add_space(8.0);
            ui.small(path.display().to_string());
        }
    }

    /// The canvas: level, ghosts, and every pointer gesture.
    fn canvas(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        if self.fit_pending {
            self.camera = Camera::fit(&self.editor.level, rect, 24.0);
            self.fit_pending = false;
        }

        // --- camera gestures -------------------------------------------------
        let hover = response.hover_pos();
        if let Some(hover) = hover {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                self.camera.zoom_at((scroll * 0.002).exp(), hover, rect);
            }
        }
        // Middle-drag, or alt-drag, moves the level under the cursor. Alt is there
        // because a two-button mouse on a trackpad rarely has a middle button.
        let panning = ui.ctx().input(|i| {
            i.pointer.middle_down() || (i.pointer.primary_down() && i.modifiers.alt)
        });
        if panning {
            if let (Some(now), Some(last)) = (hover, self.last_pointer) {
                let delta = now - last;
                self.camera.centre -= delta / self.camera.zoom;
            }
        }
        self.last_pointer = hover;

        // --- draw ------------------------------------------------------------
        view::draw_background(&painter, rect, &self.camera, &self.editor.level);
        let opts = self.opts();
        let sheet = Sheet::new(&painter, &self.atlas, &self.sprites, &self.camera, rect);
        match self.mode {
            Mode::Edit => view::draw_level(
                sheet,
                &self.editor.level,
                opts,
                Some(&self.editor.report),
            ),
            Mode::Play => {
                if let Some(session) = &self.session {
                    view::draw_sim(sheet, session.sim(), opts, session.clock);
                }
            }
        }

        if self.mode == Mode::Edit {
            self.canvas_edit_gestures(ui, &response, rect, &painter);
        }
    }

    /// Every canvas gesture the editor understands, in the order they take precedence.
    ///
    /// A drag out of the palette wins over everything: while `dragging` is set, the
    /// pointer is showing where a *drop* would land, and painting on the way there
    /// would be a surprise. The drop commits to the tile under the ghost, which is
    /// the tile the user watched the ghost snap to.
    fn canvas_edit_gestures(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        rect: Rect,
        painter: &egui::Painter,
    ) {
        // `interact_pos` rather than `hover_pos`: during a drag the pointer is
        // captured and no longer "hovering", which would freeze the ghost.
        let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let tile = self.camera.to_tile(pos, rect);
        let (tx, ty) = (tile.x.floor() as i32, tile.y.floor() as i32);
        let inside = self.editor.level.tile(tx, ty).is_some();

        if let Some(brush) = self.dragging {
            if inside {
                let target = self.camera.tile_rect(tx, ty, rect);
                if let Ok(uv) = self.atlas.uv(brush.sprite()) {
                    painter.image(
                        self.atlas.texture_id(),
                        target,
                        uv,
                        Color32::from_white_alpha(170),
                    );
                }
                painter.rect_stroke(
                    target,
                    egui::CornerRadius::ZERO,
                    Stroke::new(1.5_f32, Color32::from_rgb(250, 214, 102)),
                    egui::StrokeKind::Inside,
                );
            }
            let released = ui.ctx().input(|i| i.pointer.any_released());
            if released {
                if inside {
                    self.editor.begin_stroke();
                    self.editor.paint(tx, ty);
                    self.editor.end_stroke();
                }
                self.dragging = None;
            }
            return;
        }

        let (primary, secondary, released) = ui.ctx().input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
                i.pointer.any_released(),
            )
        });
        if released {
            self.editor.end_stroke();
        }
        if !inside || !(response.hovered() || response.dragged()) {
            return;
        }
        // Right-click erases whatever is under the cursor, regardless of the brush.
        if secondary {
            self.editor.erase_at(tx, ty);
        } else if primary {
            self.editor.paint(tx, ty);
        }
    }

    /// Move the camera to keep the player in view while playing.
    fn follow_player(&mut self, rect: Rect) {
        let Some(session) = &self.session else {
            return;
        };
        let p = session.sim().player();
        self.camera.centre = Vec2::new(p.x + p.w * 0.5, p.y + p.h * 0.5);
        self.camera.clamp_to(&self.editor.level, 6.0);
        self.camera.zoom = self.camera.zoom.clamp(18.0, 48.0);
        let _ = rect;
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(session) = &self.session {
                ui.label(session.summary());
            } else {
                ui.label(if self.editor.message.is_empty() {
                    "drag from the palette, or pick a brush and click".to_string()
                } else {
                    self.editor.message.clone()
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("zoom {:.0} px/tile", self.camera.zoom));
                if let Some(hover) = self.last_pointer {
                    ui.label(format!(
                        "tile ({:.0}, {:.0})",
                        (hover.x / self.camera.zoom).floor(),
                        (hover.y / self.camera.zoom).floor()
                    ));
                }
                if let Some(warning) = &self.warning {
                    ui.colored_label(Color32::from_rgb(240, 150, 90), warning);
                }
            });
        });
    }

    /// `--screenshot`: render one frame, save it, leave.
    fn handle_screenshot(&mut self, ctx: &Context) -> Result<(), String> {
        let Some(path) = self.screenshot.clone() else {
            return Ok(());
        };
        self.frames += 1;
        if !self.shot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.shot_requested = true;
        }
        if let Some(image) = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        }) {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            let buffer = image::RgbaImage::from_raw(
                image.width() as u32,
                image.height() as u32,
                bytes,
            )
            .ok_or("the screenshot reported an impossible size")?;
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            buffer
                .save(&path)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            println!(
                "wrote {} ({}x{})",
                path.display(),
                image.width(),
                image.height()
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return Ok(());
        }
        if self.frames > 240 {
            // Never hang an automated run: a screenshot that has not arrived after
            // four seconds is a broken window, not a slow one.
            return Err("no screenshot arrived within 240 frames".into());
        }
        ctx.request_repaint();
        Ok(())
    }
}

impl eframe::App for EvoApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.editor.refresh_report();
        self.handle_shortcuts(ctx);

        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if self.mode == Mode::Play {
            let keys = play::Keys::read(ctx);
            if let Some(session) = &mut self.session {
                session.advance(dt, keys);
            }
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("top")
            .show(ctx, |ui| self.top_bar(ui));
        egui::SidePanel::left("palette")
            .default_width(190.0)
            .show(ctx, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.palette(ui)));
        egui::SidePanel::right("inspector")
            .default_width(250.0)
            .show(ctx, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui)));
        egui::TopBottomPanel::bottom("status")
            .show(ctx, |ui| self.status_bar(ui));

        let mut canvas_rect = Rect::NOTHING;
        egui::CentralPanel::default()
            .frame(egui::Frame::default())
            .show(ctx, |ui| {
                canvas_rect = ui.available_rect_before_wrap();
                self.canvas(ui);
            });

        if self.mode == Mode::Play {
            self.follow_player(canvas_rect);
        }

        if let Err(err) = self.handle_screenshot(ctx) {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

/// A file-name-safe version of a level name.
fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '_' | '-') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}
