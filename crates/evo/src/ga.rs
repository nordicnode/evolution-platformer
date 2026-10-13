//! The genetic algorithm (§7.5): tournament selection, elitism, uniform
//! crossover, Gaussian mutation, and rayon-parallel population evaluation.
//!
//! ## Parallel determinism (Pattern A, §5.6)
//!
//! RNG construction happens *inside* the `par_iter().map()` closure and is seeded
//! per genome from `(master_seed, generation, genome_id, trial)`. `collect()`
//! preserves order, so `results[i]` always describes `genomes[i]` whatever order
//! the threads finish in. Selection and breeding then run single-threaded over
//! that ordered vector, so the RNG stream they consume does not depend on thread
//! count at all. On `wasm32` without threads, rayon falls back to sequential
//! iteration and the results are identical.

// `Rng as _` brings the trait into scope for method resolution on `sim::Rng`
// (the type alias), without shadowing the alias name.
use rand_xoshiro::rand_core::Rng as _;
use rayon::prelude::*;
use sim::rng::{episode_seed, next_f32, rng_from_seed, Rng};
use sim::Level;

use crate::fitness::{self, EpisodeResult, EvalConfig};
use crate::genome::{crossover, default_shape, mutate, Genome, NetShape};

/// GA hyperparameters (§7.5). Populated with the plan's v1 values.
#[derive(Clone, Debug)]
pub struct GaConfig {
    /// 200 native; the plan suggests 64–128 on the single-threaded web build.
    pub pop_size: usize,
    /// Tournament size, 3–5.
    pub tournament: usize,
    /// Top genomes carried over unchanged.
    pub elite: usize,
    /// Gaussian mutation standard deviation, 0.2–0.5.
    pub mutation_sigma: f32,
    /// Per-gene mutation probability, 0.03–0.10.
    pub mutation_rate: f32,
    /// Probability that a child is a crossover rather than a copy of one parent.
    pub crossover_rate: f32,
    /// Episodes per genome, averaged. 1 for MVP.
    pub trials: u32,
    /// Root seed for the whole run.
    pub master_seed: u64,
    pub shape: NetShape,
    pub action_repeat: u32,
}

impl Default for GaConfig {
    fn default() -> Self {
        GaConfig {
            pop_size: 200,
            tournament: 4,
            elite: 4,
            mutation_sigma: 0.30,
            mutation_rate: 0.05,
            crossover_rate: 0.75,
            trials: 1,
            master_seed: 1,
            shape: default_shape(),
            action_repeat: fitness::ACTION_REPEAT,
        }
    }
}

impl GaConfig {
    pub fn eval_config(&self) -> EvalConfig {
        EvalConfig {
            action_repeat: self.action_repeat,
        }
    }

    /// Clamp every field into the range the plan documents, so a stray CLI
    /// argument cannot produce a degenerate run (zero population, no elites,
    /// tournament larger than the population, and so on).
    pub fn sanitized(mut self, pop_override: Option<usize>) -> GaConfig {
        if let Some(p) = pop_override {
            self.pop_size = p;
        }
        self.pop_size = self.pop_size.max(2);
        self.tournament = self.tournament.clamp(2, self.pop_size);
        self.elite = self.elite.min(self.pop_size.saturating_sub(1));
        self.mutation_sigma = self.mutation_sigma.clamp(0.0, 2.0);
        self.mutation_rate = self.mutation_rate.clamp(0.0, 1.0);
        self.crossover_rate = self.crossover_rate.clamp(0.0, 1.0);
        self.trials = self.trials.max(1);
        self.action_repeat = self.action_repeat.clamp(1, 16);
        self
    }
}

/// One row of the fitness chart (§10 Tier 1.2) and of the CSV export (§10 Tier 2.10).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenRow {
    pub generation: u32,
    pub max_fitness: f32,
    pub avg_fitness: f32,
    pub min_fitness: f32,
    /// Agents that touched the flag this generation.
    pub wins: usize,
    /// Genome id of the generation's best agent.
    pub best_id: u32,
    /// `max > 1.5 × previous max` — the "genius agent" splash condition (§10 Tier 1.3).
    pub genius: bool,
}

/// The best genome seen so far, with the exact episode that earned it.
///
/// Keeping the `EpisodeResult` alongside the genome is what makes a saved
/// champion honest: its metadata is copied from a real evaluation rather than
/// re-derived and hoped for.
#[derive(Clone, Debug)]
pub struct BestEver {
    /// Generation in which this genome was evaluated.
    pub generation: u32,
    pub result: EpisodeResult,
    pub genome: Genome,
}

/// Everything one generation produced, including the per-genome results so the
/// watch mode can draw dead agents' positions (§10 Tier 2.6).
#[derive(Clone, Debug)]
pub struct GenerationReport {
    pub generation: u32,
    pub max_fitness: f32,
    pub avg_fitness: f32,
    pub min_fitness: f32,
    pub wins: usize,
    pub genius: bool,
    /// The generation's best genome, already scored (so it has a champion-ready
    /// fitness).
    pub best: Genome,
    /// Index-aligned with the population that was evaluated.
    pub results: Vec<EpisodeResult>,
}

/// Evaluate a whole population in parallel (Pattern A, §5.6).
pub fn evaluate_population(
    level: &Level,
    genomes: &[Genome],
    cfg: &GaConfig,
    generation: u32,
) -> Vec<EpisodeResult> {
    let eval = cfg.eval_config();
    let trials = cfg.trials.max(1);
    let master_seed = cfg.master_seed;

    genomes
        .par_iter()
        .map(|g| {
            // RNG and sim are constructed inside the closure: never shared.
            let mut best: Option<EpisodeResult> = None;
            let mut sum = 0.0f32;
            for trial in 0..trials {
                let seed = episode_seed(master_seed, generation, g.id, trial);
                let r = fitness::run_episode(level, g, &eval, seed);
                sum += r.fitness;
                if best.as_ref().is_none_or(|b| r.fitness > b.fitness) {
                    best = Some(r);
                }
            }
            let mut result = best.expect("trials >= 1");
            if trials > 1 {
                result.fitness = sum / trials as f32;
            }
            result
        })
        .collect()
}

/// The trainer: owns the population and the breeding RNG.
#[derive(Clone, Debug)]
pub struct Trainer {
    pub config: GaConfig,
    pub genomes: Vec<Genome>,
    pub generation: u32,
    rng: Rng,
    next_id: u32,
    last_max: f32,
    best_ever: Option<BestEver>,
    history: Vec<GenRow>,
}

impl Trainer {
    /// Start a fresh run from a random population.
    pub fn new(config: GaConfig) -> Trainer {
        let config = config.sanitized(None);
        let mut rng = rng_from_seed(config.master_seed);
        let shape = config.shape.clone();
        let genomes: Vec<Genome> = (0..config.pop_size)
            .map(|i| Genome::random(i as u32, shape.clone(), &mut rng))
            .collect();
        Trainer {
            next_id: config.pop_size as u32,
            config,
            genomes,
            generation: 0,
            rng,
            last_max: f32::NEG_INFINITY,
            best_ever: None,
            history: Vec::new(),
        }
    }

    /// Resume from a known population, e.g. a checkpoint or a champion plus noise.
    ///
    /// `next_id` continues past the highest id in `genomes`, so genome ids stay
    /// unique across a resumed run (they are part of every `episode_seed`).
    pub fn resume(
        config: GaConfig,
        genomes: Vec<Genome>,
        generation: u32,
    ) -> Result<Trainer, String> {
        if genomes.is_empty() {
            return Err("cannot resume from an empty population".into());
        }
        let config = config.sanitized(Some(genomes.len()));
        for g in &genomes {
            g.validate()?;
        }
        let next_id = genomes.iter().map(|g| g.id).max().unwrap_or(0) + 1;
        let mut rng = rng_from_seed(config.master_seed ^ 0x5DEE_CE66_D1CE_4B1D);
        // Keep the resumed population in order, then pad to pop_size if the
        // caller handed us fewer genomes than the config asks for.
        let mut genomes = genomes;
        let shape = config.shape.clone();
        while genomes.len() < config.pop_size {
            let id = next_id + genomes.len() as u32;
            genomes.push(Genome::random(id, shape.clone(), &mut rng));
        }
        let next_id = genomes.iter().map(|g| g.id).max().unwrap_or(0) + 1;
        Ok(Trainer {
            config,
            genomes,
            generation,
            rng,
            next_id,
            last_max: f32::NEG_INFINITY,
            best_ever: None,
            history: Vec::new(),
        })
    }

    pub fn history(&self) -> &[GenRow] {
        &self.history
    }

    /// Best genome seen so far, with its winning episode. `None` until the first
    /// [`Trainer::step`].
    pub fn best_ever(&self) -> Option<&BestEver> {
        self.best_ever.as_ref()
    }

    pub fn last_max(&self) -> f32 {
        self.last_max
    }

    /// Next genome id that will be handed out. Exposed so a test can assert that
    /// every id ever issued was distinct on first sight.
    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Evaluate the current population and produce the next generation.
    pub fn step(&mut self, level: &Level) -> GenerationReport {
        let results = evaluate_population(level, &self.genomes, &self.config, self.generation);

        debug_assert_eq!(results.len(), self.genomes.len());
        for (g, r) in self.genomes.iter_mut().zip(results.iter()) {
            g.fitness = r.fitness;
        }

        let max_fitness = results
            .iter()
            .map(|r| r.fitness)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_fitness = results
            .iter()
            .map(|r| r.fitness)
            .fold(f32::INFINITY, f32::min);
        let avg_fitness = results.iter().map(|r| r.fitness).sum::<f32>() / results.len() as f32;
        let wins = results.iter().filter(|r| r.won()).count();

        // "Genius agent": the maximum jumped by more than 50 % (§10 Tier 1.3). The
        // `last_max > 0` guard keeps the splash from firing while the population
        // is still climbing out of negative fitness, when a 1.5× ratio is noise.
        let genius = self.last_max > 0.0 && max_fitness > 1.5 * self.last_max;

        // Ranking: descending fitness, ties broken by lower index. `total_cmp`
        // gives a total order, so this cannot panic on a NaN the way
        // `partial_cmp().unwrap()` would, and it is stable across platforms.
        let mut order: Vec<usize> = (0..results.len()).collect();
        order.sort_by(|&a, &b| {
            results[b]
                .fitness
                .total_cmp(&results[a].fitness)
                .then(a.cmp(&b))
        });
        let winner = order[0];
        let mut best = self.genomes[winner].clone();

        let best_result = results[winner].clone();
        if self
            .best_ever
            .as_ref()
            .is_none_or(|b| best_result.fitness > b.result.fitness)
        {
            self.best_ever = Some(BestEver {
                generation: self.generation,
                result: best_result,
                genome: best.clone(),
            });
        }

        let row = GenRow {
            generation: self.generation,
            max_fitness,
            avg_fitness,
            min_fitness,
            wins,
            best_id: best.id,
            genius,
        };
        self.history.push(row);

        self.breed(&order);
        self.generation += 1;
        self.last_max = max_fitness;
        best.fitness = max_fitness;

        GenerationReport {
            generation: row.generation,
            max_fitness,
            avg_fitness,
            min_fitness,
            wins,
            genius,
            best,
            results,
        }
    }

    /// Build the next population in place. Single-threaded on purpose: the RNG
    /// stream must not depend on thread scheduling (§5.6).
    fn breed(&mut self, ranking: &[usize]) {
        // Copy the knobs out first: `self.tournament` takes `&mut self`, so a
        // long-lived `&self.config` borrow would not survive the loop.
        let pop_size = self.config.pop_size;
        let elite = self.config.elite;
        let crossover_rate = self.config.crossover_rate;
        let sigma = self.config.mutation_sigma;
        let rate = self.config.mutation_rate;
        let shape = self.config.shape.clone();

        let mut next: Vec<Genome> = Vec::with_capacity(pop_size);

        // Elitism: exact copies, original ids preserved so their episode seeds
        // (and therefore their fitness) will be identical next generation.
        for &i in ranking.iter().take(elite) {
            let mut g = self.genomes[i].clone();
            g.fitness = 0.0;
            next.push(g);
        }

        while next.len() < pop_size {
            let pa = self.tournament(ranking);
            let pb = self.tournament(ranking);
            let mut child = if next_f32(&mut self.rng) < crossover_rate {
                crossover(&self.genomes[pa], &self.genomes[pb], &mut self.rng)
            } else {
                self.genomes[pa].clone()
            };
            mutate(&mut child, sigma, rate, &mut self.rng);
            child.id = self.next_id;
            child.fitness = 0.0;
            child.shape = shape.clone();
            self.next_id += 1;
            next.push(child);
        }

        self.genomes = next;
    }

    /// Tournament selection over the *ranked* indices.
    ///
    /// Contenders are drawn from `ranking` but compared by fitness through their
    /// original index, so the returned index is always a valid population slot.
    fn tournament(&mut self, ranking: &[usize]) -> usize {
        let mut best = usize::MAX;
        for _ in 0..self.config.tournament {
            let pick = ranking[(self.rng.next_u32() as usize) % ranking.len()];
            if best == usize::MAX || self.genomes[pick].fitness > self.genomes[best].fitness {
                best = pick;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::hash::StateHasher;

    fn flat_level() -> Level {
        crate::fixture::flat_runway("ga-fixture", 24, 22)
    }

    /// A 24-tile runway with a three-tile pit at columns 10-12. A constant `run`
    /// strategy dies in the pit, so clearing it requires the population to learn
    /// *when* to jump. This is the fixture that can prove something about
    /// evolution; a flat runway cannot, because it is solved by a constant policy.
    fn pit_level() -> Level {
        crate::fixture::gap_runway("ga-pit-fixture", 24, 22, &[(10, 3)])
    }

    #[test]
    fn a_random_population_does_not_already_win_a_level_with_a_pit() {
        // Anti-vacuity guard for the smoke test: if generation 0 already won,
        // "the GA solves the level" would prove nothing about evolution.
        let cfg = GaConfig {
            pop_size: 24,
            master_seed: 5,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg);
        let r = t.step(&pit_level());
        assert_eq!(r.results.len(), 24);
        assert_eq!(r.wins, 0, "generation 0 must not already be winning");
        assert!(
            r.max_fitness < crate::fitness::GOAL_BONUS,
            "no genome at generation 0 may bank the goal bonus: max {}",
            r.max_fitness
        );
    }

    #[test]
    fn evaluation_is_order_preserving_and_index_aligned() {
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 8,
            master_seed: 3,
            ..GaConfig::default()
        };
        let t = Trainer::new(cfg.clone());
        let results = evaluate_population(&level, &t.genomes, &cfg, 0);
        assert_eq!(results.len(), t.genomes.len());
        for (i, r) in results.iter().enumerate() {
            assert_eq!(r.genome_id, t.genomes[i].id, "row {i} is misaligned");
        }
    }

    #[test]
    fn parallel_determinism_one_thread_vs_many_is_identical() {
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 32,
            master_seed: 17,
            ..GaConfig::default()
        };
        let t = Trainer::new(cfg.clone());

        let pool1 = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("1-thread pool");
        let pool6 = rayon::ThreadPoolBuilder::new()
            .num_threads(6)
            .build()
            .expect("6-thread pool");

        let one = pool1.install(|| evaluate_population(&level, &t.genomes, &cfg, 0));
        let six = pool6.install(|| evaluate_population(&level, &t.genomes, &cfg, 0));
        assert_eq!(one.len(), six.len());
        for i in 0..one.len() {
            assert_eq!(
                one[i].fitness, six[i].fitness,
                "fitness row {i} differs between 1 and 6 threads"
            );
            assert_eq!(one[i].trajectory_hash, six[i].trajectory_hash, "row {i}");
        }
    }

    #[test]
    fn the_ga_solves_the_flat_run_level_quickly() {
        // §11 GA smoke test: a loose bound that guards against fitness/sign bugs,
        // not a performance benchmark.
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 64,
            master_seed: 20261012,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg);
        let mut solved_at = None;
        for _ in 0..40 {
            let report = t.step(&level);
            if report.wins > 0 {
                solved_at = Some(report.generation);
                break;
            }
        }
        assert!(
            solved_at.is_some(),
            "flat-run level must be solved within 40 generations; history: {:?}",
            t.history()
                .iter()
                .map(|r| (r.generation, r.max_fitness))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn elite_genomes_survive_unchanged_and_keep_their_ids() {
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 12,
            elite: 3,
            master_seed: 8,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg.clone());
        let report = t.step(&level);
        let mut order: Vec<usize> = (0..report.results.len()).collect();
        order.sort_by(|&a, &b| {
            report.results[b]
                .fitness
                .total_cmp(&report.results[a].fitness)
                .then(a.cmp(&b))
        });
        for (slot, &ranked) in order.iter().take(cfg.elite).enumerate() {
            let original = &t.genomes[slot];
            let expected = &report.results[ranked];
            assert_eq!(original.id, expected.genome_id, "elite slot {slot}");
            assert_eq!(
                original.weights, t.genomes[slot].weights,
                "elites must be copied unchanged"
            );
        }
    }

    #[test]
    fn sanitize_clamps_degenerate_configs() {
        let c = GaConfig {
            pop_size: 0,
            tournament: 0,
            elite: 999,
            mutation_sigma: -1.0,
            mutation_rate: 5.0,
            action_repeat: 0,
            trials: 0,
            ..GaConfig::default()
        }
        .sanitized(None);
        assert!(c.pop_size >= 2);
        assert!(c.tournament >= 2 && c.tournament <= c.pop_size);
        assert!(c.elite < c.pop_size);
        assert_eq!(c.mutation_sigma, 0.0);
        assert_eq!(c.mutation_rate, 1.0);
        assert_eq!(c.action_repeat, 1);
        assert_eq!(c.trials, 1);
    }

    #[test]
    fn history_records_one_row_per_generation_in_order() {
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 10,
            master_seed: 2,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg);
        t.step(&level);
        t.step(&level);
        assert_eq!(t.history().len(), 2);
        assert_eq!(t.history()[0].generation, 0);
        assert_eq!(t.history()[1].generation, 1);
        assert!(t.best_ever().is_some());
    }

    #[test]
    fn genome_ids_are_unique_within_every_population() {
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 10,
            master_seed: 1,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg);
        let mut fresh_ids = std::collections::BTreeSet::new();

        let check = |genomes: &[Genome], fresh: &mut std::collections::BTreeSet<u32>, gen| {
            let mut in_population = std::collections::BTreeSet::new();
            for g in genomes {
                assert!(
                    in_population.insert(g.id),
                    "duplicate id {} inside generation {gen}",
                    g.id
                );
            }
            // Elites deliberately keep their id across generations, so only
            // *newly bred* ids are expected to be globally fresh. That matters:
            // the id is part of `episode_seed`, so a stable id is what makes an
            // elite's fitness identical next generation.
            for g in genomes {
                fresh.insert(g.id);
            }
        };

        check(&t.genomes, &mut fresh_ids, 0);
        for gen in 1..=3 {
            t.step(&level);
            check(&t.genomes, &mut fresh_ids, gen);
        }
        // Every id ever issued must have been distinct on first sight.
        assert_eq!(fresh_ids.len(), t.next_id() as usize);
    }

    #[test]
    fn carrying_a_genome_over_does_not_carry_its_fitness_over() {
        // `episode_seed` mixes in the generation by design (plan §5.5), so an
        // unchanged elite is re-tested against a fresh episode rather than
        // replaying the one that earned it a slot. That is what stops a lineage
        // from overfitting to a single lucky seed -- and it means `max_fitness`
        // is not required to be monotonic even with elitism.
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 8,
            elite: 4,
            master_seed: 11,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg);
        let first = t.step(&level);
        let elite_ids: Vec<u32> = t.genomes.iter().take(4).map(|g| g.id).collect();
        let second = t.step(&level);

        let mut compared = 0;
        for id in elite_ids {
            if let (Some(x), Some(y)) = (
                first.results.iter().find(|r| r.genome_id == id),
                second.results.iter().find(|r| r.genome_id == id),
            ) {
                assert_ne!(
                    x.seed, y.seed,
                    "genome {id} was re-evaluated with the same seed"
                );
                compared += 1;
            }
        }
        assert!(compared > 0, "no elite survived to be compared");
    }

    #[test]
    fn a_run_is_reproducible_from_the_master_seed() {
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 16,
            master_seed: 4242,
            ..GaConfig::default()
        };
        let mut a = Trainer::new(cfg.clone());
        let mut b = Trainer::new(cfg);
        for _ in 0..3 {
            let ra = a.step(&level);
            let rb = b.step(&level);
            assert_eq!(ra.max_fitness, rb.max_fitness);
            assert_eq!(ra.avg_fitness, rb.avg_fitness);
            assert_eq!(ra.best.weights, rb.best.weights);
        }
    }

    #[test]
    fn a_whole_populations_trajectory_hashes_are_stable_across_pool_sizes() {
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 16,
            master_seed: 77,
            ..GaConfig::default()
        };
        let t = Trainer::new(cfg.clone());
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .expect("pool");
        let a = evaluate_population(&level, &t.genomes, &cfg, 0);
        let b = pool.install(|| evaluate_population(&level, &t.genomes, &cfg, 0));
        let mut ha = StateHasher::new();
        let mut hb = StateHasher::new();
        for r in &a {
            ha.u64(r.trajectory_hash);
        }
        for r in &b {
            hb.u64(r.trajectory_hash);
        }
        assert_eq!(ha.finish(), hb.finish());
    }
}
