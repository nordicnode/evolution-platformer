//! **Shortcut hunter** (WP14): what did a winning run skip?
//!
//! The route graph is the game's *intended* path — the stands the oracle proved are reachable and
//! the crossings that close a hop. A champion that reaches the flag is not obliged to follow it:
//! a stomp chain over a pit, a spring that lands past two shelves, a crumble bridge surfed before
//! it falls, or a wall climb the routed path never needed all show up as the same fact — the body
//! stood on some of the stands the route names and not on others.
//!
//! This module turns that fact into the project's most communicable number: **the percentage of
//! the level's intended route a solution skipped**, plus the first stand it skipped and the stands
//! it used instead. The plan calls it the shortcut hunter; the honest reading is a *diff* between
//! two sequences, which is why it is cheap, deterministic and needs no new sim state: replay the
//! genome with the ordinary episode loop, record the stand the body is on whenever it changes, and
//! compare with the graph's canonical route from the spawn.
//!
//! It is a report, never a score. Nothing here feeds fitness: a skipped hop is evidence of a
//! technique, and the whole point of Phase E is that the main search would kill a technique for
//! being slow before anyone could see it.

use sim::{spawn_route_stand, Level};

use crate::fitness::{EpisodeRunner, EvalConfig};

/// What one run did relative to the level's intended route.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutReport {
    /// The stands the graph's route uses, from the spawn's stand to the flag's, in order.
    pub intended: Vec<usize>,
    /// The stands the body actually stood on, in the order it first reached them.
    pub visited: Vec<usize>,
    /// Intended stands the body never stood on. These are the skipped hops.
    pub skipped: Vec<usize>,
    /// The first intended stand the body skipped, in route order.
    pub first_skipped: Option<usize>,
    /// Intended stands skipped as a fraction of the route's hops, `0.0..=1.0`.
    pub skip_fraction: f32,
    /// Did the episode reach the flag? A route walked by a corpse is not a shortcut.
    pub won: bool,
    /// Frames the episode took, for context beside the skip number.
    pub frames: u32,
    /// One line in the module's voice.
    pub note: String,
}

impl ShortcutReport {
    /// The skip as a whole percentage, for a sentence.
    pub fn skip_percent(&self) -> f32 {
        self.skip_fraction * 100.0
    }
}

/// The graph's canonical route from `spawn`, as stand indices.
///
/// Walks [`GoalRoutes::next_hop`] from the spawn's own stand, following the hop count down to the
/// flag. A ride is appended when a stand has no crossing but does have a deck ride ([`GoalRoutes::ride_from`]):
/// waiting for the mover is part of the intended route even though it is not a press.
pub fn intended_route(level: &Level) -> Vec<usize> {
    let routes = level.routes();
    let Some(mut stand) = spawn_route_stand(level) else {
        return Vec::new();
    };
    let mut out = vec![stand];
    let goal = level.goals.first().copied();
    // A level cannot have more route stands than it has stands; the bound is a guard against a
    // graph that ever grows a cycle, not a claim about the levels this build ships.
    for _ in 0..routes.len() {
        if routes.hops_to_flag(stand) == Some(0) {
            break;
        }
        if let Some(edge) = routes.next_hop(stand, stand_anchor(level, stand), goal) {
            match edge.to {
                Some(next) if next != stand => {
                    out.push(next);
                    stand = next;
                    continue;
                }
                _ => break,
            }
        }
        // No crossing from here: the only intended move left is a ride.
        match routes.ride_from(stand) {
            Some(next) if next != stand => {
                out.push(next);
                stand = next;
            }
            _ => break,
        }
    }
    out
}

/// A representative x for a stand: the middle of its walkable span, in world units.
fn stand_anchor(level: &Level, stand: usize) -> f32 {
    let (x0, x1, _) = level.routes().zone_bounds(stand);
    (x0 as f32 + x1 as f32 + 1.0) * 0.5 * sim::TILE
}

/// Diff the graph's route against the stands a body visited.
///
/// Pure, so the arithmetic is a test rather than something a viewer has to infer from a picture.
/// `intended`'s first entry is the spawn stand and is never counted as skipped: a body starts
/// there by definition, and charging it for a hop it did not have to make would make every run
/// look like a shortcut.
pub fn diff(intended: &[usize], visited: &[usize]) -> (Vec<usize>, Option<usize>, f32) {
    if intended.len() <= 1 {
        return (Vec::new(), None, 0.0);
    }
    let skipped: Vec<usize> = intended
        .iter()
        .skip(1)
        .copied()
        .filter(|s| !visited.contains(s))
        .collect();
    let total = intended.len() - 1;
    let fraction = skipped.len() as f32 / total as f32;
    (skipped.clone(), skipped.first().copied(), fraction)
}

/// Replay `genome` on `level` and report the stands it used, the route it skipped, and whether it
/// actually won.
///
/// The episode loop is [`EpisodeRunner`], the same one training and champion replay use, so the
/// sequence is of the episode the champion's own fitness was measured on rather than a second
/// simulation with different cadence. `seed` should be the one in the champion's metadata.
pub fn trace(
    level: &Level,
    genome: &crate::genome::Genome,
    eval: &EvalConfig,
    seed: u64,
) -> ShortcutReport {
    let mut runner = EpisodeRunner::new(level, genome.clone(), eval, seed);
    let mut visited = Vec::new();
    let mut last: Option<usize> = None;
    while !runner.is_finished() {
        if let Some(stand) = runner.sim().route_stand() {
            if last != Some(stand) {
                if !visited.contains(&stand) {
                    visited.push(stand);
                }
                last = Some(stand);
            }
        }
        runner.step();
    }
    let result = runner.result();
    let intended = intended_route(level);
    let (skipped, first_skipped, skip_fraction) = diff(&intended, &visited);
    let won = result.won();
    let note = if intended.is_empty() {
        format!(
            "no route graph from the spawn on {}, so there is nothing to skip ({:.0}% skipped)",
            level.name,
            skip_fraction * 100.0
        )
    } else if won {
        format!(
            "reached the flag in {} frames, skipping {} of {} route hops ({:.0}%){}",
            result.frames,
            skipped.len(),
            intended.len() - 1,
            skip_fraction * 100.0,
            first_skipped
                .map(|s| format!("; the first skipped stand is {s}"))
                .unwrap_or_default()
        )
    } else {
        format!(
            "did not reach the flag ({}), so this route is a walk, not a shortcut",
            result.outcome.as_str()
        )
    };
    ShortcutReport {
        intended,
        visited,
        skipped,
        first_skipped,
        skip_fraction,
        won,
        frames: result.frames,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diff_never_charges_the_spawn_and_counts_each_skipped_hop_once() {
        // Route: spawn 0 -> 1 -> 2 -> 3. A body that went 0 -> 2 -> 3 skipped stand 1.
        let (skipped, first, fraction) = diff(&[0, 1, 2, 3], &[0, 2, 3]);
        assert_eq!(skipped, vec![1]);
        assert_eq!(first, Some(1));
        assert!((fraction - 1.0 / 3.0).abs() < 1e-6);

        // Staying on the spawn is not a skip of the spawn itself: zero route hops and zero
        // skipped, which is what a level with no graph should also report.
        assert_eq!(diff(&[4], &[4]), (Vec::new(), None, 0.0));
        assert_eq!(diff(&[], &[]), (Vec::new(), None, 0.0));

        // A route followed exactly is zero. The spawn appearing in `visited` is not needed for
        // this: the diff skips intended[0] on purpose.
        assert_eq!(diff(&[0, 1, 2], &[1, 2]), (Vec::new(), None, 0.0));
    }

    #[test]
    fn a_shipped_levels_intended_route_is_a_path_of_route_stands() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../levels/l2-first-gap.json");
        let level = Level::from_json_file(&path).expect("the shipped level loads");
        let route = intended_route(&level);
        assert!(route.len() >= 2, "a gap level has a route: {route:?}");
        for pair in route.windows(2) {
            assert_ne!(pair[0], pair[1], "a route does not stand still");
        }
        // Every stand on the path is a real stand of the graph.
        for stand in &route {
            assert!(*stand < level.routes().len());
        }
    }
}
