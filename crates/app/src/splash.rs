//! The splash screen: the window's first frame, and what it says about itself.
//!
//! Without one the shell opens straight into the editor on a level, which is the right first
//! thing for a person who already knows what the four tabs are and a cold start for everyone
//! else. So the first frame is this: the game's own picture — the same sky gradient, the same
//! parallax hills and drifting clouds, the same grass, trees and butterflies, drawn by the
//! same functions — and the whole thing is **running**. The name is set in the sky over it in
//! the window's pixel face, with a tagline under it and a framed menu beneath the level; an
//! agent runs the little level built for the occasion, clearing a pit, hopping a block and
//! climbing the hill to the flag. The scene holds on the waving flag for a moment before
//! starting again, so what a person watches while deciding to press a key is the game being
//! played rather than a picture of it.
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
//! starts a run, and every other key — or a click — opens the editor. Those shortcuts *do*
//! what they say, rather than being hints for a screen that would consume the press and drop
//! the viewer into the editor anyway. The window is playable from
//! the frame after that, and nothing about the splash is reachable again except by restarting
//! the app. A stray event from the window manager — a focus click, a key repeat — must not eat
//! the title before a person has seen it, which is what [`ARMED`] is for.
//!
//! **Whose run it is.** A run can leave two artifacts for the level this window opened on, and
//! the screen prefers the committee when it is there *and passes its gate*: the persisted
//! `committees/<level>.committee.json` (Item 10), validated against its best member on the run's
//! fixed validation episodes, with each member keeping its own recurrent state (see
//! [`CommitteeRun`]). A committee that fails the gate, or whose file is missing or from another
//! build, falls through to the champion: the genome out of the champion file, deciding every
//! `action_repeat` ticks through the same simulation the trainer used (see [`ChampionRun`]) — so
//! the picture is the episode the file records rather than an illustration of one, and the
//! credit line under it says *whose*. Otherwise — no artifact for this level, a file from
//! another build, or a best-ever agent that never reached the flag — the screen is the little
//! level and the script, which is what it always showed, and which is deliberately as good as
//! the champion path rather than a consolation prize.

use std::path::Path;

use egui::{Align2, Color32, Context, CornerRadius, Painter, Pos2, Rect, Stroke, Vec2};
use evo::{ActScratch, ChampionFile, Committee, CommitteeFile, Genome};
use sim::{AgentAction, Level, Sim, Tile};

use crate::atlas::Atlas;
use crate::play::{self, Keys};
use crate::theme;
use crate::ui;
use crate::view::{self, Camera, RenderOpts, Sheet, Sprites};

/// The wordmark is **set, not pasted**.
///
/// The screen used to draw `crates/app/assets/logo.png` — rainbow letters over a strip of
/// ground — as a picture hanging in the sky. On a title card it read as a decal: a baked strip of
/// terrain floating over the clouds, lettering in six colours that the game itself never uses,
/// and no relationship to the type in the menu two hundred points below it. A title screen's job
/// is to look like the game it opens, so the name is drawn in the same face every other word on
/// this screen is drawn in — Press Start 2P, the pixel face the window already embeds — and made
/// the way the game's own objects are: cream letters with a hard brown edge, standing on a shelf
/// of the accent gold, the way the menu's plates sit on the palette's rim.
///
/// Two lines, because the name is three words and the big one is the last: `ONE TWO` sits over
/// `HUNDRED` the way a box's title does, and the size difference is the whole of the hierarchy.
///
/// Both lines are **set to one measure**, the way a lockup is: `HUNDRED` sets the width, and a
/// gold rule either side of `ONE TWO` carries that width up to the small line. That single rule
/// is most of what separates a logo from two words in a font — the block of type has a shape of
/// its own, and the small line stops looking like a caption somebody forgot to centre. (The
/// first attempt stretched `ONE TWO` itself out to the measure with tracking, and at that
/// spacing its letters came apart into shapes with holes between them.)
const TITLE_ONE: &str = "ONE TWO";
const TITLE_TWO: &str = "HUNDRED";
/// The hard edge around every letter, as a fraction of its size, and how far the block it
/// stands on comes forward.
///
/// The block is measured in the **face's own pixels** — the eight-by-eight grid the letterforms
/// were drawn on — not in screen pixels. The first version extruded the letters one screen pixel
/// at a time and the side came out smeared: sixteen antialiased copies of a curve make a grey
/// edge, and a pixel logo's side is supposed to be made of squares. A grid pixel and a quarter
/// is deeper than the face's own strokes, so the side reads as a side, and shallow enough that
/// the letters are still the word.
const TITLE_EDGE: f32 = 0.5;
const TITLE_BLOCK: f32 = 1.25;
/// How far the small line is tracked beyond the face's own fit — a hair, so `ONE TWO` is a
/// deliberate line of letters rather than a default one.
const TITLE_TRACK: f32 = 1.03;
/// How much air a space gets, over and above the tracking one glyph already carries — enough
/// that `ONE TWO` reads as two words at the tracking the lockup needs.
const TITLE_WORD_GAP: f32 = 0.55;
/// The only instruction the screen needs: move the selection, then take it.
///
/// Arrow glyphs are not in the pixel face, so the keys are spelled out.
const MENU_HINT: &str = "UP / DOWN   CHOOSE      ENTER   GO";

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
/// When the title starts arriving, and how long its whole entrance takes.
///
/// The wordmark fades up and drifts a few points into place. Nothing assembles letter by letter
/// and nothing bounces: the screen's motion is the episode running behind the name.
const TITLE_AT: f32 = 0.15;
const TITLE_FADE: f32 = 0.70;
/// When the menu arrives. The mark has settled by then.
const HINT_AT: f32 = 1.15;
/// How long the finished episode is held before the little run starts over.
const HOLD: f32 = 1.15;
/// How long a dismissing event is ignored at the very start.
const ARMED: f32 = 0.35;
/// How far above the middle of the window the level is drawn, as a fraction of the window's
/// height.
///
/// A level fitted to the middle of a window this wide leaves two even bands of sky, and the
/// screen it is behind needs them unequal: the title wants sky above and below it, and the
/// menu needs a band of its own under the ground rather than the lawn's brown. Pushing
/// the world up is one number and it is what turns the layer under the ground from a state
/// the fit happened to land on into a *floor for the text*.
const RAISED: f32 = 0.13;
/// Where the ground sits in the window when a champion is playing, as a fraction of the
/// window's height.
///
/// Low enough that the lawn, the props and the agent are in the lower third of a window the
/// sky and its parallax fill above them, high enough to leave a band of its own under the
/// ground for the menu. See [`Splash::target`].
const GROUND_LINE: f32 = 0.60;
/// The seed of the splash's episode, so the splash is the same splash every launch.
const SEED: u64 = 4;
/// The smallest visible gap between the runner's head and the top of the window.
///
/// The climb is tall enough that a full jump from the summit would otherwise carry the agent
/// out of frame. The camera eases down only when the body gets this close to the top edge.
const TOP_MARGIN: f32 = 40.0;

/// The prompt's ink: warm cream, because the band under the level is dark.
const FOOT_INK: Color32 = Color32::from_rgb(222, 204, 172);
/// The stage: near-black, blue-shifted, so the gold on it reads warm.
const STAGE_INK: Color32 = Color32::from_rgb(54, 38, 24);

/// When the menu is clickable: a beat after it starts appearing.
///
/// A row is a hit target as well as a pixel, and a target that is half transparent on the
/// frame it becomes clickable is a target somebody clicks twice.
const DOORS_LAG: f32 = 0.20;

/// The panel the menu is drawn in: a framed box in the band under the level.
///
/// Wide enough for the longest option at the size it is set, and no wider. A menu is an
/// object, not a toolbar stretched across the window.
const MENU_W: f32 = 430.0;
/// One option line's height.
const LINE_H: f32 = 56.0;
/// The panel's padding.
const PANEL_PAD: f32 = 12.0;
/// The square a door's icon is drawn in, and how far it sits from the plate's left edge.
const MENU_ICON: f32 = 26.0;
const MENU_ICON_LEFT: f32 = 26.0;
/// Where a door's word starts, measured from the plate's left edge: past the icon and its gap.
const MENU_LABEL_LEFT: f32 = 58.0;
/// Padding from the window's bottom edge, for the box and the prompt under it.
const MENU_BOTTOM_PAD: f32 = 56.0;
/// The prompt's baseline above the window's bottom edge.
const PROMPT_BOTTOM: f32 = 42.0;
/// The champion credit's baseline above the window's bottom edge — above the footer's own line,
/// and centred so it reads under the menu rather than colliding with the build number.
const CREDIT_BOTTOM: f32 = 24.0;

/// One door: what it is called and where it leads.
///
/// Edit first because it is what most launches want, then play it yourself, then teach the
/// population. The descriptions, colour chips and key caps the screen used to carry are gone:
/// three verbs are enough to say what the three activities are.
struct Door {
    name: &'static str,
    action: Action,
    /// The sprite that stands for the activity, out of the shipped atlas.
    ///
    /// The same four pictures the window's own mode switch uses (`ground_1`, `player_run_1`,
    /// `walker_a`), because the front door and the dock are the same menu seen from two sides:
    /// a person who learned "the ground tile is Edit" on this screen should not have to learn
    /// it again one keypress later. The icons are the reason the rows read as a game's menu
    /// rather than as three words in a box.
    icon: &'static str,
}

const DOORS: [Door; 3] = [
    Door {
        name: "EDIT",
        action: Action::Edit,
        icon: "ground_1",
    },
    Door {
        name: "PLAY",
        action: Action::Play,
        icon: "player_run_1",
    },
    Door {
        name: "EVOLVE",
        action: Action::Evolve,
        icon: "walker_a",
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
    /// menu are all functions of it, so two frames at the same instant are the same frame.
    clock: f32,
    /// Seconds left of the hold after the episode ends, and then it starts again.
    hold: f32,
    /// Which door the keyboard is on: an index into [`DOORS`].
    ///
    /// One selection with two ways to move it. The pointer sets it on hover, so the filled
    /// row is under the cursor, and up/down set it, so the fill is where the next Enter will
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
    /// **The run's committee (Item 10), when it passed its gate.** Preferred over the single
    /// champion because the vote is the run's result rather than one member of it — and only
    /// ever shown after [`Committee::validate`] said it never loses to its best member on the
    /// run's fixed validation episodes. A file that fails the gate falls back to the champion.
    Committee(CommitteeRun),
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
    /// Whose run this is, for the caption under the menu.
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

/// A saved committee, being played back.
///
/// The same shape as [`ChampionRun`] with a vote where the genome was: each member keeps its own
/// recurrent state inside the committee, and the action is taken on the same cadence the run
/// trained at. The members are in the file's leaderboard order, which is also the tie-break, so
/// the picture is reproducible from the file alone.
struct CommitteeRun {
    committee: Committee,
    action: AgentAction,
    repeat: u32,
    seed: u64,
    credit: String,
}

impl CommitteeRun {
    fn decide(&mut self, sim: &Sim) -> AgentAction {
        if sim.stats().frames % self.repeat == 0 {
            self.action = self.committee.act(&sim.observation());
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
        // Item 10's display half: a committee that passed its gate is preferred, because it is
        // the run's result rather than one member of it. The gate is a real exam — twelve fixed
        // validation episodes against the best member — and a file that fails it falls through
        // to the champion rather than being shown as the level's solver.
        if let Some(run) = committee_run(level, champions, level_path) {
            let mut session = play::Session::new(level, run.seed);
            session.set_audible(false);
            return Splash {
                level: level.clone(),
                session,
                policy: Policy::Committee(run),
                clock: 0.0,
                hold: 0.0,
                selected: 0,
            };
        }
        let Some(run) = champion_run(level, champions, level_path) else {
            return Splash::scripted();
        };
        // The champion's level *is* this level: `check_compat` refuses a file whose level
        // hash is not this document's, so the picture and the editor are looking at the same
        // terrain, which is the point of playing the run's own agent at the front door.
        let mut session = play::Session::new(level, run.seed);
        // The demo plays the game at the viewer; it does not play its sounds at
        // them. Menu keys still blip (see `show`), the running does not.
        session.set_audible(false);
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
        let mut session = play::Session::new(&level, SEED);
        session.set_audible(false);
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
    /// A screenshot cannot show a reveal, and the opening frame has the runner still on the
    /// left. `--splash --screenshot` therefore asks for the splash at the moment the demo run
    /// is over the pit, which is the frame a person would photograph if they were showing
    /// somebody the screen. The episode is advanced as well, so the runner in that frame is
    /// running rather than standing on the spawn tile.
    pub fn skip_opening(&mut self) {
        // Two and a half seconds puts the runner past the pit on the demo level; a champion's
        // level lands wherever its own episode is at that instant.
        let skip = 2.6;
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
            Policy::Committee(run) => {
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
            Policy::Committee(run) => Some(run.credit.as_str()),
            Policy::Script => None,
        }
    }

    /// Draw the screen, and say which way the viewer asked to go — or `None` if they have not
    /// asked yet.
    ///
    /// The picture first, then the stage, then the menu: the level is the screen's *subject*,
    /// and everything drawn over it is furniture arranged around it.
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
        // level, then the agent with its dust. The level's own bounds box is the one thing the
        // play view draws that belongs to a document rather than to a picture, so it is off.
        // The title is painted after the world, in screen space, so it is a card in front of
        // the picture rather than a piece of the terrain.
        view::draw_background(sheet, &self.level, view::Backdrop::Sky, self.clock);
        let opts = RenderOpts {
            bounds: false,
            ..RenderOpts::playing()
        };
        let scripted = matches!(self.policy, Policy::Script);
        if scripted {
            view::draw_level(sheet, &self.level, opts, None, self.clock);
            view::draw_agents(
                sheet,
                self.session.sim(),
                self.clock,
                Color32::WHITE,
                self.session.anim(),
                true,
            );
        } else {
            view::draw_sim(
                sheet,
                self.session.sim(),
                opts,
                self.clock,
                self.session.anim(),
            );
        }

        // The picture is framed like a title card: the corners fall away, the wordmark hangs
        // in the sky above the level, and the band under the level's own floor is the stage the
        // menu stands on. The vignette goes down *before* the mark so the title is the
        // brightest thing on the screen rather than the thing the dark edges are cutting into.
        vignette(&painter, rect);
        title(&painter, rect, self.clock);

        let floor = sheet
            .camera
            .to_screen(0.0, (self.level.height - 1) as f32, rect)
            .y;
        stage(&painter, rect, floor);
        footer(&painter, rect, self.clock);

        let panel = menu_panel(rect);
        let from_menu = menu(
            ctx,
            &painter,
            rect,
            panel,
            &mut self.selected,
            self.clock,
            atlas,
        );
        champion_credit(&painter, rect, self.credit());
        // The sound switch belongs on the front door too: the music is already
        // playing here, and the alternative is asking a person to start the
        // game to turn it down.
        let sound_rect = sound_button(ctx, rect, atlas);

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
                        key, pressed: true, ..
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
                            // One key that does not start the game: the music
                            // is already playing, and muting it should not
                            // cost the viewer the title screen.
                            egui::Key::M => {
                                crate::sound::toggle_mute();
                                Pressed::None
                            }
                            _ => Pressed::Go(Action::Edit),
                        };
                    }
                    // A press *on a row* is not a decision either, and this is the one press
                    // the rule below has to step aside for: a row answers on the release, so
                    // taking the press here would start the editor out from under the pointer
                    // and the click would arrive somewhere it was not aimed.
                    egui::Event::PointerButton {
                        pressed: true, pos, ..
                    } if menu_is_up(self.clock) && panel.expand(10.0).contains(*pos) => {
                        continue;
                    }
                    // The sound plate is a control, not a start button: its
                    // own press toggles on release, and this press must not
                    // also dismiss the screen.
                    egui::Event::PointerButton {
                        pressed: true, pos, ..
                    } if sound_rect.contains(*pos) => {
                        continue;
                    }
                    egui::Event::PointerButton { pressed: true, .. } => {
                        return Pressed::Go(Action::Edit)
                    }
                    _ => {}
                }
            }
            Pressed::None
        });
        match pressed {
            Pressed::Move(next) => {
                if next != self.selected {
                    crate::sound::play(crate::sound::Sfx::MenuMove, 1.0);
                }
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
    /// nothing under it — and pinning it leaves the band under the ground for the menu.
    ///
    /// `max` with the fit rather than the follow zoom alone, so the same code shows a *small*
    /// level whole: a level that already fits inside the follow zoom is not worth tracking.
    fn target(&self, rect: Rect) -> Camera {
        match &self.policy {
            Policy::Script => {
                let mut camera = framed(&self.level, rect);
                let p = self.session.sim().player();
                let head = camera.to_screen(p.x, p.y, rect).y;
                if head < TOP_MARGIN {
                    camera.centre.y -= (TOP_MARGIN - head) / camera.zoom;
                }
                camera
            }
            Policy::Champion(_) | Policy::Committee(_) => {
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

/// The whole level, as the splash frames it: fitted, then raised for the menu.
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
    // Ten rows of sky over a two-row lawn, and forty tiles long: long enough that a person
    // watching sees a *level* running past rather than a diorama, short enough that the fit
    // still draws the runner at a readable size on a wide window.
    //
    // The shape is the whole of the game in three moves, in the order a player meets them: a
    // run, a pit to clear, a block to climb, and a low hill up to the flag. The old scene put
    // a nineteen-tile stone pyramid in the middle for the wordmark to stand on, and the picture
    // read as a grey mountain with a logo on it: the name now hangs in the sky where a title
    // belongs, and the ground does what ground in this game does.
    let mut level = Level::starter("splash", 40, 15);
    for tx in 0..level.width {
        level.set_tile(tx, 13, Tile::Solid);
        level.set_tile(tx, 14, Tile::Solid);
    }
    // `starter` placed the spawn and the flag on the single row it knows about; with the
    // floor two rows lower they move down with it. A goal tile is the *pole* — the lower of
    // the flag's two cells, and the one that has to have ground under it — so its row is the
    // one above the hill's summit.
    level.set_spawn_tile(1, 12);

    // The pit: two tiles of nothing in the lawn, wide enough that the script has to commit to
    // the jump and narrow enough that the game's own jump clears it with room to spare.
    for tx in 14..16 {
        level.set_tile(tx, 13, Tile::Empty);
        level.set_tile(tx, 14, Tile::Empty);
    }
    // The block: a chest-high step, which is the second thing every level in this pack asks
    // for and the reason the script reads the tile above the runner's feet.
    for tx in 22..25 {
        for ty in 11..13 {
            level.set_tile(tx, ty, Tile::Solid);
        }
    }
    // The finish: a two-tile hill with the flag on top, so the run ends on a rise with the
    // flag against the sky instead of on the flat at the edge of the frame.
    for tx in 31..37 {
        level.set_tile(tx, 12, Tile::Solid);
    }
    for tx in 33..37 {
        level.set_tile(tx, 11, Tile::Solid);
    }
    level.set_goal_tile(35, 10);
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
/// Four things are checked, and each of them is there because the alternative is a title
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
/// * **It was learned.** A generation-zero winner is the seed population's best, not a run
///   that learned anything; the demo is the better introduction.
///
/// Every refusal is silent, on purpose. The file is a build artifact in a directory the
/// window was told to look in, and a title screen that refused to open over a stale one would
/// be a worse bug than the one it would be reporting. The fallback is the demo run, and the
/// tests pin each refusal.
/// A validated committee for this level, if one is on disk and passes its gate.
///
/// The directory is the sibling of the champion directory the app was handed, so a run that
/// writes `committees/<level>.committee.json` (which is what `train --committee` does) is found
/// without a second configuration knob. Validation is the condition for showing it: the menu's
/// caption cites the counts, and a vote that lost an episode to its best member is not the
/// level's solver.
fn committee_run(
    level: &Level,
    champions: &Path,
    level_path: Option<&Path>,
) -> Option<CommitteeRun> {
    let dir = champions.with_file_name(evo::COMMITTEE_DIR);
    let mut names = Vec::new();
    if let Some(stem) = level_path.and_then(|path| path.file_stem()) {
        names.push(stem.to_string_lossy().to_string());
    }
    names.push(crate::lab::slug(&level.name));
    names.dedup();
    names.iter().find_map(|name| {
        let file = CommitteeFile::load(dir.join(format!("{name}.committee.json"))).ok()?;
        file.check_compat(level).ok()?;
        let report = file.validate(level).ok()?;
        if report.episodes == 0 || !report.passes() {
            return None;
        }
        let committee = file.to_committee().ok()?;
        Some(CommitteeRun {
            committee,
            action: AgentAction::Walk,
            repeat: file.action_repeat.max(1),
            seed: file.master_seed,
            credit: format!(
                "committee · {} members · {}/{} validation wins",
                file.members.len(),
                report.won,
                report.episodes
            ),
        })
    })
}

fn champion_run(level: &Level, champions: &Path, level_path: Option<&Path>) -> Option<ChampionRun> {
    let champion = champion_file(level, champions, level_path)?;
    let (genome, result) = evo::champion::replay(level, &champion).ok()?;
    // A generation-zero winner is the seed population's best agent, not a learned run. It
    // belongs on the level it happened to solve, not on the front door; the demo is a better
    // introduction and is what a fresh checkout gets.
    if !result.won() || champion.metadata.generation == 0 {
        return None;
    }
    Some(ChampionRun {
        genome,
        scratch: ActScratch::new(),
        action: AgentAction::Walk,
        repeat: champion.metadata.action_repeat.max(1),
        seed: champion.metadata.episode_seed,
        credit: format!(
            "champion · gen {} · fitness {:.0}",
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

/// How far the title's entrance has run: 0 not yet, 1 settled.
///
/// A pure function of the clock, so the reveal can be tested without a screen, and so the
/// wordmark being settled before the menu arrives is a fact about the timing constants rather
/// than something to eyeball.
fn title_in(clock: f32) -> f32 {
    let t = ((clock - TITLE_AT) / TITLE_FADE).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How big the two lines of the wordmark are, from the window's width.
///
/// The big line is the one that has to fit: `HUNDRED` is seven characters of a face whose glyphs
/// are square, so the size *is* roughly width-over-seven, and the small line follows at half.
/// The clamp keeps the title a title on a laptop and on a wall rather than letting a wide window
/// turn it into a banner; the small line is deliberately half the big one rather than a third,
/// because at a third its letterforms are a texture and not words.
///
/// Sizes are snapped to multiples of eight, the grid the face was drawn on. At any other size the
/// stems land between pixels and the renderer shades their edges — a whole word of grey fringes,
/// on a screen whose art is entire pixels and whose type is supposed to be, too.
fn title_sizes(rect: Rect) -> (f32, f32) {
    let fit = (rect.width() * 0.060 / 8.0).round() * 8.0;
    let big = fit.clamp(40.0, 120.0);
    (big * 0.50, big)
}

/// One pixel of the face's own grid at `size`: the face is drawn on an eight-by-eight cell, so
/// this is the width of the marks a letterform is built from.
fn title_grid(size: f32) -> f32 {
    (size / 8.0).max(1.0).round()
}

/// The width of one cell of the pixel face at `size` — the glyph's own advance, measured once.
///
/// Everything about the lockup's spacing comes from this number rather than from a guessed ratio:
/// a face whose advance is not exactly its size still gets a title whose letters sit where the
/// eye expects them. Measured on two glyphs, because one galley's width can carry the leading
/// of the pair and two halves the error.
fn title_cell(painter: &Painter, size: f32) -> f32 {
    let galley = painter.layout_no_wrap("00".to_string(), theme::pixel_font(size), Color32::WHITE);
    galley.size().x * 0.5
}

/// How much ink `text` takes when set at `size`, at the lockup's own tracking.
fn title_width(painter: &Painter, text: &str, size: f32) -> f32 {
    let cell = title_cell(painter, size) * TITLE_TRACK;
    text.chars()
        .map(|c| {
            if c == ' ' {
                cell * (1.0 + TITLE_WORD_GAP)
            } else {
                cell
            }
        })
        .sum()
}

/// One line of the lockup: a block extrusion, a hard edge, and a cream face.
///
/// Drawn a glyph at a time, because the spacing here is the lockup's and not the font's, and in
/// plates rather than one pass per word: the block's outline, the block, the letters' outline,
/// the letters. That ordering is what makes a pixel title without a bevel font — the letters
/// keep a hard silhouette in the frame's brown and a solid gold side, so a 60-point letter is a
/// made object rather than a white slab. The return value is the line's width, so the caller can
/// hang a rule off its edges.
fn title_line(
    painter: &Painter,
    mid: Pos2,
    text: &str,
    size: f32,
    face: Color32,
    block: Color32,
    arrived: f32,
) -> f32 {
    let font = theme::pixel_font(size);
    let alpha = |colour: Color32, a: f32| {
        Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), (a * 255.0) as u8)
    };
    let cell = title_cell(painter, size) * TITLE_TRACK;
    let width = title_width(painter, text, size);
    let mut cursor = mid.x - width * 0.5;
    let mut places = Vec::new();
    for ch in text.chars() {
        let step = if ch == ' ' {
            cell * (1.0 + TITLE_WORD_GAP)
        } else {
            cell
        };
        if ch != ' ' {
            places.push((ch, Pos2::new((cursor + step * 0.5).round(), mid.y.round())));
        }
        cursor += step;
    }

    let grid = title_grid(size);
    let edge = (grid * TITLE_EDGE).round().max(1.0);
    let depth = (grid * TITLE_BLOCK).round().max(2.0);
    // A *plate* is the line drawn as a solid shape: the glyphs themselves, plus the same
    // glyphs eight times around each one, which thickens the silhouette by `grow` in every
    // direction. Whole pixels, so the edge stays crisp where a stroked outline would go soft.
    let plate = |colour: Color32, off: Vec2, grow: f32| {
        let ring = if grow > 0.0 {
            vec![
                (-grow, 0.0),
                (grow, 0.0),
                (0.0, -grow),
                (0.0, grow),
                (-grow, -grow),
                (grow, -grow),
                (-grow, grow),
                (grow, grow),
            ]
        } else {
            Vec::new()
        };
        for (dx, dy) in ring {
            for (ch, at) in &places {
                painter.text(
                    *at + off + Vec2::new(dx, dy),
                    Align2::CENTER_CENTER,
                    *ch,
                    font.clone(),
                    alpha(colour, arrived),
                );
            }
        }
        for (ch, at) in &places {
            painter.text(
                *at + off,
                Align2::CENTER_CENTER,
                *ch,
                font.clone(),
                alpha(colour, arrived),
            );
        }
    };

    // The mark is three plates, back to front: the block's own outline in the frame's brown,
    // the block in the accent, and the letters on top. That ordering is the whole of the 3D —
    // the gold reads as the *side* of the letters because the front plate is outlined, so the
    // block shows as a band of gold on the bottom and right rather than as a pale echo of the
    // word behind it. The letters stand on a gold shelf, which is the one thing this screen's
    // furniture and its art already agree on.
    let back = Vec2::splat(depth);
    plate(theme::OUTLINE, back, edge * 2.0);
    plate(block, back, edge);
    plate(theme::OUTLINE, Vec2::ZERO, edge);
    plate(face, Vec2::ZERO, 0.0);
    width
}

/// Where the wordmark's top edge sits at rest, as a fraction of the window's height.
///
/// High in the sky, above the horizon the level draws, because that band is the title card: the
/// wordmark and its tagline are the only things on the screen between the top edge and the level,
/// and everything a person needs to read is below them.
const TITLE_TOP: f32 = 0.055;
/// The one line under the mark: what the game is, in the space the menu does not need.
const TAGLINE: &str = "A PLATFORMER THAT LEARNS";
/// How far under the mark's block that line sits.
const TITLE_TAGLINE_GAP: f32 = 46.0;

/// Where the two lines sit and how wide the mark is, for a window of `rect`'s size.
///
/// The numbers are computed once, in a function a test can call without a screen, because the
/// lockup has to hold at every window size the app allows: both lines inside the measure, the
/// rules long enough to read as rules, and the caption clear of the menu below it.
struct Lockup {
    small: f32,
    big: f32,
    /// The axis the mark is set on — the window's centre, less half the block.
    centre: f32,
    one: Pos2,
    two: Pos2,
    /// The width of the mark: `HUNDRED`'s ink, which the rules carry up to `ONE TWO`.
    measure: f32,
    /// `ONE TWO`'s own ink, inside the measure.
    one_w: f32,
    rule: f32,
    gap: f32,
}

impl Lockup {
    /// The bottom edge of the block under the big line, which is the mark's real bottom.
    fn bottom(&self) -> f32 {
        self.two.y + self.big * 0.5 + (title_grid(self.big) * TITLE_BLOCK).round()
    }
}

fn lockup(painter: &Painter, rect: Rect, clock: f32) -> Lockup {
    let (small, big) = title_sizes(rect);
    let measure = title_width(painter, TITLE_TWO, big);
    let one_w = title_width(painter, TITLE_ONE, small);
    let rise = (1.0 - title_in(clock)) * 12.0;
    // The block leans to the bottom right, so the mark is set half a block to the left of the
    // window's centre: a person centres a logo by its ink, and the shelf is ink.
    let centre = rect.center().x - (title_grid(big) * TITLE_BLOCK).round() * 0.5;
    let top = rect.top() + rect.height() * TITLE_TOP - rise;
    let one = Pos2::new(centre, top + small * 0.5);
    // The gap between the lines is the small line's own shelf plus a share of the big one's
    // size, so the block under `ONE TWO` sits over the cap height of `HUNDRED` instead of
    // touching it.
    let bevel = (title_grid(small) * TITLE_BLOCK).round().max(2.0);
    let two = Pos2::new(centre, one.y + small * 0.5 + bevel + big * 0.20 + big * 0.5);
    Lockup {
        small,
        big,
        centre,
        one,
        two,
        measure,
        one_w,
        rule: (small * 0.10).round().max(3.0),
        gap: small * 0.62,
    }
}

/// The title card: the two-line wordmark, its rules, and the line under it.
///
/// The mark used to be a picture — the committed `logo.png`, rainbow letters on a strip of baked
/// terrain — hung in the sky over the level. It read as a decal rather than as a title: its own
/// palette was six colours this game never uses, its base was a piece of somebody else's ground,
/// and nothing on the menu below it was set in the same hand. This is the same name drawn the way
/// every other word on the screen is drawn, in the embedded pixel face, and made the way the
/// game's own objects are: cream letters with a hard brown edge, standing on a shelf of the
/// accent gold, the small line ruled out to the big line's measure. Cream on gold on brown is the
/// hierarchy and the material at once, which is what a title needs and the old picture, six
/// colours of somebody else's logotype, did not have.
fn title(painter: &Painter, rect: Rect, clock: f32) {
    let arrived = title_in(clock);
    if arrived <= 0.0 {
        return;
    }
    let alpha = |colour: Color32, a: f32| {
        Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), (a * 255.0) as u8)
    };
    let mark = lockup(painter, rect, clock);

    // The big line sets the measure; the small one is centred on the same axis and the rules
    // either side of it carry that measure up, so the mark reads as one block of width.
    //
    // It was first solved the other way — `ONE TWO` tracked all the way out to `HUNDRED`'s two
    // edges — and at that tracking its letters came apart into six shapes with holes between
    // them. A rule is the classic answer to the same problem and the better one: the small line
    // stays a word, and the width of the mark is still one width.
    let bar = |from: f32, to: f32| {
        if to - from > mark.rule {
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(from, (mark.one.y - mark.rule * 0.5).round()),
                    Vec2::new(to - from, mark.rule),
                ),
                CornerRadius::ZERO,
                alpha(theme::ACCENT, arrived),
            );
        }
    };
    bar(
        mark.centre - mark.measure * 0.5,
        mark.one.x - mark.one_w * 0.5 - mark.gap,
    );
    bar(
        mark.one.x + mark.one_w * 0.5 + mark.gap,
        mark.centre + mark.measure * 0.5,
    );

    title_line(
        painter,
        mark.one,
        TITLE_ONE,
        mark.small,
        theme::PLATE,
        theme::ACCENT,
        arrived,
    );
    title_line(
        painter,
        mark.two,
        TITLE_TWO,
        mark.big,
        theme::PLATE,
        theme::ACCENT,
        arrived,
    );

    // The tagline fades in a beat behind the mark: the name lands, then the sentence about it,
    // and then the menu asks for a decision. It is set exactly like the title above it — cream
    // letters with a one-pixel brown outline — because that is what makes the two lines one
    // lockup instead of a title with a caption under it.
    let tag = ((clock - TITLE_AT - TITLE_FADE * 0.55) / 0.45).clamp(0.0, 1.0);
    if tag > 0.0 {
        // Under the block, not under the baseline: the shelf is part of the mark's height.
        let at = Pos2::new(mark.centre, mark.bottom() + TITLE_TAGLINE_GAP);
        let font = theme::pixel_font(12.0);
        // On a chip, because the sky behind it is not a surface: there are clouds, hills and the
        // sun crossing this line at any moment of the loop, and white type with a one-pixel edge
        // disappears into a cloud. A dark pill is the same bargain the stage under the menu makes
        // — the words get a surface of their own rather than hoping the picture stays quiet.
        let galley = painter.layout_no_wrap(TAGLINE.to_string(), font.clone(), theme::PLATE);
        let chip = Rect::from_center_size(at, galley.size() + Vec2::new(26.0, 14.0));
        painter.rect_filled(
            chip,
            CornerRadius::same(6),
            Color32::from_rgba_unmultiplied(
                STAGE_INK.r(),
                STAGE_INK.g(),
                STAGE_INK.b(),
                (tag * 168.0) as u8,
            ),
        );
        painter.text(
            at,
            Align2::CENTER_CENTER,
            TAGLINE,
            font,
            Color32::from_rgba_unmultiplied(
                theme::PLATE.r(),
                theme::PLATE.g(),
                theme::PLATE.b(),
                (tag * 255.0) as u8,
            ),
        );
    }
}

/// The corners of the picture fall away, so the level is the subject and the frame is a frame.
///
/// Three bands rather than a radial blur — a pixel-art title screen does not vignette with a
/// soft lens. The top band is the one that matters: it is what the wordmark is read against on a
/// bright sky. The side bands are deliberately gentle, because the level runs off both edges and
/// cutting into it would hide the thing the screen is showing.
fn vignette(painter: &Painter, rect: Rect) {
    let clear = Color32::from_rgba_unmultiplied(STAGE_INK.r(), STAGE_INK.g(), STAGE_INK.b(), 0);
    let band = |x0: f32, x1: f32, y0: f32, y1: f32, top: Color32, bottom: Color32| {
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(Pos2::new(x0, y0), top);
        mesh.colored_vertex(Pos2::new(x1, y0), top);
        mesh.colored_vertex(Pos2::new(x1, y1), bottom);
        mesh.colored_vertex(Pos2::new(x0, y1), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    };
    let ink =
        |a: u8| Color32::from_rgba_unmultiplied(STAGE_INK.r(), STAGE_INK.g(), STAGE_INK.b(), a);
    band(
        rect.left(),
        rect.right(),
        rect.top(),
        rect.top() + rect.height() * 0.34,
        ink(96),
        clear,
    );
    band(
        rect.left(),
        rect.right(),
        rect.bottom() - rect.height() * 0.22,
        rect.bottom(),
        clear,
        ink(70),
    );
    band(
        rect.left(),
        rect.left() + rect.width() * 0.10,
        rect.top(),
        rect.bottom(),
        ink(48),
        clear,
    );
    band(
        rect.right() - rect.width() * 0.10,
        rect.right(),
        rect.top(),
        rect.bottom(),
        clear,
        ink(48),
    );
}

/// The line along the window's bottom edge: the name on the left, the build on the right.
///
/// A title screen with nothing under the menu is a screen that stops rather than a screen that
/// is finished. Two small capitals on the frame's own edge finish it, and the build number on
/// the right is also the one thing on this screen that a bug report needs.
fn footer(painter: &Painter, rect: Rect, clock: f32) {
    let arrived = ((clock - HINT_AT - DOORS_LAG) / 0.6).clamp(0.0, 1.0);
    if arrived <= 0.0 {
        return;
    }
    let ink = FOOT_INK.gamma_multiply(arrived * 0.85);
    let y = rect.bottom() - 10.0;
    painter.text(
        Pos2::new(rect.left() + 16.0, y),
        Align2::LEFT_CENTER,
        "ONE TWO HUNDRED",
        theme::pixel_font(8.0),
        ink,
    );
    painter.text(
        Pos2::new(rect.right() - 16.0, y),
        Align2::RIGHT_CENTER,
        format!("v{}", env!("CARGO_PKG_VERSION")),
        theme::pixel_font(8.0),
        ink,
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
}

/// Whether the menu is on screen at `clock`.
///
/// One function rather than the comparison written twice, because `show` has to answer it
/// for a reason of its own: a press inside the menu belongs to the *row* under the pointer,
/// and the rule that any press starts the editor has to step aside for it. Two copies of
/// "when is it up" is how the band ends up swallowing a click on a frame the rows are not
/// drawn on.
/// The sound plate in the splash's top-right corner, and the rectangle it
/// occupied.
///
/// The rectangle is returned because this screen dismisses on *any* pointer
/// press, and a control that dismisses the screen on its way to being clicked
/// is not a control. The caller lets a press inside it through to the widget.
fn sound_button(ctx: &Context, rect: Rect, atlas: &Atlas) -> Rect {
    let muted = crate::sound::muted();
    let label = if muted { "Muted" } else { "Sound" };
    egui::Area::new(egui::Id::new("splash-sound"))
        .fixed_pos(Pos2::new(rect.max.x - 126.0, 16.0))
        .movable(false)
        .show(ctx, |ui| {
            let response = ui::icon_button(
                ui,
                Some(atlas),
                None,
                label,
                Vec2::new(84.0, 36.0),
                !muted,
                true,
            )
            .on_hover_text("sound on or off — M also works here");
            if response.clicked() {
                crate::sound::toggle_mute();
            }
        })
        .response
        .rect
}

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
    let height = PANEL_PAD * 2.0 + LINE_H * DOORS.len() as f32;
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
        Pos2::new(
            panel.left(),
            panel.top() + PANEL_PAD + index as f32 * LINE_H,
        ),
        Vec2::new(panel.width(), LINE_H),
    )
}

/// Draw the menu, and report the door that was clicked, if any.
///
/// Real widgets (`Ui::interact`) for the hit test, hand-painted for the look: a framed list,
/// one filled row for the chosen option, and the row under the pointer is the row the arrow
/// keys are on. That is enough for a front door.
fn menu(
    ctx: &Context,
    painter: &Painter,
    rect: Rect,
    panel: Rect,
    selected: &mut usize,
    clock: f32,
    atlas: &Atlas,
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
        paint_menu(painter, panel, *selected, arrived, atlas);
        prompt(painter, rect, clock);
    }
    action
}

/// The menu panel and its three rows.
///
/// One surface, one accent, no decoration: a warm plate with a dark outline, the chosen row
/// filled in the window's gold, and the verbs themselves as the only labels.
fn paint_menu(painter: &Painter, panel: Rect, selected: usize, arrived: f32, atlas: &Atlas) {
    let ink = |colour: Color32| colour.gamma_multiply(arrived);
    let radius = CornerRadius::same(theme::RADIUS_LG);
    // A drop shadow under the box, then the plate: the menu is an object standing on the stage
    // under the level, and without the shadow it is a white rectangle laid over a picture.
    painter.rect_filled(
        panel.translate(Vec2::new(0.0, 5.0)),
        radius,
        ink(Color32::from_rgba_premultiplied(16, 10, 8, 120)),
    );
    painter.rect_filled(panel, radius, ink(theme::CARD));
    painter.rect_stroke(
        panel,
        radius,
        Stroke::new(3.0_f32, ink(theme::OUTLINE)),
        egui::StrokeKind::Inside,
    );
    for (index, door) in DOORS.iter().enumerate() {
        let line = line_rect(panel, index);
        let chosen = index == selected;
        let plate = line.shrink2(Vec2::new(8.0, 4.0));
        if chosen {
            // The chosen row is a *plate*, not a tint: the same gold the dock gives the mode
            // being used, with the same brown boundary, so the two menus agree about what a
            // selected control looks like.
            painter.rect_filled(plate, CornerRadius::same(theme::RADIUS), ink(theme::ACCENT));
            painter.rect_stroke(
                plate,
                CornerRadius::same(theme::RADIUS),
                Stroke::new(2.0_f32, ink(theme::OUTLINE)),
                egui::StrokeKind::Inside,
            );
        } else if index + 1 < DOORS.len() {
            // A hairline between the rows, so the list reads as a list rather than as three
            // words floating in a box.
            let y = line.bottom();
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(line.left() + 12.0, y - 0.5),
                    Pos2::new(line.right() - 12.0, y + 0.5),
                ),
                CornerRadius::ZERO,
                ink(theme::EDGE),
            );
        }
        // The icon, on the same left margin as the word it belongs to, in the game's own art:
        // the ground tile for Edit, the runner for Play, a walker for Evolve.
        let icon = Rect::from_center_size(
            Pos2::new(plate.left() + MENU_ICON_LEFT, plate.center().y),
            Vec2::splat(MENU_ICON),
        );
        if let Ok(uv) = atlas.uv(door.icon) {
            painter.image(atlas.texture_id(), icon, uv, ink(Color32::WHITE));
        }
        painter.text(
            Pos2::new(plate.left() + MENU_LABEL_LEFT, plate.center().y),
            Align2::LEFT_CENTER,
            door.name,
            theme::pixel_font(18.0),
            ink(if chosen {
                theme::INK_ON_ACCENT
            } else {
                theme::TEXT
            }),
        );
    }
}

/// The one instruction under the box: move the selection, then take it.
fn prompt(painter: &Painter, rect: Rect, clock: f32) {
    let arrived = ((clock - HINT_AT - DOORS_LAG - 0.15) / 0.45).clamp(0.0, 1.0);
    if arrived <= 0.0 {
        return;
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - PROMPT_BOTTOM),
        Align2::CENTER_CENTER,
        MENU_HINT,
        theme::pixel_font(10.0),
        FOOT_INK.gamma_multiply(arrived),
    );
}

/// Whose run is playing, when it is a saved champion's rather than the demo.
///
/// A caption, not a footer: one short line, and only when there is something to credit.
fn champion_credit(painter: &Painter, rect: Rect, credit: Option<&str>) {
    let Some(credit) = credit else {
        return;
    };
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - CREDIT_BOTTOM),
        Align2::CENTER_CENTER,
        credit.replace('·', "-").to_uppercase(),
        theme::pixel_font(8.0),
        theme::ACCENT.gamma_multiply(0.85),
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

    /// A splash driven far enough that the menu and its prompt are both up.
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
        // on: the rows are up a beat before the prompt is drawn, and the
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

    /// The box arrives with its prompt and sits under the world.
    ///
    /// Both halves are geometry, and both are the kind a screenshot catches late: a menu drawn
    /// over the level's lawn, or a box that appears before the line that explains it. The
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
        // The prompt lives under the box, and the box leaves it its band.
        let prompt = rect.bottom() - PROMPT_BOTTOM;
        assert!(
            panel.bottom() + 8.0 < prompt,
            "the box is standing on the prompt under it: {panel:?} vs {prompt:.0}"
        );
        assert!(
            panel.bottom() <= rect.bottom() - CREDIT_BOTTOM,
            "the box falls off the window"
        );

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
        let mut highest = splash.session.sim().player().y;
        while !splash.session.sim().outcome().is_finished() && seconds < 30.0 {
            splash.advance(sim::DT);
            highest = highest.min(splash.session.sim().player().y);
            seconds += sim::DT;
        }
        assert_eq!(
            splash.session.sim().outcome(),
            Outcome::Won,
            "the splash's own runner did not get to the flag in {seconds:.1}s: {}",
            splash.session.summary()
        );
        // The scene's two moves really happened: a jump over the pit and a jump onto the block,
        // before the hill at the end. A runner that walked along the flat and reached a flag on
        // the same row would satisfy "won" and show nothing the game is about.
        assert!(
            splash.session.sim().stats().jumps >= 2,
            "the runner won with {} jumps: it never cleared the pit and the block",
            splash.session.sim().stats().jumps
        );
        assert!(
            highest < 11.9,
            "the runner only reached y {highest:.1}; it never climbed the hill to the flag"
        );
        assert!(
            seconds < 12.0,
            "the little run takes {seconds:.1}s, which is longer than a splash has"
        );
    }

    /// The camera keeps the runner inside the window through the whole run.
    ///
    /// The finish hill puts the flag two tiles above the lawn, and a full jump from the top of
    /// it would otherwise carry the agent's head above the frame. The target camera eases down
    /// by the missing margin, and this drives the whole run to prove it never loses the runner.
    #[test]
    fn the_camera_keeps_the_runner_in_frame() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));
        let mut splash = Splash::scripted();
        let mut highest = f32::MAX;
        while !splash.session.sim().outcome().is_finished() {
            splash.advance(sim::DT);
            let p = splash.session.sim().player();
            highest = highest.min(splash.camera(rect).to_screen(p.x, p.y, rect).y);
        }
        assert!(
            highest >= 0.0,
            "the runner's head reached y {highest:.1}, above the window"
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
        // And the menu has the band this framing is for: its prompt is below the level's own
        // bottom edge, not printed over the lawn.
        let ground = splash.level.height as f32;
        let bottom = settled.to_screen(0.0, ground, rect).y;
        let prompt = rect.bottom() - PROMPT_BOTTOM;
        assert!(
            prompt > bottom + 20.0,
            "the prompt at {prompt:.0} is on the world, which ends at {bottom:.0}"
        );
    }

    /// The title is fully revealed before the menu arrives to be read.
    ///
    /// Both halves matter: a mark that is still arriving while the menu is already solid is a
    /// screen that is still assembling itself while it asks the viewer to choose, and a reveal
    /// that takes longer than a person waits is a title nobody reads.
    #[test]
    fn the_title_arrives_before_the_menu() {
        // The wordmark itself is laid out and checked in
        // [`the_wordmark_is_one_lockup_at_every_window_size`]; this is the clock.
        assert_eq!(title_in(0.0), 0.0, "the title is up before the clock is");
        assert_eq!(
            title_in(TITLE_AT - 0.01),
            0.0,
            "the title arrives before its moment"
        );
        let done = TITLE_AT + TITLE_FADE;
        assert_eq!(title_in(done + 0.01), 1.0, "the title never settles");
        assert!(
            done < HINT_AT,
            "the title is still arriving at {done:.2}s, after the menu at {HINT_AT}s"
        );
        // Halfway through the fade it is halfway there: the reveal is a ramp, not a cut,
        // because a picture that snaps in at full strength reads as a pop rather than as a
        // landing.
        let mid = title_in(TITLE_AT + TITLE_FADE * 0.5);
        assert!(mid > 0.4 && mid < 0.6, "halfway reads {mid}");
    }

    /// The wordmark is one lockup at every window size: both lines inside one measure, the rules
    /// long enough to read as rules, and the caption clear of the menu.
    ///
    /// The sizes here are the window's own floor and a wall. A title that only holds at the size
    /// it was designed at is a title that breaks on somebody else's screen, and “the lines line
    /// up” is arithmetic: three window sizes are checked here because no eye checks three at once.
    #[test]
    fn the_wordmark_is_one_lockup_at_every_window_size() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        for (w, h) in [(900.0, 560.0), (1360.0, 800.0), (2560.0, 1440.0)] {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h));
            let mut laid = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    ..Default::default()
                },
                |ctx| {
                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Background,
                        egui::Id::new("wordmark"),
                    ));
                    laid = Some(lockup(&painter, rect, 2.4));
                },
            );
            let mark = laid.expect("the lockup is laid out inside a frame");
            let at = format!("{w:.0}×{h:.0}");
            assert_eq!(
                mark.small * 2.0,
                mark.big,
                "the small line is not half of {at}"
            );
            assert_eq!(
                mark.big % 8.0,
                0.0,
                "{at} is off the face's grid: {}",
                mark.big
            );
            assert!(
                (40.0..=120.0).contains(&mark.big),
                "{at} sets the big line at {}",
                mark.big
            );
            assert!(
                mark.measure > w * 0.28 && mark.measure < w * 0.62,
                "{at} sets the mark {} wide",
                mark.measure
            );
            let rules = mark.measure - mark.one_w;
            assert!(
                rules > 2.0 * (mark.gap + mark.rule),
                "{at} leaves the rules {rules} for two rules and two gaps"
            );
            assert!(
                mark.one.y - mark.small > rect.top(),
                "{at} starts the mark above the window's top edge"
            );
            assert!(
                mark.centre - mark.measure * 0.5 > rect.left()
                    && mark.centre + mark.measure * 0.5 < rect.right(),
                "{at} runs the mark off the side"
            );
            // The caption sits under the block and above the menu: the screen's three objects,
            // in that order, at every size.
            let caption = mark.bottom() + TITLE_TAGLINE_GAP + 20.0;
            assert!(
                caption < menu_panel(rect).top() || rect.height() < 620.0,
                "{at} puts the caption at {caption} under a menu starting at {}",
                menu_panel(rect).top()
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
            credit.contains("gen 7")
                && credit.contains(&format!("fitness {:.0}", champion.fitness)),
            "the credit line reads {credit:?}, which is not this file's run"
        );
    }

    /// Nothing that is not this level's champion, for this build, is played.
    ///
    /// Five refusals, each of which would otherwise be a title screen quietly showing the
    /// wrong thing: no file at all; a champion for a different level, whose terrain is not the
    /// one on screen; a file from another build, whose physics are not this one's; a best-ever
    /// agent that dies; and a generation-zero winner, which is the seed population's best
    /// rather than a learned run.
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

        // A generation-zero winner: the seed population, not a learned run.
        let seed = scratch("generation-zero");
        let mut early = save_champion(&seed, "l1-flat-run", &flat, 7);
        early.metadata.generation = 0;
        early
            .save(seed.join("l1-flat-run.json"))
            .expect("the seed champion writes");
        assert!(matches!(
            Splash::open(&flat, &seed, Some(&flat_path)).policy,
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
        // The floor is pinned low, with the menu's band under it.
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

    /// The prompt and the champion credit have their own band under the box, whatever the
    /// window.
    ///
    /// The screen plays whatever level the window opened on when that level has a champion, so
    /// the prompt cannot be placed as a fraction of the world and trusted: a level tall enough to
    /// reach the band would have it printed on its own lawn. The box is anchored to the
    /// window's bottom edge and the prompt to the box, which is what keeps the two readable on
    /// every level the screen can be opened on.
    #[test]
    fn the_prompt_sits_under_the_menu_box() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));
        let panel = menu_panel(rect);
        assert!(panel.top() >= rect.top(), "the box starts above the window");
        assert!(
            panel.bottom() < rect.bottom() - PROMPT_BOTTOM,
            "the prompt is drawn on the box"
        );
        assert!(panel.bottom() < rect.bottom() - CREDIT_BOTTOM);
        // And the box is bottom-anchored: a taller window moves it, it does not stretch it.
        let tall = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 1100.0));
        assert_eq!(
            menu_panel(tall).height(),
            menu_panel(rect).height(),
            "the box changed shape with the window"
        );
        assert!(menu_panel(tall).bottom() > panel.bottom());
    }

    /// The screen draws the world and the words, and both change with the clock.
    ///
    /// Read back the way the renderer left it — the tessellated shapes, counted by which
    /// texture they sampled — rather than by asking the code that drew them: the world is a
    /// great many atlas quads, the title and the menu are glyphs from the font texture, and a
    /// clock that stops moving the world is the one bug a splash can have that a person would
    /// notice immediately.
    #[test]
    fn the_splash_draws_the_world_and_the_words() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let atlas = Atlas::load(&ctx, assets_dir()).expect("the shipped tileset loads");
        let sprites = Sprites::load(&atlas).expect("the shipped tileset has every sprite");
        let size = Vec2::new(1360.0, 800.0);
        let rect = Rect::from_min_size(Pos2::ZERO, size);
        let counts = |clock: f32| -> (usize, usize, u32) {
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
            let (mut world, mut words) = (0, 0);
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
                    // atlas's white pixel is a filled shape; counting those would count
                    // scenery as type.
                    if quad[0].uv == egui::epaint::WHITE_UV {
                        continue;
                    }
                    // The sound plate is the one word this screen shows before
                    // the menu: it is a control, not part of the reveal.
                    if quad[0].pos.x > rect.max.x - 140.0 && quad[0].pos.y < 80.0 {
                        continue;
                    }
                    words += 1;
                }
            }
            (world, words, splash.session.sim().stats().frames)
        };

        let (world, words, frames) = counts(2.4);
        assert!(
            world > 40,
            "the splash drew {world} sprite quads: the world is not on the screen"
        );
        assert!(
            words > 40,
            "the splash drew {words} glyph quads: the title, menu and prompt are not on screen"
        );
        // The title arrives before the menu: at 0.9s the wordmark and its tagline are up (the
        // two lines are seven letters each and the tagline is twenty-four), and the menu is not
        // yet, so a frame in that window has words but fewer of them than the finished screen.
        let (_, early_words, _) = counts(0.0);
        assert_eq!(early_words, 0, "the title is already up at the first frame");
        let (_, title_words, _) = counts(0.95);
        assert!(
            title_words > 10 && title_words < words,
            "at 0.95s there are {title_words} glyph quads and the finished screen has {words}: \
             the title and the menu are not arriving at separate times"
        );
        let (_, _, later_frames) = counts(6.0);
        assert!(
            later_frames != frames,
            "the episode is on the same tick at 2.4s and 6.0s: nothing in it is running"
        );
    }
}
