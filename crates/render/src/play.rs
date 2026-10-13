//! Interactive human playtest of a level.
//!
//! This is the M1 exit-gate tool: "a human can beat L1-L3". It exists to prove the
//! physics is playable by hand, not to be a game.
//!
//! **Timing note.** Everything here is wall-clock paced (`FRAME_TIME` per frame,
//! `std::thread::sleep`) purely so the picture is watchable. The sim's determinism
//! contract (development_plan.md §5.1) lives in `sim`, which has no notion of time:
//! `Sim::step` is one tick and nothing else. Real-time pacing must never leak into
//! headless training, and this module is the only place in the workspace that is
//! allowed to sleep.

use std::io::{self, IsTerminal, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::execute;
use sim::{AgentAction, Level, Outcome, Sim};

use crate::ascii::{self, Viewport};

/// One rendered frame. 60 Hz, matching `sim::DT`'s 1/60 s tick.
pub const FRAME_TIME: Duration = Duration::from_millis(16);

/// How long a press keeps its intent alive — the stand-in for "the key is down".
///
/// A terminal reports *presses*, not a key state, and its auto-repeat is slow:
/// typically 25-30 a second after a half-second delay, and slower on some desktops.
/// Stepping one frame per press therefore moves the player at a small fraction of
/// what holding the key does. Measured on L1 through `train-cli play` in a pty,
/// before this existed: five presses a second for four seconds ended at `best x 2.40`
/// against a spawn at 2.0 — four tenths of a tile, a *progress* of 3 px, where a held
/// key covers 24 tiles in the same four seconds. That reads as "the controls are
/// broken" rather than "the level is hard". The same input with the latch reaches
/// `best x 26.9`, and twelve seconds of that slow repeat reaches the flag.
///
/// Each press now latches for this long, which bridges every common repeat rate down
/// to 5 a second. The cost is that a release coasts for up to the window; terminals
/// that report key releases (crossterm delivers `KeyEventKind::Release` where the
/// terminal supports it) end the latch immediately, so a deliberate release stays
/// crisp.
///
/// What the window buys on a real obstacle is measured in `tests/playability.rs`:
/// one latched press gives L2's first gap a 224 ms take-off window, against the
/// 144 ms a single frame's press gets from the physics' own `min_jump_hold`.
pub const HOLD_WINDOW: Duration = Duration::from_millis(200);

/// Terminal rows reserved for the control hint below the level view.
const HINT_ROWS: u16 = 2;

/// What the player achieved before quitting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaySummary {
    pub frames: u32,
    pub outcome: Outcome,
    pub progress_pixels: f32,
    pub best_x: f32,
}

impl PlaySummary {
    fn of(sim: &Sim) -> PlaySummary {
        PlaySummary {
            frames: sim.stats().frames,
            outcome: sim.outcome(),
            progress_pixels: sim.stats().progress_pixels,
            best_x: sim.stats().best_x,
        }
    }
}

/// Restores the terminal on the way out — including on panic and on early return
/// from `?`, which is why the restore lives in `Drop` and not in the happy path.
struct TerminalSession;

impl TerminalSession {
    fn enter() -> io::Result<TerminalSession> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(TerminalSession)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // Best-effort: a failure here must not mask a panic already in flight.
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Play `level` with the keyboard until the episode ends or the player quits.
///
/// Controls: left/right arrows or A/D to move, space/W/up to jump, down/S to
/// brake, R to restart, Q/Esc to quit. Holding a direction while jumping gives
/// `RunJump`; jumping alone gives `JumpInPlace`; nothing gives `Walk`.
pub fn play(level: &Level, seed: u64) -> io::Result<PlaySummary> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "play needs an interactive terminal: stdout is not a TTY",
        ));
    }

    let session = TerminalSession::enter()?;
    let mut stdout = io::stdout();
    let mut sim = Sim::new(level, seed);
    let mut keys = Keys::default();

    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;

    loop {
        let started = Instant::now();

        read_keys(&mut keys, started)?;
        if keys.take_quit() {
            break;
        }
        if keys.take_restart() {
            sim = Sim::new(level, seed);
        }

        sim.step(keys.action(started));

        let (term_cols, term_rows) = terminal::size().unwrap_or((80, 24));
        let header = ascii::header(&sim);
        let header_lines = header.lines().count() as u16;
        let view_rows = term_rows
            .saturating_sub(header_lines)
            .saturating_sub(HINT_ROWS)
            .max(4);
        let view = Viewport::following(&sim, term_cols, view_rows);
        let body = ascii::frame_in(&sim, view);

        execute!(stdout, MoveTo(0, 0), Clear(ClearType::FromCursorDown))?;
        writeln!(stdout, "{header}")?;
        writeln!(stdout, "{body}")?;
        write!(stdout, "{}", hint())?;
        stdout.flush()?;

        // Render one final frame after the episode ends, then leave: the death or
        // victory position is the most interesting frame of the whole run.
        if sim.outcome().is_finished() {
            break;
        }

        // Wall-clock pacing for human eyes only; see the module doc comment.
        let elapsed = started.elapsed();
        if elapsed < FRAME_TIME {
            sleep(FRAME_TIME - elapsed);
        }
    }

    let summary = PlaySummary::of(&sim);
    // Restore the terminal before printing, or the summary is written into the
    // alternate screen and vanishes with it.
    drop(session);
    println!(
        "{}  {}  frames {}  best x {:.2}  progress {:.0}px",
        level.name,
        summary.outcome.as_str(),
        summary.frames,
        summary.best_x,
        summary.progress_pixels
    );
    Ok(summary)
}

/// The intents a key can hold down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Left,
    Right,
    Jump,
    Brake,
}

const HELD: [Held; 4] = [Held::Left, Held::Right, Held::Jump, Held::Brake];

/// Keyboard state, including the held-key emulation described on [`HOLD_WINDOW`].
///
/// `now` is a parameter rather than a clock read, so the whole state machine is
/// testable without a terminal and without sleeping.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Keys {
    /// One deadline per [`Held`] intent, or `None` when it is not held.
    until: [Option<Instant>; HELD.len()],
    /// One-shot, deliberately not latched: a latched restart would restart the level
    /// on every frame of the window, and a latched quit would quit several times.
    quit: bool,
    restart: bool,
}

impl Keys {
    fn press(&mut self, held: Held, now: Instant) {
        self.until[held as usize] = Some(now + HOLD_WINDOW);
    }

    /// End a latch early. Only terminals that report releases get here, and for them
    /// this is what makes a release instant rather than a coast.
    fn release(&mut self, held: Held) {
        self.until[held as usize] = None;
    }

    fn is_down(&self, held: Held, now: Instant) -> bool {
        matches!(self.until[held as usize], Some(until) if until > now)
    }

    /// The action for this frame, from the intents this frame's latch makes live.
    fn action(&self, now: Instant) -> AgentAction {
        action_for(
            self.is_down(Held::Left, now),
            self.is_down(Held::Right, now),
            self.is_down(Held::Jump, now),
            self.is_down(Held::Brake, now),
        )
    }

    fn take_restart(&mut self) -> bool {
        std::mem::take(&mut self.restart)
    }

    fn take_quit(&mut self) -> bool {
        std::mem::take(&mut self.quit)
    }
}

/// Drain every pending terminal event into `keys`.
///
/// Non-blocking (`poll` with a zero timeout) so the render loop keeps its frame rate.
fn read_keys(keys: &mut Keys, now: Instant) -> io::Result<()> {
    while event::poll(Duration::from_millis(0))? {
        let Ok(event) = event::read() else {
            break;
        };
        let Event::Key(key) = event else {
            continue;
        };
        let pressed = key.kind != KeyEventKind::Release;
        let mut hold = |held: Held| {
            if pressed {
                keys.press(held, now);
            } else {
                keys.release(held);
            }
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => keys.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                keys.quit = true
            }
            KeyCode::Char('r') | KeyCode::Char('R') => keys.restart = true,
            KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('A') => hold(Held::Left),
            KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('D') => hold(Held::Right),
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') | KeyCode::Char(' ') => {
                hold(Held::Jump)
            }
            KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('S') => hold(Held::Brake),
            _ => {}
        }
    }
    Ok(())
}

/// The action a set of held intents means.
///
/// Priority is the plan's action set, not the order keys were pressed in: jump
/// beats direction, so holding a direction while jumping is a real forward jump
/// rather than a standing one. There is no jump-left in the action set, so a neutral
/// jump with left held carries momentum leftward instead.
///
/// Public because there is more than one way to play: the terminal playtest learns
/// a press from a key event and latches it, the desktop shell reads the key state
/// directly, and both call this. One definition of the keyboard, so the two cannot
/// drift into being different games.
pub fn action_for(left: bool, right: bool, jump: bool, brake: bool) -> AgentAction {
    if jump && right {
        AgentAction::RunJump
    } else if jump {
        AgentAction::JumpInPlace
    } else if right {
        AgentAction::Run
    } else if left {
        AgentAction::Retreat
    } else if brake {
        AgentAction::Brake
    } else {
        AgentAction::Walk
    }
}

/// The control hint drawn under the level view.
pub fn hint() -> String {
    format!(
        "arrows/A,D move   space/W/up jump (hold + direction = run-jump)   down/S brake   R restart   Q/Esc quit\n\
         {}",
        ascii::legend().replace('\n', " | ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a level from ASCII art, the way the ascii tests do: escaped-quote JSON
    /// rather than a raw string, because a run of `#` collides with `r#"…"#`.
    fn fixture(rows: &[&str], spawn: (i32, i32), entities: &[(&str, i32, i32)]) -> Level {
        let width = rows[0].len();
        assert!(rows.iter().all(|r| r.len() == width), "ragged fixture");
        let tiles = rows
            .iter()
            .map(|r| format!("    \"{r}\""))
            .collect::<Vec<_>>()
            .join(",\n");
        let ents = entities
            .iter()
            .map(|(k, x, y)| format!("{{\"type\":\"{k}\",\"x\":{x},\"y\":{y}}}"))
            .collect::<Vec<_>>()
            .join(",");
        let src = format!(
            "{{\"format\":\"evo-platformer-level\",\"level_format_version\":1,\"name\":\"play-fixture\",\
             \"width\":{width},\"height\":{},\"spawn\":{{\"x\":{},\"y\":{}}},\n             \"tiles\":[\n{tiles}\n],\"entities\":[{ents}]}}",
            rows.len(),
            spawn.0,
            spawn.1
        );
        Level::from_json_str(&src).expect("fixture parses")
    }

    /// A 60-tile runway, ground in row 4, spawn at column 1, goal far away.
    fn runway() -> Level {
        let sky = ".".repeat(60);
        let ground = "#".repeat(60);
        fixture(
            &[&sky, &sky, &sky, &sky, &ground],
            (1, 3),
            &[("goal", 58, 3)],
        )
    }

    fn keys() -> Keys {
        Keys::default()
    }

    #[test]
    fn held_intents_map_to_the_six_actions_in_priority_order() {
        let now = Instant::now();
        let mut k = keys();
        assert_eq!(k.action(now), AgentAction::Walk);

        k.press(Held::Brake, now);
        assert_eq!(k.action(now), AgentAction::Brake);
        k.press(Held::Left, now);
        assert_eq!(k.action(now), AgentAction::Retreat);
        k.press(Held::Right, now);
        assert_eq!(k.action(now), AgentAction::Run);

        // Jump beats direction: holding both must be a real forward jump.
        k.press(Held::Jump, now);
        assert_eq!(k.action(now), AgentAction::RunJump);
        k.release(Held::Right);
        assert_eq!(k.action(now), AgentAction::JumpInPlace);

        // The action set has no jump-left; a neutral jump carries momentum instead.
        let mut left = keys();
        left.press(Held::Left, now);
        left.press(Held::Jump, now);
        assert_eq!(left.action(now), AgentAction::JumpInPlace);
    }

    #[test]
    fn a_press_latches_until_the_window_expires_and_a_release_ends_it_at_once() {
        let start = Instant::now();
        let mut k = keys();
        k.press(Held::Right, start);
        assert!(k.is_down(Held::Right, start));
        assert!(
            k.is_down(Held::Right, start + HOLD_WINDOW - Duration::from_millis(1)),
            "the latch must still be alive at the edge of its window"
        );
        assert!(
            !k.is_down(Held::Right, start + HOLD_WINDOW + Duration::from_millis(1)),
            "and gone once it expires, or a tap would move forever"
        );

        // A release ends it immediately, which is what makes a terminal that reports
        // releases feel crisp rather than coasting.
        let mut k = keys();
        k.press(Held::Jump, start);
        k.release(Held::Jump);
        assert!(!k.is_down(Held::Jump, start));
        assert_eq!(k.action(start), AgentAction::Walk);
    }

    #[test]
    fn quit_and_restart_are_one_shot() {
        let mut k = keys();
        k.quit = true;
        k.restart = true;
        assert!(k.take_quit());
        assert!(!k.take_quit(), "a latched quit would fire on every later frame");
        assert!(k.take_restart());
        assert!(!k.take_restart());
    }

    /// The bug this whole mechanism exists for, measured against the real sim: a
    /// terminal repeating at 5 presses a second (a slow-but-real desktop setting) has
    /// to drive the player as if the key were held. Before the latch, four seconds of
    /// this advanced the player **3 pixels** on L1 — the player appeared stuck, and the
    /// only thing that worked was tapping faster than the frame rate.
    #[test]
    fn a_slow_auto_repeat_still_runs_the_player() {
        let level = runway();
        let mut sim = Sim::new(&level, 1);
        let mut k = keys();

        let start = Instant::now();
        let mut now = start;
        let mut next_press = start;
        let two_seconds = 120;
        for _ in 0..two_seconds {
            if now >= next_press {
                k.press(Held::Right, now);
                next_press = now + Duration::from_millis(200);
            }
            sim.step(k.action(now));
            now += FRAME_TIME;
        }

        // Two seconds of running covers roughly eleven tiles from a standing start;
        // the assertion is deliberately loose, and the point is the order of
        // magnitude against the 0.2 tiles a per-press step produced.
        assert!(
            sim.stats().best_x > 8.0,
            "holding right for two seconds should cross tiles, not pixels: best_x {}",
            sim.stats().best_x
        );
        assert!(
            sim.outcome() == Outcome::Running,
            "the runway is long enough for two seconds of running: {:?}",
            sim.outcome()
        );
    }

    /// And the converse: with no press at all the player must not drift. A latch that
    /// never expired would keep the level walking itself forward.
    #[test]
    fn without_a_press_the_player_stays_put() {
        let level = runway();
        let mut sim = Sim::new(&level, 1);
        let k = keys();
        let start = Instant::now();
        for i in 0..120 {
            sim.step(k.action(start + FRAME_TIME * i));
        }
        assert!(
            sim.stats().best_x < 4.0,
            "idle frames must not move the player: best_x {}",
            sim.stats().best_x
        );
    }

    #[test]
    fn hint_mentions_every_control() {
        let h = hint();
        for needle in ["move", "jump", "brake", "restart", "quit", "solid tile"] {
            assert!(h.contains(needle), "hint is missing {needle:?}: {h}");
        }
    }

    #[test]
    fn frame_time_matches_the_sim_tick() {
        assert_eq!(FRAME_TIME.as_millis(), 16);
    }
}
