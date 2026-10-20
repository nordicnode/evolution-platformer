//! **Learn from a mistake, not from the average** (Item 14).
//!
//! The GA's objective is a whole-episode number: distance closed on the flag, a bonus for winning,
//! a bonus for winning quickly. That is a fine thing to *select* on and a useless thing to *learn*
//! from. A policy that falls into the same pit two hundred times and a policy that falls into it
//! once and wastes four hundred frames on the way both look like "did not get far", and nothing in
//! the score says which of its decisions was the bad one. The population is therefore reduced to
//! hoping that a random mutation somewhere upstream of the failure improves the outcome — which is
//! exactly the behaviour that makes a hard level slow to learn.
//!
//! This module adds the missing half: a **one-sample repair**. Given a genome and an episode it
//! failed, it
//!
//! 1. replays the episode and keeps the last [`crate::fitness::CAPTURED_DECISIONS`] decisions, each
//!    with the world and the network's recurrent state at that moment,
//! 2. asks the world what each of the **eleven** actions would have done from those states, by
//!    stepping a restarted snapshot for [`REPAIR_HORIZON`] ticks,
//! 3. picks the earliest of the states where an alternative **survives** and the chosen action
//!    does not, and calls that the mistake (a progress-only difference between two deaths is
//!    the arc of the fall, not a mistake — the measured arm that accepted it went backwards), and
//! 4. moves the output-layer row for the better action toward the hidden activation it saw, and
//!    the row for the chosen action away from it, by the smallest step that actually flips the
//!    decision at that state.
//!
//! The result is a child that **provably does the better thing at the captured decision**, and
//! nothing else about it is claimed: the next generation evaluates it on a full episode like any
//! other genome, so the search keeps the repair only when it survives the ordinary objective. No
//! fitness term is touched, no champion is rewritten, and the recorded episode is unchanged.
//!
//! **What this is not.** It does not look ahead past one decision with one action held, so a
//! mistake whose fix needs a *sequence* (jump here, then brake there) is not found by it. It only
//! fires on episodes that failed to win, so it repairs losses and does not optimise the clock of a
//! win — that is the mastery league's job. And on a level with moving hazards, the captured state
//! is one phase of one world: a repair to it is a repair to that phase, which is why the child is
//! judged on fresh episodes rather than trusted.

use sim::physics::AgentAction;
use sim::{Level, Outcome, Sim};

use crate::fitness::{Decision, EpisodeRunner, EvalConfig};
use crate::genome::{ActScratch, Genome, GENE_CLAMP};
use crate::polish::output_slots;

/// Ticks each alternative action is held for when judging a decision.
///
/// At the shipped cadence of four ticks this is eight decisions — a jump arc and the landing after
/// it — which is long enough for "would jumping here have cleared the pit?" to have an answer and
/// short enough that the answer is about *this* decision rather than about a different route.
pub const REPAIR_HORIZON: u32 = 32;

/// The smallest advantage a caller should treat as meaningful, in the units [`advantage`] returns.
///
/// The judging rule is survival-only (see [`advantage`]), so every mistake carries at least
/// `1.0e5`; the floor exists for readers and tests, and to name the idea that a repair should be
/// aimed at a difference big enough to be about the decision rather than about rounding.
pub const MIN_GAIN: f32 = 1.0;

/// The first step tried by [`learn`], in gene units, and the factor each retry grows it by.
///
/// A step that is too small leaves the argmax alone and changes nothing; the point of the ladder is
/// to find the *smallest* step that flips the decision, so the repair is as local as the arithmetic
/// allows. The clamp in [`nudge`] bounds the top of the ladder, and [`LEARN_TRIES`] bounds how many
/// times it is walked.
const FIRST_STEP: f32 = 0.02;
const STEP_GROWTH: f32 = 1.6;
const LEARN_TRIES: usize = 30;

/// One decision that was worse than an available alternative.
#[derive(Clone, Debug, PartialEq)]
pub struct Mistake {
    /// The frame the decision was made on, for a report.
    pub frame: u32,
    /// The observation the policy saw, exactly as `EpisodeRunner` built it.
    pub observation: Vec<f32>,
    /// The recurrent state the policy carried into that decision.
    pub context: Vec<f32>,
    /// What it chose.
    pub chosen: AgentAction,
    /// What would have done better from the same state.
    pub better: AgentAction,
    /// How much better, in the units [`advantage`] uses: a survival advantage of at least
    /// `1.0e5`, plus the progress the surviving rollout made.
    pub gain: f32,
}

/// What one action did over the horizon, from one captured state.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Roll {
    won: bool,
    alive: bool,
    progress: f32,
}

/// Hold `action` for `horizon` ticks from `snapshot` and read what happened.
///
/// The snapshot is restarted rather than stepped: [`Sim::restart_from_snapshot`] is what the archive
/// already uses to begin an episode mid-level, and it resets the outcome, the death cause and the
/// episode accounting while keeping the world. `progress_pixels` is therefore measured **from the
/// snapshot**, which is the only reading that answers "did this action get further from here".
fn roll(snapshot: &Sim, action: AgentAction, horizon: u32) -> Roll {
    let mut sim = snapshot.restart_from_snapshot();
    for _ in 0..horizon {
        if sim.outcome().is_finished() {
            break;
        }
        sim.step(action);
    }
    let outcome = sim.outcome();
    Roll {
        won: outcome == Outcome::Won,
        alive: !outcome.is_finished(),
        progress: sim.stats().progress_pixels,
    }
}

/// How much better `alternate` is than `chosen`.
///
/// **Only survival counts, and the measurement said so.** The first version also accepted a plain
/// progress gain — "this sprint gained four pixels before the same death" — and the paired arm on
/// the custom level came back *slower* than the plain search (5 of 8 seeds solved either way, the
/// median first win 131 generations with repairs against 101 without). A thirty-two-tick rollout
/// that says one dying action is four pixels better than another dying action is describing the
/// arc of the fall, not a mistake: the nudge that follows teaches the policy to prefer an action at
/// a state where nothing could have saved it. So the bar is now "you died and another action lived",
/// which is a mistake in the plain sense of the word, and the earliest such decision in the window
/// is taken rather than the last — the last one is often already in the air.
fn advantage(chosen: Roll, alternate: Roll) -> f32 {
    if alternate.won && !chosen.won {
        1.0e6 + alternate.progress
    } else if alternate.alive && !chosen.alive {
        1.0e5 + alternate.progress
    } else {
        0.0
    }
}

/// Judge one captured decision: the best alternative, if any beats what was chosen.
fn judge(d: &Decision) -> Option<Mistake> {
    let chosen = roll(&d.sim, d.action, REPAIR_HORIZON);
    let mut best: Option<(AgentAction, Roll, f32)> = None;
    for action in AgentAction::ALL {
        if action == d.action {
            continue;
        }
        let alternate = roll(&d.sim, action, REPAIR_HORIZON);
        let gain = advantage(chosen, alternate);
        if gain <= 0.0 {
            continue;
        }
        // Strictly greater, over actions in their own order: a tie goes to the lowest action
        // index, so the same episode always names the same mistake whatever the machine.
        if best.is_none_or(|(_, _, held)| gain > held) {
            best = Some((action, alternate, gain));
        }
    }
    let (better, _, gain) = best?;
    Some(Mistake {
        frame: d.frame,
        observation: d.observation.clone(),
        context: d.context.clone(),
        chosen: d.action,
        better,
        gain,
    })
}

/// **Find up to `want` mistakes** in one episode of `genome`, worst advantage first.
///
/// One replay, one capture window, eleven rollouts per captured decision. An episode that won has
/// no mistake to repair by this module's definition and returns nothing — the clock is the mastery
/// league's problem, not this one's. `want == 0` returns before the replay, so a run that has the
/// mechanism switched off pays nothing.
pub fn find_mistakes(
    level: &Level,
    genome: &Genome,
    cfg: &EvalConfig,
    seed: u64,
    want: usize,
) -> Vec<Mistake> {
    if want == 0 {
        return Vec::new();
    }
    let mut runner = EpisodeRunner::new(level, genome.clone(), cfg, seed);
    runner.capture_decisions(true);
    while !runner.is_finished() {
        runner.step();
    }
    if runner.sim().outcome() == Outcome::Won {
        return Vec::new();
    }
    let mut found: Vec<Mistake> = runner.decisions().iter().filter_map(judge).collect();
    // **Earliest first, then the largest advantage.** The window is the last eight decisions, and if
    // five of them all survive under the same jump, the *first* of the five is the one where the
    // decision still mattered; the later ones are the same jump pressed from further along the arc.
    // Ties then go to the bigger advantage, and the frame keeps sibling children deterministic.
    found.sort_by(|a, b| {
        a.frame
            .cmp(&b.frame)
            .then(b.gain.total_cmp(&a.gain))
            .then(a.better.index().cmp(&b.better.index()))
    });
    found.truncate(want);
    found
}

/// The single worst mistake, for callers that only want one.
pub fn find_mistake(
    level: &Level,
    genome: &Genome,
    cfg: &EvalConfig,
    seed: u64,
) -> Option<Mistake> {
    find_mistakes(level, genome, cfg, seed, 1).pop()
}

/// Move the output layer so that `mistake.better` wins at `mistake.observation`.
///
/// Returns the child whose decision at that exact state is the better action, and `None` when no
/// step up to the ladder's top can get there (a saturated output, or an action whose row cannot
/// outvote the chosen one at this activation). The child is otherwise a byte-for-byte copy of its
/// parent, and its id and fitness are the caller's to set.
pub fn learn(genome: &Genome, mistake: &Mistake) -> Option<Genome> {
    let fan_in = genome.shape.last_fan_in();
    let (_, fan_out) = genome.shape.layer(genome.shape.n_layers() - 1);
    let chosen = mistake.chosen.index();
    let better = mistake.better.index();
    if chosen >= fan_out || better >= fan_out {
        return None;
    }

    // The activation vector the last layer saw. Recovered from the **captured** context rather
    // than a fresh one, so a recurrent genome is asked the question it was actually asked.
    let mut scratch = ActScratch::new();
    scratch.restore_context(&mistake.context);
    genome.forward(&mistake.observation, &mut scratch);
    let hidden = genome.last_layer_input(&scratch).to_vec();
    if hidden.len() != fan_in {
        return None;
    }
    let slots = output_slots(genome);

    let mut step = FIRST_STEP;
    for _ in 0..LEARN_TRIES {
        let mut child = genome.clone();
        nudge(&mut child, &slots, fan_in, &hidden, chosen, better, step);
        let mut check = ActScratch::new();
        check.restore_context(&mistake.context);
        if child.act(&mistake.observation, &mut check) == mistake.better {
            child.fitness = 0.0;
            return Some(child);
        }
        step *= STEP_GROWTH;
    }
    None
}

/// One perceptron-style update on the last layer: raise `better`'s row by `step × hidden`, lower
/// `chosen`'s by the same, and move both biases with them.
///
/// The biases are moved too because the output layer is `tanh(W·h + b)`: a row change scales with
/// the activation it is applied to, so on a state whose hidden vector is near zero the bias is the
/// only part of the update that can move the decision at all.
fn nudge(
    child: &mut Genome,
    slots: &[usize],
    fan_in: usize,
    hidden: &[f32],
    chosen: usize,
    better: usize,
    step: f32,
) {
    for k in 0..fan_in {
        let h = hidden[k];
        if h == 0.0 {
            continue;
        }
        let raise = slots[better * fan_in + k];
        child.weights[raise] = (child.weights[raise] + step * h).clamp(-GENE_CLAMP, GENE_CLAMP);
        let lower = slots[chosen * fan_in + k];
        child.weights[lower] = (child.weights[lower] - step * h).clamp(-GENE_CLAMP, GENE_CLAMP);
    }
    // `output_slots` is `fan_out × fan_in` weights followed by `fan_out` biases, so the bias half
    // starts at `fan_out × fan_in` in the slot list.
    let fan_out = slots.len() / (fan_in + 1);
    let bias_base = fan_in * fan_out;
    // Bias slots are **global gene indices** — `output_slots` numbers them after every weight —
    // so they are rebased onto the bias vector before they can index it.
    let n_weights = child.weights.len();
    let raise = slots[bias_base + better] - n_weights;
    child.biases[raise] = (child.biases[raise] + step).clamp(-GENE_CLAMP, GENE_CLAMP);
    let lower = slots[bias_base + chosen] - n_weights;
    child.biases[lower] = (child.biases[lower] - step).clamp(-GENE_CLAMP, GENE_CLAMP);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitness::EvalConfig;
    use crate::fixture;
    use crate::genome::default_shape;

    fn eval() -> EvalConfig {
        EvalConfig {
            action_repeat: 4,
            cut_stuck_frames: 0,
        }
    }

    /// A runway with a two-tile pit at column 12: the all-zero genome runs right and falls in,
    /// and a jump from the edge clears it. The smallest useful jump is two tiles, which is why the
    /// fixture is two wide rather than three.
    fn pit() -> Level {
        fixture::gap_runway("mistake-pit", 40, 38, &[(12, 2)])
    }

    #[test]
    fn a_pit_is_a_mistake_and_the_repair_turns_the_run_into_a_jump() {
        let level = pit();
        let genome = Genome::zeros(0, default_shape());
        let mistake = find_mistake(&level, &genome, &eval(), 7).expect("the run falls in a pit");

        assert!(
            mistake.chosen == AgentAction::Run || mistake.chosen == AgentAction::Walk,
            "the zero policy runs: {:?}",
            mistake.chosen
        );
        assert_ne!(
            mistake.better, mistake.chosen,
            "a mistake without a different action to take is not a mistake"
        );
        assert!(
            mistake.better == AgentAction::RunJump
                || mistake.better == AgentAction::JumpInPlace
                || mistake.better == AgentAction::Sprint
                || mistake.better == AgentAction::SprintJump,
            "the fixture is cleared by committing harder (jump or sprint), not by retreating: {:?}",
            mistake.better
        );
        assert!(mistake.gain >= MIN_GAIN);

        let child = learn(&genome, &mistake).expect("a step that flips the decision exists");
        let mut scratch = ActScratch::new();
        scratch.restore_context(&mistake.context);
        assert_eq!(
            child.act(&mistake.observation, &mut scratch),
            mistake.better,
            "the repair must do the better thing at the state it was learned from"
        );
        assert!(
            child.biases != genome.biases || child.weights != genome.weights,
            "the repair must change the genome, not relabel it"
        );
        assert_eq!(child.shape, genome.shape);
    }

    #[test]
    fn a_repair_is_deterministic_and_leaves_the_parent_alone() {
        let level = pit();
        let genome = Genome::zeros(0, default_shape());
        let mistake = find_mistake(&level, &genome, &eval(), 7).expect("a mistake exists");
        let a = learn(&genome, &mistake).expect("a repair exists");
        let b = learn(&genome, &mistake).expect("a repair exists");
        assert_eq!(a, b, "the same mistake must learn the same child");
        assert_eq!(
            genome,
            Genome::zeros(0, default_shape()),
            "learning must not modify its parent"
        );
    }

    #[test]
    fn an_episode_that_wins_has_no_mistake_to_learn_from() {
        // A flat runway with the flag on it: the zero policy runs into the goal.
        let level = fixture::gap_runway("mistake-flat", 12, 10, &[]);
        let genome = Genome::zeros(0, default_shape());
        let result = crate::run_episode(&level, &genome, &eval(), 3);
        assert!(
            result.won(),
            "the fixture must be won, or this proves nothing"
        );
        assert!(
            find_mistakes(&level, &genome, &eval(), 3, 3).is_empty(),
            "a win has no failure state for this module to repair"
        );
    }
}
