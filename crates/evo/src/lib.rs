//! Fixed-topology neuroevolution (development_plan.md §7).
//!
//! The population is a plain genetic algorithm over a fixed 90 → 16 → 6 network.
//! NEAT-style topology search is deliberately absent: a 2021 Nature
//! Communications robot-swarm study found plain fixed-topology GA tied CMA-ES,
//! xNES and NEAT at this scale, and MarI/O's winning network had under a dozen
//! neurons. Speciation/complexification is gated on fixed topologies visibly
//! plateauing, and is not scheduled (§12 M5).
//!
//! This crate depends on `sim` and nothing else from the workspace. It contains
//! no rendering and no terminal I/O.

pub mod champion;
pub mod fitness;
pub mod ga;
pub mod genome;
pub mod population;

pub use champion::{
    check_reproducibility, ChampionFile, ChampionMetadata, NetworkSpec, TrainingSummary,
    CHAMPION_FORMAT, CHAMPION_VERSION,
};
pub use population::{PopulationFile, POPULATION_FORMAT, POPULATION_VERSION};
pub use fitness::{
    run_episode, score, EpisodeResult, EpisodeRunner, EvalConfig, ACTION_REPEAT, FITNESS_VERSION,
};
pub use ga::{
    evaluate_population, BestEver, GaConfig, GenerationReport, GenRow, LaneKind, ShowcaseLane,
    Trainer,
};
pub use genome::{default_shape, ActScratch, Genome, NetShape, N_INPUTS, N_OUTPUTS};

/// Test-only level construction, shared by this crate's unit tests.
///
/// Fixtures are built from ASCII-art rows rather than inline JSON literals: a run
/// of `#` inside a raw string collides with the raw-string delimiter, and the
/// alternative (dodging the delimiter with ever more `#`s) makes the fixtures
/// unreadable. The same helper exists in `sim::reach`'s tests for the same reason.
#[cfg(test)]
pub(crate) mod fixture {
    use sim::Level;

    const GLYPHS: [char; 4] = ['.', '#', '^', 'G'];

    pub fn level_src(
        name: &str,
        rows: &[&str],
        spawn: (i32, i32),
        entities: &[(&str, i32, i32)],
    ) -> String {
        let width = rows[0].len();
        assert!(
            rows.iter().all(|r| r.len() == width),
            "ragged fixture rows in {name}"
        );
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                assert!(GLYPHS.contains(&c), "bad glyph {c:?} at {x},{y} in {name}");
            }
        }
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
        format!(
            "{{\"format\":\"evo-platformer-level\",\"level_format_version\":1,\"name\":\"{name}\",
               \"width\":{width},\"height\":{},\"spawn\":{{\"x\":{},\"y\":{}}},
               \"tiles\":[\n{tiles}\n],\"entities\":[{ents}]}}",
            rows.len(),
            spawn.0,
            spawn.1
        )
    }

    pub fn level(
        name: &str,
        rows: &[&str],
        spawn: (i32, i32),
        entities: &[(&str, i32, i32)],
    ) -> Level {
        Level::from_json_str(&level_src(name, rows, spawn, entities)).expect("fixture parses")
    }

    /// A flat runway `width` tiles long with the goal at `goal_x`, ground row 6.
    /// The workhorse fixture: it isolates the policy from terrain.
    ///
    /// Note this level is solved by a *constant* `run` policy, and the all-zeros
    /// genome is exactly that (its outputs tie at 0 and argmax breaks low). Any
    /// test that needs to prove something about evolution should use
    /// [`gap_runway`] instead.
    pub fn flat_runway(name: &str, width: i32, goal_x: i32) -> Level {
        runway(name, width, goal_x, &[])
    }

    /// A runway with pits at the given `(start, len)` spans. A constant `run`
    /// policy falls into the first pit, so solving this requires the population to
    /// learn *when* to jump — the point of the GA smoke test.
    pub fn gap_runway(name: &str, width: i32, goal_x: i32, pits: &[(i32, i32)]) -> Level {
        runway(name, width, goal_x, pits)
    }

    /// A flat runway carrying a single patrolling walker `walker_x` tiles from the
    /// left wall, goal at `goal_x`.
    ///
    /// This is the fixture for asking how a *stochastic* level behaves. A walker's
    /// patrol phase is drawn from the episode RNG (§5), so two trials of the same
    /// genome are two different episodes — the only case where `trials > 1` does
    /// anything. Note fitness pays for survival frames, so merely reaching the
    /// walker at a different tick is already a different score.
    pub fn walker_runway(name: &str, width: i32, goal_x: i32, walker_x: i32) -> Level {
        runway_with(name, width, goal_x, &[], &[("walker", walker_x)])
    }

    fn runway_with(
        name: &str,
        width: i32,
        goal_x: i32,
        pits: &[(i32, i32)],
        walkers: &[(&str, i32)],
    ) -> Level {
        const HEIGHT: i32 = 8;
        const GROUND: i32 = 6;
        let mut entities: Vec<(&str, i32, i32)> = walkers
            .iter()
            .map(|(kind, x)| (*kind, *x, GROUND - 1))
            .collect();
        entities.push(("goal", goal_x, GROUND - 1));
        let rows: Vec<String> = (0..HEIGHT)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        let in_pit = pits.iter().any(|(s, l)| x >= *s && x < s + l);
                        if y >= GROUND && !in_pit {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect();
        let refs: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
        level(name, &refs, (1, GROUND - 1), &entities)
    }

    fn runway(name: &str, width: i32, goal_x: i32, pits: &[(i32, i32)]) -> Level {
        const HEIGHT: i32 = 8;
        const GROUND: i32 = 6;
        let rows: Vec<String> = (0..HEIGHT)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        let in_pit = pits.iter().any(|(s, l)| x >= *s && x < s + l);
                        if y >= GROUND && !in_pit {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect();
        let refs: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
        level(name, &refs, (1, GROUND - 1), &[("goal", goal_x, GROUND - 1)])
    }
}
