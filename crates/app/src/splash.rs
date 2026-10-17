//! The splash screen: the window's first frame, and what it says about itself.
//!
//! Without one the shell opens straight into the editor on a level, which is the right first
//! thing for a person who already knows what the four tabs are and a cold start for everyone
//! else. So the first frame is this: the game's own picture — the same sky gradient, the same
//! parallax hills and drifting clouds, the same grass, trees and butterflies, drawn by the
//! same functions — with a title over it, and the whole thing is **running**. An agent on a
//! small level built for the occasion runs to the flag, jumps one gap and climbs one step on
//! the way, and the scene holds on the waving flag for a moment before starting again, so
//! what a person watches while deciding to press a key is the game being played rather than
//! a picture of it.
//!
//! What this module owns is the *screen*: the level, the little episode, the camera pull-back
//! and the words. The world is drawn by [`crate::view`], and the episode is a
//! [`crate::play::Session`] — the playtest's own session, with its own fixed timestep and its
//! own animation — driven by a script instead of a keyboard. That is deliberate: a splash
//! drawn with a second renderer, or played by a second simulation, is a picture of the game
//! that can drift away from the game, and every art or physics change would have to be made
//! twice.
//!
//! **It gets out of the way, and it leads where its sign says.** Any key or any click after
//! the first [`ARMED`] seconds dismisses it ([`Action`]): `P` opens the playtest and `E`
//! starts a run, and every other key — or a click — opens the editor. The keys printed on the
//! front door therefore *do* what they say, rather than being hints for a screen that would
//! consume the press and drop the viewer into the editor anyway. The window is playable from
//! the frame after that, and nothing about the splash is reachable again except by restarting
//! the app. A stray event from the window manager — a focus click, a key repeat — must not eat
//! the title before a person has seen it, which is what [`ARMED`] is for.
//!
//! **Whose run it is.** If the run has a saved champion for the level this window opened on,
//! that is what plays: the genome out of the champion file, on the terrain it was trained on,
//! deciding every `action_repeat` ticks through the same simulation the trainer used (see
//! [`ChampionRun`]) — so the picture is the episode the file records rather than an
//! illustration of one, and the credit line under it says *whose* (see [`ChampionFile`]).
//! Otherwise — no champion for this level, a file from another build, or a best-ever agent
//! that never reached the flag — the screen is the little level and the script, which is what
//! it always showed, and which is deliberately as good as the champion path rather than a
//! consolation prize.

use std::path::Path;

use egui::{
    Align2, Color32, Context, CornerRadius, FontId, Painter, Pos2, Rect, Stroke, Vec2,
};
use evo::{ActScratch, ChampionFile, Genome};
use sim::{AgentAction, Level, Sim, Tile};

use crate::atlas::Atlas;
use crate::play::{self, Keys};
use crate::theme;
use crate::view::{self, Camera, RenderOpts, Sheet, Sprites};

/// The lockup. Two lines rather than one, because a logo is stacked: nine letters across a
/// window this wide at a size worth reading draw a banner, and two lines draw a *title*.
///
/// The eyebrow is the genre line a box would carry above its name; the subtitle is the hook.
/// The words are the window's own, so the taskbar and the splash agree.
const TITLE_LINES: [&str; 2] = ["EVOLUTION", "PLATFORMER"];
const EYEBROW: &str = "AN ARTIFICIAL-LIFE PLATFORMER";
const SUBTITLE: &str = "a population learns to beat the level you draw";
/// The menu's own help line: how to move between the doors, and how to take one.
///
/// "Up / down" rather than arrow glyphs, which the window's font does not have: a hint that
/// renders as two empty boxes is worse than one that spells the key out.
const MENU_HINT: &str = "UP / DOWN   SELECT";

/// What a person asked for by dismissing the screen.
///
/// The splash is a doorway, and a doorway should lead where the sign says. Every key
/// opens the editor except the two the hint names, which enter the activity they name
/// directly — so the keys printed on the front door do what they say rather than dropping
/// the viewer into the editor to press the same key again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the editor.
    Edit,
    /// Open the playtest on the level the window is on.
    Play,
    /// Open the evolution view and start training.
    Evolve,
}

/// Seconds for the camera to go from close on the runner to the whole level.
///
/// The one piece of scene direction on this screen, and it is there to answer "what is this
/// game?" in the first second: a camera that starts on the runner and pulls back to the level
/// says *a person, in a place* in a way a wide shot from the first frame does not.
const PULLBACK: f32 = 1.8;
/// When the title starts appearing, how long each letter takes, and how far it rises.
///
/// Per letter rather than per line: the reveal is the animation, and a title that assembles
/// itself is one of the few things that reads as *designed* at this size.
const TITLE_AT: f32 = 0.30;
const LETTER_IN: f32 = 0.045;
const LETTER_FADE: f32 = 0.45;
const LETTER_RISE: f32 = 10.0;
/// When the menu arrives. After the title has finished assembling: a screen that is still
/// revealing its name while it offers its doors is still assembling itself at the moment it
/// invites the viewer to choose.
const HINT_AT: f32 = 1.9;
/// How long the finished episode is held before the little run starts over.
const HOLD: f32 = 1.15;
/// How long a dismissing event is ignored at the very start.
const ARMED: f32 = 0.35;
/// How far above the middle of the window the level is drawn, as a fraction of the window's
/// height.
///
/// A level fitted to the middle of a window this wide leaves two even bands of sky, and the
/// screen it is behind needs them unequal: the title wants sky above and below it, and the
/// invitation needs a band of its own under the ground rather than the lawn's brown. Pushing
/// the world up is one number and it is what turns the layer under the ground from a state
/// the fit happened to land on into a *floor for the text*.
const RAISED: f32 = 0.17;
/// Where the ground sits in the window when a champion is playing, as a fraction of the
/// window's height.
///
/// Low enough that the lawn, the props and the agent are in the lower third of a window the
/// sky and its parallax fill above them, high enough to leave a band of its own under the
/// ground for the invitation. See [`Splash::target`].
const GROUND_LINE: f32 = 0.60;
/// The seed of the splash's episode, so the splash is the same splash every launch.
const SEED: u64 = 4;

/// The game's own palette, and it is the *window's* palette rather than this screen's.
///
/// These six were local colours that happened to sit close to the panels' — close enough to
/// look deliberate, far enough that an edit to one side never reached the other. The window
/// is one palette applied once ([`crate::theme::apply`]), so the splash reads it the way
/// every other surface does: the title takes the accent, its shadow the frame's own darkest
/// surface, and the small print the dim text. Two of the six are *replaced* rather than
/// renamed — the title stopped being a cream of its own and became the accent, because a
/// screen with one thing on it wants one ink for the thing it is about.
const TITLE_INK: Color32 = theme::ACCENT;
/// The top edge of the title's bevel: the gold, lifted towards the sun.
const TITLE_LIGHT: Color32 = Color32::from_rgb(255, 246, 208);
const TITLE_SHADE: Color32 = theme::SHELL;
/// The eyebrow over the title, and the rule under it: the metal, not the sun.
const EYEBROW_INK: Color32 = theme::GOLD_EDGE;
/// The tagline is read on the *sky*, so it is the window's dark screen ink rather than its
/// cream: a pale line over a pale-blue sky was the one piece of type on this screen that had
/// to be read twice. Dark navy on sky is 8.7:1 and belongs to the same palette.
const SUBTITLE_INK: Color32 = Color32::from_rgb(24, 34, 46);
const RULE_INK: Color32 = theme::GOLD_EDGE;
/// Small print on the stage: the *screen* ink, because the band under the level is dark.
/// The old footer used the chrome's dim ink and was 1.3:1 on it — a caption nobody could read.
const FOOT_INK: Color32 = theme::SCREEN_INK_DIM;
/// The stage: near-black, blue-shifted, so the gold on it reads warm.
const STAGE_INK: Color32 = Color32::from_rgb(54, 38, 24);
/// The menu's face: the window's warm white, because the box is a piece of the game's own menu
/// set on top of the picture rather than a hole cut into it.
const PANEL_FACE: Color32 = theme::CARD;

/// When the menu is up: a beat after the invitation starts breathing.
///
/// It arrives with the caption rather than at the first frame, because a doorway offered
/// before the screen has said what it is a doorway *to* is a button nobody reads. A beat
/// and not a crossfade: a row is a hit target as well as a pixel, and a target that is
/// half transparent on the frame it becomes clickable is a target somebody clicks twice.
const DOORS_LAG: f32 = 0.25;

/// The panel the menu is drawn in: a framed box in the band under the level.
///
/// A fixed width rather than a fraction of the window, because a menu is an *object*: three
/// lines six hundred points wide on a cinema screen are a toolbar, and the same lines on a
/// small window are a wall. 520 is wide enough for a cursor, a name, a description and a key
/// on one line, and narrow enough to read as a box.
const MENU_W: f32 = 520.0;
/// One option line's height. A line of a console menu is a line of type with a cursor beside
/// it, not a card: 40 points is a name, its key and the air a blinking cursor needs.
const LINE_H: f32 = 40.0;
/// The panel's padding, and the description strip under the options.
///
/// The strip is what a console menu does instead of a sentence on every line: one hint about
/// the thing the cursor is on, in the bottom of the box.
const PANEL_PAD: f32 = 14.0;
const DESC_H: f32 = 34.0;
/// Padding from the window's bottom edge, for the box, the prompt and the key line.
const MENU_BOTTOM_PAD: f32 = 96.0;

/// One door: what it is called, what it does, the sentence the box shows while the cursor is
/// on it, and the key that also takes it.
///
/// Edit first because it is what most launches want — a splash is usually opened on the way
/// to drawing something — and then the two activities the plan lists: play it yourself, then
/// teach the population. The key chips are the same keys [`show`] reads, so a person who
/// would rather use the keyboard can read the key off the row instead of guessing at it, and
/// the pictures are the mode switch's own sprites, so the front door and the dock agree
/// about what the four activities look like.
struct Door {
    name: &'static str,
    action: Action,
    blurb: &'static str,
    key: &'static str,
}

const DOORS: [Door; 3] = [
    Door {
        name: "EDIT",
        action: Action::Edit,
        blurb: "draw a level, tile by tile",
        key: "ANY KEY",
    },
    Door {
        name: "PLAY",
        action: Action::Play,
        blurb: "take the keyboard and play it",
        key: "P",
    },
    Door {
        name: "EVOLVE",
        action: Action::Evolve,
        blurb: "train a population to beat it",
        key: "E",
    },
];

/// The splash, and everything it is doing while it is up.
pub struct Splash {
    /// The level in the picture. The splash's own for a scripted run, the level the window
    /// opened on for a champion's — a champion is only ever played where it was trained.
    level: Level,
    /// The episode: the playtest's own session, which owns the timestep and the animation,
    /// driven either by [`scripted`] or by the champion's network.
    session: play::Session,
    /// Who is playing it.
    policy: Policy,
    /// Seconds since the splash opened. The only clock: the reveal, the pull-back and the
    /// breathing invitation are all functions of it, so two frames at the same instant are
    /// the same frame.
    clock: f32,
    /// Seconds left of the hold after the episode ends, and then it starts again.
    hold: f32,
    /// Which door the keyboard is on: an index into [`DOORS`].
    ///
    /// One selection with two ways to move it. The pointer sets it on hover, so the gold bar
    /// is under the cursor, and <up>/<down> set it, so the bar is where the next Enter will
    /// go. Two cursors that could disagree would be a menu that shows one thing and does
    /// another.
    selected: usize,
}

/// What is running the level in the picture.
///
/// Two policies and no more, and the difference between them is the whole of this screen's
/// second act: one is a demonstration, the other is the run's own answer.
///
/// One champion weighs a couple of hundred bytes more than no champion, and this enum is
/// sized for the larger arm. Boxing it would trade that for an indirection on every tick of
/// the run, for a value there is exactly one of in a window that lives for a few seconds.
#[allow(clippy::large_enum_variant)]
enum Policy {
    /// The four-line lookahead script, on the level built for the occasion.
    Script,
    /// The run's own saved champion, on the level it beat.
    Champion(ChampionRun),
}

/// A saved champion, being played back.
///
/// The genome is the whole of it. The episode is not replayed from a recording of positions
/// — there is no recording — it is *re-decided*: the sim is stepped forward exactly as the
/// trainer stepped it, and the network is queried on the ticks the trainer queried it. That
/// is the only way the picture can be the game rather than an animation of it, and it is why
/// this type holds a scratch pad and an action rather than a list of frames.
///
/// The cadence is [`evo::fitness::EpisodeRunner`]'s, tick for tick: a decision whenever the
/// sim's own frame counter divides by `action_repeat`, held in between. It has to be — a
/// champion watched at another cadence is not the episode that earned its number, and the
/// test that pins this compares the run to the end against the frames, progress and outcome
/// the file records.
struct ChampionRun {
    genome: Genome,
    /// The forward pass' scratch pad, the same one [`Genome::act`] takes everywhere else.
    scratch: ActScratch,
    action: AgentAction,
    /// Physics ticks between decisions, as the champion was trained.
    repeat: u32,
    /// The episode's seed out of the file: half of the episode, and the half a champion
    /// cannot be re-run without.
    seed: u64,
    /// Whose run this is, for the line under the words.
    credit: String,
}

impl ChampionRun {
    /// The action for the tick the sim is about to run.
    fn decide(&mut self, sim: &Sim) -> AgentAction {
        if sim.stats().frames % self.repeat == 0 {
            self.action = self.genome.act(&sim.observation(), &mut self.scratch);
        }
        self.action
    }
}

impl Default for Splash {
    fn default() -> Self {
        Self::scripted()
    }
}

impl Splash {
    /// The title screen for a window opening on `level`, whose run writes to `champions`.
    ///
    /// A champion is preferred and a champion is *checked*: see [`champion_run`], which
    /// refuses a file from another build or another level and refuses a best-ever agent that
    /// never reached the flag, falling back to [`Splash::scripted`]. The fallback is not a
    /// degraded mode — it is the screen every launch showed before there was a champion to
    /// show, and it is the screen a fresh checkout gets.
    pub fn open(level: &Level, champions: &Path, level_path: Option<&Path>) -> Splash {
        let Some(run) = champion_run(level, champions, level_path) else {
            return Splash::scripted();
        };
        // The champion's level *is* this level: `check_compat` refuses a file whose level
        // hash is not this document's, so the picture and the editor are looking at the same
        // terrain, which is the point of playing the run's own agent at the front door.
        let session = play::Session::new(level, run.seed);
        Splash {
            level: level.clone(),
            session,
            policy: Policy::Champion(run),
            clock: 0.0,
            hold: 0.0,
            selected: 0,
        }
    }

    /// The screen with no champion to play: the splash's own level, and its script.
    pub fn scripted() -> Splash {
        let level = level();
        let session = play::Session::new(&level, SEED);
        Splash {
            level,
            session,
            policy: Policy::Script,
            clock: 0.0,
            hold: 0.0,
            selected: 0,
        }
    }

    /// Seconds the splash has been up.
    ///
    /// The app has no use for this — the screen is a function of its own clock and nothing
    /// outside asks it what time it is — so it exists for the tests that drive the screen to
    /// a particular instant rather than counting frames to get there.
    #[cfg(test)]
    pub fn clock(&self) -> f32 {
        self.clock
    }

    /// Skip the reveal and the pull-back, for a still frame.
    ///
    /// A screenshot cannot show a reveal — the capture is of one frame, taken within a frame
    /// or two of the window opening, when the title is a fifth of the way in and the camera
    /// is still on the runner. `--splash --screenshot` therefore asks for the splash at the
    /// end of its opening move instead of the beginning, which is the frame a person would
    /// photograph if they were showing somebody the screen. The episode is advanced as well,
    /// so the runner in that frame is running rather than standing on the spawn tile.
    pub fn skip_opening(&mut self) {
        let skip = HINT_AT + 0.9;
        let steps = (skip / sim::DT).ceil() as u32;
        for _ in 0..steps {
            self.advance(sim::DT);
        }
    }

    /// Advance the clock, and the episode with it.
    ///
    /// The episode is stepped with the same fixed timestep the playtest uses, so the run cycle
    /// and the dust are the animation the game has rather than an impression of it. When the
    /// episode ends the runner is handed no keys for [`HOLD`] seconds — it settles to a stand
    /// while the flag waves, which is the beat that makes the loop read as *finished* rather
    /// than as cut off — and then the whole little run starts again from the spawn tile.
    pub fn advance(&mut self, dt: f32) {
        self.clock += dt;
        if self.hold > 0.0 {
            // The hold: the runner is handed nothing, so it settles to a stand while the
            // clock keeps the flag and the grass moving. Then the episode starts over.
            self.hold -= dt;
            if self.hold <= 0.0 {
                self.session.restart(&self.level);
                // A champion's network may carry state between decisions (`NetShape::feedback`),
                // so the restart has to forget it: the second loop of the title screen must be
                // the episode it was the first time, not one that begins holding the tail of
                // the previous run's thoughts. Inert for the script, and for a memoryless net.
                if let Policy::Champion(run) = &mut self.policy {
                    run.scratch.reset_context();
                }
            }
            self.session.advance_actions(dt, |_| AgentAction::Walk);
            return;
        }
        // One arm per policy, and the closure *is* the policy: a function from the
        // simulation to the action for the tick it is about to run. The script reads the
        // level; the champion reads its own network. Neither of them is allowed to read the
        // frame, which is what makes the champion's episode the episode that was scored.
        match &mut self.policy {
            Policy::Script => {
                // The script is written in keys, because it was written for a person's
                // controller before there was a champion to share it with; the action is the
                // same thing one conversion later.
                let level = &self.level;
                self.session
                    .advance_actions(dt, |sim| scripted(level, sim).action());
            }
            Policy::Champion(run) => {
                self.session.advance_actions(dt, |sim| run.decide(sim));
            }
        }
        if self.session.sim().outcome().is_finished() {
            self.hold = HOLD;
        }
    }

    /// The line that says whose run this is, when it is somebody's.
    fn credit(&self) -> Option<&str> {
        match &self.policy {
            Policy::Champion(run) => Some(run.credit.as_str()),
            Policy::Script => None,
        }
    }

    /// Draw the screen, and say which way the viewer asked to go — or `None` if they have not
    /// asked yet.
    ///
    /// The picture first, then the stage, then the words and the menu: the level is the screen's
    /// *subject*, and everything drawn over it is furniture arranged around it.
    ///
    /// Any key press or any click counts, once the screen has been up for [`ARMED`] seconds: a
    /// splash is a doorway, and a doorway that argues is a wall. Two keys open the activity they
    /// name ([`Action::Play`], [`Action::Evolve`]); everything else opens the editor, which is
    /// what most launches want. **The arrows and enter are the exception**: a menu that shows a
    /// selection has to be drivable by the keys that move a selection, so up/down move the gold
    /// bar and enter takes it — and the bar is the same one the pointer sets, so the two ways in
    /// cannot disagree about which door is chosen.
    ///
    /// The one press that is not a decision is a press on the rows themselves ([`menu`]): those
    /// are widgets, they answer on the release, and a press that took the screen down on its way
    /// to them would put the editor under a click aimed at Play. So the menu's own presses are
    /// passed through to the row and every other press — the sky, the title, the level — is still
    /// "just start".
    pub fn show(&mut self, ctx: &Context, atlas: &Atlas, sprites: &Sprites) -> Option<Action> {
        let rect = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let camera = self.camera(rect);
        let sheet = Sheet::new(&painter, atlas, sprites, &camera, rect);
        // The world, exactly as the play view draws it: the sky and its parallax, then the
        // level and the agent in it with its dust. The level's own bounds box is the one
        // thing the play view draws that belongs to a document rather than to a picture, so
        // it is off.
        view::draw_background(sheet, &self.level, view::Backdrop::Sky, self.clock);
        let opts = RenderOpts {
            bounds: false,
            ..RenderOpts::playing()
        };
        view::draw_sim(
            sheet,
            self.session.sim(),
            opts,
            self.clock,
            self.session.anim(),
        );

        // The band under the level's own floor is the stage. Measured from the floor rather
        // than from a fraction of the window, because a champion is played on whatever level
        // the window opened on and this is the one line the composition can trust.
        let floor = sheet
            .camera
            .to_screen(0.0, (self.level.height - 1) as f32, rect)
            .y;
        stage(&painter, rect, floor);
        lockup(&painter, ctx, rect, self.clock);

        let panel = menu_panel(rect);
        let from_menu = menu(
            ctx,
            &painter,
            rect,
            panel,
            &mut self.selected,
            self.clock,
        );
        footer(&painter, rect, self.credit());

        if self.clock < ARMED {
            return None;
        }
        // What the keyboard asked for this frame: no decision, a new selection, or a door.
        enum Pressed {
            None,
            Move(usize),
            Go(Action),
        }
        let count = DOORS.len();
        let pressed = ctx.input(|i| {
            for event in &i.events {
                match event {
                    // A press, not a release: letting go of a key is not a decision.
                    egui::Event::Key {
                        key,
                        pressed: true,
                        ..
                    } => {
                        return match key {
                            egui::Key::ArrowDown | egui::Key::ArrowRight | egui::Key::Tab => {
                                Pressed::Move((self.selected + 1) % count)
                            }
                            egui::Key::ArrowUp | egui::Key::ArrowLeft => {
                                Pressed::Move((self.selected + count - 1) % count)
                            }
                            egui::Key::Enter | egui::Key::Space => {
                                Pressed::Go(DOORS[self.selected].action)
                            }
                            egui::Key::P => Pressed::Go(Action::Play),
                            egui::Key::E => Pressed::Go(Action::Evolve),
                            _ => Pressed::Go(Action::Edit),
                        };
                    }
                    // A press *on a row* is not a decision either, and this is the one press
                    // the rule below has to step aside for: a row answers on the release, so
                    // taking the press here would start the editor out from under the pointer
                    // and the click would arrive somewhere it was not aimed.
                    egui::Event::PointerButton {
                        pressed: true,
                        pos, ..
                    } if menu_is_up(self.clock) && panel.expand(10.0).contains(*pos) => {
                        continue;
                    }
                    egui::Event::PointerButton {
                        pressed: true, ..
                    } => return Pressed::Go(Action::Edit),
                    _ => {}
                }
            }
            Pressed::None
        });
        match pressed {
            Pressed::Move(next) => {
                self.selected = next;
                None
            }
            Pressed::Go(action) => Some(action),
            Pressed::None => from_menu,
        }
    }

    /// Where the world is drawn from: close on the runner, easing out to where the run goes.
    fn camera(&self, rect: Rect) -> Camera {
        let target = self.target(rect);
        let p = self.session.sim().player();
        let start = Camera {
            centre: Vec2::new(p.x + p.w * 0.5, p.y + p.h * 0.5),
            zoom: target.zoom * 1.6,
        };
        let t = (self.clock / PULLBACK).clamp(0.0, 1.0);
        // Ease out cubic: fast at first, which is what makes the first second of the screen
        // feel like a camera move rather than like a zoom.
        let ease = 1.0 - (1.0 - t).powi(3);
        Camera {
            centre: start.centre + (target.centre - start.centre) * ease,
            zoom: start.zoom + (target.zoom - start.zoom) * ease,
        }
    }

    /// Where the opening move is heading.
    ///
    /// The demo level is small enough to show whole, and showing it whole is the point of the
    /// pull-back — *a person, in a place*. The window's own level is not: L1 is sixty tiles of
    /// ground, five times the window's width, so fitted it is a stripe of lawn with the agent a
    /// speck on it, and a splash whose subject is a speck has no subject. A champion is
    /// therefore **followed** rather than fitted, at the window's own follow zoom
    /// ([`crate::app::FOLLOW_ZOOM`], the zoom the replay view follows its champion at) and
    /// clamped the way that view clamps: the level on screen at the size the game draws it,
    /// with the run coming to the viewer instead of being looked at from orbit.
    ///
    /// The champion's frame *follows* the agent horizontally and *pins* the floor vertically,
    /// and the two are separate on purpose: following means looking at the agent, and the one
    /// thing a camera must never do while looking at a platformer agent is bob up and down
    /// with its jumps. The floor is what the picture is composed around — the level is fifteen
    /// rows tall and the window is eight hundred points, so where the ground sits is what
    /// decides whether the screen reads as a platformer or as a stripe of lawn with a lot of
    /// nothing under it — and pinning it leaves the band under the ground for the words.
    ///
    /// `max` with the fit rather than the follow zoom alone, so the same code shows a *small*
    /// level whole: a level that already fits inside the follow zoom is not worth tracking.
    fn target(&self, rect: Rect) -> Camera {
        match &self.policy {
            Policy::Script => framed(&self.level, rect),
            Policy::Champion(_) => {
                let p = self.session.sim().player();
                let zoom = framed(&self.level, rect).zoom.max(crate::app::FOLLOW_ZOOM);
                let mut camera = Camera {
                    centre: Vec2::new(
                        p.x + p.w * 0.5,
                        self.level.height as f32
                            - (rect.height() * GROUND_LINE - rect.center().y) / zoom,
                    ),
                    zoom,
                };
                camera.clamp_to(&self.level, 2.0);
                camera
            }
        }
    }
}

/// The whole level, as the splash frames it: fitted, then raised for the words.
///
/// The pull-back ends here and the tests read it as the end of the move, which is why it is a
/// function rather than three lines inside [`Splash::camera`]: a camera that eased to *roughly*
/// the frame the screen holds would leave the level hanging off an edge for the rest of the
/// screen's life.
fn framed(level: &Level, rect: Rect) -> Camera {
    let mut fit = Camera::fit(level, rect, 24.0);
    fit.centre.y += rect.height() * RAISED / fit.zoom;
    fit
}

/// The splash's own level: a lawn, one gap, one step, and the flag.
///
/// Built rather than loaded, and the reason is the length of the thing: a splash has about
/// two seconds of a person's attention, so the level is exactly long enough to show the two
/// moves the whole game is about — a jump over a gap and a jump onto a step — and then it is
/// over and the flag is waving. A shipped level would either be too long to finish inside
/// that or too plain to be worth watching.
///
/// Its dressing — the grass, the bushes, the trees, their sway, the butterflies over the lawn
/// — is not placed here at all: the view scatters all of it from the tile coordinates, so the
/// level only has to say where the ground is and the screen dresses itself.
fn level() -> Level {
    // Ten rows of sky over a two-row lawn, and thirty-four tiles long: as short as the two
    // moves need and no taller than a level ever is. The shape is a framing decision — the
    // fit is width-limited on a window this wide, so a longer level would draw every tile
    // smaller and put the runner's head at eight pixels.
    let mut level = Level::starter("splash", 34, 13);
    // The floor is two rows deep, the way a shipped level's is — the lawn, and the dirt under
    // it — which is what lets a gap read as a pit rather than as a stripe of sky.
    for tx in 0..level.width {
        level.set_tile(tx, 11, Tile::Solid);
    }
    // `starter` placed the spawn and the flag on the single row it knows about; with the
    // floor a row higher they move up with it. A goal tile is the *pole* — the lower of the
    // flag's two cells, and the one that has to have ground under it — so it is row 10, the
    // tile standing on the lawn. A level has one flag, so both calls are *moves*: the spawn's
    // and the goal's place in the row are what changes, not how many of them there are.
    level.set_spawn_tile(1, 10);
    level.set_goal_tile(31, 10);
    // A gap three tiles wide and a step one tile high: both the widest and the tallest thing
    // the physics comfortably clears from a full run, so the runner is not relying on luck
    // and a person is not watching a near miss.
    for tx in 12..15 {
        level.set_tile(tx, 11, Tile::Empty);
        level.set_tile(tx, 12, Tile::Empty);
    }
    for tx in 20..26 {
        level.set_tile(tx, 10, Tile::Solid);
    }
    level
}

/// The keys the splash's runner is holding: right, and a jump at the moment one is needed.
///
/// The script is the smallest thing that can play this level, and it is a *lookahead* rather
/// than a clock: the column just past the runner's front foot, and whether the ground is
/// there at the row the feet are on. No ground means the level is about to open into the pit;
/// a solid tile at the row *above* it means a step in the way. Both are jumped from the edge,
/// which is the timing that matters — a held jump carries four tiles at running speed, so a
/// jump taken a tile early lands back inside the pit it was meant to clear (this is the whole
/// of what makes the shipped levels hard, and the splash's level is built to be inside that
/// window rather than at its edge).
///
/// The button is *held*, not tapped, and that is the one thing here that is not an
/// approximation of what a person does: this game's jump is variable-height, so a tap is cut
/// to [`sim::PHYSICS::jump_cut`] of its height and the runner drops into the pit it was
/// jumping. Holding while `vy < 0.0` and letting go at the top of the arc is the full jump —
/// and letting go there is also what stops a landing from being a fresh press, since a jump
/// button still held when the feet touch down is a second jump.
fn scripted(level: &Level, sim: &Sim) -> Keys {
    let p = sim.player();
    // Whole tiles, and the feet row is the row the runner is standing on — a hair below them,
    // so it is the tile under the box rather than the box's own bottom edge.
    let foot = (p.y + p.h + 0.05).floor() as i32;
    let grounded = view::solid_at(level, (p.x + p.w * 0.5).floor() as i32, foot);
    let edge = (p.x + p.w + 0.15).floor() as i32;
    let gap = !view::solid_at(level, edge, foot);
    let step = view::solid_at(level, edge, foot - 1);
    Keys {
        right: true,
        jump: p.vy < 0.0 || (grounded && (gap || step)),
        ..Keys::default()
    }
}

/// The run's champion for this level, if there is one worth playing.
///
/// Three things are checked, and each of them is there because the alternative is a title
/// screen quietly showing the wrong thing:
///
/// * **The file is where the front-ends leave it.** Two names are tried: the level *file's*
///   stem, which is what `train-cli train` and `train-cli watch` name their output after, and
///   the level's *name* slugged, which is what the window's own Save champion writes
///   ([`crate::lab::slug`]). For a shipped level the two agree; for a level whose file name
///   and title differ they do not, and a lookup that knew only one of them would be blind to
///   half the champions on disk.
/// * **It is for this build and this level.** [`evo::champion::replay`] does the checking and
///   the re-running together: another build's version trio, another level's content hash, or a
///   shape this build cannot run are all refused, so a stale champion can never drive today's
///   physics.
/// * **It wins.** The episode is re-run here, once, at startup — about what one generation
///   costs the trainer — and a champion that does not reach the flag is not played. A title
///   screen looping a run's best agent dying is worse than the demo run it replaced, and
///   "best" is best of a population that may never have beaten the level at all.
///
/// Every refusal is silent, on purpose. The file is a build artifact in a directory the
/// window was told to look in, and a title screen that refused to open over a stale one would
/// be a worse bug than the one it would be reporting. The fallback is the demo run, and the
/// tests pin each refusal.
fn champion_run(level: &Level, champions: &Path, level_path: Option<&Path>) -> Option<ChampionRun> {
    let champion = champion_file(level, champions, level_path)?;
    let (genome, result) = evo::champion::replay(level, &champion).ok()?;
    if !result.won() {
        return None;
    }
    Some(ChampionRun {
        genome,
        scratch: ActScratch::new(),
        action: AgentAction::Walk,
        repeat: champion.metadata.action_repeat.max(1),
        seed: champion.metadata.episode_seed,
        credit: format!(
            "the run's champion · generation {} · fitness {:.0}",
            champion.metadata.generation, champion.fitness
        ),
    })
}

/// A champion file for this level, under either of the two names one can have.
///
/// Each candidate is loaded *and validated against the level* — which is what
/// [`ChampionFile::load_for_level`] means — and the first that validates wins, so a stale
/// file under one name cannot hide a good one under the other.
fn champion_file(
    level: &Level,
    champions: &Path,
    level_path: Option<&Path>,
) -> Option<ChampionFile> {
    let mut names = Vec::new();
    if let Some(stem) = level_path.and_then(|path| path.file_stem()) {
        names.push(stem.to_string_lossy().to_string());
    }
    names.push(crate::lab::slug(&level.name));
    names.dedup();
    names.iter().find_map(|name| {
        ChampionFile::load_for_level(champions.join(format!("{name}.json")), level).ok()
    })
}

/// How far one letter of the title has arrived: 0 not yet, 1 fully there.
///
/// A pure function of the clock and the letter's index, so the reveal can be tested without a
/// screen, and so the last letter's arrival is a fact about the timing constants rather than
/// something to eyeball. The index runs across *both* lines, so the second line assembles
/// after the first: a title that arrives as one gesture rather than two.
fn letter_in(clock: f32, index: usize) -> f32 {
    let at = TITLE_AT + index as f32 * LETTER_IN;
    ((clock - at) / LETTER_FADE).clamp(0.0, 1.0)
}

/// How many letters the lockup has, spaces included: the reveal's own clock.
fn title_letters() -> usize {
    TITLE_LINES.iter().map(|line| line.chars().count()).sum()
}

/// The size of the title: one number for both lines, and the biggest type in the window.
///
/// Sized from the window's height rather than fixed, because the title *is* the screen's
/// subject: at 92 points "PLATFORMER" is about 600 wide, which is a logo in the middle of a
/// 1360-point window rather than a caption over one.
fn title_size(rect: Rect) -> f32 {
    (rect.height() * 0.115).clamp(36.0, 96.0)
}

/// The baseline of the lockup's first line, as a fraction of the window's height.
///
/// High enough that the sky above it is a horizon rather than a margin, low enough that the
/// two lines, the rule and the tagline clear the level's floor.
const TITLE_TOP_LINE: f32 = 0.185;
/// The vertical rhythm of the lockup: the second line's distance from the first, then the
/// rule's and the tagline's, as fractions of the title's own size.
const TITLE_LEAD: f32 = 0.88;
const RULE_LEAD: f32 = 0.55;
const SUBTITLE_LEAD: f32 = 0.97;

/// The title, its eyebrow, its rule and the tagline: the whole lockup, revealed in order.
///
/// One `painter.text` per letter rather than one per line, because the reveal is the
/// animation and a galley can only be drawn whole. The advances come from the font
/// ([`egui::epaint::Fonts::glyph_width`]), so the letters land exactly where a single call
/// would have put them, plus the letterspacing that makes type at this size read as a mark
/// rather than as a line of prose.
///
/// Every letter is drawn four times: a drop shadow, a dark outline in eight directions, the
/// gold body, and a cream edge lifted a point and a half. Two of those are the whole of the
/// outline — this is a pixel game, and an offset shadow with a hard outline is the way a
/// pixel game puts a logo on a picture. The bevel is what stops the gold reading as flat
/// yellow: the top of every stroke catches the light the sun is drawn with.
fn lockup(painter: &Painter, ctx: &Context, rect: Rect, clock: f32) {
    let size = title_size(rect);
    let centre = rect.center().x;
    let top = rect.top() + rect.height() * TITLE_TOP_LINE;

    eyebrow(painter, rect, clock);

    // A warm bloom behind the words, so the gold has light behind it rather than sky. Twelve
    // concentric discs at a few percent each: cheap, and it is the difference between type on
    // a background and type *in* a scene.
    let bloom_at = Pos2::new(centre, top + size * 0.55);
    for step in 0..14 {
        let t = step as f32 / 13.0;
        let radius = size * (2.6 - 1.9 * t);
        let alpha = (3.0 + 2.0 * t) as u8;
        painter.circle_filled(
            bloom_at,
            radius,
            Color32::from_rgba_unmultiplied(255, 240, 196, alpha),
        );
    }

    let font = FontId::monospace(size);
    let spacing = size * 0.055;
    let mut index = 0;
    for (line, text) in TITLE_LINES.iter().enumerate() {
        let advances: Vec<f32> = text
            .chars()
            .map(|c| ctx.fonts(|f| f.glyph_width(&font, c)))
            .collect();
        let width: f32 = advances.iter().sum::<f32>()
            + spacing * (text.chars().count().saturating_sub(1)) as f32;
        let y = top + line as f32 * size * TITLE_LEAD;
        let mut x = centre - width * 0.5;
        for (char_index, ch) in text.chars().enumerate() {
            let arrived = letter_in(clock, index);
            index += 1;
            if arrived > 0.0 {
                let ink = arrived.powf(0.7);
                let at = Pos2::new(x, y + (1.0 - arrived) * LETTER_RISE);
                if ch != ' ' {
                    letter(painter, ctx, at, ch, &font, ink);
                }
            }
            x += advances[char_index] + spacing;
        }
    }

    // The rule, growing from the middle out as the second line lands, with a gold diamond at
    // its centre: the one piece of jewellery on the screen, and what stops a stack of type
    // over a landscape from reading as a caption.
    // The rule grows from the middle as the title's last letter lands, and the tagline
    // follows it — timed off the reveal itself rather than off a second guess at it.
    let settled = TITLE_AT + title_letters() as f32 * LETTER_IN;
    let grown = ((clock - settled + 0.15) / 0.7).clamp(0.0, 1.0);
    if grown > 0.0 {
        let y = top + size * (TITLE_LEAD + RULE_LEAD);
        let half = size * 2.4 * grown;
        let gap = 12.0;
        let stroke = Stroke::new(2.0_f32, RULE_INK.gamma_multiply(grown));
        painter.line_segment(
            [Pos2::new(centre - half, y), Pos2::new(centre - gap, y)],
            stroke,
        );
        painter.line_segment(
            [Pos2::new(centre + gap, y), Pos2::new(centre + half, y)],
            stroke,
        );
        let gem = [
            Pos2::new(centre, y - 5.0),
            Pos2::new(centre + 5.0, y),
            Pos2::new(centre, y + 5.0),
            Pos2::new(centre - 5.0, y),
        ];
        painter.add(egui::Shape::convex_polygon(
            gem.to_vec(),
            TITLE_INK.gamma_multiply(grown),
            Stroke::new(1.0_f32, TITLE_LIGHT.gamma_multiply(grown * 0.8)),
        ));
    }

    // And the sentence under it, in the sky's own dark ink rather than the window's cream: a
    // pale line over a pale-blue sky has to be read twice, and dark-on-sky is 8.7:1.
    let arrived = ((clock - settled - 0.25) / 0.6).clamp(0.0, 1.0);
    if arrived > 0.0 {
        painter.text(
            Pos2::new(centre, top + size * (TITLE_LEAD + SUBTITLE_LEAD)),
            Align2::CENTER_CENTER,
            SUBTITLE,
            FontId::proportional((size * 0.20).clamp(14.0, 19.0)),
            SUBTITLE_INK.gamma_multiply(arrived),
        );
    }
}

/// One letter of the title: shadow, outline, gold body, lit edge.
fn letter(
    painter: &Painter,
    ctx: &Context,
    at: Pos2,
    ch: char,
    font: &FontId,
    ink: f32,
) {
    let glyph = |offset: Vec2, colour: Color32| {
        painter.text(
            at + offset,
            Align2::LEFT_CENTER,
            ch,
            font.clone(),
            colour.gamma_multiply(ink),
        );
    };
    // The drop shadow sits low and to the right, the way the sun in this game comes from the
    // upper left: one offset, so a hundred letters do not each invent their own light.
    glyph(
        Vec2::new(4.0, 6.0),
        Color32::from_rgba_unmultiplied(10, 14, 20, 170),
    );
    for dx in [-2.0, 0.0, 2.0] {
        for dy in [-2.0, 0.0, 2.0] {
            if dx != 0.0 || dy != 0.0 {
                glyph(Vec2::new(dx, dy), TITLE_SHADE);
            }
        }
    }
    glyph(Vec2::ZERO, TITLE_INK);
    // The lit top edge: the gold, lifted towards the sun. Drawn over the body rather than
    // under it, so the bevel is on the *inside* of the stroke.
    glyph(Vec2::new(0.0, -1.5), TITLE_LIGHT.gamma_multiply(0.55));
    let _ = ctx;
}

/// The genre line above the title, between two short rules.
///
/// Small, letterspaced and gold: it is the part of a box that says what the game *is*, and at
/// this size the only way it reads is as a label rather than as a sentence.
fn eyebrow(painter: &Painter, rect: Rect, clock: f32) {
    let arrived = ((clock - 0.10) / 0.55).clamp(0.0, 1.0);
    if arrived <= 0.0 {
        return;
    }
    let ink = EYEBROW_INK.gamma_multiply(arrived * 0.9);
    let font = FontId::monospace((rect.height() * 0.0155).clamp(10.0, 13.0));
    let spacing = 3.0;
    let widths: Vec<f32> = EYEBROW
        .chars()
        .map(|c| painter.layout_no_wrap(c.to_string(), font.clone(), ink).size().x)
        .collect();
    let width: f32 = widths.iter().sum::<f32>() + spacing * (widths.len() - 1) as f32;
    let y = rect.top() + rect.height() * 0.115;
    let mut x = rect.center().x - width * 0.5;
    for (index, ch) in EYEBROW.chars().enumerate() {
        if ch != ' ' {
            painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, ch, font.clone(), ink);
        }
        x += widths[index] + spacing;
    }
    let rule = Stroke::new(1.5_f32, EYEBROW_INK.gamma_multiply(arrived * 0.55));
    let left = rect.center().x - width * 0.5 - 16.0;
    let right = rect.center().x + width * 0.5 + 16.0;
    painter.line_segment([Pos2::new(left - 78.0, y), Pos2::new(left, y)], rule);
    painter.line_segment([Pos2::new(right, y), Pos2::new(right + 78.0, y)], rule);
}

/// The stage: the band under the level's floor, darkened to a near-black gradient.
///
/// The menu has to be an *object on a picture*, not a caption painted over a lawn. The level's
/// own under-ground band is already a flat murk, so darkening it costs nothing the scene was
/// using and buys the contrast the gold needs. It fades in from just above the floor line, so
/// the grass and the runner stay at full brightness and only the space below them goes down.
fn stage(painter: &Painter, rect: Rect, floor: f32) {
    let top = (floor - 30.0).clamp(rect.top(), rect.bottom() - 1.0);
    let clear = Color32::from_rgba_unmultiplied(STAGE_INK.r(), STAGE_INK.g(), STAGE_INK.b(), 0);
    let solid = Color32::from_rgba_unmultiplied(STAGE_INK.r(), STAGE_INK.g(), STAGE_INK.b(), 236);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(Pos2::new(rect.left(), top), clear);
    mesh.colored_vertex(Pos2::new(rect.right(), top), clear);
    mesh.colored_vertex(Pos2::new(rect.right(), rect.bottom()), solid);
    mesh.colored_vertex(Pos2::new(rect.left(), rect.bottom()), solid);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
    // A hairline of gold along the bottom of the window, the same lip the dock wears: the
    // stage is furniture, and the window's furniture has a gold edge.
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(rect.left(), rect.bottom() - 3.0),
            Vec2::new(rect.width(), 3.0),
        ),
        CornerRadius::ZERO,
        theme::GOLD_EDGE.gamma_multiply(0.55),
    );
}

/// Whether the menu is on screen at `clock`.
///
/// One function rather than the comparison written twice, because `show` has to answer it
/// for a reason of its own: a press inside the menu belongs to the *row* under the pointer,
/// and the rule that any press starts the editor has to step aside for it. Two copies of
/// "when is it up" is how the band ends up swallowing a click on a frame the rows are not
/// drawn on.
fn menu_is_up(clock: f32) -> bool {
    clock >= HINT_AT + DOORS_LAG
}

/// The gap after each glyph of letterspaced type: about a fifth of a capital.
const SPACING: f32 = 3.0;

/// How far the menu has arrived: 0 not yet, 1 settled.
///
/// One progress for the whole panel rather than one per row: a menu is a *box* in every game
/// that has one, and a box arrives whole.
fn menu_in(clock: f32) -> f32 {
    ((clock - HINT_AT - DOORS_LAG) / 0.30).clamp(0.0, 1.0)
}

/// Where the menu sits: a framed panel in the band under the level.
///
/// Bottom-anchored, because the window is the thing the menu is *on*: a champion is played on
/// whatever document was opened, and a level tall enough to reach the panel must not push the
/// doorway off the screen. The stage behind it is what keeps it readable when it overlaps.
fn menu_panel(rect: Rect) -> Rect {
    let height = PANEL_PAD * 2.0 + LINE_H * DOORS.len() as f32 + DESC_H;
    let top = (rect.bottom() - MENU_BOTTOM_PAD - height).max(rect.top() + 12.0);
    Rect::from_min_size(
        Pos2::new(rect.center().x - MENU_W * 0.5, top),
        Vec2::new(MENU_W, height),
    )
}

/// The rectangle of one option line, in screen space.
///
/// The one statement of where a line is, shared by the painter and the hit test — a line
/// drawn by a layout and clicked by arithmetic is how a menu ends up twenty points off its
/// own highlight.
fn line_rect(panel: Rect, index: usize) -> Rect {
    Rect::from_min_size(
        Pos2::new(panel.left(), panel.top() + PANEL_PAD + index as f32 * LINE_H),
        Vec2::new(panel.width(), LINE_H),
    )
}

/// Draw the menu, and report the door that was clicked, if any.
///
/// Real widgets (`Ui::interact`) for the hit test, hand-painted for the look: the panel is a
/// dialog box, the options are lines of type with a cursor beside them, and the row under the
/// pointer is the row the keyboard's arrow keys are on. That is the whole of a console menu —
/// a frame, a list, one cursor and one blinking prompt — and it is what this screen is: a
/// game's front door, not an application's launcher.
fn menu(
    ctx: &Context,
    painter: &Painter,
    rect: Rect,
    panel: Rect,
    selected: &mut usize,
    clock: f32,
) -> Option<Action> {
    if !menu_is_up(clock) {
        return None;
    }
    let mut action = None;
    let arrived = menu_in(clock);
    egui::Area::new(egui::Id::new("splash-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .interactable(true)
        .show(ctx, |ui| {
            // The area covers the window rather than the panel, so a line is never clipped by
            // its own container and the hit test is against the same rectangle the painter
            // used. It draws nothing itself: the panel is painted on the background layer,
            // under every panel the window might put up.
            ui.set_min_size(rect.size());
            for (index, door) in DOORS.iter().enumerate() {
                let response = ui.interact(
                    line_rect(panel, index),
                    egui::Id::new(("splash-door", index)),
                    egui::Sense::click(),
                );
                if response.hovered() {
                    *selected = index;
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked() {
                    action = Some(door.action);
                }
            }
        });
    if arrived > 0.0 {
        paint_menu(painter, panel, *selected, arrived, clock);
        prompt(painter, rect, clock);
    }
    action
}

/// The menu panel and everything in it.
///
/// A menu plate from a Nintendo game: a warm white card in a thick chocolate outline, a blue
/// `SELECT` tab on its top edge, a gold band under the cursor and a blinking red pointer. Rounded
/// corners and a hard drop shadow rather than a flat rectangle — the plate is a physical toy lying
/// on the picture, which is the whole of the look this window and its title screen share.
fn paint_menu(painter: &Painter, panel: Rect, selected: usize, arrived: f32, clock: f32) {
    let ink = |colour: Color32| colour.gamma_multiply(arrived);
    let shift = Vec2::new(0.0, (1.0 - arrived) * 12.0);
    let panel = panel.translate(shift);
    let radius = CornerRadius::same(theme::RADIUS_LG);

    // The tab: a blue ribbon on the box's top edge, the way a menu screen labels itself. It is
    // drawn first so the box's outline closes over its bottom and the tab reads as stuck *onto*
    // the plate rather than as a second panel floating above it.
    let tab_w = 196.0;
    let tab = Rect::from_min_size(
        Pos2::new(panel.center().x - tab_w * 0.5, panel.top() - 26.0),
        Vec2::new(tab_w, 36.0),
    );
    painter.rect_filled(
        tab.translate(Vec2::new(0.0, 2.0)),
        CornerRadius::same(8),
        ink(Color32::from_rgba_unmultiplied(74, 52, 32, 110)),
    );
    painter.rect_filled(tab, CornerRadius::same(8), ink(theme::RIBBON));
    painter.rect_stroke(
        tab,
        CornerRadius::same(8),
        Stroke::new(2.5_f32, ink(theme::OUTLINE)),
        egui::StrokeKind::Inside,
    );
    let tab_font = FontId::monospace(12.0);
    let tab_text = "SELECT";
    let tab_text_w = spaced_width(painter, tab_text, &tab_font);
    spaced(
        painter,
        Pos2::new(
            tab.center().x - tab_text_w * 0.5,
            panel.top() - 8.0,
        ),
        tab_text,
        tab_font,
        ink(Color32::WHITE),
        2.5,
    );

    // The box: a warm white plate with a soft brown shadow, a coin-gold hairline inside and a
    // thick chocolate outline on it. Three rings, and none of them black: this is the same plate
    // the window's own panels are cut from, which is what makes the title screen and the editor
    // look like two rooms of one house.
    painter.rect_filled(
        panel.translate(Vec2::new(0.0, 5.0)),
        radius,
        ink(Color32::from_rgba_unmultiplied(74, 52, 32, 110)),
    );
    painter.rect_filled(panel, radius, ink(PANEL_FACE));
    painter.rect_stroke(
        panel.shrink(5.0),
        CornerRadius::same(theme::RADIUS),
        Stroke::new(1.5_f32, ink(theme::GOLD_EDGE.gamma_multiply(0.8))),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        panel,
        radius,
        Stroke::new(3.0_f32, ink(theme::OUTLINE)),
        egui::StrokeKind::Inside,
    );

    // The options. The cursor blinks on the *frame's* clock, a hard on-and-off rather than a
    // fade: every game that ever had a menu blinked its cursor, and a breath reads as a glow
    // rather than as an invitation to press something.
    let blink = (clock * 2.2).fract() < 0.62;
    for (index, door) in DOORS.iter().enumerate() {
        let line = line_rect(panel, index);
        let chosen = index == selected;
        if chosen {
            // The chosen line is a coin-gold bar with the window's own rounded corner — the
            // same gold a selected tool wears, so "this is the one that is on" is one idea in
            // both rooms.
            let band = line.shrink2(Vec2::new(6.0, 3.0));
            painter.rect_filled(
                band.translate(Vec2::new(0.0, 2.0)),
                CornerRadius::same(theme::RADIUS),
                ink(theme::GOLD_EDGE.gamma_multiply(0.55)),
            );
            painter.rect_filled(band, CornerRadius::same(theme::RADIUS), ink(theme::ACCENT));
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(band.left() + 8.0, band.top() + 2.0),
                    Vec2::new((band.width() - 16.0).max(0.0), 1.0),
                ),
                CornerRadius::same(theme::RADIUS),
                ink(Color32::from_white_alpha(110)),
            );
        }
        let name_ink = if chosen {
            theme::OUTLINE
        } else {
            theme::OUTLINE.gamma_multiply(0.82)
        };
        let key_ink = theme::TEXT_DIM;
        // The cursor: the runner's own red, pointing at the line, blinking.
        if chosen && blink {
            let cursor = cursor_points(Pos2::new(panel.left() + 24.0, line.center().y));
            painter.add(egui::Shape::convex_polygon(
                cursor.to_vec(),
                ink(theme::RUN),
                Stroke::NONE,
            ));
        }
        spaced_shadowed(
            painter,
            Pos2::new(panel.left() + 48.0, line.center().y),
            door.name,
            FontId::monospace(21.0),
            ink(name_ink),
            ink(Color32::from_white_alpha(0)),
            2.0,
        );
        let key_font = FontId::monospace(11.0);
        let galley = painter.layout_no_wrap(door.key.to_string(), key_font, ink(key_ink));
        painter.galley(
            Pos2::new(
                panel.right() - 20.0 - galley.size().x,
                line.center().y - galley.size().y * 0.5,
            ),
            galley,
            ink(key_ink),
        );
    }

    // The description strip: one sentence about the highlighted door, the way a console menu
    // explains the file under the cursor rather than putting a sentence on every line.
    let top = panel.top() + PANEL_PAD + LINE_H * DOORS.len() as f32;
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(panel.left() + 14.0, top + 6.0),
            Vec2::new(panel.width() - 28.0, 1.0),
        ),
        CornerRadius::ZERO,
        ink(theme::EDGE),
    );
    painter.text(
        Pos2::new(panel.left() + 24.0, top + 6.0 + DESC_H * 0.5),
        Align2::LEFT_CENTER,
        DOORS[selected].blurb,
        FontId::proportional(13.0),
        ink(theme::TEXT_DIM),
    );
}

/// How wide a letterspaced run is: the sum of its glyphs plus the gaps between them.
fn spaced_width(painter: &Painter, text: &str, font: &FontId) -> f32 {
    let widths: Vec<f32> = text
        .chars()
        .map(|c| {
            painter
                .layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .collect();
    widths.iter().sum::<f32>() + SPACING * (widths.len().saturating_sub(1)) as f32
}

/// Letterspaced type drawn from a left-centre anchor, one glyph at a time.
///
/// The window's fonts are proportional-ish at this size, so letterspacing has to be applied by
/// hand: a run of characters is laid out with a fixed gap after each, which is what makes a
/// menu's capitals read as a *sign* rather than as a sentence.
fn spaced(
    painter: &Painter,
    at: Pos2,
    text: &str,
    font: FontId,
    colour: Color32,
    spacing: f32,
) -> f32 {
    let mut x = at.x;
    for c in text.chars() {
        let galley = painter.layout_no_wrap(c.to_string(), font.clone(), colour);
        let advance = galley.size().x;
        if c != ' ' {
            painter.galley(Pos2::new(x, at.y - galley.size().y * 0.5), galley, colour);
        }
        x += advance + spacing;
    }
    x - at.x - spacing
}

/// The cursor: a solid pixel triangle pointing at the chosen line.
fn cursor_points(at: Pos2) -> [Pos2; 3] {
    [
        Pos2::new(at.x - 5.0, at.y - 6.0),
        Pos2::new(at.x + 5.0, at.y),
        Pos2::new(at.x - 5.0, at.y + 6.0),
    ]
}

/// Letterspaced text with a one-pixel drop shadow: a menu line's own type.
fn spaced_shadowed(
    painter: &Painter,
    at: Pos2,
    text: &str,
    font: FontId,
    colour: Color32,
    shadow: Color32,
    spacing: f32,
) {
    spaced(
        painter,
        at + Vec2::new(2.0, 2.0),
        text,
        font.clone(),
        shadow,
        spacing,
    );
    spaced(painter, at, text, font, colour, spacing);
}

/// The blinking invitation under the box, and the keys that move the cursor.
///
/// "PRESS ENTER" rather than "press any key": the screen has a cursor now, so the honest
/// invitation is the key that takes it. Any *other* key still leaves for the editor, which is
/// what the Edit line's own `ANY KEY` chip says.
fn prompt(painter: &Painter, rect: Rect, clock: f32) {
    let arrived = ((clock - HINT_AT - DOORS_LAG - 0.15) / 0.45).clamp(0.0, 1.0);
    if arrived <= 0.0 {
        return;
    }
    let blink = (clock * 1.5).fract() < 0.66;
    if blink {
        let font = FontId::monospace(14.0);
        let text = "PRESS ENTER TO START";
        let width = spaced_width(painter, text, &font);
        // Gold on a brown shadow, the way a game's score line is printed over the picture: the
        // shadow is the outline's own colour, so the words belong to the same sign as the box.
        spaced_shadowed(
            painter,
            Pos2::new(rect.center().x - width * 0.5, rect.bottom() - 62.0),
            text,
            font,
            theme::ACCENT.gamma_multiply(arrived),
            theme::OUTLINE.gamma_multiply(arrived),
            SPACING,
        );
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 42.0),
        Align2::CENTER_CENTER,
        MENU_HINT,
        FontId::monospace(10.0),
        Color32::from_rgb(222, 204, 172).gamma_multiply(arrived),
    );
}

/// The bottom band: what this is, whose run it is, and what it is not.
fn footer(painter: &Painter, rect: Rect, credit: Option<&str>) {
    let font = FontId::monospace(11.0);
    painter.text(
        Pos2::new(rect.left() + 14.0, rect.bottom() - 11.0),
        Align2::LEFT_BOTTOM,
        format!("evo-app {}", env!("CARGO_PKG_VERSION")),
        font.clone(),
        FOOT_INK,
    );
    // The credit is the one line down here worth reading, so it is the one line down here
    // that is not grey: the title's gold, held back far enough to stay a caption.
    if let Some(credit) = credit {
        painter.text(
            Pos2::new(rect.center().x, rect.bottom() - 11.0),
            Align2::CENTER_BOTTOM,
            credit,
            font.clone(),
            theme::ACCENT.gamma_multiply(0.85),
        );
    }
    painter.text(
        Pos2::new(rect.right() - 14.0, rect.bottom() - 11.0),
        Align2::RIGHT_BOTTOM,
        "edit · play · evolve · replay",
        font,
        FOOT_INK,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use evo::{default_shape, run_episode, GaConfig};
    use sim::Outcome;
    use std::path::PathBuf;

    fn assets_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    /// Drive one frame of the screen with the given events, and report what it decided.
    ///
    /// The screen's own `show`, at a fixed window size, so the tests below are about what the
    /// widgets do rather than about a reimplementation of them: egui reads the events into
    /// input state before the frame runs, which is why a press and the release that follows it
    /// are two calls rather than one.
    fn frame(
        ctx: &Context,
        splash: &mut Splash,
        atlas: &Atlas,
        sprites: &Sprites,
        rect: Rect,
        events: Vec<egui::Event>,
    ) -> Option<Action> {
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(rect),
                events,
                ..Default::default()
            },
            |ctx| {
                action = splash.show(ctx, atlas, sprites);
            },
        );
        action
    }

    /// A pointer press or release at `pos`, as egui delivers one.
    fn at(pos: Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            },
        ]
    }

    /// The screen, its atlas, and the rectangle of the window the tests drive it at.
    fn screen() -> (Context, Atlas, Sprites, Rect) {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));
        (ctx, atlas, sprites, rect)
    }

    /// A splash driven far enough that the doors and the invitation are both up.
    fn armed() -> Splash {
        let mut splash = Splash::scripted();
        while splash.clock() < HINT_AT + 0.6 {
            splash.advance(sim::DT);
        }
        splash
    }

    /// The menu box, for a splash at its armed clock: the one statement of where the box is,
    /// shared with the painter (`menu_panel`).
    fn band_of(_splash: &Splash, rect: Rect) -> Rect {
        menu_panel(rect)
    }

    /// A click on a door enters the activity it names, and the press that starts the click
    /// does *not* leave for the editor on its way.
    ///
    /// The whole reason the band exists. The screen's oldest rule is "any press starts the
    /// editor", and a button is a widget that answers on the release — so without the
    /// carve-out, aiming at Play would take the window to the editor on the way down and
    /// Play would never be reached. The two frames are exactly the two a mouse sends:
    /// press, then release.
    #[test]
    fn a_click_on_a_door_enters_it_and_the_press_does_not_leave_first() {
        let (ctx, atlas, sprites, rect) = screen();
        let mut splash = armed();
        let panel = band_of(&splash, rect);
        // The middle line is Play: three lines of one height, so the second one is the middle
        // of the box.
        let pos = line_rect(panel, 1).center();

        // Two frames with no events first, and the count is not padding. An `egui::Area`
        // registers itself in the layer order on the frame it is first laid out and is only
        // hit-testable from the *next* one: a new area's own frame can draw itself and still
        // answer "no" to a pointer over it, because the layer stack the hit test consults is
        // the previous frame's. One warm-up frame therefore leaves the press on the floor of
        // the window and the *window* takes it — this test failed exactly that way, with the
        // button's rectangle printed around the pointer and `hovered()` false. Two frames is
        // a window that has drawn the screen twice, which is every frame a person can click
        // on: the doors are up for a quarter second before the invitation is drawn, and the
        // first of those is the only one an area cannot be clicked on.
        assert_eq!(
            frame(&ctx, &mut splash, &atlas, &sprites, rect, Vec::new()),
            None
        );
        assert_eq!(
            frame(&ctx, &mut splash, &atlas, &sprites, rect, Vec::new()),
            None
        );
        assert_eq!(
            frame(&ctx, &mut splash, &atlas, &sprites, rect, at(pos, true)),
            None,
            "the press on a door left for the editor instead of reaching the button"
        );
        assert_eq!(
            frame(&ctx, &mut splash, &atlas, &sprites, rect, at(pos, false)),
            Some(Action::Play),
            "the release on the Play door did not enter the playtest"
        );
    }

    /// Every other click is still "just start": the editor.
    ///
    /// The other half of the carve-out, and the one a narrow rule gets wrong: the band is a
    /// strip under the world, and a press at the top of the window is nowhere near it.
    #[test]
    fn a_press_away_from_the_doors_still_opens_the_editor() {
        let (ctx, atlas, sprites, rect) = screen();
        let mut splash = armed();
        let far = Pos2::new(rect.center().x, rect.top() + 40.0);
        assert!(far.y < band_of(&splash, rect).top());
        assert_eq!(
            frame(&ctx, &mut splash, &atlas, &sprites, rect, at(far, true)),
            Some(Action::Edit),
            "a click on the sky stopped being a way out of the screen"
        );
    }

    /// The box arrives only once the invitation has been, and it sits under the world.
    ///
    /// Both halves are geometry, and both are the kind a screenshot catches late: a menu drawn
    /// over the level's lawn, or a box that appears before the caption that explains it. The
    /// box is anchored to the window's bottom edge — a menu is a thing you *open into*, not a
    /// caption that follows the terrain — so a level tall enough to reach it must not push the
    /// doorways off the screen.
    #[test]
    fn the_menu_arrives_with_the_caption_and_sits_under_the_world() {
        assert!(!menu_is_up(0.0));
        assert!(!menu_is_up(HINT_AT));
        assert!(menu_is_up(HINT_AT + DOORS_LAG + 0.01));

        let (_, _, _, rect) = screen();
        let splash = armed();
        let world_bottom = splash
            .camera(rect)
            .to_screen(0.0, splash.level.height as f32, rect)
            .y;
        let panel = menu_panel(rect);
        assert!(
            panel.top() > world_bottom,
            "the menu is drawn over the level: box {panel:?} against world bottom {world_bottom}"
        );
        // The prompt and the key line live under the box, and the box leaves them their band.
        assert!(
            panel.bottom() <= rect.bottom() - 70.0,
            "the box is standing on the prompt under it: {panel:?}"
        );
        assert!(panel.bottom() <= rect.bottom(), "the box falls off the window");

        // A short window still holds the whole box: the top is clamped into the frame.
        let small = Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 240.0));
        let panel = menu_panel(small);
        assert!(panel.top() >= small.top() && panel.bottom() <= small.bottom());
    }

    /// The path of a shipped level, as the window would be given it.
    fn shipped_path(slug: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"))
    }

    /// A shipped level, which is what a champion is ever saved for.
    fn shipped(slug: &str) -> Level {
        Level::from_json_file(shipped_path(slug)).unwrap_or_else(|e| panic!("{slug}: {e}"))
    }

    /// A scratch directory no other test sees.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("splash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// A champion file for `level`, written the way `train-cli train` writes one.
    ///
    /// The genome is the all-zeros one, which is a free "hold right" policy (finding 2): it
    /// wins a flat level and falls in a pit, which is one of each of the two champions these
    /// tests need.
    fn save_champion(dir: &Path, name: &str, level: &Level, seed: u64) -> ChampionFile {
        let genome = Genome::zeros(1, default_shape());
        let config = GaConfig {
            pop_size: 2,
            ..GaConfig::default()
        };
        let result = run_episode(level, &genome, &config.eval_config(), seed);
        let champion = ChampionFile::new(&genome, &result, level, 7, &config);
        champion
            .save(dir.join(format!("{name}.json")))
            .expect("the champion writes");
        champion
    }

    /// Drive the screen to the end of the episode it is playing, in whole ticks.
    ///
    /// Deliberately `sim::DT` per frame rather than the wall clock a window delivers: the
    /// champion is expected to be *frame-rate independent*, and the test that says so should
    /// not be the one that hands it a convenient frame size.
    fn play_to_the_end(splash: &mut Splash) -> u32 {
        let mut frames = 0;
        while !splash.session.sim().outcome().is_finished() && frames < 20_000 {
            splash.advance(sim::DT);
            frames += 1;
        }
        frames
    }

    /// The script the splash runs has to actually play the level it is on.
    ///
    /// This is the load-bearing test of the screen: everything else here is about taste, and
    /// this is about whether the thing a person is looking at is a runner reaching a flag or
    /// a runner falling in a pit for as long as the window is open. It runs the splash's own
    /// `advance`, not a reimplementation of it, and asks it to win.
    #[test]
    fn the_splash_runner_reaches_the_flag() {
        let mut splash = Splash::scripted();
        let mut seconds = 0.0;
        while !splash.session.sim().outcome().is_finished() && seconds < 30.0 {
            splash.advance(sim::DT);
            seconds += sim::DT;
        }
        assert_eq!(
            splash.session.sim().outcome(),
            Outcome::Won,
            "the splash's own runner did not get to the flag in {seconds:.1}s: {}",
            splash.session.summary()
        );
        assert!(
            seconds < 12.0,
            "the little run takes {seconds:.1}s, which is longer than a splash has"
        );
    }

    /// And when it has won, the screen holds on the flag and then starts over.
    ///
    /// The loop is the part that makes the screen *alive* for as long as it takes somebody to
    /// decide to press a key: an episode that ends and stays ended is a still frame with a
    /// moving flag.
    #[test]
    fn the_episode_ends_and_starts_again() {
        let mut splash = Splash::scripted();
        while !splash.session.sim().outcome().is_finished() {
            splash.advance(sim::DT);
        }
        // The hold, and a little past it.
        for _ in 0..((HOLD + 0.2) / sim::DT).ceil() as u32 {
            splash.advance(sim::DT);
        }
        assert_eq!(
            splash.session.sim().outcome(),
            Outcome::Running,
            "the little run never started again"
        );
        assert!(
            splash.session.sim().stats().frames < 30,
            "the restart rewound the episode to {} frames rather than to the start",
            splash.session.sim().stats().frames
        );
    }

    /// The camera starts on the runner and ends on the whole level.
    ///
    /// The opening move, as geometry: closer than the fit at the first frame, exactly the fit
    /// once the pull-back is over. A camera that eased to *about* the fit would leave the
    /// level hanging off one edge of the window for the rest of the screen's life.
    #[test]
    fn the_camera_pulls_back_to_the_whole_level() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));
        let splash = Splash::scripted();
        let fit = framed(&splash.level, rect);
        let opening = splash.camera(rect);
        assert!(
            fit.centre.y > Camera::fit(&splash.level, rect, 24.0).centre.y,
            "the frame is not raised: the words have no band of their own"
        );
        assert!(
            opening.zoom > fit.zoom * 1.2,
            "the splash opens at {} points a tile against a fit of {}",
            opening.zoom,
            fit.zoom
        );
        let p = splash.session.sim().player();
        assert!(
            (opening.centre.x - (p.x + p.w * 0.5)).abs() < 0.01,
            "the opening frame is not on the runner"
        );

        let mut splash = Splash::scripted();
        for _ in 0..((PULLBACK + 0.2) / sim::DT).ceil() as u32 {
            splash.advance(sim::DT);
        }
        let settled = splash.camera(rect);
        assert!(
            (settled.zoom - fit.zoom).abs() < 0.01 && (settled.centre - fit.centre).length() < 0.01,
            "after the pull-back the camera is at {settled:?} and the fit is at {fit:?}"
        );
        // And the words have the band this framing is for: the invitation and the keys are
        // below the level's own bottom edge, not printed over the lawn.
        let ground = splash.level.height as f32;
        let bottom = settled.to_screen(0.0, ground, rect).y;
        let invitation = rect.bottom() - rect.height() * 0.125;
        assert!(
            invitation > bottom + 20.0,
            "the invitation at {invitation:.0} is on the world, which ends at {bottom:.0}"
        );
    }

    /// The title is fully revealed before the invitation arrives to be read.
    ///
    /// Both halves matter: a letter that is still arriving when the "press any key" line is
    /// solid is a screen that is still assembling itself while it tells you to leave, and a
    /// reveal that takes longer than a person waits is a title nobody reads.
    #[test]
    fn the_title_arrives_before_the_invitation() {
        let last = title_letters() - 1;
        assert_eq!(
            letter_in(0.0, 0),
            0.0,
            "the title is up before the clock is"
        );
        assert!(letter_in(0.0, last) == 0.0);
        for index in 0..title_letters() {
            assert!(
                letter_in(TITLE_AT + index as f32 * LETTER_IN - 0.01, index) == 0.0,
                "letter {index} arrives early"
            );
            let done = TITLE_AT + index as f32 * LETTER_IN + LETTER_FADE;
            assert_eq!(
                letter_in(done + 0.01, index),
                1.0,
                "letter {index} never settles"
            );
            assert!(
                done < HINT_AT,
                "letter {index} is still arriving at {done:.2}s, after the invitation at {HINT_AT}s"
            );
        }
    }

    /// The screen is a doorway: a key or a click opens the game, and an event in the first
    /// [`ARMED`] seconds is not a person.
    ///
    /// Both halves are the point. A splash that never lets go is a wall, and one that lets go
    /// on the window manager's opening focus click — or on the key repeat of a key the viewer
    /// was already holding when the window opened — has eaten the title before anybody read it.
    #[test]
    fn a_key_or_a_click_opens_the_door_and_an_early_one_does_not() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));
        // A key the screen does not name, so this test is about "any key opens the door"
        // rather than about what a *specific* key means (that is the next test's job).
        let key = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        let click = egui::Event::PointerButton {
            pos: rect.center(),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        };
        let opened = |clock: f32, events: Vec<egui::Event>| -> Option<Action> {
            let mut splash = Splash::scripted();
            while splash.clock() < clock {
                splash.advance(sim::DT);
            }
            let mut dismissed = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    dismissed = splash.show(ctx, &atlas, &sprites);
                },
            );
            dismissed
        };

        assert!(
            opened(ARMED - 0.05, vec![key.clone()]).is_none(),
            "a key in the first {ARMED}s dismissed the screen"
        );
        assert!(
            opened(ARMED - 0.05, vec![click.clone()]).is_none(),
            "a click in the first {ARMED}s dismissed the screen"
        );
        assert!(
            opened(ARMED + 0.1, Vec::new()).is_none(),
            "the screen dismissed itself with nobody there"
        );
        assert_eq!(
            opened(ARMED + 0.1, vec![key]),
            Some(Action::Edit),
            "an unnamed key opens the editor"
        );
        assert_eq!(
            opened(ARMED + 0.1, vec![click]),
            Some(Action::Edit),
            "a click opens the editor"
        );
    }

    /// The front door leads where its sign says: `P` opens the playtest and `E` starts a
    /// run, rather than both dropping the viewer into the editor to press the key again.
    ///
    /// This is the whole of the fix that made the title screen's key hints honest: before
    /// it, the hints named keys the editor — not the splash — would have had to act on, and
    /// the press that dismissed the screen was consumed by the screen, so the key did
    /// nothing at all.
    #[test]
    fn the_named_keys_open_the_activity_they_name() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));
        let press = |key: egui::Key| -> Option<Action> {
            let mut splash = Splash::scripted();
            while splash.clock() < ARMED + 0.1 {
                splash.advance(sim::DT);
            }
            let event = egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            };
            let mut chosen = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    events: vec![event],
                    ..Default::default()
                },
                |ctx| {
                    chosen = splash.show(ctx, &atlas, &sprites);
                },
            );
            chosen
        };
        assert_eq!(press(egui::Key::P), Some(Action::Play));
        assert_eq!(press(egui::Key::E), Some(Action::Evolve));
        assert_eq!(press(egui::Key::V), Some(Action::Edit));
    }

    /// The champion on the title screen is the episode the file records, tick for tick.
    ///
    /// This is the test the whole feature rests on. The screen is not replaying recorded
    /// positions — there is no recording — it is *re-deciding*: the simulation is stepped the
    /// way the trainer stepped it and the network is queried on the ticks the trainer queried
    /// it. A cadence wrong by one tick would put a different agent on the title screen from the
    /// one the file describes, which would make the picture a lie. So the screen is driven to
    /// the end and asked to agree with the file about where the episode finished, on which
    /// frame, with how much ground covered, and how it ended.
    #[test]
    fn the_title_screen_plays_the_episode_the_champion_file_records() {
        let level = shipped("l1-flat-run");
        let path = shipped_path("l1-flat-run");
        let dir = scratch("champion");
        let champion = save_champion(&dir, "l1-flat-run", &level, 7);
        assert_eq!(
            champion.metadata.outcome, "won",
            "the fixture champion has to win, or this test proves nothing"
        );

        let mut splash = Splash::open(&level, &dir, Some(&path));
        assert!(
            matches!(splash.policy, Policy::Champion(_)),
            "the champion in {} was not picked up",
            dir.display()
        );
        assert_eq!(
            splash.level.content_hash(),
            level.content_hash(),
            "a champion is played on its own terrain"
        );
        assert_eq!(
            splash.session.sim().stats().frames,
            0,
            "nothing has run yet"
        );

        play_to_the_end(&mut splash);
        let stats = splash.session.sim().stats();
        assert_eq!(stats.frames, champion.metadata.frames, "frames");
        assert_eq!(stats.best_x, champion.metadata.best_x, "best_x");
        assert_eq!(
            stats.progress_pixels, champion.metadata.progress_pixels,
            "progress"
        );
        assert_eq!(
            splash.session.sim().outcome().as_str(),
            champion.metadata.outcome,
            "outcome"
        );
        // And it is the champion *playing*, not a picture of one: the runner has moved, and
        // the credit line says whose run this is.
        assert!(stats.best_x > level.spawn.x, "the agent never moved");
        let credit = splash.credit().expect("a champion run says whose it is");
        assert!(
            credit.contains("generation 7")
                && credit.contains(&format!("fitness {:.0}", champion.fitness)),
            "the credit line reads {credit:?}, which is not this file's run"
        );
    }

    /// Nothing that is not this level's champion, for this build, is played.
    ///
    /// Four refusals, each of which would otherwise be a title screen quietly showing the
    /// wrong thing: no file at all; a champion for a different level, whose terrain is not the
    /// one on screen; a file from another build, whose physics are not this one's; and a
    /// best-ever agent that dies — the commonest of the four, because a champion is the best a
    /// run *got*, and a run on a hard level may never have beaten it.
    #[test]
    fn nothing_but_this_levels_champion_for_this_build_is_played() {
        let flat = shipped("l1-flat-run");
        let flat_path = shipped_path("l1-flat-run");
        let pit = shipped("l2-first-gap");
        let pit_path = shipped_path("l2-first-gap");

        // Nothing on disk: the demo run, which is also what a fresh checkout gets.
        let empty = scratch("nothing-saved");
        let splash = Splash::open(&flat, &empty, Some(&flat_path));
        assert!(matches!(splash.policy, Policy::Script));
        assert_eq!(
            splash.level.width,
            level().width,
            "the demo level is the splash's"
        );

        // A champion trained on another level.
        let elsewhere = scratch("another-level");
        save_champion(&elsewhere, "l1-flat-run", &pit, 7);
        assert!(matches!(
            Splash::open(&flat, &elsewhere, Some(&flat_path)).policy,
            Policy::Script
        ));

        // A file this build cannot reproduce.
        let stale = scratch("another-build");
        let mut champion = save_champion(&stale, "l1-flat-run", &flat, 7);
        champion.metadata.fitness_version += 1;
        champion
            .save(stale.join("l1-flat-run.json"))
            .expect("the tampered file writes");
        assert!(matches!(
            Splash::open(&flat, &stale, Some(&flat_path)).policy,
            Policy::Script
        ));

        // The run's best agent, which never reached the flag — on the level it was trained
        // for, so the win check is the only thing that can refuse it.
        let loser = scratch("best-lost");
        let recorded = save_champion(&loser, "l2-first-gap", &pit, 7);
        assert_ne!(
            recorded.metadata.outcome, "won",
            "the fixture champion has to lose, or this test proves nothing"
        );
        assert!(matches!(
            Splash::open(&pit, &loser, Some(&pit_path)).policy,
            Policy::Script
        ));
    }

    /// A champion is looked for under both of the names the two front-ends give it.
    ///
    /// `train-cli train` and `train-cli watch` write `<level file stem>.json`; the window's own
    /// Save champion writes `<slugged level name>.json` ([`crate::lab::slug`]). For every
    /// shipped level those are the same string — which is why the divergence has gone
    /// unnoticed — and for a level whose file name and title differ they are not, which is why
    /// the lookup tries both rather than picking one and being blind to half the disk.
    #[test]
    fn a_champion_is_found_under_either_of_its_two_names() {
        let level = shipped("l1-flat-run");
        assert_eq!(crate::lab::slug(&level.name), "l1-flat-run");

        // The terminal's name, given the terminal's level path.
        let by_stem = scratch("by-stem");
        save_champion(&by_stem, "l1-flat-run", &level, 7);
        assert!(matches!(
            Splash::open(&level, &by_stem, Some(&shipped_path("l1-flat-run"))).policy,
            Policy::Champion(_)
        ));

        // The window's name, on a level whose file is called something else entirely — and
        // the terminal's name being *absent* is what makes this the interesting case.
        let by_name = scratch("by-name");
        save_champion(&by_name, "l1-flat-run", &level, 7);
        let odd =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels/l1-flat-run-copy.json");
        assert!(matches!(
            Splash::open(&level, &by_name, Some(&odd)).policy,
            Policy::Champion(_)
        ));

        // And neither name is invented: a champion under an unrelated name is not this
        // level's.
        let unrelated = scratch("unrelated-name");
        save_champion(&unrelated, "some-other-level", &level, 7);
        assert!(matches!(
            Splash::open(&level, &unrelated, Some(&shipped_path("l1-flat-run"))).policy,
            Policy::Script
        ));
    }

    /// A champion is followed rather than fitted, because the level is the window's own.
    ///
    /// The demo level is thirty-four tiles long and the pull-back can show all of it; L1 is
    /// sixty tiles long and fitted it would draw the agent at a fifth of its play size, which is
    /// a title screen about a speck. So the champion's frame is a *tracking* one at the window's
    /// own follow zoom, and this test pins both halves of that: the zoom, and the tracking.
    #[test]
    fn a_champion_is_followed_at_the_games_own_zoom() {
        let level = shipped("l1-flat-run");
        let path = shipped_path("l1-flat-run");
        let dir = scratch("followed");
        save_champion(&dir, "l1-flat-run", &level, 7);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));

        let mut splash = Splash::open(&level, &dir, Some(&path));
        let fit = framed(&level, rect);
        let target = splash.target(rect);
        assert!(
            target.zoom > fit.zoom,
            "a fitted {}x{} level draws at {}, not the follow zoom",
            level.width,
            level.height,
            fit.zoom
        );
        assert_eq!(target.zoom, crate::app::FOLLOW_ZOOM);
        // The opening frame is closer still, on the runner: the pull-back is a move, not a cut.
        assert!(splash.camera(rect).zoom > target.zoom);
        // The floor is pinned low, with the words' band under it.
        let floor = target.to_screen(0.0, level.height as f32, rect).y;
        assert!(
            (floor - rect.height() * GROUND_LINE).abs() < 1.0,
            "the floor is drawn at {floor:.0}, not {} of the window",
            GROUND_LINE
        );
        assert!(
            menu_panel(rect).top() > floor,
            "the menu box is drawn on the ground it is meant to be under"
        );

        // And it tracks: two seconds of running carries the camera with the agent.
        let before = splash.target(rect).centre.x;
        for _ in 0..120 {
            splash.advance(sim::DT);
        }
        let after = splash.target(rect).centre.x;
        assert!(
            after > before + 5.0,
            "the camera moved from {before:.1} to {after:.1} while the agent ran"
        );
    }

    /// The prompt and the key line have their own band under the box, whatever the window.
    ///
    /// The screen plays whatever level the window opened on when that level has a champion, so
    /// the words cannot be placed as a fraction of the world and trusted: a level tall enough to
    /// reach the band would have the prompt printed on its own lawn. The box is anchored to the
    /// window's bottom edge and the prompt to the box, which is what keeps the two readable on
    /// every level the screen can be opened on.
    #[test]
    fn the_prompt_sits_under_the_menu_box() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));
        let panel = menu_panel(rect);
        assert!(panel.top() >= rect.top(), "the box starts above the window");
        // The blinking prompt is at `rect.bottom() - 62`, the key line at `- 42`.
        assert!(
            panel.bottom() < rect.bottom() - 62.0,
            "the prompt is drawn on the box"
        );
        assert!(panel.bottom() < rect.bottom() - 42.0);
        // And the box is bottom-anchored: a taller window moves it, it does not stretch it.
        let tall = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 1100.0));
        assert_eq!(
            menu_panel(tall).height(),
            menu_panel(rect).height(),
            "the box changed shape with the window"
        );
        assert!(menu_panel(tall).bottom() > panel.bottom());
    }

    /// The screen draws the world and the words, and both of them change with the clock.
    ///
    /// Read back the way the renderer left it — the tessellated shapes, counted by which
    /// texture they sampled — rather than by asking the code that drew them: the world is a
    /// great many atlas quads and the words are a great many glyphs from the font texture, and
    /// a clock that stops moving either of them is the one bug a splash can have that a
    /// person would notice immediately.
    #[test]
    fn the_splash_draws_the_world_and_the_words() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let size = Vec2::new(1360.0, 800.0);
        let rect = Rect::from_min_size(Pos2::ZERO, size);
        // The line the title is drawn on. Counting glyphs *here* rather than counting all of
        // them is what makes this a test of the reveal: the footer and the four keys are on
        // the screen from the first frame, and a count of every glyph in the window would be
        // a count of those.
        // The lockup's own band: the eyebrow, both title lines and the tagline under them,
        // and nothing the footer or the menu contributes.
        let line = Rect::from_min_size(
            Pos2::new(rect.left(), rect.height() * 0.08),
            Vec2::new(rect.width(), rect.height() * 0.32),
        );
        let counts = |clock: f32| -> (usize, usize, usize) {
            let mut splash = {
                let mut splash = Splash::scripted();
                while splash.clock() < clock {
                    splash.advance(sim::DT);
                }
                splash
            };
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    ..Default::default()
                },
                |ctx| {
                    splash.show(ctx, &atlas, &sprites);
                },
            );
            let (mut world, mut words, mut title) = (0, 0, 0);
            for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
                let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                    continue;
                };
                if mesh.texture_id == atlas.texture_id() {
                    world += mesh.vertices.len() / 4;
                    continue;
                }
                if mesh.texture_id != egui::TextureId::default() {
                    continue;
                }
                for quad in mesh.vertices.chunks(4) {
                    if quad.len() < 4 {
                        continue;
                    }
                    // A *word* is a quad that samples a glyph. A quad that samples the font
                    // atlas's white pixel is a filled shape — the sunbeams and the vignette
                    // the sky is drawn with are meshes too, and they land on the title's own
                    // line, so counting them would count scenery as type.
                    if quad[0].uv == egui::epaint::WHITE_UV {
                        continue;
                    }
                    let centre = quad.iter().fold(Pos2::ZERO, |a, v| a + v.pos.to_vec2()) / 4.0;
                    words += 1;
                    if line.contains(centre) {
                        title += 1;
                    }
                }
            }
            (world, words, title)
        };

        let letters = title_letters();
        let (world, words, title) = counts(2.4);
        assert!(
            world > 40,
            "the splash drew {world} sprite quads: the world is not on the screen"
        );
        assert!(
            words > 60,
            "the splash drew {words} glyph quads: the words are not on the screen"
        );
        assert!(
            title >= letters,
            "{title} glyphs on the title's line for a {letters}-letter title"
        );
        let (_, _, early) = counts(0.0);
        assert_eq!(early, 0, "the title is already up at the first frame");
        let (later_world, _, _) = counts(6.0);
        assert!(
            later_world != world,
            "the world is the same picture at 2.4s and 6.0s: nothing in it is running"
        );
    }
}
