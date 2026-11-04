//! **Ask "what if?" about a sequence, not a single decision.**
//!
//! [`crate::mistakes`] repairs one decision: it replays the episode, tries every action from
//! each of the last states, and keeps the action that survives where the chosen one did not.
//! That is the right question for a pit or a spike, and the wrong one for a deck, a patrol or a
//! moving platform: the move that survives those is usually a *sequence* — wait here, then run,
//! then jump — whose first action is not better on its own. The one-decision repair cannot see
//! it, because every action it tries is held for one fixed horizon and the policy takes over
//! immediately afterwards.
//!
//! This module is the sequence question. It replays the policy's own losing episode, keeps a
//! window of decision blocks, and forks the world before the end of that window. From the fork it
//! searches a beam over **whole decision blocks** in the real simulator, and scores each prefix
//! by handing the body back to the policy and running the rest of the episode out. The question
//! every branch asks is therefore the only one that matters: *if the body had done this for a
//! few blocks, would the policy have got further?* The winning prefix is distilled back into a
//! genome by the output-layer step [`crate::mistakes::learn`] uses, applied to each decision the
//! branch changed; the child is an ordinary member of the next generation — evaluated, ranked
//! and kept or discarded like every other child.
//!
//! The search is deliberately bounded. `lookback` decisions are captured, `depth` of them are
//! searched, and `beam` prefixes survive each block, so one call costs at most
//! `depth × beam × 11` short policy rollouts — and a rollout from the fork only has the last
//! `depth` blocks left to run, so the whole call is a few episodes' worth of physics.
//!
//! **What it does not claim.** It is a local search around one losing episode on one seed. A
//! branch that wins here is a hypothesis for the population, not a champion; the policy may
//! still fail on another seed, and the child is only accepted by the ordinary objective in the
//! next generation. It also does not repair a win's clock — that is the mastery league's job.

use rayon::prelude::*;
use sim::physics::AgentAction;
use sim::{Level, Outcome, Sim};

use crate::fitness::{self, Decision, EpisodeRunner, EvalConfig};
use crate::genome::{ActScratch, Genome};
use crate::imitate::{clone_from_play_init, exact_clone, ImitateConfig};
use crate::mistakes::{learn, Mistake};
use crate::route_play::{Decision as DemoDecision, RoutePlay};

/// How hard one what-if call looks.
///
/// The defaults are sized for the failure this helper exists for: a body that dies at the same
/// place for several generations. A fork thirty-two decision blocks before the death is a
/// hundred and twenty-eight ticks at the shipped cadence — long enough to cover a deck's arrival
/// or a patrol's turn, not only a jump and its landing — and a beam of eight keeps a wait, both
/// directions and both jump gears alive while they are being compared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WhatIfConfig {
    /// Decision blocks kept from the losing episode. Must cover at least [`Self::depth`].
    pub lookback: usize,
    /// Decision blocks the branch may replace before the policy takes over again.
    pub depth: u32,
    /// Prefixes kept after each block.
    pub beam: usize,
}

impl Default for WhatIfConfig {
    fn default() -> Self {
        WhatIfConfig {
            // The fork is the last thirty-two blocks of the episode, so a branch has room to
            // start a wait or a run-up before the move that actually killed the body. The
            // lookback is wider than that so the demonstration of the walk to the fork stays
            // available to a caller that wants it.
            lookback: 48,
            depth: 32,
            beam: 8,
        }
    }
}

impl WhatIfConfig {
    pub fn sanitized(mut self) -> Self {
        self.lookback = self.lookback.clamp(1, 256);
        self.depth = self.depth.clamp(1, 64);
        self.beam = self.beam.clamp(1, 32);
        self.lookback = self.lookback.max(self.depth as usize);
        self
    }
}

/// What one episode did, on the same readings every branch is judged by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub outcome: Outcome,
    pub best_x: f32,
    pub frames: u32,
    /// Pixels closed from the branch's own start. Reported, not used to rank: branches start
    /// at the same fork, so `best_x` already answers "which one got further".
    pub progress_pixels: f32,
}

impl Reading {
    fn of(result: &fitness::EpisodeResult) -> Self {
        Reading {
            outcome: result.outcome,
            best_x: result.best_x,
            frames: result.frames,
            progress_pixels: result.progress_pixels,
        }
    }

    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
    }

    /// Ordering key: a win first, then the branch that got furthest, with the **quicker** branch
    /// as the tie-break. Distance leads because that is the objective ([`fitness::score`] pays
    /// progress, not survival); time is a tie-break in the same direction, because a branch that
    /// reaches the same frontier later spent the difference waiting. A wait is only worth
    /// keeping when the rollout after it gets further — which is exactly the future the branch
    /// search is asking about.
    fn key(&self) -> (u8, f32, f32) {
        match self.outcome {
            Outcome::Won => (2, -(self.frames as f32), self.best_x),
            _ => (1, self.best_x, -(self.frames as f32)),
        }
    }
}

/// One branch: the prefix the search chose, and what happened after it.
#[derive(Clone, Debug, PartialEq)]
pub struct WhatIf {
    /// The block actions the branch substituted for the policy's, from the fork.
    pub prefix: Vec<AgentAction>,
    /// The episode after the prefix, with the policy back in control.
    pub reading: Reading,
    /// The child distilled from the branch, when the teacher could move the policy to reproduce
    /// it and the child improved on the losing episode from spawn.
    pub refined: Option<Genome>,
    /// What the refined child did from spawn. `None` when nothing was learned.
    pub refined_reading: Option<Reading>,
}

/// One call's report. Returned even when no branch beats the original, because the caller's
/// telemetry should be able to say "the helper looked and found nothing" rather than guess.
#[derive(Clone, Debug, PartialEq)]
pub struct WhatIfReport {
    pub seed: u64,
    /// The generation's own episode, before the fork.
    pub original: Reading,
    /// The fork's frame on the original episode, and what killed it.
    pub fork_frame: u32,
    pub death_cause: Option<&'static str>,
    /// Branches scored, including those pruned after their first block.
    pub explored: usize,
    /// The best branch, only when it beat the original by [`Reading::key`].
    pub best: Option<WhatIf>,
    /// One line in the helper's voice.
    pub note: String,
}

/// Replay `genome`'s episode, fork it before the end, and search whole-block deviations.
///
/// Deterministic in `(level, genome, eval, seed, cfg)`: candidates are expanded in action order,
/// scored by a deterministic policy rollout and sorted stably, so a tie keeps the earlier
/// candidate and two calls name the same branch.
pub fn search(
    level: &Level,
    genome: &Genome,
    eval: &EvalConfig,
    seed: u64,
    cfg: WhatIfConfig,
) -> WhatIfReport {
    let cfg = cfg.sanitized();
    let repeat = eval.action_repeat.max(1);
    let mut runner = EpisodeRunner::new(level, genome.clone(), eval, seed);
    runner.capture_decisions_window(cfg.lookback);
    while !runner.is_finished() {
        runner.step();
    }
    let death_cause = runner.sim().death_cause().map(|cause| cause.as_str());
    let start_frame = runner
        .decisions()
        .first()
        .map(|d| d.frame)
        .unwrap_or(runner.sim().stats().frames);
    let decisions = runner.decisions().to_vec();
    let original_reading = Reading::of(&runner.result());

    if original_reading.won() {
        return WhatIfReport {
            seed,
            original: original_reading,
            fork_frame: start_frame,
            death_cause,
            explored: 0,
            best: None,
            note: "the episode already won, so there is no failure to think about".to_string(),
        };
    }
    if genome.shape.commitment {
        // A cadence head chooses how long each block holds, so the search cannot step a block
        // for a fixed `repeat` and compare prefixes on the policy's own grid. The guard is here
        // rather than in the caller so a commitment run gets an honest "not searched" report
        // instead of a branch that means something else.
        return WhatIfReport {
            seed,
            original: original_reading,
            fork_frame: start_frame,
            death_cause,
            explored: 0,
            best: None,
            note: "the policy has a commitment head; whole-block prefixes need a fixed cadence"
                .to_string(),
        };
    }
    if decisions.len() < 2 {
        return WhatIfReport {
            seed,
            original: original_reading,
            fork_frame: start_frame,
            death_cause,
            explored: 0,
            best: None,
            note: format!(
                "the episode ended after {} decision(s); there is no block to fork",
                decisions.len()
            ),
        };
    }

    // Fork **depth blocks before the end**, not at the last decision. The fatal move can be
    // several blocks before the death (a jump that is already in the air, a run-up that started
    // too late), and a fork at the final decision has no room to replace it.
    let depth = cfg.depth as usize;
    let fork = decisions.len().saturating_sub(depth);
    let fork_frame = decisions[fork].frame;
    let mut beam: Vec<Node> = vec![Node {
        sim: decisions[fork].sim.clone(),
        prefix: Vec::new(),
    }];
    let mut explored = 0usize;

    for _ in 0..depth {
        let next: Vec<(Reading, Node)> = beam
            .par_iter()
            .flat_map_iter(|node| {
                AgentAction::ALL.iter().filter_map(|&action| {
                    let mut sim = node.sim.clone();
                    for _ in 0..repeat {
                        if sim.outcome().is_finished() {
                            // A block that dies before it ends cannot be a prefix: the policy
                            // never gets the body back to show whether the branch was better.
                            return None;
                        }
                        sim.step(action);
                    }
                    let mut prefix = node.prefix.clone();
                    prefix.push(action);
                    let reading = rollout(genome, eval, seed, &sim);
                    Some((reading, Node { sim, prefix }))
                })
            })
            .collect();
        explored += next.len();
        if next.is_empty() {
            break;
        }
        beam = ranked(next, cfg.beam);
        if beam.is_empty() {
            break;
        }
    }

    let Some(best_node) = beam.into_iter().next() else {
        return WhatIfReport {
            seed,
            original: original_reading,
            fork_frame,
            death_cause,
            explored,
            best: None,
            note: "every branch died inside its own prefix; there is nothing to hand back"
                .to_string(),
        };
    };
    let best_reading = rollout(genome, eval, seed, &best_node.sim);
    if best_reading.key() <= original_reading.key() {
        return WhatIfReport {
            seed,
            original: original_reading,
            fork_frame,
            death_cause,
            explored,
            best: None,
            note: format!(
                "searched {explored} branches; the best died at x {:.1} after {} frames, no better than the episode",
                best_reading.best_x, best_reading.frames
            ),
        };
    }

    // Re-run the winning branch's **prefix** with capture so the (state, action) pairs can be
    // distilled. Only the prefix can differ from the policy; the rollout after it is already
    // the policy's own episode, so replaying it would only fill the capture window until the
    // decisions that actually changed scrolled out.
    let mut replay = EpisodeRunner::from_snapshot(
        genome.clone(),
        eval,
        seed,
        decisions[fork].sim.restart_from_snapshot(),
    );
    replay.capture_decisions_window(best_node.prefix.len() + 1);
    for action in &best_node.prefix {
        for _ in 0..repeat {
            if replay.is_finished() {
                break;
            }
            replay.step_with_override(Some(*action));
        }
    }
    let branch_decisions = replay.decisions().to_vec();

    // Distillation is a **search over children**, not one repair: the single-decision step and
    // the same step applied in branch order can disagree about which change generalises back to
    // spawn, so each candidate is replayed from spawn and the best one that beats the original
    // episode is kept. A child that does no better than its parent is a wasted population slot.
    // The gain recorded on the synthetic mistake is the branch's frontier over the episode's:
    // provenance for a repair that is never written beside the child, but it is the real number
    // rather than a placeholder that would read like a priority if it were ever surfaced.
    let gain = best_reading.best_x - original_reading.best_x;
    let mut candidates = distill(genome, &branch_decisions, gain);
    // **The better distillation is the whole route, not a weight patch.** The found branch is a
    // winning suffix from the fork; prepending the policy's own prefix makes it a winning episode
    // *from spawn*, which is a demonstration a clone can be fitted to. The output-layer step above
    // asks one row to make the branch's first action win at one state, and on `l11`'s final jump
    // that state is not separable from the approach — so the branch was found and thrown away.
    // Replaying the policy to the fork frame, substituting the branch, and cloning the resulting
    // winning trajectory asks for the behaviour the level actually needs.
    if let Some(child) = route_clone(level, genome, eval, seed, fork_frame, &best_node.prefix) {
        if !candidates
            .iter()
            .any(|c| c.weights == child.weights && c.biases == child.biases)
        {
            candidates.push(child);
        }
    }
    let mut refined = None;
    let mut refined_reading = None;
    for child in candidates {
        let reading = Reading::of(&fitness::run_episode(level, &child, eval, seed));
        if reading.key() <= original_reading.key() {
            continue;
        }
        if refined_reading.is_none_or(|best: Reading| reading.key() > best.key()) {
            refined = Some(child);
            refined_reading = Some(reading);
        }
    }

    let note = format!(
        "forked at frame {fork_frame}; searched {explored} branches; best {}-block branch {} after {} frames (x {:.1}){}",
        best_node.prefix.len(),
        best_reading.outcome.as_str(),
        best_reading.frames,
        best_reading.best_x,
        if refined.is_some() {
            " — distilled into a child"
        } else {
            " — no child improved on the episode"
        }
    );
    WhatIfReport {
        seed,
        original: original_reading,
        fork_frame,
        death_cause,
        explored,
        best: Some(WhatIf {
            prefix: best_node.prefix,
            reading: best_reading,
            refined,
            refined_reading,
        }),
        note,
    }
}

/// A beam node: the world after a prefix, and the prefix itself.
struct Node {
    sim: Sim,
    prefix: Vec<AgentAction>,
}

/// Run the policy from `sim` to the end. The snapshot is restarted exactly as every other
/// mid-level episode in the trainer is, so the reading is comparable across branches and the
/// world clock keeps counting from the fork.
fn rollout(genome: &Genome, eval: &EvalConfig, seed: u64, sim: &Sim) -> Reading {
    let mut runner =
        EpisodeRunner::from_snapshot(genome.clone(), eval, seed, sim.restart_from_snapshot());
    while !runner.is_finished() {
        runner.step();
    }
    Reading::of(&runner.result())
}

/// Keep the best `want` candidates by reading, with stable ties.
fn ranked(mut scored: Vec<(Reading, Node)>, want: usize) -> Vec<Node> {
    scored.sort_by(|a, b| {
        b.0.key()
            .partial_cmp(&a.0.key())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(want);
    scored.into_iter().map(|(_, node)| node).collect()
}

/// Turn the branch's own `(state, action)` pairs into candidate children.
///
/// The teacher is the one [`crate::mistakes`] already uses: a minimal output-layer step that
/// makes the branch's action win at a captured state. Two arrangements are tried because they
/// fail in different directions. **One child per changed decision** moves a single row, so it is
/// the most local and the most likely to generalise to nearby states; **the sequential repair**
/// applies those steps in branch order, each against the weights the previous one produced, so a
/// branch whose later blocks are only reachable after the earlier ones can still be taught as a
/// chain. The caller replays every candidate from spawn and keeps the best one.
fn distill(genome: &Genome, changed: &[Decision], gain: f32) -> Vec<Genome> {
    let mut out = Vec::new();
    let differed: Vec<&Decision> = changed
        .iter()
        .filter(|d| {
            let mut scratch = ActScratch::new();
            scratch.restore_context(&d.context);
            genome.act(&d.observation, &mut scratch) != d.action
        })
        .collect();
    if differed.is_empty() {
        return out;
    }

    for d in &differed {
        if let Some(child) = learn_one(genome, d, gain) {
            out.push(child);
        }
    }
    if let Some(child) = learn_sequence(genome, &differed, gain) {
        if !out
            .iter()
            .any(|c| c.weights == child.weights && c.biases == child.biases)
        {
            out.push(child);
        }
    }
    out
}

/// **Clone the whole winning episode**: the policy's own prefix, the branch, then the policy again.
///
/// The branch search only keeps a window of decisions, so the prefix before the window cannot be
/// read out of it; it is recovered by replaying the deterministic policy from spawn and finding the
/// decision with the fork's frame. The branch is then substituted block for block, the policy takes
/// over from its end, and the resulting trajectory is a full winning demonstration from spawn.
///
/// A clone of that trajectory can be exact (`exact_clone`), which is the point: the reason the
/// output-layer repair threw the `l11` branch away was that the last jump is not linearly separable
/// from the approach in the output layer, not that the branch was wrong. When the exact constructor
/// does not apply, the ordinary gradient clone is tried; the caller replays the child from spawn and
/// keeps it only if it beats the episode, so a bad fit costs a population slot no more than the
/// old repair did.
fn route_clone(
    level: &Level,
    genome: &Genome,
    eval: &EvalConfig,
    seed: u64,
    fork_frame: u32,
    prefix: &[AgentAction],
) -> Option<Genome> {
    // The original episode, replayed only to locate the fork by the frame it was captured at. The
    // replay is deterministic, so this is the same block the branch search forked, and its first
    // `fork` decisions are the prefix a from-spawn demonstration needs.
    let mut probe = EpisodeRunner::new(level, genome.clone(), eval, seed);
    probe.capture_decisions_window(4096);
    while !probe.is_finished() {
        probe.step();
    }
    let fork = probe
        .decisions()
        .iter()
        .position(|d| d.frame == fork_frame)?;

    // **Replay the whole episode from spawn, forcing the prefix the probe took.** The branch was
    // scored from a snapshot whose episode clock had been restarted; a clone trained on that
    // snapshot's observations is not a policy for the spawn episode (the clock is part of what the
    // sensors read). Forcing the probe's prefix reproduces the fork state exactly, forcing the
    // branch replaces the block the search chose, and letting the policy act afterwards gives the
    // real from-spawn episode — the one the child will be judged on.
    let mut runner = EpisodeRunner::new(level, genome.clone(), eval, seed);
    runner.capture_decisions_window(4096);
    let mut block = 0usize;
    while !runner.is_finished() {
        let forced = if block < fork {
            probe.decisions().get(block).map(|d| d.action)
        } else if block < fork + prefix.len() {
            Some(prefix[block - fork])
        } else {
            None
        };
        // `step` is one **tick**, not one block: the override is consumed only where the policy's
        // commitment has run out, and the return value says whether this tick was such a boundary.
        // Passing the block's action on the intermediate ticks is harmless (it is ignored), so the
        // counter moves exactly once per decision and lines up with the probe's decisions.
        if runner.step_with_override(forced) {
            block += 1;
        }
        if block > 4096 {
            break;
        }
    }
    let hops = runner.sim().stats().route_hops_closed;
    eprintln!(
        "route_clone: fork {fork} at frame {fork_frame}, probe {} decisions, replay {block} blocks",
        probe.decisions().len()
    );
    let decisions: Vec<DemoDecision> = runner
        .decisions()
        .iter()
        .map(|d| DemoDecision {
            observation: d
                .observation
                .as_slice()
                .try_into()
                .expect("a captured observation is N_INPUTS wide"),
            action: d.action,
        })
        .collect();
    let result = runner.result();
    eprintln!(
        "route_clone: from spawn {:?} at x {:.1} after {} frames",
        result.outcome, result.best_x, result.frames
    );
    if result.outcome != Outcome::Won {
        return None;
    }
    let play = RoutePlay {
        decisions,
        segments: vec![0],
        outcome: Outcome::Won,
        frames: result.frames,
        seed,
        hops_closed: hops,
        best_block: 0,
        best_stand: None,
        route_hops: None,
        note: format!(
            "full route from a {}-block what-if branch at frame {fork_frame}",
            prefix.len()
        ),
        trace: Vec::new(),
    };
    if let Some(exact) = exact_clone(&play, &genome.shape) {
        eprintln!("route_clone: exact over {} decisions", play.decisions.len());
        return Some(exact);
    }
    eprintln!(
        "route_clone: no exact; gradient over {} decisions",
        play.decisions.len()
    );
    // **Continue from the parent, not from random weights.** The parent already reproduces the
    // prefix; a from-scratch fit spends its budget re-learning a route the population has, and on
    // `l11`'s 252-decision win it never reproduced the branch. Starting from the parent makes the
    // prefix a fixed point and spends the epochs on the block the what-if actually changed.
    let cfg = ImitateConfig {
        epochs: 600,
        lr: 0.02,
        prefer_agreement: true,
        ..ImitateConfig::default()
    };
    clone_from_play_init(&play, genome.shape.clone(), &cfg, Some(genome))
        .ok()
        .map(|report| report.genome)
}

/// The smallest output-layer step that makes `decision.action` win at the captured state.
fn learn_one(genome: &Genome, decision: &Decision, gain: f32) -> Option<Genome> {
    let mut scratch = ActScratch::new();
    scratch.restore_context(&decision.context);
    let chosen = genome.act(&decision.observation, &mut scratch);
    if chosen == decision.action {
        return None;
    }
    let mistake = Mistake {
        frame: decision.frame,
        observation: decision.observation.clone(),
        context: decision.context.clone(),
        chosen,
        better: decision.action,
        gain,
    };
    learn(genome, &mistake).map(|mut child| {
        child.fitness = 0.0;
        child
    })
}

/// Apply [`learn_one`] in branch order, each step asked against the weights the previous one
/// produced. The guarantee is local to each captured state; the spawn replay is what accepts it.
fn learn_sequence(genome: &Genome, differed: &[&Decision], gain: f32) -> Option<Genome> {
    let mut child = genome.clone();
    let mut changed = 0usize;
    for decision in differed {
        if let Some(next) = learn_one(&child, decision, gain) {
            child = next;
            changed += 1;
        }
    }
    (changed > 0).then_some(child)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::genome::default_shape;

    fn eval() -> EvalConfig {
        EvalConfig {
            action_repeat: 4,
            cut_stuck_frames: 0,
        }
    }

    #[test]
    fn a_sequence_that_survives_a_pit_is_found_and_repaired() {
        // The all-zero policy runs right into a two-tile pit at column 12. The fix is a jump,
        // and the branch search is allowed to place it anywhere in the last eight blocks.
        let level = fixture::gap_runway("whatif-pit", 40, 38, &[(12, 2)]);
        // **A random parent, not the all-zero net.** Zeros are a saddle for the output-layer
        // step: every hidden activation is zero, so the error cannot reach the first layer and
        // the repair can only learn a constant action. A random parent is the ordinary case the
        // helper runs on, and the one where a distilled child can change a decision.
        let mut rng = sim::rng::rng_from_seed(11);
        let genome = Genome::random(0, default_shape(), &mut rng);
        let report = search(&level, &genome, &eval(), 7, WhatIfConfig::default());

        assert!(!report.original.won(), "the fixture must be lost first");
        assert!(
            report.explored > 0,
            "the search has to try at least one branch: {}",
            report.note
        );
        let best = report.best.expect("a jump over a two-tile pit exists");
        assert!(
            !best.prefix.is_empty(),
            "the branch must name the block it changed"
        );
        assert!(
            best.reading.key() > report.original.key(),
            "the branch has to beat the episode: {:?} against {:?}",
            best.reading,
            report.original
        );
        let child = best.refined.expect("a child should learn the branch");
        let reading = best.refined_reading.expect("the child was re-scored");
        assert!(
            reading.key() > report.original.key(),
            "the child must improve the episode from spawn: {:?} against {:?}",
            reading,
            report.original
        );
        // The fixture's best branch is an improvement, not a win, so this asserts the repair's
        // contribution rather than claiming the branch is a solution. The full-route clone is only
        // offered when the branch *does* win (see `route_clone`), which is the case this helper
        // exists for; the branch here times out short of the flag.
        assert_ne!(
            child, genome,
            "the distilled child must be a different genome, not a relabelled parent"
        );
    }

    /// **A winning branch becomes a winning child, not a weight patch that dies.** The branch
    /// search's answers are suffixes; the full-route clone is the only distillation that turns one
    /// into a genome that can win from spawn. This fixture is short enough that a branch can reach
    /// the flag, which is the case `l11`'s final jump is in.
    #[test]
    fn a_winning_branch_is_distilled_into_a_child_that_wins_from_spawn() {
        let level = fixture::gap_runway("whatif-win", 18, 14, &[(8, 2)]);
        let genome = Genome::zeros(0, default_shape());
        let report = search(&level, &genome, &eval(), 7, WhatIfConfig::default());
        let best = report.best.expect("a jump branch exists");
        assert!(
            best.reading.won(),
            "the fixture must give the search a winning branch: {:?}",
            report.note
        );
        let reading = best
            .refined_reading
            .expect("the full route is distilled into a child");
        assert!(
            reading.won(),
            "the child must win from spawn, not merely get further: {:?}",
            reading
        );
    }

    #[test]
    fn a_winning_episode_returns_without_searching() {
        let level = fixture::gap_runway("whatif-flat", 12, 10, &[]);
        let genome = Genome::zeros(0, default_shape());
        let report = search(&level, &genome, &eval(), 3, WhatIfConfig::default());
        assert!(report.original.won());
        assert_eq!(report.explored, 0);
        assert!(report.best.is_none());
    }

    #[test]
    fn the_same_call_names_the_same_branch() {
        let level = fixture::gap_runway("whatif-deterministic", 40, 38, &[(12, 2)]);
        let genome = Genome::zeros(0, default_shape());
        let a = search(&level, &genome, &eval(), 7, WhatIfConfig::default());
        let b = search(&level, &genome, &eval(), 7, WhatIfConfig::default());
        assert_eq!(a, b);
    }
}
