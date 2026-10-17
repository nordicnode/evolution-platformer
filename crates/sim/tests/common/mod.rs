//! Shared helpers for the `sim` integration tests.
//!
//! Integration tests are separate crates: they cannot see `sim`'s `#[cfg(test)]`
//! items, nor its dependencies. Everything here is therefore built from `sim`'s
//! public API plus this file's own dependency-free helpers.

#![allow(dead_code)]

/// Build level JSON from ASCII-art rows.
///
/// Deliberately *not* a raw string literal: fixtures contain runs of `#`
/// immediately after a quote (`"####"`), which terminates `r#"…"#` and
/// `r##"…"##` alike. Formatting the JSON also keeps fixtures readable as art.
pub fn fixture_src<S: AsRef<str>>(
    name: &str,
    rows: &[S],
    spawn: (i32, i32),
    entities: &[(&str, i32, i32)],
    level_format_version: u32,
) -> String {
    let width = rows
        .first()
        .map(|r| r.as_ref().chars().count())
        .unwrap_or(0);
    let tiles = rows
        .iter()
        .map(|r| format!("\"{}\"", r.as_ref()))
        .collect::<Vec<_>>()
        .join(",");
    let ents = entities
        .iter()
        .map(|(kind, x, y)| format!("{{\"type\":\"{kind}\",\"x\":{x},\"y\":{y}}}"))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        concat!(
            "{{",
            "\"format\":\"evo-platformer-level\",",
            "\"level_format_version\":{version},",
            "\"name\":\"{name}\",",
            "\"width\":{width},",
            "\"height\":{height},",
            "\"spawn\":{{\"x\":{sx},\"y\":{sy}}},",
            "\"tiles\":[{tiles}],",
            "\"entities\":[{ents}]",
            "}}"
        ),
        version = level_format_version,
        name = name,
        width = width,
        height = rows.len(),
        sx = spawn.0,
        sy = spawn.1,
        tiles = tiles,
        ents = ents
    )
}

/// A parsed fixture level.
///
/// Panics with the offending source text on failure, and pre-checks that the art
/// is rectangular and uses only legal glyphs — a typo in a fixture should say so
/// rather than surface as a puzzling physics assertion.
pub fn fixture<S: AsRef<str>>(
    name: &str,
    rows: &[S],
    spawn: (i32, i32),
    entities: &[(&str, i32, i32)],
) -> sim::Level {
    assert!(!rows.is_empty(), "fixture {name} has no rows");
    let width = rows[0].as_ref().chars().count();
    for (i, row) in rows.iter().enumerate() {
        let row = row.as_ref();
        assert_eq!(
            row.chars().count(),
            width,
            "fixture {name} row {i} is not {width} columns wide: {row:?}"
        );
        for (c, ch) in row.chars().enumerate() {
            assert!(
                matches!(
                    ch,
                    '.' | '#'
                        | 'S'
                        | 'B'
                        | 'W'
                        | '^'
                        | 'G'
                        | 'd'
                        | 's'
                        | 'b'
                        | 'w'
                        | 'v'
                        | '~'
                        | 'L'
                        | 'V'
                        | '='
                ),
                "fixture {name} row {i} column {c} has illegal glyph {ch:?}"
            );
        }
    }
    let src = fixture_src(name, rows, spawn, entities, 1);
    match sim::Level::from_json_str(&src) {
        Ok(level) => level,
        Err(err) => panic!("fixture {name} failed to parse: {err}\nsource: {src}"),
    }
}

/// Load one of the shipped levels from the repository's `levels/` directory.
pub fn level(slug: &str) -> sim::Level {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../levels")
        .join(format!("{slug}.json"));
    match sim::Level::from_json_file(&path) {
        Ok(level) => level,
        Err(err) => panic!("shipped level {slug} failed to load: {err}"),
    }
}

/// Settle a freshly spawned sim onto the ground and clear the jump-button edge.
///
/// `PlayerState::spawn` leaves `grounded == false`, and the jump is
/// edge-triggered on `jump_held_prev`, so a jump action issued in the first ticks
/// of an episode is silently ignored. Tests that want a jump on demand settle
/// first and then assert they are actually on the ground.
pub fn settle(sim: &mut sim::Sim) {
    for _ in 0..4 {
        sim.step(sim::AgentAction::Walk);
    }
    assert!(
        sim.player().grounded,
        "settle() did not put the body on the ground"
    );
}

/// Step a fixed action `n` times.
pub fn step_n(sim: &mut sim::Sim, action: sim::AgentAction, n: u32) {
    for _ in 0..n {
        sim.step(action);
    }
}

/// Step until the episode finishes, with a hard cap so a broken sim cannot hang
/// the suite. Returns the number of steps taken.
pub fn run_until_finished(sim: &mut sim::Sim, action: sim::AgentAction, cap: u32) -> u32 {
    let mut steps = 0;
    while !sim.outcome().is_finished() && steps < cap {
        sim.step(action);
        steps += 1;
    }
    steps
}

/// Ground row 3, spawn row 2, width 2000.
///
/// The width is the point. `max_frames` is `width * FRAMES_PER_TILE`, i.e. 80_000,
/// so the frame cap cannot end an episode inside a 10_000-frame test, and the goal
/// is ~19_900 ticks away. A test can therefore simulate a genuine 10_000 frames
/// with the episode still running.
pub const LONG_RUNWAY_WIDTH: i32 = 2000;

/// A hazard-free runway long enough to exhaust a 10_000-frame budget.
pub fn long_runway() -> sim::Level {
    let sky = ".".repeat(LONG_RUNWAY_WIDTH as usize);
    let ground = "#".repeat(LONG_RUNWAY_WIDTH as usize);
    let rows = vec![sky.clone(), sky.clone(), sky.clone(), ground];
    fixture(
        "long-runway",
        &rows,
        (1, 2),
        &[("goal", LONG_RUNWAY_WIDTH - 1, 2)],
    )
}

/// A dependency-free SplitMix64-style generator.
///
/// The integration tests cannot reach `sim`'s own RNG, and they must not depend
/// on it either: nothing about the sim's determinism may hinge on the test
/// harness's action sequences. This is deliberately its own tiny copy.
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Lcg {
        Lcg(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, n)`.
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0, "below(0) has no valid output");
        ((self.next_u64() >> 32) as u32) % n
    }
}

/// Pseudo-random actions covering the whole action space.
pub fn fixed_actions(seed: u64, n: usize) -> Vec<sim::AgentAction> {
    let mut lcg = Lcg::new(seed);
    (0..n)
        .map(|_| {
            let i = lcg.below(sim::AgentAction::COUNT as u32) as usize;
            sim::AgentAction::from_index(i)
        })
        .collect()
}

/// Mostly `Run`, with a hop every 24 ticks.
///
/// Keeps an agent advancing to the right, so an episode on a long level runs its
/// whole frame budget instead of tripping the no-progress rule.
pub fn forward_actions(seed: u64, n: usize) -> Vec<sim::AgentAction> {
    let mut lcg = Lcg::new(seed);
    (0..n)
        .map(|i| {
            if i % 24 == 23 {
                sim::AgentAction::RunJump
            } else if lcg.below(64) == 0 {
                sim::AgentAction::JumpInPlace
            } else {
                sim::AgentAction::Run
            }
        })
        .collect()
}
