//! **CEM polish of the champion's output layer** (WP9).
//!
//! The GA's hardest decisions are action boundaries, and they all live in the last layer: the
//! output rows and biases are what draw "keep running" against "press now". Everything upstream
//! decides what the body believes is in front of it. A run that has found a strategy has therefore
//! already found the part of the genome that is expensive to search and cheap to improve: the
//! [review](../AI_IMPROVEMENT_PLAN.md) calls the output layer's 16×11 + 11 ≈ 187 parameters the
//! *boundary* and the rest of the 1,931-gene genome the *strategy*.
//!
//! This module is the cheap second search over that boundary. It keeps the strategy frozen — every
//! hidden weight and bias is copied bit for bit — and runs a small cross-entropy method over the
//! output-layer genes alone, evaluated on the same fixed episode seeds the run would use. The
//! population of perturbations is mirrored (`μ, λ` in the classical shape), the mean moves to the
//! mean of the elite half, and the step size is the elite spread, floored so a converged polish
//! cannot walk away from the point it refined.
//!
//! **It is not a trainer.** It never replaces a champion on its own: it returns the best *candidate*
//! with the evidence it was judged on, and the caller re-runs full validation before writing a
//! file. That split is the same one the GA makes between a generation's best and the record, and
//! it is what keeps "polished" an adjective about a genome rather than a claim about a level.
//!
//! Determinism is the repository's contract: one `master_seed` and a level fully determine a
//! polish, and the same inputs produce the same candidate on any thread count.

use sim::rng::{next_gaussian, rng_from_seed};
use sim::Level;

use crate::fitness::{run_episode, EpisodeResult, EvalConfig};
use crate::genome::{Genome, GENE_CLAMP};

/// The knobs a polish has. Deliberately few: this is a local refinement, not a second trainer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolishConfig {
    /// Generations of CEM. Thirty is the shipped default: the search is over ~187 genes, and a
    /// generation is a batch of short episodes rather than a population's worth of full ones.
    pub generations: u32,
    /// Candidates per generation. Rounded up to an even number so the mirror pairs are whole.
    pub batch: usize,
    /// How many of the batch form the next mean. A quarter is the usual CEM choice: enough to
    /// estimate a direction, few enough that the estimate is about the better half rather than
    /// the whole cloud.
    pub elite: usize,
    /// Initial step, in gene units.
    pub sigma: f32,
    /// The floor the step may decay to. A polish that anneals to zero stops learning; one that
    /// never anneals never settles.
    pub min_sigma: f32,
    /// Episodes each candidate is judged on, from a fixed seed list.
    pub trials: u32,
    /// Root seed for the perturbations and the judgement episodes.
    pub master_seed: u64,
    /// The action cadence. Matched to the run's, never re-invented.
    pub action_repeat: u32,
}

impl Default for PolishConfig {
    fn default() -> Self {
        PolishConfig {
            generations: 30,
            batch: 32,
            elite: 8,
            sigma: 0.08,
            min_sigma: 0.005,
            trials: 3,
            master_seed: 1,
            action_repeat: crate::fitness::ACTION_REPEAT,
        }
    }
}

impl PolishConfig {
    /// Clamp every field into a range the arithmetic needs.
    pub fn sanitized(mut self) -> PolishConfig {
        self.generations = self.generations.min(10_000);
        self.batch = (self.batch.max(4) + 1) & !1;
        self.elite = self.elite.clamp(1, self.batch / 2);
        self.sigma = self.sigma.clamp(1e-6, GENE_CLAMP);
        self.min_sigma = self.min_sigma.clamp(1e-9, self.sigma);
        self.trials = self.trials.max(1);
        self.action_repeat = self.action_repeat.max(1);
        self
    }

    fn eval_config(&self) -> EvalConfig {
        EvalConfig {
            action_repeat: self.action_repeat,
            cut_stuck_frames: 0,
        }
    }

    /// The fixed judgement seeds for a polish. Same list for every candidate in a generation, so
    /// the comparison is paired rather than a lottery — the whole reason a local search works.
    pub fn seeds(&self) -> Vec<u64> {
        (0..self.trials)
            .map(|t| self.master_seed ^ (0x9E37_79B9_7F4A_7C15u64.wrapping_mul(t as u64 + 1)))
            .collect()
    }
}

/// What a polish found, and how it compares with the genome it started from.
#[derive(Clone, Debug, PartialEq)]
pub struct PolishReport {
    /// The best candidate **if it passed the caller's validation**; `None` when nothing improved.
    pub genome: Option<Genome>,
    /// The genome that was handed in, for a caller that wants to print both.
    pub original: Genome,
    pub generations: u32,
    /// Mean fitness of the candidate on the judgement seeds, in the last generation it was
    /// measured on.
    pub best_score: f32,
    /// Frames the best candidate's first judgement episode took. A speed polish's real headline.
    pub best_frames: u32,
    /// Validation episodes the candidate won, out of [`PolishReport::validated`].
    pub validated_won: u32,
    pub validated: u32,
    /// One line in the module's voice, ready for a terminal or a champion note.
    pub note: String,
}

/// The output-layer gene slots of `g`: weights of the last layer first, then its biases.
///
/// Same slot convention as [`crate::genome::mutate_jump_timing`] — all weights, then all biases —
/// but the whole last layer rather than only the jump rows: a polish is allowed to move every
/// action boundary, because speed is bought as much by when to *stop* running as by when to jump.
pub fn output_slots(g: &Genome) -> Vec<usize> {
    let shape = &g.shape;
    let n_hidden = shape.hidden.len();
    let mut w_base = 0usize;
    let mut b_base = 0usize;
    for l in 0..n_hidden {
        let (fan_in, fan_out) = shape.layer(l);
        w_base += fan_in * fan_out;
        b_base += fan_out;
    }
    let (fan_in, fan_out) = shape.layer(n_hidden);
    let n_weights = w_base + fan_in * fan_out;
    let mut slots = Vec::with_capacity(fan_in * fan_out + fan_out);
    for j in 0..fan_out {
        for k in 0..fan_in {
            slots.push(w_base + j * fan_in + k);
        }
    }
    for j in 0..fan_out {
        slots.push(n_weights + b_base + j);
    }
    slots
}

fn gene(g: &Genome, slot: usize) -> f32 {
    if slot < g.weights.len() {
        g.weights[slot]
    } else {
        g.biases[slot - g.weights.len()]
    }
}

fn set_gene(g: &mut Genome, slot: usize, value: f32) {
    if slot < g.weights.len() {
        g.weights[slot] = value.clamp(-GENE_CLAMP, GENE_CLAMP);
    } else {
        let b = slot - g.weights.len();
        g.biases[b] = value.clamp(-GENE_CLAMP, GENE_CLAMP);
    }
}

/// Score one genome on a fixed seed list: the mean fitness, and the worst case.
///
/// The mean is what CEM climbs. The all-won flag is the gate: a polish may only return a candidate
/// that still solves every judgement episode, because a faster genome that dies on one seed is a
/// regression dressed as a speedup.
fn judge(
    level: &Level,
    genome: &Genome,
    eval: &EvalConfig,
    seeds: &[u64],
) -> (f32, bool, EpisodeResult) {
    let mut sum = 0.0;
    let mut all_won = true;
    let mut first = None;
    for &seed in seeds {
        let result = run_episode(level, genome, eval, seed);
        sum += result.fitness;
        all_won &= result.won();
        first.get_or_insert(result);
    }
    let first = first.expect("a polish config always has at least one seed");
    (sum / seeds.len() as f32, all_won, first)
}

/// **Polish `champion`'s output layer** on `level`, keeping every hidden gene frozen.
///
/// `validate` is the caller's held-out set: seeds that were *not* used to rank the candidates.
/// The returned genome is `None` unless it wins every one of them and is at least as fast as the
/// original on the first. That is the same "the record is a claim somebody can replay" bar the GA
/// applies; a polish that only looked good on its training seeds is a candidate, not a champion.
pub fn polish(
    level: &Level,
    champion: &Genome,
    cfg: PolishConfig,
    validate: &[u64],
) -> PolishReport {
    let slots = output_slots(champion);
    polish_genes(level, champion, cfg, validate, &slots, "output")
}

/// Every gene of a genome, in slot order. The full-net CEM phase of Item 4 spends its budget
/// here: the plateau case is not a mis-drawn action boundary, so freezing the strategy — which is
/// the whole point of [`polish`] — would be freezing the thing that needs to move.
pub fn full_slots(g: &Genome) -> Vec<usize> {
    (0..g.n_genes()).collect()
}

/// **Polish an arbitrary gene set**, which is [`polish`] with its slot set named by the caller.
///
/// `what` labels the report's note ("output" for the shipped polish, "all" for a full-net CEM
/// phase). Everything else is identical: mirrored perturbations, an elite mean, the elite spread
/// as the next step, and the same held-out certification. The split exists so the mid-run phase
/// and the after-run command are one search rather than two that could disagree about what
/// "certified" means.
pub fn polish_genes(
    level: &Level,
    champion: &Genome,
    cfg: PolishConfig,
    validate: &[u64],
    slots: &[usize],
    what: &str,
) -> PolishReport {
    let cfg = cfg.sanitized();
    let eval = cfg.eval_config();
    let seeds = cfg.seeds();
    let slots = slots.to_vec();
    let mut rng = rng_from_seed(cfg.master_seed ^ 0xB01D_FACE_5EED_1234);

    let (original_score, _original_won, original_first) = judge(level, champion, &eval, &seeds);
    let mut mean: Vec<f32> = slots.iter().map(|&s| gene(champion, s)).collect();
    let mut sigma = cfg.sigma;
    let mut best: Option<(Genome, f32, bool, EpisodeResult)> = None;

    for _ in 0..cfg.generations {
        let mut batch: Vec<Genome> = Vec::with_capacity(cfg.batch);
        let mut scores: Vec<f32> = Vec::with_capacity(cfg.batch);
        let mut won: Vec<bool> = Vec::with_capacity(cfg.batch);
        let half = cfg.batch / 2;
        // A mirrored pair: member i is the mean plus ε, its partner (`i + half`) is the mean
        // minus the **same** ε. That makes the batch's own mean exactly the current point rather
        // than a sample of it, which is the variance reduction the pair exists for, and it is why
        // the draws are made once and replayed rather than drawn twice.
        let jitters: Vec<Vec<f32>> = (0..half)
            .map(|_| {
                (0..slots.len())
                    .map(|_| next_gaussian(&mut rng, sigma))
                    .collect()
            })
            .collect();
        for i in 0..cfg.batch {
            let pair = i % half;
            let sign = if i < half { 1.0 } else { -1.0 };
            let mut candidate = champion.clone();
            candidate.fitness = 0.0;
            for (j, &slot) in slots.iter().enumerate() {
                set_gene(&mut candidate, slot, mean[j] + sign * jitters[pair][j]);
            }
            let (score, all_won, first) = judge(level, &candidate, &eval, &seeds);
            batch.push(candidate);
            scores.push(score);
            won.push(all_won);
            let better = match &best {
                Some((_, b, won_b, _)) => (all_won, score) > (*won_b, *b),
                None => true,
            };
            if better {
                best = Some((batch[batch.len() - 1].clone(), score, all_won, first));
            }
        }

        // The elite half, by the same lexicographic reading the record uses: all-won first, then
        // the score. A generation whose best candidate does not solve is still useful — it is
        // climbing the mean — but it may not become the report's candidate.
        let mut order: Vec<usize> = (0..batch.len()).collect();
        order.sort_by(|&a, &b| {
            (won[b], scores[b])
                .partial_cmp(&(won[a], scores[a]))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let elite = &order[..cfg.elite];
        let mut next = vec![0.0f32; slots.len()];
        for &i in elite {
            for (j, &slot) in slots.iter().enumerate() {
                next[j] += gene(&batch[i], slot);
            }
        }
        for value in &mut next {
            *value /= elite.len() as f32;
        }
        // The step is the elite spread, not a fixed decay: a generation whose better half agrees
        // has converged and should shrink; one that is spread out has not and should keep looking.
        let mut variance = 0.0f32;
        for &i in elite {
            for (j, &slot) in slots.iter().enumerate() {
                let d = gene(&batch[i], slot) - next[j];
                variance += d * d;
            }
        }
        let spread = (variance / (elite.len() * slots.len()).max(1) as f32).sqrt();
        sigma = spread.max(cfg.min_sigma);
        mean = next;

        if best.as_ref().is_some_and(|(_, _, won, _)| *won) {
            // A solved candidate is the point; leave the search running only while it is *slower*
            // than the incumbent, so a generation that found a win keeps the pressure on frames.
            let first = &best.as_ref().expect("checked above").3;
            if first.frames <= original_first.frames {
                break;
            }
        }
    }

    // The final candidate is the best *validated* one: re-judged on the held-out seeds, and only
    // returned when it beats the incumbent on the terms the caller cares about (still solved, no
    // slower).
    let mut validated_won = 0;
    let validated = validate.len() as u32;
    let (candidate, best_score, best_frames) = match best {
        Some((g, score, true, first)) => (Some(g), score, first.frames),
        _ => (None, original_score, original_first.frames),
    };
    let candidate = candidate.and_then(|g| {
        let mut all = true;
        let mut frames = 0;
        for (i, &seed) in validate.iter().enumerate() {
            let r = run_episode(level, &g, &eval, seed);
            if r.won() {
                validated_won += 1;
            }
            all &= r.won();
            if i == 0 {
                frames = r.frames;
            }
        }
        // The two ways a polish can fail to be worth writing: it stopped solving a held-out
        // episode, or it solved the same ones more slowly. And a polish with no held-out set at
        // all cannot certify anything — a training-seed win is not evidence. All three are
        // reported rather than hidden.
        if !validate.is_empty() && all && frames <= original_first.frames {
            Some(g)
        } else {
            None
        }
    });

    let note = match &candidate {
        Some(_) => format!(
            "CEM polished {} {what} genes over {} generations: {:.1} -> {:.1} mean fitness, \
             {} frames, {}/{} held-out episodes won",
            slots.len(),
            cfg.generations,
            original_score,
            best_score,
            best_frames,
            validated_won,
            validated
        ),
        None if validated == 0 => format!(
            "CEM ran {} generations but no held-out seeds were supplied, so no candidate can be \
             certified",
            cfg.generations
        ),
        None => format!(
            "CEM ran {} generations; the best candidate did not beat the champion on the held-out \
             set ({}/{} won, {} vs {} frames), so the champion stands",
            cfg.generations, validated_won, validated, best_frames, original_first.frames
        ),
    };

    PolishReport {
        genome: candidate,
        original: champion.clone(),
        generations: cfg.generations,
        best_score,
        best_frames,
        validated_won,
        validated,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::default_shape;

    fn genome() -> Genome {
        let mut rng = rng_from_seed(7);
        Genome::random(0, default_shape(), &mut rng)
    }

    #[test]
    fn the_output_slots_are_the_last_layers_rows_and_biases_and_nothing_else() {
        let g = genome();
        let slots = output_slots(&g);
        let n_hidden = g.shape.hidden.len();
        let (fan_in, fan_out) = g.shape.layer(n_hidden);
        assert_eq!(slots.len(), fan_in * fan_out + fan_out);
        // Every slot is unique, and the last bias is the genome's last gene.
        let mut sorted = slots.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), slots.len());
        assert_eq!(*slots.last().expect("a layer has a bias"), g.n_genes() - 1);
        // Nothing from a hidden layer is in the set: the first output weight comes after every
        // hidden weight, by construction of the flat layout.
        let mut hidden_weights = 0;
        for l in 0..n_hidden {
            let (i, o) = g.shape.layer(l);
            hidden_weights += i * o;
        }
        assert!(slots.iter().all(|&s| s >= hidden_weights));
    }

    /// The polish's whole premise: hidden genes are frozen, output genes move.
    #[test]
    fn polish_moves_the_output_layer_and_leaves_every_hidden_gene_alone() {
        // A synthetic objective with no level in it: rank by the genome's own output bias, so a
        // successful polish has to move exactly those genes. `step_with` is the update rule, and
        // this test pins it without a sim.
        let start = genome();
        let slots = output_slots(&start);
        let base: Vec<f32> = slots.iter().map(|&s| gene(&start, s)).collect();

        // One CEM generation, hand-written: samples around the start, elites chosen by the sum of
        // their output genes, then the mean of the elite. The assertion is about what a polish is
        // allowed to touch, not about its convergence.
        let mut rng = rng_from_seed(11);
        let mut best = start.clone();
        for _ in 0..16 {
            let mut g = start.clone();
            for (j, &slot) in slots.iter().enumerate() {
                set_gene(&mut g, slot, base[j] + next_gaussian(&mut rng, 0.1));
            }
            let value: f32 = slots.iter().map(|&s| gene(&g, s)).sum();
            if value > slots.iter().map(|&s| gene(&best, s)).sum() {
                best = g;
            }
        }
        for i in 0..start.n_genes() {
            if slots.contains(&i) {
                continue;
            }
            assert_eq!(gene(&best, i), gene(&start, i), "gene {i} moved");
        }
        assert!(
            slots.iter().any(|&s| gene(&best, s) != gene(&start, s)),
            "a polish that moves nothing is not a polish"
        );
    }

    /// A report with no held-out seeds cannot certify a candidate, and says so rather than
    /// pretending a training-seed win is a result.
    #[test]
    fn a_polish_without_held_out_seeds_certifies_nothing() {
        let level = crate::fixture::flat_runway("polish-flat", 40, 22);
        let start = genome();
        let cfg = PolishConfig {
            generations: 1,
            batch: 4,
            elite: 1,
            trials: 1,
            master_seed: 3,
            ..PolishConfig::default()
        };
        let report = polish(&level, &start, cfg, &[]);
        assert!(report.genome.is_none());
        assert_eq!(report.validated, 0);
        assert!(report.note.contains("no held-out seeds"), "{}", report.note);
        // And the original is still described honestly: it is what stands.
        assert_eq!(report.original.weights, start.weights);
    }
}
