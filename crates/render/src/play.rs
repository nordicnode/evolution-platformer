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

    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;

    loop {
        let started = Instant::now();

        let intent = read_intent()?;
        if intent.quit {
            break;
        }
        if intent.restart {
            sim = Sim::new(level, seed);
        }

        sim.step(intent.action());

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

/// One frame's worth of keyboard intent, latched from every pending event.
struct Intent {
    left: bool,
    right: bool,
    jump: bool,
    brake: bool,
    restart: bool,
    quit: bool,
}

impl Intent {
    fn action(&self) -> AgentAction {
        if self.jump && self.right {
            AgentAction::RunJump
        } else if self.jump {
            AgentAction::JumpInPlace
        } else if self.right {
            AgentAction::Run
        } else if self.left {
            AgentAction::Retreat
        } else if self.brake {
            AgentAction::Brake
        } else {
            AgentAction::Walk
        }
    }
}

/// Drain every pending terminal event and latch it into one [`Intent`].
///
/// Non-blocking (`poll` with a zero timeout) so the render loop keeps its frame
/// rate. Key *release* events are ignored; a held key arrives as repeats on the
/// terminals that support them.
fn read_intent() -> io::Result<Intent> {
    let mut intent = Intent {
        left: false,
        right: false,
        jump: false,
        brake: false,
        restart: false,
        quit: false,
    };

    while event::poll(Duration::from_millis(0))? {
        let Ok(event) = event::read() else {
            break;
        };
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => intent.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                intent.quit = true
            }
            KeyCode::Char('r') | KeyCode::Char('R') => intent.restart = true,
            KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('A') => intent.left = true,
            KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('D') => intent.right = true,
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') | KeyCode::Char(' ') => {
                intent.jump = true
            }
            KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('S') => intent.brake = true,
            _ => {}
        }
    }
    Ok(intent)
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

    #[test]
    fn intent_maps_to_the_six_actions_in_priority_order() {
        let base = Intent {
            left: false,
            right: false,
            jump: false,
            brake: false,
            restart: false,
            quit: false,
        };
        assert_eq!(base.action(), AgentAction::Walk);
        assert_eq!(
            Intent { brake: true, ..base }.action(),
            AgentAction::Brake
        );
        assert_eq!(
            Intent { left: true, ..base }.action(),
            AgentAction::Retreat
        );
        assert_eq!(Intent { right: true, ..base }.action(), AgentAction::Run);
        assert_eq!(
            Intent { jump: true, ..base }.action(),
            AgentAction::JumpInPlace
        );
        // Jump beats direction: holding both must be a real forward jump.
        assert_eq!(
            Intent {
                jump: true,
                right: true,
                ..base
            }
            .action(),
            AgentAction::RunJump
        );
        assert_eq!(
            Intent {
                jump: true,
                left: true,
                ..base
            }
            .action(),
            AgentAction::JumpInPlace,
            "the action set has no jump-left; a neutral jump carries momentum instead"
        );
        assert_eq!(
            Intent {
                jump: true,
                brake: true,
                ..base
            }
            .action(),
            AgentAction::JumpInPlace
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
