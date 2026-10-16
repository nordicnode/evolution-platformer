//! An evolution-strategy baseline for the genetic algorithm (§7, REVIEW.md §4).
//!
//! Every claim this project makes about its search is a comparison against an *earlier version of
//! the same search*. That answers "did this change help" and cannot answer "is the search the
//! binding constraint at all" — for that, the GA needs an arm that is not a GA. This module is a
//! classic OpenAI-ES-style `(µ, λ)`: a mean genome, `λ` mirrored Gaussian perturbations of it per
//! generation, ranked by fitness, and a mean that moves in the direction the ranks point.
//!
//! Three properties are deliberate, and each of them is what makes it a *baseline* rather than a
//! second implementation of the same idea:
//!
//! * **The episode budget is the GA's.** The batch is sized from the same population number and the
//!   fitnesses come from [`crate::ga::evaluate_population`] with a [`GaConfig`] mirroring this
//!   config, so both arms spend the same episodes per generation, use the same `episode_seed`
//!   formula, the same `--trials` averaging and the same scoring. Comparing two searches that were
//!   given different budgets measures the budgets, not the searches.
//! * **It climbs a *smooth* estimate of the gradient.** ES does not need a lineage to improve on: it
//!   averages `λ` samples of the objective and moves the mean along the rank-weighted direction.
//!   That is a real difference in kind from tournament selection, and it is exactly the difference
//!   worth measuring on a fitness landscape with a plateau in it.
//! * **The update is separable from the objective.** [`EsTrainer::step_with`] is the whole update
//!   rule and takes fitnesses as a closure; [`EsTrainer::step`] is that plus the real evaluation.
//!   The unit test that proves the mean climbs is therefore a test of the *rule*, with no level in
//!   it at all — which is the only way a search's arithmetic can be pinned.
//!
//! Determinism is the repository's contract and is kept here the same way: every perturbation is
//! drawn from one stream seeded by [`EsConfig::master_seed`] in a fixed order, so a config plus a
//! level fully determines a run, and two trainers built from one seed produce identical rows.

use sim::rng::{next_gaussian, rng_from_seed, Rng};
use sim::Level;

use crate::fitness::ACTION_REPEAT;
use crate::ga::{evaluate_population, GaConfig};
use crate::genome::{default_shape, Genome, NetShape, GENE_CLAMP};

/// The knobs a baseline run has, and the reason each default is what it is.
///
/// The numbers that matter are the two that make the comparison fair: `pop_size` is the GA's own
/// default population, and `action_repeat` is the sim's cadence, so "one generation" means the same
/// compute in both arms.
#[derive(Clone, Debug, PartialEq)]
pub struct EsConfig {
    /// Perturbations per generation, i.e. `λ` (`λ/2` mirrored pairs). Rounded down to an even
    /// number by [`EsConfig::batch`], because a mirror without its pair is half an estimate.
    pub pop_size: usize,
    /// Standard deviation of the perturbation, in gene units. The GA's `mutation_sigma` analogue is
    /// 0.30 and is a *mutation* rate; an ES step wants a smaller, steadier probe, and 0.05 keeps the
    /// rank ordering informative over the whole run instead of saturating on the biggest mutants.
    pub sigma: f32,
    /// Learning rate: how far along the rank-weighted direction the mean moves each generation.
    /// Small, because a `λ = 200` estimate is noisy in 1 700 dimensions and the mean has to outlive
    /// any single generation's sample.
    pub learning_rate: f32,
    /// Root seed for the whole run, and the only source of randomness.
    pub master_seed: u64,
    pub shape: NetShape,
    pub action_repeat: u32,
    /// Episodes per genome, averaged — the GA's `--trials`, matched rather than re-invented.
    pub trials: u32,
}

impl Default for EsConfig {
    fn default() -> Self {
        EsConfig {
            pop_size: 200,
            sigma: 0.05,
            learning_rate: 0.01,
            master_seed: 1,
            shape: default_shape(),
            action_repeat: ACTION_REPEAT,
            trials: 1,
        }
    }
}

impl EsConfig {
    /// Clamp every field into the range the arithmetic needs, so a stray command-line argument
    /// cannot produce a degenerate run: a batch of zero, a step of zero (which would freeze the
    /// mean), a negative learning rate (which is a gradient *descent* on a fitness) or a zero trial
    /// count (which is a division by zero in the mean).
    pub fn sanitized(mut self) -> EsConfig {
        self.pop_size = self.pop_size.max(2);
        self.sigma = self.sigma.clamp(1e-6, GENE_CLAMP);
        self.learning_rate = self.learning_rate.clamp(1e-6, 1.0);
        self.trials = self.trials.max(1);
        self.action_repeat = self.action_repeat.max(1);
        self
    }

    /// `λ`: how many perturbations are evaluated per generation. Even, at least 2.
    pub fn batch(&self) -> usize {
        (self.pop_size.max(2) / 2) * 2
    }
}

/// One generation of a baseline run, shaped like the GA's `GenRow` so the same chart can print it.
#[derive(Clone, Debug, PartialEq)]
pub struct EsRow {
    pub generation: u32,
    /// Best fitness in the batch.
    pub max_fitness: f32,
    /// Mean fitness of the batch.
    pub mean_fitness: f32,
    /// Whether any genome of the batch touched the flag.
    pub solved: bool,
}

/// A `(µ, λ)` evolution strategy over the same fixed-topology genome the GA uses.
pub struct EsTrainer {
    cfg: EsConfig,
    /// The distribution's mean: the thing that is being optimised, and the arm's actual artifact.
    mean: Genome,
    rng: Rng,
    generation: u32,
    /// Best evaluated *point* — a perturbation, not the mean — with the generation that found it.
    best: Option<(u32, Genome, f32)>,
    history: Vec<EsRow>,
}

impl EsTrainer {
    pub fn new(cfg: EsConfig) -> EsTrainer {
        let cfg = cfg.sanitized();
        let mut rng = rng_from_seed(cfg.master_seed);
        // The mean starts random for the same reason the GA's population does: a zero start is a
        // policy, and a baseline that begins on a policy is measuring that policy's luck.
        let mean = Genome::random(0, cfg.shape.clone(), &mut rng);
        EsTrainer {
            cfg,
            mean,
            rng,
            generation: 0,
            best: None,
            history: Vec::new(),
        }
    }

    pub fn config(&self) -> &EsConfig {
        &self.cfg
    }

    /// The current mean: the genome this arm would ship if the run stopped here.
    pub fn mean(&self) -> &Genome {
        &self.mean
    }

    pub fn history(&self) -> &[EsRow] {
        &self.history
    }

    /// The best point the search ever *evaluated*, with the generation that found it.
    pub fn best_ever(&self) -> Option<(u32, &Genome, f32)> {
        self.best.as_ref().map(|(g, genome, f)| (*g, genome, *f))
    }

    /// One generation against a real level: sample, evaluate, update.
    pub fn step(&mut self, level: &Level) -> EsRow {
        let cfg = self.eval_config();
        let generation = self.generation;
        // The win flag cannot come back through the fitness closure, so it is read from the
        // results the closure sees. Nothing else in the update depends on it.
        let mut solved = false;
        let mut row = self.step_with(|batch| {
            let results = evaluate_population(level, batch, &cfg, generation);
            solved = results.iter().any(|r| r.won());
            results.iter().map(|r| r.mean_fitness).collect()
        });
        row.solved = solved;
        if let Some(last) = self.history.last_mut() {
            last.solved = solved;
        }
        row
    }

    /// One generation against an *objective*: the entire update rule, with no level in it.
    ///
    /// `evaluate` is handed the batch of perturbations — `λ` genomes, mirrored in pairs — and
    /// returns one fitness per genome, in the same order. The mean then moves by
    ///
    /// ```text
    /// mean_j += (lr / (λ σ)) · Σ_i w_i · ε_ij
    /// ```
    ///
    /// where `ε_ij = (member_ij - mean_j) / σ` is the unit perturbation that produced the member and
    /// `w_i` is its centred rank, normalised so the weights sum to one in absolute value. Every gene
    /// is clamped to `±GENE_CLAMP` after the step, exactly as [`crate::genome::mutate`] clamps: an
    /// unclamped mean can walk out to values that pin every hidden unit into tanh saturation, which
    /// is a dead lineage in the GA and a dead distribution here.
    pub fn step_with<F>(&mut self, mut evaluate: F) -> EsRow
    where
        F: FnMut(&[Genome]) -> Vec<f32>,
    {
        let members = self.sample();
        let fitness = evaluate(&members);
        assert_eq!(
            fitness.len(),
            members.len(),
            "the objective must score every member of the batch"
        );

        // Centred ranks: the update is a *direction*, so what a member's fitness earns it is its
        // place in the ordering, not the size of the number. That is what makes the arm usable on a
        // fitness whose scale drifts through a run (this project's does: 250 early, 11 000 when a
        // flag is touched) without a second constant to tune per level.
        let lambda = fitness.len() as f32;
        let mut order: Vec<usize> = (0..fitness.len()).collect();
        order.sort_by(|a, b| {
            fitness[*a]
                .partial_cmp(&fitness[*b])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut weights = vec![0.0f32; fitness.len()];
        for (rank, &i) in order.iter().enumerate() {
            weights[i] = rank as f32 - (lambda - 1.0) * 0.5;
        }
        let total: f32 = weights.iter().map(|w| w.abs()).sum();
        if total > 0.0 {
            for w in &mut weights {
                *w /= total;
            }
        }

        let sigma = self.cfg.sigma;
        let gain = self.cfg.learning_rate / lambda;
        let mut new_w = vec![0.0f32; self.mean.weights.len()];
        let mut new_b = vec![0.0f32; self.mean.biases.len()];
        for (i, member) in members.iter().enumerate() {
            let w = weights[i] * gain / (sigma * sigma);
            for ((acc, gene), mean) in new_w
                .iter_mut()
                .zip(member.weights.iter())
                .zip(self.mean.weights.iter())
            {
                *acc += w * (gene - mean);
            }
            for ((acc, gene), mean) in new_b
                .iter_mut()
                .zip(member.biases.iter())
                .zip(self.mean.biases.iter())
            {
                *acc += w * (gene - mean);
            }
        }
        for (gene, step) in self.mean.weights.iter_mut().zip(new_w.iter()) {
            *gene = (*gene + *step).clamp(-GENE_CLAMP, GENE_CLAMP);
        }
        for (gene, step) in self.mean.biases.iter_mut().zip(new_b.iter()) {
            *gene = (*gene + *step).clamp(-GENE_CLAMP, GENE_CLAMP);
        }

        // The best *point*, not the best mean: a baseline's headline is the genome it found, and the
        // mean is a distribution rather than a policy that anyone can replay.
        let mut max_fitness = f32::NEG_INFINITY;
        let mut sum = 0.0f32;
        for (member, f) in members.iter().zip(&fitness) {
            max_fitness = max_fitness.max(*f);
            sum += *f;
            let better = match &self.best {
                Some((_, _, best)) => *f > *best,
                None => true,
            };
            if better {
                self.best = Some((self.generation, member.clone(), *f));
            }
        }
        let row = EsRow {
            generation: self.generation,
            max_fitness,
            mean_fitness: sum / fitness.len() as f32,
            solved: false,
        };
        self.generation += 1;
        self.history.push(row.clone());
        row
    }

    /// `λ` perturbations of the mean in mirrored pairs, drawn from one stream in a fixed order.
    ///
    /// Mirroring is not thrift: a pair straddles the mean, so the two fitnesses differ by the
    /// objective's local slope along that direction with the second-order term largely canceling.
    /// It halves the variance of the estimate for the same number of episodes, which matters when
    /// `λ` is two orders of magnitude below the number of genes.
    fn sample(&mut self) -> Vec<Genome> {
        let batch = self.cfg.batch();
        let sigma = self.cfg.sigma;
        let mut members = Vec::with_capacity(batch);
        // Ids are unique per (generation, member) because they are part of `episode_seed`: two
        // batches that reused one id would be scored on the same episodes, which for a baseline
        // whose whole point is averaging over samples would be a silent loss of half its evidence.
        let base = self.generation.wrapping_mul(batch as u32);
        for pair in 0..batch / 2 {
            let n_w = self.mean.weights.len();
            let n_b = self.mean.biases.len();
            let mut eps_w = Vec::with_capacity(n_w);
            let mut eps_b = Vec::with_capacity(n_b);
            for _ in 0..n_w {
                eps_w.push(next_gaussian(&mut self.rng, 1.0));
            }
            for _ in 0..n_b {
                eps_b.push(next_gaussian(&mut self.rng, 1.0));
            }
            for sign in [1.0f32, -1.0] {
                let mut member = self.mean.clone();
                member.id = base + (pair * 2) as u32 + if sign > 0.0 { 0 } else { 1 };
                for (j, eps) in eps_w.iter().enumerate() {
                    member.weights[j] =
                        (self.mean.weights[j] + sign * sigma * eps).clamp(-GENE_CLAMP, GENE_CLAMP);
                }
                for (j, eps) in eps_b.iter().enumerate() {
                    member.biases[j] =
                        (self.mean.biases[j] + sign * sigma * eps).clamp(-GENE_CLAMP, GENE_CLAMP);
                }
                member.fitness = 0.0;
                members.push(member);
            }
        }
        members
    }

    /// The [`GaConfig`] this arm evaluates through.
    ///
    /// Only the fields that decide *what an episode is* are set from here; everything else is the
    /// GA's own default and is deliberately not inherited, because the GA's search knobs (elites,
    /// tournaments, fine children) describe a population this arm does not have. `pop_size` is
    /// carried so a caller reading the config sees the batch it is actually paying for.
    fn eval_config(&self) -> GaConfig {
        GaConfig {
            pop_size: self.cfg.batch(),
            trials: self.cfg.trials,
            master_seed: self.cfg.master_seed,
            shape: self.cfg.shape.clone(),
            action_repeat: self.cfg.action_repeat,
            ..GaConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The comparison is only as good as this equality.** An ES arm that pays for a different
    /// number of episodes per generation, or queries the network on a different cadence, is not a
    /// baseline for the GA — it is a second experiment, and any difference between them would be
    /// the arithmetic rather than the search. Both numbers come from the same place (the GA's own
    /// default and the shared `ACTION_REPEAT`), which is the only way they stay equal when one of
    /// them moves: this test is what fails when someone changes one and not the other.
    #[test]
    fn the_baselines_default_budget_matches_the_gas() {
        let ga = GaConfig::default();
        let es = EsConfig::default();
        assert_eq!(es.pop_size, ga.pop_size, "λ must be the GA's population size");
        assert_eq!(es.action_repeat, ga.action_repeat, "the cadence is shared");
        assert_eq!(es.trials, ga.trials, "and so is the trial count");
        assert_eq!(es.batch(), ga.pop_size, "and the batch really is that size");
        assert_eq!(es.shape, ga.shape, "a smaller net would search a smaller space");
    }

    /// A deliberately tiny net for the arithmetic tests.
    ///
    /// The update rule's estimate is only a *gradient* when the batch outnumbers the genes: with
    /// `λ` samples in `n` dimensions the estimate correlates with the true gradient at roughly
    /// `sqrt(λ / n)`, so a test on the shipped 1 739-gene net at `λ = 64` would be measuring noise
    /// and would pass or fail on the seed. Twenty-three genes against sixty-four samples is a
    /// setting where the rule can be judged on its arithmetic.
    fn small_shape() -> NetShape {
        NetShape {
            inputs: 4,
            hidden: vec![3],
            outputs: 2,
            feedback: 0,
        }
    }

    fn trainer(pop_size: usize, learning_rate: f32) -> EsTrainer {
        EsTrainer::new(EsConfig {
            pop_size,
            learning_rate,
            sigma: 0.05,
            shape: small_shape(),
            ..EsConfig::default()
        })
    }

    fn genes(g: &Genome) -> Vec<f32> {
        g.weights.iter().chain(&g.biases).copied().collect()
    }

    fn distance(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>()
    }

    /// The reason [`EsTrainer::step_with`] exists: the update rule climbs, on an objective that has
    /// nothing to do with a level. A rule with a flipped sign, a gain of zero, or weights that do
    /// not read the ranks would all leave the mean where it started or move it *away*, and every one
    /// of those failures is invisible in a training curve because a fitness chart cannot tell a
    /// climbing mean from a lucky batch.
    #[test]
    fn the_update_rule_climbs_a_synthetic_objective() {
        let mut t = trainer(64, 0.05);
        let mut target_rng = rng_from_seed(99);
        let target = Genome::random(1, small_shape(), &mut target_rng);
        let target_genes = genes(&target);

        let start = distance(&genes(t.mean()), &target_genes);
        for _ in 0..300 {
            t.step_with(|batch| {
                batch
                    .iter()
                    .map(|g| -distance(&genes(g), &target_genes))
                    .collect()
            });
        }
        let end = distance(&genes(t.mean()), &target_genes);
        assert!(
            end < start * 0.5,
            "the mean should be much closer to the target after 300 generations: {start} -> {end}"
        );
        assert!(
            t.best_ever().map(|(_, _, f)| f).unwrap_or(f32::NEG_INFINITY) > -start * 0.5,
            "the best evaluated point should be closer than the mean started"
        );
        assert_eq!(t.history().len(), 300);
        assert_eq!(t.history().last().unwrap().generation, 299);
    }

    /// A run is reproducible from its seed (§5.6). Two trainers built from one config must agree on
    /// every row and on the mean itself, or the baseline's numbers are not comparable with anything
    /// — including its own earlier version, which is the only thing it is here to be compared with.
    #[test]
    fn the_same_seed_gives_the_same_rows_and_the_same_mean() {
        let objective = |batch: &[Genome]| -> Vec<f32> {
            batch.iter().map(|g| -genes(g).iter().map(|x| x * x).sum::<f32>()).collect()
        };
        let mut a = trainer(32, 0.05);
        let mut b = trainer(32, 0.05);
        for _ in 0..25 {
            a.step_with(objective);
            b.step_with(objective);
        }
        assert_eq!(a.history(), b.history());
        assert_eq!(genes(a.mean()), genes(b.mean()));

        // And the perturbations really are mirrored: the first pair is one vector and its negative,
        // so their mid-point is the mean it was drawn from.
        let mut c = trainer(4, 0.05);
        let members = c.sample();
        assert_eq!(members.len(), 4);
        for j in 0..members[0].weights.len() {
            let mid = 0.5 * (members[0].weights[j] + members[1].weights[j]);
            assert!(
                (mid - c.mean().weights[j]).abs() < 1e-5,
                "a mirrored pair must straddle the mean at gene {j}"
            );
        }
        assert_ne!(members[0].id, members[1].id, "and they cannot share a seed");
    }

    /// A batch has to be even and non-empty whatever the operator typed: `--pop 1` is a request for
    /// a mirror without a pair, and a zero-gain step would silently freeze the run while every row
    /// still printed a plausible fitness.
    #[test]
    fn a_degenerate_batch_is_clamped_and_still_steps() {
        for pop in [1usize, 3] {
            let mut t = trainer(pop, 0.05);
            assert_eq!(t.config().batch(), 2, "pop {pop} must clamp to one mirrored pair");
            let row = t.step_with(|batch| vec![1.0; batch.len()]);
            assert!(row.max_fitness.is_finite() && row.mean_fitness.is_finite());
        }
        let clamped = EsConfig {
            pop_size: 0,
            sigma: 0.0,
            learning_rate: -1.0,
            trials: 0,
            action_repeat: 0,
            ..EsConfig::default()
        }
        .sanitized();
        assert!(clamped.pop_size >= 2 && clamped.sigma > 0.0 && clamped.learning_rate > 0.0);
        assert!(clamped.trials >= 1 && clamped.action_repeat >= 1);

        // A flat objective leaves the ranks meaningless but not empty, so the rule still takes a
        // (noisy, tiny) step — what must not happen is a NaN or an infinity reaching the genome
        // from a zero total weight, which is the failure mode of normalising that sum.
        let mut flat = trainer(8, 0.5);
        flat.step_with(|batch| vec![0.0; batch.len()]);
        assert!(
            genes(flat.mean()).iter().all(|g| g.is_finite()),
            "a flat objective must not poison the mean"
        );
    }
}
