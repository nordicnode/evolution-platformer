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

/// The lockup is the game's logo itself: the approved wordmark, drawn as art rather than
/// set as type.
///
/// It used to be the words EVOLUTION PLATFORMER lettered in the window's monospace with a
/// bevel, which said what the program *is* and nothing about the game it had become. The
/// game is called One Two Hundred now, and it has a logo — chunky pixel letters cycling the
/// game's own palette on a strip of its own ground — so the front door carries the logo the
/// way a box carries its cover art: as a picture, embedded in the binary
/// (`crates/app/assets/logo.png`), not redrawn in a font that happens to be nearby. A
/// redrawn logo is a second logo, and the two would drift.
///
/// The eyebrow is the genre line a box would carry above its name; the subtitle is the hook.
const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");
/// The logo's own aspect, width over height, so its rectangle on screen is the picture's
/// shape rather than a guess at it.
const LOGO_ASPECT: f32 = 1515.0 / 420.0;
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
/// When the logo starts arriving, and how long its whole entrance takes.
///
/// The logo arrives as one gesture rather than letter by letter: it is a picture, and a
/// picture that assembles itself piece by piece reads as a loading screen. The entrance is
/// the game's own motion rather than a user interface's (see [`logo_pose`]): the mark
/// falls the last stretch under gravity, lands on its ground strip with a squash and a
/// puff of dust, and then sways almost imperceptibly for as long as the screen is up — a
/// sign standing in the world, subject to the same weather as everything else in it.
const LOGO_AT: f32 = 0.30;
const LOGO_FADE: f32 = 0.95;
/// The fraction of the entrance that is the fall; the rest is the landing's settle.
const LOGO_FALL: f32 = 0.62;
/// How far above its resting place the logo starts its fall.
const LOGO_DROP: f32 = 46.0;
/// The deepest the landing squash takes the mark, as a fraction of its height.
const LOGO_SQUASH: f32 = 0.075;
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

/// The screen's share of the game's palette, taken from the logo's own paint box.
///
/// The logo's letters are outlined in one dark plum and shadowed in one flat slate, and the
/// menu below it wears the same two, so the plate on the stage reads as furniture from the
/// same object rather than as a dialog that wandered in from the editor. The bright fills
/// live on the doors themselves (see [`Door::colour`]): each door carries one of the
/// palette families the logo cycles — gold, vermilion, water — and the selected row is
/// *filled* with it, the way a letter of the wordmark is.
/// The outline every letter of the logo wears, and the menu plate with it.
const PLUM: Color32 = Color32::from_rgb(44, 24, 36);
/// The logo's hard shadow: flat slate, offset, never blurred.
const SLATE: Color32 = Color32::from_rgb(86, 100, 124);
/// The tagline is read on the *sky*, so it is the window's dark screen ink rather than its
/// cream: a pale line over a pale-blue sky was the one piece of type on this screen that had
/// to be read twice. Dark navy on sky is 8.7:1 and belongs to the same palette.
const SUBTITLE_INK: Color32 = Color32::from_rgb(24, 34, 46);
/// The footer's quiet ink: the warm cream the prompt's key line is printed in, because the
/// band under the level is dark and the footer's small print has to be read on it. The old
/// footer used the chrome's dim ink and was 1.3:1 on it — a caption nobody could read.
const FOOT_INK: Color32 = Color32::from_rgb(222, 204, 172);
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
/// it, not a card: 42 points is a name, its colour chip, its key cap and the air a blinking
/// cursor needs.
const LINE_H: f32 = 42.0;
/// The panel's padding, and the description plaque under the options.
///
/// The plaque is what a console menu does instead of a sentence on every line: one hint
/// about the thing the cursor is on, in a little plate of its own in the bottom of the box.
const PANEL_PAD: f32 = 14.0;
const DESC_H: f32 = 36.0;
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
    /// The door's own colour out of the logo's cycle: the mid tone of one palette family,
    /// bright enough to fill a row and dark enough to hold dark type. The selected row is
    /// filled with it, so the cursor's position is said in the logo's language — a bright
    /// letter among plain ones — rather than in the editor's.
    colour: Color32,
}

const DOORS: [Door; 3] = [
    Door {
        name: "EDIT",
        action: Action::Edit,
        blurb: "draw a level, tile by tile",
        key: "ANY KEY",
        colour: Color32::from_rgb(255, 214, 102),
    },
    Door {
        name: "PLAY",
        action: Action::Play,
        blurb: "take the keyboard and play it",
        key: "P",
        colour: Color32::from_rgb(244, 128, 52),
    },
    Door {
        name: "EVOLVE",
        action: Action::Evolve,
        blurb: "train a population to beat it",
        key: "E",
        colour: Color32::from_rgb(66, 154, 192),
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
    /// The logo, decoded and uploaded on the first frame that draws it.
    ///
    /// Lazy because a texture needs the context, and the splash is built before the window
    /// hands it one. One upload for the screen's whole life: the picture never changes.
    logo: Option<egui::TextureHandle>,
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
            logo: None,
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
            logo: None,
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

    /// The logo as a texture, decoded and uploaded on first use.
    ///
    /// Nearest filtering, like the atlas: the picture is pixel art, and a smoothed logo
    /// over a nearest-neighbour world would be two rendering decisions on one screen.
    fn logo_texture(&mut self, ctx: &Context) -> egui::TextureHandle {
        if let Some(handle) = &self.logo {
            return handle.clone();
        }
        let decoded = image::load_from_memory(LOGO_PNG).expect("the embedded logo decodes");
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            rgba.as_raw(),
        );
        let handle = ctx.load_texture(
            "one-two-hundred-logo",
            image,
            egui::TextureOptions::NEAREST,
        );
        self.logo = Some(handle.clone());
        handle
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
        let logo = self.logo_texture(ctx);
        lockup(&painter, rect, self.clock, &logo);

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

/// How far the logo's entrance has run: 0 not yet, 1 fully landed and settled.
///
/// A pure function of the clock, so the reveal can be tested without a screen, and so the
/// logo being settled before the invitation arrives is a fact about the timing constants
/// rather than something to eyeball.
fn logo_in(clock: f32) -> f32 {
    ((clock - LOGO_AT) / LOGO_FADE).clamp(0.0, 1.0)
}

/// The size the logo is drawn at: the screen's subject, sized from the window.
///
/// Wide enough to be the thing the screen is *about* — a logo a third of the window wide
/// is a caption — and capped, because past a size the picture is just nearer, not bigger
/// in any way that matters. The height follows from the picture's own aspect.
fn logo_size(rect: Rect) -> Vec2 {
    let width = (rect.width() * 0.54).clamp(400.0, 740.0);
    Vec2::new(width, width / LOGO_ASPECT)
}

/// Where the logo's top edge sits at rest, as a fraction of the window's height.
///
/// High enough that the sky above it is a margin rather than a second subject, low enough
/// that the mark, its tagline and the level's floor stack without touching.
const LOGO_TOP: f32 = 0.098;

/// Where the logo is and how it is shaped at `clock`: its vertical offset from rest
/// (negative is above it) and its height as a fraction of itself.
///
/// The entrance is the game's own motion, not an easing curve off a user interface shelf.
/// The mark *falls* the last stretch under gravity — the distance closes with the square
/// of the time, so it arrives faster than it left, which is the whole difference between
/// falling and sliding — lands with the squash every landing in this game has, and then
/// breathes: a two-point sway on a slow period, the smallest motion that keeps a sign
/// standing in a living scene from reading as pasted onto it. Both halves are timed off
/// the one ramp ([`logo_in`]), so the reveal stays one number a test can reason about.
fn logo_pose(clock: f32) -> (f32, f32) {
    let p = logo_in(clock);
    if p <= 0.0 {
        return (-LOGO_DROP, 1.0);
    }
    if p >= 1.0 {
        // Settled: the sway. Slow and small, there to be felt rather than watched.
        let sway = ((clock - LOGO_AT - LOGO_FADE) * std::f32::consts::TAU / 3.4).sin();
        return (sway * 1.6, 1.0);
    }
    if p < LOGO_FALL {
        let t = p / LOGO_FALL;
        return (-LOGO_DROP * (1.0 - t * t), 1.0);
    }
    // The landing squash: a fast dip to the deepest point, then a slower recovery with a
    // breath of overshoot above full height before it rests — the shape of every landing
    // the physics in this game draws.
    let s = (p - LOGO_FALL) / (1.0 - LOGO_FALL);
    let squash = if s < 0.28 {
        let u = s / 0.28;
        LOGO_SQUASH * (1.0 - (1.0 - u) * (1.0 - u))
    } else {
        let u = (s - 0.28) / 0.72;
        LOGO_SQUASH * (1.0 - u) - 0.018 * (u * std::f32::consts::PI).sin() * (1.0 - u)
    };
    (0.0, 1.0 - squash)
}

/// The lockup: the eyebrow, the logo and the tagline, arriving in that order.
///
/// The picture does the talking. Around it, two lines of the pixel face the logo is set in
/// (see [`crate::theme::pixel_font`]): the genre line above, in the sky's dark ink, and the
/// hook below it. Nothing else is drawn up here — the old lockup's rule and diamond went
/// away with the set type, because jewellery around a picture is a frame competing with
/// its subject.
fn lockup(painter: &Painter, rect: Rect, clock: f32, logo: &egui::TextureHandle) {
    let centre = rect.center().x;

    eyebrow(painter, rect, clock);

    let arrived = logo_in(clock);
    if arrived > 0.0 {
        // Solid almost at once: a thing that is falling should not look like a ghost, so
        // the fade is only there to keep the first frame from being a pop.
        let ink = (arrived / 0.18).clamp(0.0, 1.0);
        let size = logo_size(rect);
        let (offset, squash_y) = logo_pose(clock);
        let squash_x = 1.0 + (1.0 - squash_y) * 0.6;
        let drawn = Vec2::new(size.x * squash_x, size.y * squash_y);
        // Bottom-anchored through the whole entrance: the mark falls *onto* its ground
        // strip and squashes *into* it, so the strip stays planted where it lands and
        // only the top of the picture moves.
        let bottom = rect.top() + rect.height() * LOGO_TOP + size.y + offset;
        let dest = Rect::from_min_max(
            Pos2::new(centre - drawn.x * 0.5, bottom - drawn.y),
            Pos2::new(centre + drawn.x * 0.5, bottom),
        );
        // A warm bloom behind the mark, so the colours have light behind them rather than
        // flat sky. Fourteen concentric discs at a few percent each: cheap, and it is the
        // difference between a picture on a background and a picture *in* a scene.
        for step in 0..14 {
            let t = step as f32 / 13.0;
            let radius = size.x * (0.52 - 0.36 * t);
            let alpha = ((3.0 + 2.0 * t) * ink) as u8;
            painter.circle_filled(
                dest.center(),
                radius,
                Color32::from_rgba_unmultiplied(255, 240, 196, alpha),
            );
        }
        painter.image(
            logo.id(),
            dest,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            // A pure alpha fade rather than a gamma multiply: the multiply darkens as it
            // thins, so the mark's first frames would read as a grey watermark instead of
            // as the picture arriving.
            Color32::from_rgba_unmultiplied(255, 255, 255, (ink * 255.0) as u8),
        );
        landing_dust(painter, dest, clock);

        // And the sentence under it, in the sky's own dark ink rather than the window's
        // cream: a pale line over a pale-blue sky has to be read twice, and dark-on-sky is
        // 8.7:1. It follows the logo in, timed off the landing rather than off the clock's
        // start, so the two never arrive half over each other.
        let tagline = ((clock - LOGO_AT - LOGO_FADE * 0.55) / 0.6).clamp(0.0, 1.0);
        if tagline > 0.0 {
            painter.text(
                Pos2::new(centre, dest.bottom() + 30.0),
                Align2::CENTER_CENTER,
                SUBTITLE,
                theme::pixel_font(13.0),
                SUBTITLE_INK.gamma_multiply(tagline),
            );
        }
    }
}

/// The puff of dust the landing kicks up, along the logo's base.
///
/// Alive for half a second from the moment the fall ends, and a pure function of the
/// clock like everything else on this screen: three pixel squares a side, thrown outward
/// and a little upward, growing and thinning as they go. It is the dust the runner kicks
/// up, said at the size this picture needs — the punctuation that makes the landing a
/// landing in the game's world rather than an animation over it.
fn landing_dust(painter: &Painter, dest: Rect, clock: f32) {
    let impact = LOGO_AT + LOGO_FADE * LOGO_FALL;
    let age = clock - impact;
    if !(0.0..=0.5).contains(&age) {
        return;
    }
    let fade = 1.0 - age / 0.5;
    let ink = Color32::from_rgba_unmultiplied(246, 242, 230, (150.0 * fade) as u8);
    for side in [-1.0_f32, 1.0] {
        for i in 0..3 {
            let f = i as f32;
            let out = 4.0 + age * (46.0 + f * 12.0);
            let x = if side < 0.0 {
                dest.left() + 10.0 - out
            } else {
                dest.right() - 10.0 + out
            };
            let y = dest.bottom() - 3.0 - age * (12.0 + f * 5.0) - f * 2.0;
            let size = 3.0 + age * 12.0 + f * 1.5;
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(x - size * 0.5, y - size * 0.5), Vec2::splat(size)),
                CornerRadius::ZERO,
                ink,
            );
        }
    }
}

/// The genre line above the logo, between two short rules.
///
/// Small and in the pixel face, because it is part of the same sign: the line a box carries
/// over its name to say what the game *is*. Dark on the sky for the subtitle's reason — it
/// has to be read, not just be present.
fn eyebrow(painter: &Painter, rect: Rect, clock: f32) {
    let arrived = ((clock - 0.10) / 0.55).clamp(0.0, 1.0);
    if arrived <= 0.0 {
        return;
    }
    let ink = SUBTITLE_INK.gamma_multiply(arrived * 0.9);
    let font = theme::pixel_font(11.0);
    let y = rect.top() + rect.height() * 0.066;
    let centre = rect.center().x;
    let galley = painter.layout_no_wrap(EYEBROW.to_string(), font.clone(), ink);
    painter.text(Pos2::new(centre, y), Align2::CENTER_CENTER, EYEBROW, font, ink);
    let half = galley.size().x * 0.5;
    let rule = Stroke::new(2.0_f32, SUBTITLE_INK.gamma_multiply(arrived * 0.5));
    painter.line_segment(
        [
            Pos2::new(centre - half - 84.0, y),
            Pos2::new(centre - half - 14.0, y),
        ],
        rule,
    );
    painter.line_segment(
        [
            Pos2::new(centre + half + 14.0, y),
            Pos2::new(centre + half + 84.0, y),
        ],
        rule,
    );
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

/// A colour mixed toward a background: `t` of `colour`, the rest of `bg`.
///
/// The one bit of paint arithmetic the menu needs twice: an unchosen row is tinted with
/// its door's colour rather than filled with it, and the description plaque is tinted
/// with the chosen door's. Mixing in plain RGB is right here — these are flat paints
/// being blended on a plate, not light being simulated.
fn blend(colour: Color32, bg: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 * t + b as f32 * (1.0 - t)).round() as u8;
    Color32::from_rgb(
        mix(colour.r(), bg.r()),
        mix(colour.g(), bg.g()),
        mix(colour.b(), bg.b()),
    )
}

/// A colour taken down toward black: the shaded foot of a filled row, the way the logo's
/// letters are darker at the bottom of their gradient than at the top.
fn shade(colour: Color32, by: f32) -> Color32 {
    let cut = |a: u8| (a as f32 * by).round() as u8;
    Color32::from_rgb(cut(colour.r()), cut(colour.g()), cut(colour.b()))
}

/// Text with the logo's hard shadow under it: the same words twice, plum first, offset by
/// a couple of points, never blurred — the way the wordmark's shadow is printed.
fn shadow_text(
    painter: &Painter,
    at: Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    ink: Color32,
) {
    // The shadow fades with its word: a shadow at full strength under a word still
    // arriving would be a second, darker word arriving early.
    let shadow = PLUM.gamma_multiply(ink.a() as f32 / 255.0);
    painter.text(at + Vec2::new(2.0, 2.0), align, text, font.clone(), shadow);
    painter.text(at, align, text, font, ink);
}

/// The menu panel and everything in it.
///
/// The plate is dressed the way the logo is: a warm white face in a thick plum outline
/// with a hard slate shadow under it, square-ish corners, and a grass tab on its top edge
/// naming what it is. Every row is a piece of the same furniture, chosen or not: a band
/// tinted with its door's colour out of the logo's cycle, a square colour chip with the
/// letter's white top rim, the name in the pixel face, and the key it answers to carried
/// on a little key cap with its own plum outline and slate shadow. The chosen row is the
/// same band at full strength — *filled* with its door's colour, shaded at its foot and
/// rimmed in white at the top, the way a letter of the wordmark is — so moving the cursor
/// raises a row rather than swapping it for a different object.
fn paint_menu(painter: &Painter, panel: Rect, selected: usize, arrived: f32, clock: f32) {
    let ink = |colour: Color32| colour.gamma_multiply(arrived);
    let shift = Vec2::new(0.0, (1.0 - arrived) * 12.0);
    let panel = panel.translate(shift);
    let radius = CornerRadius::same(4);

    // The tab: a tab of the lawn's own green stuck on the box's top edge, the way a menu
    // screen labels itself. It is drawn first so the box's outline closes over its bottom
    // and the tab reads as stuck *onto* the plate rather than as a second panel floating
    // above it. Its shadow is the logo's slate, hard and offset, its face is rimmed with
    // white along the top like a letter, and its word carries the plum shadow.
    let tab_w = 196.0;
    let tab = Rect::from_min_size(
        Pos2::new(panel.center().x - tab_w * 0.5, panel.top() - 26.0),
        Vec2::new(tab_w, 36.0),
    );
    painter.rect_filled(tab.translate(Vec2::new(4.0, 4.0)), radius, ink(SLATE));
    painter.rect_filled(tab, radius, ink(Color32::from_rgb(104, 186, 68)));
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(tab.left() + 8.0, tab.top() + 2.0),
            Vec2::new((tab.width() - 16.0).max(0.0), 2.0),
        ),
        CornerRadius::ZERO,
        ink(Color32::from_white_alpha(140)),
    );
    painter.rect_stroke(
        tab,
        radius,
        Stroke::new(3.0_f32, ink(PLUM)),
        egui::StrokeKind::Inside,
    );
    shadow_text(
        painter,
        tab.center(),
        Align2::CENTER_CENTER,
        "SELECT",
        theme::pixel_font(10.0),
        ink(Color32::WHITE),
    );

    // The box: a warm white plate with the logo's hard slate shadow and thick plum
    // outline. Two rings rather than the chrome's three — the logo has no gold hairline,
    // and the plate is dressed as the logo's furniture, not the editor's.
    painter.rect_filled(panel.translate(Vec2::new(6.0, 7.0)), radius, ink(SLATE));
    painter.rect_filled(panel, radius, ink(PANEL_FACE));
    painter.rect_stroke(
        panel,
        radius,
        Stroke::new(4.0_f32, ink(PLUM)),
        egui::StrokeKind::Inside,
    );

    // The options. The cursor blinks on the *frame's* clock, a hard on-and-off rather than a
    // fade: every game that ever had a menu blinked its cursor, and a breath reads as a glow
    // rather than as an invitation to press something.
    let blink = (clock * 2.2).fract() < 0.62;
    for (index, door) in DOORS.iter().enumerate() {
        let line = line_rect(panel, index);
        let chosen = index == selected;
        let band = line.shrink2(Vec2::new(6.0, 3.0));
        // Every row casts the slate shadow, chosen or not: a row is a tile lying on the
        // plate, and a tile that only casts a shadow while the cursor is on it is two
        // different objects taking turns.
        painter.rect_filled(band.translate(Vec2::new(3.0, 3.0)), radius, ink(SLATE));
        if chosen {
            // The chosen line at full strength: filled with the door's colour, shaded at
            // its foot, rimmed with white along the top and outlined in the logo's plum —
            // a letter of the wordmark, laid on its side as a row.
            painter.rect_filled(band, radius, ink(door.colour));
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(band.left() + 3.0, band.bottom() - 8.0),
                    Pos2::new(band.right() - 3.0, band.bottom() - 3.0),
                ),
                CornerRadius::ZERO,
                ink(shade(door.colour, 0.82)),
            );
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(band.left() + 8.0, band.top() + 2.0),
                    Vec2::new((band.width() - 16.0).max(0.0), 2.0),
                ),
                CornerRadius::ZERO,
                ink(Color32::from_white_alpha(160)),
            );
            painter.rect_stroke(
                band,
                radius,
                Stroke::new(3.0_f32, ink(PLUM)),
                egui::StrokeKind::Inside,
            );
        } else {
            // The unchosen line at rest: the same band, tinted with the door's colour
            // rather than filled with it, under a quieter plum outline.
            painter.rect_filled(band, radius, ink(blend(door.colour, PANEL_FACE, 0.30)));
            painter.rect_stroke(
                band,
                radius,
                Stroke::new(2.0_f32, ink(blend(PLUM, PANEL_FACE, 0.55))),
                egui::StrokeKind::Inside,
            );
        }
        // The cursor: a solid plum triangle pointing at the line, blinking.
        if chosen && blink {
            let cursor = cursor_points(Pos2::new(panel.left() + 27.0, line.center().y));
            painter.add(egui::Shape::convex_polygon(
                cursor.to_vec(),
                ink(PLUM),
                Stroke::NONE,
            ));
        }
        // The door's colour chip: a square of its colour with a plum outline and the
        // letter's white top rim. On the chosen row the fill is already the band's own
        // colour, so the chip is the plate's white with the colour at its heart instead —
        // the same chip, read against a louder ground.
        let chip = Rect::from_min_size(
            Pos2::new(panel.left() + 44.0, line.center().y - 11.0),
            Vec2::new(22.0, 22.0),
        );
        if chosen {
            painter.rect_filled(chip, CornerRadius::same(2), ink(PANEL_FACE));
            painter.rect_filled(
                chip.shrink(6.0),
                CornerRadius::ZERO,
                ink(door.colour),
            );
        } else {
            painter.rect_filled(chip, CornerRadius::same(2), ink(door.colour));
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(chip.left() + 3.0, chip.top() + 2.0),
                    Vec2::new((chip.width() - 6.0).max(0.0), 2.0),
                ),
                CornerRadius::ZERO,
                ink(Color32::from_white_alpha(160)),
            );
        }
        painter.rect_stroke(
            chip,
            CornerRadius::same(2),
            Stroke::new(2.0_f32, ink(PLUM)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            Pos2::new(panel.left() + 78.0, line.center().y),
            Align2::LEFT_CENTER,
            door.name,
            theme::pixel_font(15.0),
            ink(PLUM),
        );
        // The key, on a cap: a little plate of the panel's own white with the slate
        // shadow and the plum outline every other object on this screen wears, so the
        // hint reads as a *key* rather than as a footnote that wandered onto the row.
        let key_font = theme::pixel_font(8.0);
        let galley = painter.layout_no_wrap(door.key.to_string(), key_font.clone(), ink(PLUM));
        let cap = Rect::from_min_size(
            Pos2::new(
                panel.right() - 16.0 - (galley.size().x + 18.0),
                line.center().y - 11.0,
            ),
            Vec2::new(galley.size().x + 18.0, 22.0),
        );
        painter.rect_filled(
            cap.translate(Vec2::new(2.0, 2.0)),
            CornerRadius::same(2),
            ink(SLATE),
        );
        painter.rect_filled(cap, CornerRadius::same(2), ink(PANEL_FACE));
        painter.rect_stroke(
            cap,
            CornerRadius::same(2),
            Stroke::new(2.0_f32, ink(PLUM)),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            Pos2::new(
                cap.center().x - galley.size().x * 0.5,
                cap.center().y - galley.size().y * 0.5,
            ),
            galley,
            ink(PLUM),
        );
    }

    // The description plaque: one sentence about the highlighted door, the way a console
    // menu explains the file under the cursor rather than putting a sentence on every
    // line — carried on a plate of its own, tinted with the chosen door's colour, with a
    // square of that colour standing in for a bullet.
    let top = panel.top() + PANEL_PAD + LINE_H * DOORS.len() as f32;
    let plaque = Rect::from_min_max(
        Pos2::new(panel.left() + 14.0, top + 5.0),
        Pos2::new(panel.right() - 14.0, top + DESC_H - 3.0),
    );
    painter.rect_filled(
        plaque.translate(Vec2::new(3.0, 3.0)),
        radius,
        ink(SLATE),
    );
    painter.rect_filled(
        plaque,
        radius,
        ink(blend(DOORS[selected].colour, PANEL_FACE, 0.30)),
    );
    painter.rect_stroke(
        plaque,
        radius,
        Stroke::new(2.0_f32, ink(blend(PLUM, PANEL_FACE, 0.55))),
        egui::StrokeKind::Inside,
    );
    let bullet = Rect::from_min_size(
        Pos2::new(plaque.left() + 12.0, plaque.center().y - 5.0),
        Vec2::new(10.0, 10.0),
    );
    painter.rect_filled(bullet, CornerRadius::ZERO, ink(DOORS[selected].colour));
    painter.rect_stroke(
        bullet,
        CornerRadius::ZERO,
        Stroke::new(2.0_f32, ink(PLUM)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        Pos2::new(plaque.left() + 34.0, plaque.center().y),
        Align2::LEFT_CENTER,
        DOORS[selected].blurb,
        theme::pixel_font(9.0),
        ink(PLUM),
    );
}

/// The cursor: a solid pixel triangle pointing at the chosen line.
fn cursor_points(at: Pos2) -> [Pos2; 3] {
    [
        Pos2::new(at.x - 6.0, at.y - 7.0),
        Pos2::new(at.x + 6.0, at.y),
        Pos2::new(at.x - 6.0, at.y + 7.0),
    ]
}

/// The blinking invitation under the box, and the keys that move the cursor.
///
/// "PRESS ENTER" rather than "press any key": the screen has a cursor now, so the honest
/// invitation is the key that takes it. Any *other* key still leaves for the editor, which is
/// what the Edit line's own `ANY KEY` chip says. Gold on a plum shadow, in the pixel face:
/// the way a game's score line is printed over the picture, and in the type its logo is set
/// in, so the invitation belongs to the same sign as the box above it.
fn prompt(painter: &Painter, rect: Rect, clock: f32) {
    let arrived = ((clock - HINT_AT - DOORS_LAG - 0.15) / 0.45).clamp(0.0, 1.0);
    if arrived <= 0.0 {
        return;
    }
    let blink = (clock * 1.5).fract() < 0.66;
    if blink {
        let font = theme::pixel_font(13.0);
        let at = Pos2::new(rect.center().x, rect.bottom() - 62.0);
        painter.text(
            at + Vec2::new(2.0, 2.0),
            Align2::CENTER_CENTER,
            "PRESS ENTER TO START",
            font.clone(),
            PLUM.gamma_multiply(arrived),
        );
        painter.text(
            at,
            Align2::CENTER_CENTER,
            "PRESS ENTER TO START",
            font,
            theme::ACCENT.gamma_multiply(arrived),
        );
    }
    shadow_text(
        painter,
        Pos2::new(rect.center().x, rect.bottom() - 42.0),
        Align2::CENTER_CENTER,
        MENU_HINT,
        theme::pixel_font(9.0),
        FOOT_INK.gamma_multiply(arrived),
    );
}

/// The bottom band: what this is, whose run it is, and what it is not.
///
/// Lettered like everything else on this screen, because a footer in the window's
/// monospace was a line of the *tool* showing through the game's front door: the game's
/// name in the pixel face in gold on the plum shadow, its version after it in the quiet
/// cream, and the four activities at the right each in the light tone of its own colour
/// family from the logo's cycle (Replay takes the grass family — it is the one activity
/// with no door of its own up above), separated by little squares of cream rather than by
/// a middot the pixel face does not carry.
fn footer(painter: &Painter, rect: Rect, credit: Option<&str>) {
    let base = rect.bottom() - 11.0;
    // The brand, at the left: the name in gold, then the version in the quiet ink, both on
    // the plum shadow the rest of the screen's stage type wears.
    let name_font = theme::pixel_font(9.0);
    let name_at = Pos2::new(rect.left() + 14.0, base);
    shadow_text(
        painter,
        name_at,
        Align2::LEFT_BOTTOM,
        "ONE TWO HUNDRED",
        name_font.clone(),
        theme::ACCENT,
    );
    let name_w = painter
        .layout_no_wrap("ONE TWO HUNDRED".to_string(), name_font, theme::ACCENT)
        .size()
        .x;
    painter.text(
        Pos2::new(name_at.x + name_w + 12.0, base),
        Align2::LEFT_BOTTOM,
        format!("V{}", env!("CARGO_PKG_VERSION")),
        theme::pixel_font(8.0),
        FOOT_INK,
    );
    // The credit is the one line down here worth reading, so it is the one line down here
    // in gold: the title's gold, held back far enough to stay a caption. Set in the pixel
    // face like the rest of the band, uppercased to match it, with the middots the face
    // does not carry traded for hyphens it does.
    if let Some(credit) = credit {
        shadow_text(
            painter,
            Pos2::new(rect.center().x, base),
            Align2::CENTER_BOTTOM,
            &credit.replace('·', "-").to_uppercase(),
            theme::pixel_font(8.0),
            theme::ACCENT.gamma_multiply(0.9),
        );
    }
    // The activities, at the right, laid out right to left so the row ends flush with the
    // window's edge: each word in its family's light tone, with a three-point square of
    // the quiet cream between one word and the next.
    let modes: [(&str, Color32); 4] = [
        ("EDIT", Color32::from_rgb(255, 214, 102)),
        ("PLAY", Color32::from_rgb(255, 186, 116)),
        ("EVOLVE", Color32::from_rgb(112, 200, 224)),
        ("REPLAY", Color32::from_rgb(150, 220, 96)),
    ];
    let mode_font = theme::pixel_font(8.0);
    let mut right = rect.right() - 14.0;
    for (word, colour) in modes.iter().rev() {
        let galley = painter.layout_no_wrap(word.to_string(), mode_font.clone(), *colour);
        let at = Pos2::new(right - galley.size().x, base);
        painter.text(at + Vec2::new(2.0, 2.0), Align2::LEFT_BOTTOM, word, mode_font.clone(), PLUM);
        painter.galley(
            Pos2::new(at.x, at.y - galley.size().y),
            galley.clone(),
            *colour,
        );
        right = at.x - 10.0;
        if word != &"EDIT" {
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(right + 2.0, base - 7.0), Vec2::new(3.0, 3.0)),
                CornerRadius::ZERO,
                FOOT_INK,
            );
            right -= 10.0;
        }
    }
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

    /// The logo is fully revealed before the invitation arrives to be read.
    ///
    /// Both halves matter: a mark that is still arriving when the "press enter" line is
    /// solid is a screen that is still assembling itself while it tells you to leave, and a
    /// reveal that takes longer than a person waits is a logo nobody reads.
    #[test]
    fn the_logo_arrives_before_the_invitation() {
        assert_eq!(logo_in(0.0), 0.0, "the logo is up before the clock is");
        assert_eq!(
            logo_in(LOGO_AT - 0.01),
            0.0,
            "the logo arrives before its moment"
        );
        let done = LOGO_AT + LOGO_FADE;
        assert_eq!(logo_in(done + 0.01), 1.0, "the logo never settles");
        assert!(
            done < HINT_AT,
            "the logo is still arriving at {done:.2}s, after the invitation at {HINT_AT}s"
        );
        // Halfway through the fade it is halfway there: the reveal is a ramp, not a cut,
        // because a picture that snaps in at full strength reads as a pop rather than as a
        // landing.
        let mid = logo_in(LOGO_AT + LOGO_FADE * 0.5);
        assert!(mid > 0.4 && mid < 0.6, "halfway reads {mid}");
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
        crate::theme::apply(&ctx);
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
        crate::theme::apply(&ctx);
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

    /// The screen draws the world, the logo and the words, and the world changes with the
    /// clock.
    ///
    /// Read back the way the renderer left it — the tessellated shapes, counted by which
    /// texture they sampled — rather than by asking the code that drew them: the world is a
    /// great many atlas quads, the logo is a quad sampling its own texture, and the words are
    /// a great many glyphs from the font texture, and a clock that stops moving any of them
    /// is the one bug a splash can have that a person would notice immediately.
    #[test]
    fn the_splash_draws_the_world_and_the_words() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let size = Vec2::new(1360.0, 800.0);
        let rect = Rect::from_min_size(Pos2::ZERO, size);
        // The lockup's own band: the logo and the tagline under it, and nothing the footer
        // or the menu contributes. Counting *here* rather than counting everything is what
        // makes this a test of the reveal: the footer is on the screen from the first frame,
        // and a count of every glyph in the window would be a count of that.
        let line = Rect::from_min_size(
            Pos2::new(rect.left(), rect.height() * 0.08),
            Vec2::new(rect.width(), rect.height() * 0.32),
        );
        let counts = |clock: f32| -> (usize, usize, usize, usize) {
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
            // The frame just drawn uploaded the logo; its id is how its quad is known from
            // every other quad on the screen.
            let logo_id = splash.logo.as_ref().map(|handle| handle.id());
            let (mut world, mut words, mut title, mut logo) = (0, 0, 0, 0);
            for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
                let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                    continue;
                };
                if mesh.texture_id == atlas.texture_id() {
                    world += mesh.vertices.len() / 4;
                    continue;
                }
                if Some(mesh.texture_id) == logo_id {
                    for quad in mesh.vertices.chunks(4) {
                        if quad.len() < 4 {
                            continue;
                        }
                        let centre =
                            quad.iter().fold(Pos2::ZERO, |a, v| a + v.pos.to_vec2()) / 4.0;
                        if line.contains(centre) {
                            logo += 1;
                        }
                    }
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
                    // the sky is drawn with are meshes too, and they land on the lockup's own
                    // band, so counting them would count scenery as type.
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
            (world, words, title, logo)
        };

        let (world, words, title, logo) = counts(2.4);
        assert!(
            world > 40,
            "the splash drew {world} sprite quads: the world is not on the screen"
        );
        assert!(
            words > 60,
            "the splash drew {words} glyph quads: the words are not on the screen"
        );
        assert!(
            logo >= 1,
            "the splash drew {logo} logo quads on the lockup's band: the logo is not on the screen"
        );
        assert!(
            title >= 30,
            "{title} glyphs on the lockup's band: the tagline alone is more than that"
        );
        let (_, _, early_words, early_logo) = counts(0.0);
        assert_eq!(early_words, 0, "the tagline is already up at the first frame");
        assert_eq!(early_logo, 0, "the logo is already up at the first frame");
        let (later_world, _, _, _) = counts(6.0);
        assert!(
            later_world != world,
            "the world is the same picture at 2.4s and 6.0s: nothing in it is running"
        );
    }
}
