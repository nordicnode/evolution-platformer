//! The keys, on one card, in the view they belong to.
//!
//! The window has no menus and about two dozen keys, and until this module existed the only
//! teaching anywhere was the hover text on individual widgets plus the title screen's two
//! lines: a person who wanted to know how to pause a run, slow it down or scrub the record
//! had to guess, and a guess is exactly what a *game* should never require of its controls
//! (AUDIT.md F1). `?` (or `F1`) opens this card, and what it lists is the keys of whichever
//! view is on screen — four views, four cards, one key.
//!
//! **The card is a list, not a generator.** The bindings live in three readers
//! ([`crate::app::EvoApp::handle_shortcuts`], [`crate::app::EvoApp::replay_keys`] and
//! [`crate::panel::commands_for`], which the two run views share) and this is a second
//! *statement* of them, which is a maintenance debt accepted on purpose: a sheet generated
//! from those readers would need every key to be routed through one table first, and that
//! refactor is worth doing on the day the table starts disagreeing with them. What is tested
//! here is what can be: that every view has a card, that no card claims the same key twice,
//! and that the close key is on every card — so a row that is added in haste is caught by the
//! suite even though a row that is *forgotten* cannot be.
//!
//! The rows are also written in the game's voice rather than a manual's: `W` is "jump — hold
//! it to jump higher" because that is the fact a player needs and the one the physics rewards,
//! and `N` is "run one generation, then hold" because that is what inspecting a run means.

use egui::{Context, RichText};

use crate::app::Mode;
use crate::theme;

/// One key and what it does.
pub struct Key {
    /// What to press, written the way a keyboard has it printed on it.
    pub keys: &'static str,
    /// What pressing it does, in one line.
    pub what: &'static str,
}

/// The keys that open and close the card, named on every card so a viewer who opened it by
/// accident can see the way out.
pub const CLOSE: &str = "?  or  F1";

/// The editor's keys.
const EDIT: &[Key] = &[
    Key {
        keys: "left drag",
        what: "paint with the held brush",
    },
    Key {
        keys: "right drag",
        what: "erase — the entity first, then the terrain",
    },
    Key {
        keys: "middle drag",
        what: "move the level under the pointer (alt-drag does it too)",
    },
    Key {
        keys: "Ctrl+click",
        what: "take the brush from what is under the pointer",
    },
    Key {
        keys: "shift drag",
        what: "a straight line from where the drag started",
    },
    Key {
        keys: "wheel",
        what: "zoom",
    },
    Key {
        keys: "Ctrl+Z",
        what: "undo — one stroke at a time",
    },
    Key {
        keys: "Ctrl+Shift+Z",
        what: "redo",
    },
    Key {
        keys: "Ctrl+S",
        what: "save the level to its file",
    },
    Key {
        keys: "/",
        what: "search the palette — every part, whichever shelf it is on",
    },
    Key {
        keys: "G",
        what: "the guides — grid, reachability and the verdict",
    },
    Key {
        keys: "F",
        what: "fit the whole level in the window",
    },
    Key {
        keys: "P",
        what: "play it yourself",
    },
    Key {
        keys: "E",
        what: "evolve an AI on it",
    },
    Key {
        keys: "V",
        what: "watch the run's best agent",
    },
    Key {
        keys: "M",
        what: "sound on or off",
    },
];

/// The playtest's keys: the same physics and the same action set the evolution sees.
const PLAY: &[Key] = &[
    Key {
        keys: "A  /  \u{2190}",
        what: "run left",
    },
    Key {
        keys: "D  /  \u{2192}",
        what: "run right",
    },
    Key {
        keys: "W  /  \u{2191}  /  space",
        what: "jump — hold it to jump higher",
    },
    Key {
        keys: "shift",
        what: "sprint while held",
    },
    Key {
        keys: "S  /  \u{2193}",
        what: "brake",
    },
    Key {
        keys: "T",
        what: "hand the level to the AI — it plans the route and plays it, or gives the keys back",
    },
    Key {
        keys: "R",
        what: "start the run again",
    },
    Key {
        keys: "G",
        what: "the AI's best route, ghosted beside you",
    },
    Key {
        keys: "F",
        what: "fit the whole level in the window",
    },
    Key {
        keys: "M",
        what: "sound on or off",
    },
    Key {
        keys: "P  /  Esc",
        what: "back to the editor",
    },
];

/// The evolution view's keys: the terminal watch mode's own, so the two front-ends teach one
/// keyboard rather than two.
const EVOLVE: &[Key] = &[
    Key {
        keys: "space",
        what: "pause the run",
    },
    Key {
        keys: "N",
        what: "score one generation, then hold",
    },
    Key {
        keys: "+  /  -",
        what: "speed the animation up or down",
    },
    Key {
        keys: "shift+M",
        what: "raise the mutation rate",
    },
    Key {
        keys: "M",
        what: "lower the mutation rate",
    },
    Key {
        keys: "H",
        what: "back to the live generation",
    },
    Key {
        keys: "\u{2190}  /  \u{2192}",
        what: "step through the generations the run has recorded",
    },
    Key {
        keys: "F",
        what: "follow the leader, or show the whole level",
    },
    Key {
        keys: "G",
        what: "ghosts — every agent in the generation, or none",
    },
    Key {
        keys: "S",
        what: "save the champion",
    },
    Key {
        keys: "C",
        what: "continue from the champion saved for this level",
    },
    Key {
        keys: "V",
        what: "watch the best agent run it",
    },
    Key {
        keys: "K",
        what: "the trick codex — what this level has produced",
    },
    Key {
        keys: "P  /  E  /  Esc",
        what: "back to the editor",
    },
];

/// The replay's keys: a transport for one episode.
const REPLAY: &[Key] = &[
    Key {
        keys: "space",
        what: "pause the episode",
    },
    Key {
        keys: "+  /  -",
        what: "play it faster or slower",
    },
    Key {
        keys: "G",
        what: "the rest of the generation, ghosted beside the star",
    },
    Key {
        keys: "F",
        what: "follow the star, or show the whole level",
    },
    Key {
        keys: "R",
        what: "run those episodes again",
    },
    Key {
        keys: "S",
        what: "save the champion",
    },
    Key {
        keys: "K",
        what: "the trick codex — what this level has produced",
    },
    Key {
        keys: "M",
        what: "sound on or off",
    },
    Key {
        keys: "V  /  P  /  E  /  Esc",
        what: "leave the replay",
    },
];

/// The card for a view: its name, and its keys.
pub fn rows(mode: Mode) -> (&'static str, &'static [Key]) {
    match mode {
        Mode::Edit => ("Edit", EDIT),
        Mode::Play => ("Play", PLAY),
        Mode::Evolve => ("Evolve", EVOLVE),
        Mode::Replay => ("Replay", REPLAY),
    }
}

/// Draw the card, if it is open.
///
/// A floating window rather than a panel, for the same reason the shelf is one: it is a thing
/// the viewer *opened*, and it belongs over whatever they were looking at rather than beside
/// it. `open` is the caller's flag, so the window's own close box and the `?` key are one
/// piece of state.
pub fn window(ctx: &Context, mode: Mode, open: &mut bool) {
    if !*open {
        return;
    }
    let (view, rows) = rows(mode);
    egui::Window::new(format!("Keys — {view}"))
        .open(open)
        .resizable(false)
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!("the {view} view, and nothing else's keys"))
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.separator();
            egui::Grid::new("keys-grid")
                .num_columns(2)
                .spacing([18.0, 5.0])
                .show(ui, |ui| {
                    for row in rows {
                        ui.label(
                            RichText::new(row.keys)
                                .monospace()
                                .strong()
                                .color(theme::ACCENT_INK),
                        );
                        ui.label(RichText::new(row.what).color(theme::TEXT));
                        ui.end_row();
                    }
                    // The row every card has, written in the same grid as the rest so it reads
                    // as one more key rather than as a footnote.
                    ui.label(
                        RichText::new(CLOSE)
                            .monospace()
                            .strong()
                            .color(theme::ACCENT_INK),
                    );
                    ui.label(RichText::new("close this card").color(theme::TEXT));
                    ui.end_row();
                });
            ui.separator();
            ui.label(
                RichText::new(
                    "the dock's buttons do the same jobs by pointer, and the details window \
                     renames the level",
                )
                .small()
                .color(theme::TEXT_DIM),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every view has a card, and it is not empty.
    ///
    /// The bug this exists for is a person who cannot find out how to pause a run: a card that
    /// is missing, or blank, is that bug back again.
    #[test]
    fn every_view_has_a_card_with_rows_on_it() {
        for mode in Mode::ALL {
            let (view, rows) = rows(mode);
            assert!(!view.is_empty());
            assert!(
                rows.len() >= 6,
                "the {view} card has only {} rows",
                rows.len()
            );
            for row in rows {
                assert!(!row.keys.is_empty(), "a {view} row has no key");
                assert!(
                    !row.what.is_empty(),
                    "a {view} row has no words: {}",
                    row.keys
                );
            }
        }
    }

    /// No card names the same key twice, and no card forgets the way out.
    ///
    /// Two rows for one key is how a card starts lying: the second row is the one that is
    /// wrong, and a person reads the first. The close key is asserted separately because it is
    /// the one row that is not in the table — it is added by [`window`] — and a card you can
    /// open and not close is worse than no card.
    #[test]
    fn no_card_claims_one_key_twice_and_every_card_shows_the_way_out() {
        for mode in Mode::ALL {
            let (view, rows) = rows(mode);
            for (i, a) in rows.iter().enumerate() {
                for b in &rows[i + 1..] {
                    assert_ne!(a.keys, b.keys, "the {view} card lists {} twice", a.keys);
                }
            }
            assert!(!CLOSE.is_empty());
        }
    }

    /// The four cards are four different lists: a view's keys are its own.
    ///
    /// The `?` key belongs to all of them and to none of the tables, so a card that lists it as
    /// a row would be the one row that is a lie — the key is bound in one place for all four
    /// views, not per view.
    #[test]
    fn a_view_does_not_borrow_another_views_keys() {
        for mode in Mode::ALL {
            let (view, rows) = rows(mode);
            for row in rows {
                assert!(
                    !row.keys.contains('?') && !row.keys.contains("F1"),
                    "the {view} card lists the close key as one of its own: {}",
                    row.keys
                );
            }
        }
        // And the playtest's own action keys are on the playtest's card and nowhere else: the
        // editor has no `W` and no `space` at all, and the two run views must not offer the
        // jump (they have `space` of their own, which is *pause* — the row is asserted above).
        let (_, play) = rows(Mode::Play);
        assert!(play.iter().any(|k| k.keys.contains('W')));
        let (_, edit) = rows(Mode::Edit);
        assert!(
            !edit
                .iter()
                .any(|k| k.keys.contains('W') || k.keys.contains("space")),
            "the editor's card offers the playtest's keys"
        );
        for mode in [Mode::Evolve, Mode::Replay] {
            let (view, rows) = rows(mode);
            assert!(
                !rows.iter().any(|k| k.keys.contains('W')),
                "the {view} card offers the playtest's jump"
            );
        }
    }
}
