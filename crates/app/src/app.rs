//! The window: panels, the canvas, and the three things you do on it.
//!
//! Layout, top to bottom and left to right:
//!
//! ```text
//! +---------------------------------------------------------------+
//! | level name   New  Open  Save | Undo Redo | Edit/Play/Evolve   |
//! +---------+-------------------------------------------+---------+
//! | palette |                                           | verdict |
//! | (drag   |         the level, or the run             | physics |
//! |  these) |                                           | counts  |
//! | resize  |                                           | lanes   |
//! +---------+-------------------------------------------+---------+
//! | fitness chart (evolution only)                                 |
//! +---------------------------------------------------------------+
//! | status: message, tile under the cursor, zoom                   |
//! +---------------------------------------------------------------+
//! ```
//!
//! Drag and drop is implemented directly on top of egui's pointer rather than with
//! its payload API, for one reason: the drop has to be *placed*, and a brush that
//! lands on the wrong tile is worse than no brush at all. Holding the drag state
//! here means the canvas can draw the ghost sprite at the exact tile the release
//! will commit to, so what the user sees is what they get.
//!
//! The evolution view is the terminal's watch mode with the frame drawing replaced by
//! this window's: `crate::lab` owns the trainer on its own thread and hands over one
//! finished generation at a time, `crate::panel` draws what the run is doing, and this
//! module is where the two meet the keys. The one rule it inherits intact is that a run
//! advances **when the viewer has finished watching, not when a timer fires**.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Color32, Context, Pos2, Rect, Sense, Stroke, Vec2};
use evo::GaConfig;
use sim::{Level, Tile};

use crate::atlas::Atlas;
use crate::editor::{Brush, Editor};
use crate::lab::{self, Lab, Lanes, Packet};
use crate::panel::{self, BestLine, Command, RunSummary, ViewState};
use crate::play;
use crate::view::{self, Camera, RenderOpts, Sheet, Sprites};
use render::chart::HistoryPoint;

/// Height of the fitness chart's strip at the bottom of the window.
const CHART_PANEL_HEIGHT: f32 = 150.0;

/// Points per tile the follow camera zooms to.
///
/// Following means looking at one agent, and the whole-level fit would make that
/// pointless: at a fit zoom a 60-tile level draws a six-pixel agent. The zoom is only
/// raised to this, never lowered, so a viewer who has zoomed in further keeps their zoom.
const FOLLOW_ZOOM: f32 = 34.0;

/// Which of the three things the window is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Play,
    Evolve,
}

/// How a run is set up, from the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabOptions {
    /// Open in the evolution view rather than the editor.
    pub evolve: bool,
    pub pop_size: usize,
    pub seed: u64,
    /// Stop after this many generations. `None` runs until the window is closed.
    pub generations: Option<u32>,
    /// Where `Save champion` writes, and where a curriculum step's checkpoint goes.
    pub champions: PathBuf,
    /// Start the run from a population checkpoint instead of a fresh population, as
    /// `train-cli watch --resume` does.
    pub resume: Option<PathBuf>,
    /// Easier level to fall back to when the run stalls (§12 M5), loaded when the run
    /// starts — a mistyped path is then an error at startup rather than a surprise forty
    /// generations into a stalled run, which is the terminal's rule too.
    pub curriculum: Option<PathBuf>,
    /// Generations a curriculum step spends on that easier level.
    pub curriculum_generations: u32,
}

impl Default for LabOptions {
    fn default() -> Self {
        LabOptions {
            evolve: false,
            pop_size: GaConfig::default().pop_size,
            seed: GaConfig::default().master_seed,
            generations: None,
            champions: PathBuf::from("champions"),
            resume: None,
            curriculum: None,
            curriculum_generations: lab::DEFAULT_CURRICULUM_GENERATIONS,
        }
    }
}

/// Open the window and run until the user closes it.
pub fn run(
    level: PathBuf,
    assets: PathBuf,
    screenshot: Option<PathBuf>,
    lab: LabOptions,
) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 800.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title("evolution platformer"),
        ..Default::default()
    };
    eframe::run_native(
        "evolution platformer",
        options,
        Box::new(move |cc| {
            let app = EvoApp::new(cc, level, assets, screenshot, lab)?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| format!("cannot open a window: {e}"))
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

    // --- the evolution view ------------------------------------------------
    options: LabOptions,
    /// The seed the *current* run is using. Restart bumps it, so pressing Restart
    /// twice gives two populations rather than the same one twice — while the number
    /// stays on screen, so a run worth keeping can be reproduced from it.
    seed: u64,
    lab: Option<Lab>,
    /// The generation being animated, when one is on screen.
    lanes: Option<Lanes>,
    /// The newest finished generation. Held rather than reached for through `lanes`,
    /// because the panel, the death map and `Save champion` all need it after the
    /// animation of it has ended.
    packet: Option<Packet>,
    summary: Option<RunSummary>,
    view: ViewState,
    /// `n`: run exactly one generation, then pause.
    single_step: bool,
    /// Seconds left on the genius note (§10 Tier 1.3). The panel's own genius flag lasts
    /// as long as its generation does; a splash has to outlive the packet to be seen.
    genius_left: f32,
    /// A transient status note and its remaining seconds.
    note: Option<(String, f32)>,
    /// The mutation rate the run has actually been told about, so the worker is not
    /// woken every frame to be told the same number again.
    applied_rate: Option<f32>,
    /// The screenshot path has primed the run with a real generation.
    shot_primed: bool,
    /// A run that stopped at its cap has been saved once. The terminal writes the best
    /// genome when a watched run ends, and this is the same promise kept per run rather
    /// than per keypress.
    saved_end: bool,
}

impl EvoApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        level_path: PathBuf,
        assets: PathBuf,
        screenshot: Option<PathBuf>,
        options: LabOptions,
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

        let mut app = EvoApp {
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
            seed: options.seed,
            options,
            lab: None,
            lanes: None,
            packet: None,
            summary: None,
            view: ViewState::default(),
            single_step: false,
            saved_end: false,
            genius_left: 0.0,
            note: None,
            applied_rate: None,
            shot_primed: false,
        };
        if app.options.evolve {
            app.enter_evolve();
        }
        Ok(app)
    }

    fn opts(&self) -> RenderOpts {
        match self.mode {
            Mode::Edit => RenderOpts {
                grid: self.grid,
                ..RenderOpts::editing()
            },
            Mode::Play | Mode::Evolve => RenderOpts::playing(),
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

    // -----------------------------------------------------------------------
    // the evolution view
    // -----------------------------------------------------------------------

    /// Show the evolution view, starting a run the first time and after any edit that
    /// moved the level out from under the run that is already going.
    ///
    /// Leaving and coming back keeps a live run: the population, its chart and its
    /// record are worth preserving across a look at the editor, and this is the only way
    /// to look at the run and the level it is training on side by side.
    fn enter_evolve(&mut self) {
        if self.lab.is_none() {
            let cfg = GaConfig {
                pop_size: self.options.pop_size,
                master_seed: self.seed,
                ..GaConfig::default()
            };
            let plan = match self.run_plan() {
                Ok(plan) => plan,
                Err(err) => {
                    self.editor.message = err;
                    return;
                }
            };
            match Lab::start(
                &self.editor.level,
                cfg,
                lab::DEFAULT_BEST_LANES,
                lab::DEFAULT_RANDOM_LANES,
                plan,
                self.options.champions.clone(),
            ) {
                Ok(lab) => {
                    // A run handed a population from somewhere else says so once: the
                    // numbers on screen are a claim about a level, and a checkpoint from
                    // a different one is exactly the thing a viewer must not have to
                    // guess.
                    if let Some(origin) = lab.origin() {
                        self.note = Some((origin.to_string(), 12.0));
                    }
                    self.lab = Some(lab);
                    self.saved_end = false;
                }
                Err(err) => {
                    // A worker that cannot start is a status message, not a lost window.
                    self.editor.message = err;
                    return;
                }
            }
        } else if !self
            .lab
            .as_ref()
            .is_some_and(|lab| lab.trained_on(&self.editor.level))
        {
            // The numbers on screen are a claim about a level that no longer exists.
            self.restart_run("the level changed, so this is a fresh run on the edit");
        }
        self.mode = Mode::Evolve;
        self.fit_pending = true;
    }

    /// The run the command line asked for: how long, from where, and what to do when it
    /// stalls.
    ///
    /// The easier level is loaded here rather than by the worker, so a level that cannot
    /// be read is reported where the viewer is looking instead of inside a thread that is
    /// already training. Both paths are the terminal's: `--curriculum` is a startup error
    /// there too, and both front-ends must accept and refuse the same command lines.
    fn run_plan(&self) -> Result<lab::RunPlan, String> {
        let curriculum = match &self.options.curriculum {
            Some(path) => Some(lab::Curriculum {
                level: Level::from_json_file(path)
                    .map_err(|e| format!("--curriculum {}: {e}", path.display()))?,
                generations: self.options.curriculum_generations.max(1),
            }),
            None => None,
        };
        Ok(lab::RunPlan {
            cap: self.options.generations,
            resume: self.options.resume.clone(),
            curriculum,
        })
    }

    /// Start a fresh population on the level as it stands, on the next seed up.
    ///
    /// The seed is bumped rather than reused: a Restart that reproduced the identical
    /// run would be a button with no effect. Bumping it keeps every run reproducible from
    /// the number the status bar shows.
    fn restart_run(&mut self, why: &str) {
        self.seed = self.seed.wrapping_add(1);
        let Some(lab) = &mut self.lab else {
            return;
        };
        lab.restart(&self.editor.level, self.seed);
        lab.set_mutation_rate(self.view.mutation_rate);
        self.lanes = None;
        self.packet = None;
        self.summary = None;
        self.single_step = false;
        self.view.scrub = None;
        self.saved_end = false;
        self.applied_rate = Some(self.view.mutation_rate);
        self.note = Some((format!("{why} (seed {})", self.seed), 6.0));
    }

    /// One frame of the evolution view: ask for the next generation when the viewer has
    /// finished watching this one, take it when the worker has produced it, and animate
    /// whatever is on screen.
    fn evolve_step(&mut self, dt: f32) {
        if let Some((_, left)) = &mut self.note {
            *left -= dt;
        }
        if self.note.as_ref().is_some_and(|(_, left)| *left <= 0.0) {
            self.note = None;
        }
        if self.genius_left > 0.0 {
            self.genius_left -= dt;
        }

        // The run's rate limit, and the reason a paused window costs nothing: while a
        // generation is on screen — or while the viewer is paused or scrubbing — nothing
        // asks the worker for another one, so it stays parked instead of training ahead.
        if self.lanes.is_none() && !self.view.paused && self.view.scrub.is_none() {
            if let Some(lab) = &mut self.lab {
                if !lab.asked() {
                    lab.request_next();
                }
            }
        }

        let mut ended = false;
        if let Some(lanes) = self.lab.as_mut().and_then(|lab| lab.take()) {
            let packet = lanes.packet().clone();
            if packet.genius {
                self.genius_left = 6.0;
            }
            // A scrubbed route the record has since dropped is no longer somewhere the
            // viewer can be: the log keeps the most recent improvements and forgets the
            // oldest, so a cursor left on one of those would show a route that is gone.
            if let Some(generation) = self.view.scrub {
                if !packet.traces.iter().any(|t| t.generation == generation) {
                    self.view.scrub = None;
                }
            }
            self.summary = Some(self.summarise(&packet));
            self.packet = Some(packet);
            self.lanes = Some(lanes);
            ended = self.packet.as_ref().is_some_and(|packet| packet.finished);
        }

        if ended {
            self.save_if_run_ended();
        }

        let Some(lanes) = &mut self.lanes else {
            return;
        };
        if !self.view.paused {
            lanes.advance(dt, self.view.speed);
        }
        if lanes.is_finished() {
            self.lanes = None;
            if self.single_step {
                self.single_step = false;
                self.view.paused = true;
            }
        }
    }

    /// What the panel says about a finished generation.
    ///
    /// The two facts a packet cannot carry are added here, each from whoever has them:
    /// the level's width (the death-cluster rule needs it to know which marks are even
    /// inside the level) and the champion's episode, which lives in `evo`'s best-ever
    /// record rather than in the report.
    fn summarise(&self, packet: &Packet) -> RunSummary {
        let mut summary = RunSummary::new(packet, &packet.history);
        let Some(lab) = &self.lab else {
            return summary;
        };
        summary = summary.with_level(lab.level().width);
        if let Some(best) = &packet.best {
            // The flag's own column, for the "tiles to go" number: fitness is a weighted
            // progress score and cannot be read as distance on a level nobody has seen.
            let goal_x = lab
                .level()
                .goals
                .first()
                .map(|goal| goal.x)
                .unwrap_or(0.0);
            summary = summary.with_champion(BestLine::from_episode(
                best.generation,
                best.genome.id,
                best.result.fitness,
                best.result.outcome.as_str(),
                best.result.won(),
                best.result.frames,
                best.result.best_x,
                goal_x,
            ));
        }
        summary
    }

    /// Write the run's champion out if the run has reached its cap.
    ///
    /// A watched run is a real run — the terminal's rule — so the best genome it found is
    /// saved when it stops, and watching instead of training wastes nothing. Only a
    /// *capped* run ends by itself, and a cap is something the viewer asked for, so this
    /// cannot write a file nobody wanted. Once per run: a run that is over stays over
    /// until it is restarted, and a file rewritten every frame would be a disk spinner.
    fn save_if_run_ended(&mut self) {
        if self.saved_end || !self.packet.as_ref().is_some_and(|packet| packet.finished) {
            return;
        }
        self.saved_end = true;
        self.save_champion();
    }

    /// Write the best-ever genome the same way `train-cli train` writes one, so watching
    /// instead of training wastes nothing: the file is the same format, with the same
    /// provenance, and `train-cli replay` can re-verify it.
    fn save_champion(&mut self) {
        let best = self.packet.as_ref().and_then(|packet| packet.best.clone());
        let (Some(best), Some(lab)) = (best, self.lab.as_ref()) else {
            self.note = Some(("nothing to save: no generation has been scored".into(), 6.0));
            return;
        };
        // The run names its own files (`lab::stem`), so the champion a viewer saves
        // while watching is the same file, in the same place, as the one the terminal
        // writes for the same level.
        let dir = lab.out_dir().to_path_buf();
        let stem = lab.stem();
        let note = match lab::save_champion(&best, lab.level(), lab.config(), &dir, &stem) {
            Ok(path) => format!(
                "saved champion gen {} fitness {:.2} -> {}",
                best.generation,
                best.result.fitness,
                path.display()
            ),
            Err(err) => format!("not saved: {err}"),
        };
        self.note = Some((note, 8.0));
    }

    /// Walk the recorded improvements, one route at a time.
    fn scrub_by(&mut self, delta: i32) {
        let Some(summary) = &self.summary else {
            return;
        };
        let generations: Vec<u32> = summary.recorded.iter().map(|r| r.generation).collect();
        if generations.is_empty() {
            return;
        }
        let steps = delta.unsigned_abs();
        for _ in 0..steps {
            self.view.scrub = if delta < 0 {
                panel::scrub_back(&generations, self.view.scrub)
            } else {
                panel::scrub_forward(&generations, self.view.scrub)
            };
        }
    }

    /// Act on the keys that are not part of playing: undo, save, mode switching, and the
    /// evolution view's own controls.
    fn handle_shortcuts(&mut self, ctx: &Context) {
        let (undo, redo, grid, play, evolve, fit, save, restart) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Z) && i.modifiers.command,
                (i.key_pressed(egui::Key::Y) && i.modifiers.command)
                    || (i.key_pressed(egui::Key::Z) && i.modifiers.command && i.modifiers.shift),
                i.key_pressed(egui::Key::G),
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::E),
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
        // `f` is one key with two meanings, split by mode rather than by modifier: in the
        // editor it fits the level in the window, and in the evolution view it is the
        // terminal's follow/whole-level toggle. Both are the same idea — "show me all of
        // it" against "put me on the thing that matters" — and following re-fits when it
        // is switched off, which is what makes the two modes agree about the key.
        if fit && self.mode != Mode::Evolve {
            self.fit_pending = true;
        }
        if save {
            self.save(None);
        }
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        match self.mode {
            Mode::Edit => {
                if play {
                    self.enter_play();
                }
                if evolve {
                    self.enter_evolve();
                }
            }
            Mode::Play => {
                if play || escape {
                    self.leave_play();
                }
                if restart {
                    if let Some(session) = &mut self.session {
                        session.restart(&self.editor.level);
                    }
                }
            }
            Mode::Evolve => {
                if play || evolve || escape {
                    self.mode = Mode::Edit;
                }
                if fit {
                    self.apply(Command::ToggleFollow);
                }
                self.evolve_keys(ctx);
            }
        }
    }

    /// The evolution view's keys.
    ///
    /// They are the terminal watch mode's own bindings (`space` pauses, `-`/`+` change
    /// speed, `m`/`M` change the mutation rate, `n` steps one generation, `left`/`right`
    /// walk the record, `h` returns to live, `s` saves the champion, `f` follows the
    /// leader) — two front-ends for one game teaching two different keyboards would be a
    /// bug, not a feature.
    fn evolve_keys(&mut self, ctx: &Context) {
        let keys = ctx.input(|i| panel::Keys {
            pause: i.key_pressed(egui::Key::Space),
            step: i.key_pressed(egui::Key::N),
            faster: i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
            slower: i.key_pressed(egui::Key::Minus),
            mutate_up: i.key_pressed(egui::Key::M) && i.modifiers.shift,
            mutate_down: i.key_pressed(egui::Key::M) && !i.modifiers.shift,
            live: i.key_pressed(egui::Key::H),
            follow: i.key_pressed(egui::Key::F),
            save: i.key_pressed(egui::Key::S) && !i.modifiers.command,
            older: i.key_pressed(egui::Key::ArrowLeft),
            newer: i.key_pressed(egui::Key::ArrowRight),
        });
        // What a key means is `panel::commands_for`'s business; this is only the reading.
        let commands = panel::commands_for(keys, &self.view);
        for command in commands {
            self.apply(command);
        }
    }

    /// Prime a scripted frame with a real run.
    ///
    /// Two things have to be true of a `--screenshot` of this view or it verifies nothing:
    /// there has to be a *generation* on screen (eight agents, a chart with a point on it),
    /// and with `--generations` there has to be a **history**, because a chart of one
    /// generation is a single dot. So the cap is run out first, taking each generation
    /// without animating it — the population advances exactly as it would have, and nobody
    /// watches the frames in between, which is the terminal mode's headless behaviour — and
    /// then the newest generation is left running for a moment so its agents are off the
    /// spawn.
    fn prime_scripted_run(&mut self) {
        let target = self.options.generations;
        let mut rounds = 0u32;
        loop {
            let Some(lanes) = self
                .lab
                .as_mut()
                .and_then(|lab| lab.wait_for_first(Duration::from_secs(120)))
            else {
                return;
            };
            let packet = lanes.packet().clone();
            if packet.genius {
                self.genius_left = 6.0;
            }
            self.summary = Some(self.summarise(&packet));
            let last = packet.finished || target.is_none();
            self.packet = Some(packet);
            self.lanes = Some(lanes);
            if last {
                break;
            }
            self.lanes = None;
            if let Some(lab) = &mut self.lab {
                lab.request_next();
            }
            rounds += 1;
            if rounds > 10_000 {
                // A cap that never reports itself finished must not hang a scripted run.
                return;
            }
        }
        for _ in 0..40 {
            self.evolve_step(1.0 / 60.0);
        }
        // The scripted path took its generations without passing through the take that
        // normally notices a capped run ending, so the same end-of-run save is offered
        // here: a `--screenshot --generations N` run leaves the same artifacts a session
        // at the keyboard would.
        self.save_if_run_ended();
    }

    /// The right-hand panel in the evolution view.
    fn evolve_panel(&mut self, ui: &mut egui::Ui) {
        let Some(summary) = self.summary.take() else {
            // Before the first generation lands there is nothing to summarise, and a
            // panel of zeros would read as a run that is going nowhere rather than one
            // that has not started.
            ui.heading("Evolution");
            match &self.lab {
                Some(lab) => {
                    ui.label(format!("{}: evaluating the first generation…", lab.level_name()));
                    ui.small(format!("pop {}   seed {}", lab.pop_size(), self.seed));
                }
                None => {
                    ui.label("no run on this level yet");
                    if ui.button("Start a run").clicked() {
                        self.enter_evolve();
                    }
                }
            }
            return;
        };

        let changed = self
            .lab
            .as_ref()
            .is_some_and(|lab| !lab.trained_on(&self.editor.level));
        let actions = panel::lab_panel(ui, &summary, &mut self.view, changed);
        self.summary = Some(summary);

        if actions.pause {
            self.apply(Command::TogglePause);
        }
        if actions.step {
            self.apply(Command::StepOne);
        }
        if actions.restart {
            self.apply(Command::Restart);
        }
        if actions.save {
            self.apply(Command::Save);
        }
        if actions.resume {
            self.apply(Command::Live);
        }
        if actions.follow {
            self.apply(Command::ToggleFollow);
        }
        if actions.scrub_delta != 0 {
            self.apply(Command::Scrub(actions.scrub_delta));
        }
        if self.applied_rate != Some(self.view.mutation_rate) {
            self.apply(Command::MutationRate(self.view.mutation_rate));
        }
    }

    /// Do one thing the viewer asked for, from a button, a key or the sliders.
    ///
    /// The two front-ends differ only in how they read the request; what a request *means*
    /// lives here, once.
    fn apply(&mut self, command: Command) {
        match command {
            Command::TogglePause => {
                self.view.paused = !self.view.paused;
                self.single_step = false;
            }
            Command::StepOne => {
                // Inspecting, not watching: run a generation and then hold still.
                self.view.paused = false;
                self.single_step = true;
            }
            Command::Restart => self.restart_run("restarted"),
            Command::Save => self.save_champion(),
            Command::Live => self.view.scrub = None,
            Command::ToggleFollow => {
                self.view.follow = !self.view.follow;
                // Leaving follow gives the whole level back rather than leaving the camera
                // wherever the last agent stood: the two states of one key should be the
                // two views, not a zoom level the viewer has to fix by hand.
                if !self.view.follow {
                    self.fit_pending = true;
                }
            }
            Command::Scrub(delta) => self.scrub_by(delta),
            Command::Speed(step) => {
                self.view.speed = lab::step_speed(self.view.speed, step > 0);
            }
            Command::MutationRate(rate) => {
                self.view.mutation_rate = rate;
                // The run's rate, not the view's: the worker applies it from the next
                // generation on, so a change shows up in the very next offspring.
                if self.applied_rate != Some(rate) {
                    if let Some(lab) = &mut self.lab {
                        lab.set_mutation_rate(rate);
                    }
                    self.applied_rate = Some(rate);
                }
            }
        }
    }

    fn save(&mut self, path: Option<PathBuf>) {
        // An untitled level saves by its own name, into the levels directory.
        let target = path.or_else(|| {
            self.editor.path.clone().or_else(|| {
                Some(self.levels_dir.join(format!("{}.json", lab::slug(&self.editor.level.name))))
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
            ui.selectable_value(&mut mode, Mode::Evolve, "Evolve");
            if mode != self.mode {
                match mode {
                    Mode::Play => self.enter_play(),
                    Mode::Evolve => self.enter_evolve(),
                    Mode::Edit => {
                        if self.mode == Mode::Play {
                            self.leave_play();
                        } else {
                            // The run keeps going — paused and parked — so coming back to
                            // it does not mean starting from generation zero again.
                            self.mode = Mode::Edit;
                        }
                    }
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

        // Following is a camera rule, applied after the viewer's own gestures so the frame
        // being drawn is the frame the camera is on rather than the one before it. It is
        // the terminal's `f`: earlier only because an eighty-column frame cannot hold a
        // whole level, and here it is the viewer choosing to look at one agent.
        if self.mode == Mode::Evolve && self.view.follow && !self.fit_pending {
            self.follow_leader();
        }

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
            Mode::Evolve => self.canvas_evolve(sheet),
        }

        if self.mode == Mode::Edit {
            self.canvas_edit_gestures(ui, &response, rect, &painter);
        }
    }

    /// Put the camera on the leader: the agent the terminal mode's `f` follows, or the
    /// end of the route being scrubbed.
    ///
    /// A scrubbed route is centred on where it stopped, because that ending is the
    /// obstacle the moment being shown is about — the same rule the terminal view uses,
    /// so the two views point at the same place for the same recorded generation.
    fn follow_leader(&mut self) {
        let Some(lab) = &self.lab else {
            return;
        };
        let level = lab.level().clone();
        let target = match self.view.scrub {
            Some(generation) => self
                .packet
                .as_ref()
                .and_then(|packet| packet.traces.iter().find(|t| t.generation == generation))
                .and_then(|trace| trace.end())
                .map(|p| Vec2::new(p.x, p.y)),
            // The rule itself is `lab`'s, so the window and the terminal cannot pick
            // different agents to follow and it is covered by a test that needs no window.
            None => self
                .lanes
                .as_ref()
                .and_then(lab::leader_centre)
                .map(|(x, y)| Vec2::new(x, y)),
        };
        let Some(target) = target else {
            return;
        };
        self.camera.centre = target;
        self.camera.zoom = self.camera.zoom.max(FOLLOW_ZOOM);
        self.camera.clamp_to(&level, 2.0);
    }

    /// The evolution canvas: the level the run is training on, then the generation's
    /// agents over it, then whatever else the view is showing.
    ///
    /// The level comes from the **run**, not from the editor: a lane has to be drawn on
    /// the terrain its episode was evaluated on. When the two have diverged, the panel
    /// says so and offers a restart — drawing the edit under an old run's agents would be
    /// the one thing worse than saying nothing.
    fn canvas_evolve(&self, sheet: Sheet<'_>) {
        let Some(lab) = &self.lab else {
            return;
        };
        view::draw_level(sheet, lab.level(), self.opts(), None);

        if let Some(generation) = self.view.scrub {
            // A recorded route replaces the live lanes rather than joining them: the two
            // are answers to different questions (where a past champion got to, and where
            // this generation is now), and overlaid they answer neither.
            if let Some(trace) = self
                .packet
                .as_ref()
                .and_then(|packet| packet.traces.iter().find(|t| t.generation == generation))
            {
                view::draw_route(sheet, trace);
            }
            return;
        }

        if let Some(lanes) = &self.lanes {
            // The champion solid, the rest ghosted: eight agents in one place is a smear
            // otherwise, and which one is which is what the panel's lane list is for.
            for i in 0..lanes.len() {
                let lane = &lanes.lanes()[i];
                let alpha = if lane.rank == 0 {
                    255
                } else if lane.best {
                    190
                } else {
                    110
                };
                view::draw_agents(
                    sheet,
                    lanes.sim(i),
                    lanes.clock(),
                    Color32::from_white_alpha(alpha),
                );
                view::draw_lane_tag(
                    sheet,
                    lanes.sim(i),
                    Color32::from_rgb(lane.colour[0], lane.colour[1], lane.colour[2]),
                );
            }
        }

        if self.view.show_deaths {
            if let Some(packet) = &self.packet {
                view::draw_deaths(sheet, &packet.deaths, packet.generation, lab::DEATH_WINDOW);
            }
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

    /// The status line in the evolution view: where the run is, and what the viewer just
    /// did to it.
    fn evolve_status(&self) -> String {
        let speed = format!("{}x", self.view.speed);
        let paused = if self.view.paused { "  paused" } else { "" };
        match (&self.lanes, &self.packet) {
            (Some(lanes), _) => format!(
                "generation {} — {} of {} lanes still running — {:.1}s animated — {} — seed {}{}",
                lanes.packet().generation + 1,
                lanes.running(),
                lanes.len(),
                lanes.clock(),
                speed,
                self.seed,
                paused
            ),
            (None, Some(packet)) => format!(
                "generation {} scored — watching it finished — {} — seed {}{}",
                packet.generation + 1,
                speed,
                self.seed,
                paused
            ),
            (None, None) => format!("evaluating the first generation… — {} — seed {}", speed, self.seed),
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.mode == Mode::Evolve {
                ui.label(self.evolve_status());
                if let Some((note, _)) = &self.note {
                    ui.colored_label(Color32::from_rgb(250, 214, 102), note.clone());
                }
            } else if let Some(session) = &self.session {
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
        if self.mode == Mode::Evolve {
            self.evolve_step(dt);
            // A run is always moving, so the window redraws whether or not egui thinks
            // anything changed.
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("top")
            .show(ctx, |ui| self.top_bar(ui));
        // The palette is editing furniture; the evolution view would rather have the
        // width for the level.
        if self.mode == Mode::Edit {
            egui::SidePanel::left("palette")
                .default_width(190.0)
                .show(ctx, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.palette(ui)));
        }
        egui::SidePanel::right("inspector")
            .default_width(if self.mode == Mode::Evolve { 310.0 } else { 250.0 })
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match self.mode {
                    Mode::Evolve => self.evolve_panel(ui),
                    Mode::Edit | Mode::Play => self.inspector(ui),
                })
            });
        if self.mode == Mode::Evolve {
            egui::TopBottomPanel::bottom("chart")
                .exact_height(CHART_PANEL_HEIGHT)
                .show(ctx, |ui| {
                    // The run's *own* history, from the newest packet: re-deriving it from
                    // the trainer would be a second answer to the same question.
                    let history: &[HistoryPoint] = match &self.packet {
                        Some(packet) => &packet.history,
                        None => &[],
                    };
                    panel::chart_panel(ui, history, CHART_PANEL_HEIGHT - 22.0, self.view.scrub);
                });
        }
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

        if self.screenshot.is_some() && self.mode == Mode::Evolve && !self.shot_primed {
            self.shot_primed = true;
            self.prime_scripted_run();
        }

        if let Err(err) = self.handle_screenshot(ctx) {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

