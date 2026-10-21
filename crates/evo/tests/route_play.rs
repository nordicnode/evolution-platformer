//! The route replay, checked against the shipped pack.
//!
//! A replay is the oracle's route driven through the real episode loop, so it is the one artifact in
//! the workspace that can answer "is this level beatable *by a body*" without evolution — and it is
//! the material the imitation warm start is built from. What it must be is **honest**: a walk that
//! reaches the flag, or a named reason it did not.
//!
//! The measurement this file pins, ten episode seeds per level (`EVO_REPLAY_VERDICT` below is the
//! table, `W` = won, `D` = died, `S` = stuck):
//!
//! | level | walkers | seeds 1–10 |
//! |---|---|---|
//! | `l1-flat-run`, `l2-first-gap`, `l4-rhythm`, `l5-tightrope`, `l7-ascent`, `l7-wall-school` | 0 | `WWWWWWWWWW` |
//! | `l3-gauntlet` | 1 | `WWDWDWDDWD` |
//! | `l9-wait-and-climb` | 1 | `WWDWDWDDWD` |
//! | `l6-marathon` | 1 | `DDDDDDDDDD` |
//! | `l8-long-jump` | 1 | `DDDDDDDDDD` |
//!
//! Three claims come out of that, and they are what this file checks:
//!
//! 1. **A route over static terrain is walked, deterministically.** The six walker-free levels are
//!    10 of 10, and they are the ones that make the replay prove its parts: `l1` is nothing but
//!    distance (the floor of the mechanism), `l2` needs a jump, `l5` a pit crossed at a precise
//!    take-off, `l7-ascent` and `l7-wall-school` a wall climb (the arm that is a rhythm rather than a
//!    press). A pad (`v`) is covered by the unit tests beside the oracle, where a fixture can be
//!    built to the pad's exact tile.
//! 2. **A patrol is the one thing a static plan cannot express.** `l3-gauntlet` and
//!    `l9-wait-and-climb` walk five seeds in ten: which five is the patrol's phase, drawn from the
//!    episode seed, and a route that arrives on the wrong phase is blocked. The replay says so
//!    rather than pretending, and the levels it *does* walk are walked all the way (all eight of
//!    `l9`'s hops).
//! 3. **Two levels are named failures, not silence.** `l6-marathon` closes **all five** of its route
//!    hops and then dies on the walk to the flag — the route is executed perfectly and a patrol on
//!    the last stretch kills the body. `l8-long-jump` is the open discrepancy: the local sweep that
//!    validates a press (`route_play`'s `press_lands`, added because this level's press was being
//!    made half a block past its aim) says the move clears the hop from where the body is, and the
//!    live flight lands short of the promised stand. Same position, same speed, same action — so the
//!    gap is the oracle's *clean spawn state* against the state a live body carries into the press,
//!    and on a five-tile pit against a sprint jump that difference is a landing. It is pinned here
//!    as a known failure with its trace rather than tuned away, because a replay that quietly stopped
//!    reporting this would stop being the instrument this pack is checked with.

use evo::{replay, EvalConfig};
use sim::Level;

/// The verdict per level over seeds 1–10, as measured (see the module docs for the reading).
///
/// A string rather than a set of levels: the *rate* is the claim — "walked on the seeds whose patrol
/// phase allows it" is a different sentence from "walked", and the difference is what a level's
/// difficulty report should carry.
const VERDICT: [(&str, &str); 10] = [
    ("l1-flat-run", "WWWWWWWWWW"),
    ("l2-first-gap", "WWWWWWWWWW"),
    ("l3-gauntlet", "WWDWDWDDWD"),
    ("l4-rhythm", "WWWWWWWWWW"),
    ("l5-tightrope", "WWWWWWWWWW"),
    ("l6-marathon", "DDDDDDDDDD"),
    ("l7-ascent", "WWWWWWWWWW"),
    ("l7-wall-school", "WWWWWWWWWW"),
    ("l8-long-jump", "DDDDDDDDDD"),
    ("l9-wait-and-climb", "WWDWDWDDWD"),
];

fn level(slug: &str) -> Level {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../levels")
        .join(format!("{slug}.json"));
    match Level::from_json_file(&path) {
        Ok(level) => level,
        Err(err) => panic!("shipped level {slug} failed to load: {err}"),
    }
}

/// The whole table, re-derived. A level whose verdict moved in *either* direction fails: a level
/// that stops being walked is a regression in the oracle, the controller or the physics, and a level
/// the replay can now walk is a number every report in `FINDINGS.md` should hear about.
#[test]
fn the_replay_walks_exactly_what_the_table_says() {
    let cfg = EvalConfig::default();
    let mut mismatches = Vec::new();
    for (slug, expected) in VERDICT {
        let level = level(slug);
        let mut got = String::new();
        for seed in 1..=10u64 {
            let play = replay(&level, seed, &cfg);
            got.push(match play.outcome {
                sim::Outcome::Won => 'W',
                sim::Outcome::Died => 'D',
                sim::Outcome::Stuck => 'S',
                sim::Outcome::TimedOut => 'T',
                sim::Outcome::Running => 'R',
            });
        }
        if got != expected {
            let first = expected
                .chars()
                .zip(got.chars())
                .position(|(a, b)| a != b)
                .map(|i| i as u64 + 1)
                .unwrap_or(1);
            let play = replay(&level, first, &cfg);
            mismatches.push(format!(
                "{slug}: expected {expected}, got {got}\n  seed {first}: {}\n{}",
                play.note,
                play.trace.join("\n")
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "the replay's verdicts moved:\n\n{}",
        mismatches.join("\n\n")
    );
}

/// A walk that wins closes hops, and a walk that fails names why.
///
/// The point of the second half is that a failure is *legible*: `l6-marathon` closed all five hops
/// before it died, which is a different defect from a hop that could not be made at all, and the
/// trace has to be the thing that says which.
#[test]
fn a_verdict_comes_with_its_reason() {
    let cfg = EvalConfig::default();

    let won = replay(&level("l9-wait-and-climb"), 1, &cfg);
    assert!(won.won(), "{}", won.note);
    assert_eq!(
        won.route_hops,
        Some(won.hops_closed),
        "a win has to close the whole route: {} hops of {:?}",
        won.hops_closed,
        won.route_hops
    );
    assert!(
        won.decisions.len() > 4,
        "a win in {} decisions is not a route, it is a spawn on the flag",
        won.decisions.len()
    );

    let lost = replay(&level("l6-marathon"), 1, &cfg);
    assert!(!lost.won());
    assert!(
        lost.note.contains("Died"),
        "a failed walk has to say how it ended: {}",
        lost.note
    );
    assert!(
        lost.trace.len() > 1,
        "a failed walk with an empty trace is the silence this file exists to prevent"
    );
    // The route itself was executed: this level's failure is *past* the last hop, which is what
    // makes it a patrol story rather than a crossing story.
    assert_eq!(
        lost.hops_closed,
        lost.route_hops.unwrap_or(u32::MAX),
        "l6-marathon is the level the replay walks hop for hop and then loses on the last stretch; \
         if it now fails a hop, the reading has changed and the table needs re-measuring"
    );
}

/// **A replay is a function of `(level, seed)`**, so two of them agree exactly.
///
/// The demonstration is only useful as *the* witness for a level: if two replays of one level could
/// differ, the pairs imitated from one of them would be a claim about one run of the controller
/// rather than about the level. The controller reads the live sim and the sim is a pure function of
/// its seed, so this is expected to hold — and it is cheap enough to hold rather than hope, because
/// the patrol phases are drawn from that same seed.
#[test]
fn a_replay_is_deterministic() {
    for slug in ["l1-flat-run", "l2-first-gap", "l9-wait-and-climb"] {
        let lv = level(slug);
        let a = replay(&lv, 7, &EvalConfig::default());
        let b = replay(&lv, 7, &EvalConfig::default());
        assert_eq!(a.decisions, b.decisions, "{slug}: two replays disagreed");
        assert_eq!(
            a.frames, b.frames,
            "{slug}: two replays took different times"
        );
        assert_eq!(
            a.outcome, b.outcome,
            "{slug}: two replays ended differently"
        );
    }
}

/// The `sim` element tests' vine-block fixture, in the level format: a vine hanging from the middle
/// of a one-tile-thick block, with the flag beside it on top. The only way to the flag is over the
/// block the vine hangs from, so a replay that wins this has pulled up.
fn vine_block() -> Level {
    let rows = [
        "..............",
        "..............",
        "....######....",
        "......V.......",
        "......V.......",
        "......V.......",
        "......V.......",
        "..............",
        "##############",
        "##############",
    ];
    let tiles = rows
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(",");
    let src = format!(
        "{{\"format\":\"evo-platformer-level\",\"level_format_version\":1,\
         \"name\":\"vine-block\",\"width\":{},\"height\":{},\
         \"spawn\":{{\"x\":6,\"y\":7}},\"tiles\":[{tiles}],\
         \"entities\":[{{\"type\":\"goal\",\"x\":10,\"y\":1}}]}}",
        rows[0].len(),
        rows.len()
    );
    Level::from_json_str(&src).expect("vine-block parses")
}

/// **A vine over the block it hangs from is a route a replay can walk.**
///
/// The oracle answering "reachable" is only half the claim: the plan has to be *executable*, and this
/// presses it through the real episode loop. The up-exit is the interesting part — the replay holds
/// the climb with no direction, and the physics' pull-up is what puts the body on the block — so a
/// plan that only worked as arithmetic would fail here.
#[test]
fn the_replay_pulls_up_over_the_block_a_vine_hangs_from() {
    let play = replay(&vine_block(), 3, &EvalConfig::default());
    assert!(play.won(), "{}", play.trace.join("\n"));
}

/// **The route walk swings.** A rope crossing is the one move whose outcome is decided by *when*
/// the button comes up, so it is the strongest check the replay has: the plan carries a release
/// tick, the replay's swing clock counts the ticks the body is actually on the line, and a spin of
/// either by one is a landing in the pit. The scratch level's gap is wider than a sprint jump, so a
/// win here is a swing and cannot be anything else.
#[test]
fn the_replay_swings_across_the_pit() {
    let level = level("scratch-rope");
    let cfg = EvalConfig::default();
    for seed in 1..=5 {
        let play = replay(&level, seed, &cfg);
        assert!(
            play.won(),
            "seed {seed}: the rope crossing did not replay:\n{}",
            play.trace.join("\n")
        );
    }
    let play = replay(&level, 3, &cfg);
    assert!(
        play.trace
            .iter()
            .any(|line| line.contains("Rope") && line.contains("(1 hops closed)")),
        "the win has to be the swing's:\n{}",
        play.trace.join("\n")
    );
}
