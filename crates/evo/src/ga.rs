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
//!   offspring a normal child gets (eighty-six genes at the default rate and a 1 739-gene
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
use crate::genome::{crossover, default_shape, mutate, mutate_few, reheat, Genome, NetShape, MAX_FEEDBACK};

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
    /// Offspring per generation bred from the **quality-diversity archive** rather than from
    /// the population's own ranking: [`Trainer::archive`] holds the best genome found in each
    /// `(progress, checkpoints)` niche, and these children are perturbations of those
    /// incumbents. 0 disables the third scale.
    ///
    /// This is what makes the archive a *search* mechanism rather than a report. Selection
    /// reads one number, so a genome that is one ledge further along on a level whose scoring
    /// plateaus is worth nothing to it and dies out with its lineage; the archive keeps such a
    /// genome as the incumbent of its own niche, and breeding from it hands the population the
    /// stepping stone that plain fitness ranking threw away. The draw is deterministic per
    /// generation ([`Trainer::archive_parents`]) rather than random, so the trainer's RNG
    /// stream — which the rest of breeding, and therefore every measured reading in
    /// `REVIEW.md`, depends on — is untouched by this mechanism.
    ///
    /// **0 by default, and that is a measurement rather than a preference** — the same bargain
    /// [`GaConfig::adaptive_sigma`] makes. `scratch-tower` is this pack's plateau: twelve seeds
    /// at pop 200 for 300 generations, `--stop-on-win`, a run counted as solved when a
    /// generation first reports a win and scored at that generation's number:
    ///
    /// | `archive_children` | solved | mean first-win generation |
    /// |---|---|---|
    /// | 0 (default) | **10 / 12** | 94 |
    /// | 2 | 7 / 12 | 121 |
    /// | 4 | 6 / 12 | 88 |
    ///
    /// The dose makes it worse, not better: a plateau is a level where the population already
    /// stands on the only good behaviour, and putting a fifth of each generation's parents
    /// outside the ranking spends budget on niches that are *behind* it. Elsewhere it is now
    /// hard to find anything the trickle buys: on `l3-gauntlet`, 12 of 12 seeds at a mean
    /// first-win generation of 16 against 18 for the plain run — which is inside the noise of
    /// twelve seeds, and a change from the arm that was measured before [`sim::sensors::VERSION`]
    /// went to v6, where the same comparison was 15 against 41. That win was real and it was
    /// the sensor's to lose: a threat the observation reported late made the walker levels slow
    /// to learn, and handing a fifth of the population to the archive papered over it. With the
    /// enemy view distance honest, `l6-marathon` solves in single-digit generations either way
    /// (12 of 12 at 9 against 11) and the tower still loses six seeds to four. A mechanism whose
    /// one measured effect is a loss on the hardest level in the pack is offered rather than
    /// shipped on. The archive itself keeps filling either way, which is what makes the panel's
    /// diversity reading honest at 0.
    pub archive_children: usize,
    /// Breed those archive children only when the run is **out of ideas in both senses**: no
    /// new best for [`GaConfig::stagnation_generations`] *and* no new behavioural niche over the
    /// same window. False — the default — lets them trickle in every generation.
    ///
    /// **This is the reward-level version of the quality-diversity question, and it is a
    /// different mechanism from the one that was measured.** Trickling from the archive every
    /// generation was measured as a loss on the pack's plateau (see [`GaConfig::archive_children`]):
    /// on a plateau the population already stands on the only good behaviour, so spending a fifth
    /// of each generation on niches that are *behind* it is pure cost. But that is an argument
    /// about *when* the archive's incumbents are worth breeding, not about whether they ever are:
    /// a run that is still improving, or still finding new behaviour, is a run that does not need
    /// them, and a run where **both have stopped** is exactly the run the mechanism was invented
    /// for. Gating on both halves also makes the archive's own coverage the diversity signal, so
    /// nothing new has to be measured for it.
    ///
    /// **Measured, and the measurement is why the flag is a flag rather than the default.**
    /// Twelve seeds at pop 200, 300 generations, `--stop-on-win`, first-win generation,
    /// against the same arm with the gate off, at `--archive-children 2`:
    ///
    /// | level | off | every generation | gated |
    /// |---|---|---|---|
    /// | `scratch-tower` (plateau) | **10 / 12**, mean 94 | 7 / 12, mean 121 | 9 / 12, mean **76** |
    /// | `scratch-spike-tight` (hazard floor) | 1 / 12, mean 117 | **3 / 12**, mean 105 | 1 / 12, mean 192 |
    ///
    /// Paired on the seeds, the gate is the first archive reading that is not a loss on the
    /// plateau: against the ungated trickle it gains seed 3 and loses 4, 6, 8, 9 and 12, and
    /// against the gate off it gains 3 and loses only 6 and 9 — one fewer solve, and the solves
    /// it finds arrive seventeen generations earlier on average, which is why it is worth having
    /// at all. On the hazard floor it is the *opposite* and it is a real cost: the ungated
    /// trickle earns its keep there (3 of 12 against 1), and the gate suppresses exactly that,
    /// because a run grinding across a hazard floor is a run that keeps finding new bests and new
    /// niches, so the condition it waits for never arrives. So: `--archive-children 2` for a
    /// hazard floor, `--archive-children 2 --archive-on-diversity` for a plateau, and both stay
    /// off by default.
    pub archive_on_diversity: bool,
    /// Fitness units paid to a genome for behaving **rarely**: the selection score is
    /// `mean_fitness + novelty_weight × rarity`, where rarity is how little of the population
    /// shares the genome's behavioural cell (`0.0..=1.0`, and exactly `1.0` for a genome alone
    /// in its cell). `0.0` — the default — is the plain fitness ranking every earlier
    /// measurement in this repository was taken with, and at `0.0` the code path is skipped
    /// entirely so a run is bit-identical to what it was before this field existed.
    ///
    /// **This is the reward-level half of the quality-diversity question**, and it is a
    /// different mechanism from [`GaConfig::archive_children`] in three ways that matter.
    /// Trickling from the archive changes *who is bred* and was measured as a loss on the pack's
    /// plateau. This changes *what the population is ranked by*, and it does not need the archive
    /// to be a parent pool at all — it needs only the archive's two behaviour descriptors, which
    /// are already computed for every genome every generation. And because it enters the
    /// **selection** score rather than the recorded one, nothing it touches is advertised as a
    /// result: [`EpisodeResult::mean_fitness`], the chart, the champion and every number in
    /// `REVIEW.md` still read the real fitness. What the bonus does is keep a rare behaviour's
    /// lineage in the tournament long enough to be improved, which is precisely what an elitist
    /// tournament over one number destroys on a level whose score plateaus ([Mouret &
    /// Clune](https://arxiv.org/abs/1504.04909), the novelty-search literature the review cites).
    ///
    /// **The size of the bonus is in fitness units, on purpose.** One tile of progress is
    /// `PIXELS_PER_TILE = 16`, so a weight of `16` means "being alone in a niche is worth a tile
    /// of ground" — legible next to the terms it competes with, and small enough that a genome
    /// cannot climb by being strange: the goal is `10 000`, and rarity is capped at one tile per
    /// unit of weight however the population is spread out.
    ///
    /// **Measured, and the measurement decides how it ships:** see `REVIEW.md` §4's novelty
    /// section — the arm was run on the plateau and on the chain level at several weights and
    /// the default stayed `0.0`. The honest summary of it is in that table rather than here,
    /// because a mechanism that reads one number to make a policy general has to earn its place
    /// against the number it is reading.
    pub novelty_weight: f32,
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
    /// and relative magnitudes of 1 739 genes — and moves it a long way, which is the move a
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
    /// Episodes per genome, averaged. 1 lets [`GaConfig::effective_trials`] decide from the
    /// level (three where there are walkers, one where there are not); 2 or more is taken as
    /// the operator's own answer everywhere.
    pub trials: u32,
    /// Take [`GaConfig::recommended_trials_for`]'s advice for a level whose `trials` is still
    /// the default 1. On by default, because the alternative is what made `l6-marathon` a
    /// six-generation lottery; `false` pins every level to the literal `trials`, which is what
    /// the measurements taken before this lock existed can be reproduced with.
    pub auto_trials: bool,
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
            archive_children: 0,
            archive_on_diversity: false,
            novelty_weight: 0.0,
            adaptive_sigma: false,
            stagnation_generations: 20,
            immigrants: 8,
            reheat_factor: 3.0,
            trials: 1,
            auto_trials: true,
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

    /// How many episodes a level is worth judging a genome on.
    ///
    /// Three where there are patrolling walkers, one where there are not. Three rather than two
    /// because this is the number the command line has been using for walker levels since the
    /// trials work — as a hard-coded literal beside the flag name, which is why it is a function
    /// here now and not three copies of an opinion; and because the noise lives in the *patrol
    /// phase*, which is re-drawn per generation, so what has to be estimated is a mean over
    /// phases and two is the thinnest sample that can show a genome is unreliable rather than
    /// merely unlucky.
    ///
    /// One where there are no walkers, and that is not a default being coy: a walker-free level is
    /// deterministic, so every trial would be the same episode with the same seed, and tripling
    /// the cost of a generation to compute a mean of one number is exactly the mistake the warning
    /// in `train-cli` is printed to prevent.
    pub fn recommended_trials_for(level: &Level) -> u32 {
        if level.walkers.is_empty() {
            1
        } else {
            3
        }
    }

    /// How many episodes this genome will actually be judged on, on this level.
    ///
    /// **A walker level is a stochastic level, and one episode cannot measure a stochastic
    /// policy.** `episode_seed` mixes in the generation (finding 14), so a genome's patrol
    /// phase is re-rolled every generation: an elite can be handed a phase it never solved and
    /// fall out of the run, and — the reason this is wired rather than left as a suggestion —
    /// a level can be "solved" by a population that has learned nothing, because one agent out
    /// of two hundred happened to walk into the flag. `l6-marathon`, the longest level in the
    /// pack, was won in **generation 6 of 300** at pop 200 by exactly that accident.
    ///
    /// So a run whose `trials` is still the default `1` takes [`GaConfig::auto_trials`]'s
    /// recommendation on a level with walkers in it, and `trials` set to anything else (2 or
    /// more) is the operator's own answer and is used everywhere. The costly reading is the
    /// other way round: on a level with no walkers every trial is the same episode, so the
    /// recommendation is 1 and nothing is doubled.
    pub fn effective_trials(&self, level: &Level) -> u32 {
        if self.trials > 1 {
            self.trials
        } else if self.auto_trials {
            Self::recommended_trials_for(level)
        } else {
            1
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
        self.archive_children = self.archive_children.min(extra_cap);
        // The rarity bonus is an addition to the selection score, so it is bounded by what it
        // could pay: at 512 a maximally rare genome (an extreme of the grid with a population of
        // one) would out-earn the goal bonus, which would turn a tie-break into the objective.
        // The bound is a ceiling on a knob, not a policy about its value — the arm that was
        // measured used 16.
        self.novelty_weight = self.novelty_weight.clamp(0.0, NOVELTY_WEIGHT_MAX);
        // The reheat step is derived from sigma, so this bound is on the *multiple*: 8.0 with the
        // shipped sigma of 0.30 already asks for 2.4 and lands on the ceiling, and a negative
        // factor would mean a batch thrown *towards* the champion, which is not a search move.
        self.reheat_factor = self.reheat_factor.clamp(0.0, 8.0);
        // One generation is not stagnation, it is a generation; 0 disables the rule.
        if self.stagnation_generations == 1 {
            self.stagnation_generations = 2;
        }
        self.trials = self.trials.max(1);
        // Memory is a gene count rather than a behaviour, so the bound is about arithmetic:
        // `--feedback 100000` should clamp to a slower run, never to a failed allocation, and
        // the state can never be asked to be wider than the first hidden layer twice over.
        self.shape.feedback = self.shape.feedback.min(MAX_FEEDBACK);
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

/// The ceiling on [`GaConfig::novelty_weight`], in fitness units.
///
/// `512` is `32` tiles — more than any progress-only score a level can produce, which is the
/// point: above it the rarity bonus would stop being a tie-break on a plateau and start being the
/// objective, and a population maximising rarity alone is a random walk with extra steps. The
/// measured arm used `16`, one tile.
pub const NOVELTY_WEIGHT_MAX: f32 = 512.0;

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
    /// Children bred from the quality-diversity archive into the generation that follows
    /// this row (see [`GaConfig::archive_children`]).
    pub archive_children: usize,
    /// Behavioural niches the archive holds at the end of this generation, and how many the
    /// grid has room for ([`ARCHIVE_BEHAVIOUR_CELLS`]).
    ///
    /// The run's *diversity* reading: coverage falls when the whole population converges on
    /// one way of moving, which is the failure mode a fitness chart cannot show.
    pub archive_bits: usize,
    /// Finish bands the archive holds a winner in, of [`ARCHIVE_TIME_BINS`].
    ///
    /// The run's *speed* reading, and deliberately a separate number from the one above: how many
    /// ways of moving the population has found, and how many speeds it has won at. A run that has
    /// solved a level four times at the same pace reports one band, which is the honest answer to
    /// "has it learned to be quick" — it has not.
    pub archive_time_bits: usize,
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

/// The run's fastest **winning** episode: the clock, and whose it was.
///
/// A separate record from [`BestEver`], and a *run*-level one: the champion is the genome with
/// the highest fitness — since fitness v8 quickness is part of that score, but only part of it,
/// so the two can be different genomes the moment a run has more than one winner — while this is
/// the time itself. It is what the window shows a viewer to race against, and what the terminal
/// prints when a run ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FastWin {
    /// The generation that first beat the previous record.
    pub generation: u32,
    pub genome_id: u32,
    /// Frames the winning episode took.
    pub frames: u32,
    /// The level's own frame budget when it was run ([`sim::frame_budget`]), carried so a
    /// reader can say what fraction of what the terrain allows the win actually used.
    pub budget_frames: u32,
}

impl FastWin {
    /// The budget left unspent, as a fraction of the whole (`0.0..=1.0`).
    ///
    /// The same ratio [`crate::fitness::SPEED_BONUS`] pays on, computed once here so a readout
    /// and the score cannot come to different conclusions about how quick an episode was.
    pub fn spare(&self) -> f32 {
        if self.budget_frames == 0 {
            return 0.0;
        }
        (self.budget_frames.saturating_sub(self.frames)) as f32 / self.budget_frames as f32
    }
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
    /// Genomes that solved **every** level of the suite this generation was judged on; always
    /// `0` on a single-level run, where [`GenerationReport::wins`] is the solve signal. This is
    /// the number a suite run stops on, because one level won is not the suite.
    pub suite_solved: usize,
    pub genius: bool,
    /// The generation's best genome, already scored (so it has a champion-ready
    /// fitness). By **recorded** fitness: with `novelty_weight` set, the ranking is the
    /// novelty-weighted one and this is still the real best (see [`GaConfig::novelty_weight`]).
    pub best: Genome,
    /// The mutation size the children of this generation were bred with (see [`GenRow`]).
    pub mutation_sigma: f32,
    /// Immigrants bred into the generation that follows this one.
    pub immigrants: usize,
    /// Children bred from the archive into the generation that follows this one.
    pub archive_children: usize,
    /// Behavioural niches occupied at the end of this generation (see [`GenRow`]).
    pub archive_bits: usize,
    /// Finish bands holding a winner at the end of this generation (see [`GenRow`]).
    pub archive_time_bits: usize,
    /// The quickest win the run has recorded **so far** — carried on every report rather than
    /// summarised, because it is the run's record rather than the generation's: a generation that
    /// won nothing leaves the previous best time standing, and a viewer racing against it needs the
    /// number to be the same one on every frame.
    pub fastest: Option<FastWin>,
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
///
/// One level: [`evaluate_suite`] on a suite of one, deliberately the *same* code path rather than
/// a second copy of it, because a suite of one level is the run every measurement in `REVIEW.md`
/// was taken with and it has to stay bit-identical to it.
pub fn evaluate_population(
    level: &Level,
    genomes: &[Genome],
    cfg: &GaConfig,
    generation: u32,
) -> Vec<EpisodeResult> {
    evaluate_suite(std::slice::from_ref(level), genomes, cfg, generation)
}

/// Evaluate a whole population on a **suite** of levels (REVIEW §4 item 5): one generation
/// scored on every level in it, in parallel across genomes exactly as a single level is.
///
/// The per-genome work is [`suite_score`], which is also the function a front-end calls to ask the
/// same question about one genome — the champion, after the run — so "what did this suite do" has
/// one implementation rather than two that could disagree.
pub fn evaluate_suite(
    levels: &[Level],
    genomes: &[Genome],
    cfg: &GaConfig,
    generation: u32,
) -> Vec<EpisodeResult> {
    assert!(!levels.is_empty(), "a suite needs at least one level");
    genomes
        .par_iter()
        .map(|g| suite_score(levels, g, cfg, generation).0)
        .collect()
}

/// One genome on a whole suite: the result selection sees, and the per-level means beside it.
///
/// **The aggregate is the genome's *worst* level** — the minimum of the per-level mean fitnesses —
/// and it is the minimum rather than the sum the review's item 5 proposed, for an arithmetic
/// reason worth writing down, because a sum sounds more natural and is worse. Touching a flag is
/// worth [`fitness::GOAL_BONUS`] = `10 000` while a whole level of progress is worth about `1 000`,
/// so in a sum — or in a mean, which is a sum over a constant — **one easy level owns the score**:
/// a genome that solves the easiest level of three and stands still on the other two out-scores one
/// that is ninety percent of the way through all three, and the population is told to specialise by
/// the very reward that was built to stop it. A minimum cannot say that. Raising your worst level
/// is the only way up, which is what "generalises" means here, and it reads in the run's own units:
/// *the suite score is the score of the level this genome is worst at.*
///
/// The representative episode ([`EpisodeResult::fitness`]) is trial 0 of the suite's **first**
/// level, so a champion saved from a suite run is a claim about that level and `replay` checks it
/// there. [`EpisodeResult::suite_wins`] is raised only for a suite of more than one level, and a
/// level counts as won **only when every one of its trials won** — the conservative reading,
/// because a walker level judged on one patrol phase is a coin flip, and a suite that credited a
/// single phase would inherit exactly the lottery [`GaConfig::auto_trials`] was wired to remove.
///
/// Episode seeds stay `(master_seed, generation, genome id, trial)` with no level index, so the
/// suite's first level is played on the seed a single-level run would have given it and a
/// champion's recorded `episode_seed` keeps meaning what it meant.
pub fn suite_score(
    levels: &[Level],
    genome: &Genome,
    cfg: &GaConfig,
    generation: u32,
) -> (EpisodeResult, Vec<f32>) {
    assert!(!levels.is_empty(), "a suite needs at least one level");
    let eval = cfg.eval_config();
    let master_seed = cfg.master_seed;
    let mut representative: Option<EpisodeResult> = None;
    let mut level_means: Vec<f32> = Vec::with_capacity(levels.len());
    let mut suite_wins = 0u32;

    for (li, level) in levels.iter().enumerate() {
        // Per level, because the trial count is a property of the level: a walker level is
        // stochastic and judged on more than one patrol phase. See `effective_trials`.
        let trials = cfg.effective_trials(level);
        let mut sum = 0.0f32;
        let mut won_every_trial = true;
        for trial in 0..trials {
            // RNG and sim are constructed inside the loop: never shared.
            let seed = episode_seed(master_seed, generation, genome.id, trial);
            let r = fitness::run_episode(level, genome, &eval, seed);
            sum += r.fitness;
            won_every_trial &= r.won();
            // Trial 0 of the first level is the representative: it is the one anyone can
            // reproduce, so it is the one a champion records.
            if li == 0 && trial == 0 {
                representative = Some(r);
            }
        }
        level_means.push(sum / trials as f32);
        if won_every_trial {
            suite_wins += 1;
        }
    }

    let mut result = representative.expect("a suite has at least one level");
    // One level is not a suite: `suite_wins` stays 0, so a single-level run's report cannot be
    // misread as "this genome solved the suite", and the aggregate below is the mean of its own
    // trials exactly as it always was.
    if levels.len() > 1 {
        result.suite_wins = suite_wins;
    }
    result.mean_fitness = level_means.iter().copied().fold(f32::INFINITY, f32::min);
    (result, level_means)
}

/// The score every genome in a generation is **ranked and bred by**, given the recorded results.
///
/// At [`GaConfig::novelty_weight`] `0.0` — the default — this is the recorded fitness itself, and
/// the caller skips this function entirely so a run is bit-identical to the one before the field
/// existed. Otherwise each genome is paid for behaving *rarely*: rarity is `1 - share`, where
/// `share` is the fraction of the population occupying the same behavioural cell, and a genome
/// alone in its cell collects the whole weight.
///
/// Two things about that measure are deliberate rather than cheap. The cells are the quality-
/// diversity archive's own descriptors — how far toward the flag the genome got, how many safe
/// landing zones it stood on, and (since fitness v8) which band of the level's budget it won in —
/// so the rarity reading is the *archive's own*, already computed for every genome every
/// generation, with no new distance function to define, version and defend. The time axis makes
/// the bonus *finer* for winners than it used to be, which is the intended direction: a genome
/// that arrives at the flag quicker now stands in a rarer cell than one that crawled there, and
/// the unfinished keep crowding into the one slot they share. And it is a *population* reading
/// taken once per generation, so it has no memory to seed and no drift to reproduce: the run stays
/// a function of (genome, levels, seed).
fn selection_scores(level: &Level, results: &[EpisodeResult], weight: f32) -> Vec<f32> {
    let mut counts = [0usize; ARCHIVE_CELLS];
    let mut cells: Vec<usize> = Vec::with_capacity(results.len());
    for r in results {
        let (p_bin, c_bin, time_slot) = MapElitesArchive::descriptor(level, r);
        let idx = MapElitesArchive::cell_index(p_bin, c_bin, time_slot);
        counts[idx] += 1;
        cells.push(idx);
    }
    let pop = results.len().max(1) as f32;
    results
        .iter()
        .zip(cells)
        .map(|(r, idx)| r.mean_fitness + weight * (1.0 - counts[idx] as f32 / pop))
        .collect()
}

/// Number of progress bins in the MAP-Elites quality-diversity archive.
pub const ARCHIVE_PROGRESS_BINS: usize = 8;
/// Number of checkpoint bins in the MAP-Elites quality-diversity archive.
pub const ARCHIVE_CHECKPOINT_BINS: usize = 6;
/// Number of **finish-time** bins per behavioural niche (new in fitness v8).
///
/// Four quarters of the level's own frame budget ([`sim::frame_budget`]), so the archive holds
/// the quickest winner it has seen in each band of each niche. The axis exists because a winning
/// genome has *saturated* the progress axis — every winner is at the flag — and what is left to
/// tell two of them apart is how long they took. Without it, the fastest route found in a niche
/// is replaceable by a slower genome that scores a couple of points better on something else, and
/// the route dies with its lineage. With it, a niche has an incumbent per speed, so "get there
/// quicker" is a hill in the archive as well as in the score.
///
/// Four, and not more: the bands are about *ordering* fast from slow — the score already
/// measures how fast, in [`crate::fitness::SPEED_BONUS`] — and each extra band multiplies the
/// grid, which is a real cost to the coverage a run can ever reach.
pub const ARCHIVE_TIME_BINS: usize = 4;
/// Slots per behavioural niche: the finish bands, plus one for everything that did not finish.
///
/// One slot for the unfinished rather than four, because an episode that never reached the flag
/// has no time to band — and because collapsing them is what keeps the archive's *behaviour*
/// coverage ([`ARCHIVE_BEHAVIOUR_CELLS`]) the number it always was.
pub const ARCHIVE_TIME_SLOTS: usize = ARCHIVE_TIME_BINS + 1;
/// Behavioural niches in the archive (`8 × 6 = 48`).
///
/// The denominator of a run's *diversity* reading, and deliberately not [`ARCHIVE_CELLS`]: what
/// the panel's "N of 48 niches" counts is how much of the behaviour space the run has covered,
/// and a denominator that multiplied in the time axis would report that coverage as four times
/// emptier than it is.
pub const ARCHIVE_BEHAVIOUR_CELLS: usize = ARCHIVE_PROGRESS_BINS * ARCHIVE_CHECKPOINT_BINS;
/// Total cells in the MAP-Elites archive (`48 × 5 = 240`).
pub const ARCHIVE_CELLS: usize = ARCHIVE_BEHAVIOUR_CELLS * ARCHIVE_TIME_SLOTS;

/// One occupied cell of the [`MapElitesArchive`].
#[derive(Clone, Debug)]
pub struct ArchiveEntry {
    pub genome: Genome,
    pub fitness: f32,
    pub progress_bin: usize,
    pub checkpoint_bin: usize,
    /// Which finish band this entry won in, or `None` for an entry that never reached the flag.
    pub finish_band: Option<usize>,
    pub generation: u32,
}

/// A 2D MAP-Elites quality-diversity archive keyed by `(progress_bin, checkpoint_bin)`.
///
/// Keeps the highest-scoring genome discovered in each behavioural niche — how far
/// toward the flag the agent got and how many safe landing zones it actually stood on.
/// That preserves intermediate stepping-stone behaviours (like landing safely on a new
/// ledge vs leaping blindly past it) that plain tournament selection over fitness alone
/// can discard on a plateau.
#[derive(Clone, Debug)]
pub struct MapElitesArchive {
    cells: Vec<Option<ArchiveEntry>>,
}

impl Default for MapElitesArchive {
    fn default() -> Self {
        Self::new()
    }
}

impl MapElitesArchive {
    pub fn new() -> Self {
        MapElitesArchive {
            cells: vec![None; ARCHIVE_CELLS],
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(None);
    }

    /// Number of occupied **behavioural niches** — distinct `(progress, checkpoint)` pairs, however
    /// many speeds are held in them.
    ///
    /// The reading the run reports as its diversity, so it is deliberately blind to the time axis:
    /// a run that has found one route at four different speeds has found *one* way of moving, and a
    /// count that said four would report the opposite of what a viewer is looking at.
    pub fn occupied(&self) -> usize {
        let mut pairs = [false; ARCHIVE_BEHAVIOUR_CELLS];
        for (i, cell) in self.cells.iter().enumerate() {
            if cell.is_some() {
                pairs[i / ARCHIVE_TIME_SLOTS] = true;
            }
        }
        pairs.iter().filter(|held| **held).count()
    }

    /// How many cells of the grid are held, i.e. behaviour *and* speed.
    pub fn occupied_cells(&self) -> usize {
        self.cells.iter().filter(|c| c.is_some()).count()
    }

    /// How many of the finish bands hold a winner (`0..=ARCHIVE_TIME_BINS`).
    ///
    /// The run's *speed* coverage: a run that has won four times inside the same quarter of the
    /// level's budget has one band, and a run whose winners have got progressively quicker shows
    /// up here as the count climbing. It is reported beside the diversity count rather than folded
    /// into it, because the two say different things — how many ways of moving, and how many
    /// speeds.
    pub fn time_bands(&self) -> usize {
        let mut bands = [false; ARCHIVE_TIME_BINS];
        for cell in self.cells.iter().flatten() {
            if let Some(band) = cell.finish_band {
                if band < ARCHIVE_TIME_BINS {
                    bands[band] = true;
                }
            }
        }
        bands.iter().filter(|held| **held).count()
    }

    /// Fraction of the archive's *behaviour* space currently filled (`0.0..=1.0`).
    pub fn coverage(&self) -> f32 {
        self.occupied() as f32 / ARCHIVE_BEHAVIOUR_CELLS as f32
    }

    /// The flat cell index of a `(progress, checkpoint, time)` niche.
    pub fn cell_index(p_bin: usize, c_bin: usize, time_slot: usize) -> usize {
        (p_bin * ARCHIVE_CHECKPOINT_BINS + c_bin) * ARCHIVE_TIME_SLOTS + time_slot
    }

    /// The finish-time slot an episode belongs to: `0` for anything that did not reach the flag,
    /// otherwise `1 + band`, where the band is which quarter of the level's own budget it won in.
    ///
    /// The budget comes from the level rather than from the episode's stats because a descriptor is
    /// computed for every genome every generation and `sim::frame_budget` is a function of the
    /// level's size: deriving it here means the archive cannot disagree with the clock an episode
    /// was actually run against.
    pub fn time_slot(level: &Level, r: &EpisodeResult) -> usize {
        if !r.won() {
            return 0;
        }
        let budget = sim::frame_budget(level).max(1);
        let used = (r.frames as f32 / budget as f32).clamp(0.0, 0.9999);
        let band = ((used * ARCHIVE_TIME_BINS as f32) as usize).min(ARCHIVE_TIME_BINS - 1);
        1 + band
    }

    /// Iterate over every occupied niche in deterministic row-major order.
    pub fn entries(&self) -> impl Iterator<Item = &ArchiveEntry> {
        self.cells.iter().filter_map(|c| c.as_ref())
    }

    /// Compute the `(progress_bin, checkpoint_bin, time_slot)` niche for an episode result.
    ///
    /// On a routeable climb with multiple route hops, the progress axis blends Euclidean
    /// approach with geodesic route-hop completion (`route_hops_closed`), and the secondary
    /// axis counts `checkpoints + route_hops_closed`. That separates distinct platforms on a
    /// switchback tower into their own MAP-Elites niches even when Euclidean distance reverses.
    ///
    /// The third axis is the finish band ([`Self::time_slot`]): `0` for an episode that did not
    /// reach the flag, otherwise which quarter of the level's budget it won in.
    pub fn descriptor(level: &Level, r: &EpisodeResult) -> (usize, usize, usize) {
        let span_px = (level.width.max(level.height).max(1) as f32) * sim::PIXELS_PER_TILE;
        let euclid_frac = (r.progress_pixels.max(0.0) / span_px.max(1.0)).clamp(0.0, 0.9999);
        let frac = if r.route_hops_closed > 0 {
            let total_hops = level.routes().len().saturating_sub(1).max(1) as f32;
            let hop_frac = (r.route_hops_closed as f32 / total_hops).clamp(0.0, 0.9999);
            euclid_frac.max(hop_frac)
        } else {
            euclid_frac
        };
        let p_bin = ((frac * ARCHIVE_PROGRESS_BINS as f32) as usize)
            .min(ARCHIVE_PROGRESS_BINS - 1);
        let c_bin = ((r.checkpoints + r.route_hops_closed) as usize).min(ARCHIVE_CHECKPOINT_BINS - 1);
        (p_bin, c_bin, Self::time_slot(level, r))
    }

    /// Observe a generation's evaluated population and update any niche where a genome
    /// improved on the niche's incumbent. Returns how many cells were newly filled or
    /// improved.
    pub fn observe(
        &mut self,
        level: &Level,
        genomes: &[Genome],
        results: &[EpisodeResult],
        generation: u32,
    ) -> usize {
        let mut updated = 0usize;
        for (g, r) in genomes.iter().zip(results.iter()) {
            let (p_bin, c_bin, time_slot) = Self::descriptor(level, r);
            let idx = Self::cell_index(p_bin, c_bin, time_slot);
            let replace = match &self.cells[idx] {
                None => true,
                Some(incumbent) => r.mean_fitness > incumbent.fitness,
            };
            if replace {
                let mut stored = g.clone();
                stored.fitness = r.mean_fitness;
                self.cells[idx] = Some(ArchiveEntry {
                    genome: stored,
                    fitness: r.mean_fitness,
                    progress_bin: p_bin,
                    checkpoint_bin: c_bin,
                    finish_band: (time_slot > 0).then(|| time_slot - 1),
                    generation,
                });
                updated += 1;
            }
        }
        updated
    }
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
    /// Quality-diversity MAP-Elites archive tracking the best genome in each
    /// `(progress, checkpoints, finish band)` cell.
    archive: MapElitesArchive,
    /// The quickest win this run has recorded, if any (see [`FastWin`]).
    fastest: Option<FastWin>,
    /// **The episode** that set [`Trainer::fastest`]: the genome, and the result carrying the
    /// seed it ran on.
    ///
    /// Carried beside the time rather than derived from it, because a time is not an episode.
    /// [`FastWin`] is four numbers and no network: it says *9.15 s*, and only the genome plus
    /// the seed that produced those frames can be run again to show somebody what 9.15 s looks
    /// like. Everything that wants to *race* the record — the playtest's ghost — needs this,
    /// and everything that wants to *read* it needs [`FastWin`]. Keeping them apart is also how
    /// the two stay honest: `fastest.frames == fastest_agent.result.frames` is an invariant this
    /// module maintains in one place, and a front-end that shows a ghost beside a time is
    /// therefore showing the same episode twice rather than two different ones.
    fastest_agent: Option<BestEver>,
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
            archive: MapElitesArchive::new(),
            fastest: None,
            fastest_agent: None,
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
            archive: MapElitesArchive::new(),
            fastest: None,
            fastest_agent: None,
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

    /// What [`GaConfig::archive_on_diversity`] waits for, as one question.
    ///
    /// True when the trickle should fire this generation: either it is ungated, or the run has
    /// stopped improving **and** stopped discovering. The window is
    /// [`GaConfig::stagnation_generations`] (floored at 2, because "flat over one generation"
    /// is a description of a single step rather than of a plateau), and both halves have to hold
    /// at once — a run that is still climbing does not need the archive's help, and a run that
    /// is still finding new behaviour has not run out of ideas even if its best has not moved.
    ///
    /// Read from the history rather than from the stall counter, on purpose: the counter is
    /// reset by the reheat, so a run with the reheat disabled would have no stall signal at all,
    /// and "no improvement" is a fact about the recorded rows that both rules can share without
    /// one of them owning it.
    fn archive_is_due(&self) -> bool {
        if !self.config.archive_on_diversity {
            return true;
        }
        let window = self.config.stagnation_generations.max(2) as usize;
        self.improvement_is_flat(window) && self.diversity_is_flat(window)
    }

    /// No row in the last `window` generations scored better than the best row before them.
    fn improvement_is_flat(&self, window: usize) -> bool {
        let history = self.history();
        if history.len() < window {
            return false;
        }
        let split = history.len() - window;
        let before = history[..split]
            .iter()
            .map(|row| row.max_fitness)
            .fold(f32::NEG_INFINITY, f32::max);
        let since = history[split..]
            .iter()
            .map(|row| row.max_fitness)
            .fold(f32::NEG_INFINITY, f32::max);
        since <= before
    }

    /// The archive holds no more behavioural niches than it did `window` generations ago.
    ///
    /// Coverage rather than spread, because coverage is what the archive already measures and
    /// what a converged run loses: a population that has all agreed on one way to move stops
    /// finding new niches, whatever the distances between its genomes.
    fn diversity_is_flat(&self, window: usize) -> bool {
        let history = self.history();
        if history.len() < window {
            return false;
        }
        let then = history[history.len() - window].archive_bits;
        self.archive.occupied() <= then
    }

    /// Read-only view of the run's MAP-Elites quality-diversity archive.
    pub fn archive(&self) -> &MapElitesArchive {
        &self.archive
    }

    pub fn history(&self) -> &[GenRow] {
        &self.history
    }

    /// Best genome seen so far, with its winning episode. `None` until the first
    /// [`Trainer::step`].
    pub fn best_ever(&self) -> Option<&BestEver> {
        self.best_ever.as_ref()
    }

    /// The quickest win the run has recorded so far, if it has won at all.
    pub fn fastest(&self) -> Option<FastWin> {
        self.fastest
    }

    /// The **episode** behind [`Trainer::fastest`]: the winning genome and the result whose
    /// seed replays it.
    ///
    /// This is what a front-end needs to *show* the record rather than quote it — the playtest's
    /// ghost races this episode, so the time on the screen and the agent running beside the
    /// player are the same record. `None` exactly when [`Trainer::fastest`] is, because the two
    /// are written together in [`Trainer::advance`].
    pub fn fastest_agent(&self) -> Option<&BestEver> {
        self.fastest_agent.as_ref()
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
        self.advance(std::slice::from_ref(level), false).0
    }

    /// As [`Trainer::step`], on a **suite** of levels (REVIEW §4 item 5): one generation, one
    /// episode budget per genome per level, ranked by the genome's worst level.
    ///
    /// The trainer itself does not own the levels — [`Trainer::step`] is handed one for the same
    /// reason — so a suite is a slice at the call site rather than a field. That is what makes a
    /// curriculum step, a suite and an ordinary run the *same* trainer with the same RNG stream:
    /// changing what a generation is scored on cannot change how it is bred, and a run that adds a
    /// level half-way through keeps its history, its champion and its ids.
    pub fn step_suite(&mut self, levels: &[Level]) -> GenerationReport {
        self.advance(levels, false).0
    }

    /// As [`Trainer::step`], but also returns the population that was evaluated.
    ///
    /// Watch mode needs it: after `step` returns, `self.genomes` is already the
    /// *next* generation, so the genomes a [`GenerationReport`] describes are gone.
    /// The copy costs about 1.2 MB per generation at pop 200, which is why plain
    /// training does not pay for it.
    pub fn step_verbose(&mut self, level: &Level) -> (GenerationReport, Vec<Genome>) {
        self.advance(std::slice::from_ref(level), true)
    }

    /// As [`Trainer::step_verbose`], on a suite.
    pub fn step_suite_verbose(&mut self, levels: &[Level]) -> (GenerationReport, Vec<Genome>) {
        self.advance(levels, true)
    }

    fn advance(&mut self, levels: &[Level], keep_population: bool) -> (GenerationReport, Vec<Genome>) {
        let results = evaluate_suite(levels, &self.genomes, &self.config, self.generation);

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

        // The run's fastest **win**, which is a different record from the best-ever genome: the
        // champion is the highest-scoring genome, and since fitness v8 quickness is part of that
        // score but not all of it (jumps and checkpoints are the rest). This is the clock itself —
        // the number a viewer races against — and it is a record of the whole *run*, so it is only
        // replaced by something strictly quicker.
        let budget_frames = sim::frame_budget(&levels[0]);
        if let Some((idx, frames)) = results
            .iter()
            .enumerate()
            .filter(|(_, r)| r.won())
            .map(|(i, r)| (i, r.frames))
            .min_by_key(|(_, frames)| *frames)
        {
            let faster = match &self.fastest {
                None => true,
                Some(held) => frames < held.frames,
            };
            if faster {
                self.fastest = Some(FastWin {
                    generation: self.generation,
                    genome_id: self.genomes[idx].id,
                    frames,
                    budget_frames,
                });
                // The episode itself, so the window can *show* the record and not only quote
                // it. Stored here, in the one place the time is set, which is what makes
                // `fastest.frames == fastest_agent.result.frames` an invariant rather than a
                // coincidence: the ghost of this agent reaches the flag on exactly the tick
                // this time names, because it is the same genome on the same seed.
                self.fastest_agent = Some(BestEver {
                    generation: self.generation,
                    result: results[idx].clone(),
                    genome: self.genomes[idx].clone(),
                });
            }
        }

        // "Genius agent": the maximum jumped by more than 50 % (§10 Tier 1.3). The
        // `last_max > 0` guard keeps the splash from firing while the population
        // is still climbing out of negative fitness, when a 1.5× ratio is noise.
        let genius = self.last_max > 0.0 && max_fitness > 1.5 * self.last_max;

        // Ranking: descending **selection score**, ties broken by lower index. `total_cmp`
        // gives a total order, so this cannot panic on a NaN the way
        // `partial_cmp().unwrap()` would, and it is stable across platforms.
        //
        // The selection score is the recorded fitness unless `novelty_weight` is set, in which
        // case it is that fitness plus a rarity bonus (see `selection_scores`), and the branch is
        // skipped entirely at 0.0 so a plain run is bit-identical to one from before the field
        // existed. Everything *recorded* — the chart, the champion, every number in `REVIEW.md` —
        // reads `results`, so a rarity bonus can change what is bred and never what is claimed.
        let selection: Vec<f32> = if self.config.novelty_weight > 0.0 {
            selection_scores(&levels[0], &results, self.config.novelty_weight)
        } else {
            results.iter().map(|r| r.mean_fitness).collect()
        };
        let mut order: Vec<usize> = (0..results.len()).collect();
        order.sort_by(|&a, &b| selection[b].total_cmp(&selection[a]).then(a.cmp(&b)));

        // The generation's best genome by **recorded** fitness, and it is a separate answer from
        // the ranking on purpose: with a rarity bonus in play the top of `order` is the strangest
        // genome rather than the best one, and a champion is a claim about the level that must not
        // be made by the strangest genome in the generation. At `novelty_weight` 0 the two indices
        // are the same one, by the same tie-break (lowest index wins a tie).
        let winner = if self.config.novelty_weight > 0.0 {
            (0..results.len())
                .max_by(|&a, &b| {
                    results[a]
                        .mean_fitness
                        .total_cmp(&results[b].mean_fitness)
                        .then(b.cmp(&a))
                })
                .unwrap_or(order[0])
        } else {
            order[0]
        };
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

        // The archive keys on the **first** level of the suite rather than on every level at once:
        // one behavioural grid cannot hold two levels' "how far along" in the same axis without
        // inventing a distance between them, and the first level is the one a champion is recorded
        // on. A suite's niche count therefore reads the suite's first level, which is a limit of
        // the reading rather than of the search.
        self.archive
            .observe(&levels[0], &self.genomes, &results, self.generation);

        // ---- the search's own bookkeeping: stagnation, then the 1/5th rule -----
        if max_fitness > self.last_max {
            self.stagnation = 0;
        } else {
            self.stagnation = self.stagnation.saturating_add(1);
        }
        if self.config.adaptive_sigma {
            self.tune_sigma(&results);
        }

        // The suite's own solve signal: a genome that won **every** level, which is the only thing
        // "the suite fell" can mean. Zero on a single-level run by construction (`suite_wins` is
        // only raised for a suite of more than one level), so `wins` keeps its old meaning there.
        let suite_solved = if levels.len() > 1 {
            results
                .iter()
                .filter(|r| r.suite_wins as usize == levels.len())
                .count()
        } else {
            0
        };

        let (immigrants, archive_children) = self.breed(&order, &selection);
        let archive_bits = self.archive.occupied();
        let archive_time_bits = self.archive.time_bands();

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
            archive_children,
            archive_bits,
            archive_time_bits,
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
                suite_solved,
                genius,
                best,
                mutation_sigma: row.mutation_sigma,
                immigrants: row.immigrants,
                archive_children: row.archive_children,
                archive_bits: row.archive_bits,
                archive_time_bits: row.archive_time_bits,
                fastest: self.fastest,
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
    ///
    /// `selection` is the score the population is ranked and bred by, index-aligned with
    /// `self.genomes` ([`selection_scores`]). It is passed in rather than read off the genomes
    /// because the genomes carry the **recorded** fitness: a rarity bonus belongs to the ranking
    /// and to the tournament, and keeping it out of `Genome::fitness` is what stops it reaching a
    /// champion, the chart, or the 1/5th rule's parent baseline.
    fn breed(&mut self, ranking: &[usize], selection: &[f32]) -> (usize, usize) {
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
            // the anchor's shape survives — a one-sigma step on 1 739 genes perturbs each by its
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
            }            // The counter restarts with the batch, so the population gets the whole
            // `stagnation_generations` to do something with it before more arrives: a
            // population flooded with reheats every generation is just a random restart.
            self.stagnation = 0;
        }
        budget -= immigrants;

        // 4. Archive children: perturbations of the incumbent of a behavioural niche, drawn
        //    from the quality-diversity archive rather than from this generation's ranking.
        //
        //    The rank a genome gets is one number, and on a level whose scoring plateaus the
        //    genome that has learned the *next* obstacle is not the one with the highest
        //    number: it is one ledge further along with a worse time, or two checkpoints
        //    deeper with less distance closed. Ranking alone therefore discards it, and with
        //    it the lineage that would have made the next step. The archive keeps exactly one
        //    such genome per niche, and the slots here are its descendants — a small, steady
        //    trickle rather than a takeover, because the archive's incumbents are usually
        //    behind the population's best and would lose every tournament they entered.
        //
        //    After the immigrants rather than before them, so the two mechanisms that exist
        //    to open a stalled search stay where their own documents put them: the batch is
        //    "directly after the fine children", which is what stops a converged population
        //    outvoting it in the ranking tie-breaks, and this trickle is the one that can wait.
        //
        //    Parents are `NO_PARENT`: an incumbent is often from an earlier generation, and
        //    the 1/5th rule must not compare a child against a score from a different
        //    generation's episodes.
        let mut archive_children = 0usize;
        if self.config.archive_children > 0 && self.archive_is_due() {
            let slots = self
                .config
                .archive_children
                .min(budget.saturating_sub(1));
            for incumbent in self.archive_parents(slots) {
                let mut child = incumbent;
                mutate(&mut child, sigma, rate, &mut self.rng);
                child.id = self.next_id;
                child.fitness = 0.0;
                child.shape = shape.clone();
                self.next_id += 1;
                next.push(child);
                parents.push(NO_PARENT);
                archive_children += 1;
            }
        }

        // 5. Coarse children fill the rest.
        while next.len() < pop_size {
            let pa = self.tournament(ranking, selection);
            let pb = self.tournament(ranking, selection);
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
        (immigrants, archive_children)
    }

    /// Up to `want` archive incumbents to breed from this generation.
    ///
    /// **Deterministic, and deliberately so.** The cells are walked from an offset that moves
    /// with the generation — `generation % occupied`, then every `occupied / want`-th cell in
    /// row-major order — so two runs of the same seed breed from the same niches in the same
    /// order, and no draw is taken from the trainer's RNG stream. That stream is what makes
    /// every measured reading in `REVIEW.md` reproducible, and a mechanism that quietly
    /// re-rolled the sequence would invalidate the sweep tables rather than extend them.
    ///
    /// Row-major order over the archive grid is also behavioural order: rows are progress
    /// bins, so stepping by a stride samples the whole range of "how far along" rather than
    /// the first few niches in the corner. Fewer occupied cells than slots means fewer
    /// children, which is the honest outcome — breeding the same incumbent twice spends the
    /// budget on one candidate.
    fn archive_parents(&self, want: usize) -> Vec<Genome> {
        let entries: Vec<&ArchiveEntry> = self.archive.entries().collect();
        if entries.is_empty() || want == 0 {
            return Vec::new();
        }
        let take = want.min(entries.len());
        let stride = (entries.len() / take).max(1);
        let offset = (self.generation as usize) % entries.len();
        (0..take)
            .map(|i| entries[(offset + i * stride) % entries.len()].genome.clone())
            .collect()
    }

    /// Tournament selection over the *ranked* indices.
    ///
    /// Contenders are drawn from `ranking` but compared by their **selection score** through
    /// their original index, so the returned index is always a valid population slot. The score
    /// is the recorded fitness unless `novelty_weight` is set; a contender that is rare therefore
    /// has a real chance of winning a tournament it would otherwise lose, which is the whole of
    /// what the reward-level mechanism does.
    fn tournament(&mut self, ranking: &[usize], selection: &[f32]) -> usize {
        let mut best = usize::MAX;
        for _ in 0..self.config.tournament {
            let pick = ranking[(self.rng.next_u32() as usize) % ranking.len()];
            if best == usize::MAX || selection[pick] > selection[best] {
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
            route_hops_closed: 0,
            best_stand: None,
            suite_wins: 0,
            death_pos: None,
            seed: 0,
            trajectory_hash: 0,
        }
    }

    /// A scored episode that also lands in a chosen behavioural cell, for the rarity tests.
    ///
    /// `progress_pixels` and `checkpoints` are the two descriptors the archive keys on, so a
    /// fixture that sets them is a fixture that decides which niche it occupies.
    fn episode_in(fitness: f32, progress_pixels: f32, checkpoints: u32) -> EpisodeResult {
        EpisodeResult {
            progress_pixels,
            checkpoints,
            ..episode(fitness)
        }
    }

    // -----------------------------------------------------------------------
    // the suite trainer (REVIEW.md §4 item 5)
    // -----------------------------------------------------------------------

    /// A suite of one level is the run every earlier measurement was taken with, so it has to
    /// be the *same* run rather than a similar one: same generations, same numbers, same
    /// trajectory hashes. If this ever fails, every table in `REVIEW.md` taken after the suite
    /// landed is measuring a different search from the ones taken before it.
    #[test]
    fn a_suite_of_one_level_is_the_run_it_always_was() {
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 12,
            master_seed: 5,
            ..GaConfig::default()
        };
        let mut one = Trainer::new(cfg.clone());
        let mut suite = Trainer::new(cfg);
        for generation in 0..4 {
            let a = one.step(&level);
            let b = suite.step_suite(std::slice::from_ref(&level));
            assert_eq!(a.generation, generation);
            assert_eq!(a.max_fitness, b.max_fitness, "generation {generation}");
            assert_eq!(a.avg_fitness, b.avg_fitness);
            assert_eq!(a.min_fitness, b.min_fitness);
            assert_eq!(a.wins, b.wins);
            assert_eq!(a.best.id, b.best.id);
            assert_eq!(a.results, b.results, "episode for episode");
            assert_eq!(a.suite_solved, 0, "one level is not a suite");
        }
        assert_eq!(one.history(), suite.history());
    }

    /// The suite's own solve signal needs **every** level and **every** trial, and it is raised
    /// only when there is more than one level to solve.
    ///
    /// The zero genome is the fixture that makes this checkable without a training run: it is
    /// all-zeros, so it always runs, so it solves a flat runway (the test in `fitness.rs` pins
    /// that) and dies in a pit.
    #[test]
    fn a_suite_win_needs_every_level_and_is_zero_for_a_single_level() {
        let flat = flat_level();
        let pit = pit_level();
        let cfg = GaConfig {
            pop_size: 8,
            ..GaConfig::default()
        };
        let g = Genome::zeros(1, cfg.shape.clone());

        let (single, means) = suite_score(std::slice::from_ref(&flat), &g, &cfg, 0);
        assert_eq!(means.len(), 1);
        assert!(single.won(), "always-run solves a flat runway");
        assert_eq!(
            single.suite_wins, 0,
            "one level is not a suite, however solved it is"
        );

        let (split, means) = suite_score(&[flat.clone(), pit.clone()], &g, &cfg, 0);
        assert_eq!(means.len(), 2);
        assert_eq!(
            split.suite_wins, 1,
            "flat is solved and the pit is not: that is one level of two"
        );
        assert_eq!(
            split.mean_fitness,
            means[0].min(means[1]),
            "the aggregate is the worst level"
        );
        assert_eq!(split.fitness, single.fitness, "the representative is trial 0 of level 0");

        let (both, _) = suite_score(&[flat.clone(), flat], &g, &cfg, 0);
        assert_eq!(both.suite_wins, 2, "two levels, two wins");
    }

    /// **Why the aggregate is a minimum and not the sum the review proposed.**
    ///
    /// The arithmetic, with no run in it: one win is worth `GOAL_BONUS = 10 000` while a level's
    /// whole progress score is worth about a thousand, so the mean of per-level scores rewards
    /// the genome that specialises on the easiest level. The minimum rewards the genome that has
    /// no weak level. This test is the doc comment's argument as an assertion — if someone
    /// changes the aggregate to a mean or a sum, it fails and says why.
    #[test]
    fn a_summed_suite_would_let_the_easiest_level_own_the_score() {
        let specialist = [fitness::GOAL_BONUS + 200.0, 300.0, 250.0];
        let generalist = [900.0, 950.0, 980.0];
        let mean = |xs: [f32; 3]| xs.iter().sum::<f32>() / 3.0;
        let worst = |xs: [f32; 3]| xs.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            mean(specialist) > mean(generalist),
            "a mean lets one solved level outscore three nearly-solved ones: {:.0} against {:.0}",
            mean(specialist),
            mean(generalist)
        );
        assert!(
            worst(generalist) > worst(specialist),
            "a minimum cannot: the specialist's worst level is {:.0}, the generalist's is {:.0}",
            worst(specialist),
            worst(generalist)
        );
    }

    /// A suite generation is a deterministic function of (population, levels, seed) like any
    /// other, and the second level does not disturb the first level's episodes: a champion
    /// trained on a suite carries the same trial-0 seed a single-level run would have given it.
    #[test]
    fn a_suite_is_reproducible_and_the_first_level_keeps_its_single_level_seed() {
        let flat = flat_level();
        let pit = pit_level();
        let cfg = GaConfig {
            pop_size: 10,
            master_seed: 21,
            ..GaConfig::default()
        };
        let suite = [flat.clone(), pit.clone()];

        let mut a = Trainer::new(cfg.clone());
        let mut b = Trainer::new(cfg.clone());
        let (first, population) = a.step_suite_verbose(&suite);
        let (again, _) = b.step_suite_verbose(&suite);
        assert_eq!(first.results, again.results, "same seed, same episodes");

        // The same population read on the first level alone must produce the same representative
        // episode the suite produced for it: trial 0 of level 0, from the same seed. This is what
        // makes a suite champion's recorded `episode_seed` a claim `replay` can check.
        let alone = evaluate_population(&flat, &population, &cfg, first.generation);
        for (suite_result, single_result) in first.results.iter().zip(alone.iter()) {
            assert_eq!(suite_result.seed, single_result.seed);
            assert_eq!(suite_result.fitness, single_result.fitness);
            assert_eq!(suite_result.trajectory_hash, single_result.trajectory_hash);
        }
    }

    // -----------------------------------------------------------------------
    // the rarity bonus (REVIEW.md §4 item 4's reward half)
    // -----------------------------------------------------------------------

    /// Rarity is the share of the population in the genome's own behavioural cell, and the bonus
    /// is exactly the weight times the rarity it lacks: three genomes, two in one cell and one
    /// alone in another, so the arithmetic is checkable by hand.
    #[test]
    fn rarity_is_the_share_of_the_population_in_the_genomes_own_niche() {
        let level = flat_level();
        let weight = 16.0f32;
        let paired = episode_in(100.0, 100.0, 1);
        let paired_twin = episode_in(90.0, 100.0, 1);
        let alone = episode_in(80.0, 300.0, 3);
        let results = vec![paired.clone(), paired_twin, alone.clone()];
        let scores = selection_scores(&level, &results, weight);

        // The two in the populated cell share it three ways (2 of 3), the solitary one does not.
        let crowded = weight * (1.0 - 2.0 / 3.0);
        let solitary = weight * (1.0 - 1.0 / 3.0);
        assert!((scores[0] - paired.mean_fitness - crowded).abs() < 1e-4, "{}", scores[0]);
        assert!((scores[1] - 90.0 - crowded).abs() < 1e-4, "{}", scores[1]);
        assert!((scores[2] - alone.mean_fitness - solitary).abs() < 1e-4, "{}", scores[2]);
        let bonus = |i: usize| scores[i] - results[i].mean_fitness;
        assert!(
            bonus(2) > bonus(1),
            "a rarer genome collects a bigger bonus than a crowded one: {:.2} against {:.2}",
            bonus(2),
            bonus(1)
        );
        // And the *recorded* numbers are untouched: this function returns scores, it does not
        // rewrite the episodes they came from.
        assert_eq!(results[0].mean_fitness, paired.mean_fitness);
        assert_eq!(results[2].mean_fitness, alone.mean_fitness);
    }

    /// A bonus big enough can outrank a fitter genome — that is the mechanism — and the default
    /// run must be untouched by its existence.
    #[test]
    fn a_rarity_bonus_can_outrank_a_fitter_genome_and_is_off_at_zero() {
        let level = flat_level();
        // Two genomes in one cell, one alone in another, and a ten-point fitness gap against a
        // third-of-the-population rarity gap: at a weight of 32 the crowded pair's bonus is 10.7
        // and the solitary genome's is 21.3, which is 10.7 points of reward for being rare.
        let crowded_a = episode_in(0.0, 100.0, 1);
        let crowded_b = episode_in(0.0, 100.0, 1);
        let fit_but_common = episode_in(110.0, 100.0, 1);
        let rare = episode_in(100.0, 300.0, 3);
        let results = vec![fit_but_common, crowded_a, crowded_b, rare];

        let off = selection_scores(&level, &results, 0.0);
        assert_eq!(
            off,
            results.iter().map(|r| r.mean_fitness).collect::<Vec<f32>>(),
            "at weight 0 the selection score is the recorded fitness, untouched"
        );

        let on = selection_scores(&level, &results, 32.0);
        assert!(
            on[3] > on[0],
            "the rare genome ({:.1}) must be able to outrank the fitter common one ({:.1})",
            on[3],
            on[0]
        );
    }

    /// The whole point of putting the bonus in the *selection* score: a run with it on and a run
    /// with it off record the same kind of numbers. The trainer's `GenRow` is the chart and the
    /// champion's fitness comes from `results`, so this is the invariant that keeps a novelty run
    /// comparable with every table in `REVIEW.md`.
    #[test]
    fn a_novelty_run_reports_the_same_kind_of_numbers_as_a_plain_one() {
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 12,
            master_seed: 3,
            ..GaConfig::default()
        };
        let novelty = GaConfig {
            novelty_weight: 16.0,
            ..cfg.clone()
        };
        let mut plain = Trainer::new(cfg);
        let mut driven = Trainer::new(novelty);
        // Three generations, because the point is that the *run* carries on being ordinary, and a
        // single step could not tell an unaffected run from one that had not got going.
        let mut a = plain.step(&level);
        let mut b = driven.step_suite(std::slice::from_ref(&level));
        for _ in 1..3 {
            a = plain.step(&level);
            b = driven.step_suite(std::slice::from_ref(&level));
        }

        // The maximum on the chart is a real episode's fitness in both runs, and the best genome
        // is the generation's real best rather than its strangest.
        let real_max = |report: &GenerationReport| {
            report
                .results
                .iter()
                .map(|r| r.mean_fitness)
                .fold(f32::NEG_INFINITY, f32::max)
        };
        assert_eq!(a.max_fitness, real_max(&a));
        assert_eq!(b.max_fitness, real_max(&b));
        assert_eq!(b.best.fitness, b.max_fitness);
        assert!(
            b.results.iter().any(|r| r.mean_fitness == b.max_fitness),
            "the champion's number is one of the episodes that were actually run"
        );

        // And the ranking is the rarity order, recomputed here from the generation's own results:
        // a bonus that reached the chart instead of the ranking would pass every assertion above
        // and change nothing about what is bred.
        let ranked_by = |scores: &[f32]| {
            let mut order: Vec<usize> = (0..scores.len()).collect();
            order.sort_by(|&x, &y| scores[y].total_cmp(&scores[x]).then(x.cmp(&y)));
            order
        };
        assert_eq!(
            b.ranking,
            ranked_by(&selection_scores(&level, &b.results, 16.0)),
            "a novelty run is ranked by the rarity scores"
        );
        assert_eq!(
            a.ranking,
            ranked_by(&a.results.iter().map(|r| r.mean_fitness).collect::<Vec<f32>>()),
            "a plain run is ranked by the recorded fitness"
        );
        // The genome that would be saved as this run's champion is the best **real** episode of
        // the run, which is the claim a champion file makes. A rarity bonus that leaked into
        // `best_ever` would save a strange genome and advertise its strangeness as a score.
        let run_best = driven
            .history()
            .iter()
            .map(|r| r.max_fitness)
            .fold(f32::NEG_INFINITY, f32::max);
        let champion = driven
            .best_ever()
            .expect("three generations ran")
            .result
            .mean_fitness;
        assert_eq!(
            champion, run_best,
            "the novelty run's champion is the best episode it actually ran"
        );
        assert_eq!(driven.history().len(), 3, "three generations were recorded");
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
    /// reheat is an estimate of the step it was thrown at (1 739 samples, so a couple of percent
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
        // **Forty generations, and the window is a measurement rather than a round number.** It
        // was twelve, and the sensors' v7 slot moved this fixture's trajectory far enough that its
        // first twelve generations now sit at a success rate the one-fifth rule does not act on,
        // so a twelve-generation window compared two constant 0.30 paths and called them different
        // — which is exactly the assertion being made here, failing for the right reason. The claim
        // is "the tuning does something"; the window has to be long enough for the rule to act.
        const GENERATIONS: usize = 40;
        let run = || {
            let mut t = Trainer::new(cfg.clone());
            let mut sigmas = Vec::new();
            let mut immigrant_rows = Vec::new();
            for _ in 0..GENERATIONS {
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
        let fixed_sigmas: Vec<f32> = (0..GENERATIONS)
            .map(|_| fixed.step(&level).mutation_sigma)
            .collect();
        assert_ne!(
            fixed_sigmas, run().0,
            "a self-tuning run and a fixed one produced the same size throughout"
        );
    }

    /// The quality-diversity trickle is off by default, and the default is the point of this
    /// test rather than a formality: the measurements on [`GaConfig::archive_children`] are what
    /// turn it off, and a default that drifted back to a positive number would quietly re-ship a
    /// mechanism that loses the pack's hardest level.
    #[test]
    fn the_archive_breeds_only_when_it_is_asked_to() {
        assert_eq!(
            GaConfig::default().archive_children,
            0,
            "the archive trickle is measured as a loss on scratch-tower and must be opt-in"
        );
        // And the archive is still filled on a default run, which is what makes a diversity
        // reading worth printing: the record and the breeding are separate switches.
        let mut t = Trainer::new(GaConfig {
            pop_size: 24,
            master_seed: 5,
            ..GaConfig::default()
        });
        t.step(&pit_level());
        t.step(&pit_level());
        assert!(t.archive().occupied() > 0);
        assert_eq!(t.history().last().unwrap().archive_children, 0);
    }

    /// The archive is a search mechanism, not a report: its incumbents breed.
    ///
    /// The count in the row is the claim — `archive_children` slots were filled from the
    /// archive on every generation after the first, and the knob that turns it off really
    /// turns it off. Together with the niche test below this is what separates "the run keeps
    /// a diversity record" from "the run uses one".
    #[test]
    fn archive_children_are_bred_every_generation_and_the_knob_turns_them_off() {
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 32,
            master_seed: 7,
            archive_children: 4,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg.clone());
        for _ in 0..3 {
            t.step(&level);
        }
        let last = t.history().last().expect("three generations were run");
        assert!(
            last.archive_children >= 1,
            "no child came from the archive in a run whose archive holds {} niches",
            last.archive_bits
        );
        assert!(last.archive_children <= cfg.archive_children);
        assert!(
            last.archive_bits <= ARCHIVE_CELLS,
            "the archive reported more niches than its grid has"
        );

        // Off is off: the same run with the knob at zero reports zero, so the count above is
        // the mechanism rather than a constant somebody prints.
        let mut off = Trainer::new(GaConfig {
            archive_children: 0,
            ..cfg
        });
        for _ in 0..3 {
            off.step(&level);
        }
        assert!(
            off.history().iter().all(|row| row.archive_children == 0),
            "archive children arrived with the mechanism disabled"
        );
        // And the archive itself is still filled: what the knob controls is the breeding, not
        // the record.
        assert!(off.archive().occupied() > 0);
    }

    /// A walker level is judged on more than one patrol phase, a walker-free one is not, and an
    /// explicit trial count is the operator's answer everywhere.
    ///
    /// This is the wiring behind finding 2.5. The recommendation existed for two passes and
    /// nothing called it, so every walker level in every table in `REVIEW.md` was decided by a
    /// single patrol phase — including `l6-marathon`, which was "solved" at generation 6 by one
    /// agent walking into the flag while the population knew nothing.
    #[test]
    fn a_walker_level_is_judged_on_more_than_one_patrol_phase() {
        let mut cfg = GaConfig::default();
        let walker_level = crate::fixture::walker_runway("trials-plate", 24, 22, 8);

        // A level with no walker: every trial is the same episode, so nothing is doubled.
        assert!(pit_level().walkers.is_empty());
        assert_eq!(cfg.effective_trials(&pit_level()), 1);
        // A level with one: the recommendation applies while `trials` is left at its default.
        assert!(!walker_level.walkers.is_empty());
        assert_eq!(cfg.effective_trials(&walker_level), 3);

        // An explicit number is honoured everywhere, including on the walker level.
        cfg.trials = 4;
        assert_eq!(cfg.effective_trials(&walker_level), 4);
        assert_eq!(cfg.effective_trials(&pit_level()), 4);

        // And the lock can be released, which is how the numbers taken before it existed are
        // reproduced: `--no-auto-trials` with `--trials 1` is the old behaviour exactly.
        cfg.trials = 1;
        cfg.auto_trials = false;
        assert_eq!(cfg.effective_trials(&walker_level), 1);
        assert_eq!(cfg.effective_trials(&pit_level()), 1);
    }

    /// Every trial is really *run*, on its own episode seed, and the score that selection reads
    /// is their mean — not one episode's number wearing the others' names.
    ///
    /// Asserted against `run_episode` called by hand with the seeds the trainer is documented to
    /// use, so this pins the arithmetic rather than re-stating it. The *count* comes from the
    /// trainer's own rule rather than from a literal here, which is what keeps this test honest
    /// when the recommendation moves (it was two phases for a pass, and is three): a test that
    /// hard-coded the mean of two episodes kept passing while a third was being run and ignored.
    #[test]
    fn every_selection_trial_is_run_on_its_own_seed_and_averaged_into_the_score() {
        let level = crate::fixture::walker_runway("trials-mean", 24, 22, 8);
        let cfg = GaConfig {
            pop_size: 2,
            trials: 1,
            auto_trials: true,
            master_seed: 5,
            ..GaConfig::default()
        };
        let generation = 3;
        let genome = Genome::zeros(7, cfg.shape.clone());
        let eval = cfg.eval_config();
        let score = |trial: u32| {
            fitness::run_episode(
                &level,
                &genome,
                &eval,
                episode_seed(cfg.master_seed, generation, genome.id, trial),
            )
            .fitness
        };
        let trials = cfg.effective_trials(&level);
        assert_eq!(trials, 3, "the walker rule is what decides this count");
        let scores: Vec<f32> = (0..trials).map(score).collect();
        assert_ne!(
            scores[0], scores[1],
            "a walker's patrol phase must actually differ between trials, or this test proves nothing"
        );

        let results = evaluate_population(&level, std::slice::from_ref(&genome), &cfg, generation);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].fitness, scores[0],
            "trial 0 stays the representative episode a champion records"
        );
        assert_eq!(
            results[0].mean_fitness,
            scores.iter().sum::<f32>() / trials as f32,
            "selection must read the mean of every trial it ran"
        );

        // And the release hatch really releases it: with the rule switched off the same genome on
        // the same level is scored on the one episode `trials` asks for, which is the arithmetic
        // every pre-existing table in `REVIEW.md` was taken with.
        let one = GaConfig {
            auto_trials: false,
            ..cfg.clone()
        };
        let one_result = evaluate_population(&level, std::slice::from_ref(&genome), &one, generation);
        assert_eq!(one.effective_trials(&level), 1);
        assert_eq!(one_result[0].mean_fitness, scores[0]);
    }

    /// The diversity-gated archive waits for a run that has stopped *both* improving and
    /// discovering, and then fires.
    ///
    /// The gate is the whole point of [`GaConfig::archive_on_diversity`]: trickling from the
    /// archive every generation was measured as a loss on the plateau, and the mechanism's own
    /// premise is that a run which is still finding new behaviour does not need it. The level here
    /// is a short flat runway with a reachable flag, so improvement stops almost at once and
    /// coverage saturates — the case the gate exists for — and the same run with the gate open
    /// (the default behaviour) is checked beside it.
    #[test]
    fn the_archive_can_wait_for_a_run_that_has_stopped_having_ideas() {
        let level = crate::fixture::flat_runway("qd-gate", 10, 8);
        let cfg = GaConfig {
            pop_size: 24,
            archive_children: 4,
            archive_on_diversity: true,
            stagnation_generations: 4,
            master_seed: 3,
            ..GaConfig::default()
        };

        let mut gated = Trainer::new(cfg.clone());
        let mut rows = Vec::new();
        for _ in 0..14 {
            gated.step(&level);
            rows.push(*gated.history().last().unwrap());
        }
        assert_eq!(
            rows[0].archive_children, 0,
            "one generation of history is not a plateau, so the gate cannot open yet"
        );
        assert_eq!(
            rows[1].archive_children, 0,
            "nor is two: the window is `stagnation_generations`"
        );
        assert!(
            rows.iter().any(|row| row.archive_children > 0),
            "a run that has stopped improving and stopped discovering must eventually be given the \
             archive's incumbents; rows: {:?}",
            rows.iter().map(|r| (r.max_fitness, r.archive_bits, r.archive_children)).collect::<Vec<_>>()
        );
        // Coverage is what the gate reads, and it must be flat by the time the gate opens: the
        // two halves of the condition are recorded, so a future change to either is visible here.
        assert!(
            gated.diversity_is_flat(cfg.stagnation_generations as usize),
            "the run that opened the gate is the run with flat coverage"
        );

        // The same run with the gate released: children from generation one, which is the
        // behaviour every earlier measurement of the trickle was taken with.
        let mut open = Trainer::new(GaConfig {
            archive_on_diversity: false,
            ..cfg
        });
        for _ in 0..3 {
            open.step(&level);
        }
        assert!(
            open.history().iter().any(|row| row.archive_children > 0),
            "an ungated trickle arrives immediately"
        );
    }

    /// The incumbents chosen to breed are the archive's own, drawn across the grid, and the
    /// choice moves with the generation.
    ///
    /// The contract of [`Trainer::archive_parents`], which is where the mechanism's
    /// determinism lives: every parent is a genome the archive really holds, a run asks for no
    /// more than the archive has, and a *later* generation breeds from a different niche —
    /// otherwise one incumbent would parent every batch until something displaced it.
    #[test]
    fn the_archive_breads_from_its_own_incumbents_and_moves_on() {
        let level = pit_level();
        let mut t = Trainer::new(GaConfig {
            pop_size: 32,
            master_seed: 11,
            ..GaConfig::default()
        });
        assert!(t.archive_parents(3).is_empty(), "nothing archived yet");
        for _ in 0..4 {
            t.step(&level);
        }
        let held: Vec<u32> = t.archive().entries().map(|e| e.genome.id).collect();
        assert!(held.len() > 1, "the archive holds {} niches", held.len());

        let picked = t.archive_parents(3);
        assert_eq!(picked.len(), 3.min(held.len()));
        for genome in &picked {
            assert!(
                held.contains(&genome.id),
                "genome {} is not an archive incumbent",
                genome.id
            );
        }
        // More than the archive has is not more children: the same incumbent twice would spend
        // two slots on one candidate.
        assert_eq!(t.archive_parents(ARCHIVE_CELLS).len(), held.len());

        // The pick moves with the generation: a run that always bred from niche zero would
        // never exploit the rest of the grid.
        let here = t.archive_parents(1)[0].id;
        t.generation += 1;
        let next = t.archive_parents(1)[0].id;
        assert_ne!(here, next, "the same incumbent parents every generation");
    }

    #[test]
    fn map_elites_archive_retains_best_genome_per_behavioural_niche() {
        let level = pit_level();
        let cfg = GaConfig {
            pop_size: 24,
            master_seed: 19,
            ..GaConfig::default()
        };
        let mut t = Trainer::new(cfg);
        assert_eq!(t.archive().occupied(), 0);
        for _ in 0..4 {
            t.step(&level);
        }
        assert!(
            t.archive().occupied() > 0,
            "archive must populate behavioural niches as generations evaluate"
        );
        assert!(t.archive().coverage() > 0.0 && t.archive().coverage() <= 1.0);
        for entry in t.archive().entries() {
            assert!(entry.progress_bin < ARCHIVE_PROGRESS_BINS);
            assert!(entry.checkpoint_bin < ARCHIVE_CHECKPOINT_BINS);
            assert!(entry.fitness.is_finite());
        }
    }

    /// One episode, built rather than run, so a test can say exactly what a genome did.
    fn staged_episode(
        id: u32,
        frames: u32,
        progress: f32,
        checkpoints: u32,
        outcome: sim::Outcome,
    ) -> EpisodeResult {
        EpisodeResult {
            genome_id: id,
            fitness: 0.0,
            mean_fitness: 0.0,
            outcome,
            frames,
            progress_pixels: progress,
            best_x: progress / 16.0,
            stomps: 0,
            jumps: 4,
            checkpoints,
            route_hops_closed: 0,
            best_stand: None,
            suite_wins: 0,
            death_pos: None,
            seed: 1,
            trajectory_hash: 0,
        }
    }

    /// The archive holds a niche per **speed** as well as per behaviour: two winners that took the
    /// same route at different paces are two cells and *one* way of moving.
    ///
    /// That split is the whole reason the time axis is a third bin rather than a wider first one.
    /// The behaviour count is the run's diversity reading, and a run that has found one route at
    /// four speeds has found one way of moving; a grid that multiplied the two together and
    /// reported the product would say exactly the opposite of what a viewer is looking at.
    #[test]
    fn the_archive_keeps_a_niche_per_speed_and_counts_one_behaviour() {
        let level = crate::fixture::flat_runway("time-bands", 60, 15);
        let budget = sim::frame_budget(&level);
        let shape = GaConfig::default().shape.clone();
        // The same behaviour — the same progress, the same landings — at two paces: a quarter of
        // the budget, and three quarters of it.
        let fast = staged_episode(101, budget / 4, 900.0, 2, sim::Outcome::Won);
        let slow = staged_episode(102, budget * 3 / 4, 900.0, 2, sim::Outcome::Won);
        assert_ne!(
            MapElitesArchive::time_slot(&level, &fast),
            MapElitesArchive::time_slot(&level, &slow),
            "the fixture's two paces have to land in different finish bands for this to prove anything"
        );
        assert!(MapElitesArchive::time_slot(&level, &fast) > 0, "a win is not slot zero");

        let mut archive = MapElitesArchive::new();
        let genomes = vec![Genome::zeros(101, shape.clone()), Genome::zeros(102, shape)];
        archive.observe(&level, &genomes, &[fast, slow], 0);
        assert_eq!(archive.occupied_cells(), 2, "two speeds, two cells");
        assert_eq!(archive.occupied(), 1, "and one way of moving");
        assert_eq!(archive.time_bands(), 2, "both bands hold a winner");
        assert!(archive.coverage() <= 1.0 / ARCHIVE_BEHAVIOUR_CELLS as f32 + 1e-6);

        // A losing episode has no time to band, however quickly it died: it shares the one
        // unfinished slot, which is what keeps the behaviour count honest — the archive is not
        // three-quarters empty because most of the population died young.
        let died = staged_episode(103, 4, 900.0, 2, sim::Outcome::Died);
        let dead = vec![Genome::zeros(103, GaConfig::default().shape)];
        archive.observe(&level, &dead, &[died], 0);
        assert_eq!(archive.occupied_cells(), 3);
        assert_eq!(archive.occupied(), 1, "a death fills no new behaviour");
        assert_eq!(archive.time_bands(), 2, "and no finish band");
        assert!(
            archive
                .entries()
                .any(|entry| entry.finish_band.is_none() && entry.genome.id == 103),
            "the dying genome is held as its niche's unfinished incumbent"
        );
    }

    /// The run keeps the quickest win it has seen, and keeps it across generations that win nothing.
    #[test]
    fn a_run_reports_the_quickest_win_it_has_recorded() {
        let level = crate::fixture::flat_runway("fastest", 10, 8);
        let cfg = GaConfig {
            pop_size: 24,
            master_seed: 5,
            ..GaConfig::default()
        };
        let eval = cfg.eval_config();
        let mut t = Trainer::new(cfg);
        let mut quickest: Option<u32> = None;
        for _ in 0..12 {
            let report = t.step(&level);
            for r in report.results.iter().filter(|r| r.won()) {
                quickest = Some(quickest.map_or(r.frames, |q| q.min(r.frames)));
            }
            match t.fastest() {
                Some(record) => {
                    assert_eq!(
                        Some(record.frames),
                        quickest,
                        "the run's record disagrees with the wins in its own reports"
                    );
                    assert_eq!(record.budget_frames, sim::frame_budget(&level));
                    assert!(record.spare() > 0.0 && record.spare() <= 1.0);

                    // And the **episode** behind the time is the one that time was measured on.
                    // This is the invariant everything that *shows* the record depends on: a
                    // front-end that races the ghost of `fastest_agent` beside the number in
                    // `fastest` would otherwise be showing two different agents' races — a ghost
                    // that finishes late, or dies, under a time it never ran. Asserting the
                    // identity is not enough on its own (the two fields could be written from
                    // the same wrong index), so the genome is also *replayed*: the same level,
                    // the same seed, and it must come back on exactly the recorded tick.
                    let agent = t
                        .fastest_agent()
                        .expect("the time and the episode are recorded together");
                    assert_eq!(agent.generation, record.generation);
                    assert_eq!(agent.genome.id, record.genome_id, "not the genome that won");
                    assert_eq!(agent.result.frames, record.frames);
                    let again = crate::fitness::run_episode(
                        &level,
                        &agent.genome,
                        &eval,
                        agent.result.seed,
                    );
                    assert_eq!(
                        again.outcome,
                        sim::Outcome::Won,
                        "the record's own episode must still win when it is replayed"
                    );
                    assert_eq!(
                        again.frames, record.frames,
                        "the episode behind the record finishes on {} frames, but replayed to {}",
                        record.frames, again.frames
                    );
                }
                None => assert!(
                    quickest.is_none(),
                    "a report held a win the run's own record missed"
                ),
            }
        }
        assert!(
            t.fastest().is_some(),
            "this fixture is won by a random population, so twelve generations must record a time"
        );
    }
}
