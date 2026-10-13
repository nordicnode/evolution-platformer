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
//! **It gets out of the way.** Any key or any click after the first [`ARMED`] seconds
//! dismisses it into the editor exactly as it opens today; the window is playable from the
//! frame after that, and nothing about the splash is reachable again except by restarting the
//! app. A stray event from the window manager — a focus click, a key repeat — must not eat the
//! title before a person has seen it, which is what [`ARMED`] is for.
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

use egui::{Align2, Color32, Context, FontId, Painter, Pos2, Rect, Vec2};
use evo::{ActScratch, ChampionFile, Genome};
use sim::{AgentAction, Level, Sim, Tile};

use crate::atlas::Atlas;
use crate::play::{self, Keys};
use crate::view::{self, Camera, RenderOpts, Sheet, Sprites};

/// The words. The title is the window's own title, so the taskbar and the splash agree.
const TITLE: &str = "EVOLUTION PLATFORMER";
const SUBTITLE: &str = "a population learns to beat the level you draw";
/// The invitation, and the four keys a person needs to know about to use the window at all.
/// The app's own bindings, in the order the tabs appear in the top bar.
const HINT: &str = "press any key";
const KEYS: &str = "P play      E evolve      V replay      F fit the level";

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
/// When the invitation arrives, and how fast it breathes. After the title has finished
/// assembling: a screen that is still revealing its name while it says "press any key" is
/// still assembling itself at the moment it invites the viewer to leave.
const HINT_AT: f32 = 1.9;
const HINT_HZ: f32 = 1.3;
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
const RAISED: f32 = 0.075;
/// Where the ground sits in the window when a champion is playing, as a fraction of the
/// window's height.
///
/// Low enough that the lawn, the props and the agent are in the lower third of a window the
/// sky and its parallax fill above them, high enough to leave a band of its own under the
/// ground for the invitation. See [`Splash::target`].
const GROUND_LINE: f32 = 0.78;
/// The seed of the splash's episode, so the splash is the same splash every launch.
const SEED: u64 = 4;

/// The game's own palette, so the words cannot be a different game's colours.
const TITLE_INK: Color32 = Color32::from_rgb(255, 240, 190); // the sun's face
const TITLE_SHADE: Color32 = Color32::from_rgb(20, 24, 32); // the panel background, darker
const SUBTITLE_INK: Color32 = Color32::from_rgb(214, 220, 228); // panel TEXT
const HINT_INK: Color32 = Color32::from_rgb(250, 214, 102); // panel WIN_TICK
const RULE_INK: Color32 = Color32::from_rgb(120, 216, 120); // panel MAX_LINE
const FOOT_INK: Color32 = Color32::from_rgb(160, 168, 180); // panel TEXT_DIM

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
        let skip = PULLBACK + 0.5;
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

    /// Draw the screen, and say whether the viewer has asked for the game instead.
    ///
    /// Any key press or any click counts, once the screen has been up for [`ARMED`] seconds:
    /// a splash is a doorway, and a doorway that argues is a wall. Nothing else about the app
    /// runs while this is up — the caller returns early — so the key that dismisses the splash
    /// cannot also press a button in the editor underneath it.
    pub fn show(&self, ctx: &Context, atlas: &Atlas, sprites: &Sprites) -> bool {
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

        // Where the world ends. The invitation is placed under it rather than at a fixed
        // height, because a champion is played on whatever document the window opened on and
        // a document can be tall enough to reach the band the words want.
        let world_bottom = sheet
            .camera
            .to_screen(0.0, self.level.height as f32, rect)
            .y;
        title(&painter, ctx, rect, self.clock);
        subtitle(&painter, rect, self.clock);
        invitation(&painter, rect, self.clock, world_bottom);
        footer(&painter, rect, self.credit());

        self.clock >= ARMED
            && ctx.input(|i| {
                i.events.iter().any(|event| {
                    // A press, not a release: letting go of a key is not a decision, and a
                    // pointer that has left the window has not made one either.
                    matches!(
                        event,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::PointerButton { pressed: true, .. }
                    )
                })
            })
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
    // tile standing on the lawn.
    level.goals.clear();
    level.set_spawn_tile(1, 10);
    level.add_goal_tile(31, 10);
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
/// something to eyeball.
fn letter_in(clock: f32, index: usize) -> f32 {
    let at = TITLE_AT + index as f32 * LETTER_IN;
    ((clock - at) / LETTER_FADE).clamp(0.0, 1.0)
}

/// Around the title, in points, as a fraction of the window.
fn title_size(rect: Rect) -> f32 {
    (rect.height() * 0.092).clamp(26.0, 64.0)
}

/// The line the title sits on, as a fraction of the window's height.
///
/// High enough to be in the sky and clear of the treeline, low enough that the space above
/// it is a horizon rather than a margin.
const TITLE_LINE: f32 = 0.275;

/// The title, revealed a letter at a time.
///
/// One `painter.text` per letter rather than one for the line, because the reveal is the
/// animation and a galley can only be drawn whole. The advances come from the font
/// ([`egui::epaint::Fonts::glyph_width`]), so the letters land exactly where a single call
/// would have put them: this is a line of text that is drawn in pieces, not a hand-laid one.
///
/// Each letter is drawn twice, a shade under and to the right in the panel's own near-black
/// and then in the sun's cream. Two calls is the whole of the outline: this is a pixel game,
/// and a 2px offset shadow is the way a pixel game puts type on a picture.
fn title(painter: &Painter, ctx: &Context, rect: Rect, clock: f32) {
    let font = FontId::monospace(title_size(rect));
    let advances: Vec<f32> = TITLE
        .chars()
        .map(|c| ctx.fonts(|f| f.glyph_width(&font, c)))
        .collect();
    let width: f32 = advances.iter().sum();
    let mut x = rect.center().x - width * 0.5;
    let y = rect.top() + rect.height() * TITLE_LINE;
    for (index, ch) in TITLE.chars().enumerate() {
        let arrived = letter_in(clock, index);
        if arrived > 0.0 {
            let ink = arrived.powf(0.7);
            let at = Pos2::new(x, y + (1.0 - arrived) * LETTER_RISE);
            if ch != ' ' {
                painter.text(
                    at + Vec2::new(2.0, 3.0),
                    Align2::LEFT_CENTER,
                    ch,
                    font.clone(),
                    TITLE_SHADE.gamma_multiply(ink),
                );
                painter.text(
                    at,
                    Align2::LEFT_CENTER,
                    ch,
                    font.clone(),
                    TITLE_INK.gamma_multiply(ink),
                );
            }
        }
        x += advances[index];
    }
    // And a rule under it, growing from the middle out as the title arrives: the one piece of
    // decoration on the screen, and it is what stops a line of type over a landscape from
    // reading as a caption.
    let grown = ((clock - TITLE_AT) / (TITLE_AT + LETTER_IN * TITLE.len() as f32)).clamp(0.0, 1.0);
    if grown > 0.0 {
        let rule = Rect::from_center_size(
            Pos2::new(rect.center().x, y + title_size(rect) * 0.75),
            Vec2::new(width * 0.6 * grown, 3.0),
        );
        painter.rect_filled(rule, 0.0, RULE_INK.gamma_multiply(grown));
    }
}

/// The line under the title: what the window is *for*, in one sentence.
fn subtitle(painter: &Painter, rect: Rect, clock: f32) {
    let arrived = ((clock - TITLE_AT - 0.6) / 0.6).clamp(0.0, 1.0);
    if arrived <= 0.0 {
        return;
    }
    painter.text(
        Pos2::new(
            rect.center().x,
            rect.top() + rect.height() * TITLE_LINE + title_size(rect) * 1.9,
        ),
        Align2::CENTER_CENTER,
        SUBTITLE,
        FontId::proportional((title_size(rect) * 0.44).clamp(14.0, 24.0)),
        SUBTITLE_INK.gamma_multiply(arrived),
    );
}

/// Where the invitation sits: the band under the ground, and never on the world.
///
/// The composition wants it at a fixed height — seven eighths of the way down, in the layer
/// under the level — and the only thing that can be wrong with that is the world in the
/// picture: a champion is played on whatever document the window opened on, and a level
/// reaching that far down would have the words printed on its own lawn. So the wanted line is
/// pushed below the world when it has to be, and held inside the window after that.
fn invitation_line(rect: Rect, world_bottom: f32) -> f32 {
    let wanted = rect.bottom() - rect.height() * 0.125;
    wanted.max(world_bottom + 34.0).min(rect.bottom() - 24.0)
}

/// The breathing invitation, and the four keys a person needs to know about.
fn invitation(painter: &Painter, rect: Rect, clock: f32, world_bottom: f32) {
    let arrived = ((clock - HINT_AT) / 0.5).clamp(0.0, 1.0);
    if arrived <= 0.0 {
        return;
    }
    // A breath rather than a blink: two frames of hidden and two of shown reads as a broken
    // sprite, and this has to survive being looked at for as long as somebody takes to read
    // the keys under it.
    let breath = 0.55 + 0.45 * (clock * std::f32::consts::TAU * HINT_HZ).sin();
    // Under the ground rather than on it: the band below the level is empty haze, and a line
    // of type there is legible on every window size instead of on the ones where the fit
    // happened to leave a gap under the lawn.
    let y = invitation_line(rect, world_bottom);
    painter.text(
        Pos2::new(rect.center().x, y),
        Align2::CENTER_CENTER,
        HINT,
        FontId::monospace((title_size(rect) * 0.44).clamp(16.0, 28.0)),
        HINT_INK.gamma_multiply(arrived * breath),
    );
    painter.text(
        Pos2::new(
            rect.center().x,
            (y + title_size(rect) * 0.95).min(rect.bottom() - 30.0),
        ),
        Align2::CENTER_CENTER,
        KEYS,
        FontId::monospace((title_size(rect) * 0.30).clamp(12.0, 17.0)),
        FOOT_INK.gamma_multiply(arrived * 0.9),
    );
}

/// The bottom band: what this is, whose run it is, and what it is not.
fn footer(painter: &Painter, rect: Rect, credit: Option<&str>) {
    let font = FontId::monospace(11.0);
    painter.text(
        Pos2::new(rect.left() + 12.0, rect.bottom() - 10.0),
        Align2::LEFT_BOTTOM,
        format!("evo-app {}", env!("CARGO_PKG_VERSION")),
        font.clone(),
        FOOT_INK,
    );
    // The credit is the one line down here worth reading, so it is the one line down here
    // that is not grey: the warm cream of the title, held back far enough to stay a caption.
    if let Some(credit) = credit {
        painter.text(
            Pos2::new(rect.center().x, rect.bottom() - 10.0),
            Align2::CENTER_BOTTOM,
            credit,
            font.clone(),
            TITLE_INK.gamma_multiply(0.72),
        );
    }
    painter.text(
        Pos2::new(rect.right() - 12.0, rect.bottom() - 10.0),
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
        let last = TITLE.chars().count() - 1;
        assert_eq!(
            letter_in(0.0, 0),
            0.0,
            "the title is up before the clock is"
        );
        assert!(letter_in(0.0, last) == 0.0);
        for index in 0..TITLE.chars().count() {
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
        let key = egui::Event::Key {
            key: egui::Key::P,
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
        let opened = |clock: f32, events: Vec<egui::Event>| -> bool {
            let mut splash = Splash::scripted();
            while splash.clock() < clock {
                splash.advance(sim::DT);
            }
            let mut dismissed = false;
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
            !opened(ARMED - 0.05, vec![key.clone()]),
            "a key in the first {ARMED}s dismissed the screen"
        );
        assert!(
            !opened(ARMED - 0.05, vec![click.clone()]),
            "a click in the first {ARMED}s dismissed the screen"
        );
        assert!(
            !opened(ARMED + 0.1, Vec::new()),
            "the screen dismissed itself with nobody there"
        );
        assert!(
            opened(ARMED + 0.1, vec![key]),
            "a key does not dismiss the screen"
        );
        assert!(
            opened(ARMED + 0.1, vec![click]),
            "a click does not dismiss the screen"
        );
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
            invitation_line(rect, floor) > floor,
            "the invitation is drawn on the ground it is meant to be under"
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

    /// The invitation is drawn under the world it is over, whatever the world is.
    ///
    /// The screen plays whatever level the window opened on when that level has a champion, so
    /// the words cannot be placed as a fraction of the window and trusted: a level tall enough
    /// to reach the band would have "press any key" printed on its own lawn. The rule is the
    /// wanted line, pushed below the world when the world takes it, and held inside the window
    /// after that.
    #[test]
    fn the_invitation_is_drawn_under_the_world_it_is_over() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 800.0));
        let wanted = rect.bottom() - rect.height() * 0.125;
        assert_eq!(
            invitation_line(rect, 500.0),
            wanted,
            "the usual case is untouched"
        );
        assert!(
            invitation_line(rect, wanted - 10.0) > wanted,
            "the world pushes it down"
        );
        assert!(
            invitation_line(rect, 900.0) <= rect.bottom() - 24.0,
            "a world that reaches the bottom cannot push the words out of the window"
        );
        // And the splash's own level leaves it exactly where the composition wants it.
        let splash = Splash::scripted();
        let camera = splash.camera(rect);
        let bottom = camera.to_screen(0.0, splash.level.height as f32, rect).y;
        assert_eq!(invitation_line(rect, bottom), wanted);
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
        let line = Rect::from_center_size(
            Pos2::new(rect.center().x, rect.top() + rect.height() * TITLE_LINE),
            Vec2::new(rect.width(), 90.0),
        );
        let counts = |clock: f32| -> (usize, usize, usize) {
            let splash = {
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
                    let centre = quad.iter().fold(Pos2::ZERO, |a, v| a + v.pos.to_vec2()) / 4.0;
                    words += 1;
                    if line.contains(centre) {
                        title += 1;
                    }
                }
            }
            (world, words, title)
        };

        let letters = TITLE.chars().filter(|c| *c != ' ').count();
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
