//! **TAS polish (WP12): a 1-tick optimiser over a champion's own run.**
//!
//! A policy decides once per [`EvalConfig::action_repeat`] ticks and holds its arm in between.
//! Tricks that need 1–3 tick precision — a take-off on the last sub-tile offset, a jump cut one
//! tick earlier, a buffered press landing inside the coyote window — are therefore inexpressible
//! to the population however long it evolves. This module takes the **action sequence** of a
//! validated champion, expands it to one action per tick, and runs a deterministic local search
//! over per-tick substitutions and press shifts.
//!
//! It is deliberately not a genome optimiser. The result is a *route artifact*: a list of tick
//! actions, the seed it ran under, and the frames it took. It is labelled machine-polished so the
//! distinction between "evolved" and "polished" stays honest, and it is never written as a
//! champion. A polished route belongs in the same drawer as a plan route.
//!
//! # The search
//!
//! Every candidate is a full episode from spawn, scored purely on **frames to win**; a candidate
//! that dies, times out or gets stuck is rejected. Mutations are local:
//!
//! * substitute one tick's action with another of the eleven;
//! * shift one press (a transition into a jump action) one, two or three ticks earlier or later;
//! * copy a press window from a nearby successful position.
//!
//! The first improvement found is accepted (first-improvement hill climbing), and the stream is
//! seeded so `(level, route, config)` determines the artifact. The search stops early when a pass
//! over the jump boundaries finds nothing.
//!
//! # What it can and cannot do
//!
//! It cannot create a route that does not already win: the champion's sequence is the starting
//! point, and a mutation is accepted only if the full episode still wins. It therefore shaves
//! frames from a working line rather than discovering a new one. That is exactly the plan's
//! stated trade-off — the league finds lines, TAS makes them precise.

use serde::{Deserialize, Serialize};
use sim::hash::StateHasher;
use sim::physics::AgentAction;
use sim::{Level, Outcome, Sim};

use crate::fitness::EvalConfig;
use crate::genome::{ActScratch, Genome};

/// Format tag for a route artifact. This is not a champion and must never be loaded as one.
pub const TAS_FORMAT: &str = "evo-platformer-tas-route";
pub const TAS_VERSION: u32 = 1;

/// How hard to polish.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TasConfig {
    /// Full candidate episodes to spend.
    pub iterations: usize,
    /// Seed for the mutation stream. Deterministic, like everything else.
    pub seed: u64,
    /// Ticks either side of a press a shift may move it.
    pub shift: u32,
    /// Stop after this many consecutive passes with no improvement.
    pub patience: usize,
}

impl Default for TasConfig {
    fn default() -> Self {
        TasConfig {
            iterations: 2_000,
            seed: 0x7A5_0F11_5EED,
            shift: 3,
            patience: 2,
        }
    }
}

/// A tick-resolution route: the artifact, not a genome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TasRoute {
    pub format: String,
    pub version: u32,
    /// Human label. Always [`TasKind::MachinePolished`].
    pub kind: TasKind,
    pub level_name: String,
    pub level_hash: String,
    /// The champion this route started from, when it came from one.
    pub source_genome_id: Option<u32>,
    /// Episode seed. A route is only a claim under the seed it was run on.
    pub seed: u64,
    /// One action per physics tick, from the first tick to the winning tick inclusive.
    pub actions: Vec<AgentAction>,
    pub frames: u32,
    /// State hash after the last action, so a replay can prove it walked the same line.
    pub final_state_hash: u64,
    /// One line for a report.
    pub note: String,
}

/// Why this route exists. The distinction the plan insists on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TasKind {
    /// Produced by this optimiser from a champion's action sequence.
    MachinePolished,
    /// A route that came from the planner or a replay and was not polished.
    Unpolished,
    /// A trick-hunt trajectory (WP14): a fragment found by searching a non-goal objective. It is
    /// not a win and never a champion, and its value is the technique it demonstrates.
    Discovered,
}

impl TasRoute {
    pub fn won(&self) -> bool {
        self.frames > 0 && self.frames as usize == self.actions.len()
    }

    pub fn to_json_pretty(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("cannot serialize TAS route: {e}"))
    }

    pub fn from_json_str(src: &str) -> Result<TasRoute, String> {
        let route: TasRoute =
            serde_json::from_str(src).map_err(|e| format!("invalid TAS route JSON: {e}"))?;
        if route.format != TAS_FORMAT || route.version != TAS_VERSION {
            return Err(format!(
                "not a TAS route this build reads (format {:?} version {})",
                route.format, route.version
            ));
        }
        Ok(route)
    }
}

/// The outcome of one full-episode replay.
#[derive(Clone, Debug, PartialEq)]
pub struct TasPlay {
    pub outcome: Outcome,
    pub frames: u32,
    pub final_state_hash: u64,
    pub death_pos: Option<(f32, f32)>,
}

impl TasPlay {
    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
    }
}

/// Play a tick-action sequence from spawn.
///
/// An action past the episode's natural end is ignored: the sim is finished and `step` is a no-op,
/// so a candidate cannot be credited for actions after a win or a death.
pub fn play(level: &Level, seed: u64, actions: &[AgentAction]) -> TasPlay {
    let mut sim = Sim::new(level, seed);
    for action in actions {
        if sim.outcome().is_finished() {
            break;
        }
        sim.step(*action);
    }
    TasPlay {
        outcome: sim.outcome(),
        frames: sim.stats().frames,
        final_state_hash: sim.state_hash(),
        death_pos: sim.stats().death_pos,
    }
}

/// Record an arbitrary tick-action trajectory as a route artifact.
///
/// This is what the trick hunter (WP14) writes: a trajectory that maximises something the route
/// oracle does not value, replayed from spawn so its frames and final state hash are measured
/// rather than asserted. A non-winning fragment is still recorded — that is the point of a
/// discovery — while [`TasRoute::won`] keeps the distinction available to every reader.
pub fn route_from_actions(
    level: &Level,
    seed: u64,
    actions: &[AgentAction],
    note: String,
) -> TasRoute {
    let played = play(level, seed, actions);
    TasRoute {
        format: TAS_FORMAT.into(),
        version: TAS_VERSION,
        kind: TasKind::Discovered,
        level_name: level.name.clone(),
        level_hash: level.content_hash_string(),
        source_genome_id: None,
        seed,
        actions: actions.to_vec(),
        frames: played.frames,
        final_state_hash: played.final_state_hash,
        note,
    }
}

/// Record a champion's own episode as a tick-action sequence.
///
/// The sequence is exactly the actions the policy held, one per tick, so replaying it under the
/// same seed reproduces the champion's episode bit for bit. If the episode did not win, the route
/// is returned anyway with `frames == 0` and a note saying so; polish will refuse it.
pub fn route_from_genome(level: &Level, genome: &Genome, cfg: &EvalConfig, seed: u64) -> TasRoute {
    let repeat = cfg.action_repeat.max(1);
    let mut sim = Sim::new(level, seed);
    let mut scratch = ActScratch::new();
    let mut actions: Vec<AgentAction> = Vec::new();
    let mut held = AgentAction::Walk;
    let mut trace = StateHasher::new();
    while !sim.outcome().is_finished() {
        let frames = sim.stats().frames;
        if frames % repeat == 0 {
            held = genome.act(&sim.observation(), &mut scratch);
            trace.u32(frames);
            trace.u8(held.index() as u8);
        }
        sim.step(held);
        actions.push(held);
    }
    trace.u64(sim.state_hash());
    let won = sim.outcome() == Outcome::Won;
    TasRoute {
        format: TAS_FORMAT.into(),
        version: TAS_VERSION,
        kind: TasKind::Unpolished,
        level_name: level.name.clone(),
        level_hash: level.content_hash_string(),
        source_genome_id: Some(genome.id),
        seed,
        actions: if won { actions } else { Vec::new() },
        frames: if won { sim.stats().frames } else { 0 },
        final_state_hash: sim.state_hash(),
        note: if won {
            format!(
                "champion {} won in {} frames; {} tick actions recorded",
                genome.id,
                sim.stats().frames,
                sim.stats().frames
            )
        } else {
            format!(
                "champion {} did not win ({} at {} frames), so there is nothing to polish",
                genome.id,
                sim.outcome().as_str(),
                sim.stats().frames
            )
        },
    }
}

/// What polish did.
#[derive(Clone, Debug)]
pub struct TasReport {
    pub route: TasRoute,
    /// Frames the input route took, or 0 when it did not win.
    pub before_frames: u32,
    /// Candidate episodes spent.
    pub evaluated: usize,
    /// Mutations accepted.
    pub accepted: usize,
    /// True when the input route did not win and was returned unchanged.
    pub refused: bool,
}

impl TasReport {
    pub fn line(&self) -> String {
        if self.refused {
            return self.route.note.clone();
        }
        format!(
            "tas: {} frames -> {} frames ({} saved, {:.1}% quicker), {} candidates, {} accepted",
            self.before_frames,
            self.route.frames,
            self.before_frames.saturating_sub(self.route.frames),
            100.0 * self.before_frames.saturating_sub(self.route.frames) as f32
                / self.before_frames.max(1) as f32,
            self.evaluated,
            self.accepted
        )
    }
}

/// Polish a winning tick-action route.
///
/// The returned route is labelled [`TasKind::MachinePolished`] even when no mutation was accepted:
/// it has been *through* the optimiser, which is what the label means. Its `note` says how many
/// frames were saved. A non-winning input is refused rather than "improved" into a win by luck,
/// because that would make the artifact a search result wearing the champion's name.
pub fn polish(level: &Level, route: &TasRoute, cfg: TasConfig) -> TasReport {
    if !route.won() || play(level, route.seed, &route.actions).outcome != Outcome::Won {
        let mut refused = route.clone();
        refused.note = format!(
            "tas refused: the input route does not win ({} actions, {} frames)",
            route.actions.len(),
            route.frames
        );
        return TasReport {
            route: refused,
            before_frames: 0,
            evaluated: 0,
            accepted: 0,
            refused: true,
        };
    }
    let before_frames = route.frames;
    let mut best = route.clone();
    best.kind = TasKind::MachinePolished;
    best.actions.truncate(route.frames as usize);
    let mut best_play = play(level, route.seed, &best.actions);
    // Truncation is exact for a winning route; assert it rather than trusting the input.
    if !best_play.won() {
        let mut refused = route.clone();
        refused.note =
            "tas refused: the route's recorded frame count does not replay to a win".into();
        return TasReport {
            route: refused,
            before_frames,
            evaluated: 1,
            accepted: 0,
            refused: true,
        };
    }

    let mut rng = sim::rng::rng_from_seed(cfg.seed);
    let boundaries = press_boundaries(&best.actions);
    let mut evaluated = 1usize;
    let mut accepted = 0usize;
    let mut stale = 0usize;
    let mut round = 0usize;

    while evaluated < cfg.iterations && stale < cfg.patience.max(1) {
        let mut improved = false;
        let targets = if boundaries.is_empty() {
            (0..best.actions.len()).collect::<Vec<_>>()
        } else {
            boundaries.clone()
        };
        for &at in &targets {
            if evaluated >= cfg.iterations {
                break;
            }
            if let Some((candidate, play)) = try_substitutions(
                level,
                route.seed,
                &best.actions,
                at,
                &mut rng,
                &mut evaluated,
                cfg.iterations,
            ) {
                if play.won() && play.frames < best_play.frames {
                    best.actions = candidate;
                    best_play = play;
                    best.frames = best_play.frames;
                    best.final_state_hash = best_play.final_state_hash;
                    accepted += 1;
                    improved = true;
                    stale = 0;
                }
            }
            if evaluated >= cfg.iterations {
                break;
            }
            if let Some((candidate, play)) = try_shift(
                level,
                route.seed,
                &best.actions,
                at,
                cfg.shift,
                &mut evaluated,
                cfg.iterations,
            ) {
                if play.won() && play.frames < best_play.frames {
                    best.actions = candidate;
                    best_play = play;
                    best.frames = best_play.frames;
                    best.final_state_hash = best_play.final_state_hash;
                    accepted += 1;
                    improved = true;
                    stale = 0;
                }
            }
        }
        round += 1;
        if !improved {
            stale += 1;
            // A full pass with no improvement is a local optimum at this move set. One more pass
            // after the boundary list is refreshed is enough; the patience knob is the cap.
            if stale < cfg.patience.max(1) {
                continue;
            }
        }
        if round > 64 {
            break;
        }
    }

    best.note = format!(
        "machine-polished from {} frames: {} frames ({} saved), {} candidates, {} accepted",
        before_frames,
        best_play.frames,
        before_frames.saturating_sub(best_play.frames),
        evaluated,
        accepted
    );
    TasReport {
        route: best,
        before_frames,
        evaluated,
        accepted,
        refused: false,
    }
}

/// Indices where the action changes into a jump at a block boundary: the presses worth shifting.
fn press_boundaries(actions: &[AgentAction]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut previous = None;
    for (i, action) in actions.iter().enumerate() {
        if action.wants_jump() && previous != Some(*action) {
            out.push(i);
        }
        previous = Some(*action);
    }
    // A route with no presses still has a body: search the launch and landing neighbourhoods.
    if out.is_empty() {
        let stride = actions.len().max(1) / 16;
        out.extend((0..actions.len()).step_by(stride.max(1)));
    }
    out
}

fn try_substitutions(
    level: &Level,
    seed: u64,
    actions: &[AgentAction],
    at: usize,
    rng: &mut sim::rng::Rng,
    evaluated: &mut usize,
    cap: usize,
) -> Option<(Vec<AgentAction>, TasPlay)> {
    for _ in 0..AgentAction::COUNT {
        if *evaluated >= cap {
            return None;
        }
        let action = AgentAction::ALL
            [(sim::rng::next_f32(rng) * AgentAction::COUNT as f32) as usize % AgentAction::COUNT];
        if action == actions[at] {
            continue;
        }
        let mut candidate = actions.to_vec();
        candidate[at] = action;
        *evaluated += 1;
        let play = play(level, seed, &candidate);
        if play.won() {
            candidate.truncate(play.frames as usize);
            return Some((candidate, play));
        }
    }
    None
}

fn try_shift(
    level: &Level,
    seed: u64,
    actions: &[AgentAction],
    at: usize,
    shift: u32,
    evaluated: &mut usize,
    cap: usize,
) -> Option<(Vec<AgentAction>, TasPlay)> {
    for delta in [-1i32, 1, -2, 2, -3, 3] {
        if delta.unsigned_abs() > shift {
            continue;
        }
        let to = at as i32 + delta;
        if to < 0 || to as usize >= actions.len() || *evaluated >= cap {
            continue;
        }
        let mut candidate = actions.to_vec();
        candidate.swap(at, to as usize);
        *evaluated += 1;
        let play = play(level, seed, &candidate);
        if play.won() {
            candidate.truncate(play.frames as usize);
            return Some((candidate, play));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::default_shape;

    fn flat() -> Level {
        crate::fixture::flat_runway("tas-flat", 24, 22)
    }

    #[test]
    fn a_champion_route_replays_and_polish_never_makes_it_slower() {
        let level = flat();
        let genome = Genome::zeros(7, default_shape());
        let cfg = EvalConfig::default();
        let route = route_from_genome(&level, &genome, &cfg, 3);
        assert!(route.won(), "{}", route.note);
        let report = polish(
            &level,
            &route,
            TasConfig {
                iterations: 200,
                ..TasConfig::default()
            },
        );
        assert!(!report.refused, "{}", report.route.note);
        assert!(
            report.route.frames <= report.before_frames,
            "polish made the route slower: {} -> {}",
            report.before_frames,
            report.route.frames
        );
        let replay = play(&level, route.seed, &report.route.actions);
        assert!(replay.won(), "a polished route must replay to the flag");
        assert_eq!(replay.frames, report.route.frames);
        assert_eq!(replay.final_state_hash, report.route.final_state_hash);
    }

    #[test]
    fn a_non_winning_route_is_refused_rather_than_polished_into_a_win() {
        let level = crate::fixture::gap_runway("tas-gap", 24, 22, &[(10, 4)]);
        let genome = Genome::zeros(9, default_shape());
        let route = route_from_genome(&level, &genome, &EvalConfig::default(), 3);
        assert!(!route.won(), "the zero genome runs into the pit");
        let report = polish(&level, &route, TasConfig::default());
        assert!(report.refused);
        assert_eq!(report.accepted, 0);
    }

    #[test]
    fn the_artifact_round_trips_and_carries_its_label() {
        let level = flat();
        let route = route_from_genome(
            &level,
            &Genome::zeros(1, default_shape()),
            &EvalConfig::default(),
            5,
        );
        let json = route.to_json_pretty().expect("serializes");
        let back = TasRoute::from_json_str(&json).expect("parses");
        assert_eq!(back, route);
        assert_eq!(back.kind, TasKind::Unpolished);
    }
}
