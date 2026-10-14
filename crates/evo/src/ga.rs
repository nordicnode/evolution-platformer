//! The genetic algorithm (§7.5): tournament selection, elitism, uniform
//! crossover, Gaussian mutation, and rayon-parallel population evaluation.
//!
//! ## The search is two-scale, restarts on a plateau, and can tune its own step
//!
//! A textbook GA is *one* move — a coarse Gaussian step over five percent of the genome —
//! applied to every child, at a size the operator chose before the run started. Two of the
//! three additions in [`Trainer::breed`] and its neighbourhood are **on by default**, because
//! they were measured against that textbook arm on the shipped pack and did not cost it
//! anything; the third is **off by default**, because it did.
//!
//! * **Fine children** (`fine_children`, default 8). Offspring per generation are
//!   perturbations of *one to three genes* of the generation's best genome, beside the coarse
//!   offspring a normal child gets (eighty-six genes at the default rate and a 1 723-gene
//!   genome). A coarse child of a nearly-good lineage is a fresh lottery ticket; a fine one is
//!   a step. On `scratch-tower` this alone solved seed 2 at generation 37 where the textbook
//!   arm needed 172.
//! * **A reheated restart on stagnation** (`stagnation_generations` 20, `immigrants` 8). After
//!   twenty generations without a new best, eight copies of the run's **best-ever genome** are
//!   bred in beside the elites, each with **every gene** moved by the run's own mutation size.
//!   A converged population does not recover by its ordinary step — every child of one strategy
//!   is a variation on the strategy that is stuck — and the first version of this bred in eight
//!   *random* genomes instead, which threw away everything the run had learned. Reheating from
//!   the champion is the same act of "go somewhere else" anchored at the one point known to be
//!   good: see [`crate::genome::reheat`].
//! * **A self-tuning mutation size** ([`GaConfig::adaptive_sigma`], off by default).
//!   Rechenberg's 1/5th rule: grow the step when more than a fifth of the children beat their
//!   parent, shrink it when fewer do. It is right about slopes and wrong about plateaus — see
//!   [`Trainer::tune_sigma`] for the three readings that were measured and what each of them
//!   did to a level whose whole difficulty is a plateau.
//!
//! All three are deterministic functions of the master seed and the population — the lineage
//! bookkeeping they need (`parent_fitness`) is parallel-safe because breeding runs
//! single-threaded over an ordered result vector (§5.6) — so a run is still reproducible,
//! which is what makes "this change helped" a claim anyone can check. The default
//! configuration's before/after against the textbook arm is on the record in `REVIEW.md`:
//! equal within a generation or two on the levels the pack solves in under twenty generations,
//! and on the hard level a lower median when it solves (37 and 101 against 161-172) on two
//! seeds of six, where the textbook arm solves three of six.
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
use crate::genome::{crossover, default_shape, mutate, mutate_few, reheat, Genome, NetShape};

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
    /// Offspring per generation bred as a *fine* perturbation of the generation's best
    /// genome: one to [`FINE_GENE_MAX`] genes, against the coarse
    /// `mutation_rate × n_genes` a normal child gets. 0 disables the second scale.
    pub fine_children: usize,
    /// Let the mutation size tune itself by the rule described in [`Trainer::tune_sigma`].
    ///
    /// **Off by default, and that is a measurement rather than a preference.** On
    /// `scratch-tower` — the hardest level in the pack — a fixed 0.30 solves three seeds out
    /// of six at generations 161-172, while every reading of the rule measured does worse:
    /// shrinking on a low success rate collapses to the 0.05 floor and never solves it
    /// (0/3), growing on a low rate spends the run at 1.0 and breaks the jump timing the
    /// level needs (1/3), and holding the size on a plateau still bleeds a generation's worth
    /// of 0.82 every time a new best appears and ends at the floor (0/3). The rule is right
    /// about slopes and wrong about plateaus, and this pack is mostly plateaus, so it is
    /// offered rather than applied. `false` freezes `mutation_sigma` at the value given.
    pub adaptive_sigma: bool,
    /// Generations without a new best before the population is given a shake. 0 disables.
    pub stagnation_generations: u32,
    /// How many immigrants arrive when that happens, as **reheated copies of the run's
    /// best-ever genome** — every gene moved by the reheat step — so the batch explores a
    /// shell around the best point found rather than restarting from noise.
    ///
    /// The name is the pre-existing one and is still accurate: these genomes are bred in from
    /// outside the population's own offspring. What changed is the source. Random immigrants
    /// throw away everything the run learned; a reheat keeps the champion's *shape* — the signs
    /// and relative magnitudes of 1 723 genes — and moves it a long way, which is the move a
    /// plateau actually needs. See [`crate::genome::reheat`] and [`GaConfig::reheat_sigma`].
    pub immigrants: usize,
    /// The reheat step, as a **multiple of the run's own mutation size**. 1.0 means the batch
    /// is thrown exactly as far as an ordinary mutation step, but at *every* gene instead of at
    /// five percent of them — about a twentyfold increase in displacement, which is the move a
    /// plateau needs and an ordinary child cannot make.
    ///
    /// **3.0 by default, from a sweep, not from taste.** 0.5, 1.0, 2.0 and 3.0 were each run
    /// over twelve seeds of `scratch-tower`, and 1.0 against 3.0 over twenty-four: at the shipped
    /// sigma of 0.30 that is a batch thrown 0.90 from the champion instead of 0.30, which solved
    /// 14 of 24 seeds against 10, and 9 of 12 against 6 on the first sample, spending less of the
    /// budget both times. The paired seeds went 5 gained for 1 lost. The sample cannot establish
    /// the size of that gain (the paired exact test is p ~ 0.2), and 0.5, 1.0 and 2.0 sit within
    /// noise of each other — what the sweep says is that a plateau wants a *hard* shake and that
    /// the exact gentle temperature does not matter. See `REVIEW.md` §4 for the tables.
    pub reheat_factor: f32,
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
            fine_children: 8,
            adaptive_sigma: false,
            stagnation_generations: 20,
            immigrants: 8,
            reheat_factor: 3.0,
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

    /// The Gaussian step a plateau batch is thrown at, in gene units.
    ///
    /// The run's own mutation size times [`GaConfig::reheat_factor`], held inside
    /// [`MUTATION_SIGMA_RANGE`]. The ceiling matters and is not a formality: at
    /// `GENE_CLAMP` 4.0 with Xavier weights near 0.15, a step of 1.0 or more essentially
    /// redraws every gene, so the clamp is what keeps the hottest reheat short of being the
    /// random restart this mechanism replaced — and it is why the default factor is 3.0 (0.90 at
    /// the shipped sigma) rather than 4.0, which saturates the ceiling and would make the factor
    /// unreadable as a knob.
    pub fn reheat_sigma(&self) -> f32 {
        (self.mutation_sigma * self.reheat_factor)
            .clamp(MUTATION_SIGMA_RANGE[0], MUTATION_SIGMA_RANGE[1])
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
        // Neither extra may swamp the population it is mixed into. Both are *nudges*:
        // a fine-children or immigrant quota above a quarter of the population is not
        // the same search with a smaller step, it is a different search arguing with
        // itself — and one of the two mechanisms exists precisely to stop that.
        let extra_cap = (self.pop_size / 4).max(1);
        self.fine_children = self.fine_children.min(extra_cap);
        self.immigrants = self.immigrants.min(extra_cap);
        // The reheat step is derived from sigma, so this bound is on the *multiple*: 8.0 with the
        // shipped sigma of 0.30 already asks for 2.4 and lands on the ceiling, and a negative
        // factor would mean a batch thrown *towards* the champion, which is not a search move.
        self.reheat_factor = self.reheat_factor.clamp(0.0, 8.0);
        // One generation is not stagnation, it is a generation; 0 disables the rule.
        if self.stagnation_generations == 1 {
            self.stagnation_generations = 2;
        }
        self.trials = self.trials.max(1);
        self.action_repeat = self.action_repeat.clamp(1, 16);
        self
    }
}

/// The widest fine-child step, in genes: `1..=FINE_GENE_MAX` of them per fine child.
///
/// Three rather than one so the fine half of the search is itself two-scale — a single
/// gene is the smallest step that can change anything, and a three-gene draw is the
/// largest that is still *local* beside the eighty-six a coarse child moves.
pub const FINE_GENE_MAX: usize = 3;

/// The range the self-tuning mutation size is kept inside, `[min, max]`.
///
/// 0.05 is a step small enough to polish and still large enough to matter against a
/// `GENE_CLAMP` of 4.0; 1.0 is large enough to rewire a policy and small enough that the
/// population has not simply become a random restart. The lower end is also where the
/// rule parks on a level that is already solved, which is what "converged" should look
/// like from the outside.
pub const MUTATION_SIGMA_RANGE: [f32; 2] = [0.05, 1.0];

/// The 1/5th rule's step: Rechenberg's 0.82.
///
/// Multiply the size by `1.0 / SIGMA_BACKOFF` when more than a fifth of the children
/// improve on their parent, by `SIGMA_BACKOFF` when fewer do. Geometric rather than
/// additive, so the size cannot cross zero or run away.
pub const SIGMA_BACKOFF: f32 = 0.82;

/// The success rate the 1/5th rule aims for. Below it the size shrinks; above it grows.
pub const SIGMA_TARGET_RATE: f32 = 0.2;

/// Marker for "this slot is not a child of a genome in this population", i.e. an elite
/// (a parent), an immigrant (no parent here) or the first generation of a run.
///
/// A sentinel rather than an `Option<f32>` per genome: it costs one `f32` a slot instead
/// of eight bytes and a branch, and the comparison it enables — `parent == NO_PARENT` — is
/// exact for the one value that can only be written here.
const NO_PARENT: f32 = f32::NEG_INFINITY;

/// The mutation-rate ladder the front-ends expose (§10 Tier 2.9's slider, inside §7.5's
/// documented 0.03–0.10).
///
/// It lives beside the hyperparameter it tunes rather than in each front-end, because
/// the terminal's `m`/`M` keys and the window's slider are the same control: two copies
/// of a ladder are two chances for the two views to disagree about which rates are even
/// reachable. A ladder rather than a continuous dial, so every value on screen is a
/// value the other front-end can get back to and no keypress can drive a run somewhere
/// §7.5 does not document. 0.05 is the trainer's default, so a fresh run starts in the
/// middle and both directions are available — which is the point of having a control at
/// all. The two reasons to reach for it are opposite: a population that has converged
/// onto an obstacle needs shaking *up* (finding 16's plateau), and one that keeps
/// destroying the genome that nearly solved the level needs calming *down*.
pub const MUTATION_RATES: [f32; 4] = [0.03, 0.05, 0.08, 0.10];

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
    /// The mutation size in force when these children were bred, rounded for a chart.
    ///
    /// Tracked because with [`GaConfig::adaptive_sigma`] it is not a constant of the run
    /// but a *reading*: watching it fall is how a viewer sees the search stop exploring
    /// and start polishing, and watching it rise is how they see a plateau being attacked.
    pub mutation_sigma: f32,
    /// Immigrants bred into the generation that follows this row.
    pub immigrants: usize,
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
    /// The mutation size the children of this generation were bred with (see [`GenRow`]).
    pub mutation_sigma: f32,
    /// Immigrants bred into the generation that follows this one.
    pub immigrants: usize,
    /// Index-aligned with the population that was evaluated.
    pub results: Vec<EpisodeResult>,
    /// Indices into `results`, best fitness first, ties broken by lower index.
    /// Stored rather than recomputed so breeding and lane selection cannot
    /// disagree about what "the best genomes" means.
    pub ranking: Vec<usize>,
}

/// Which half of the "best-4 + random-4" split a showcase lane came from (§10 Tier 1.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneKind {
    /// Top of the ranking: what the population can do at its best.
    Best,
    /// Drawn from the rest: what it is still exploring.
    Random,
}

/// One agent to draw in watch mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShowcaseLane {
    /// Index into the evaluated population (see [`Trainer::step_verbose`]).
    pub index: usize,
    /// Position in the showcase: 0 is the champion.
    pub rank: usize,
    pub genome_id: u32,
    pub kind: LaneKind,
    /// Fitness of the episode this lane will reproduce (trial 0).
    pub fitness: f32,
    /// Mean fitness over all trials, i.e. the metric selection used.
    pub mean_fitness: f32,
    /// The exact seed the evaluator used, so the lane replays faithfully rather
    /// than being a fresh episode with the same genome.
    pub seed: u64,
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
            let mut sum = 0.0f32;
            let mut representative: Option<EpisodeResult> = None;
            for trial in 0..trials {
                let seed = episode_seed(master_seed, generation, g.id, trial);
                let r = fitness::run_episode(level, g, &eval, seed);
                sum += r.fitness;
                // Trial 0 is the representative: it is the one anyone can
                // reproduce, so it is the one a champion records.
                if trial == 0 {
                    representative = Some(r);
                }
            }
            let mut result = representative.expect("trials >= 1");
            result.mean_fitness = sum / trials as f32;
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
    /// Per-slot fitness of the genome each slot was bred from, or [`NO_PARENT`] where that
    /// question does not apply. This is the 1/5th rule's whole memory: without it a
    /// generation cannot ask whether its children are beating their parents.
    parent_fitness: Vec<f32>,
    /// Generations since the best fitness last improved. Drives the immigrant rule.
    stagnation: u32,
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
            // No lineage on the first generation: every genome is a starting point, so the
            // rule has nothing to compare until this population has bred once.
            parent_fitness: vec![NO_PARENT; config.pop_size],
            config,
            genomes,
            generation: 0,
            rng,
            last_max: f32::NEG_INFINITY,
            best_ever: None,
            history: Vec::new(),
            stagnation: 0,
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
            // A resumed population's parents are not here to be read, so the first
            // generation after a resume does not steer the mutation size: the rule picks up
            // again from the first generation this trainer breeds itself.
            parent_fitness: vec![NO_PARENT; config.pop_size],
            config,
            genomes,
            generation,
            rng,
            next_id,
            last_max: f32::NEG_INFINITY,
            best_ever: None,
            history: Vec::new(),
            stagnation: 0,
        })
    }

    /// Replace the population with `genomes`, keeping everything the **run** owns.
    ///
    /// This is what a curriculum step returns through (§12 M5): the excursion trains
    /// a population on an easier level and hands it back, and the run it came from has
    /// to continue — the chart history it has drawn, the best-ever genome it will
    /// save as the champion, the genius baseline and the RNG stream all belong to the
    /// run, not to the population. [`Trainer::resume`] cannot do this, because it
    /// builds a *fresh* trainer and therefore forgets all of it.
    ///
    /// Genome ids are the one thing that cannot simply be swapped in: they are part
    /// of every `episode_seed`, so duplicates would make two members of the population
    /// share episodes, and a stale `next_id` would hand a new genome an id that is
    /// already in use. Both are checked here rather than trusted.
    pub fn adopt(&mut self, genomes: Vec<Genome>) -> Result<(), String> {
        if genomes.len() != self.config.pop_size {
            return Err(format!(
                "cannot adopt {} genomes into a population of {}",
                genomes.len(),
                self.config.pop_size
            ));
        }
        for g in &genomes {
            g.validate()?;
        }
        let mut ids: Vec<u32> = genomes.iter().map(|g| g.id).collect();
        ids.sort_unstable();
        if let Some(dup) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("adopted population has duplicate genome id {}", dup[0]));
        }
        if let Some(max) = ids.last() {
            // Ids at or above the current `next_id` are the excursion's own; ids
            // below it are the ones this run issued. Either way, nothing already in
            // use may be handed out again.
            self.next_id = self.next_id.max(max.saturating_add(1));
        }
        self.genomes = genomes;
        // An adopted population arrives without its parents (they belong to the excursion),
        // so the 1/5th rule starts over rather than comparing against a lineage that is not
        // in this run.
        self.parent_fitness = vec![NO_PARENT; self.config.pop_size];
        Ok(())
    }

    /// The mutation size children are currently bred with.
    ///
    /// With [`GaConfig::adaptive_sigma`] this is a reading rather than a setting, which is
    /// why a front-end wants it: it is the one number that says whether the population is
    /// exploring or polishing right now.
    pub fn mutation_sigma(&self) -> f32 {
        self.config.mutation_sigma
    }

    /// Generations since the best fitness last improved. At or past
    /// [`GaConfig::stagnation_generations`], reheated copies of the best-ever genome are
    /// being bred in.
    pub fn stagnation(&self) -> u32 {
        self.stagnation
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
        self.advance(level, false).0
    }

    /// As [`Trainer::step`], but also returns the population that was evaluated.
    ///
    /// Watch mode needs it: after `step` returns, `self.genomes` is already the
    /// *next* generation, so the genomes a [`GenerationReport`] describes are gone.
    /// The copy costs about 1.2 MB per generation at pop 200, which is why plain
    /// training does not pay for it.
    pub fn step_verbose(&mut self, level: &Level) -> (GenerationReport, Vec<Genome>) {
        self.advance(level, true)
    }

    fn advance(&mut self, level: &Level, keep_population: bool) -> (GenerationReport, Vec<Genome>) {
        let results = evaluate_population(level, &self.genomes, &self.config, self.generation);

        debug_assert_eq!(results.len(), self.genomes.len());
        for (g, r) in self.genomes.iter_mut().zip(results.iter()) {
            // The genome's own `fitness` is the metric selection uses. The
            // per-episode number lives in the result, so nothing here has to
            // guess which of the two it is holding.
            g.fitness = r.mean_fitness;
        }

        // Snapshot after scoring but before breeding replaces the population, so
        // the vector handed back really is "the population as evaluated" -- with
        // its scores on it. Only when asked: this copy is ~1.2 MB at pop 200, and
        // plain training never needs it.
        let pop = if keep_population {
            self.genomes.clone()
        } else {
            Vec::new()
        };

        let metric = |r: &EpisodeResult| r.mean_fitness;
        let max_fitness = results.iter().map(metric).fold(f32::NEG_INFINITY, f32::max);
        let min_fitness = results.iter().map(metric).fold(f32::INFINITY, f32::min);
        let avg_fitness = results.iter().map(metric).sum::<f32>() / results.len() as f32;
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
                .mean_fitness
                .total_cmp(&results[a].mean_fitness)
                .then(a.cmp(&b))
        });
        let winner = order[0];
        let mut best = self.genomes[winner].clone();

        let best_result = results[winner].clone();
        if self
            .best_ever
            .as_ref()
            .is_none_or(|b| best_result.mean_fitness > b.result.mean_fitness)
        {
            self.best_ever = Some(BestEver {
                generation: self.generation,
                result: best_result,
                genome: best.clone(),
            });
        }

        // ---- the search's own bookkeeping: stagnation, then the 1/5th rule -----
        if max_fitness > self.last_max {
            self.stagnation = 0;
        } else {
            self.stagnation = self.stagnation.saturating_add(1);
        }
        if self.config.adaptive_sigma {
            self.tune_sigma(&results);
        }

        let immigrants = self.breed(&order);

        let row = GenRow {
            generation: self.generation,
            max_fitness,
            avg_fitness,
            min_fitness,
            wins,
            best_id: best.id,
            genius,
            mutation_sigma: self.config.mutation_sigma,
            immigrants,
        };
        self.history.push(row);

        self.generation += 1;
        self.last_max = max_fitness;
        best.fitness = max_fitness;

        (
            GenerationReport {
                generation: row.generation,
                max_fitness,
                avg_fitness,
                min_fitness,
                wins,
                genius,
                best,
                mutation_sigma: row.mutation_sigma,
                immigrants: row.immigrants,
                results,
                ranking: order,
            },
            pop,
        )
    }

    /// One step of the size rule (see the module docs).
    ///
    /// The sample is the children whose parent this population still knows: an elite *is* a
    /// parent, an immigrant has no parent here, and a resumed or adopted population has no
    /// lineage at all until it has bred once — all three are skipped rather than guessed at.
    ///
    /// A child is compared against its parent's fitness *from the generation the parent was
    /// scored in*, which on a walker-free level is an exact comparison (a genome's episode is
    /// a function of its genes) and on a walker level carries one generation of episode
    /// noise. That is why the rule waits for a sample worth believing — a quarter of the
    /// population — before it moves: a single noisy comparison is not a trend.
    ///
    /// **The size moves only while the run is finding new bests, and is held while it is
    /// not.** Under Rechenberg's rule a low success rate means the step is overshooting the
    /// improvements that are still there, and shrinking is the correct response. On a plateau
    /// that reading is simply wrong: the population is sitting on a value it cannot leave, and
    /// a smaller step makes it less able to leave, not more. So the way down is taken only in
    /// productive stretches, and a run that has stopped improving keeps the step that got it
    /// there. That distinction is what stops the rule freezing a stalled lineage at the floor.
    ///
    /// **Measured, not assumed — and the measurements are why this is off by default.** On
    /// `scratch-tower`, whose entire difficulty is a plateau at 128.3, three readings were run
    /// against the same three seeds:
    ///
    /// * pure Rechenberg (shrink on a low rate, plateau or not) walked 0.30 to the 0.05 floor
    ///   in eight stagnant generations and then never solved the level at all (0/3);
    /// * growing on a plateau instead spent the run at the 1.0 ceiling, breaking the jump
    ///   timing the level needs (1/3);
    /// * holding the size on a plateau — what this function now does — still bleeds
    ///   0.82 per generation in the stretches where a best *is* found, and ended at 0.050 and
    ///   0.061 on the seeds it failed (0/3), where the flat-0.30 textbook arm solved all three
    ///   at generations 161-172.
    ///
    /// So the rule is offered rather than applied: [`GaConfig::adaptive_sigma`] is off by
    /// default, and the two mechanisms that *can* answer a plateau — the local step and the
    /// restart — are what answer it.
    fn tune_sigma(&mut self, results: &[EpisodeResult]) {
        let mut counted = 0usize;
        let mut improved = 0usize;
        for (i, r) in results.iter().enumerate() {
            let parent = self.parent_fitness[i];
            if parent == NO_PARENT {
                continue;
            }
            counted += 1;
            if r.mean_fitness > parent {
                improved += 1;
            }
        }
        if counted < (self.config.pop_size / 4).max(4) {
            return;
        }
        let rate = improved as f32 / counted as f32;
        // Held while the best is standing still, moved only in productive stretches: the
        // reasoning, and the three measured readings behind it, are on this function's docs.
        let factor = if self.stagnation > 0 {
            1.0
        } else if rate > SIGMA_TARGET_RATE {
            1.0 / SIGMA_BACKOFF
        } else {
            SIGMA_BACKOFF
        };
        self.config.mutation_sigma = (self.config.mutation_sigma * factor)
            .clamp(MUTATION_SIGMA_RANGE[0], MUTATION_SIGMA_RANGE[1]);
    }

    /// The lanes watch mode draws: the top `best_k` genomes of the generation just
    /// evaluated, plus `random_k` more drawn from everything else (§10 Tier 1.5).
    ///
    /// Best-plus-random rather than best-only on purpose: a population that has
    /// collapsed onto one strategy is exactly the failure the watch view has to
    /// make visible, and a chart cannot show it.
    ///
    /// The random draw uses a showcase RNG seeded from the generation, never the
    /// trainer's own stream, so choosing what to watch cannot change what is
    /// evolved (§5.6). `population` must be the vector [`Trainer::step_verbose`]
    /// returned alongside `report`.
    pub fn showcase(
        &self,
        report: &GenerationReport,
        population: &[Genome],
        best_k: usize,
        random_k: usize,
    ) -> Vec<ShowcaseLane> {
        debug_assert_eq!(report.ranking.len(), report.results.len());
        let mut lanes: Vec<ShowcaseLane> = Vec::with_capacity(best_k + random_k);
        let mut taken = vec![false; report.results.len()];

        for (rank, &index) in report.ranking.iter().take(best_k).enumerate() {
            taken[index] = true;
            lanes.push(lane(index, rank, LaneKind::Best, report, population));
        }

        let mut pool: Vec<usize> = report
            .ranking
            .iter()
            .copied()
            .filter(|i| !taken[*i])
            .collect();
        if random_k > 0 && !pool.is_empty() {
            let mut rng = rng_from_seed(episode_seed(
                self.config.master_seed,
                report.generation,
                u32::MAX,
                SHOWCASE_TRIAL,
            ));
            let draws = random_k.min(pool.len());
            for n in 0..draws {
                // `swap_remove` gives a uniform pick *without* replacement in one
                // pass, so the same genome cannot appear in two lanes.
                let pick = (rng.next_u32() as usize) % pool.len();
                let index = pool.swap_remove(pick);
                lanes.push(lane(index, best_k + n, LaneKind::Random, report, population));
            }
        }

        lanes
    }

    /// Build the next population in place, and report how many immigrants it took.
    ///
    /// Single-threaded on purpose: the RNG stream must not depend on thread scheduling
    /// (§5.6). Four kinds of slot, and the difference between them is written down in
    /// [`Trainer::parent_fitness`] because the 1/5th rule reads it:
    ///
    /// 1. **Elites** — exact copies, original ids preserved so their episode seeds (and
    ///    therefore their fitness) will be identical next generation. They are *parents*.
    /// 2. **Fine children** — one to [`FINE_GENE_MAX`] genes of the generation's best
    ///    genome perturbed. This is the second scale of the search: the coarse child below
    ///    moves ~86 genes, this one moves up to three, so a lineage that is nearly right is
    ///    improved rather than re-rolled.
    /// 3. **Immigrants** — reheats of the run's best-ever genome, only after
    ///    `stagnation_generations` without a new best. They are placed directly after the fine
    ///    children so that a population that has converged cannot quietly outvote them: the
    ///    elites and the fine children are already in front of them in the ranking tie-breaks.
    /// 4. **Coarse children** — the textbook crossover/mutation offspring, which fill
    ///    whatever is left. At least one slot is always reserved for them, so no
    ///    configuration of the two extras can turn this into a search with no crossover in
    ///    it at all.
    fn breed(&mut self, ranking: &[usize]) -> usize {
        // Copy the knobs out first: `self.tournament` takes `&mut self`, so a
        // long-lived `&self.config` borrow would not survive the loop.
        let pop_size = self.config.pop_size;
        let elite = self.config.elite.min(pop_size.saturating_sub(1));
        let crossover_rate = self.config.crossover_rate;
        let sigma = self.config.mutation_sigma;
        let rate = self.config.mutation_rate;
        let shape = self.config.shape.clone();

        let mut next: Vec<Genome> = Vec::with_capacity(pop_size);
        let mut parents: Vec<f32> = Vec::with_capacity(pop_size);

        // 1. Elitism.
        for &i in ranking.iter().take(elite) {
            let mut g = self.genomes[i].clone();
            g.fitness = 0.0;
            next.push(g);
            parents.push(NO_PARENT);
        }

        // 2. Fine children of the generation's best, budgeted so the coarse children below
        // always get at least one slot.
        let mut budget = pop_size - next.len();
        let fine = self.config.fine_children.min(budget.saturating_sub(1));
        if fine > 0 {
            let best_slot = ranking[0];
            let best_fitness = self.genomes[best_slot].fitness;
            for _ in 0..fine {
                let mut child = self.genomes[best_slot].clone();
                let genes = 1 + (next_f32(&mut self.rng) * FINE_GENE_MAX as f32) as usize;
                mutate_few(&mut child, genes, sigma, &mut self.rng);
                child.id = self.next_id;
                child.fitness = 0.0;
                child.shape = shape.clone();
                self.next_id += 1;
                next.push(child);
                parents.push(best_fitness);
            }
        }
        budget -= fine;

        // 3. Immigrants, only when the run has stopped moving — and reheated copies of the
        //    best genome the run has found, not fresh random ones.
        let stagnated = self.config.stagnation_generations > 0
            && self.stagnation >= self.config.stagnation_generations;
        let immigrants = if stagnated {
            self.config.immigrants.min(budget.saturating_sub(1))
        } else {
            0
        };
        if immigrants > 0 {
            // The anchor is the run's best-ever genome — the one point in the whole run that is
            // known to be good — falling back to this generation's best if there is no record
            // yet. Breeding only ever happens after a generation has been scored, so the
            // fallback is a belt rather than a path anything takes.
            let anchor = self
                .best_ever
                .as_ref()
                .map(|b| b.genome.clone())
                .unwrap_or_else(|| self.genomes[ranking[0]].clone());
            // The reheat step is **the run's own mutation size, applied to every gene** rather
            // than to five percent of them, scaled by `reheat_factor`. At the default rate that
            // is about a twentyfold increase in how far the batch lands from the anchor, while
            // the anchor's shape survives — a one-sigma step on 1 723 genes perturbs each by its
            // own magnitude instead of replacing it. The factor is swept in `REVIEW.md` §4.
            let reheat_sigma = self.config.reheat_sigma();
            for _ in 0..immigrants {
                let mut child = reheat(&anchor, reheat_sigma, &mut self.rng);
                child.id = self.next_id;
                child.shape = shape.clone();
                self.next_id += 1;
                next.push(child);
                // No parent *in this population*: the anchor is usually an ancestor from an
                // earlier generation, so a child-vs-parent comparison against it would be
                // mixing generations — and the rule is trying to measure exactly that noise.
                parents.push(NO_PARENT);
            }
            // The counter restarts with the batch, so the population gets the whole
            // `stagnation_generations` to do something with it before more arrives: a
            // population flooded with reheats every generation is just a random restart.
            self.stagnation = 0;
        }

        // 4. Coarse children fill the rest.
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
            // The child's baseline is the mean of the two parents *this generation*, which
            // is what makes the 1/5th rule's comparison a statement about the child's own
            // genes rather than about how the level happens to be scoring today.
            parents.push((self.genomes[pa].fitness + self.genomes[pb].fitness) * 0.5);
        }

        self.genomes = next;
        self.parent_fitness = parents;
        immigrants
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

/// Seed salt used for the showcase RNG, kept far from any real trial index so the
/// lane draw can never collide with an evaluated episode.
const SHOWCASE_TRIAL: u32 = 0x5EED;

fn lane(
    index: usize,
    rank: usize,
    kind: LaneKind,
    report: &GenerationReport,
    population: &[Genome],
) -> ShowcaseLane {
    let r = &report.results[index];
    debug_assert_eq!(population[index].id, r.genome_id);
    ShowcaseLane {
        index,
        rank,
        genome_id: r.genome_id,
        kind,
        fitness: r.fitness,
        mean_fitness: r.mean_fitness,
        seed: r.seed,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use sim::hash::StateHasher;

    /// The ladder is the range §7.5 documents, and the trainer's own default is a rung,
    /// so a fresh run starts *on* the control rather than beside it.
    #[test]
    fn the_mutation_ladder_is_the_documented_range() {
        let default = GaConfig::default().mutation_rate;
        assert!(
            MUTATION_RATES.contains(&default),
            "the default rate must be one of the rungs: {MUTATION_RATES:?}"
        );
        for pair in MUTATION_RATES.windows(2) {
            assert!(pair[0] < pair[1], "the ladder must rise: {MUTATION_RATES:?}");
        }
        for rate in MUTATION_RATES {
            assert!((0.03..=0.10).contains(&rate), "{rate} is outside §7.5");
        }
    }

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
    fn adopting_a_population_changes_the_run_without_resetting_it() {
        // What a curriculum excursion depends on: the population comes back changed,
        // and everything the *run* owns — the chart it has drawn, the champion it
        // will save, the ids it must never reissue — survives the swap.
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 16,
            master_seed: 11,
            ..GaConfig::default()
        };
        let mut run = Trainer::new(cfg.clone());
        for _ in 0..3 {
            run.step(&level);
        }
        let history_before = run.history().to_vec();
        let best_before = run
            .best_ever()
            .map(|b| (b.generation, b.result.fitness, b.genome.id));
        let next_id_before = run.next_id();

        // Stand in for the excursion: someone else's population, with ids of its own.
        let mut excursion = Trainer::resume(cfg.clone(), run.genomes.clone(), run.generation)
            .expect("resumes the population");
        for _ in 0..2 {
            excursion.step(&level);
        }
        let adopted = excursion.genomes.clone();
        assert_ne!(
            adopted.iter().map(|g| g.id).collect::<Vec<_>>(),
            run.genomes.iter().map(|g| g.id).collect::<Vec<_>>(),
            "a generation of evolution must have happened in the excursion"
        );
        run.adopt(adopted).expect("adopts a full population");

        assert_eq!(
            run.history(),
            history_before.as_slice(),
            "the chart belongs to the run, not to the population"
        );
        assert_eq!(
            run.best_ever().map(|b| (
                b.generation,
                b.result.fitness,
                b.genome.id
            )),
            best_before,
            "the champion is the best of *this* level's evaluations"
        );
        assert!(
            run.next_id() >= next_id_before,
            "ids already issued may never be handed out again"
        );

        // And the adopted population is the one that breeds from here.
        let report = run.step(&level);
        assert_eq!(report.results.len(), cfg.pop_size);
        assert_eq!(run.history().len(), history_before.len() + 1);
    }

    #[test]
    fn adopt_refuses_a_population_that_would_break_the_run() {
        let cfg = GaConfig {
            pop_size: 4,
            master_seed: 2,
            ..GaConfig::default()
        };
        let mut run = Trainer::new(cfg);
        let ok = run.genomes.clone();

        let mut short = ok.clone();
        short.pop();
        assert!(run.adopt(short).unwrap_err().contains("cannot adopt 3 genomes"));

        let mut dupes = ok.clone();
        dupes[2].id = dupes[0].id;
        assert!(run
            .adopt(dupes)
            .unwrap_err()
            .contains("duplicate genome id"));

        let mut broken = ok.clone();
        broken[1].weights.clear();
        assert!(run.adopt(broken).is_err(), "a malformed genome is refused");

        // Every refusal left the population it already had.
        run.adopt(ok.clone()).expect("the original still fits");
        assert_eq!(run.genomes.len(), 4);
        assert_eq!(run.genomes.iter().map(|g| g.id).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_random_population_does_not_already_win_a_level_with_a_pit() {
        // Anti-vacuity guard for the smoke test: if generation 0 already solved the level,
        // "the GA solves the level" would prove nothing about evolution.
        //
        // Swept over twelve seeds rather than checked on one, and that is a fix rather than
        // tidying. A three-tile pit is *inside* a running jump, so a randomly wired policy
        // clears it by luck now and then, and a single frozen seed reads as "never" for a
        // level where it happens. Measured at 0.5.0, winners per 24 genomes across seeds
        // 1..12: 1, 1, 0, 0, 1, 1, 0, 2, 0, 1, 2, 1 — never more than a twelfth of the
        // population. What no seed shows is a population that has *solved* the level: a
        // genome that does not cross the pit banks about 155 of the 10 337 a winner scores,
        // because on this fixture there is no partial credit past the pit.
        for seed in 1..=12u64 {
            let cfg = GaConfig {
                pop_size: 24,
                master_seed: seed,
                ..GaConfig::default()
            };
            let mut t = Trainer::new(cfg);
            let r = t.step(&pit_level());
            assert_eq!(r.results.len(), 24);
            assert!(
                r.wins <= 2,
                "seed {seed}: {} of 24 random genomes crossed the pit unaided, which is no \
                 longer a lucky genome — this fixture is meant to need evolution",
                r.wins
            );
            let best_loser = r
                .results
                .iter()
                .filter(|x| !x.won())
                .map(|x| x.fitness)
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                best_loser < crate::fitness::GOAL_BONUS / 10.0,
                "seed {seed}: the best genome that did not reach the flag banked {best_loser}, \
                 which is not \"nowhere near solving it\""
            );
        }
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
    fn watching_does_not_change_what_is_evolved() {
        // Watch mode calls `showcase` between generations. That draw uses its own
        // RNG seed, so it must be invisible to the GA: if it consumed the trainer's
        // stream, every watched run would evolve differently from the same unwatched
        // run, and the whole determinism contract would depend on whether a human
        // was looking (§5.6).
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 16,
            master_seed: 1234,
            ..GaConfig::default()
        };

        let mut plain = Trainer::new(cfg.clone());
        let mut plain_history = Vec::new();
        for _ in 0..4 {
            plain_history.push(plain.step(&level).max_fitness);
        }

        let mut watched = Trainer::new(cfg);
        let mut watched_history = Vec::new();
        for _ in 0..4 {
            let (report, pop) = watched.step_verbose(&level);
            let lanes = watched.showcase(&report, &pop, 4, 4);
            assert_eq!(lanes.len(), 8, "best-4 + random-4");
            watched_history.push(report.max_fitness);
        }

        assert_eq!(
            plain_history, watched_history,
            "choosing lanes to watch perturbed the GA's RNG stream"
        );
        assert_eq!(plain.next_id(), watched.next_id(), "breeding diverged");
    }

    #[test]
    fn every_showcase_lane_replays_the_episode_that_was_evaluated() {
        // The fidelity guarantee watch mode rests on: a lane is the *same* episode
        // the trainer scored, not a fresh episode on the same genome. Otherwise the
        // animation and the fitness column would be describing different runs.
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 16,
            master_seed: 99,
            ..GaConfig::default()
        };
        let eval = cfg.eval_config();
        let mut t = Trainer::new(cfg);
        let (report, population) = t.step_verbose(&level);
        let lanes = t.showcase(&report, &population, 2, 2);
        assert_eq!(lanes.len(), 4);

        for lane in &lanes {
            let genome = &population[lane.index];
            assert_eq!(genome.id, lane.genome_id, "lane points at the wrong genome");
            let replayed = crate::fitness::run_episode(&level, genome, &eval, lane.seed);
            assert_eq!(
                replayed.trajectory_hash, report.results[lane.index].trajectory_hash,
                "lane {} ({:?}) is not the episode that was scored",
                lane.genome_id, lane.kind
            );
            assert_eq!(replayed.fitness, lane.fitness);
            assert_eq!(replayed.outcome, report.results[lane.index].outcome);
        }

        // No genome may appear twice: the champion and a random pick colliding
        // would silently show one agent in two lanes.
        let ids: std::collections::BTreeSet<u32> = lanes.iter().map(|l| l.genome_id).collect();
        assert_eq!(ids.len(), lanes.len(), "lanes are not distinct: {lanes:?}");

        // The split is what the plan asks for: the top of the ranking, plus
        // outsiders. The champion lane must be the ranking's first entry.
        assert_eq!(lanes[0].genome_id, report.results[report.ranking[0]].genome_id);
        assert!(lanes.iter().take(2).all(|l| l.kind == LaneKind::Best));
        assert!(lanes.iter().skip(2).all(|l| l.kind == LaneKind::Random));
    }

    #[test]
    fn showcase_asks_for_more_random_lanes_than_exist_without_duplicating() {
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 6,
            master_seed: 4,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg);
        let (report, population) = t.step_verbose(&level);
        // 6 genomes, 4 best + 99 random: only 2 outsiders exist.
        let lanes = t.showcase(&report, &population, 4, 99);
        assert_eq!(lanes.len(), 6, "cannot invent genomes that do not exist");
        let ids: std::collections::BTreeSet<u32> = lanes.iter().map(|l| l.genome_id).collect();
        assert_eq!(ids.len(), 6);
    }

    #[test]
    fn plain_step_is_the_same_run_without_the_population_copy() {
        // `step` and `step_verbose` differ only in whether they hand back the
        // population they evaluated. Plain training runs ~200 generations a second
        // and must not pay ~1.2 MB of copying per generation for a snapshot it
        // discards, but any drift between the two paths would mean the numbers
        // watch mode shows are not the numbers training used.
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 8,
            master_seed: 6,
            ..GaConfig::default()
        };
        let mut plain = Trainer::new(cfg.clone());
        let mut verbose = Trainer::new(cfg);
        for generation in 0..3 {
            let a = plain.step(&level);
            let (b, population) = verbose.step_verbose(&level);

            assert_eq!(a.generation, b.generation);
            assert_eq!(a.max_fitness, b.max_fitness, "generation {generation}");
            assert_eq!(a.avg_fitness, b.avg_fitness);
            assert_eq!(a.wins, b.wins);
            assert_eq!(a.ranking, b.ranking);
            assert_eq!(a.best.id, b.best.id);

            // The snapshot is the population *as evaluated*: ids in population order,
            // carrying the very fitness this generation was scored with.
            assert_eq!(population.len(), 8);
            for (i, genome) in population.iter().enumerate() {
                assert_eq!(genome.fitness, b.results[i].mean_fitness, "slot {i}");
            }
            // The live population has already been bred, so its scores are reset --
            // which is exactly why the snapshot has to exist for watch mode.
            assert!(plain.genomes.iter().all(|g| g.fitness == 0.0));
        }
    }

    #[test]
    fn trials_only_matter_on_a_level_that_is_stochastic() {
        // `--trials` averages N episodes. A walker's patrol phase is the only thing
        // in the sim drawn from the episode RNG (§5), so on a walker-free level
        // every trial is the *same* episode: the mean must equal the representative
        // exactly, and paying for N copies of one number buys nothing. On a level
        // with a walker the trials genuinely differ, which is the only reason the
        // flag exists. (This test previously used L3 and asserted the opposite,
        // wrongly: random genomes die in L3's first pit, twenty tiles short of its
        // walker, so their trials were identical for a reason that had nothing to do
        // with the walker.)
        let cfg = GaConfig {
            pop_size: 8,
            trials: 3,
            master_seed: 11,
            ..GaConfig::default()
        };

        let flat = crate::fixture::flat_runway("trials-flat", 24, 22);
        let (flat_report, _) = Trainer::new(cfg.clone()).step_verbose(&flat);
        let worst = flat_report
            .results
            .iter()
            .map(|r| (r.mean_fitness - r.fitness).abs())
            .fold(0.0f32, f32::max);
        // "All trials are the same episode" is a claim about the episodes, but the only handle
        // this API exposes on them is the mean of their scores — so the tolerance has to be
        // the one f32 summation actually has, and that scales with the score. Three values
        // near 1e4 fold into a mean with a few ULPs of slack; the absolute 1e-6 this replaced
        // was luck, and it broke the moment a reward term moved the total by 88.
        let slack = 8.0 * f32::EPSILON * flat_report.max_fitness.abs().max(1.0);
        assert!(
            worst <= slack,
            "walker-free level: all trials are the same episode, but the mean drifted from \
             the representative by {worst}, which is past the {slack} that summing three \
             scores near {} can explain",
            flat_report.max_fitness
        );

        // The walker stands four tiles ahead of the spawn, so any genome that moves
        // right at all meets it -- and meets it at a different moment in each trial.
        let guarded = crate::fixture::walker_runway("trials-walker", 24, 22, 5);
        let (walker_report, _) = Trainer::new(cfg.clone()).step_verbose(&guarded);
        let differing = walker_report
            .results
            .iter()
            .filter(|r| (r.mean_fitness - r.fitness).abs() > 1e-3)
            .count();
        assert!(
            differing >= 2,
            "only {differing}/{} genomes saw different trials",
            walker_report.results.len()
        );

        // Not a lucky draw from that random population: a genome that is nothing but
        // "hold right" always reaches the walker.
        let run_only = vec![Genome::zeros(0, cfg.shape.clone())];
        let scored = evaluate_population(&guarded, &run_only, &cfg, 0);
        assert!(
            (scored[0].mean_fitness - scored[0].fitness).abs() > 1e-3,
            "the constant-run genome saw identical trials on a level with a walker"
        );
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

    // -- the three search mechanisms ------------------------------------------

    /// How many genes two genomes do not share.
    ///
    /// Both are read through the same shape, so a length mismatch would be a broken
    /// fixture rather than a search result, and it is asserted rather than assumed.
    fn gene_distance(a: &Genome, b: &Genome) -> usize {
        assert_eq!(a.weights.len(), b.weights.len(), "shape mismatch");
        assert_eq!(a.biases.len(), b.biases.len(), "shape mismatch");
        a.weights
            .iter()
            .zip(&b.weights)
            .chain(a.biases.iter().zip(&b.biases))
            .filter(|(x, y)| x != y)
            .count()
    }

    /// A scored episode carrying only the one number these tests read.
    ///
    /// The rule compares `mean_fitness` against a parent's, so nothing else here has to be
    /// plausible — and building a real episode would make a unit test of an arithmetic rule
    /// depend on the physics crate's fixtures.
    fn episode(fitness: f32) -> EpisodeResult {
        EpisodeResult {
            genome_id: 0,
            fitness,
            mean_fitness: fitness,
            outcome: sim::Outcome::Running,
            frames: 0,
            progress_pixels: 0.0,
            best_x: 0.0,
            stomps: 0,
            jumps: 0,
            checkpoints: 0,
            death_pos: None,
            seed: 0,
            trajectory_hash: 0,
        }
    }

    #[test]
    fn fine_children_are_local_steps_from_the_generations_best() {
        // The second scale of the search: a fine child is one to three genes away from the
        // best genome, where a coarse child at the default rate moves about eighty-six.
        // Mutation is switched off entirely so the two scales cannot be confused — with
        // `mutation_rate: 0` and no crossover, a coarse child is an exact copy of one of its
        // parents, so anything that differs from the best *at all* is a fine child.
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 32,
            elite: 2,
            fine_children: 6,
            mutation_rate: 0.0,
            crossover_rate: 0.0,
            adaptive_sigma: false,
            stagnation_generations: 0,
            master_seed: 21,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg.clone());
        t.step(&level);

        // Slot 0 is the elite: the best genome as it was evaluated, which is the parent the
        // fine children were bred from. The breed order is elites, fine, immigrants, coarse.
        let best = t.genomes[0].clone();
        for slot in cfg.elite..cfg.elite + cfg.fine_children {
            let child = &t.genomes[slot];
            let distance = gene_distance(&best, child);
            assert!(
                (1..=FINE_GENE_MAX).contains(&distance),
                "fine child in slot {slot} differs from the best genome in {distance} genes; \
                 a local step is 1..={FINE_GENE_MAX} of its {} genes",
                best.n_genes()
            );
            assert_ne!(child.id, best.id, "a fine child must be a new genome");
        }

        // And the fine half is a budget, not the population: with a zero mutation rate every
        // coarse child is a copy, which is what the remaining slots must look like.
        let copies = t.genomes[cfg.elite + cfg.fine_children..]
            .iter()
            .filter(|c| gene_distance(&best, c) == 0)
            .count();
        assert!(copies > 0, "a zero-rate coarse child must be a copy of a parent");
    }

    #[test]
    fn a_stagnant_run_keeps_its_step_instead_of_shrinking_it_to_nothing() {
        // The regression this rule was rewritten for, pinned deterministically. With mutation
        // off and no crossover a child's genes are one of its parents', and on a walker-free
        // level a genome's episode is a function of its genes — so the best genome can never
        // be beaten, the success rate is zero, and a purely capacity-driven rule would walk
        // the step down to the floor and freeze there. The measured version holds instead: the
        // step that got the population to the plateau is the step that will get it off.
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 32,
            elite: 2,
            fine_children: 0,
            mutation_rate: 0.0,
            crossover_rate: 0.0,
            adaptive_sigma: true,
            stagnation_generations: 0,
            master_seed: 5,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg.clone());
        assert_eq!(t.mutation_sigma(), cfg.mutation_sigma, "a run starts where it was told");
        for _ in 0..40 {
            t.step(&level);
        }
        assert!(t.stagnation() > 0, "this fixture must be sitting still");
        assert_eq!(
            t.mutation_sigma(),
            cfg.mutation_sigma,
            "a run that never finds a new best must keep the step it started with, not back \
             off to the floor and freeze"
        );
        // The size the run actually bred with is the size on the row, not the one it started
        // from: a chart of the search has to be able to read it.
        let last = t.history().last().expect("a generation ran");
        assert_eq!(last.mutation_sigma, t.mutation_sigma());
    }

    #[test]
    fn the_one_fifth_rule_reads_the_success_rate_of_the_children() {
        // The rule's own arithmetic, with the population's scores supplied rather than
        // earned: two runs that differ *only* in whether the children beat their parents must
        // move the size in opposite directions, which is the whole claim the mechanism makes.
        let cfg = GaConfig {
            pop_size: 24,
            elite: 0,
            fine_children: 0,
            adaptive_sigma: true,
            master_seed: 3,
            ..GaConfig::default()
        };
        let start = cfg.mutation_sigma;

        let mut winner = Trainer::new(cfg.clone());
        winner.parent_fitness = vec![100.0; cfg.pop_size];
        let results: Vec<EpisodeResult> = (0..cfg.pop_size).map(|_| episode(110.0)).collect();
        winner.tune_sigma(&results);
        assert!(
            winner.mutation_sigma() > start,
            "every child beating its parent must grow the step: {start} -> {}",
            winner.mutation_sigma()
        );

        let mut loser = Trainer::new(cfg.clone());
        loser.parent_fitness = vec![100.0; cfg.pop_size];
        let results: Vec<EpisodeResult> = (0..cfg.pop_size).map(|_| episode(90.0)).collect();
        loser.tune_sigma(&results);
        assert!(
            loser.mutation_sigma() < start,
            "no child beating its parent must shrink the step: {start} -> {}",
            loser.mutation_sigma()
        );

        // A sample too thin to believe is not a trend: one short of a quarter of the
        // population moves nothing, whatever the rate looks like.
        let mut thin = Trainer::new(cfg.clone());
        thin.parent_fitness = vec![NO_PARENT; cfg.pop_size];
        let known = (cfg.pop_size / 4).max(4) - 1;
        for slot in thin.parent_fitness.iter_mut().take(known) {
            *slot = 100.0;
        }
        let results: Vec<EpisodeResult> = (0..cfg.pop_size).map(|_| episode(110.0)).collect();
        thin.tune_sigma(&results);
        assert_eq!(
            thin.mutation_sigma(),
            start,
            "{known} comparable children is too small a sample to steer the search"
        );

        // A plateau holds the size even when the children are winning: "improving" here means
        // beating a *parent* in this population, which is not the same as finding a new best,
        // and it is the latter that the rule keys on.
        let mut stalled = Trainer::new(cfg.clone());
        stalled.stagnation = 3;
        stalled.parent_fitness = vec![100.0; cfg.pop_size];
        let results: Vec<EpisodeResult> = (0..cfg.pop_size).map(|_| episode(110.0)).collect();
        stalled.tune_sigma(&results);
        assert_eq!(stalled.mutation_sigma(), start);

        // And the step cannot walk out of its documented range however long it wins: a size
        // of a hundred is a random restart, not a mutation.
        let mut runaway = Trainer::new(cfg.clone());
        for _ in 0..60 {
            runaway.parent_fitness = vec![100.0; cfg.pop_size];
            let results: Vec<EpisodeResult> = (0..cfg.pop_size).map(|_| episode(110.0)).collect();
            runaway.tune_sigma(&results);
        }
        assert_eq!(runaway.mutation_sigma(), MUTATION_SIGMA_RANGE[1]);

        // The gate lives in `advance`, not in the rule: with the flag off, a run must be bred
        // at the size it was given however its children do. Asserted through a real run rather
        // than by calling the rule directly, because calling the rule directly is precisely
        // what the flag is *not* consulted for.
        let mut gated = Trainer::new(GaConfig {
            adaptive_sigma: false,
            ..cfg
        });
        for _ in 0..6 {
            gated.step(&flat_level());
        }
        assert_eq!(
            gated.mutation_sigma(),
            start,
            "--fixed-sigma must keep the size exactly where the run was told to put it"
        );
    }

    /// The fraction of genes two genomes agree on the *sign* of.
    ///
    /// The statistic that separates a reheat from a restart. A genome's genes are small
    /// (`GENE_CLAMP` is 4.0 but Xavier starts weights near 0.15), so a fresh random genome is
    /// already close to the anchor in magnitude and no distance test can tell the two apart.
    /// What a reheat keeps is the anchor's shape — which way each gene points — and a random
    /// genome agrees with any given anchor about half the time by construction.
    fn sign_agreement(a: &Genome, b: &Genome) -> f32 {
        assert_eq!(a.weights.len(), b.weights.len(), "shape mismatch");
        assert_eq!(a.biases.len(), b.biases.len(), "shape mismatch");
        let mut same = 0usize;
        let mut total = 0usize;
        for (x, y) in a
            .weights
            .iter()
            .zip(&b.weights)
            .chain(a.biases.iter().zip(&b.biases))
        {
            total += 1;
            if x.signum() == y.signum() {
                same += 1;
            }
        }
        same as f32 / total as f32
    }

    /// The standard deviation of the per-gene difference between two genomes, which for a
    /// reheat is an estimate of the step it was thrown at (1 723 samples, so a couple of percent
    /// of sampling error — hence the loose bounds tests put on it).
    fn step_size(a: &Genome, b: &Genome) -> f32 {
        let diffs: Vec<f32> = a
            .weights
            .iter()
            .zip(&b.weights)
            .chain(a.biases.iter().zip(&b.biases))
            .map(|(x, y)| x - y)
            .collect();
        let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
        (diffs.iter().map(|d| (d - mean).powi(2)).sum::<f32>() / (diffs.len() - 1) as f32).sqrt()
    }

    /// Pearson correlation between two genomes read as one flat vector of genes.
    ///
    /// The statistic the shipped default needs, because at the hot end the sign agreement falls
    /// to about 0.57 against a chance level of 0.50 and no longer carries the claim. Correlation
    /// does: a reheat is the anchor plus independent noise, which the theory puts at
    /// `sigma_genes / sqrt(sigma_genes^2 + step^2)` — about 0.16 at the shipped temperature, and
    /// zero for a genome drawn without reference to the anchor.
    fn gene_correlation(a: &Genome, b: &Genome) -> f32 {
        let x: Vec<f32> = a.weights.iter().chain(&a.biases).copied().collect();
        let y: Vec<f32> = b.weights.iter().chain(&b.biases).copied().collect();
        let n = x.len() as f32;
        let mx = x.iter().sum::<f32>() / n;
        let my = y.iter().sum::<f32>() / n;
        let cov: f32 = x.iter().zip(&y).map(|(p, q)| (p - mx) * (q - my)).sum();
        let vx: f32 = x.iter().map(|p| (p - mx).powi(2)).sum();
        let vy: f32 = y.iter().map(|q| (q - my).powi(2)).sum();
        cov / (vx.sqrt() * vy.sqrt())
    }

    #[test]
    fn the_reheat_temperature_is_the_runs_own_step_scaled_and_capped() {
        // The mapping the sweep in REVIEW.md §4 is a sweep *of*: the batch is thrown at the
        // run's mutation size times the factor, and never past the documented ceiling.
        let at = |sigma: f32, factor: f32| {
            GaConfig {
                mutation_sigma: sigma,
                reheat_factor: factor,
                ..GaConfig::default()
            }
            .reheat_sigma()
        };
        // Compared with a tolerance rather than by equality: `0.30 * 3.0` is 0.90000004 in f32,
        // and the sweep this test guards is about where the batch lands, not about the last bit
        // of a product.
        let near = |got: f32, want: f32| (got - want).abs() < 1e-5;
        assert!(near(at(0.30, 1.0), 0.30), "the default is the run's own step");
        assert!(near(at(0.30, 0.5), 0.15), "a cooler reheat is half of it");
        assert!(near(at(0.30, 2.0), 0.60));
        assert!(near(at(0.30, 3.0), 0.90));
        // The ceiling is what keeps the hottest reheat short of a random restart: 0.30 x 4.0
        // asks for 1.2 and gets 1.0, and a run already told 1.0 cannot ask for more.
        assert_eq!(at(0.30, 4.0), MUTATION_SIGMA_RANGE[1]);
        assert_eq!(at(1.0, 8.0), MUTATION_SIGMA_RANGE[1]);
        // And a factor of zero is the floor rather than no reheat at all: a batch thrown at zero
        // would be a copy of the anchor in every gene, which is not a search move.
        assert_eq!(at(0.30, 0.0), MUTATION_SIGMA_RANGE[0]);

    }

    /// Breed one plateau batch on a fixture that cannot improve, and hand back the anchor the
    /// batch was thrown from with the batch itself.
    ///
    /// Mutation is off, so the maximum cannot move on this walker-free level and the stagnation
    /// counter reaches its threshold on a schedule rather than by luck.
    fn a_plateau_batch(factor: f32) -> (Genome, Vec<Genome>) {
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 32,
            elite: 4,
            fine_children: 0,
            mutation_rate: 0.0,
            crossover_rate: 0.0,
            stagnation_generations: 1,
            immigrants: 8,
            reheat_factor: factor,
            master_seed: 31,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg.clone());
        // Three generations: a `stagnation_generations` of 1 is sanitised to 2, so generation 2 is
        // the first with two still generations behind it.
        let mut report = t.step(&level);
        for _ in 0..2 {
            report = t.step(&level);
        }
        assert_eq!(report.immigrants, cfg.immigrants, "the batch must have fired");
        // The anchor is the run's best-ever as it stood when the batch was bred — the same genome,
        // since nothing has been scored since (which is why the batch could fire at all).
        let anchor = t
            .best_ever()
            .expect("a scored generation leaves a best-ever")
            .genome
            .clone();
        let batch = t.genomes[cfg.elite..cfg.elite + cfg.immigrants].to_vec();
        (anchor, batch)
    }

    #[test]
    fn a_plateau_batch_is_thrown_at_the_temperature_the_config_asks_for() {
        // The mapping above is arithmetic; this is the same claim measured off the genomes that
        // were actually bred, which is what a sweep of the knob changes.
        for factor in [1.0, 2.0, GaConfig::default().reheat_factor] {
            let (anchor, batch) = a_plateau_batch(factor);
            let expected = (0.30 * factor).clamp(MUTATION_SIGMA_RANGE[0], MUTATION_SIGMA_RANGE[1]);
            for (i, child) in batch.iter().enumerate() {
                let step = step_size(&anchor, child);
                assert!(
                    (0.85 * expected..1.15 * expected).contains(&step),
                    "factor {factor}: child {i} was thrown {step:.3} from the anchor where the \
                     config asks for {expected:.3}"
                );
            }
        }
    }

    #[test]
    fn even_the_hottest_batch_is_still_a_pull_towards_the_champion() {
        // The honest shape of the shipped default, stated as a measurement rather than as a
        // slogan: at 3.0 the batch is thrown 0.90 at genes whose own scale is about 0.15, so most
        // of each child is noise and the anchor survives only as a bias. A *bias* is the whole
        // difference between this and the random restart it replaced, and it is faint enough that
        // this test is the only place the claim is made precisely — the sign agreement the other
        // test uses is worthless at this temperature (it falls to ~0.57 against a chance level of
        // 0.50), which is why that test pins its own factor.
        let factor = GaConfig::default().reheat_factor;
        assert!(factor > 1.0, "this test is about the hot end");
        let (anchor, batch) = a_plateau_batch(factor);
        let mean = batch.iter().map(|c| gene_correlation(&anchor, c)).sum::<f32>()
            / batch.len() as f32;
        // Theory: sigma_genes / sqrt(sigma_genes^2 + step^2) with sigma_genes about 0.15 and a
        // step of 0.90 is about 0.16. The threshold is far below the estimate and far above zero,
        // and the seed is fixed, so this is a deterministic claim either way.
        assert!(
            mean > 0.05,
            "a batch at {factor}x correlates with the champion at only {mean:.3}: this is a \
             random restart, not a reheat"
        );
    }

    #[test]
    fn immigrants_arrive_only_on_the_stagnation_threshold() {
        // Deterministic stagnation: with mutation off, no crossover and no fine children, the
        // best genome is an exact copy of a previous best every generation, and on a
        // walker-free level a genome's score is a function of its genes — so the maximum
        // cannot move and the counter must reach the threshold on schedule.
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 32,
            elite: 4,
            fine_children: 0,
            mutation_rate: 0.0,
            crossover_rate: 0.0,
            adaptive_sigma: false,
            stagnation_generations: 3,
            immigrants: 6,
            // Pinned to the *coolest* interesting temperature rather than left at the default.
            // This test is about the batch being a reheat at all, and a shape claim needs margin:
            // at the shipped 3.0 the anchor's pull is real but faint (the batch is thrown 0.90 at
            // genes whose own scale is about 0.15, so it is mostly noise with a bias), and the
            // temperature has its own test below.
            reheat_factor: 1.0,
            master_seed: 9,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg.clone());

        // Generation 3 is the first with three still generations behind it, and it is the
        // only one that is allowed to have brought anybody in.
        for generation in 0..cfg.stagnation_generations {
            let report = t.step(&level);
            assert_eq!(report.generation, generation);
            assert_eq!(
                report.immigrants, 0,
                "generation {generation} has not been still for {} generations yet",
                cfg.stagnation_generations
            );
        }
        let report = t.step(&level);
        assert_eq!(report.immigrants, cfg.immigrants, "the threshold is inclusive");

        // The breed order is elites, fine, immigrants, coarse, so the reheated batch sits in
        // slots elite..elite + immigrants. Each one is a long walk from the anchor that keeps
        // the anchor's shape — which is the difference between reheating the champion and
        // restarting from noise, and the reason the batch exists at all.
        let anchor = t
            .best_ever()
            .expect("a scored generation leaves a best-ever")
            .genome
            .clone();
        let mut rng = rng_from_seed(7);
        let restart = Genome::random(9999, cfg.shape.clone(), &mut rng);
        let chance = sign_agreement(&anchor, &restart);
        for slot in cfg.elite..cfg.elite + cfg.immigrants {
            let child = &t.genomes[slot];
            let moved = gene_distance(&anchor, child);
            assert!(
                moved > 0,
                "slot {slot} is the anchor itself, so nothing was reheated"
            );
            let kept = sign_agreement(&anchor, child);
            assert!(
                kept > chance + 0.1,
                "slot {slot} agrees with the anchor on {kept:.3} of its genes, against the \
                 {chance:.3} a fresh random genome manages: this is a restart wearing a \
                 reheat's name"
            );
        }
        // ...and the counter restarts with them, so the run gets a full interval to do
        // something with the new genomes before more arrive.
        assert_eq!(t.stagnation(), 0);

        // One burst is a nudge, not a regime: with the rule switched off, no generation ever
        // brings a stranger in.
        let mut off = Trainer::new(GaConfig {
            stagnation_generations: 0,
            ..cfg.clone()
        });
        for generation in 0..10 {
            assert_eq!(
                off.step(&level).immigrants,
                0,
                "generation {generation} with the rule off"
            );
        }
    }

    #[test]
    fn the_search_bookkeeping_is_as_reproducible_as_the_rest_of_the_run() {
        // Adaptive sigma and immigrant bursts are state carried between generations, which is
        // exactly the kind of thing that can quietly depend on timing or on a hash map's
        // iteration order. Same seed, same run, same readings.
        let level = flat_level();
        let cfg = GaConfig {
            pop_size: 24,
            master_seed: 1234,
            adaptive_sigma: true,
            stagnation_generations: 4,
            immigrants: 4,
            ..GaConfig::default()
        };
        let run = || {
            let mut t = Trainer::new(cfg.clone());
            let mut sigmas = Vec::new();
            let mut immigrant_rows = Vec::new();
            for _ in 0..12 {
                let report = t.step(&level);
                sigmas.push(report.mutation_sigma);
                immigrant_rows.push(report.immigrants);
            }
            (sigmas, immigrant_rows, t.next_id(), t.stagnation())
        };
        assert_eq!(run(), run(), "the search's own state is not reproducible");

        // And the tuning is really doing something: the same run with it switched off cannot
        // produce the same sigma trajectory, or the mechanism is dead code.
        let mut fixed = Trainer::new(GaConfig {
            adaptive_sigma: false,
            ..cfg.clone()
        });
        let fixed_sigmas: Vec<f32> = (0..12)
            .map(|_| fixed.step(&level).mutation_sigma)
            .collect();
        assert_ne!(
            fixed_sigmas, run().0,
            "a self-tuning run and a fixed one produced the same size throughout"
        );
    }
}
