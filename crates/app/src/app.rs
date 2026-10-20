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
use std::time::{Duration, Instant};

use egui::{Color32, Context, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use evo::GaConfig;
use sim::{Level, Tile};

use crate::atlas::{self, Atlas};
use crate::editor::{free_level_name, Brush, Editor, Shape};
use crate::help;
use crate::lab::{self, Lab, Lanes, Packet};
use crate::panel::{self, BestLine, Command, RunSummary, ViewState};
use crate::play;
use crate::replay::Replay;
use crate::shelf;
use crate::splash::{Action, Splash};
use crate::theme;
use crate::ui;
use crate::view::{self, Camera, RenderOpts, Sheet, Sprites};
use render::chart::HistoryPoint;

/// Height of the fitness chart's strip at the bottom of the window.
const CHART_PANEL_HEIGHT: f32 = 150.0;

/// How long the shutter between two of the window's four views stays closed.
///
/// Short on purpose. A mode change here is a whole different screen — sky, hills and agents where
/// graph paper and a brush were — and an instant cut between two such screens reads as a glitch
/// rather than as a change; but this is a tool that is switched between constantly while a level
/// is drawn, and anything longer than a fifth of a second is a tax on the person doing that
/// (AUDIT.md V1). The same order as the title screen's own camera ease, which is where the
/// window's idea of "a movement" was already set.
const MODE_FADE: f32 = 0.18;

/// How long a discovery card stays on screen, slow-motion included.
///
/// Ten seconds rather than the genius note's six: a genius note is a jump in a number and can be
/// read at a glance, while a find is a name, a sentence and a route to watch. It is still a
/// moment rather than a dialog — it fades without being dismissed, and the codex keeps it.
const DISCOVERY_SECONDS: f32 = 10.0;

/// Points per tile the follow camera zooms to.
///
/// Following means looking at one agent, and the whole-level fit would make that
/// pointless: at a fit zoom a 60-tile level draws a six-pixel agent. The zoom is only
/// raised to this, never lowered, so a viewer who has zoomed in further keeps their zoom.
///
/// The title screen reads it too: a champion there is followed at this same zoom, because
/// the agent on the front door should be the size of the agent in the replay rather than the
/// size a fit happens to make it.
pub(crate) const FOLLOW_ZOOM: f32 = 34.0;

/// Which of the four things the window is doing.
///
/// The first three are the plan's (§12 M4): draw a level, play it yourself, evolve on it.
/// The fourth is not a fourth activity so much as the answer to the third — the run's best
/// agent running the level at the speed the game runs at, with the population it came from
/// ghosted beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Play,
    Evolve,
    Replay,
}

impl Mode {
    /// The four modes in the order the dock's switch draws them, which is also the order they were
    /// reached in before there was a switch: what a document is, then the three things to do with it.
    ///
    /// One list rather than a `match` at each end, so the cell the pointer picks and the mode it
    /// selects cannot drift apart — an off-by-one in a hand-written pair of matches turns *Play* into
    /// *Evolve*, which is a bug nobody would look for in a tool bar.
    pub const ALL: [Mode; 4] = [Mode::Edit, Mode::Play, Mode::Evolve, Mode::Replay];

    /// This mode's cell in [`Mode::ALL`].
    pub fn index(self) -> usize {
        match self {
            Mode::Edit => 0,
            Mode::Play => 1,
            Mode::Evolve => 2,
            Mode::Replay => 3,
        }
    }
}

/// How the window is set up to open, and how a run on it is set up, from the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabOptions {
    /// Open in the playtest rather than the editor.
    ///
    /// The one field here that is not about a run: the playtest is the mode a person enters to
    /// *race* a run, and racing needs the AI's best time, which only exists after something has
    /// been scored. Started this way it is still worth the flag — the window opens on the level's
    /// physics with the clock running, which is what a person who drew a level wants next — and
    /// `--evolve` is how a script asks for both `--play` and a run to beat.
    pub play: bool,
    /// Open with the AI's **deliberate** controller at the keys: plan the level with the
    /// real simulator and play the plan, instead of waiting for a population to find it.
    ///
    /// This is the System-2 half of the agent. It does not replace a run — a run is still
    /// how a fast policy is bred — but it is the answer on a level where a reflex has no
    /// timing to learn from, and `T` in any playtest turns it on and off.
    pub think: bool,
    /// Open in the evolution view rather than the editor.
    pub evolve: bool,
    /// Open in the replay of the run's quickest win rather than the editor.
    ///
    /// It starts a run, like `--evolve`, and takes it to its cap first when one was given:
    /// a replay needs something scored, and with `--generations N` what is replayed is the
    /// whole run's best agent rather than the first generation's.
    pub replay: bool,
    /// Whether a replay opens with the last scored generation ghosted.
    ///
    /// On unless `--no-ghosts` says otherwise: the field is what the view exists to show.
    /// The key `g` and the panel's checkbox flip the same thing at the keyboard, so the
    /// flag is an opening position rather than a separate mode.
    pub ghosts: bool,
    pub pop_size: usize,
    pub seed: u64,
    /// Stop after this many generations. `None` runs until the window is closed.
    pub generations: Option<u32>,
    /// Where `Save champion` writes, and where a curriculum step's checkpoint goes.
    pub champions: PathBuf,
    /// Start the run from a population checkpoint instead of a fresh population, as
    /// `train-cli watch --resume` does.
    pub resume: Option<PathBuf>,
    /// Start a **new** population from one saved champion, as `train-cli train
    /// --from-champion` does. The alternative to [`LabOptions::resume`], not a second step:
    /// a checkpoint is a population and a champion is one genome.
    pub from_champion: Option<PathBuf>,
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
            play: false,
            think: false,
            evolve: false,
            replay: false,
            ghosts: true,
            pop_size: GaConfig::default().pop_size,
            seed: GaConfig::default().master_seed,
            generations: None,
            champions: PathBuf::from("champions"),
            resume: None,
            from_champion: None,
            curriculum: None,
            curriculum_generations: lab::DEFAULT_CURRICULUM_GENERATIONS,
        }
    }
}

/// What a document-replacing control was asked to do.
///
/// One type for **New** and for a card on the shelf because they are one act — the document on
/// the canvas is thrown away — and because the question a dirty document asks has to be answered
/// about *a* replacement rather than about whichever control happened to ask it. It is kept
/// rather than performed when the answer is "not while there is unsaved work", which is what
/// lets the notice under the dock offer the same replacement again ([`EvoApp::discard_notice`]).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Replace {
    /// A blank level.
    New,
    /// The level at this path.
    Open(PathBuf),
}

impl Replace {
    /// How the question and the status line name this replacement.
    fn label(&self) -> String {
        match self {
            Replace::New => "New".to_string(),
            Replace::Open(path) => {
                let file = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                format!("Opening {file}")
            }
        }
    }
}

/// Whether a document-replacing control should refuse this press, and what to remember about it.
///
/// The rule behind [`EvoApp::refuses_to_discard`], split out as a pure function so the pair of
/// facts it is made of can be tested without a window: `(refused, armed)`. A clean document is
/// never asked about; a dirty one is refused once and the press is remembered against the
/// document's **content hash**; the same content pressed again goes through; and an edit in
/// between is a different document, so it is asked about again — which is exactly the sequence
/// a person who *is* looking at the level produces.
fn discard_guard(dirty: bool, armed: Option<u64>, hash: u64) -> (bool, Option<u64>) {
    if !dirty || armed == Some(hash) {
        return (false, None);
    }
    (true, Some(hash))
}

/// The string the OS window's title bar should be wearing, for a mode and a document title.
///
/// Pure, so the *shape* of the rule is testable without a window: the document leads, because
/// that is the part a narrow taskbar keeps and the part that tells two windows of this game
/// apart; the mode follows, because "which of the four things is this window doing" is the
/// second question asked of a title; the app's name trails, where a title bar puts it.
fn window_title_for(mode: Mode, doc: &str) -> String {
    match mode {
        Mode::Edit => format!("{doc} — one two hundred"),
        Mode::Play => format!("{doc} — playtest — one two hundred"),
        Mode::Evolve => format!("{doc} — evolve — one two hundred"),
        Mode::Replay => format!("{doc} — replay — one two hundred"),
    }
}

/// Open the window and run until the user closes it.
pub fn run(
    level: PathBuf,
    assets: PathBuf,
    screenshot: Option<PathBuf>,
    lab: LabOptions,
    splash: Option<bool>,
) -> Result<(), String> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1360.0, 800.0])
        .with_min_inner_size([900.0, 560.0])
        .with_title("one two hundred");
    // The runner, at four times its sixteen pixels: the size a taskbar's icon slot wants, cut
    // from the game's own art rather than drawn twice (AUDIT.md F5). No icon at all when the
    // assets cannot be read — the atlas load below reports *that* failure in full, and a second
    // complaint about the same missing directory would help nobody.
    if let Some(icon) = atlas::window_icon(&assets, "player_idle_a", 4) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "one two hundred",
        options,
        Box::new(move |cc| {
            let app = EvoApp::new(cc, level, assets, screenshot, lab, splash)?;
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
    /// The deliberate controller driving the playtest, when `T` (or `--think`) turned it
    /// on. Kept beside the session rather than inside it: the session is the world, and
    /// the controller is a policy about it — the same separation the evolved genome has.
    driver: Option<evo::Driver>,
    /// Whether the deliberate controller is at the keys right now.
    thinking: bool,
    levels_dir: PathBuf,
    /// Pending grid size, applied by the Resize button.
    pending_size: (i32, i32),
    grid: bool,
    /// Whether the inspector is showing the level's *numbers* — its size, what it is made of,
    /// the physics it may ask for — or just its verdict.
    ///
    /// Closed by default, and that is the point rather than a preference: a document's
    /// diagnostics are not what a person looks at while drawing, and a panel that shows them
    /// all the time is a panel nobody reads. The verdict stays, because that one is a fact
    /// about the level rather than a fact about the sim.
    details_open: bool,
    /// Brush being dragged out of the palette, if any.
    dragging: Option<Brush>,
    /// Pointer position from the previous frame, for panning.
    last_pointer: Option<Pos2>,
    /// The tile under the pointer, in the camera's own coordinates.
    ///
    /// Kept rather than recomputed in the status bar: the bar has the screen position but
    /// not the canvas rectangle, which is what turning one into the other needs — and a
    /// readout that divides the pointer by the zoom (which is what this used to do) prints
    /// a tile that is only the right one while the camera sits at the origin.
    hover_tile: Option<(i32, i32)>,
    /// A loaded atlas problem, shown once instead of panicking mid-frame.
    warning: Option<String>,
    /// Render one frame, save it, and leave. See `--screenshot`.
    screenshot: Option<PathBuf>,
    shot_requested: bool,
    frames: u32,
    fit_pending: bool,
    /// The title screen, while it is up. `None` for the rest of the session: it is dismissed
    /// once and never comes back, and holding a finished one would keep a level, a session
    /// and an animation alive behind every view the app draws afterwards.
    splash: Option<Splash>,

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
    /// The run's quickest win being replayed, with the last scored generation ghosted.
    ///
    /// A run that has not won yet replays its best-scoring attempt instead
    /// ([`crate::replay::StarBasis`]).
    ///
    /// `Some` only in [`Mode::Replay`]: the roster is built on the way in from the newest
    /// packet and dropped on the way out, so a stale field can never be drawn under a run
    /// that has moved on.
    replay: Option<Replay>,
    view: ViewState,
    /// `n`: run exactly one generation, then pause.
    single_step: bool,
    /// Seconds left on the genius note (§10 Tier 1.3). The panel's own genius flag lasts
    /// as long as its generation does; a splash has to outlive the packet to be seen.
    genius_left: f32,
    /// **The newest find and its remaining seconds (WP16).** The card is the moment; the packet's
    /// own field only lasts one generation, and a discovery deserves long enough to be read.
    discovery: Option<(evo::Discovery, f32)>,
    /// The speed to return to after a discovery's slow-motion moment, when the viewer has not
    /// chosen a different one in the meantime.
    speed_before_discovery: Option<u32>,
    /// The codex page, loaded when `K` opens it rather than read per frame.
    codex_view: Option<evo::Codex>,
    /// Whether the trick codex is up.
    codex_open: bool,
    /// A transient status note and its remaining seconds.
    note: Option<(String, f32)>,
    /// The mutation rate the run has actually been told about, so the worker is not
    /// woken every frame to be told the same number again.
    applied_rate: Option<f32>,
    /// The scripted path has primed the run with a real generation: `--screenshot` needs a
    /// frame with a generation in it, and `--replay` needs something scored to replay.
    primed: bool,
    /// A run that stopped at its cap has been saved once. The terminal writes the best
    /// genome when a watched run ends, and this is the same promise kept per run rather
    /// than per keypress.
    saved_end: bool,

    // --- the level shelf ---------------------------------------------------
    /// Whether the shelf is open. A window rather than a menu, because a menu is a list of
    /// names and the whole point of the shelf is the picture beside each name.
    shelf_open: bool,
    /// The pack as it was when the shelf was last opened, or last written to. Scanning it per
    /// frame would be a directory read and a parse per level per frame (see [`shelf::scan`]).
    shelf: Vec<shelf::Entry>,

    // --- the document's own state ------------------------------------------
    /// The content hash the "this would discard your work" warning was armed against.
    ///
    /// `Some(hash)` means the last press was *refused* because the document was edited, and
    /// this is what it looked like at the time (see [`EvoApp::refuses_to_discard`]).
    discard_armed: Option<u64>,
    /// The replacement a dirty document refused, while the question is on screen.
    ///
    /// `None` means there is nothing to ask: either no replacement was attempted, or the one
    /// that was has been answered. It is a *pending act* rather than a flag so the notice's
    /// own button can perform exactly what was asked for.
    discard_wanted: Option<Replace>,
    /// The string currently on the OS window's title bar. Held so the title is only ever set
    /// when it actually changes (see [`EvoApp::sync_window_title`]).
    os_title: String,
    /// Whether the key card is up ([`help`]).
    help_open: bool,
    /// Seconds left of the shutter between two modes, and the mode the last frame was in.
    ///
    /// The comparison is made *after* a frame has drawn rather than by every writer of `mode`
    /// remembering to say so: there are four places that change it — [`EvoApp::set_mode`], two key
    /// handlers and [`EvoApp::open_level`] — and a transition that only happened on the polite
    /// one would be worse than none at all.
    mode_fade: f32,
    last_mode: Mode,
    /// The rename field's working text, and whether the field is being typed in right now.
    ///
    /// A buffer rather than a `TextEdit` bound straight to the document, because a half-typed
    /// name must not be pushed into the level and its undo history one keystroke at a time. The
    /// `bool` is what keeps the buffer honest: while the field is focused it belongs to the
    /// typist, and the moment it is not, it is reseeded from the document — so a name that
    /// changed some other way (an undo, a save that numbered a collision, the shelf opening
    /// another level entirely) can never leave this field showing the previous document's name.
    rename_buf: String,
    rename_focused: bool,
    /// Where *Levels* and *New* were drawn, for the tests that press them.
    ///
    /// Test-only, and deliberately so: a running window reaches these controls through
    /// their own responses, and nothing here is read outside `#[cfg(test)]`. It exists
    /// because the alternative is a test that guesses at the dock's layout — and the row is
    /// as wide as the document's name and the mode switch, so the guess would be wrong the
    /// first time somebody renamed a level.
    #[cfg(test)]
    tool_rects: Vec<(&'static str, Rect)>,
}

impl EvoApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        level_path: PathBuf,
        assets: PathBuf,
        screenshot: Option<PathBuf>,
        options: LabOptions,
        splash: Option<bool>,
    ) -> Result<EvoApp, String> {
        EvoApp::build(
            &cc.egui_ctx,
            level_path,
            assets,
            screenshot,
            options,
            splash,
        )
    }

    /// The window, built from a context rather than from a creation event.
    ///
    /// Split out so the whole window can be **driven without a window**: a creation context
    /// exists only inside `eframe`, which left every control in the dock and every card on the
    /// shelf untested at the one thing they are for — being pressed and changing the document.
    /// The tests below run real frames and click real buttons through this, so "New clears the
    /// level" is a fact about the window rather than a fact about `Editor::fresh`.
    fn build(
        ctx: &Context,
        level_path: PathBuf,
        assets: PathBuf,
        screenshot: Option<PathBuf>,
        options: LabOptions,
        splash: Option<bool>,
    ) -> Result<EvoApp, String> {
        // The window's own look, installed once: every panel, button and slider after this
        // line draws itself in the game's palette rather than egui's stock one.
        theme::apply(ctx);
        // Sound starts with the window: the device opens once here and every
        // effect after this is a `crate::sound::play` away. A machine with no
        // audio device leaves the game silent rather than unstarted.
        crate::sound::init();
        let atlas = Atlas::load(ctx, &assets)?;
        let sprites = Sprites::load(&atlas)?;

        let levels_dir = level_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("levels"));
        // Where a champion for this level would be: the run's own output directory, which is
        // also where the window's Save champion writes.
        let champions_dir = options.champions.clone();
        let editor = match Editor::open(&level_path) {
            Ok(editor) => editor,
            Err(err) => {
                // A *scripted* frame is the one caller that cannot read the message below.
                // `--screenshot` exists to verify the renderer without a human watching, and
                // a PNG of a blank 60 × 15 level is a green check for a level that never
                // opened: the exit code is the only channel that script has (AUDIT.md B3), so
                // an unreadable `--level` fails the process rather than photographing the
                // fallback. A person at a window still gets their editor — opening a bad file
                // must not lose the window, and they can see what it says.
                if screenshot.is_some() {
                    return Err(format!("--level {}: {err}", level_path.display()));
                }
                // The fallback is a *new document*, so it takes a free name like any other:
                // two failed opens in one directory must not both want `untitled.json`.
                let name = free_level_name(&levels_dir, "untitled");
                let mut editor = Editor::fresh(&name, 60, 15);
                editor.message = format!("could not open {}: {err}", level_path.display());
                editor
            }
        };
        let pending_size = (editor.level.width, editor.level.height);

        // The title screen is the window's first frame, except for a scripted one: the editor
        // is what `--screenshot` exists to photograph and a run is what `--evolve` and
        // `--replay` do, and a splash in front of either would hold a script up for as long as
        // nobody presses a key. `--splash --screenshot` asks for the screen on purpose, and
        // gets it finished rather than half-revealed, because a still cannot show a reveal.
        //
        // It plays the run's own champion for this level when the champions directory holds
        // one — the level is the document this window opened, so what is on the title screen
        // is what is about to be edited — and the demo run when it does not.
        let splash = if splash.unwrap_or(screenshot.is_none()) {
            let mut splash =
                Splash::open(&editor.level, &champions_dir, Some(level_path.as_path()));
            if screenshot.is_some() {
                splash.skip_opening();
            }
            Some(splash)
        } else {
            None
        };

        let mut app = EvoApp {
            atlas,
            sprites,
            camera: Camera::default(),
            editor,
            mode: Mode::Edit,
            session: None,
            driver: None,
            thinking: false,
            levels_dir,
            pending_size,
            grid: true,
            details_open: false,
            dragging: None,
            last_pointer: None,
            hover_tile: None,
            warning: None,
            screenshot,
            shot_requested: false,
            frames: 0,
            fit_pending: true,
            splash,
            seed: options.seed,
            options,
            lab: None,
            lanes: None,
            packet: None,
            summary: None,
            replay: None,
            view: ViewState::default(),
            single_step: false,
            saved_end: false,
            genius_left: 0.0,
            discovery: None,
            speed_before_discovery: None,
            codex_view: None,
            codex_open: false,
            note: None,
            applied_rate: None,
            primed: false,
            shelf_open: false,
            shelf: Vec::new(),
            discard_armed: None,
            discard_wanted: None,
            os_title: String::new(),
            rename_buf: String::new(),
            rename_focused: false,
            #[cfg(test)]
            tool_rects: Vec::new(),
            help_open: false,
            mode_fade: 0.0,
            // The editor, always — and the comparison in `update` does the rest: a `--play` or
            // `--evolve` launch enters its mode below, before the first frame, so the first frame
            // sees the difference and the window opens *into* its view behind the same shutter the
            // switch uses rather than cutting to it.
            last_mode: Mode::Edit,
        };
        // `--replay` is a run too: it opens the evolution view, scores what it was asked
        // for, and then shows the best agent it found.
        if app.options.evolve || app.options.replay {
            app.enter_evolve();
        } else if app.options.play {
            // Opening on the playtest, for the person who has just built a level and wants to run
            // it: the clock starts at the spawn and the run's best time — when this window has one
            // — is what the line in the dock is measured against.
            app.enter_play();
        }
        Ok(app)
    }

    fn opts(&self) -> RenderOpts {
        match self.mode {
            Mode::Edit => RenderOpts {
                grid: self.grid,
                ..RenderOpts::editing()
            },
            Mode::Play | Mode::Evolve | Mode::Replay => RenderOpts::playing(),
        }
    }

    /// The terrain the canvas is actually drawing.
    ///
    /// The replay's level during a replay and the run's while watching one — an edit made
    /// since a run started is a *different* level, and it is the one thing on this screen
    /// that is not on the screen. Fitting the camera to the document instead is how a
    /// "fit the level" ends up pointing at the wrong picture; the editor and the playtest
    /// are the two views that really are about the document.
    fn viewed_level(&self) -> &Level {
        match self.mode {
            Mode::Replay => self
                .replay
                .as_ref()
                .map(|replay| replay.level())
                .unwrap_or(&self.editor.level),
            Mode::Evolve => self
                .lab
                .as_ref()
                .map(|lab| lab.level())
                .unwrap_or(&self.editor.level),
            Mode::Edit | Mode::Play => &self.editor.level,
        }
    }

    fn enter_play(&mut self) {
        // A replay is only valid while it is the mode on screen: leaving it for the
        // keyboard drops the pool of episodes rather than parking a second simulation
        // behind the one being played. Going back rebuilds it from the same packet.
        self.replay = None;
        // Item 7: every human attempt is recorded as data until it ends. A completed one is
        // written beside the champions (the play module owns the naming and the format); a death
        // or an abandoned run writes nothing, because a demonstration has to reach the flag to be
        // one. The directory is a working-directory sibling of `champions/` so a checkout's demos
        // are inspectable without knowing an app-internal path.
        let mut session = play::Session::new(&self.editor.level, 1);
        session.capture_demos("demos");
        self.session = Some(session);
        self.driver = None;
        self.thinking = self.options.think;
        if self.thinking {
            self.start_thinking();
        }
        self.attach_champion_ghost();
        self.mode = Mode::Play;
    }

    /// Put the deliberate controller at the keys.
    ///
    /// The whole-route planner first: it is the one that can wait out a deck's cycle, and
    /// it costs a few seconds once rather than a search per tick. If the level has no
    /// route it falls back to the local model-based controller, which is worse at long
    /// waits but needs no global plan and reacts to the body's actual state.
    fn start_thinking(&mut self) {
        if self.driver.is_some() {
            self.thinking = true;
            return;
        }
        let planned = evo::Driver::planned(&self.editor.level, 1, sim::plan::PlanConfig::default());
        let whole = planned.is_some();
        self.driver =
            Some(planned.unwrap_or_else(|| evo::Driver::local(evo::ThinkConfig::default())));
        self.thinking = true;
        self.note = Some((
            if whole {
                "the AI planned the level — T gives the keys back".to_string()
            } else {
                "the AI is thinking a few moves ahead — T gives the keys back".to_string()
            },
            4.0,
        ));
    }

    /// Attach the record-holder as the playtest's ghost, when there is one to race.
    ///
    /// **The ghost is the episode the dock's time was measured on**, not merely some good
    /// agent: [`Packet::fastest_agent`] is the genome and seed behind [`Packet::fastest`], and
    /// the two are written together by the trainer. So the ghost reaches the flag on exactly
    /// the tick the line beside it quotes, and the player is racing one record rather than a
    /// time from one agent and the shape of another. Taking the best-ever genome instead — a
    /// different agent the moment a run has two winners, and often not a winner at all — put a
    /// ghost on screen that could die, or finish seconds late, while the line above it claimed
    /// a time it had never run. That was the bug this signature fixes.
    ///
    /// Nothing to race yet is not an error and not a placeholder: with no win recorded there is
    /// no time to quote either, so the playtest is simply solo until there is one, and the
    /// ghost button says so. The guard on the level is the other half — an edit since the run
    /// started makes the run's numbers a claim about terrain nobody can see
    /// ([`Lab::trained_on`]), and a ghost replayed on the wrong level would be a second player
    /// rather than a record.
    fn attach_champion_ghost(&mut self) {
        let (Some(lab), Some(packet), Some(session)) = (&self.lab, &self.packet, &mut self.session)
        else {
            return;
        };
        if !lab.trained_on(&self.editor.level) {
            return;
        }
        let Some(record) = &packet.fastest_agent else {
            return;
        };
        session.attach_ghost(play::GhostSpec {
            genome: record.genome.clone(),
            eval: lab.eval_config(),
            seed: record.result.seed,
        });
    }

    fn leave_play(&mut self) {
        self.mode = Mode::Edit;
        self.session = None;
        // The body is gone, so the pool it was standing beside is silent.
        crate::sound::set_ambience(0.0, 0.0);
    }

    // -----------------------------------------------------------------------
    // the replay
    // -----------------------------------------------------------------------

    /// Watch the run's quickest win play the level, with the population ghosted beside it.
    ///
    /// The roster comes from the newest scored packet: the episode the run's quickest time was
    /// measured on, and the population of that packet's generation — the last one the run scored,
    /// which is older than the record's own generation as soon as the search moves past it. Building
    /// it trains nothing and the run stays parked behind this view, where it was, until the viewer
    /// goes back to it.
    ///
    /// **The quickest win, not the champion**, and the same record the playtest's ghost is built
    /// from ([`Self::attach_champion_ghost`]): the champion is the highest-*fitness* genome, and
    /// quickness is only part of that score, so following it put a slower agent in front of a field
    /// of ghosts that reached the flag first — while the dock and the strip quoted the quicker time
    /// that agent had never run. A viewer racing a clock gets the agent that set it; a run that has
    /// never won has no clock and gets its best attempt, and says so
    /// ([`crate::replay::StarBasis`]).
    fn enter_replay(&mut self) {
        let built = match (&self.lab, &self.packet) {
            (Some(lab), Some(packet)) => lab.replay(packet),
            (None, _) => {
                self.editor.message = "no run on this level yet — press E to train on it".into();
                return;
            }
            (_, None) => {
                self.editor.message =
                    "no generation scored yet: a replay needs an agent to watch".into();
                return;
            }
        };
        let Some(mut replay) = built else {
            // No packet, or no record in one: the first evaluation always sets a best-scoring
            // agent, so this is a window that has not been handed a scored generation at all.
            self.editor.message = "this run has no agent to replay yet".into();
            return;
        };
        // The opening view is the whole level, because the field is the thing this mode
        // exists to show: a camera pinned to the star would hide most of the population
        // behind the frame's edges, and "who else got through" is unreadable from a window
        // that follows one agent. `f` follows the star for a viewer who wants that
        // instead, and `f` again brings the whole field back.
        self.view.follow = false;
        self.fit_pending = true;
        replay.ghosts = self.options.ghosts;
        self.note = None;
        self.replay = Some(replay);
        self.mode = Mode::Replay;
    }

    /// Leave the replay for `to`: the run it came from, or the editor.
    fn leave_replay(&mut self, to: Mode) {
        self.replay = None;
        self.mode = to;
        crate::sound::set_ambience(0.0, 0.0);
        // Following was the replay's opening view, and the camera is left wherever the
        // star stood: going back to a view that is not following gets the whole level
        // back, which is the same rule the follow toggle keeps.
        if to == Mode::Edit || !self.view.follow {
            self.fit_pending = true;
        }
    }

    /// Is there anything to replay? A run has to have scored a generation.
    fn can_replay(&self) -> bool {
        self.lab.is_some() && self.packet.is_some()
    }

    /// One frame of the replay: step the clock unless the viewer has paused it.
    fn replay_step(&mut self, dt: f32) {
        if let Some(replay) = &mut self.replay {
            if !replay.paused {
                replay.advance(dt);
            }
        }
    }

    /// The replay's own keys: playback, the ghosts, and the camera.
    ///
    /// The run's controls — step a generation, save the champion, scrub the record, dial
    /// the mutation rate — belong to the view the replay was entered from, so they are
    /// dropped here rather than ignored further down: a key that does nothing visible but
    /// moves the run behind the curtain is worse than a key that does nothing.
    fn replay_keys(&mut self, ctx: &Context) {
        let keys = ctx.input(|i| panel::Keys {
            pause: i.key_pressed(egui::Key::Space),
            faster: i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
            slower: i.key_pressed(egui::Key::Minus),
            follow: i.key_pressed(egui::Key::F),
            ghosts: i.key_pressed(egui::Key::G),
            ..panel::Keys::default()
        });
        // Which key means what is still `panel::commands_for`'s business; the replay only
        // narrows the set of commands it is prepared to act on.
        for command in panel::commands_for(keys, &self.view) {
            match command {
                Command::TogglePause
                | Command::ToggleGhosts
                | Command::ToggleFollow
                | Command::Speed(_) => self.apply(command),
                _ => {}
            }
        }
    }

    /// Put the camera on the agent being replayed.
    ///
    /// Not `follow_leader`: that rule belongs to a generation, where "furthest along" is
    /// the question being asked. Here there is one agent the view exists for, and it is
    /// that one.
    fn follow_champion(&mut self) {
        let Some(replay) = &self.replay else {
            return;
        };
        let p = replay.star_sim().player();
        self.camera.centre = Vec2::new(p.x + p.w * 0.5, p.y + p.h * 0.5);
        self.camera.zoom = self.camera.zoom.max(FOLLOW_ZOOM);
        self.camera.clamp_to(replay.level(), 2.0);
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
        // A run has no single body to stand beside a pool: the beds belong to
        // a playtest or a replay, so the last one is faded out here.
        crate::sound::set_ambience(0.0, 0.0);
        // Training needs something to reach. A document whose flag has been erased has no
        // progress to score and no bonus to win, so a run on it is a room of agents walking
        // into the wall under a "0 tiles to go" readout — say why instead of starting one.
        // The flag is one click away, and the message lands in the status bar of the view the
        // user is still in.
        if self.editor.level.goals.is_empty() {
            self.editor.message =
                "no goal on this level: place the flag before training on it".into();
            return;
        }
        if self.lab.is_none() {
            let cfg = GaConfig {
                pop_size: self.options.pop_size,
                master_seed: self.seed,
                // **The window is a mastery league by default** (WP11). A watched run's whole
                // promise is that it keeps getting better after it solves the level, and before
                // this the window had no way to ask for that: a run that found the flag once
                // went on optimising the crowned fitness, which barely notices the clock. The
                // league arms only after the record wins, so learning the level is unchanged.
                mastery: true,
                // **Item 14 in the window.** A watched run is where a mistake is legible: the
                // stall, the pit, the frame the body died. Three repair children per generation
                // are bred from the best genome's own losing decisions, so the population spends
                // part of each generation fixing what it just got wrong instead of waiting for a
                // random mutation upstream of the failure to come good. `--repair-children` is
                // the same knob in the terminal, and `GaConfig::default()` keeps it off.
                repair_children: 3,
                ..GaConfig::default()
            };
            let plan = match self.run_plan() {
                Ok(plan) => plan,
                Err(err) => {
                    self.editor.message = err;
                    return;
                }
            };
            let stem = self.run_stem();
            match Lab::start(
                &self.editor.level,
                cfg,
                lab::DEFAULT_BEST_LANES,
                lab::DEFAULT_RANDOM_LANES,
                plan,
                self.options.champions.clone(),
                stem,
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
            seed_from_champion: self.options.from_champion.clone(),
            curriculum,
            // A fresh run on a level somebody has just drawn starts from a walk of it; a run
            // handed a population or a champion already has the better starting point. See
            // `lab::RunPlan::warm_from_plan`.
            warm_from_plan: self.options.resume.is_none() && self.options.from_champion.is_none(),
        })
    }

    /// The stem this run's artifacts wear: the document's file stem when it has one, and the
    /// slugged display name only for a document that has never been saved.
    ///
    /// The file wins because it is what the rest of the workspace is keyed on — the shelf looks
    /// for a champion beside the level under the *file's* stem, `train-cli` writes
    /// `<file stem>.json`, and the title screen looks there first. A custom copy of a shipped
    /// level keeps the shipped level's name, so naming the champion after the name put a policy
    /// trained on one level over the pack's own champion for another (see
    /// [`lab::artifact_stem`]).
    fn run_stem(&self) -> String {
        lab::artifact_stem(&self.editor.level, self.editor.path.as_deref())
    }

    /// Start a fresh population on the level as it stands, on the next seed up.
    ///
    /// The seed is bumped rather than reused: a Restart that reproduced the identical
    /// run would be a button with no effect. Bumping it keeps every run reproducible from
    /// the number the status bar shows.
    fn restart_run(&mut self, why: &str) {
        self.seed = self.seed.wrapping_add(1);
        let stem = self.run_stem();
        let Some(lab) = &mut self.lab else {
            return;
        };
        lab.restart(&self.editor.level, self.seed, stem);
        lab.set_mutation_rate(self.view.mutation_rate);
        self.reset_run_view();
        self.note = Some((format!("{why} (seed {})", self.seed), 6.0));
    }

    /// Start a new population from the champion **saved for this level**.
    ///
    /// The window's half of `train-cli train --from-champion`, and the answer to the one hole
    /// a saved champion left: the file on disk was readable only by the terminal, so restarting
    /// the game meant restarting the search from nothing. The champion is loaded before the old
    /// run is touched — a file this build cannot train on leaves the run where it was and says
    /// why — and the new run starts at generation 0 with the champion's own episode as its
    /// record, exactly as the terminal's seeded run does.
    ///
    /// The file is the one [`Lab::champion_path`] names, which is where [`Self::save_champion`]
    /// writes: a level's champion and the control that continues it cannot name two files.
    fn continue_from_champion(&mut self) {
        let Some(lab) = &self.lab else {
            self.note = Some((
                "start a run first: there is no champion saved yet".into(),
                6.0,
            ));
            return;
        };
        let path = lab.champion_path();
        let seed = match lab::champion_seed(&path, &self.editor.level) {
            Ok(seed) => seed,
            Err(err) => {
                self.note = Some((
                    format!(
                        "no champion to continue at {} ({err}) — S saves this run's best first",
                        path.display()
                    ),
                    10.0,
                ));
                return;
            }
        };
        // A new seed, like Restart: a continued run has to be reproducible from the number on
        // screen, and reusing the old one would make the new shell the old one's duplicate.
        // It is only adopted once the restart is accepted, so a champion this run cannot
        // train on leaves the seed showing the run that is actually running.
        let next_seed = self.seed.wrapping_add(1);
        let stem = self.run_stem();
        let Some(lab) = &mut self.lab else {
            return;
        };
        if let Err(err) = lab.restart_from_champion(&self.editor.level, next_seed, stem, seed) {
            self.note = Some((format!("cannot continue from the champion: {err}"), 10.0));
            return;
        }
        self.seed = next_seed;
        lab.set_mutation_rate(self.view.mutation_rate);
        let origin = lab
            .origin()
            .unwrap_or("seeded from a saved champion")
            .to_string();
        self.reset_run_view();
        self.note = Some((format!("{origin} (seed {})", self.seed), 10.0));
    }

    /// Drop everything the screen holds about the run it was watching.
    ///
    /// Shared by the two ways a run starts again — a fresh population and one continued from a
    /// champion — so a new run cannot leave the old chart, record or scrub position on screen.
    fn reset_run_view(&mut self) {
        self.lanes = None;
        self.packet = None;
        self.summary = None;
        self.single_step = false;
        self.view.scrub = None;
        self.saved_end = false;
        self.applied_rate = Some(self.view.mutation_rate);
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
        self.tick_discovery(dt);

        // The run's rate limit, and the reason a paused window costs nothing: while a
        // generation is on screen — or while the viewer is paused or scrubbing — nothing
        // asks the worker for another one, so it stays parked instead of training ahead.
        //
        // Since the worker is now asked for one generation *ahead* (see the lookahead in
        // the take below), this is the second half of the same rule rather than the whole
        // of it: it covers the frame a generation finishes and the next one is not queued
        // yet, and the first generation of a run, where there is nothing on screen to
        // take. `Lab` refuses a second request while one is in flight, so the two paths
        // cannot ask twice between them.
        if self.lanes.is_none() && !self.view.paused && self.view.scrub.is_none() {
            if let Some(lab) = &mut self.lab {
                if !lab.asked() {
                    lab.request_next();
                }
            }
        }
        let taken = self.lab.as_mut().and_then(|lab| lab.take());
        let mut ended = false;
        if let Some(lanes) = taken {
            let packet = lanes.packet().clone();
            if packet.genius {
                self.genius_left = 6.0;
            }
            if let Some(found) = packet.discovery.clone() {
                self.show_discovery(found);
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

            // **One generation of lookahead.** Ask for the next generation as soon as this
            // one is on screen, rather than when its animation ends: the worker's own work
            // then hides under animation the viewer was going to watch anyway, instead of
            // happening on a level with nobody on it.
            //
            // It is worth a generation of slack because generations are not all the same
            // size. A scored generation is 27-50 ms at pop 200 on a shipped level, and the
            // one packet that used to be forty times that — the stall's practice detour, forty
            // generations of practice in one piece, **1.73 s** measured on `scratch-tower` — no
            // longer arrives here at all: `lab::Detour` runs it on its own thread and the
            // practised population is adopted the generation after it lands. What is left for
            // the lookahead to hide is ordinary variation, which is what a generation of slack
            // is for.
            //
            // The bound the old rule existed for is unchanged: the worker computes at most
            // one generation beyond the one on screen, because `Lab::request_next` refuses a
            // second request while one is in flight and the worker will not start another
            // until the window has taken what it queued. What moved is only *when* it may
            // start — so a paused or scrubbing window still parks it after the one generation
            // already in flight, rather than during the whole of the generation on screen.
            if !ended && !self.view.paused && self.view.scrub.is_none() {
                if let Some(lab) = &mut self.lab {
                    if !lab.asked() {
                        lab.request_next();
                    }
                }
            }
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
            let goal_x = lab.level().goals.first().map(|goal| goal.x).unwrap_or(0.0);
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
        // A field with the keyboard *has* the keyboard: typing a level's name must not also be
        // pressing P for playtest and E for evolve, which is what happened to every letter this
        // view binds the moment the details window grew a text field. egui knows when a widget
        // wants the keys, so ask it rather than tracking focus in a second place.
        if ctx.wants_keyboard_input() {
            return;
        }
        // The key card is one key in every view, and it is *not* mode-split: the question it
        // answers — "what can I press here?" — is the same question in all four.
        if ctx.input(|i| i.key_pressed(egui::Key::Questionmark) || i.key_pressed(egui::Key::F1)) {
            self.help_open = !self.help_open;
        }
        // Sound on/off. Not in the evolution view: there M already means the
        // mutation rate, and one key with two jobs is a bug report waiting to
        // be filed. The sounds themselves are synthesised in `crate::sound`;
        // this is only the switch.
        if self.mode != Mode::Evolve && ctx.input(|i| i.key_pressed(egui::Key::M)) {
            crate::sound::toggle_mute();
        }
        // **The codex (WP16).** Loaded when the page is opened rather than every frame: it is a
        // file, and a window that re-read it at 60 Hz would be a disk read per frame to show the
        // same journal. It belongs to the document's artifacts, which is where a run writes it.
        if matches!(self.mode, Mode::Evolve | Mode::Replay)
            && ctx.input(|i| i.key_pressed(egui::Key::K))
        {
            self.codex_open = !self.codex_open;
            if self.codex_open {
                self.load_codex_view();
            }
        }
        let (undo, redo, grid, play, evolve, fit, save, restart, watch, think) = ctx.input(|i| {
            (
                // Undo excludes shift so it does not also fire on ctrl+shift+Z: that chord
                // is the standard redo, and leaving undo on it made the pair cancel out —
                // the undo ran and the redo immediately undid it, so the level never moved.
                i.key_pressed(egui::Key::Z) && i.modifiers.command && !i.modifiers.shift,
                (i.key_pressed(egui::Key::Y) && i.modifiers.command)
                    || (i.key_pressed(egui::Key::Z) && i.modifiers.command && i.modifiers.shift),
                i.key_pressed(egui::Key::G),
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::E),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::S) && i.modifiers.command,
                i.key_pressed(egui::Key::R),
                i.key_pressed(egui::Key::V),
                i.key_pressed(egui::Key::T),
            )
        });
        if undo && self.editor.undo() {
            self.editor.message = "undo".into();
        }
        if redo && self.editor.redo() {
            self.editor.message = "redo".into();
        }
        // Grid is the *editor's* furniture — it is only drawn in `Mode::Edit` (`opts`) — so
        // the key belongs to that view alone. Toggling it everywhere made `G` do two things
        // at once in the evolution view, where it also means "ghosts": the ghosts flipped on
        // screen and the editor's grid silently flipped behind them.
        if grid && self.mode == Mode::Edit {
            self.grid = !self.grid;
        }
        // `f` is one key with two meanings, split by mode rather than by modifier: in the
        // editor it fits the level in the window, and in the views that watch an agent
        // move it is the terminal's follow/whole-level toggle. Both are the same idea —
        // "show me all of it" against "put me on the thing that matters" — and following
        // re-fits when it is switched off, which is what makes the modes agree about the
        // key.
        if fit && matches!(self.mode, Mode::Edit | Mode::Play) {
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
                if watch {
                    self.enter_replay();
                }
            }
            Mode::Play => {
                if play || escape {
                    self.leave_play();
                }
                // `T` hands the level to the deliberate controller, or gives the keys
                // back. It is a toggle rather than a one-way takeover because watching
                // the AI and racing it are the two things this view is for.
                if think {
                    self.thinking = !self.thinking;
                    if self.thinking {
                        // A plan is a route from the spawn, so handing over mid-run
                        // has to start the run again: block 80 of the plan is not the
                        // block this body is on. Restarting is what makes the
                        // takeover exact rather than approximate.
                        if let Some(session) = &mut self.session {
                            session.restart(&self.editor.level);
                        }
                        if let Some(driver) = &mut self.driver {
                            driver.reset();
                        }
                        self.start_thinking();
                    } else {
                        self.note = Some(("the keys are yours".to_string(), 2.0));
                    }
                }
                if restart {
                    if let Some(session) = &mut self.session {
                        session.restart(&self.editor.level);
                    }
                    if let Some(driver) = &mut self.driver {
                        driver.reset();
                    }
                }
                // `grid` is the G key, which the editor claims for its grid: here it is
                // the ghost instead, the same mode-split the F key already works by. A
                // playtest with no champion is simply solo — the key quietly does nothing.
                if grid {
                    if let Some(session) = &mut self.session {
                        session.toggle_ghost();
                    }
                }
            }
            Mode::Evolve => {
                if play || evolve || escape {
                    self.mode = Mode::Edit;
                }
                if watch {
                    self.enter_replay();
                }
                // `f` is handled by `evolve_keys` (it maps to `Command::ToggleFollow` there,
                // like every other key); applying it here too toggled follow twice, which
                // cancelled out. `fit` is only the editor's fit-to-window below.
                self.evolve_keys(ctx);
            }
            Mode::Replay => {
                // The same keys that leave every other view leave this one, plus `v`
                // itself: one key that enters the replay and leaves it, the way `e` and
                // `p` already toggle the views they open.
                if play || evolve || escape || watch {
                    self.leave_replay(Mode::Evolve);
                }
                if restart {
                    if let Some(replay) = &mut self.replay {
                        replay.restart();
                    }
                }
                // As in the evolution view, `f` lives in `replay_keys`; applying it here as
                // well toggled follow twice.
                self.replay_keys(ctx);
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
            ghosts: i.key_pressed(egui::Key::G),
            save: i.key_pressed(egui::Key::S) && !i.modifiers.command,
            continue_champion: i.key_pressed(egui::Key::C) && !i.modifiers.command,
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
        if self.options.replay {
            // `--replay` replayed instead of watching the run: enter with the scored
            // generation already in hand, and step the episode far enough that a captured
            // frame shows agents away from the spawn pad.
            self.enter_replay();
            if let Some(replay) = &mut self.replay {
                for _ in 0..90 {
                    replay.advance(sim::DT);
                }
                // A scripted frame has to be a *fixed* frame: the window goes on stepping
                // the episode in real time while the screenshot is in flight, so a
                // `--screenshot` that left the clock running would capture whatever the
                // machine's frame timing landed on and two runs of one command would
                // disagree by a tick or two. Held here, the frame is the episode's
                // 1.5 s mark every time — which is what makes the render numbers in the
                // FINDINGS.md a measurement rather than a coincidence. A run watched by a
                // human is not scripted and still opens playing.
                if self.screenshot.is_some() {
                    replay.paused = true;
                }
            }
        } else {
            for _ in 0..40 {
                self.evolve_step(1.0 / 60.0);
            }
        }
        // The scripted path took its generations without passing through the take that
        // normally notices a capped run ending, so the same end-of-run save is offered
        // here: a `--screenshot --generations N` run leaves the same artifacts a session
        // at the keyboard would.
        self.save_if_run_ended();
    }

    /// The replay canvas: the level the episode is on, the ghosts, then the star.
    ///
    /// The level comes from the **replay** rather than from the editor: the episode was
    /// scored on the run's terrain, and drawing it on an edit made since would be showing
    /// a different game from the one being replayed.
    ///
    /// `clock` is the replay's own (see the caller), so the level's scenery belongs to the
    /// episode being watched rather than to the window.
    fn canvas_replay(&self, sheet: Sheet<'_>, clock: f32) {
        let Some(replay) = &self.replay else {
            return;
        };
        view::draw_level(sheet, replay.level(), self.opts(), None, clock);
        view::draw_replay(sheet, replay);
    }

    /// The right-hand panel in the evolution view.
    ///
    /// The panel and the Details window are two halves of one board: the panel is the menu a viewer
    /// acts on while a run is going (see [`panel::lab_panel`]) and the window is the instrument board
    /// behind it ([`panel::lab_details`]). Both report the same [`panel::Actions`], so what a widget
    /// *means* is handled once, in [`Self::apply_actions`], wherever it was drawn.
    fn evolve_panel(&mut self, ui: &mut egui::Ui) {
        let Some(summary) = self.summary.take() else {
            // Before the first generation lands there is nothing to summarise, and a panel of zeros
            // would read as a run that is going nowhere rather than one that has not started.
            match &self.lab {
                Some(lab) => {
                    ui.label(format!(
                        "{}: evaluating the first generation…",
                        lab.level_name()
                    ));
                    ui.label(
                        egui::RichText::new(format!("pop {}   seed {}", lab.pop_size(), self.seed))
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                }
                None => {
                    ui.label("no run on this level yet");
                    if ui::menu_row(
                        ui,
                        Some(&self.atlas),
                        Some(ui::GLYPH_PLAY),
                        "Start a run",
                        ui.available_width(),
                        false,
                        true,
                    )
                    .clicked()
                    {
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
        // Whether *Continue champion* has a file, asked now rather than read off the generation:
        // Save and Continue are two presses in one generation, and the second must see what the
        // first wrote.
        let mut summary = summary;
        summary.champion_saved = self
            .lab
            .as_ref()
            .is_some_and(|lab| lab.champion_path().is_file());
        let actions = panel::lab_panel(ui, Some(&self.atlas), &summary, &mut self.view, changed);
        self.summary = Some(summary);
        self.apply_actions(actions);
    }

    /// The right-hand panel in the replay: the transport for one episode, and the way to its numbers.
    fn replay_panel(&mut self, ui: &mut egui::Ui) {
        // Scoped, so the replay is handed to the widgets and the window gets it back before anything
        // is acted on: a widget that changed the state it was drawing would be two implementations of
        // one control.
        let actions = {
            let Some(replay) = &mut self.replay else {
                return;
            };
            panel::replay_panel(ui, Some(&self.atlas), replay, &self.view)
        };
        self.apply_actions(actions);
    }

    /// Do what the panels' widgets asked for.
    ///
    /// One place for one vocabulary: a pause is a pause whether it came from the run's menu, from the
    /// window behind it, from a key, or from the replay's own transport. Every widget reports intent
    /// and nothing acts on itself, which is what keeps a key and a button from disagreeing about what
    /// the same word means.
    fn apply_actions(&mut self, actions: panel::Actions) {
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
        if actions.continue_champion {
            self.apply(Command::ContinueChampion);
        }
        if actions.resume {
            self.apply(Command::Live);
        }
        if actions.follow {
            self.apply(Command::ToggleFollow);
        }
        if actions.ghosts {
            self.apply(Command::ToggleGhosts);
        }
        if actions.scrub_delta != 0 {
            self.apply(Command::Scrub(actions.scrub_delta));
        }
        // The panel's pace buttons reach the run through the same command the `+`/`-` keys do, so a
        // button and a key cannot come to different conclusions about what "one rung faster" means.
        if actions.speed != 0 {
            self.apply(Command::Speed(actions.speed));
        }
        // Not a command: the window itself is what this one opens.
        if actions.details {
            self.details_open = !self.details_open;
        }
        // The run's rate, not the view's: the worker applies it from the next generation on, so a
        // change shows up in the very next offspring.
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
                // The replay has its own clock: pausing it must not pause the run behind
                // it, which is parked rather than stopped and has a state worth keeping.
                if let Some(replay) = &mut self.replay {
                    replay.paused = !replay.paused;
                } else {
                    self.view.paused = !self.view.paused;
                    self.single_step = false;
                }
            }
            Command::StepOne => {
                // Inspecting, not watching: run a generation and then hold still.
                self.view.paused = false;
                self.single_step = true;
            }
            Command::Restart => {
                // In the replay this is "run those episodes again"; in the run, a fresh
                // population. Both are "start over", and one word keeps one meaning per
                // subject rather than a second key for a second verb.
                if self.mode == Mode::Replay {
                    if let Some(replay) = &mut self.replay {
                        replay.restart();
                    }
                } else {
                    self.restart_run("restarted");
                }
            }
            Command::Save => self.save_champion(),
            Command::ContinueChampion => self.continue_from_champion(),
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
            Command::ToggleGhosts => {
                if let Some(replay) = &mut self.replay {
                    replay.ghosts = !replay.ghosts;
                } else {
                    // In the run's own view the key is a pointer at the feature rather
                    // than a control: ghosts belong to the replay, and saying so once is
                    // how a viewer finds it.
                    self.note = Some((
                        "ghosts play in the replay — press V to watch the run's best agent".into(),
                        6.0,
                    ));
                }
            }
            Command::Scrub(delta) => self.scrub_by(delta),
            Command::Speed(step) => {
                // Two clocks, one ladder: the replay's speed is its own, so pausing or
                // slowing it never touches the rate the run is watched at.
                if let Some(replay) = &mut self.replay {
                    replay.speed = lab::step_speed(replay.speed, step > 0);
                } else {
                    self.view.speed = lab::step_speed(self.view.speed, step > 0);
                }
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

    // -----------------------------------------------------------------------
    // the level shelf
    // -----------------------------------------------------------------------

    /// Read the pack and open the shelf on it.
    ///
    /// Scanned here rather than per frame: the shelf is opened by a person, and a directory
    /// read plus a parse of every level in it is a tenth of a second at worst — once, when it
    /// is asked for, rather than sixty times a second.
    fn open_shelf(&mut self) {
        self.shelf = shelf::scan(&self.levels_dir, Some(&self.options.champions));
        self.shelf_open = true;
    }

    /// The shelf window: the pack as pictures, and what a click on one means.
    fn shelf_window(&mut self, ctx: &Context) {
        if !self.shelf_open {
            return;
        }
        let mut open = true;
        let mut chosen: Option<(PathBuf, bool)> = None;
        egui::Window::new("Levels")
            .open(&mut open)
            .default_width(220.0)
            .max_height(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} — {}",
                            self.levels_dir.display(),
                            if self.shelf.is_empty() {
                                "nothing here".to_string()
                            } else {
                                format!("{} levels", self.shelf.len())
                            }
                        ))
                        .small()
                        .color(theme::TEXT_DIM),
                    );
                    if ui.button("Rescan").clicked() {
                        self.shelf = shelf::scan(&self.levels_dir, Some(&self.options.champions));
                    }
                });
                ui.separator();
                match shelf::show(ui, &self.shelf, self.editor.path.as_deref()) {
                    shelf::Action::Open(path) => chosen = Some((path, false)),
                    shelf::Action::Play(path) => chosen = Some((path, true)),
                    shelf::Action::None => {}
                }
            });
        self.shelf_open = open;
        if let Some((path, play)) = chosen {
            self.open_level(&path);
            if play {
                self.enter_play();
            }
        }
    }

    /// Open a level from the shelf, and put the camera on it.
    ///
    /// The editor, the playtest and the shelf all go through this, so "open a level" means
    /// one thing: a document, a camera fitted to it, and no session left over from whatever
    /// was on screen before.
    fn open_level(&mut self, path: &std::path::Path) {
        self.replace(Replace::Open(path.to_path_buf()));
    }

    /// Ask to replace the document with `want`, and do it unless the old one has unsaved edits.
    ///
    /// One door for **New** and every card on the shelf, because they are the same act: the
    /// document on the canvas is thrown away and another one takes its place. Two copies of
    /// that rule is how one of them ends up forgetting to drop a playtest session — which is
    /// exactly what **New** used to do.
    fn replace(&mut self, want: Replace) {
        if self.refuses_to_discard(want.clone()) {
            return;
        }
        self.apply_replace(want);
    }

    /// Do the replacement: install the document *and* take every view of the old one down.
    ///
    /// **A document is not the only copy of a level in this window.** A playtest session, a
    /// replayed episode and a running population each hold their own copy of the terrain they
    /// started on, and while one of those views is up it is *that* copy the canvas draws.
    /// Swapping only `editor` therefore left **New** — and a card on the shelf — looking like it
    /// had done nothing at all whenever the window was in Play or Evolve: the old level stayed on
    /// screen with its clock still running, and the new document was only visible in the dock's
    /// title. (Measured through real clicks: `mode=Play session=true drawn_is_old=true`.)
    ///
    /// So the whole window moves: the run is dropped (joining its worker, [`lab::Lab`]'s own
    /// `Drop`), the playtest and the replay go with it, the mode returns to the editor, and the
    /// camera is fitted to the level that is actually on the canvas now.
    fn apply_replace(&mut self, want: Replace) {
        let editor = match want {
            Replace::New => {
                // A new document starts with a name the pack does not already hold, because the
                // name *is* the file it will be saved as (AUDIT.md B2).
                let name = free_level_name(&self.levels_dir, "untitled");
                Editor::fresh(&name, 60, 15)
            }
            Replace::Open(path) => match Editor::open(&path) {
                Ok(editor) => editor,
                Err(err) => {
                    self.editor.message = err;
                    return;
                }
            },
        };
        self.editor = editor;
        self.pending_size = (self.editor.level.width, self.editor.level.height);
        self.fit_pending = true;
        // Every view that was about the old document, dropped rather than left drawing it.
        self.session = None;
        self.replay = None;
        // The run is *stopped*, and on a thread of its own: dropping a `Lab` joins its worker,
        // which can be the better part of a generation away — a window that froze for a second
        // after New would read as the button having hung, which is the same complaint in another
        // shape. The worker is told to quit before it is dropped, so nothing is left training a
        // level nobody is looking at.
        if let Some(lab) = self.lab.take() {
            std::thread::spawn(move || drop(lab));
        }
        self.lanes = None;
        self.packet = None;
        self.summary = None;
        self.saved_end = false;
        self.primed = false;
        self.note = None;
        self.applied_rate = None;
        self.view.scrub = None;
        self.mode = Mode::Edit;
        self.discard_armed = None;
        self.discard_wanted = None;
        // The rename field is a buffer over the document, so a document that is gone takes the
        // half-typed name with it: reseeded from the new level on the next frame.
        self.rename_focused = false;
        self.rename_buf.clear();
    }

    /// Write the document, naming an unnamed one first.
    fn save(&mut self, path: Option<PathBuf>) {
        // An unsaved document saves by its own name, into the levels directory — and takes a
        // free variant of it if the pack already holds that name (see [`Self::name_the_document`]).
        let target = path
            .or_else(|| self.editor.path.clone())
            .unwrap_or_else(|| self.name_the_document());
        if let Err(err) = self.editor.save(Some(target)) {
            self.editor.message = format!("not saved: {err}");
            return;
        }
        // The write answered any question the document was holding: there is nothing left to
        // lose, so a notice that was waiting on a replacement goes away with the edit.
        self.discard_wanted = None;
        self.discard_armed = None;
        // A save can *create* a level, so the shelf's copy of the pack is now stale. It is
        // refreshed where the write happened rather than when the shelf is opened, so the
        // card for the level just written is there the first time it is looked for.
        if self.shelf_open {
            self.shelf = shelf::scan(&self.levels_dir, Some(&self.options.champions));
        }
    }

    /// The question a dirty document asks before a replacement throws it away.
    ///
    /// It is a card under the dock rather than a modal dialog, and it is *the same two presses*
    /// the rule has always been: pressing **New** (or the card) again is still the answer, and
    /// the notice's own button is that press with a name on it. What changed is that the question
    /// is now somewhere a person is looking. It used to be a sentence in the status line only,
    /// which made a refused **New** and a shelf card that would not open look like controls that
    /// did nothing at all: the report that started this was "the levels buttons show the levels
    /// but do not let me load them", and the window had in fact asked, quietly, at the bottom.
    ///
    /// Nothing here can answer itself: with no press, the card stays until **Keep** is chosen or
    /// the document is saved.
    fn discard_notice(&mut self, ctx: &Context) {
        let Some(want) = self.discard_wanted.clone() else {
            return;
        };
        let sentence = format!(
            "{} would throw away this level's unsaved edits.",
            want.label()
        );
        let mut discard = false;
        let mut keep = false;
        egui::Area::new(egui::Id::new("discard-notice"))
            .order(egui::Order::Foreground)
            // Above the status bar, rather than over the middle of the window: the card has
            // to be answerable *while the shelf is open* — pressing the same card again is
            // the other answer — so it must not sit on top of the cards it is asking about.
            .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -46.0))
            // A question that is being asked *now* arrives now: a card that fades in over a
            // quarter of a second is a card a person has already clicked past.
            .fade_in(false)
            .show(ctx, |ui| {
                theme::card_frame().show(ui, |ui| {
                    ui.set_max_width(430.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("This level has unsaved edits")
                                .strong()
                                .color(theme::WARN_INK),
                        );
                        ui.label(
                            egui::RichText::new(&sentence)
                                .small()
                                .color(theme::TEXT_DIM),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let d = ui::icon_button(
                                ui,
                                Some(&self.atlas),
                                None,
                                "Discard",
                                ui::TOOL_SIZE,
                                false,
                                true,
                            );
                            if d.on_hover_text("throw the edits away and continue")
                                .clicked()
                            {
                                discard = true;
                            }
                            if ui::icon_button(
                                ui,
                                Some(&self.atlas),
                                None,
                                "Keep editing",
                                ui::TOOL_SIZE,
                                false,
                                true,
                            )
                            .on_hover_text("back to the level — Ctrl+S writes it")
                            .clicked()
                            {
                                keep = true;
                            }
                        });
                    });
                });
            });
        if discard {
            // The press that was already answered once: clear the arming so the guard does not
            // ask about the same document a second time, then do what was asked.
            self.discard_armed = None;
            self.discard_wanted = None;
            self.apply_replace(want);
        } else if keep {
            self.discard_wanted = None;
            self.discard_armed = None;
            self.editor.message.clear();
        }
    }

    /// The path an unnamed document saves to, naming the document after it.
    ///
    /// The document's own name is preferred — it is what the person typed, and a save should
    /// not quietly rename what something is called — and only a name the pack has already used
    /// is replaced by the first free variant of it. **The level is renamed, not just the
    /// file:** a document called `untitled` living in `untitled-3.json` would be a lie the
    /// shelf, the dock and the window title all tell back, so the two move together.
    ///
    /// No undo step is pushed. This is not an edit; it is the write deciding what to call
    /// itself, and it happens *before* the save clears `dirty` — so a person who renames the
    /// document afterwards still has their own rename on the top of the history, not this one.
    fn name_the_document(&mut self) -> PathBuf {
        let name = free_level_name(&self.levels_dir, &self.editor.level.name);
        self.editor.level.name = name.clone();
        self.levels_dir.join(format!("{}.json", lab::slug(&name)))
    }

    /// Whether the document about to be replaced is edited, in which case this press was the
    /// *warning* and the caller must not replace it.
    ///
    /// Two controls throw the document away — the dock's **New** and any click on a level card
    /// — and both did it silently: forty dragged tiles died to a mis-click, and the only sign
    /// that anything had been pending was a dot in a plaque nobody was looking at (AUDIT.md
    /// B1). The guard is a second press rather than a modal dialog, because a question nobody
    /// reads is worse than a question the *same gesture* answers, and the status line is
    /// already the editor's voice. `what` names the control, because "are you sure" is only a
    /// question if it says what about.
    ///
    /// The arming is keyed to the document's **content hash**, not to a `bool`: an edit made
    /// between the two presses re-arms the warning instead of being discarded by it, which is
    /// exactly what a person who *is* paying attention produces — look at the level, touch it,
    /// click the card again.
    fn refuses_to_discard(&mut self, want: Replace) -> bool {
        let (refused, armed) = discard_guard(
            self.editor.dirty,
            self.discard_armed,
            self.editor.level.content_hash(),
        );
        self.discard_armed = armed;
        if refused {
            self.editor.message = format!(
                "{} would throw away the unsaved edits to this level — press again to \
                 discard them, or Ctrl+S to save first",
                want.label()
            );
            // The status line is a whisper at the bottom of the window, and a card that does
            // nothing where the pointer is standing reads as a broken card rather than as a
            // question. The same refusal is therefore also a card under the dock, with both
            // answers on it (see [`Self::discard_notice`]).
            self.discard_wanted = Some(want);
        } else {
            self.discard_wanted = None;
        }
        refused
    }

    /// The title bar this window should be wearing right now.
    fn window_title(&self) -> String {
        window_title_for(self.mode, &self.editor.title())
    }

    /// Keep the OS window title on the document.
    ///
    /// The title bar was the constant `"one two hundred"` for the life of the window, so
    /// the two windows a person most often has open — a level and the level it was copied
    /// from — were indistinguishable in a taskbar, a window list or a bundle of screenshots
    /// (AUDIT.md B5). Sent only when the string changes: a viewport command per frame is a
    /// window message per frame for a title that has not moved.
    fn sync_window_title(&mut self, ctx: &Context) {
        let title = self.window_title();
        if title != self.os_title {
            self.os_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    /// The dock: the game's own tool bar, carried across the top of the window.
    ///
    /// The window used to open on a row of egui's stock buttons lying flat on a stock panel, which
    /// is what a *tool* looks like rather than what a game looks like. This is the furniture
    /// instead: the level's name chalked on a sign, the six things a person does to the document as
    /// pictures in a row, the switch that changes what the window *is* — Edit, Play, Evolve, Replay
    /// — and, at the right, the reading of whatever is running.
    ///
    /// Three rules live here, and they are the ones a call site gets wrong.
    ///
    /// * **What the window is doing is one object, not four buttons.** The four modes are
    ///   exclusive, so they are drawn inside one socket with the chosen cell standing out of the
    ///   group ([`ui::segmented`]); four independent plates would say four things that could all be
    ///   on at once.
    /// * **The controls are the game's own art.** Each button carries a sprite out of the shipped
    ///   atlas — the signpost for the shelf, the crate for a save, the runner for Play — so the tool
    ///   bar is made of the same sixteen-pixel pictures as the level behind it.
    /// * **The right-hand end is dropped, not clipped, when the window is narrow.** A dock that
    ///   silently eats its own reading is worse than one that says less, so the file path goes
    ///   first and the run's banner second, and the controls that *do* something never do.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        theme::dock_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                let (title, subtitle) = self.dock_title();
                let _ = ui::plaque(ui, &title, &subtitle).on_hover_text(format!(
                    "{}\n● marks a canvas the disk has not caught up with — a save writes this \
                     file; the details window renames the level",
                    self.editor.title()
                ));
                ui.add_space(2.0);
                self.dock_tools(ui);
                ui.add_space(2.0);
                self.dock_modes(ui);
                ui.add_space(2.0);
                self.dock_sound(ui);
                ui.add_space(2.0);
                self.dock_readout(ui);
            });
        });
    }

    /// What the sign at the left of the dock says: the document, and where it lives.
    ///
    /// The level's own name leads, because that is the thing being worked on; the file is the
    /// second line, where it reads as the *where* rather than as the what. The dot on that line is
    /// the one fact the window cannot recover for a person who misses it: the disk has fallen behind
    /// the canvas, and a closed window loses the difference.
    ///
    /// **The two states are the same width, on purpose.** The line used to read `● file — not on
    /// disk` when edited and `file` otherwise, and a plaque is as wide as the words on it — so the
    /// first stroke of a session widened the sign by a hundred points and shoved every tool in the
    /// dock sideways, and the save that followed shoved them back. A mark is the fact either way:
    /// a filled dot when the canvas is ahead of the disk, the same two columns of blank when it is
    /// not, and the sentence on the plaque's hover.
    fn dock_title(&self) -> (String, String) {
        let file = self
            .editor
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.json", self.editor.level.name));
        let subtitle = if self.editor.dirty {
            format!("● {file}")
        } else {
            format!("  {file}")
        };
        (self.editor.level.name.clone(), subtitle)
    }

    /// The seven tools down the left of the dock, in the order a person reaches for them.
    ///
    /// Every response is collected before any of them is acted on. Acting is what mutates the
    /// editor, and the buttons have to be laid out from one frame's state: a *Save* that moved the
    /// levels directory out from under *Levels…* mid-row would be a bar that disagreed with itself
    /// for a frame, which is exactly the kind of flicker a prototype has and a game does not.
    ///
    /// **Each is a picture and its own name.** Two of the six show the thing they act on out of the
    /// game's own art — the signpost for the pack of levels, the crate a save writes — and the four
    /// the world has no picture of carry a glyph drawn for this row alone (`tools/art/ui.py`): a turn
    /// arrow each way for the stroke history, a bracket frame for *Fit*'s cousin *Details*, a
    /// magnifier for the level's own numbers. Every one of them carries its name under the picture:
    /// the art is what makes the bar part of the game, and the word is what makes it readable to
    /// somebody who has not memorised six pictures yet — see [`ui::icon_button`]. What is on the
    /// hovers is the sentence a word cannot hold.
    fn dock_tools(&mut self, ui: &mut egui::Ui) {
        #[cfg(test)]
        self.tool_rects.clear();
        let levels = ui::icon_button(
            ui,
            Some(&self.atlas),
            Some("signpost"),
            "Levels",
            ui::TOOL_SIZE,
            self.shelf_open,
            true,
        );
        self.remember_tool("Levels", levels.rect);
        let levels = levels
            .on_hover_text("the pack, as pictures: click to open, double-click to playtest")
            .clicked();
        let new = ui::icon_button(
            ui,
            Some(&self.atlas),
            Some("spawn_marker"),
            "New",
            ui::TOOL_SIZE,
            false,
            true,
        );
        self.remember_tool("New", new.rect);
        let new = new.on_hover_text("a fresh 60 × 15 level").clicked();
        let save = ui::icon_button(
            ui,
            Some(&self.atlas),
            Some("crate"),
            "Save",
            ui::TOOL_SIZE,
            // Gold while the canvas is ahead of the disk. It is the one control in the dock that
            // has something to say about the *document* rather than about the run, and a save
            // waiting to happen is the thing this bar exists to not let anyone forget.
            self.editor.dirty,
            true,
        )
        .on_hover_text("write the level to disk — Ctrl+S")
        .clicked();
        let undo = ui::icon_button(
            ui,
            Some(&self.atlas),
            Some(ui::GLYPH_UNDO),
            "Undo",
            ui::TOOL_SIZE,
            false,
            self.editor.can_undo(),
        )
        .on_hover_text("step back through the stroke history — Ctrl+Z")
        .clicked();
        let redo = ui::icon_button(
            ui,
            Some(&self.atlas),
            Some(ui::GLYPH_REDO),
            "Redo",
            ui::TOOL_SIZE,
            false,
            self.editor.can_redo(),
        )
        .on_hover_text("step forward again — Ctrl+Y")
        .clicked();
        // The one control here that opens rather than acts: the level's own numbers, which are
        // looked at once and then wanted out of the way (see [`Self::details_window`]).
        let details = ui::icon_button(
            ui,
            Some(&self.atlas),
            Some(ui::GLYPH_INFO),
            "Details",
            ui::TOOL_SIZE,
            self.details_open,
            true,
        )
        .on_hover_text(
            "what this level is made of: its size, its materials, and the physics it may ask for",
        )
        .clicked();
        // The one tool in the row that acts on the *document* rather than on a tile: mirroring.
        // It sits after the history because that is where its undo step lands — flip, look, and
        // Ctrl+Z puts it back — and it is a tool rather than a palette cell because it is not
        // something you *paint* with: `sim` has always known how to flip a level and the editor
        // had no way to ask (AUDIT.md F3).
        let mirror = ui::icon_button(
            ui,
            Some(&self.atlas),
            Some(ui::GLYPH_MIRROR),
            "Mirror",
            ui::TOOL_SIZE,
            false,
            true,
        )
        .on_hover_text("flip the whole level left to right — one undo step, and the name stays")
        .clicked();

        if levels {
            self.open_shelf();
        }
        if new {
            // The same door the shelf's cards use, so New and "open that level" cannot drift
            // apart about what replacing a document means (`EvoApp::apply_replace`).
            self.replace(Replace::New);
        }
        if save {
            self.save(None);
        }
        if mirror && self.editor.mirror_horizontal() {
            // A mirror can move the spawn and the flag, so the pending grid size is re-read and
            // any session on the old terrain is dropped: the playtest is a run of *this* level,
            // and a level that has just been flipped is a different one.
            self.pending_size = (self.editor.level.width, self.editor.level.height);
            self.session = None;
        }
        if undo && self.editor.undo() {
            self.editor.message = "undo".into();
        }
        if redo && self.editor.redo() {
            self.editor.message = "redo".into();
        }
        if details {
            self.details_open = !self.details_open;
        }
    }

    /// Remember where a dock tool was drawn, for the tests that press it.
    #[cfg(test)]
    fn remember_tool(&mut self, name: &'static str, rect: Rect) {
        self.tool_rects.push((name, rect));
    }

    /// Nothing, in the window: a real press reaches the control's own response.
    #[cfg(not(test))]
    fn remember_tool(&mut self, _name: &'static str, _rect: Rect) {}

    /// The mode switch, and the two controls that belong to the mode it selects.
    ///
    /// *Fit* sits here rather than in the document tools because it is about the *view* of the level
    /// rather than about the level, and *Restart* appears only while a person is playing: a control
    /// that cannot do what it says is worse than a control that is not there, and this is the bar
    /// that teaches the window.
    ///
    /// The switch names all four of its cells rather than only the chosen one: a mode a person has
    /// not entered yet is exactly the mode they cannot identify from a picture of a walker, and the
    /// gold plate already says which one is on. See [`ui::segmented`].
    fn dock_modes(&mut self, ui: &mut egui::Ui) {
        const OPTIONS: [(Option<&str>, &str); 4] = [
            (Some("ground_1"), "Edit"),
            (Some("player_run_1"), "Play"),
            (Some("walker_a"), "Evolve"),
            // The run's answer is the flag: `goal_flag_2` is the waving banner rather than the
            // pole, and at eighteen points the banner is the half of it that reads.
            (Some("goal_flag_2"), "Replay"),
        ];
        // Replay is offered once there is something to replay: a mode that opens on an empty field
        // would be a cell that cannot do what it says.
        let enabled = [true, true, true, self.can_replay()];
        let (chosen, _) =
            ui::segmented(ui, Some(&self.atlas), &OPTIONS, self.mode.index(), &enabled);
        if self.mode == Mode::Play {
            let restart = ui::icon_button(
                ui,
                Some(&self.atlas),
                Some(ui::GLYPH_RESTART),
                "Restart",
                ui::TOOL_SIZE,
                false,
                true,
            )
            .on_hover_text("put the agent back at the spawn — R")
            .clicked();
            if restart {
                if let Some(session) = &mut self.session {
                    session.restart(&self.editor.level);
                }
            }
            // The record-holder's ghost, when there is one to race: the same walker icon
            // the replay's Ghosts row wears, because it is the same idea — another
            // agent's episode, drawn translucent over your own.
            let (has_ghost, ghost_shown) = self
                .session
                .as_ref()
                .map(|s| (s.has_ghost(), s.ghost_sim().is_some()))
                .unwrap_or((false, false));
            let ghost = ui::icon_button(
                ui,
                Some(&self.atlas),
                Some("walker_a"),
                "Ghost",
                ui::TOOL_SIZE,
                ghost_shown,
                has_ghost,
            )
            .on_hover_text(if has_ghost {
                "show the run's quickest win racing beside you — G"
            } else {
                "no time to race yet — press E to train on this level"
            })
            .clicked();
            if ghost {
                if let Some(session) = &mut self.session {
                    session.toggle_ghost();
                }
            }
        }
        let fit = ui::icon_button(
            ui,
            Some(&self.atlas),
            Some(ui::GLYPH_FIT),
            "Fit",
            ui::TOOL_SIZE,
            false,
            true,
        )
        .on_hover_text("frame the whole level — F")
        .clicked();
        if fit {
            self.fit_pending = true;
        }
        if let Some(cell) = chosen {
            self.set_mode(Mode::ALL[cell]);
        }
    }

    /// The sound switch: on every screen, because a mute you can only reach
    /// from one view is a mute someone cannot find from the view making the
    /// noise. The plate is gold while sound is on and plain once it is off, so
    /// the state is the face rather than a word to decode.
    ///
    /// `M` stays the shortcut in Edit, Play and Replay; Evolve already gives
    /// that key to the mutation rate, which is why the plate exists at all.
    fn dock_sound(&mut self, ui: &mut egui::Ui) {
        let muted = crate::sound::muted();
        let label = if muted { "Muted" } else { "Sound" };
        let response = ui::icon_button(
            ui,
            Some(&self.atlas),
            None,
            label,
            ui::TOOL_SIZE,
            !muted,
            true,
        )
        .on_hover_text(
            "sound on or off — this plate is on every screen; M toggles it outside Evolve",
        );
        if response.clicked() {
            crate::sound::toggle_mute();
        }
    }

    /// The right-hand end: the one line about what is running.
    ///
    /// The file this document would be saved to used to live out here as well, and it is gone — the
    /// plaque at the other end already names it, twice, and a dock that says the same thing at both
    /// ends is spending the middle of the bar on a caption. What is left is the reading, which is the
    /// only thing on this bar that changes on its own.
    ///
    /// It is drawn only where there is room for it, and the room is *measured* rather than guessed:
    /// the width of everything to its left depends on the level's name and on which mode is on, and
    /// a banner cut in half by a clip rectangle reads as a bug rather than as a smaller window.
    fn dock_readout(&mut self, ui: &mut egui::Ui) {
        let Some((text, lamp)) = self.dock_banner() else {
            return;
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let _ = ui::banner(ui, &text, lamp, ui.available_width());
        });
    }

    /// The one line the dock carries about what is happening, if anything is.
    ///
    /// It is the run's own numbers, not a label: the mode is already said twice, by the gold cell
    /// and by the status bar, so what is worth the space here is the thing that changes — which
    /// generation is on screen, how good it is, and how many of the population touched the flag.
    /// The lamp is that reading's own colour: green once anything has won, amber once the search has
    /// stopped moving, the accent while it is still improving.
    ///
    /// **A run has no line here.** It used to, and it was the same three numbers the panel beside
    /// the canvas was already showing — a generation, a best fitness, and a win count — read out a
    /// second time two hundred points away from themselves. What is left is the two modes whose
    /// panel does *not* carry their number: a playthrough's score, and the replay's line, whose seed
    /// belongs to the run behind the episode rather than to the episode a viewer is watching.
    fn dock_banner(&self) -> Option<(String, Color32)> {
        match self.mode {
            Mode::Edit => None,
            // The one thing the dock has to say about a run: a run that died quietly. The
            // worker's panic used to be thrown away at the join, `is_finished` went true on its
            // own, and the window drew a dead run as a finished one — leaving the viewer to
            // wonder why the curve had stopped (AUDIT.md B4).
            Mode::Evolve => self.lab.as_ref().filter(|lab| lab.panicked()).map(|_| {
                (
                    "training stopped: a worker thread died — Restart to run it again".to_string(),
                    theme::BAD,
                )
            }),
            // The race, not the readout: the strip along the bottom already prints this episode's
            // frame count, its position and its progress, and the one thing the dock can add is the
            // comparison a person is playing for — their own clock against the AI's best time, which
            // is the number the Evolve panel showed them before they came here.
            Mode::Play => self.session.as_ref().map(|session| {
                let record = self.packet.as_ref().and_then(|packet| packet.fastest);
                // The lamp follows the race rather than the episode: gold while the clock is at or
                // under the AI's best time (which is `Session::beating`, the same rule while the
                // agent runs and once it has touched the flag), the runner's own colour otherwise.
                let lamp = match record {
                    Some(fast) if session.beating(fast) => theme::GOOD,
                    _ => theme::RUN,
                };
                (session.race_line(record), lamp)
            }),
            Mode::Replay => self
                .replay
                .as_ref()
                .map(|replay| {
                    let star = replay.star();
                    (
                        format!(
                            "replay  {} ({} frames)  best {:.1}  seed {}",
                            sim::format_time(star.frames),
                            star.frames,
                            star.fitness,
                            self.seed
                        ),
                        theme::GOOD,
                    )
                })
                .or_else(|| {
                    self.packet.as_ref().map(|packet| {
                        (
                            format!("replay  best {:.1}  seed {}", packet.max_fitness, self.seed),
                            theme::GOOD,
                        )
                    })
                }),
        }
    }

    /// Change what the window is doing, with the bookkeeping each transition owes.
    ///
    /// The switch says *which* mode was chosen and this decides what that means: leaving the replay
    /// drops the pool of episodes it built (a second simulation parked behind the first is not
    /// something either view wants), leaving Play drops the session, and coming back to the editor
    /// from Evolve keeps the run paused and parked rather than starting it over.
    fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        match mode {
            Mode::Play => self.enter_play(),
            Mode::Evolve => {
                if self.mode == Mode::Replay {
                    // Back to the run as it was left: the replay trained nothing, so there is
                    // nothing here to restart.
                    self.leave_replay(Mode::Evolve);
                } else {
                    self.enter_evolve();
                }
            }
            Mode::Replay => self.enter_replay(),
            Mode::Edit => {
                if self.mode == Mode::Play {
                    self.leave_play();
                } else if self.mode == Mode::Replay {
                    self.leave_replay(Mode::Edit);
                } else {
                    // The run keeps going — paused and parked — so coming back to it does not
                    // mean starting from generation zero again.
                    self.mode = Mode::Edit;
                }
            }
        }
    }

    /// The picture a palette entry draws, as `(atlas name, mirrored)`.
    ///
    /// Two brushes do not have one fixed picture, and both are the same kind of mistake if
    /// they are drawn from [`Brush::sprite`] anyway:
    ///
    /// * **Scenery** places whichever prop the picker under the palette has selected, so its
    ///   button and its ghost have to track that selection, or the palette shows a bush while
    ///   the brush plants a tree.
    /// * **Turret** has a *direction*, and the direction is the placement: a turret that fires
    ///   left is drawn firing left everywhere else in the window, so a palette that showed the
    ///   right-facing picture for both would be promising a turret the level will not get.
    ///
    /// The turret is answered by [`crate::view::turret_picture`] — the *same* function the canvas
    /// uses — rather than by a name looked up here, because the two ways of getting this picture
    /// (a second atlas cell here, a mirror there) are exactly how the window can end up showing
    /// two different turrets for one setting. The `flip` half of the answer is why this returns a
    /// pair: the palette and the ghost paint through [`crate::view::facing`] with it.
    fn brush_icon(&self, brush: Brush) -> (&'static str, bool) {
        match brush {
            Brush::Scenery => (self.editor.scene_sprite, false),
            Brush::Turret => crate::view::turret_picture(self.editor.turret_dir),
            _ => (brush.sprite(), false),
        }
    }

    /// The sprite drawn *under* a brush's icon, when it has one.
    ///
    /// Only a cracked brush does, and the two halves of that are asked separately and on purpose:
    /// [`Brush::is_cracked`] is whether the brush paints something that needs two layers at all,
    /// and [`Brush::base_sprite`] is which picture goes underneath. A palette cell drawn from the
    /// second alone would show a *sound* material's face on a brush that paints a brittle one the
    /// day somebody adds a fifth brittle material without an icon, which is the failure this
    /// window has already had once (`Brush::sprite`'s own note: a missing name is a blank button
    /// and no error anywhere).
    fn brush_base(brush: Brush) -> Option<&'static str> {
        if brush.is_cracked() {
            brush.base_sprite()
        } else {
            None
        }
    }

    /// The palette: what to paint with, in a wrapped grid of tiles, and the shape the stroke is.
    ///
    /// A wrapped *grid* rather than one brush per row (which is what this was): sixteen
    /// brushes laid out one to a line is a column the height of the panel that says nothing
    /// about which of them go together. Wrapped, one material and its brittle self sit side by
    /// side, the hazard and the pad follow them, and the entities come after — and the panel
    /// scrolls for the frames where the window is too short to show all of it, which is the
    /// bargain a grid of this many cells makes rather than a claim that it always fits.
    ///
    /// **The panel has one line of type in it, and it names the brush that is on.** The title used
    /// to be a bare *Parts*, with the selected brush named under the grid and a sentence about
    /// dragging between them: three separate lines saying what the gold cell and its hover already
    /// said, in the one panel a person looks at the level beside. What is left is a heading that
    /// carries the answer to "what am I holding" for the two seconds a person wonders it.
    ///
    /// Every cell is a [`ui::icon_button`]'s plate, drawn through the same [`ui::bevel`] as the
    /// dock's tools: a resting cell is a card, the one under the pointer lifts, the one being
    /// pressed sinks into its socket, and the selected brush is the palette's gold with its picture
    /// still legible on it. That is the whole point of having the vocabulary in one place — the
    /// thing a person paints with and the thing a person opens a level with answer the pointer the
    /// same way, so the window reads as one object rather than as a panel beside a tool bar.
    fn palette(&mut self, ui: &mut egui::Ui) {
        // The one line of type in the panel, and it answers the one question a person asks of a
        // palette: which brush is in hand. The instruction the window needs — that a brush is
        // dragged onto the level — is on every cell's own hover, where a person asking "what is
        // this cell" is already looking, and that is the difference between a game that teaches and
        // a tutorial that never ends.
        let heading = match self.editor.brush {
            Some(brush) => format!("PARTS · {}", brush.label().to_uppercase()),
            None => "PARTS".to_string(),
        };
        panel::section(ui, &heading);
        // Forty-eight rather than forty: a cell drawn as a plate spends [`theme::PRESS_SINK`] and
        // its lip on the picture, and a sixteen-pixel tile at twenty-six points is the smallest an eye
        // can still tell a plank from a brick at.
        let cell = 48.0;
        ui.horizontal_wrapped(|ui| {
            for brush in Brush::ALL {
                let selected = self.editor.brush == Some(brush);
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(cell, cell), Sense::click_and_drag());
                let state = ui::PlateState::of(&response, selected, true);
                let plate = ui::bevel(state, rect);
                let painter = ui.painter_at(rect);
                ui::paint(&painter, &plate);
                // Two layers, when a brush has two. A cracked brush's own icon is a *transparent
                // crack* — what it paints is a material plus a crack — so its cell draws the
                // material the crack breaks underneath it, in the same socket and at the same
                // size. That is the pair the level draws for a brittle tile, and it is what makes
                // a cracked cell read as "the floor above this one, about to go" rather than as a
                // scratch on the palette. Every other brush has no base layer and is unchanged.
                let (icon, flip) = self.brush_icon(brush);
                for (name, flip) in [
                    Self::brush_base(brush).map(|n| (n, false)),
                    Some((icon, flip)),
                ]
                .into_iter()
                .flatten()
                {
                    if let Ok(uv) = self.atlas.uv(name) {
                        painter.image(
                            self.atlas.texture_id(),
                            plate.content.shrink(2.0),
                            crate::view::facing(uv, flip),
                            Color32::WHITE,
                        );
                    }
                }
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
            }
        });
        // The shape of the stroke: one tile at a time, or a box, or a line. A switch rather than
        // a menu, because it is changed as often as the brush itself — and the same switch the
        // dock uses for the modes, so the window has one way of saying "one of these three".
        // The switch, under the grid it modifies: the heading already named the brush, so this is
        // the whole of what is left of the panel's words — three four-letter ones, on the control
        // that spends them.
        ui.add_space(12.0);
        let shapes: [(Option<&str>, &str); 3] = [
            (None, Shape::Free.label()),
            (None, Shape::Rect.label()),
            (None, Shape::Line.label()),
        ];
        let chosen = Shape::ALL
            .iter()
            .position(|shape| *shape == self.editor.shape)
            .unwrap_or(0);
        let (picked, row) =
            ui::segmented_sized(ui, None, &shapes, chosen, &[true; 3], Vec2::new(52.0, 30.0));
        // The row's own sentence, on the row rather than repeated under it: what the three cells
        // are for is the question a person has once, and the answer belongs where the question is.
        row.on_hover_text(format!(
            "the shape of the stroke\n{}\n{}\n{}",
            Shape::Free.about(),
            Shape::Rect.about(),
            Shape::Line.about()
        ));
        if let Some(cell) = picked {
            let shape = Shape::ALL[cell];
            self.editor.shape = shape;
            self.editor.message = format!("{} stroke", shape.label());
        }

        // The three brushes that carry a number, and the numbers, shown only while one of them
        // is in hand: a flyer's beat and a platform's travel are the same knob (how far it
        // goes), a platform also has a width, and a turret has a direction instead. The row is
        // conditional for the scenery picker's own reason — a control that belongs to one brush
        // is noise in front of the other twenty.
        if matches!(
            self.editor.brush,
            Some(Brush::Flyer | Brush::Turret | Brush::Platform)
        ) {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if self.editor.brush == Some(Brush::Turret) {
                    let right = self.editor.turret_dir > 0;
                    let options: [(Option<&str>, &str); 2] =
                        [(None, "fire left"), (None, "fire right")];
                    let (picked, row) = ui::segmented_sized(
                        ui,
                        None,
                        &options,
                        usize::from(right),
                        &[true; 2],
                        Vec2::new(74.0, 28.0),
                    );
                    row.on_hover_text("Which way the turret's row is dangerous.");
                    if let Some(cell) = picked {
                        self.editor.turret_dir = if cell == 0 { -1 } else { 1 };
                    }
                } else if self.editor.brush == Some(Brush::Flyer) {
                    ui.label(egui::RichText::new("beat").small().color(theme::TEXT_DIM));
                    ui.add(egui::DragValue::new(&mut self.editor.enemy_span).range(1..=24))
                        .on_hover_text(
                            "How many tiles the flyer beats across, centred on the click. \
                             The default is eight, and the whole point of a flyer is the span: \
                             two tiles is a nuisance in one doorway, eight is a thing to time.",
                        );
                } else {
                    ui.label(egui::RichText::new("travel").small().color(theme::TEXT_DIM));
                    ui.add(egui::DragValue::new(&mut self.editor.platform_span).range(1..=12))
                        .on_hover_text(
                            "How far the platform travels, from where it is placed, in the \
                             direction chosen below.",
                        );
                }
                if self.editor.brush == Some(Brush::Platform) {
                    ui.label(egui::RichText::new("width").small().color(theme::TEXT_DIM));
                    ui.add(egui::DragValue::new(&mut self.editor.platform_tiles).range(1..=8))
                        .on_hover_text("How many tiles wide the shelf is.");
                    let vertical = self.editor.platform_vertical;
                    let options: [(Option<&str>, &str); 2] = [(None, "slide"), (None, "lift")];
                    let (picked, row) = ui::segmented_sized(
                        ui,
                        None,
                        &options,
                        usize::from(vertical),
                        &[true; 2],
                        Vec2::new(58.0, 28.0),
                    );
                    row.on_hover_text("Slide along the level, or lift through it.");
                    if let Some(cell) = picked {
                        self.editor.platform_vertical = cell == 1;
                    }
                }
            });
        }

        // A platform's **direction**, on its own line beneath the row that holds its travel,
        // its width and its axis: the palette is one column wide and a fourth control does not
        // fit beside three. It is the same two-cell switch a turret's facing uses, with the
        // words the axis makes true — a slider goes left or right, a lift goes down or up — so
        // the one thing a person has to learn is that the switch means "which way".
        if self.editor.brush == Some(Brush::Platform) {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("goes").small().color(theme::TEXT_DIM));
                let vertical = self.editor.platform_vertical;
                let (back, forward) = if vertical {
                    ("down", "up")
                } else {
                    ("left", "right")
                };
                let options: [(Option<&str>, &str); 2] = [(None, back), (None, forward)];
                let (picked, row) = ui::segmented_sized(
                    ui,
                    None,
                    &options,
                    usize::from(self.editor.platform_dir > 0),
                    &[true; 2],
                    Vec2::new(62.0, 28.0),
                );
                row.on_hover_text(
                    "Which way the platform travels from where it is placed. A lift goes up \
                     or down; a slider goes left or right.",
                );
                if let Some(cell) = picked {
                    self.editor.platform_dir = if cell == 0 { -1 } else { 1 };
                }
            });
        }

        // The scenery picker, shown only while the scenery brush is the active one: two
        // dozen thumbnails would bury the eleven brushes that are used together, and the
        // choice only matters to the one brush that needs it.
        if self.editor.brush == Some(Brush::Scenery) {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Scenery")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.horizontal_wrapped(|ui| {
                for name in crate::editor::SCENERY_SPRITES {
                    let selected = self.editor.scene_sprite == *name;
                    // The same plate as a brush cell, a size down: picking a tree is picking a brush,
                    // and it should not be a different kind of object because it lives in a
                    // conditional row. The colours used to be four literals — a grey here, the
                    // accent's own gold written out as "250, 214, 102" there — which is how a window
                    // ends up with two golds in it.
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(34.0, 34.0), Sense::click());
                    let state = ui::PlateState::of(&response, selected, true);
                    let plate = ui::bevel(state, rect);
                    let painter = ui.painter_at(rect);
                    ui::paint(&painter, &plate);
                    if let Ok(uv) = self.atlas.uv(name) {
                        painter.image(
                            self.atlas.texture_id(),
                            plate.content.shrink(1.0),
                            uv,
                            Color32::WHITE,
                        );
                    }
                    if response.on_hover_text(*name).clicked() {
                        self.editor.scene_sprite = name;
                        self.editor.message = format!("place {name}");
                    }
                }
            });
        }
    }

    /// Put a find on screen and slow the animation down so it can be seen.
    ///
    /// The slow-motion half is the plan's "the moment": the record was made by the generation
    /// about to animate, and a 50x pass over it is over before a person looks up. The previous
    /// speed is restored when the card expires, unless the viewer chose another one meanwhile.
    fn show_discovery(&mut self, found: evo::Discovery) {
        if self.speed_before_discovery.is_none() {
            self.speed_before_discovery = Some(self.view.speed);
        }
        self.view.speed = crate::lab::SPEEDS[0];
        // The codex page is a snapshot; a new find makes it stale. Reload it at once when it is
        // on screen, so the card and the page never disagree about what the level has produced.
        if self.codex_open {
            self.load_codex_view();
        } else {
            self.codex_view = None;
        }
        self.discovery = Some((found, DISCOVERY_SECONDS));
    }

    /// Read the document's codex from disk. The document's, not the run's: they share a stem
    /// unless the level was edited after the run started, and the page is about the level on
    /// screen. Missing or stale files are reported and shown as an empty journal rather than
    /// taken as an error — a codex is a record, not a precondition for drawing a window.
    fn load_codex_view(&mut self) {
        let stem = crate::lab::artifact_stem(&self.editor.level, self.editor.path.as_deref());
        let path = evo::Codex::path_for(&self.options.champions, &stem);
        self.codex_view = Some(
            evo::Codex::load(&self.editor.level, &path).unwrap_or_else(|err| {
                eprintln!("codex: {err}");
                evo::Codex::empty(&self.editor.level)
            }),
        );
    }

    /// Age the discovery card, and put the speed back when it expires.
    ///
    /// The speed is restored only if it is still the slow one, so a viewer who chooses a speed
    /// during the moment keeps their choice. Split out of [`Self::evolve_step`] so the expiry —
    /// the half a fast test cannot reach by animating — is a plain function of `dt`.
    fn tick_discovery(&mut self, dt: f32) {
        if let Some((_, left)) = &mut self.discovery {
            *left -= dt;
            if *left > 0.0 {
                return;
            }
            self.discovery = None;
            if let Some(speed) = self.speed_before_discovery.take() {
                if self.view.speed == crate::lab::SPEEDS[0] {
                    self.view.speed = speed;
                }
            }
        }
    }

    /// The discovery card: a name, one line and the evidence, over the run, for a few seconds.
    fn discovery_card(&mut self, ctx: &Context) {
        let Some((found, _)) = &self.discovery else {
            return;
        };
        egui::Window::new("discovery")
            .id(egui::Id::new("discovery-card"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -54.0])
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::CARD)
                    .stroke(egui::Stroke::new(1.5_f32, theme::GOLD_EDGE)),
            )
            .show(ctx, |ui| {
                ui.set_max_width(320.0);
                ui.label(
                    egui::RichText::new("A FIND")
                        .small()
                        .strong()
                        .color(theme::ACCENT_INK),
                );
                ui.label(
                    egui::RichText::new(&found.name)
                        .heading()
                        .color(theme::RUN_INK),
                );
                ui.label(egui::RichText::new(&found.line).color(theme::TEXT));
                ui.label(
                    egui::RichText::new(format!(
                        "gen {} · genome {} · {} frames · K for the codex",
                        found.generation, found.genome_id, found.frames
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
            });
    }

    /// The trick codex: what this level has produced, newest last.
    fn codex_window(&mut self, ctx: &Context) {
        if !self.codex_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Trick codex")
            .open(&mut open)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                let Some(codex) = &self.codex_view else {
                    ui.label(
                        egui::RichText::new("no codex for this level")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                    return;
                };
                ui.label(
                    egui::RichText::new(format!(
                        "{} — {} find{}",
                        codex.level_name,
                        codex.len(),
                        if codex.len() == 1 { "" } else { "s" }
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
                ui.separator();
                if codex.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "nothing found yet — a run writes here when it moves the record \
                             and does something this build can name",
                        )
                        .color(theme::TEXT_DIM),
                    );
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for entry in codex.entries() {
                            ui.label(
                                egui::RichText::new(&entry.name)
                                    .strong()
                                    .color(theme::RUN_INK),
                            );
                            ui.label(egui::RichText::new(&entry.line).color(theme::TEXT));
                            ui.label(
                                egui::RichText::new(format!(
                                    "gen {} · genome {} · {} frames · seed {}",
                                    entry.generation, entry.genome_id, entry.frames, entry.seed
                                ))
                                .small()
                                .color(theme::TEXT_DIM),
                            );
                            ui.separator();
                        }
                    });
            });
        self.codex_open = open;
    }

    /// The level's numbers, in a window of their own, closed until they are asked for.
    ///
    /// What stood here was a *column*: a verdict at the top and six labelled sections of readings
    /// beneath it, permanently on screen beside the canvas. That is a properties window, and a
    /// properties window beside a level is the single clearest thing that makes an editor look like
    /// a tool instead of like a game — half a panel of cream, most of the time, saying nothing. So
    /// the canvas has the whole width now, the one reading that has to be up *at all times* — can
    /// the spawn reach the flag — lives in the strip along the bottom ([`Self::status_line`]), and
    /// everything else waits behind the dock's magnifier.
    ///
    /// A floating window rather than a panel because of what it is *for*: it is about the document
    /// while it is open and about nothing at all while it is closed, and a panel that can be empty
    /// is a panel that is mostly empty. The dock is what says whether it exists.
    ///
    /// **One window, four answers** ([`Self::details`]). It holds what the mode on screen has that
    /// its panel does not: the level's own numbers and its verdict in the editor, and a run's — the
    /// search's step and stall, the field, the record — beside a population. That is what lets the
    /// panels be menus ([`panel::lab_panel`]) without losing a single number.
    fn details_window(&mut self, ctx: &Context) {
        if !self.details_open {
            return;
        }
        // The title bar's own ✕ and the dock's plate write the same flag: two ways out of one
        // window, because a floating window with no close of its own is a window a person leaves
        // open by accident.
        let mut open = true;
        egui::Window::new("Details")
            .open(&mut open)
            .resizable(false)
            // Wide enough for a label and its reading on one line with room between them: a board of
            // `label        value` rows needs the two columns to be separated or it reads as prose
            // again, which is the thing the rows exist to stop.
            .default_width(340.0)
            // A run's board is long — a row per agent in the field, the record, the ladders — and a
            // window that grew to fit it would put its own title bar off the top of a small display.
            // Scrolled rather than truncated, because the numbers at the bottom are as wanted as the
            // ones at the top.
            .scroll([false, true])
            .max_height((ctx.screen_rect().height() - 140.0).max(160.0))
            // Sits where the panel used to, so the muscle memory still lands — and it is dragged
            // from there like any window.
            .default_pos(Pos2::new(
                (ctx.screen_rect().right() - 320.0).max(12.0),
                76.0,
            ))
            .show(ctx, |ui| self.details(ui));
        self.details_open = open;
    }

    /// The verdict as a lamp and a word: the one reading that is always on screen.
    ///
    /// Shared by the details window and the strip along the bottom, so the two cannot come to
    /// disagree about whether this level can be beaten — the colour and the sentence are one
    /// reading, and a lamp of one colour beside a word of another is worse than no lamp at all.
    fn verdict_lamp(&self, ui: &mut egui::Ui) {
        let colour = if self.editor.beatable() {
            theme::GOOD_INK
        } else {
            theme::BAD_INK
        };
        // A light reads from across the room in a way a sentence does not, and the verdict is read
        // while looking at the *level*, not at the words.
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let centre = rect.center();
        painter.circle_filled(centre, 5.0, colour);
        painter.circle_stroke(
            centre,
            5.0,
            Stroke::new(1.0_f32, colour.gamma_multiply(0.6)),
        );
        let verdict = self.editor.verdict();
        let label = ui.colored_label(colour, verdict);
        if let Some(plan) = self.editor.plan_verdict() {
            label.on_hover_text(format!("{}\n{}", plan.label(), plan.detail()));
        }
    }

    /// What the details window holds, which depends on what the window is showing.
    ///
    /// One window rather than one per mode, because it answers the same question in each: *what else
    /// is true here that the panel beside the canvas is not saying?* In the editor that is the
    /// verdict, the physics a jump may ask for, and what the level is made of; in a run it is the
    /// search's own numbers, the field, and the record. A viewer learns one control — the dock's
    /// magnifier — and gets whichever answer belongs to the mode they are in.
    fn details(&mut self, ui: &mut egui::Ui) {
        match self.mode {
            Mode::Evolve => self.details_run(ui),
            Mode::Replay => self.details_replay(ui),
            Mode::Edit | Mode::Play => self.details_level(ui),
        }
    }

    /// The Details window while a run is on screen: the board behind the run's menu.
    ///
    /// The summary is taken and put back exactly as [`Self::evolve_panel`] does it, and for the same
    /// reason: the widget layer borrows what it draws, and the window needs it again the moment the
    /// frame is over.
    fn details_run(&mut self, ui: &mut egui::Ui) {
        self.run_settings_section(ui);
        let Some(summary) = self.summary.take() else {
            // The same sentence the panel shows before the first generation: opening the window on a
            // run that has not scored yet should say so rather than show an empty board.
            match &self.lab {
                Some(lab) => {
                    ui.label(format!(
                        "{}: evaluating the first generation…",
                        lab.level_name()
                    ));
                }
                None => {
                    ui.label("no run on this level yet");
                }
            }
            return;
        };
        let actions = panel::lab_details(ui, &summary, &mut self.view);
        self.summary = Some(summary);
        self.apply_actions(actions);
    }

    /// The run's own settings, for the person who wants a different run.
    ///
    /// Every knob here was the command line's alone: the window showed a banner reading "pop 200
    /// seed 1" and had no way to make it anything else, so trying a population of 400 meant
    /// killing the window and relaunching — which abandons the run on screen and everything it has
    /// already scored (AUDIT.md F2). Four fields, because four are the ones a person actually
    /// turns while watching a run.
    ///
    /// **The button starts a *new* run and says so.** The population size, the generation cap and
    /// the curriculum are all fixed when a run starts (`Lab::start`): there is no such thing as
    /// changing them mid-run, and a control that accepted the numbers and then ignored them would
    /// be a lie told by a widget. What is *not* here says the same thing in reverse — the curriculum
    /// level, the resume checkpoint and the champions directory are paths, and a file picker in a
    /// window with no file manager is a worse answer than the flag whose name is written beside it.
    fn run_settings_section(&mut self, ui: &mut egui::Ui) {
        panel::section(ui, "Run settings");
        ui.horizontal(|ui| {
            ui.label("population");
            ui.add(egui::DragValue::new(&mut self.options.pop_size).range(2..=4096));
        });
        ui.horizontal(|ui| {
            ui.label("seed");
            ui.add(egui::DragValue::new(&mut self.seed));
        });
        // The cap is a *choice* rather than a number, because "run until I close the window" is
        // the default and has no number that means it.
        ui.horizontal(|ui| {
            let mut capped = self.options.generations.is_some();
            if ui.checkbox(&mut capped, "stop after").changed() {
                self.options.generations = if capped {
                    Some(self.options.generations.unwrap_or(100))
                } else {
                    None
                };
            }
            if let Some(cap) = &mut self.options.generations {
                ui.add(egui::DragValue::new(cap).range(1..=100_000));
                ui.label("generations");
            } else {
                ui.label(
                    egui::RichText::new("until the window closes")
                        .small()
                        .color(theme::TEXT_DIM),
                );
            }
        });
        ui.horizontal(|ui| {
            ui.label("curriculum step");
            ui.add(
                egui::DragValue::new(&mut self.options.curriculum_generations).range(1..=10_000),
            );
            ui.label("generations");
        });
        ui.small(
            egui::RichText::new(
                "--curriculum <PATH>, --resume <PATH>, --from-champion <PATH> and --champions \
                 <DIR> are the command line's: paths are set where the window is opened",
            )
            .color(theme::TEXT_DIM),
        );
        let live = self.lab.is_some();
        let label = if live {
            "Apply — starts a new run"
        } else {
            "Start a run with these"
        };
        if ui.button(label).clicked() {
            self.apply_run_settings();
        }
        if live {
            ui.small(
                egui::RichText::new(
                    "a run's population and cap are fixed when it starts, so these make a \
                     *new* run rather than changing this one",
                )
                .color(theme::TEXT_DIM),
            );
        }
    }

    /// Start a fresh run on the settings as they now stand.
    ///
    /// One function rather than two paths, because the two cases are the same case: a run that is
    /// going is dropped and a new one built from `self.options` and `self.seed`, and a level with no
    /// run yet simply builds the one. `enter_evolve` is where that construction lives — it is the
    /// only place that knows how a run is made, and a second one here would be a second answer to
    /// what a run is.
    fn apply_run_settings(&mut self) {
        self.lab = None;
        self.lanes = None;
        self.packet = None;
        self.summary = None;
        self.replay = None;
        self.view.scrub = None;
        self.saved_end = false;
        self.enter_evolve();
        if self.lab.is_some() {
            // The seed is on the note because a run *is* a function of it: a population worth
            // keeping is a number worth reading off the screen (the same bargain the Restart
            // button makes).
            self.note = Some((
                format!("new run: pop {} seed {}", self.options.pop_size, self.seed),
                6.0,
            ));
        }
    }

    /// The Details window while an episode is on screen: whose record it is, and the field behind it.
    fn details_replay(&mut self, ui: &mut egui::Ui) {
        let Some(replay) = &mut self.replay else {
            ui.label("no replay on screen");
            return;
        };
        let actions = panel::replay_details(ui, replay);
        self.apply_actions(actions);
    }

    /// The Details window while a level is on screen: the verdict first, because it is what a person
    /// opens the window to argue with, then the physics a jump may ask for, what the level is made
    /// of, and the size of its grid.
    fn details_level(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| self.verdict_lamp(ui));
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

        // The rest of the window is a *board*: a label on the left, the reading on the right, and
        // one right edge down the page. It used to be eight `ui.small(format!(...))` sentences — the
        // same information, but a reader had to find the number inside each sentence and there was no
        // column to compare down. `ui::stat_row` is that decision as one function (see its doc), and
        // `panel::section` is the same section rule every other board in the window uses, so the four
        // Details windows cannot drift into four different machines for drawing a heading.
        let physics = &sim::PHYSICS;
        panel::section(ui, "What a jump can do");
        ui::stat_row(
            ui,
            "jump",
            format!(
                "{:.2} tiles high, {:.2} far",
                physics.max_jump_height(),
                physics.max_jump_distance()
            ),
        );
        // The two abilities that change what a level may ask for, in the same units as the jump
        // above: a pit wider than the running jump is a sprint jump, and a rise taller than the jump
        // height is a wall climb. A designer reading this board is exactly who finding 44's numbers
        // are for.
        ui::stat_row(
            ui,
            "sprint jump",
            format!("{:.2} tiles far", physics.sprint_jump_distance()),
        );
        ui::stat_row(
            ui,
            "wall climb",
            format!("{:.2} tiles high", physics.wall_jump_height()),
        );
        ui::stat_row(
            ui,
            "run speed",
            format!(
                "{:.2}/tick ({:.2} sprinting)",
                physics.run_speed, physics.sprint_speed
            ),
        );
        ui::stat_row(ui, "gravity", format!("{:.4}", physics.gravity));

        panel::section(ui, "The level");
        // **The name is the file.** Renaming is the other half of the naming bug: a level's name
        // is what its file is called, so a document that cannot be renamed is a document that
        // cannot be named at all — every level a person drew was called `untitled` (AUDIT.md B2).
        // Commit on Enter or on the button, never per keystroke, and let [`Editor::rename`] refuse
        // a blank: an empty field is somebody still typing.
        if !self.rename_focused {
            self.rename_buf = self.editor.level.name.clone();
        }
        ui.horizontal(|ui| {
            ui.label("name");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.rename_buf)
                    .desired_width(150.0)
                    .hint_text("level name"),
            );
            self.rename_focused = field.has_focus();
            let typed = field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if typed || ui.button("Rename").clicked() {
                let name = self.rename_buf.clone();
                if self.editor.rename(&name) {
                    self.rename_buf = self.editor.level.name.clone();
                }
            }
        });
        let level = &self.editor.level;
        ui::stat_row(ui, "grid", format!("{}x{}", level.width, level.height));
        ui::stat_row(ui, "spawn", format!("{:?}", level.spawn_tile()));
        // The frame cap this level's own size sets, and the clock an agent is scored against: the
        // one number that says how long a solution is allowed to take, which is what the run's best
        // time is read against.
        ui::stat_row(
            ui,
            "episode cap",
            format!(
                "{} frames ({})",
                sim::frame_budget(level),
                sim::format_time(sim::frame_budget(level))
            ),
        );
        ui::stat_row(
            ui,
            "goal",
            match level.goal_tiles().first() {
                Some((gx, gy)) => format!("({gx}, {gy})"),
                // A level has one flag, or none at all for the moment between deleting the flag
                // and placing it again — and a flag-less document is one the writer refuses, which
                // is worth knowing here rather than at Save.
                None => "none — place one before saving".into(),
            },
        );
        ui::stat_row(ui, "walkers", level.walkers.len().to_string());
        // Every solid tile counts as ground, including the brittle ones — they hold a body up
        // exactly as their material does, and it is the *level* the row is describing. Two of the
        // readings are named out loud when a level has any, in the existing voice: sand, because
        // it is the tile whose *material* an author chooses deliberately (a level with sand in it
        // is being built as a beach), and cracked, because a bridge that gives way is the level's
        // whole idea rather than a texture. The two are separate axes rather than a partition — a
        // cracked beach is sand and is brittle — and a row that showed one and hid the other would
        // make a level's most important decision invisible from the window it was drawn in.
        let (mut solid, mut sand, mut cracked, mut spike, mut spring) = (0, 0, 0, 0, 0);
        for ty in 0..level.height {
            for tx in 0..level.width {
                let Some(tile) = level.tile(tx, ty) else {
                    continue;
                };
                if tile.is_solid() {
                    solid += 1;
                }
                if tile.is_sand() {
                    sand += 1;
                }
                if tile.is_cracked() {
                    cracked += 1;
                }
                if tile == Tile::Spike {
                    spike += 1;
                }
                if tile.is_spring() {
                    spring += 1;
                }
            }
        }
        let mut census = if cracked > 0 {
            format!("{solid} solid ({sand} sand, {cracked} cracked)")
        } else if sand > 0 {
            format!("{solid} solid ({sand} sand)")
        } else {
            format!("{solid} solid")
        };
        census.push_str(&format!(", {spike} spikes"));
        if spring > 0 {
            census.push_str(&format!(", {spring} springs"));
        }
        ui::stat_row(ui, "tiles", census);
        // The content hash, which a champion file records: shown because a viewer comparing a run
        // against a level wants to know the two are the same terrain, and it is the hash rather
        // than a date because that is what the file can be checked against.
        ui::stat_row(ui, "content", level.content_hash_string());

        panel::section(ui, "Resize the grid");
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
        // The zoom, which used to be the status bar's business: a number about the view that is set
        // with the wheel and read once, so it belongs with the readings a person looks up rather
        // than with the two the editor watches.
        ui.add_space(6.0);
        ui::stat_row(ui, "zoom", format!("{:.0} px/tile", self.camera.scale()));
    }

    /// The canvas: level, ghosts, and every pointer gesture.
    fn canvas(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        if self.fit_pending {
            // Fit what is on screen, not the document: a replay is fitted to the terrain
            // its episode was scored on (see `viewed_level`).
            let fit = Camera::fit(self.viewed_level(), rect, 24.0);
            self.camera = fit;
            self.fit_pending = false;
        }

        // --- camera gestures -------------------------------------------------
        let hover = response.hover_pos();
        self.hover_tile = hover.map(|p| {
            let tile = self.camera.to_tile(p, rect);
            (tile.x.floor() as i32, tile.y.floor() as i32)
        });
        // The pointer over the editor's canvas is a *tool* rather than a pointer at something: the
        // crosshair is where the next tile lands. Moving the level under it is the one gesture that
        // is not a placement, so it wears the grab hand while it is happening (AUDIT.md F5).
        if self.mode == Mode::Edit && hover.is_some() {
            ui.ctx().set_cursor_icon(if self.dragging.is_some() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Crosshair
            });
        }
        if let Some(hover) = hover {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                self.camera.zoom_at((scroll * 0.002).exp(), hover, rect);
            }
        }
        // Middle-drag, or alt-drag, moves the level under the cursor. Alt is there
        // because a two-button mouse on a trackpad rarely has a middle button.
        let panning = ui
            .ctx()
            .input(|i| i.pointer.middle_down() || (i.pointer.primary_down() && i.modifiers.alt));
        if panning {
            if let (Some(now), Some(last)) = (hover, self.last_pointer) {
                let delta = now - last;
                // Divided by the scale the frame is drawn at, not by the raw zoom: the
                // level has to move exactly as far as the grid under the cursor says it did.
                self.camera.centre -= delta / self.camera.scale();
            }
        }
        self.last_pointer = hover;

        // Following is a camera rule, applied after the viewer's own gestures so the frame
        // being drawn is the frame the camera is on rather than the one before it. It is
        // the terminal's `f`: earlier only because an eighty-column frame cannot hold a
        // whole level, and here it is the viewer choosing to look at one agent — the
        // leader of a generation, or the champion of a replay.
        if self.view.follow && !self.fit_pending {
            match self.mode {
                Mode::Evolve => self.follow_leader(),
                Mode::Replay => self.follow_champion(),
                Mode::Edit | Mode::Play => {}
            }
        }

        // --- draw ------------------------------------------------------------
        let opts = self.opts();
        // The camera the frame is drawn through is exactly the viewer's: it is not displaced by
        // anything the simulation does. A landing, a stomp and a death each put a burst on the
        // agent's own animation, which is where they belong — a jolt that moved the whole level
        // under the eye read as the window being kicked rather than the body hitting the ground.
        let sheet = Sheet::new(&painter, &self.atlas, &self.sprites, &self.camera, rect);
        // What keeps the scene alive — the swaying grass, the flag, the clouds — runs on
        // the replay's own clock while an episode is on screen, so pausing the replay
        // pauses all of it and two frames of one episode are the same picture whatever the
        // machine's frame timing did. The editor and the evolution view have no episode to
        // take a clock from, so they keep the window's.
        let clock = match self.mode {
            Mode::Replay => self.replay.as_ref().map_or(0.0, |replay| replay.clock()),
            Mode::Edit | Mode::Play | Mode::Evolve => ui.ctx().input(|i| i.time) as f32,
        };

        // The editor draws a document — graph paper behind the level — while watching a
        // run draws a game: sky, hills and clouds, parallaxed against the camera. The two
        // are the same renderer, and which one is up is the only difference.
        let (backdrop, level) = match self.mode {
            Mode::Edit => (view::Backdrop::Checker, &self.editor.level),
            Mode::Play | Mode::Evolve | Mode::Replay => (
                view::Backdrop::Sky,
                self.lab
                    .as_ref()
                    .map(|lab| lab.level())
                    .unwrap_or(&self.editor.level),
            ),
        };
        view::draw_background(sheet, level, backdrop, clock);

        match self.mode {
            Mode::Edit => view::draw_level(
                sheet,
                &self.editor.level,
                opts,
                Some(&self.editor.report),
                clock,
            ),
            Mode::Play => {
                if let Some(session) = &self.session {
                    // The body is drawn *between* the simulator's ticks (`Session::body_at`):
                    // the sim runs at 60 Hz whatever the display does, and drawing the last
                    // tick's position is what made the playtest stutter on a fast screen.
                    view::draw_sim_at(
                        sheet,
                        session.sim(),
                        opts,
                        session.clock,
                        session.anim(),
                        Some(session.body_at()),
                    );
                    // The champion beside the player, translucent so it never hides them:
                    // the best band's own tint, because that is what a ghost looks like
                    // everywhere else in this window.
                    if let (Some(gsim), Some(ganim)) = (session.ghost_sim(), session.ghost_anim()) {
                        view::draw_agent_body_at(
                            sheet,
                            gsim,
                            view::cohort_tint(0),
                            ganim,
                            session.ghost_at(),
                        );
                    }
                }
            }
            Mode::Evolve => self.canvas_evolve(sheet, clock),
            Mode::Replay => self.canvas_replay(sheet, clock),
        }

        // The light the frame is *seen* through, over everything it contains: a vignette at
        // the edges and a soft band of shade under the top of the window, which is what stops
        // a pixel landscape from reading as a flat sheet of tiles. Not in the editor: a
        // document is about what the file says, and the editor passes no judgement on it.
        if self.mode != Mode::Edit {
            view::draw_atmosphere(&painter, rect);
        }
        // The scoreboard goes *over* the atmosphere, not under it: it is a readout on the
        // machine the game is running on, and a vignette over a number is a number somebody
        // has to lean in to read.
        if self.mode == Mode::Play {
            self.play_hud(&painter, rect);
        }

        if self.mode == Mode::Edit {
            self.canvas_edit_gestures(ui, &response, rect, &painter);
        }
    }

    /// The playtest's scoreboard: the three numbers a player reads while playing.
    ///
    /// A dark, gold-framed strip in the corner of the picture, the way every game that has ever
    /// had a timer printed it: a label in gold caps over a value in cream, one cell per number,
    /// cells divided by the same gold hairline the panels use. It is deliberately not the status
    /// bar — the strip along the bottom is the *window's* readout (the mode, the keys, this
    /// episode's speed run of statistics), and this is the *player's*: the clock they are racing,
    /// their own best attempt at this level, and how much of the distance to the flag is behind
    /// them. Reading it while playing means it has to be where the eye already is.
    fn play_hud(&self, painter: &egui::Painter, rect: Rect) {
        let Some(session) = &self.session else {
            return;
        };
        let stats = session.sim().stats();
        // The same quantity the fitness scores and the status line prints: the share of the
        // distance to the flag that has been closed, so a level whose goal is above the start
        // reads honestly rather than as a number about `x`.
        let progress = if stats.goal_distance_pixels > 0.0 {
            (100.0 * stats.progress_pixels / stats.goal_distance_pixels).clamp(0.0, 100.0)
        } else {
            100.0
        };
        let cells: [(&str, String); 3] = [
            ("TIME", sim::format_time(session.frames())),
            (
                "YOUR BEST",
                session
                    .best()
                    .map(sim::format_time)
                    .unwrap_or_else(|| "--".to_string()),
            ),
            ("TO THE FLAG", format!("{progress:.0}%")),
        ];
        let label_font = FontId::monospace(9.0);
        let value_font = FontId::monospace(17.0);
        // Measured before it is drawn, like every plate in the window: a cell sized by a guess
        // clips the one value that happened to be wider than the guess.
        let widths: Vec<f32> = cells
            .iter()
            .map(|(label, value)| {
                let l =
                    painter.layout_no_wrap(label.to_string(), label_font.clone(), theme::RIBBON);
                let v = painter.layout_no_wrap(value.clone(), value_font.clone(), theme::OUTLINE);
                l.size().x.max(v.size().x) + 26.0
            })
            .collect();
        let height = 46.0;
        let total: f32 = widths.iter().sum();
        let panel = Rect::from_min_size(
            Pos2::new(rect.left() + 16.0, rect.top() + 16.0),
            Vec2::new(total, height),
        );
        // A warm-white card with the same brown outline and rounded corner as the window's own
        // panels, and a gold hairline inside it: the scoreboard is a piece of the game's menu
        // pinned over the picture, not a bar of the operating system.
        painter.rect_filled(
            panel.translate(Vec2::new(0.0, 4.0)),
            CornerRadius::same(theme::RADIUS_LG),
            Color32::from_rgba_unmultiplied(74, 52, 32, 90),
        );
        painter.rect_filled(panel, CornerRadius::same(theme::RADIUS_LG), theme::CARD);
        painter.rect_stroke(
            panel,
            CornerRadius::same(theme::RADIUS_LG),
            Stroke::new(3.0_f32, theme::OUTLINE),
            StrokeKind::Inside,
        );
        painter.rect_stroke(
            panel.shrink(5.0),
            CornerRadius::same(theme::RADIUS),
            Stroke::new(1.0_f32, theme::GOLD_EDGE.gamma_multiply(0.7)),
            StrokeKind::Inside,
        );
        let mut x = panel.left();
        for (index, ((label, value), width)) in cells.iter().zip(widths.iter()).enumerate() {
            let cell = Rect::from_min_size(Pos2::new(x, panel.top()), Vec2::new(*width, height));
            if index > 0 {
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(cell.left(), cell.top() + 8.0),
                        Vec2::new(1.0, height - 16.0),
                    ),
                    CornerRadius::ZERO,
                    theme::EDGE,
                );
            }
            let l = painter.layout_no_wrap(label.to_string(), label_font.clone(), theme::RIBBON);
            painter.galley(
                Pos2::new(cell.left() + 13.0, cell.top() + 7.0),
                l,
                theme::RIBBON,
            );
            let v = painter.layout_no_wrap(value.clone(), value_font.clone(), theme::OUTLINE);
            painter.galley(
                Pos2::new(cell.left() + 13.0, cell.bottom() - 7.0 - v.size().y),
                v,
                theme::OUTLINE,
            );
            x += width;
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
    fn canvas_evolve(&self, sheet: Sheet<'_>, clock: f32) {
        let Some(lab) = &self.lab else {
            return;
        };
        // With no generation on screen there is no simulation to draw the level's enemies, so
        // the document's own placement stands in for them — the same static pass the editor
        // draws. Without this the frame between entering the view and the first packet (and
        // any paused or restarted run) is a level with nothing alive in it, which reads as the
        // editor having lost the chaser that is plainly there in Edit and Play.
        let opts = if self.lanes.is_none() {
            view::RenderOpts {
                entities: true,
                ..self.opts()
            }
        } else {
            self.opts()
        };
        view::draw_level(sheet, lab.level(), opts, None, clock);

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
                // The champion is the agent the generation is about, so it is the one
                // that gets dust, a landing puff and a death burst. Eight of everything at
                // once is noise, and the ghosted lanes are there to be seen, not read.
                view::draw_agents(
                    sheet,
                    lanes.sim(i),
                    lanes.clock(),
                    Color32::from_white_alpha(alpha),
                    lanes.anim(i),
                    lane.rank == 0,
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
                // The same ghost the hover draws, because it is the same promise: this is the
                // tile, and this is whether the level will take it.
                self.draw_brush_ghost(painter, rect, brush, tx, ty);
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

        let (primary, secondary, released, picking, line) = ui.ctx().input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
                i.pointer.any_released(),
                // `command` rather than `ctrl` so the key is the platform's own modifier, the same
                // one Ctrl+Z and Ctrl+S already answer to.
                i.modifiers.command,
                i.modifiers.shift,
            )
        });
        if released {
            self.editor.end_stroke();
        }
        if !inside || !(response.hovered() || response.dragged()) {
            return;
        }
        // The brush in hand, drawn at the tile the *next* press would take. Until this existed the
        // brush's target was invisible until the click landed, which is the one piece of feedback
        // every drawing tool has (AUDIT.md V2) — and it is the same picture the drag out of the
        // palette draws, on purpose: a brush in hand looks the same whether it was picked up a
        // moment ago or an hour ago.
        let hover_ghost = !primary && !secondary;
        if hover_ghost {
            if let Some(brush) = self.editor.brush {
                self.draw_brush_ghost(painter, rect, brush, tx, ty);
            }
        }
        // The shift key is the *stroke's* shape, not the palette's (see [`Editor::line`]): a long
        // straight floor is what a steady hand cannot draw, so the constraint is asked for per
        // stroke rather than becoming a mode somebody forgets they are in.
        self.editor.line = line;
        // `command`-click takes the brush *from* the level instead of putting it down: the
        // eyedropper. It accepts **either** button deliberately: on the platform where ctrl-click
        // is translated into a right-click, a person holding the modifier to pick arrives as
        // `secondary`, and answering that with the eraser would be the one gesture the modifier
        // exists to prevent. A plain right-click has no modifier and still erases.
        if picking && (primary || secondary) {
            self.editor.pick_at(tx, ty);
        } else if secondary {
            // Right-click erases whatever is under the cursor, regardless of the brush.
            self.editor.erase_at(tx, ty);
        } else if primary {
            self.editor.paint(tx, ty);
        }
    }

    /// The ghost of the held brush, at the tile the pointer is over.
    ///
    /// Extracted because two gestures need to draw it — dragging a brush out of the palette, and
    /// simply hovering the canvas with one in hand — and two copies of the goal's two-tile footprint
    /// would be two chances for the flag's ghost to promise the wrong cell (AUDIT.md V2).
    fn draw_brush_ghost(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        brush: Brush,
        tx: i32,
        ty: i32,
    ) {
        let target = self.camera.tile_rect(tx, ty, rect);
        // A goal is two tiles of art, and the ghost has to show both: the flag hangs
        // in the cell *above* the one being clicked (`Level::goal_covers`), so a ghost
        // drawing only the clicked cell would promise the flag in the wrong place —
        // and hide the fact that the cell the outline is really asking about is the
        // one above the cursor.
        let footprint = if brush == Brush::Goal {
            let above = self.camera.tile_rect(tx, ty - 1, rect);
            for (cell, name) in [
                (target, "goal_bottom"),
                (above, "goal_top"),
                (above, "goal_flag_1"),
            ] {
                if let Ok(uv) = self.atlas.uv(name) {
                    painter.image(
                        self.atlas.texture_id(),
                        cell,
                        uv,
                        Color32::from_white_alpha(170),
                    );
                }
            }
            target.union(above)
        } else if brush == Brush::Platform {
            // A moving platform is several tiles wide, and a one-cell ghost would promise a
            // shelf the click will not place. The width is the brush's own setting, so the
            // ghost draws exactly the shelf about to land — ends and middle included.
            let tiles = self.editor.platform_tiles.max(1);
            let mut span = target;
            for i in 0..tiles {
                let cell = self.camera.tile_rect(tx + i, ty, rect);
                span = span.union(cell);
                let piece = if i == 0 && tiles == 1 {
                    "mover_m"
                } else if i == 0 {
                    "mover_l"
                } else if i == tiles - 1 {
                    "mover_r"
                } else {
                    "mover_m"
                };
                if let Ok(uv) = self.atlas.uv(piece) {
                    painter.image(
                        self.atlas.texture_id(),
                        cell,
                        uv,
                        Color32::from_white_alpha(170),
                    );
                }
            }
            span
        } else {
            // The same two layers the palette cell draws, for the same reason: the ghost of a
            // cracked brush has to promise a *cracked tile* on the level, not a bare crack.
            let (icon, flip) = self.brush_icon(brush);
            for (name, flip) in [
                Self::brush_base(brush).map(|n| (n, false)),
                Some((icon, flip)),
            ]
            .into_iter()
            .flatten()
            {
                if let Ok(uv) = self.atlas.uv(name) {
                    painter.image(
                        self.atlas.texture_id(),
                        target,
                        crate::view::facing(uv, flip),
                        Color32::from_white_alpha(170),
                    );
                }
            }
            target
        };
        // The ghost shows the *drop*, so it has to show a refused drop too: the
        // outline is the colour of the answer, and the sprite is faded when the
        // answer is no.
        let placeable = self.editor.can_place(brush, tx, ty);
        painter.rect_stroke(
            footprint,
            egui::CornerRadius::ZERO,
            Stroke::new(1.5_f32, if placeable { theme::ACCENT } else { theme::BAD }),
            egui::StrokeKind::Inside,
        );
    }

    /// Move the camera to keep the player in view while playing.
    fn follow_player(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let (x, y) = session.camera_centre();
        self.camera.centre = Vec2::new(x, y);
        self.camera.clamp_to(&self.editor.level, 6.0);
        self.camera.zoom = self.camera.zoom.clamp(18.0, 48.0);
        // The camera is placed, so there is nothing left to fit. Without this the canvas would fit
        // the whole level for the first frame of a playtest and the follow would snap in on the
        // second — a flash of the map where the game should be.
        self.fit_pending = false;
    }

    /// The status line in the evolution view: where the run is, and what the viewer just
    /// did to it.
    ///
    /// Short on purpose, and it reads nothing that the panel beside the canvas cannot be asked
    /// about: this is the strip a viewer passes their eye along, not a report. It used to spell the
    /// field out ("40 of 40 lanes still running"), name the animation clock to a tenth of a second,
    /// and put the seed in the middle of the sentence, which is three more numbers to walk past on
    /// the way to the one that changed. What is left is the generation, whether the field is still
    /// moving, how fast it is going, and whether it is paused — plus the seed, because a number
    /// nobody can reproduce is a number nobody can check.
    fn evolve_status(&self) -> String {
        let speed = format!("{}x", self.view.speed);
        let mut line = match (&self.lanes, &self.packet) {
            (Some(lanes), _) => format!(
                "gen {} · {} of {} running · {}",
                lanes.packet().generation + 1,
                lanes.running(),
                lanes.len(),
                speed
            ),
            (None, Some(packet)) => {
                format!("gen {} scored · {}", packet.generation + 1, speed)
            }
            (None, None) => format!("evaluating the first generation… · {speed}"),
        };
        if self.view.paused {
            line.push_str(" · paused");
        }
        line.push_str(&format!(" · seed {}", self.seed));
        line
    }

    /// The strip along the bottom of the window.
    ///
    /// Cut into the window rather than lying on it ([`theme::hud_frame`]): it is a readout, and
    /// the four views each put their own sentence in it. Its height is what the frame's own padding
    /// makes it and nothing more — the strip is the one piece of the window that is allowed to be
    /// quiet, because everything in it is either a light or a number.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        // The HUD: the same warm cream as the dock, outlined in the same brown, drawn edge to
        // edge (the panel's own frame is cleared at the call site) so the strip is the bottom of
        // the window rather than a well cut into it.
        ui.style_mut().visuals.override_text_color = Some(theme::TEXT);
        let frame = theme::hud_frame();
        frame.show(ui, |ui| {
            self.status_line(ui);
        });
    }

    /// The line along the bottom: what this view is doing, and what the pointer is over.
    ///
    /// The mode used to lead this line — `edit`, `play`, `evolve`, `replay` in the mode's own
    /// colour — and it is gone, because the dock's switch says it in gold two hundred points above
    /// and saying it twice is the kind of redundancy that reads as a prototype: a bar that repeats
    /// the tool bar is a bar with nothing of its own to say.
    ///
    /// What is left in the editor is the **verdict** and then what just happened: the one reading a
    /// person drawing a level needs at all times, as a lamp and a word, and the sentence a stroke, an
    /// undo or a refusal just produced. The verdict lives here — and not in a panel of its own — for
    /// the reason the panel it came from is gone: half a window of cream to say four words is a
    /// properties pane, and this bar has the room.
    ///
    /// The zoom is gone from the right-hand end as well, into the window that holds the other
    /// numbers ([`Self::details_window`]). It is a fact about the *view* that is read once after it
    /// is set, and this end of the bar is for what a person asks while drawing: the tile under the
    /// pointer, and anything wrong with the atlas.
    fn status_line(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // The left-hand sentence is capped before it is drawn: a long line — a replay's
            // summary, a save confirmation with a path in it — must end in an ellipsis rather
            // than run under the pointer readout at the other end of the strip. Measured from
            // the strip's own width so a narrow window truncates sooner rather than later.
            let room = (ui.available_width() - 150.0).max(120.0);
            ui.scope(|ui| {
                ui.set_max_width(room);
                if matches!(self.mode, Mode::Evolve | Mode::Replay) {
                    // Each view speaks for itself: the run's line is about generations, and the
                    // replay's is about the one episode it is showing. Both carry the seed or
                    // the record, because a number nobody can reproduce is a number nobody can
                    // check.
                    let line = if self.mode == Mode::Replay {
                        self.replay.as_ref().map(|replay| replay.summary())
                    } else {
                        Some(self.evolve_status())
                    };
                    // Truncated rather than allowed to run off the end of the window: these two
                    // sentences are the longest in the window (a replay's line names the episode,
                    // its score and the field behind it; a save confirmation names a path), and a
                    // strip whose left-hand label is as wide as it likes pushes everything after
                    // it — including the tile under the pointer — off the screen entirely. An
                    // ellipsis says "there is more of this sentence" instead of hiding the rest.
                    ui.add(
                        egui::Label::new(line.unwrap_or_else(|| "no replay on screen".to_string()))
                            .truncate(),
                    );
                    if let Some((note, _)) = &self.note {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(note.clone()).color(theme::ACCENT_INK),
                            )
                            .truncate(),
                        );
                    }
                } else if let Some(session) = &self.session {
                    // The playthrough's line in the runner's own colour, which is what the removed
                    // `play` word was saying: the bar still says which of the four things this is,
                    // it just says it in ink rather than in a label.
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(session.summary()).color(theme::RUN_INK),
                        )
                        .truncate(),
                    );
                } else {
                    // The editor's line: the verdict, then only what just happened. Its message is
                    // empty while nothing is happening on purpose — a bar that talks to itself is
                    // chatter — which leaves the lamp and the word, and those never stop mattering.
                    self.verdict_lamp(ui);
                    ui.label(
                        egui::RichText::new(self.editor.message.clone()).color(theme::TEXT_DIM),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // **The one hint that has to be permanent.** A window with two dozen keys and no
                // menu to hold them has to say where the list is, or nobody goes looking for it:
                // the card existed as a keybinding nobody was told about (AUDIT.md F1).
                // Both of these sit on the *sunken* status bar, which is the one chrome surface
                // the tertiary ink is not cleared for: readouts on the well take `TEXT_DIM`.
                ui.label(egui::RichText::new("? keys").small().color(theme::TEXT_DIM))
                    .on_hover_text("the keys for this view — F1 does the same");
                if let Some((tx, ty)) = self.hover_tile {
                    ui.label(
                        egui::RichText::new(format!("tile ({tx}, {ty})"))
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                }
                if let Some(warning) = &self.warning {
                    // The warning's *ink*, not its fill: `WARN` is 1.5:1 on this surface and is a
                    // lamp's colour, not a label's.
                    ui.colored_label(theme::WARN_INK, warning);
                }
            });
        });
    }

    /// One frame of the title screen, and what the viewer asked for, if anything.
    fn splash_frame(&mut self, ctx: &Context) -> Option<Action> {
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let splash = self.splash.as_mut()?;
        splash.advance(dt);
        splash.show(ctx, &self.atlas, &self.sprites)
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
            let buffer =
                image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, bytes)
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

/// A frame longer than this is one a viewer can feel, and the threshold [`FrameClock`] reports at.
///
/// Two frames at 60 Hz, which is the smallest hitch that reads as a stutter rather than as a
/// frame: the instrument is for attributing a *freeze*, and a log of every 17 ms frame would be
/// noise nobody reads.
const SLOW_FRAME: Duration = Duration::from_millis(33);

/// The frame time log, on when `EVO_FRAME_LOG` is set.
///
/// "The window freezes" is a report no profiler can take, because the window is not doing
/// anything: it is waiting. This prints every frame that took longer than [`SLOW_FRAME`], with
/// the mode and the work the frame did, so a stall can be attributed instead of guessed at —
/// which is how the practice detour's 1.7 s packet was found to be the one freeze worth fixing.
/// Off by default and one env-var read when set, so the shipped window pays nothing.
fn slow_frame_log() -> bool {
    std::env::var_os("EVO_FRAME_LOG").is_some()
}

impl eframe::App for EvoApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }
}

impl EvoApp {
    /// One frame of the window.
    ///
    /// Split from [`eframe::App::update`] only so a test can run a frame: `eframe::Frame` can
    /// only be made by `eframe`, and a harness that needs one cannot exist. Everything else is
    /// the frame a person sees, drawn in the same order by the same code.
    fn frame(&mut self, ctx: &Context) {
        let frame_started = Instant::now();
        // The title bar follows the document from the first frame, splash or not: a window
        // that named itself once and never again is how two open levels came to look like one
        // window twice (AUDIT.md B5).
        self.sync_window_title(ctx);
        // The title screen owns the frame while it is up: the app's own panels are not drawn
        // and nothing else runs, so a key that dismisses it cannot also reach a button in the
        // editor behind it — the same event would otherwise both close the screen and start a
        // playtest. Everything below is the frame after that.
        if self.splash.is_some() {
            let action = self.splash_frame(ctx);
            if let Err(err) = self.handle_screenshot(ctx) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
            // The screen is alive whether or not the viewer does anything, and a window that
            // repaints only on input would show a frozen runner to somebody deciding.
            ctx.request_repaint();
            if let Some(action) = action {
                crate::sound::play(crate::sound::Sfx::MenuConfirm, 1.0);
                self.splash = None;
                // The door leads where the sign says: the two keys the title screen names
                // enter the activity they name instead of dropping the viewer into the
                // editor to press the same key again.
                match action {
                    Action::Edit => {}
                    Action::Play => self.enter_play(),
                    Action::Evolve => self.enter_evolve(),
                }
            }
            return;
        }

        self.editor.refresh_report();
        self.handle_shortcuts(ctx);

        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if self.mode == Mode::Play {
            // Same rule as the shortcuts: a text field with the keyboard is typing, not running
            // the agent — otherwise renaming a level mid-run makes the runner walk into a pit.
            let keys = if ctx.wants_keyboard_input() {
                play::Keys::default()
            } else {
                play::Keys::read(ctx)
            };
            let thinking = self.thinking;
            if let Some(session) = &mut self.session {
                if thinking {
                    if let Some(driver) = &mut self.driver {
                        // The controller decides on the session's own tick grid, so the
                        // episode on screen is the one it reasoned about — not a replay
                        // against the frame rate.
                        session.advance_actions(dt, |sim| driver.next(sim));
                    } else {
                        session.advance(dt, keys);
                    }
                } else {
                    session.advance(dt, keys);
                }
                // A death restarts itself after a beat ([`play::AUTO_RESTART_DELAY`]):
                // the window does for the player what the session's signal asks for, and
                // `R` still restarts at once — this is the pause doing it instead.
                if session.restart_due() {
                    session.restart(&self.editor.level);
                    if let Some(driver) = &mut self.driver {
                        driver.reset();
                    }
                }
            }
            ctx.request_repaint();
        }
        if self.mode == Mode::Evolve {
            self.evolve_step(dt);
            // A **paused** run is not moving, so it should not cost a frame sixty times a
            // second. Redraw while the run is live, while the first generation is still being
            // scored, and while something is counting down (a note, the genius splash, a
            // discovery card); input repaints the window by itself, so the controls still
            // respond while it sits still.
            //
            // This is the second half of the long-run lag: the fitness chart used to redraw
            // every generation's points on every one of those frames. The chart is bounded
            // now (see `panel::chart_geometry`), but a paused run has no reason to ask for
            // the frame at all.
            let live = self.lab.is_some()
                && !self.view.paused
                && !self.packet.as_ref().is_some_and(|packet| packet.finished);
            let animating = !self.view.paused
                && self
                    .lanes
                    .as_ref()
                    .is_some_and(|lanes| !lanes.is_finished());
            let loading = self.lab.is_some() && self.packet.is_none();
            let counting = self.note.as_ref().is_some_and(|(_, left)| *left > 0.0)
                || self.genius_left > 0.0
                || self.discovery.is_some();
            if live || animating || loading || counting {
                ctx.request_repaint();
            }
        }
        if self.mode == Mode::Replay {
            self.replay_step(dt);
            // The episode is moving (or held by the viewer), and either way the frame has
            // to be redrawn for the clock on it to be worth reading.
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_bar(ui));
        // The palette is editing furniture; the evolution view would rather have the
        // width for the level.
        if self.mode == Mode::Edit {
            egui::SidePanel::left("palette")
                .default_width(190.0)
                // A panel of *chrome*: cream, with the same gold lip as the dock. The window's
                // menus are one material and the two things you look at — the canvas and the
                // chart — are another, which is the whole of what stops this reading as a tool.
                .frame(theme::panel_frame())
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.palette(ui))
                });
        }
        // A run's panel, and *only* a run's. The editor used to have one as well — the verdict at
        // the top and six sections of readings under it — and it is gone: everything the editor has
        // to say that must be up all the time fits in the strip along the bottom, and the rest is a
        // window a person opens ([`Self::details_window`]). An empty panel is the cheapest way to
        // make a game look like an application, and it was doing exactly that.
        //
        // What the two run views keep is different: their panel holds the controls a run is *driven*
        // with — pausing it, spending a generation, saving the champion — so it is busy by nature
        // rather than empty by default, and it sits beside the chart that says whether any of it is
        // working.
        if matches!(self.mode, Mode::Evolve | Mode::Replay) {
            egui::SidePanel::right("run")
                .frame(theme::panel_frame())
                .default_width(310.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| match self.mode {
                        Mode::Evolve => self.evolve_panel(ui),
                        Mode::Replay => self.replay_panel(ui),
                        // Unreachable — the panel only exists for the two modes above. Written out
                        // rather than `unreachable!()` so that adding a mode to the enum is a
                        // compile error here rather than a panic at runtime.
                        Mode::Edit | Mode::Play => {}
                    })
                });
        }
        // The chart is the run's shape, and the replay is a moment *of* that run: keeping
        // it up is what makes a jump in the curve and an agent clearing the obstacle it
        // stands for two views of one thing rather than two screens.
        if matches!(self.mode, Mode::Evolve | Mode::Replay) {
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
            .frame(egui::Frame::new())
            .show(ctx, |ui| self.status_bar(ui));
        // Over everything, like a window should be: the shelf is a separate thing the viewer
        // opened, not a panel of the view they were in.
        self.shelf_window(ctx);
        // Over the canvas like the shelf, and over the shelf's own z-order if both are up: the
        // card answers a question about the window, not about a level.
        help::window(ctx, self.mode, &mut self.help_open);

        // The playtest's camera is placed **before** the canvas is drawn, and on the same
        // interpolated clock the body is drawn on. Setting it afterwards is a camera one frame
        // behind the body it follows; setting it from the sim's last tick while the body is drawn
        // between two ticks is worse — the body's offset from the centre saws from zero to a whole
        // tick's travel every tick, which is the playtest's "jerky" (see
        // [`play::Session::camera_centre`]). Both are free to get right and both are invisible
        // until a screen draws faster than the simulation.
        if self.mode == Mode::Play {
            self.follow_player();
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::default())
            .show(ctx, |ui| self.canvas(ui));

        // The level's numbers float over the canvas rather than holding a column of their own, so
        // they are drawn after it and end up in front of it — which is where a window belongs.
        self.details_window(ctx);
        // A find is a moment, not a reading: the card is drawn over everything and fades on its
        // own timer, and the codex is the page it is recorded on.
        self.discovery_card(ctx);
        self.codex_window(ctx);
        // ...and so does the one question the window asks about the document itself.
        self.discard_notice(ctx);

        // A scripted frame has to contain a real run, and `--replay` needs something
        // scored to replay: both paths prime the run with actual generations first, and
        // what differs is only what the window does with them afterwards.
        let scripted = self.screenshot.is_some() || self.options.replay;
        if scripted && self.mode == Mode::Evolve && !self.primed {
            self.primed = true;
            self.prime_scripted_run();
        }

        // **The cut between the four things this window does.** A whole different screen — sky,
        // hills and agents where graph paper and a brush were — arrived instantly, and a cut that
        // hard reads as a glitch rather than as a change (AUDIT.md V1). Compared *here*, after
        // everything has drawn, so every way into a mode fades: the switch, the keys, the shelf
        // opening a level, the title screen's doors.
        if self.mode != self.last_mode {
            self.last_mode = self.mode;
            // **A scripted frame is a fixed frame.** `--screenshot` is how the renderer is verified
            // without a human watching, and the first frame of a `--play` or `--evolve` launch is
            // exactly when the shutter is fully closed: the photograph would be of the shutter.
            // The window a person watches gets the fade; the camera does not.
            if self.screenshot.is_none() {
                self.mode_fade = MODE_FADE;
            }
        }
        if self.mode_fade > 0.0 {
            self.mode_fade = (self.mode_fade - dt).max(0.0);
            // Eased out, and over *everything* — the dock, the panels, the shelf, the key card:
            // a shutter that left a strip of the old screen showing would be the flicker it
            // exists to cover.
            let t = self.mode_fade / MODE_FADE;
            let alpha = (t * t * 235.0) as u8;
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("mode-shutter"),
            ))
            .rect_filled(
                ctx.screen_rect(),
                0.0,
                egui::Color32::from_rgba_unmultiplied(
                    theme::SHELL.r(),
                    theme::SHELL.g(),
                    theme::SHELL.b(),
                    alpha,
                ),
            );
            ctx.request_repaint();
        }

        if let Err(err) = self.handle_screenshot(ctx) {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
        if slow_frame_log() {
            let took = frame_started.elapsed();
            if took > SLOW_FRAME {
                eprintln!(
                    "frame {:?} in {:?} (generation {:?}, {} lanes)",
                    took,
                    self.mode,
                    self.packet.as_ref().map(|p| p.generation),
                    self.lanes.as_ref().map(|l| l.len()).unwrap_or(0)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A levels directory of our own, holding copies of two shipped levels.
    ///
    /// A temp directory rather than `levels/`: these tests press *Save*, *New* and the
    /// shelf, and a test that can write into the pack is a test that can corrupt it.
    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("evo-app-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("levels")).unwrap();
        std::fs::create_dir_all(root.join("champions")).unwrap();
        let pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../levels");
        for slug in ["l1-flat-run", "l2-first-gap"] {
            std::fs::copy(
                pack.join(format!("{slug}.json")),
                root.join(format!("levels/{slug}.json")),
            )
            .unwrap();
        }
        root
    }

    fn app_on(ctx: &Context, level: &Path) -> EvoApp {
        let options = LabOptions {
            champions: level.parent().unwrap().join("../champions"),
            ..Default::default()
        };
        EvoApp::build(
            ctx,
            level.to_path_buf(),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"),
            None,
            options,
            Some(false),
        )
        .expect("the window builds")
    }

    #[test]
    fn think_mode_puts_the_deliberate_controller_at_the_keys() {
        let ctx = Context::default();
        let root = scratch("think");
        let mut app = app_on(&ctx, &root.join("levels/l2-first-gap.json"));
        app.options.think = true;
        app.enter_play();
        assert!(app.thinking, "--think opens with the AI at the keys");
        let mut driver = app.driver.take().expect("a driver was built");
        let mut session = app.session.take().expect("a playtest session");
        for _ in 0..1200 {
            if session.sim().outcome().is_finished() {
                break;
            }
            session.advance_actions(1.0 / 60.0, |sim| driver.next(sim));
        }
        assert_eq!(
            session.sim().outcome(),
            sim::Outcome::Won,
            "the deliberate controller reached the gap level's flag"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "plans the whole l11 route; run with --ignored for the headline check"]
    fn think_mode_solves_the_elements() {
        let ctx = Context::default();
        let level =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../levels/l11-the-elements.json");
        let mut app = app_on(&ctx, &level);
        app.options.think = true;
        app.enter_play();
        let mut driver = app.driver.take().expect("a deliberate driver");
        let mut session = app.session.take().expect("a playtest session");
        for _ in 0..2000 {
            if session.sim().outcome().is_finished() {
                break;
            }
            session.advance_actions(1.0 / 60.0, |sim| driver.next(sim));
        }
        assert_eq!(
            session.sim().outcome(),
            sim::Outcome::Won,
            "the deliberate controller reached l11's flag"
        );
    }

    /// Run one real frame of the window, with `events` delivered to it.
    ///
    /// `time` advances by a sixtieth per frame, as a real window's does: egui decides whether two
    /// presses are a double-click from the clock, and a harness that fed every frame `time = 0`
    /// would make every click in a test the second half of one.
    fn frame(ctx: &Context, app: &mut EvoApp, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0))),
            time: Some(ctx.input(|i| i.time) + 1.0 / 60.0),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.frame(ctx));
    }

    /// Press and release at `at` over three frames, the way a hand does.
    fn click(ctx: &Context, app: &mut EvoApp, at: Pos2) {
        // Hover first: egui decides what a press lands on from the pass before it, so a
        // press that arrives in the same frame as the movement that aimed it hits nothing.
        frame(ctx, app, vec![egui::Event::PointerMoved(at)]);
        frame(
            ctx,
            app,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        frame(
            ctx,
            app,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
    }

    fn centre_of(app: &EvoApp, name: &str) -> Pos2 {
        app.tool_rects
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("{name} was not drawn; have {:?}", app.tool_rects))
    }

    /// A window on a scratch pack, with a context of its own.
    ///
    /// A context per scenario rather than one per test: egui remembers a window's position, its
    /// size and its scroll *by id*, so two scenarios sharing a context would be two windows with
    /// one memory, and the second would be clicking wherever the first left the frame.
    fn window(tag: &str) -> (Context, PathBuf, EvoApp) {
        let root = scratch(tag);
        let level = root.join("levels/l1-flat-run.json");
        let ctx = egui::Context::default();
        let mut app = app_on(&ctx, &level);
        frame(&ctx, &mut app, vec![]);
        (ctx, root, app)
    }

    /// A point on the shelf's second card.
    ///
    /// The cards start a known distance below the window's title bar (the title, the directory
    /// line and its *Rescan* button, a separator, then the cards), each about 150 points tall,
    /// and the whole card is the control — so a point in the second one is enough. Named as
    /// "the card that is not the document already open", because that is the card whose click
    /// has a visible result.
    fn second_card(ctx: &Context) -> Pos2 {
        let area = ctx
            .memory(|m| m.area_rect(egui::Id::new("Levels")))
            .expect("the shelf is up");
        Pos2::new(area.left() + 60.0, area.top() + 230.0)
    }

    /// **New clears the level — and every view that was drawing the old one.**
    ///
    /// Two facts, and the second is the bug this pins: pressing New inside a playtest swapped
    /// the document but left the session (and so the canvas, and its clock) on the level that
    /// had just been thrown away, so the button looked like it did nothing. The same was true
    /// of a run, whose population keeps its own copy of the terrain it was started on.
    #[test]
    fn the_new_button_clears_the_level_and_every_view_of_it() {
        let (ctx, root, mut app) = window("new-button");

        // A clean document: one press is the whole interaction.
        let before = app.editor.level.content_hash();
        let at = centre_of(&app, "New");
        click(&ctx, &mut app, at);
        assert_ne!(
            app.editor.level.content_hash(),
            before,
            "New left the old level on the canvas"
        );
        assert_eq!(app.editor.path, None, "a new document has no file yet");
        assert_eq!((app.editor.level.width, app.editor.level.height), (60, 15));
        assert_eq!(app.mode, Mode::Edit);

        // In a playtest: the session, the mode and the canvas all move to the new document.
        let (ctx, _root, mut app) = window("new-button-play");
        app.enter_play();
        assert!(app.session.is_some(), "the playtest has a session");
        let at = centre_of(&app, "New");
        click(&ctx, &mut app, at);
        assert_eq!(app.mode, Mode::Edit, "New left the window in the playtest");
        assert!(
            app.session.is_none(),
            "the playtest session outlived the level it was playing"
        );
        assert!(
            app.replay.is_none() && app.lab.is_none(),
            "a run or a replay survived the document it was about"
        );
        assert_eq!(
            app.viewed_level().content_hash(),
            app.editor.level.content_hash(),
            "the canvas is still drawing the old level"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A card on the shelf opens its level**, and a dirty document is asked about first.
    ///
    /// The report was that the shelf "shows the levels but does not let me load them". Both
    /// halves are pinned here: a click on a card really does replace the document, and the
    /// press that does *not* is the one a dirty document refused — which now says so on a card
    /// under the dock instead of only in the status line, and which the second press (or the
    /// card's own button) answers.
    #[test]
    fn a_card_on_the_shelf_opens_its_level_and_a_dirty_document_is_asked_first() {
        let (ctx, root, mut app) = window("shelf-card");
        let levels_at = centre_of(&app, "Levels");
        click(&ctx, &mut app, levels_at);
        assert!(app.shelf_open, "the Levels button did not open the shelf");
        let card = second_card(&ctx);
        click(&ctx, &mut app, card);
        assert!(
            app.editor
                .path
                .as_ref()
                .is_some_and(|p| p.ends_with("l2-first-gap.json")),
            "a click on the second card opened {:?}",
            app.editor.path
        );
        assert_eq!(app.mode, Mode::Edit);

        // The same click on a document with unsaved edits is a question, not a no-op: the
        // document stays, the notice is up, and the press *again* is the answer.
        let (ctx, _root2, mut app) = window("shelf-card-dirty");
        app.editor.dirty = true;
        let levels_at = centre_of(&app, "Levels");
        click(&ctx, &mut app, levels_at);
        let card = second_card(&ctx);
        click(&ctx, &mut app, card);
        assert!(
            app.editor
                .path
                .as_ref()
                .is_some_and(|p| p.ends_with("l1-flat-run.json")),
            "a refused press replaced the document anyway: {:?}",
            app.editor.path
        );
        assert!(
            app.editor.message.contains("l2-first-gap.json"),
            "the refusal does not name the level that was asked for: {:?}",
            app.editor.message
        );
        assert!(
            ctx.memory(|m| m.area_rect(egui::Id::new("discard-notice")))
                .is_some(),
            "the refusal was a whisper: no notice was drawn"
        );
        assert_eq!(
            app.discard_wanted,
            Some(Replace::Open(app.levels_dir.join("l2-first-gap.json")))
        );

        click(&ctx, &mut app, card);
        assert!(
            app.editor
                .path
                .as_ref()
                .is_some_and(|p| p.ends_with("l2-first-gap.json")),
            "the second press did not open the level: {:?}",
            app.editor.path
        );
        assert!(
            app.discard_wanted.is_none(),
            "the question outlived its answer"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The notice's own **Discard** button performs the replacement that was asked for.
    #[test]
    fn the_discard_buttons_answer_the_question_they_are_asked() {
        let (ctx, root, mut app) = window("discard-button");
        app.editor.dirty = true;
        let level = app.levels_dir.join("l2-first-gap.json");
        app.replace(Replace::Open(level.clone()));
        assert!(app.discard_wanted.is_some(), "no question was asked");
        // Two frames: an anchored area is placed from its own remembered size, so the first
        // frame it is up is drawn wherever the cursor was.
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let notice = ctx
            .memory(|m| m.area_rect(egui::Id::new("discard-notice")))
            .expect("the notice is drawn");
        // The first plate on the notice's bottom row is *Discard*.
        click(
            &ctx,
            &mut app,
            Pos2::new(notice.left() + 34.0, notice.bottom() - 26.0),
        );
        assert_eq!(
            app.editor.path.as_deref(),
            Some(level.as_path()),
            "Discard did not perform the replacement it was asked about"
        );
        assert!(app.discard_wanted.is_none());

        // And Keep editing leaves the document exactly where it was.
        let (ctx, _root2, mut app) = window("keep-button");
        app.editor.dirty = true;
        let before = app.editor.level.content_hash();
        app.replace(Replace::Open(app.levels_dir.join("l2-first-gap.json")));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let notice = ctx
            .memory(|m| m.area_rect(egui::Id::new("discard-notice")))
            .expect("the notice is drawn");
        click(
            &ctx,
            &mut app,
            Pos2::new(notice.left() + 130.0, notice.bottom() - 26.0),
        );
        assert_eq!(
            app.editor.level.content_hash(),
            before,
            "Keep editing threw the work away"
        );
        assert!(app.discard_wanted.is_none(), "the question stayed up");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **The dock's tools do not move when the document becomes dirty.**
    ///
    /// The plaque is as wide as its own words, and its second line used to grow by
    /// `● … — not on disk` at the first stroke, shoving every tool to its right sideways and
    /// pulling them back on the save. Two states of one sign have to be one width.
    #[test]
    fn the_dock_does_not_move_when_the_document_becomes_dirty() {
        let (ctx, root, mut app) = window("dock-jump");
        let clean = app.tool_rects.clone();
        app.editor.dirty = true;
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            clean, app.tool_rects,
            "the first edit moved the tools in the dock"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The discard guard asks twice, and an edit between the two presses starts the question over.
    ///
    /// The bug this pins lost work: **New** and any click on a level card replaced the document
    /// unconditionally, and the only sign that anything was pending was a dot in a plaque nobody
    /// was looking at (AUDIT.md B1). Three facts make the rule: a clean document is never asked
    /// about, a dirty one is refused once, and the *same* content pressed again goes through —
    /// while an edit in between re-arms it, because that is what looking at the level and
    /// touching it produces.
    #[test]
    fn the_discard_guard_asks_twice_and_an_edit_between_the_asks_re_arms_it() {
        // Clean: nothing to lose, no question.
        assert_eq!(discard_guard(false, None, 7), (false, None));
        // Dirty: the first press only warns, and remembers *what* it warned about.
        let (refused, armed) = discard_guard(true, None, 7);
        assert!(refused, "the first press on a dirty document is refused");
        assert_eq!(armed, Some(7));
        // The second press on the same content is the answer to the question.
        assert_eq!(discard_guard(true, armed, 7), (false, None));
        // An edit between the two presses makes it a different document, so it is asked again.
        let (refused, armed) = discard_guard(true, armed, 8);
        assert!(
            refused,
            "a press after an edit must warn again, not discard the newer version"
        );
        assert_eq!(armed, Some(8));
        // And a document that was saved (so no longer dirty) is never guarded, whatever was armed
        // before it: the guard cannot outlive the work it was protecting.
        assert_eq!(discard_guard(false, Some(8), 8), (false, None));
    }

    fn a_find_named(name: &str) -> evo::Discovery {
        evo::Discovery {
            name: name.to_string(),
            line: "a line about the find".to_string(),
            kind: evo::DiscoveryKind::Technique,
            signature: format!("technique:{name}"),
            level_name: "L1-flat-run".to_string(),
            level_hash: "test".to_string(),
            generation: 3,
            genome_id: 7,
            frames: 200,
            seed: 1,
            skip_percent: None,
            replay: Vec::new(),
        }
    }

    #[test]
    fn a_find_puts_a_card_up_slows_the_run_and_gives_the_speed_back() {
        let ctx = egui::Context::default();
        let root = scratch("discovery-card");
        let mut app = app_on(&ctx, &root.join("levels/l1-flat-run.json"));
        app.view.speed = 16;
        app.show_discovery(a_find_named("Corpse Bridge"));
        assert_eq!(
            app.discovery.as_ref().map(|(d, _)| d.name.as_str()),
            Some("Corpse Bridge")
        );
        assert_eq!(
            app.view.speed,
            crate::lab::SPEEDS[0],
            "the moment is slow motion"
        );
        // Drawing the card is part of a real frame, so this is also the check that it does not
        // borrow or panic on the way to the screen.
        frame(&ctx, &mut app, Vec::new());
        app.tick_discovery(DISCOVERY_SECONDS + 0.1);
        assert!(app.discovery.is_none(), "the card expires on its own");
        assert_eq!(app.view.speed, 16, "and the previous speed comes back");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **Continue from a saved champion, through the window's own key.**
    ///
    /// The gap this closes: a saved champion outlived the process, but only the terminal could
    /// read it back into a population, so restarting the game meant restarting the search from
    /// nothing. This writes a champion for the level, opens the window on it, presses `C`, and
    /// checks the run that comes back is seeded from the file — the origin line, the seed, and
    /// the path the panel offers are its visible facts.
    #[test]
    fn c_continues_the_run_from_the_champion_saved_for_the_level() {
        let ctx = Context::default();
        let root = scratch("continue-champion");
        let mut app = app_on(&ctx, &root.join("levels/l1-flat-run.json"));
        app.options.pop_size = 8;

        // A champion for this level, written the way `train-cli train` writes one.
        let level = app.editor.level.clone();
        let genome = evo::Genome::zeros(12_345, evo::default_shape());
        let config = GaConfig {
            pop_size: 8,
            ..GaConfig::default()
        };
        let result = evo::run_episode(&level, &genome, &config.eval_config(), 7);
        let champion = evo::ChampionFile::new(&genome, &result, &level, 4, &config);
        let path = app.options.champions.join("l1-flat-run.json");
        champion.save(&path).expect("the champion writes");

        app.enter_evolve();
        assert!(app.lab.is_some(), "the evolution view starts a run");
        assert!(
            app.lab.as_ref().unwrap().champion_path().is_file(),
            "the panel offers the champion this level saved"
        );

        let key = egui::Event::Key {
            key: egui::Key::C,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        frame(&ctx, &mut app, vec![key]);

        let lab = app.lab.as_ref().expect("the run is still there");
        let origin = lab
            .origin()
            .expect("the continued run says where it started");
        assert!(origin.contains("seeded from gen 4"), "{origin}");
        assert!(origin.contains("id 12345"), "{origin}");
        let note = app.note.as_ref().expect("the status bar says it too");
        assert!(note.0.contains("seeded from"), "{}", note.0);
        assert!(note.0.contains("(seed "), "{}", note.0);
        assert_eq!(lab.stem(), "l1-flat-run");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_codex_page_reads_the_documents_journal() {
        let ctx = egui::Context::default();
        let root = scratch("codex-page");
        let mut app = app_on(&ctx, &root.join("levels/l1-flat-run.json"));
        let mut journal = evo::Codex::empty(&app.editor.level);
        journal.record(a_find_named("Corpse Bridge"));
        let path = evo::Codex::path_for(&app.options.champions, "l1-flat-run");
        journal.save(&path).expect("the codex writes");
        app.mode = Mode::Evolve;
        let key = egui::Event::Key {
            key: egui::Key::K,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        frame(&ctx, &mut app, vec![key]);
        assert!(app.codex_open, "K opens the page");
        assert_eq!(
            app.codex_view.as_ref().map(|codex| codex.len()),
            Some(1),
            "the journal on disk is what the page shows"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The title bar names the document first, then the mode, then the game.
    ///
    /// The bug this pins: the title was the constant `"one two hundred"` for the life of
    /// the window, so two open levels were indistinguishable in a taskbar — and the part a
    /// narrow taskbar keeps is the left-hand end, which was the part that never changed
    /// (AUDIT.md B5).
    #[test]
    fn the_window_title_leads_with_the_document_and_names_the_mode() {
        let edit = window_title_for(Mode::Edit, "l1-flat-run.json");
        assert!(edit.starts_with("l1-flat-run.json"), "{edit}");
        assert!(edit.ends_with("one two hundred"), "{edit}");
        // Every mode says which of the four things this window is doing, and no two agree: a
        // window in play and a window in evolve have to be told apart in a taskbar too.
        let titles: Vec<String> = Mode::ALL
            .iter()
            .map(|mode| window_title_for(*mode, "l1-flat-run.json"))
            .collect();
        for (i, a) in titles.iter().enumerate() {
            for b in &titles[i + 1..] {
                assert_ne!(a, b, "two modes wear the same window title");
            }
        }
        assert!(titles[1].contains("playtest"), "{}", titles[1]);
        assert!(titles[2].contains("evolve"), "{}", titles[2]);
        assert!(titles[3].contains("replay"), "{}", titles[3]);
    }
}
