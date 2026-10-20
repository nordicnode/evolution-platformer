//! Curriculum resume: what a run does when it stops moving (§12 M5, findings 19–22).
//!
//! The plan's stall chain ends here, and this is the module both front-ends call.
//! Its premise is finding 22: **a stalled population is the best starting material a
//! run has**. Neither patience nor the live mutation-rate knob unsticks these levels
//! (findings 19 and 20), so the answer to "this level is not being solved" is a
//! different landscape, not a reset. The population that is stuck already performs
//! most of the jump it keeps dying on; it needs the *other* parts of its policy
//! rearranged, and a level it can actually get somewhere on is what rearranges them.
//!
//! Two artifacts come out of one step, and the difference between them matters:
//!
//! * **A checkpoint** of the stuck population, written *before* anything moves it.
//!   That is the plan's "restart from the checkpoint at the stuck point", and it is
//!   what makes an unrepeatable run repeatable: `watch --resume` (and the window's
//!   own resume) is the way back into a run that would otherwise take a lucky seed to
//!   reproduce. It also means the detour is never destructive — the population as it
//!   was is on disk whatever the excursion learns.
//! * **The trained population**, adopted back into the run it came from. The
//!   excursion trains under the *run's own* hyperparameters, including the mutation
//!   rate the viewer has dialled in, so what comes back is the population this run
//!   would have bred, had the level been easier.
//!
//! The excursion runs on a separate [`Trainer`] and is adopted through
//! [`Trainer::adopt`], which is the whole point of that method: the run keeps its own
//! chart history, its own best-ever champion, its own genius baseline and its own RNG
//! stream. Those belong to *the run*, not to whichever level the population last saw,
//! and a curriculum step that reset them would make the run's chart a lie (a cliff,
//! or a spike, that no agent ever experienced).
//!
//! The step is deliberately **not animated** in either front-end. At pop 200 a
//! generation costs about 14 ms, so the whole detour is over in well under a second,
//! while forty seconds of a *different level* on screen would bury the thing the
//! viewer is actually waiting for — the hard level's next generation. What the viewer
//! gets instead is [`CurriculumStep::note`], naming what happened, and a run whose
//! population has changed underneath it.
//!
//! **Specialist fusion (SMARTER_AI_PLAN.md Item 9).** A campaign leaves one clone per leg, each
//! fitted to a different crossing. [`fuse_specialists`] breeds those clones into each other with
//! the shipped uniform crossover, and [`fused_seeding`] offers the result beside the whole-level
//! genomes the caller already has. The experiment is *who the parents are*, not a new operator:
//! crossover between two genomes stuck at the same wall mixes two failures, while crossover
//! between two specialists mixes two successes. The gate is episodes to a validated win on
//! `l11-the-elements` and `l10-the-gauntlet-ii`, fused seeding against the campaign's existing
//! continuation (train the best single chain further) at equal total episodes. The kill criterion
//! is the same measurement the other way: if fusion does not beat continuation, segment
//! specialists do not compose, and the campaign keeps its current shape. Nothing here changes a
//! campaign's default behaviour — fusion is a caller's choice.
//!
//! The premise needs an *easier level*, and a level somebody drew by hand is not always
//! there: a run can stall on a level whose author is still sitting in front of it. What
//! this module derives instead is the run's own **next move** ([`practice_level`],
//! finding 50): the run's level with the spawn moved to the closest stand to the flag its
//! best genome has managed ([`stuck_point`]) and the flag moved to the stand the route
//! visits just past it ([`next_stand`]). Every episode of the detour is then the one
//! crossing the run is stuck on, rather than the whole chain — which is the one thing a
//! stalled population cannot practise from a spawn twenty platforms away.

use std::path::{Path, PathBuf};

use sim::rng::{next_f32, Rng};
use sim::{Level, Rect};

use crate::fitness::{EpisodeRunner, EvalConfig};
use crate::ga::Trainer;
use crate::genome::{crossover, Genome};

/// Generations spent on the easier level when a run stalls (§12 M5, finding 22).
///
/// Measured, not guessed: a 40-generation detour was the cheapest that rescued both
/// stalled seeds tried, and the longer ones were strictly worse on the seed that was
/// going to solve anyway (L5 seed 3 won at overall generation 100 with 40 detour
/// generations, and at 284 and 321 with 100 and 200).
///
/// It lives here rather than in either front-end because the terminal's
/// `--curriculum-generations` and the window's own knob are the same number: two
/// copies of a detour length are two chances for the two front-ends to disagree about
/// what a curriculum step costs.
pub const DEFAULT_CURRICULUM_GENERATIONS: u32 = 40;

/// What one curriculum step did, as numbers rather than as a sentence.
///
/// The numbers are here and the phrasing is below so that the terminal's log line and
/// the window's panel row cannot disagree about them: the two front-ends differ in
/// where they put the line, never in what it claims.
#[derive(Clone, Debug)]
pub struct CurriculumStep {
    /// Name of the easier level the population was trained on.
    pub easier_level: String,
    /// Generations actually spent there.
    pub generations: u32,
    /// The checkpoint written from the stuck population, before anything moved it.
    pub checkpoint: PathBuf,
    /// Best fitness reached during the excursion.
    pub best: f32,
    /// The easier level's own generation when the flag was first reached there.
    pub flag_at: Option<u32>,
}

impl CurriculumStep {
    /// The step as one line, for the terminal's log and the window's panel.
    ///
    /// Three facts, in the order the viewer needs them: how long the detour took, how
    /// it went, and that the run is back where it was. The flag clause says plainly
    /// when the easier level was *not* solved either — a detour that learned nothing
    /// is a result about the level, and hiding it would invite reading the note as
    /// "this run is fixed now".
    pub fn note(&self, main_level: &str) -> String {
        format!(
            "curriculum: {} generations on {} from the stalled population \
             (checkpoint -> {}, best there {:.2}, {}); back on {}",
            self.generations,
            self.easier_level,
            self.checkpoint.display(),
            self.best,
            match self.flag_at {
                Some(g) => format!("flag reached at its generation {g}"),
                None => "flag not reached there yet".to_string(),
            },
            main_level
        )
    }
}

/// One curriculum step: checkpoint the stuck population, train it on the easier level
/// for a while, and hand it back to the run it came from (§12 M5).
///
/// `checkpoint_path` is the caller's decision rather than this function's, because the
/// two front-ends name their artifacts with the same convention they use for champions
/// (`<level>.population.json` beside `<level>.json`), and a file named here would drift
/// from it. `stall_note` goes into the checkpoint's own note, so a resumed run can say
/// *why* the population exists (the stall that triggered it, in the front-end's words).
///
/// `mutation_rate` is applied to the excursion from its first generation:
/// [`Trainer::resume`] takes the config by value, and the rate the viewer has dialled
/// in is part of "the hyperparameters this run is using".
///
/// Errors only for real failures — a checkpoint that cannot be written, a population
/// that cannot be adopted. A detour that learns nothing is a result, not an error.
pub fn curriculum_step(
    trainer: &mut Trainer,
    main_level: &Level,
    easier: &Level,
    generations: u32,
    checkpoint_path: &Path,
    mutation_rate: f32,
    stall_note: &str,
) -> Result<CurriculumStep, String> {
    detour(
        trainer,
        Orders {
            main_level,
            excursion_levels: std::slice::from_ref(easier),
            easier_name: &easier.name,
            generations,
            checkpoint_path,
            mutation_rate,
            stall_note,
            timing_seeds: 0,
        },
    )
}

/// One practice step (finding 50): the derived practice level, with the run's own level
/// still in the detour's judgement, then the population handed back.
///
/// The detour is a two-level **suite** — `[practice, main]`, ranked by the worst of the
/// two — and that is the whole difference from [`curriculum_step`]. A population bred
/// only on the practice level is bred only for the move: nothing in a practice episode
/// needs the climb the run already earned, drift erodes it over forty generations, and
/// what comes back cannot reach the spot it has just learned to leave (measured — see
/// finding 50). Keeping the main level in the detour's ranking means every genome is
/// judged on *both* the climb and the next move, which is the one property the transfer
/// needs; `evaluate_suite`'s minimum is exactly the "hold both" rule.
///
/// The minimum only holds both when the two levels can *score* both, and that is what
/// the practice flag is for ([`practice_level`]): one move away, worth the same 10 000 as
/// a level win. A genome that makes the move is judged on the climb; a genome that keeps
/// the climb but loses the move falls back under the move's score. The first version of
/// this step left the flag at the level's own end, where a practice episode can only bank
/// the tail of the run's score — the minimum was then always the tail, the climb was bred
/// away every detour, and the run came back *worse* than it left (finding 50 has the
/// numbers).
///
/// The practice level is suite position 0: a suite counts `wins` on its first level, so
/// [`CurriculumStep::flag_at`] keeps meaning "the easier level was won there" — the move
/// being made — while a genome that wins *both* levels is the run's imminent win.
///
/// And the population that arrives there is seeded with the hop's lesson (finding 53):
/// jump-timing children of the stuck genome — one to three of its jump-decision genes
/// moved at the run's own step — take the weakest slots, so the detour's first generation
/// is choosing between takes on the crossing's timing instead of between clones of the
/// one take that stalled. [`curriculum_step`] does not do this: an authored easier level
/// is a whole landscape to re-learn, not one move to polish.
pub fn practice_step(
    trainer: &mut Trainer,
    main_level: &Level,
    practice: &Level,
    generations: u32,
    checkpoint_path: &Path,
    mutation_rate: f32,
    stall_note: &str,
) -> Result<CurriculumStep, String> {
    let levels = [practice.clone(), main_level.clone()];
    detour(
        trainer,
        Orders {
            main_level,
            excursion_levels: &levels,
            easier_name: &practice.name,
            generations,
            checkpoint_path,
            mutation_rate,
            stall_note,
            timing_seeds: PRACTICE_TIMING_SEEDS,
        },
    )
}

/// How many jump-timing seeds a practice detour starts with (finding 53).
///
/// Eight — the batch size the trainer's own reheating ships at
/// ([`crate::ga::GaConfig::immigrants`]) and the same fraction of a 200-strong population:
/// enough that the practice level's first generation holds several takes on the
/// crossing's timing to choose between, few enough that the detour is still the
/// population that got stuck. Capped at a quarter of the population whatever this says,
/// the same ceiling [`crate::ga::GaConfig::sanitized`] puts on the trainer's own
/// side-payments.
///
/// The count is a constant rather than a config field on purpose: `GaConfig` is written
/// into every champion file, and a knob nobody has swept does not belong in the format.
/// The measurement that decides the value is in finding 53.
///
pub const PRACTICE_TIMING_SEEDS: usize = 8;

/// Replace the population's weakest slots with jump-timing children of `anchor`: fine
/// mutations of its jump-decision genes, one to [`crate::ga::FINE_GENE_MAX`] of them at
/// the run's own mutation size.
///
/// This is the difference between a detour and a *lesson* (finding 53). The population
/// arrives at the practice level holding one take on the crossing — the one it stalled
/// with — and forty generations of ordinary breeding explore around that take at coarse
/// scale; a near-miss at a hop needs the same strategy with the take-off a tick earlier,
/// which is a boundary misplaced by a step (see [`crate::genome::mutate_jump_timing`]).
/// Seeding several such takes in the weakest slots means the practice level's very first
/// generation is choosing between timings instead of rediscovering the approach.
///
/// The ids are fresh and above every id the run has issued, so the excursion's own
/// breeding cannot hand them out twice; and the batch comes from its own random stream,
/// derived from the run's master seed, because the trainer's RNG is the run's measured
/// lineage and must not move because a detour seeded itself.
fn seed_jump_timing(
    population: &mut [Genome],
    anchor: &Genome,
    seeds: usize,
    sigma: f32,
    master_seed: u64,
) {
    let seed_count = seeds
        .min(population.len() / 4)
        .min(population.len().saturating_sub(1));
    if seed_count == 0 {
        return;
    }
    // The weakest slots make room: they are the least of what got stuck, and the practice
    // level is about to re-judge every slot anyway. The anchor itself is **not** a candidate:
    // it is the run's proven genome, and the one thing a lesson must not spend is the take that
    // can already win. It is put back exactly if it is absent, which is what carries the record
    // through a detour rather than only through the checkpoint.
    let anchor_at = population
        .iter()
        .position(|g| crate::ga::same_genes(g, anchor));
    let mut weakest: Vec<usize> = (0..population.len())
        .filter(|&i| Some(i) != anchor_at)
        .collect();
    weakest.sort_by(|&a, &b| population[a].fitness.total_cmp(&population[b].fitness));
    let mut rng = sim::rng_from_seed(master_seed ^ 0x7A11_7A11_71A1_5EED);
    let first_id = population.iter().map(|g| g.id).max().unwrap_or(0) + 1;
    let seed_slots: Vec<usize> = weakest.iter().take(seed_count).copied().collect();
    for (offset, &slot) in seed_slots.iter().enumerate() {
        let genes = 1 + (sim::rng::next_f32(&mut rng) * crate::ga::FINE_GENE_MAX as f32) as usize;
        let mut child = crate::genome::mutate_jump_timing(anchor, genes, sigma, &mut rng);
        child.id = first_id + offset as u32;
        population[slot] = child;
    }
    if anchor_at.is_none() {
        if let Some(&slot) = weakest.get(seed_slots.len()) {
            let mut copy = anchor.clone();
            copy.fitness = 0.0;
            copy.id = first_id + seed_slots.len() as u32;
            while population.iter().any(|g| g.id == copy.id) {
                copy.id += 1;
            }
            population[slot] = copy;
        }
    }
}

/// One detour's orders: where it goes, for how long, and what it takes with it.
///
/// A struct rather than eight arguments because the two steps that call [`detour`] fill in the
/// same list with different values, and "what a detour is told" is a thing that reads better
/// named than counted.
struct Orders<'a> {
    /// The run's own level: what the checkpoint is taken on, and what the population returns to.
    main_level: &'a Level,
    /// The levels the excursion trains on, in suite order — the practice level first where
    /// there is one.
    excursion_levels: &'a [Level],
    /// The name to report for the excursion: position 0 of `excursion_levels`.
    easier_name: &'a str,
    generations: u32,
    checkpoint_path: &'a Path,
    mutation_rate: f32,
    stall_note: &'a str,
    /// The practice step's lesson budget ([`PRACTICE_TIMING_SEEDS`]). The authored curriculum
    /// step passes 0 and keeps its documented contract — same genomes, same ids, same
    /// hyperparameters — because stepping back to an authored easier level is re-learning the
    /// whole landscape, not polishing one move.
    timing_seeds: usize,
}

/// The plumbing both steps share: checkpoint the stuck population, train it on the
/// excursion levels, adopt it back.
fn detour(trainer: &mut Trainer, orders: Orders<'_>) -> Result<CurriculumStep, String> {
    let Orders {
        main_level,
        excursion_levels,
        easier_name,
        generations,
        checkpoint_path,
        mutation_rate,
        stall_note,
        timing_seeds,
    } = orders;
    // The stuck population on disk, before anything moves it: this is the plan's
    // "checkpoint at the stuck point", and it is what makes an unrepeatable run
    // repeatable — `--resume` is the way back in.
    let checkpoint = crate::population::PopulationFile::new(
        &trainer.genomes,
        trainer.generation,
        main_level,
        &trainer.config,
        format!("stalled: {stall_note}"),
    )
    // The record travels with the cohort: a checkpoint without it can resume a solved run that
    // can no longer breed its own champion.
    .with_best(trainer.best_ever().map(|b| &b.genome));
    // A bare file name has an empty parent, and creating "" is an error rather than a
    // no-op, so the directory is only made when there is one.
    if let Some(dir) = checkpoint_path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
    {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    checkpoint.save(checkpoint_path)?;

    // A real training run on the easier level, from the population that is stuck:
    // same genomes, same ids (they are part of every episode seed), same
    // hyperparameters — including the mutation rate the viewer has dialled in.
    let mut cfg = trainer.config.clone();
    cfg.mutation_rate = mutation_rate;
    // The practice step's seeds come first and take the weakest slots, so the first
    // generation on the practice level is already choosing between takes on the hop's
    // timing rather than between clones of the one take that stalled (finding 53).
    let mut population = trainer.genomes.clone();
    if timing_seeds > 0 {
        let anchor = trainer
            .best_ever()
            .map(|b| b.genome.clone())
            .unwrap_or_else(|| {
                population
                    .iter()
                    .max_by(|a, b| a.fitness.total_cmp(&b.fitness))
                    .cloned()
                    .expect("a resumed population is never empty")
            });
        seed_jump_timing(
            &mut population,
            &anchor,
            timing_seeds,
            cfg.mutation_sigma,
            trainer.config.master_seed,
        );
    }
    let mut excursion = Trainer::resume(cfg, population, trainer.generation)?;
    let mut best = f32::NEG_INFINITY;
    let mut flag_at = None;
    for _ in 0..generations {
        let report = if excursion_levels.len() == 1 {
            excursion.step(&excursion_levels[0])
        } else {
            excursion.step_suite(excursion_levels)
        };
        best = best.max(report.max_fitness);
        if report.wins > 0 && flag_at.is_none() {
            flag_at = Some(excursion.generation);
        }
    }
    let spent = excursion.generation.saturating_sub(trainer.generation);
    // Carry the practice level's lesson back with the population (finding 54). Forty
    // generations of breeding on `[practice, main]` can leave the excursion's *best* genome
    // — the one that actually made the crossing and kept the climb — behind whatever the
    // last generation happened to rank first. Swapping it into the adopted population's
    // weakest slot hands the run the lesson itself rather than only its descendants.
    // Read before the population is moved out of the trainer: `best_ever` borrows it.
    let excursion_best = excursion.best_ever().cloned();
    let mut adopted = excursion.genomes;
    if timing_seeds > 0 && generations > 0 {
        if let Some(best) = excursion_best {
            let max_id = adopted
                .iter()
                .map(|g| g.id)
                .max()
                .unwrap_or(0)
                .max(trainer.next_id());
            let weakest = adopted
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.fitness.total_cmp(&b.fitness))
                .map(|(i, _)| i);
            if let Some(slot) = weakest {
                let mut transfer = best.genome;
                transfer.id = max_id + 1;
                transfer.fitness = 0.0;
                adopted[slot] = transfer;
            }
        }
    }
    trainer.adopt(adopted)?;

    Ok(CurriculumStep {
        easier_level: easier_name.to_string(),
        generations: spent,
        checkpoint: checkpoint_path.to_path_buf(),
        best,
        flag_at,
    })
}

/// The spawn tile a body standing at `(col, ty)` can actually start from.
///
/// Its centre is not always over its support: a body at a ledge's edge stands with its
/// feet on the last solid column and its centre past it, and a practice spawn dropped
/// into the gap would be refused (or fall). The ground it is overhanging is one tile
/// either side, and the spot itself wins when it is already standable.
fn standable_spawn(main: &Level, col: i32, ty: i32) -> Option<(i32, i32)> {
    [col, col - 1, col + 1]
        .into_iter()
        .find(|c| main.spawn_stands_on_ground(*c, ty))
        .map(|c| (c, ty))
}

/// How deep along the route a stand is, as one orderable key: **has a crossing ahead first**,
/// then crossings left to the flag, then straight-line approach.
///
/// The order matters and each of its three parts is a measurement rather than a taste.
///
/// * **A crossing ahead beats everything else.** A run whose furthest stand is the flag's own
///   ground has nothing left to practise — the detour would be the episode it just finished —
///   so a stand with a route step still in front of it is preferred over one without, even
///   when the latter reads closer. On a level whose last crossing is a pit, this is the
///   difference between practising the jump and being told there is nothing to practise.
/// * **Then the route's own count.** Straight-line distance is not monotone along a zig-zag
///   climb: up a leg that doubles back, the *next* platform reads farther from the flag than
///   the one the body is standing on, so an approach-only rule practises the ground just
///   climbed from (finding 50/51). Graph distance falls by exactly one per canonical
///   crossing, which is the ordering the practice level needs.
/// * **Then the straight line**, which only has to break ties between stands that share a
///   rung — and on a single-level floor, where every stand shares one rung, it *is* the rule,
///   which is what keeps a flat fixture's answer unchanged.
fn route_rank(routes: &sim::reach::GoalRoutes, stand: usize, approach: f32) -> StandRank {
    let hops = routes.hops_to_flag(stand).unwrap_or(u32::MAX);
    let crossing_ahead = hops > 0
        && routes
            .crossing_stands_from(stand)
            .any(|j| routes.hops_to_flag(j).is_some_and(|hj| hj < hops));
    StandRank {
        nothing_ahead: u8::from(!crossing_ahead),
        hops,
        approach_bits: approach.to_bits(),
    }
}

/// A stand's depth along the route, as one totally-ordered key (see [`route_rank`]).
///
/// A struct rather than a tuple because it has to be `Ord` and its third field is a distance:
/// the approach is kept as the **bit pattern** of a non-negative finite `f32`, which for those
/// values compares exactly as the number does and needs no float ordering (and no `total_cmp`
/// at every comparison in the loop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct StandRank {
    /// `0` when the stand still has a crossing toward the flag in front of it, `1` when it has
    /// none — so a stand with something left to learn outranks one without, whatever the ruler
    /// says.
    nothing_ahead: u8,
    /// Crossings left to the flag; `u32::MAX` where the flag cannot be reached at all.
    hops: u32,
    /// The straight-line distance to the flag, over the whole route's tie-breaking.
    approach_bits: u32,
}

/// Where a stalled run actually is: the deepest stand its genome has **stood** on, reported
/// as the spawn tile of the spot it stood in.
///
/// "Stood" is the operative word. [`EpisodeStats::closest_approach`](sim::EpisodeStats)
/// is the running minimum over *every* state the body passed through, including the apex
/// of a leap it then fell back from — a number it cannot keep and no landing will match.
/// Finding 50 is what that costs on a zig-zag climb: one free wall-jump apex outranked
/// the next four platforms of the route, so arriving on any of them scored no progress
/// and selection preferred the body that stayed where it was. This reads the body's own
/// position on the frames it is **grounded**, which is exactly "how far the run has
/// gotten", and it is the spot a detour should start from.
///
/// "Deepest" is [`route_rank`]'s ordering, which is the route's rather than the ruler's: a
/// stand is better than the spawn's when it is further along the level's move graph, even
/// where the straight line says otherwise.
///
/// `None` when the genome never stood anywhere better than it started — a run stuck on its
/// own spawn has no next move to practise, and a detour there would be the same episode it
/// is already failing.
pub fn stuck_point(
    level: &Level,
    genome: &Genome,
    eval: &EvalConfig,
    seed: u64,
) -> Option<(i32, i32)> {
    stall_spots(level, genome, eval, seed).map(|(_, tile)| tile)
}

/// Where a stalled run is, as **both** the route stand it has got furthest on and a tile it can be
/// spawned at.
///
/// Two answers because a stall can happen on a moving platform, and only one of the two is a tile a
/// spawn may be placed on. The **stand** is the deepest rung of the route the genome has actually
/// stood on — terrain or a deck — and it is what the next practice flag is measured from; the
/// **tile** is the deepest one of those that is *ground*, because [`Level::set_spawn_tile`] refuses
/// a spawn with nothing solid under it and a deck is not a tile.
///
/// Splitting them is what makes the stall chain work on a level whose route crosses a shuttle. With
/// one answer (the ground tile), a body that had ridden a deck and died on the next one was reported
/// as stalled at the ledge it left from, so every practice excursion re-taught the crossing the run
/// had already made — measured on the level in this finding: two excursions of 40 generations each,
/// both "tile (16, 12) to tile (18, 12)", while the champion was dying at `x` 35.8, two decks along.
fn stall_spots(
    level: &Level,
    genome: &Genome,
    eval: &EvalConfig,
    seed: u64,
) -> Option<(Option<usize>, (i32, i32))> {
    let mut runner = EpisodeRunner::new(level, genome.clone(), eval, seed);
    let routes = level.routes();
    // The episode opens on the spawn, so the first reading is the spawn's own: the
    // baseline the deepest stand has to beat.
    let start_approach = runner.sim().goal_approach();
    let spawn_surface = Level::tile_y(level.spawn.y + level.spawn.h + 0.05);
    let spawn_col = Level::tile_x(level.spawn.x + level.spawn.w * 0.5);
    let start = routes
        .zone_at(spawn_col, spawn_surface)
        .or_else(|| routes.zone_at(spawn_col - 1, spawn_surface))
        .or_else(|| routes.zone_at(spawn_col + 1, spawn_surface))
        .map(|s| route_rank(routes, s, start_approach))
        // A spawn that is not on a route stand at all: nothing to compare against, so any
        // stand the body reaches is an improvement.
        .unwrap_or(StandRank {
            nothing_ahead: 1,
            hops: u32::MAX,
            approach_bits: start_approach.to_bits(),
        });
    // `(rank, stand)`, lower rank wins — the route frontier, decks included.
    let mut best: Option<(StandRank, usize)> = None;
    // And the deepest *ground* tile, which is what a spawn may be placed on.
    let mut best_tile: Option<(StandRank, (i32, i32))> = None;
    while !runner.is_finished() {
        runner.step();
        let sim = runner.sim();
        let p = sim.player();
        if !p.grounded {
            continue;
        }
        // The same `(column, surface)` pair the sim's checkpoint counting stands the
        // body on; `set_spawn_tile` wants the row the body *occupies*, one above the
        // surface it stands on (`Level::set_spawn_tile`).
        let col = Level::tile_x(p.x + p.w * 0.5);
        let surface = Level::tile_y(p.y + p.h + 0.05);
        let Some(stand) = routes.zone_at(col, surface) else {
            continue;
        };
        let rank = route_rank(routes, stand, sim.goal_approach());
        if rank >= start {
            continue;
        }
        if best.is_none_or(|(b, _)| rank < b) {
            best = Some((rank, stand));
        }
        // A deck is a stand and not a tile: `standable_spawn` asks the grid, which has nothing
        // solid at a deck's row or above it.
        if let Some(tile) = standable_spawn(level, col, surface - 1) {
            if best_tile.is_none_or(|(b, _)| rank < b) {
                best_tile = Some((rank, tile));
            }
        }
    }
    Some((best.map(|(_, s)| s), best_tile.map(|(_, t)| t)?))
}

/// How close a body **standing** at `(col, surface_row)` is to the flag, in tiles.
///
/// The stand's own tile read against the goal's rect — the same reading [`stuck_point`]
/// takes off the episode's grounded frames. Since finding 50's tower measurement this
/// cannot decide what is *forward* (on a zig-zag it does not fall as the run climbs),
/// so [`next_stand`] gives it two smaller jobs: saying which way an equal-rung crossing
/// points, and breaking ties between forward stands.
fn stand_approach(goal: &Rect, col: i32, surface_row: i32) -> f32 {
    let a = Rect {
        x: col as f32,
        y: (surface_row - 1) as f32,
        w: 1.0,
        h: 1.0,
    };
    let dx = (goal.x - a.right()).max(a.x - goal.right()).max(0.0);
    let dy = (goal.y - a.bottom()).max(a.y - goal.bottom()).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// The next stand past `from`, toward the flag: the landing zone a stalled run has to
/// reach **next**, and the tile of it a body arriving from `from` lands on (finding 50).
///
/// "Next" is read off the **route**, with the straight line only breaking ties. A
/// candidate is one canonical crossing off the stand the run is stuck on that is not
/// *further* from the flag along the move graph ([`sim::reach::goal_routes`]): strictly
/// fewer crossings left is route progress, and the same count left is a stepping stone
/// — a shorter hop across what lies in front of the body (the near ledge of a wide pit)
/// that decomposes the stuck crossing without costing the run anything. A crossing that
/// leaves **more** crossings to the flag is the run being sent back the way it came.
///
/// The straight-line reading this replaced could not tell any of those apart. It asked
/// only which stand *reads* closer to the flag, and on a zig-zag that is wrong in both
/// directions — finding 50's tower arm caught it at work: a practice flag handed to the
/// ground the body had just climbed from (the route turns away from the flag up a leg,
/// so the stand behind reads closer), and a practice episode ordered three crossings
/// long (the stands up a leg read *farther* than its top, so the ones between them were
/// filtered out of the running). Graph distance is monotone along a route, so the
/// move-graph count decides what is forward; the straight line only orders the forward
/// moves that share a rung of it.
///
/// One crossing, by construction. Every stand returned here is the far end of a move
/// the game has, so the practice level this feeds ([`practice_level`]) is the one
/// crossing the run is stuck on rather than a chain of them — and a chosen stand is a
/// stand the body can actually reach, the guarantee the first version of this function
/// did not have.
///
/// The returned tile is the stand's row the body would occupy — the anchor
/// [`Level::set_goal_tile`] wants — chosen as the zone's tile closest to the stand the
/// body is coming from, which is where a jump aimed at the zone lands.
///
/// `None` when there is no next stand: the flag's own ground (walking to the flag is
/// not a crossing to practise), a stand with no route to the flag left (the honest
/// answer when the level cannot be won from here), or a level with no flag at all.
pub fn next_stand(main: &Level, from: (i32, i32)) -> Option<(i32, i32)> {
    let routes = sim::reach::goal_routes(main);
    let stuck = routes.zone_at(from.0, from.1 + 1)?;
    next_after(main, &routes, stuck)
}

/// [`next_stand`] asked from a **stand** rather than a tile.
///
/// The difference is a moving platform: a deck's stand has no tile that names it, because there is
/// nothing solid at its row, so a caller that has just been told "the deepest stand this genome stood
/// on is a deck" cannot express that as `(col, row)`. [`practice_level_for_stall`] needs exactly
/// that: the frontier of a run that has ridden a shuttle is a deck, and the crossing it has to
/// practise next is measured from it.
pub fn next_stand_from(main: &Level, stand: usize) -> Option<(i32, i32)> {
    next_after(main, &sim::reach::goal_routes(main), stand)
}

fn next_after(main: &Level, routes: &sim::reach::GoalRoutes, stuck: usize) -> Option<(i32, i32)> {
    let goal = main.goals.first()?;
    let (from_surface, from_x0, from_x1) = routes.zone_bounds(stuck);
    let fx = ((from_x0 + from_x1) as f32 * 0.5).floor() as i32;
    let from_approach = stand_approach(goal, fx, from_surface);
    let from_hops = routes.hops_to_flag(stuck)?;

    // Every stand the graph knows, not every standable tile of the terrain: since `MOVE_SET_VERSION`
    // 10 that set includes **both ends of a moving platform's travel**, and a stall at a shuttle's
    // ledge is the case this loop was blind to. The decks are not standable tiles — nothing solid is
    // drawn at their row — so a scan of the grid could never find the one stand the run has to
    // practise reaching, and `practice_level_for_stall` answered "no crossing toward the flag" on
    // every stall in front of a platform.
    //
    // `(distance to the stand the body is on, approach, tile)`: among the route's forward steps the
    // nearest wins, with the approach breaking a tie the way the ordering above is broken. A step is
    // a **crossing**: the one edge that is not (the ride, below) is skipped, because there is nothing
    // to press and the practice level would be a walk up a deck the body is already standing on.
    let mut best: Option<(f32, f32, (i32, i32))> = None;
    for zone in 0..routes.len() {
        if zone == stuck {
            continue;
        }
        // The other end of this stand's own ride is not a move to practise. Nothing is pressed to
        // get there (it is the cycle), and if it is the same deck the practice level would ask the
        // excursion to walk up a run of platform it is already standing on — measured on the
        // shuttle in this finding, the flag from a deck landed on the *same deck's* far end and the
        // level the run needed to practise was never built.
        if routes.ride_from(stuck) == Some(zone) {
            continue;
        }
        if !routes.crossing_stands_from(stuck).any(|j| j == zone) {
            continue;
        }
        let Some(hops) = routes.hops_to_flag(zone) else {
            continue;
        };
        let (surface, start, end) = routes.zone_bounds(zone);
        // Progress in the route's own units: strictly fewer crossings left to the
        // flag, or the same count and a stand that reads closer — a stepping stone
        // forward, never one back the way the run came.
        let forward = hops < from_hops
            || (hops == from_hops
                && (start..=end).any(|c| stand_approach(goal, c, surface) < from_approach));
        if !forward {
            continue;
        }
        for c in start..=end {
            let tile = (c, surface - 1);
            let d =
                (((c - fx) as f32).powi(2) + ((tile.1 - (from_surface - 1)) as f32).powi(2)).sqrt();
            let a = stand_approach(goal, c, surface);
            if best.is_none_or(|(bd, ba, _)| (d, a) < (bd, ba)) {
                best = Some((d, a, tile));
            }
        }
    }
    best.map(|(_, _, tile)| tile)
}

/// The run's own level from `spawn_tile`, with the flag moved to the next stand past it:
/// the landscape a stalled population practises its **next move** on (finding 50).
///
/// Two things move and nothing else: the spawn to the furthest stand the run has
/// managed, and the flag to the stand just beyond it ([`next_stand`]). The grid, the
/// walkers and everything else are the run's own, so the detour's episode is the run's
/// episode *from further along it*, and what the excursion learns is learnable in place.
///
/// The flag move is the mechanism of the whole feature, not a convenience. Winning the
/// practice level is worth the same 10 000 as winning the level, so the two-level suite
/// in [`practice_step`] can hold "made the move" and "kept the climb" at once. The first
/// version of this derivation left the flag at the level's own end; measured on the
/// zig-zag tower, a practice episode could then only bank the tail of the run's score
/// (best 230 against a climb worth 294), the suite's minimum was *always* the tail, and
/// forty generations of the detour bred the climb away — the run came back scoring less
/// on its own level than the population it left with. Finding 50 has the numbers.
///
/// The name carries both spots because the level is written beside the run's checkpoint
/// and a file called by the original name would be indistinguishable from the level it
/// was derived from.
///
/// Refuses a spawn tile the body cannot stand on (the level moved underneath the run)
/// and a stand with no crossing toward the flag (nothing to practise: the body is
/// already on the flag's own ground, or the flag cannot be reached from here at all).
/// The caller's reaction to either is the old behaviour — a stall with no fallback,
/// said out loud and left alone.
pub fn practice_level(main: &Level, spawn_tile: (i32, i32)) -> Result<Level, String> {
    practice_level_from(main, spawn_tile, None)
}

/// [`practice_level`] measured from the stand the run is **on**, which need not be the
/// stand the spawn tile is on.
///
/// The spawn has to be ground and the frontier need not be: a run that has ridden a
/// shuttle and died on the deck past it is stuck on a deck, and the crossing to practise is
/// the one off *that* deck. Asking [`next_stand`] about the spawn tile instead answers the
/// crossing it made two decks ago — the practice flag lands behind the champion, the
/// excursion wins trivially, and the excursion teaches nothing (the stall chain repeated
/// `(16, 12) to (18, 12)` while the champion died at `x` 35.8; see [`stall_spots`]).
pub fn practice_level_from(
    main: &Level,
    spawn_tile: (i32, i32),
    from_stand: Option<usize>,
) -> Result<Level, String> {
    let (tx, ty) = spawn_tile;
    if !main.spawn_stands_on_ground(tx, ty) {
        return Err(format!(
            "practice spawn at tile ({tx}, {ty}) does not stand on ground in {}",
            main.name
        ));
    }
    let next = match from_stand {
        Some(stand) => next_stand_from(main, stand),
        None => next_stand(main, spawn_tile),
    };
    let Some((gx, gy)) = next else {
        return Err(format!(
            "no crossing toward the flag from tile ({tx}, {ty}) in {}",
            main.name
        ));
    };
    let mut practice = main.clone();
    practice.set_spawn_tile(tx, ty);
    // The old flag is *moved*, through the same mutator the editor's goal brush uses,
    // so the derived level cannot be one the rules would refuse to make. It is taken off
    // first because `set_goal_tile` refuses a marker that would cover the goal already
    // there — the move is one operation in the editor and two here.
    if practice.goal_tiles().first().copied() != Some((gx, gy)) {
        for (ox, oy) in practice.goal_tiles() {
            practice.remove_goal_at(ox, oy);
        }
        if !practice.set_goal_tile(gx, gy) {
            return Err(format!(
                "the practice flag will not stand at tile ({gx}, {gy}) in {}",
                main.name
            ));
        }
    }
    practice.name = format!(
        "{} practice at tile ({tx}, {ty}) to tile ({gx}, {gy})",
        main.name
    );
    Ok(practice)
}

/// Write a derived practice level beside the run's checkpoint
/// (`x.population.json` becomes `x.practice.json`), so the detour's landscape is an
/// artifact somebody can open and play rather than a sentence in a note.
///
/// The name is derived from the checkpoint path rather than passed in, for the same
/// reason [`curriculum_step`] does not name its own checkpoint: two front-ends with two
/// naming conventions are two chances to disagree about which file is which.
pub fn save_practice_level(practice: &Level, checkpoint_path: &Path) -> Result<PathBuf, String> {
    // `<stem>.population.json` becomes `<stem>.practice.json`. `Path::with_extension`
    // is not this: it replaces only the *last* extension and would name the file
    // `<stem>.population.practice.json`, which is a name nobody would look for.
    let file = checkpoint_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = file.strip_suffix(".population.json").unwrap_or_else(|| {
        Path::new(&file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
    });
    let path = checkpoint_path.with_file_name(format!("{stem}.practice.json"));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    practice.to_json_file(&path)?;
    Ok(path)
}

/// The practice level a stalled run falls back to when it has no authored easier level:
/// its own level, starting where its best genome has stood furthest and ending at the
/// next stand past it (finding 50).
///
/// `Err` names why there is nothing to practise from: no generation scored yet, the
/// best genome never stood anywhere it did not start from, or the route from where it is
/// stuck has no crossing toward the flag. The caller's reaction is the old behaviour — a
/// stall with no fallback — but said out loud: the run is told why it stays where it is,
/// because a silent skip is indistinguishable from a dead stall chain.
pub fn practice_level_for_stall(trainer: &Trainer, main: &Level) -> Result<Level, String> {
    let best = trainer.best_ever().ok_or_else(|| {
        "no generation has scored yet, so there is nowhere to practise from".to_string()
    })?;
    let eval = trainer.config.eval_config();
    let (stand, spot) =
        stall_spots(main, &best.genome, &eval, best.result.seed).ok_or_else(|| {
            "the best genome has never stood anywhere it did not start from".to_string()
        })?;
    practice_level_from(main, spot, stand)
}

/// **One leg of a planned route, as a level that can be practised (WP4).**
///
/// The unit is the one the stall detour already proved: a single crossing with both its entry and
/// its exit in place, not the whole level. Errors in a whole-route clone compound at the first
/// imperfect block; a segment is short enough that the student's own states stay near the
/// demonstration it was fitted on.
///
/// The segment carries the planner's block actions for its own span, so a caller can fit a
/// demonstration (or seed a population) without re-running the search. `level` is the same world
/// with its spawn moved to `entry` and its flag moved to `exit`; nothing else about the geometry
/// changes.
#[derive(Clone, Debug)]
pub struct PlanSegment {
    pub index: usize,
    pub entry: (i32, i32),
    pub exit: (i32, i32),
    /// First decision block of the route this segment covers (inclusive).
    pub start_block: usize,
    /// One past the last block (exclusive).
    pub end_block: usize,
    /// Frames the planned route had used when the segment ended.
    pub frames: u32,
    /// The block actions to hold through the segment.
    pub actions: Vec<sim::physics::AgentAction>,
    pub level: Level,
}

impl PlanSegment {
    pub fn blocks(&self) -> usize {
        self.end_block.saturating_sub(self.start_block)
    }
}

/// Build a practice level from an arbitrary entry tile to an arbitrary exit tile.
///
/// [`practice_level_from`] always ends at the *next* route rung, which is right for a stall and
/// wrong for a planned segment that may span several. This is the same construction with the
/// destination named: the spawn is validated as ground, the old flag is removed through the same
/// mutator the editor uses, and the new one is placed with `set_goal_tile` so a level the rules
/// would refuse can never be produced here.
pub fn practice_level_between(
    main: &Level,
    spawn_tile: (i32, i32),
    goal_tile: (i32, i32),
) -> Result<Level, String> {
    let (tx, ty) = spawn_tile;
    if !main.spawn_stands_on_ground(tx, ty) {
        return Err(format!(
            "segment spawn at tile ({tx}, {ty}) does not stand on ground in {}",
            main.name
        ));
    }
    let mut practice = main.clone();
    practice.set_spawn_tile(tx, ty);
    if practice.goal_tiles().first().copied() != Some(goal_tile) {
        for (ox, oy) in practice.goal_tiles() {
            practice.remove_goal_at(ox, oy);
        }
        if !practice.set_goal_tile(goal_tile.0, goal_tile.1) {
            return Err(format!(
                "the segment flag will not stand at tile ({}, {}) in {}",
                goal_tile.0, goal_tile.1, main.name
            ));
        }
    }
    practice.name = format!(
        "{} segment ({tx}, {ty}) to ({}, {})",
        main.name, goal_tile.0, goal_tile.1
    );
    Ok(practice)
}

/// Split a planned route at the **route stands it actually lands on**, and derive one practice
/// level per leg (WP4).
///
/// Boundaries are only ground stands: a deck has no tile a spawn can be placed on, so a hand-off
/// on a moving platform would produce a level the engine refuses. A route whose only intermediate
/// stands are decks therefore yields one segment (spawn to flag), which is the honest answer —
/// there is nowhere ground-based to hand over.
///
/// The split is deterministic: it reads the planner's own block actions, replays them at the
/// policy cadence, and records the first block that lands on each new ground stand. No path is
/// invented and no stand is assumed; a level the search could not route returns `Err` rather than
/// an empty campaign.
pub fn plan_segments(
    main: &Level,
    route: &sim::plan::TimedRoute,
) -> Result<Vec<PlanSegment>, String> {
    let goal = main
        .goal_tiles()
        .first()
        .copied()
        .ok_or_else(|| format!("{} has no goal flag to route to", main.name))?;
    let spawn_col = Level::tile_x(main.spawn.x + main.spawn.w * 0.5);
    // The body's own cell, not the floor under it: `spawn_stands_on_ground` asks about the empty
    // cell the player occupies, while the route stands are described by their *surface* row.
    let spawn_row = Level::tile_y(main.spawn.y + main.spawn.h - 0.05);
    let entry = (spawn_col, spawn_row);
    if !main.spawn_stands_on_ground(entry.0, entry.1) {
        return Err(format!(
            "{}'s spawn does not stand on ground, so no segment can start there",
            main.name
        ));
    }

    let routes = sim::reach::goal_routes(main);
    // `(block, body tile, goal tile, frames)`: the hand-off points in route order. The two tiles
    // differ by one row — a spawn occupies the empty cell above the surface a flag's pole stands
    // on — and keeping them apart is what stops a segment flag being placed in mid-air.
    type Mark = (usize, (i32, i32), (i32, i32), u32);
    let mut marks: Vec<Mark> = vec![(0, entry, (entry.0, entry.1 + 1), 0)];
    let mut last_stand = routes.zone_at(entry.0, entry.1 + 1);
    let repeat = crate::ACTION_REPEAT.max(1);
    let mut sim = sim::Sim::new(main, route.seed);
    for (block, action) in route.actions.iter().enumerate() {
        for _ in 0..repeat {
            if sim.outcome().is_finished() {
                break;
            }
            sim.step(*action);
        }
        if let Some(stand) = sim.route_stand() {
            if Some(stand) != last_stand {
                if let Some((body, goal)) = ground_tile_for_stand(main, &routes, stand) {
                    // A hand-off that lands back where the previous one did is not a boundary.
                    if marks.last().is_none_or(|(_, _, held, _)| *held != goal) {
                        marks.push((block + 1, body, goal, sim.stats().frames));
                    }
                    last_stand = Some(stand);
                }
            }
        }
        if sim.outcome().is_finished() {
            break;
        }
    }
    // The flag is the last exit even when the route's final stand is a deck: the goal tile is
    // always a legal place to put a practice flag.
    if marks.last().is_none_or(|(_, _, held, _)| *held != goal) {
        marks.push((
            route.actions.len(),
            (goal.0, goal.1 - 1),
            goal,
            route.frames,
        ));
    }
    if marks.len() < 2 {
        return Err(format!(
            "the planned route in {} has only one ground stand; there is no segment boundary",
            main.name
        ));
    }

    let mut out = Vec::with_capacity(marks.len() - 1);
    for (i, pair) in marks.windows(2).enumerate() {
        let (start_block, entry, _, _) = pair[0];
        let (end_block, _, exit, frames) = pair[1];
        if exit == entry {
            continue;
        }
        let level = practice_level_between(main, entry, exit)?;
        out.push(PlanSegment {
            index: i,
            entry,
            exit,
            start_block,
            end_block,
            frames,
            actions: route
                .actions
                .get(start_block..end_block)
                .unwrap_or(&[])
                .to_vec(),
            level,
        });
    }
    if out.is_empty() {
        return Err(format!(
            "the planned route in {} has no non-empty ground segment",
            main.name
        ));
    }
    for (i, segment) in out.iter_mut().enumerate() {
        segment.index = i;
    }
    Ok(out)
}

/// The ground tile inside a route stand, or `None` for a deck.
///
/// `zone_bounds` names a surface row and a column span; a deck's span has nothing solid at that
/// row, and that is exactly the predicate [`Level::spawn_stands_on_ground`] answers.
fn ground_tile_for_stand(
    main: &Level,
    routes: &sim::reach::GoalRoutes,
    stand: usize,
) -> Option<((i32, i32), (i32, i32))> {
    // A stand's row is the *surface* row (the floor the body stands on); a spawn tile is the
    // empty cell above it. Asking `spawn_stands_on_ground` about the surface row itself would
    // always answer no, because that cell is solid.
    let (surface, x0, x1) = routes.zone_bounds(stand);
    let body_row = surface - 1;
    (x0..=x1).find_map(|col| {
        main.spawn_stands_on_ground(col, body_row)
            .then_some(((col, body_row), (col, surface)))
    })
}

/// The decisions a planned segment demonstrates, observed from the state the route reaches at
/// the segment's first block.
///
/// This is the whole-route clone cut to the leg without cutting the *observations*: the prefix is
/// replayed on the full level so the body is at the right state, then the segment's own block
/// actions are recorded at the policy cadence. A caller that fitted this against the practice
/// level instead would be labelling states by a world that starts at the segment, which is not
/// where the student will meet them.
fn segment_demonstration(
    main: &Level,
    route: &sim::plan::TimedRoute,
    segment: &PlanSegment,
) -> crate::route_play::RoutePlay {
    let repeat = crate::ACTION_REPEAT.max(1);
    let mut sim = sim::Sim::new(main, route.seed);
    for action in route.actions.iter().take(segment.start_block) {
        for _ in 0..repeat {
            if sim.outcome().is_finished() {
                break;
            }
            sim.step(*action);
        }
    }
    let mut decisions = Vec::new();
    for action in &segment.actions {
        if sim.outcome().is_finished() {
            break;
        }
        decisions.push(crate::route_play::Decision {
            observation: sim.observation(),
            action: *action,
        });
        for _ in 0..repeat {
            if sim.outcome().is_finished() {
                break;
            }
            sim.step(*action);
        }
    }
    crate::route_play::RoutePlay {
        decisions,
        segments: vec![0],
        outcome: sim.outcome(),
        frames: sim.stats().frames,
        seed: route.seed,
        hops_closed: sim.stats().route_hops_closed,
        best_block: segment.blocks(),
        best_stand: sim.route_stand(),
        route_hops: Some(sim.stats().route_hops_closed),
        note: format!(
            "segment {} demonstration: {} blocks on {} from block {}",
            segment.index + 1,
            segment.blocks(),
            main.name,
            segment.start_block
        ),
        trace: Vec::new(),
    }
}

/// What a segment campaign did.
#[derive(Clone, Debug)]
pub struct SegmentCampaign {
    pub segments: usize,
    pub steps: Vec<CurriculumStep>,
    /// The main level's record after the campaign, when the caller trains on it afterwards.
    pub solved: bool,
    /// The per-leg clones the campaign fitted, in leg order — one per segment whose
    /// demonstration produced a genotype. Item 9's raw material: each is a specialist for its
    /// own crossing and nothing else, which is what makes crossing two of them an interesting
    /// parent choice rather than a different draw from the same population.
    pub specialists: Vec<Genome>,
}

impl SegmentCampaign {
    /// The leg specialists, in leg order.
    pub fn specialists(&self) -> &[Genome] {
        &self.specialists
    }
}

/// The knobs of specialist fusion (Item 9).
///
/// Deliberately two: how many children to breed, and how often a child is a cross rather than a
/// copy. The crossover itself — per-gene uniform — is [`crate::genome::crossover`], unchanged.
/// Fusion is about who the parents are, and a new operator would confound the two questions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FuseConfig {
    /// Maximum fused children to breed. Zero is a legal request for none.
    pub shares: usize,
    /// Probability that a child is a cross of two specialists rather than a copy of one. `1.0`
    /// — the default — crosses every child, which is the arm the item describes. Lower values
    /// are for a caller that wants the specialists themselves to survive the seeding intact.
    pub cross_rate: f32,
}

impl Default for FuseConfig {
    fn default() -> Self {
        FuseConfig {
            shares: 8,
            cross_rate: 1.0,
        }
    }
}

impl FuseConfig {
    /// Clamp the knobs into the range the arithmetic needs.
    pub fn sanitized(mut self) -> FuseConfig {
        self.shares = self.shares.min(1_000);
        self.cross_rate = self.cross_rate.clamp(0.0, 1.0);
        self
    }
}

/// **Breed the leg specialists into each other** (Item 9).
///
/// Every child is a cross of two *distinct* specialists (or, below `cross_rate` 1.0, a copy of
/// one), using the shipped per-gene uniform crossover. Parents are walked in route order with a
/// rotating partner, so two specialists that met at adjacent legs are crossed first and a
/// campaign of `n` legs still produces all `n` as a first parent before any repeats. Fewer than
/// two specialists yields no children rather than an error: one leg has nothing to compose.
///
/// **Ids belong to the caller.** [`crossover`] copies the first parent's id, and this function
/// leaves it there: the trainer issues ids (they are part of every episode seed), exactly as it
/// does for `mutate`, `reheat` and the archive's children. Fitness is cleared, because a child
/// has not been judged.
///
/// The parents are read only; fusion never mutates its inputs.
pub fn fuse_specialists(specialists: &[Genome], cfg: &FuseConfig, rng: &mut Rng) -> Vec<Genome> {
    let cfg = cfg.sanitized();
    if specialists.len() < 2 || cfg.shares == 0 {
        return Vec::new();
    }
    let n = specialists.len();
    let mut out = Vec::with_capacity(cfg.shares.min(n * 2).max(1));
    for i in 0..cfg.shares {
        let pa = i % n;
        // Distinct by construction (`n >= 2`), and adjacent in route order on the first lap,
        // which is the pairing the item is about: neighbour legs hand the body over to each
        // other, so a cross of them is a whole crossing rather than two unrelated tricks.
        let pb = (pa + 1 + (i / n) % (n - 1)) % n;
        let child = if next_f32(rng) < cfg.cross_rate {
            crossover(&specialists[pa], &specialists[pb], rng)
        } else {
            let mut copy = specialists[pa].clone();
            copy.fitness = 0.0;
            copy
        };
        out.push(child);
    }
    out
}

/// The seeding list a fused campaign offers: fused children first, then the caller's whole-level
/// genomes, without duplicates by gene equality.
///
/// Order is deterministic and meaningful. The fused children lead because they are the arm being
/// tested; the whole-level genomes follow as the continuation's own material, so a caller that
/// takes the first *k* slots is running "fusion first, continuation after" rather than an
/// interleaving nobody could attribute a win to. A genome whose genes already appear earlier is
/// dropped, which is what stops a specialist that is *also* the campaign's best whole-level
/// genome from spending two slots.
pub fn fused_seeding(
    campaign: &SegmentCampaign,
    best_whole: &[Genome],
    cfg: &FuseConfig,
    rng: &mut Rng,
) -> Vec<Genome> {
    let specialists = campaign.specialists();
    let mut out: Vec<Genome> = Vec::new();
    for genome in fuse_specialists(specialists, cfg, rng)
        .into_iter()
        .chain(best_whole.iter().cloned())
    {
        // A candidate already in the output, or one that is just a leg specialist offered
        // again as a whole-level genome, spends a slot on a policy the campaign already has.
        if out.iter().any(|held| same_genes(held, &genome))
            || specialists.iter().any(|s| same_genes(s, &genome))
        {
            continue;
        }
        out.push(genome);
    }
    out
}

/// Exact gene equality, for de-duplication: ids and recorded fitness are not genes, and two
/// copies of one policy are one candidate however they are labelled.
fn same_genes(a: &Genome, b: &Genome) -> bool {
    a.shape == b.shape
        && a.weights.len() == b.weights.len()
        && a.biases.len() == b.biases.len()
        && a.weights
            .iter()
            .zip(&b.weights)
            .all(|(x, y)| x.to_bits() == y.to_bits())
        && a.biases
            .iter()
            .zip(&b.biases)
            .all(|(x, y)| x.to_bits() == y.to_bits())
}

/// **Segment chaining (WP4):** train each leg as a practice level with the main level in the
/// suite, in route order.
///
/// This is [`practice_step`] repeated over [`plan_segments`], and the repetition is the design:
/// every leg is judged on `[segment, main]` ranked worst-of-two, so a policy that graduates from
/// segment *k* has not been allowed to forget segment *k-1* or the original level. A fresh
/// population is not created between legs; the same run carries its record and elite slots, and
/// [`Trainer::adopt`] re-inserts the record if an excursion's survivors dropped it.
///
/// `checkpoint` is the base path for the per-segment population files; each leg writes one beside
/// it (`<stem>.segment-01.population.json`), so a campaign that is stopped mid-way is inspectable
/// leg by leg.
pub fn segment_campaign(
    trainer: &mut Trainer,
    main: &Level,
    route: &sim::plan::TimedRoute,
    segments: &[PlanSegment],
    generations_per_segment: u32,
    checkpoint: &Path,
    mutation_rate: f32,
) -> Result<SegmentCampaign, String> {
    if segments.is_empty() {
        return Err("a segment campaign needs at least one segment".into());
    }
    let mut steps = Vec::with_capacity(segments.len());
    let mut specialists: Vec<Genome> = Vec::new();
    for segment in segments {
        // **The demonstration for this leg only.** The plan's step 2 is explicit that a segment is
        // seeded from the planner's own actions over that span, not from a whole-route clone: the
        // whole-route clone's failures are what this package exists to avoid. The clone is fitted
        // from observations the full level actually had at those blocks, then injected into the
        // population as one warm parent; the run's record and its other genomes are untouched.
        let play = segment_demonstration(main, route, segment);
        if !play.decisions.is_empty() {
            let fit = crate::imitate::ImitateConfig {
                epochs: 200,
                prefer_agreement: true,
                ..crate::imitate::ImitateConfig::default()
            };
            if let Ok(report) =
                crate::imitate::clone_from_play(&play, trainer.config.shape.clone(), &fit)
            {
                // Keep one copy as Item 9's raw material and inject the other as a warm parent:
                // the campaign's default run is untouched, and fusion is a caller's choice.
                specialists.push(report.genome.clone());
                let _ = trainer.inject(report.genome);
            }
        }
        let stem = checkpoint
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "campaign".into());
        let path = checkpoint.with_file_name(format!(
            "{stem}.segment-{:02}.population.json",
            segment.index + 1
        ));
        let note = format!(
            "segment {}/{}: ({}, {}) to ({}, {}) over {} blocks",
            segment.index + 1,
            segments.len(),
            segment.entry.0,
            segment.entry.1,
            segment.exit.0,
            segment.exit.1,
            segment.blocks()
        );
        let step = practice_step(
            trainer,
            main,
            &segment.level,
            generations_per_segment,
            &path,
            mutation_rate,
            &note,
        )?;
        steps.push(step);
    }
    Ok(SegmentCampaign {
        segments: segments.len(),
        steps,
        solved: trainer.best_ever().is_some_and(|b| b.result.solved()),
        specialists,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{flat_runway, gap_runway};
    use crate::ga::GaConfig;

    /// A population small enough that a detour costs milliseconds. The GA's own
    /// behaviour is `ga`'s business; what is tested here is that the step writes what
    /// it says it wrote and hands back what it took.
    fn tiny() -> GaConfig {
        GaConfig {
            pop_size: 4,
            elite: 1,
            tournament: 2,
            mutation_rate: 0.10,
            master_seed: 3,
            ..GaConfig::default()
        }
    }

    /// A directory of this run's own, so two tests running at once cannot write over
    /// one another's checkpoint.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evo-curriculum-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("a temp directory");
        dir
    }

    /// The population after one real generation: a trained population with ids, which
    /// is the thing a stall actually happens to.
    fn trained(main: &Level, config: &GaConfig) -> Trainer {
        let mut trainer = Trainer::new(config.clone());
        trainer.step(main);
        trainer
    }

    fn genome_ids(trainer: &Trainer) -> Vec<u32> {
        let mut ids: Vec<u32> = trainer.genomes.iter().map(|g| g.id).collect();
        ids.sort_unstable();
        ids
    }

    /// Every weight in the population, in order: the cheap way to ask "is this the
    /// same population?" without depending on how breeding happens to order it.
    fn genome_weights(trainer: &Trainer) -> Vec<f32> {
        trainer
            .genomes
            .iter()
            .flat_map(|g| g.weights.iter().copied())
            .collect()
    }

    #[test]
    fn a_practice_detour_starts_with_jump_timing_seeds_around_the_stuck_genome() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        // Fine children off: they are uniform 1–3 gene moves of the best, and one whose
        // draw happens to land in the jump rows would look exactly like a timing seed.
        let config = GaConfig {
            pop_size: 16,
            fine_children: 0,
            ..tiny()
        };
        let mut trainer = trained(&main, &config);
        let old_ids = genome_ids(&trainer);
        let anchor = trainer
            .best_ever()
            .expect("a scored population has a best")
            .genome
            .clone();
        let eval = config.eval_config();
        let spot = stuck_point(
            &main,
            &anchor,
            &eval,
            trainer.best_ever().expect("above").result.seed,
        )
        .expect("the run stood somewhere");
        let practice = practice_level(&main, spot).expect("somewhere to practise");

        let dir = temp_dir("timing-seeds");
        let checkpoint = dir.join("hard.population.json");
        // Zero generations out and back: what the test reads is exactly the population
        // the practice level's first generation is about to see.
        practice_step(&mut trainer, &main, &practice, 0, &checkpoint, 0.10, "test")
            .expect("the detour runs");

        let slots = crate::genome::jump_timing_slots(&anchor.shape);
        let is_timing_seed = |g: &Genome| {
            let n_w = anchor.weights.len();
            let mut moved: Vec<usize> = (0..n_w)
                .filter(|&i| anchor.weights[i] != g.weights[i])
                .collect();
            moved.extend(
                (0..anchor.biases.len())
                    .filter(|&i| anchor.biases[i] != g.biases[i])
                    .map(|i| n_w + i),
            );
            !moved.is_empty() && moved.len() <= 3 && moved.iter().all(|m| slots.contains(m))
        };
        let seeds: Vec<u32> = trainer
            .genomes
            .iter()
            .filter(|g| is_timing_seed(g))
            .map(|g| g.id)
            .collect();
        // Eight asked for ([`PRACTICE_TIMING_SEEDS`]), and the quarter-of-population
        // ceiling answers four at pop 16 — the same ceiling the trainer puts on its own
        // side-payments.
        assert_eq!(seeds.len(), 4, "the weakest slots hold the timing takes");
        assert!(
            seeds.iter().all(|id| !old_ids.contains(id)),
            "the seeds carry fresh ids the excursion cannot hand out twice"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_step_writes_a_checkpoint_that_loads_back_with_the_stuck_population() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        let easier = flat_runway("easy", 24, 22);
        let config = tiny();
        let mut trainer = trained(&main, &config);
        let ids = genome_ids(&trainer);
        let generation = trainer.generation;

        let dir = temp_dir("checkpoint");
        let path = dir.join("hard.population.json");
        let step = curriculum_step(
            &mut trainer,
            &main,
            &easier,
            2,
            &path,
            0.10,
            "no new best in a long time",
        )
        .expect("a step against a real level succeeds");

        assert_eq!(step.checkpoint, path);
        assert!(path.exists(), "the stuck population is written down");
        let checkpoint =
            crate::population::PopulationFile::load(&path).expect("the checkpoint loads back");
        assert_eq!(checkpoint.generation, generation);
        let mut written = checkpoint.genome_ids.clone();
        written.sort_unstable();
        assert_eq!(
            written, ids,
            "the checkpoint is the population it came from"
        );
        assert!(
            checkpoint.note.contains("stalled"),
            "the checkpoint records why it exists: {:?}",
            checkpoint.note
        );
        assert!(
            checkpoint.note.contains("no new best in a long time"),
            "the stall's own words are kept: {:?}",
            checkpoint.note
        );
        assert!(
            checkpoint.best.is_some(),
            "a checkpoint carries the run's best-ever genome, so the excursion can hand it back"
        );
        // The level it was taken on is the *main* level, not the one it was just
        // trained on: a resumed run has to be told which level the numbers mean.
        assert!(checkpoint.level_note(&easier).is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_adopted_population_can_still_be_bred_without_issuing_an_id_twice() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        let easier = flat_runway("easy", 24, 22);
        let config = tiny();
        let mut trainer = trained(&main, &config);

        let dir = temp_dir("ids");
        let path = dir.join("hard.population.json");
        curriculum_step(&mut trainer, &main, &easier, 2, &path, 0.10, "stalled")
            .expect("a step succeeds");

        // Genome ids are part of every episode seed, so the run has to be able to
        // carry on breeding without handing out an id that is already in the
        // population — which is what `adopt` maintains and what this checks.
        let mut ids = genome_ids(&trainer);
        let unique = {
            let before = ids.len();
            ids.dedup();
            before == ids.len()
        };
        assert!(unique, "the adopted population has duplicate ids: {ids:?}");
        assert!(
            trainer.next_id() > ids.iter().copied().max().unwrap_or(0),
            "the next id {} is not past the highest adopted id",
            trainer.next_id()
        );
        let next = trainer.step(&main);
        assert!(
            next.results.len() == config.pop_size,
            "the run breeds on from the adopted population"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_step_hands_a_changed_population_back_and_leaves_the_runs_clock_alone() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        let easier = flat_runway("easy", 24, 22);
        let config = tiny();
        let mut trainer = trained(&main, &config);
        let before = genome_weights(&trainer);
        let generation = trainer.generation;

        let dir = temp_dir("changed");
        let path = dir.join("hard.population.json");
        let step = curriculum_step(&mut trainer, &main, &easier, 3, &path, 0.10, "stalled")
            .expect("a step succeeds");

        // The detour's own accounting: three generations were spent, all of them on
        // the easier level.
        assert_eq!(step.generations, 3);
        assert_eq!(step.easier_level, easier.name);
        // The run's own clock is untouched: a detour is not progress on the hard
        // level, and a generation number that jumped by the length of the excursion
        // would claim that three generations of the main run had happened.
        assert_eq!(
            trainer.generation, generation,
            "the excursion must not advance the run's own generation counter"
        );
        // And what comes back is a different population: an excursion on a level with
        // any signal at all has to move it, or the step is a very expensive no-op.
        assert_ne!(
            genome_weights(&trainer),
            before,
            "the population came back unchanged, so the detour did nothing"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_note_says_how_long_it_took_where_it_went_and_that_the_run_is_back() {
        let step = CurriculumStep {
            easier_level: "l1-flat-run".into(),
            generations: 40,
            checkpoint: PathBuf::from("champions/l5.population.json"),
            best: 812.345,
            flag_at: Some(163),
        };
        let note = step.note("l5-tightrope");
        // The fitness is compared against the struct's own number rather than a literal:
        // `{:.2}` of 812.345 is 812.34 or 812.35 depending on which side of the decimal
        // the value's nearest `f32` sits, and a test that pinned one of them would be
        // asserting a rounding rather than the note.
        let best = format!("best there {:.2}", step.best);
        for wanted in [
            "40 generations",
            "l1-flat-run",
            "l5-tightrope",
            "checkpoint -> champions/l5.population.json",
            best.as_str(),
            "flag reached at its generation 163",
        ] {
            assert!(note.contains(wanted), "{note:?} omits {wanted:?}");
        }

        // A detour that solved nothing says so rather than leaving the clause out:
        // the absence of a flag is a fact about the easier level too.
        let unsolved = CurriculumStep {
            flag_at: None,
            ..step
        };
        assert!(unsolved
            .note("l5-tightrope")
            .contains("flag not reached there yet"));
    }

    #[test]
    fn a_run_with_no_population_to_checkpoint_is_an_error_rather_than_a_silent_no_op() {
        // The default rung of the ladder, so the constant the two front-ends share is
        // the one this module's docs promise: a detour long enough to matter.
        assert_eq!(DEFAULT_CURRICULUM_GENERATIONS, 40);

        let main = gap_runway("hard", 16, 14, &[(6, 3)]);
        let easier = flat_runway("easy", 16, 14);
        // Writing into a path whose parent is a *file* cannot work, and the failure
        // has to come back as an error the front-end can put on screen.
        let dir = temp_dir("unwritable");
        let file = dir.join("not-a-directory");
        std::fs::write(&file, b"").expect("a file where a directory should be");
        let mut trainer = trained(&main, &tiny());
        let err = curriculum_step(
            &mut trainer,
            &main,
            &easier,
            1,
            &file.join("hard.population.json"),
            0.05,
            "stalled",
        )
        .expect_err("a checkpoint that cannot be written is an error");
        assert!(err.contains("cannot create"), "{err:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // the derived practice level (finding 50)
    // -----------------------------------------------------------------------

    /// A ground floor with the flag floating high over the spawn: a body that jumps
    /// on the spot passes closer to the flag than it can ever *stand* to be. The
    /// fixture finding 50 needs — every stand is on row 5, every jump apex beats one.
    fn overhang(name: &str) -> Level {
        crate::fixture::level(
            name,
            &[
                "......", "......", "......", "......", "......", "......", "######", "######",
            ],
            (1, 5),
            &[("goal", 2, 2)],
        )
    }

    /// The tile of the closest approach over **every** frame the episode passed
    /// through — apexes included. The reading `stuck_point` deliberately does not
    /// take, kept here so the difference between the two is a test and not a claim.
    ///
    /// `grounded_only` selects the reading: `true` tracks stands (the rule under
    /// test), `false` tracks every frame (the rule it must not regress to).
    fn reference_point(
        level: &Level,
        genome: &Genome,
        eval: &EvalConfig,
        seed: u64,
        grounded_only: bool,
    ) -> Option<(i32, i32)> {
        let mut runner = EpisodeRunner::new(level, genome.clone(), eval, seed);
        let start = runner.sim().goal_approach();
        let mut best: Option<(f32, (i32, i32))> = None;
        while !runner.is_finished() {
            runner.step();
            let sim = runner.sim();
            let p = sim.player();
            if grounded_only && !p.grounded {
                continue;
            }
            let approach = sim.goal_approach();
            if grounded_only && approach >= start {
                continue;
            }
            let col = Level::tile_x(p.x + p.w * 0.5);
            let surface = Level::tile_y(p.y + p.h + 0.05);
            if best.is_none_or(|(a, _)| approach < a) {
                best = Some((approach, (col, surface - 1)));
            }
        }
        best.map(|(_, tile)| tile)
    }

    #[test]
    fn the_stuck_point_is_where_the_body_stood_and_not_where_it_flew() {
        let level = overhang("overhang");
        let shape = crate::genome::default_shape();
        let eval = EvalConfig::default();
        let mut saw_the_gap = false;
        for seed in 1..=8u64 {
            let mut rng = sim::rng::rng_from_seed(seed * 7717);
            let genome = Genome::random(1, shape.clone(), &mut rng);
            let stood = stuck_point(&level, &genome, &eval, seed);
            assert_eq!(
                stood,
                reference_point(&level, &genome, &eval, seed, true),
                "stuck_point must read the body's stands, seed {seed}"
            );
            let flew = reference_point(&level, &genome, &eval, seed, false);
            // Random policies jump on this level; where one did, its flight apex
            // outranked every landing and the two readings disagree — which is the
            // whole difference finding 50 is about.
            if stood != flew {
                saw_the_gap = true;
                assert_ne!(
                    stood, flew,
                    "seed {seed}: the apex reading must not be what stuck_point returns"
                );
            }
        }
        assert!(
            saw_the_gap,
            "no random genome's apex beat its stands on the overhang fixture, so the \
             grounded reading is not being exercised"
        );
    }

    #[test]
    fn the_stuck_point_is_past_the_spawn_on_a_level_the_body_actually_crosses() {
        // The all-zeros genome is a constant `run` (finding 2): it walks right along
        // the runway, stands a little closer to the flag with every step, and dies in
        // the pit. The closest stand is therefore the pit's near edge — the furthest
        // the run has gotten — and not the apex of anything.
        let level = gap_runway("pitted", 24, 22, &[(8, 3)]);
        let genome = Genome::zeros(1, crate::genome::default_shape());
        let spot = stuck_point(&level, &genome, &EvalConfig::default(), 7)
            .expect("an always-run genome crosses ground toward the flag");
        assert_eq!(spot.1, 5, "the body only ever stands on the ground row");
        assert!(
            spot.0 > 5,
            "the stuck point should be near the pit at x=8, not near the spawn at x=1: {:?}",
            spot
        );
    }

    #[test]
    fn a_practice_level_moves_the_spawn_and_the_flag_and_touches_nothing_else() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        let practice = practice_level(&main, (5, 5)).expect("tile 5,5 stands on ground");

        assert_eq!(practice.spawn_tile(), (5, 5), "the spawn moved");
        assert!(practice.spawn_stands_on_ground(5, 5));
        // The flag moves too, to the next stand past the spawn — the far edge of the
        // pit, one crossing away — and not to the level's own end. That is finding
        // 50's mechanism: winning the practice level is worth the same 10 000 as
        // winning the level, so the two-level suite can hold the move and the climb
        // at once instead of ranking by the tail and breeding the climb away.
        assert_eq!(
            practice.goal_tiles(),
            vec![(11, 5)],
            "the flag is the next stand, at the pit's far edge"
        );
        assert_eq!(
            main.goal_tiles(),
            vec![(22, 5)],
            "and the level it came from keeps its own flag"
        );
        assert_eq!(practice.width, main.width);
        assert_eq!(practice.height, main.height);
        assert_eq!(practice.walkers, main.walkers, "same patrols");
        // Same grid tile for tile: the flag is an entity, not a tile, so moving it
        // leaves the landscape — including the pit — exactly as the run knows it.
        for ty in 0..main.height {
            for tx in 0..main.width {
                assert_eq!(
                    practice.get(tx, ty),
                    main.get(tx, ty),
                    "tile ({tx}, {ty}) moved"
                );
            }
        }
        assert!(
            practice
                .name
                .contains("practice at tile (5, 5) to tile (11, 5)"),
            "the name says where it starts and where the move ends: {:?}",
            practice.name
        );
        // The derived level is written beside the run's checkpoint, so it has to
        // survive the writer like any other document.
        let json = practice.to_json_pretty().expect("serializes");
        let back = Level::from_json_str(&json).expect("parses back");
        assert_eq!(
            back.content_hash_string(),
            practice.content_hash_string(),
            "the practice level round-trips through the writer unchanged"
        );
    }

    #[test]
    fn the_next_stand_is_the_nearest_stand_closer_to_the_flag_and_never_the_stuck_one() {
        // A pit with two stands past it: a floating ledge short of the far ground, and
        // the far ground itself under the flag. The next *move* is the nearer stand —
        // the ledge — even though the far ground is closer to the flag, and the zone
        // the body is standing on is never its own next move even when it stretches
        // toward the flag.
        let main = crate::fixture::level(
            "two-stands",
            &[
                "........", "........", "......#.", "........", "....#...", "........", "##....##",
                "##....##",
            ],
            (0, 5),
            &[("goal", 7, 5)],
        );
        assert_eq!(
            next_stand(&main, (0, 5)),
            Some((4, 3)),
            "the near ledge is the next move, not the flag's own ground"
        );

        // And a body already on the ledge looks past it: the far ground is next.
        assert_eq!(
            next_stand(&main, (4, 3)),
            Some((6, 5)),
            "from the ledge, the ground under the flag is the next stand"
        );

        // A stand from which nothing is closer to the flag has no next move — the
        // run is stuck at the end of its route, and there is nothing to practise.
        assert_eq!(
            next_stand(&main, (6, 5)),
            None,
            "the flag's own ground has no stand past it"
        );
        assert_eq!(
            next_stand(&gap_runway("pitted", 24, 22, &[(8, 3)]), (15, 5)),
            None,
            "ground the body can walk to the flag from is not a crossing to practise"
        );
    }

    #[test]
    fn the_next_stand_is_the_routes_next_crossing_even_when_the_straight_line_disagrees() {
        // A zig-zag climb — the shape finding 50's tower is built from. The route
        // doubles back away from the flag up the first leg, so the stand it visits next
        // reads *farther* from the flag than the stand behind it, while the stand two
        // crossings up the next leg reads closer than either. The straight-line rule
        // practised exactly that stand on the tower — or the ground the body had just
        // climbed from — and the route's own answer is the one short jump in front of
        // the body.
        let main = crate::fixture::level(
            "zigzag",
            &[
                "..........",
                "..........",
                ".......##.",
                "..........",
                ".....##...",
                "..........",
                ".##.......",
                "..........",
                "....##....",
                "..........",
            ],
            (5, 7),
            &[("goal", 8, 1)],
        );
        assert_eq!(
            next_stand(&main, (5, 7)),
            Some((2, 5)),
            "the crossing in front of the body is the next move up the first leg, \
             even though the flag's direction is the other way"
        );
        assert_eq!(
            next_stand(&main, (2, 5)),
            Some((5, 3)),
            "from there the route turns back toward the flag"
        );
        assert_eq!(
            next_stand(&main, (5, 3)),
            Some((7, 1)),
            "and the last crossing ends on the flag's own ground"
        );
        assert_eq!(
            next_stand(&main, (7, 1)),
            None,
            "the flag's own ground has no stand past it"
        );
    }

    #[test]
    fn a_practice_crossing_can_hop_a_hazard_the_body_cannot_walk_through() {
        // A floor spike at standing height cuts the ground in two. The stand across it
        // is not walking distance — it is a crossing like any other — so when it is the
        // route's next stand, the practice flag goes over the spike instead of being
        // refused as ground the body already owns.
        let main = crate::fixture::level(
            "spiked",
            &[
                "........", "........", "........", "........", "........", "...^....", "########",
                "########",
            ],
            (1, 5),
            &[("goal", 7, 5)],
        );
        assert_eq!(
            next_stand(&main, (2, 5)),
            Some((4, 5)),
            "the practice flag goes on the far side of the spike"
        );
    }

    /// A level whose only crossing is a **moving platform**, written the way the editor writes
    /// one: two ledges and a shuttle between them.
    fn shuttle_runway() -> Level {
        let json = r#########"{
          "format": "evo-platformer-level",
          "level_format_version": 3,
          "name": "shuttle",
          "width": 24,
          "height": 8,
          "spawn": { "x": 1, "y": 5 },
          "tiles": [
            "........................",
            "........................",
            "........................",
            "........................",
            "........................",
            "........................",
            "########........########",
            "########........########"
          ],
          "entities": [
            { "type": "platform", "x": 9, "y": 6, "tiles": 3, "span": 5, "axis": "x", "dir": 1 },
            { "type": "goal", "x": 22, "y": 5 }
          ]
        }"#########;
        Level::from_json_str(json).expect("the shuttle parses")
    }

    #[test]
    fn a_practice_crossing_off_a_deck_is_measured_from_the_deck() {
        // The stall chain on a level whose route crosses a shuttle. The frontier is a deck, and a
        // deck has no tile a spawn may be placed on, so the run is reported as stalled on the
        // ground behind it. Measuring the practice flag off *that ground* answers the crossing the
        // run already made — the flag lands on the deck it has just ridden, the excursion wins
        // trivially, and the crossing it actually died on is never practised. Measuring it off the
        // deck answers the move after the ride.
        let main = shuttle_runway();
        let routes = main.routes();
        let deck = (0..routes.len())
            .find(|&s| routes.ride_from(s).is_some())
            .expect("the shuttle's decks are stands");

        let behind = practice_level_from(&main, (1, 5), None).expect("ground behind the pit");
        assert_eq!(
            behind.goal_tiles(),
            vec![(9, 5)],
            "the spawn's own stand practises the jump onto the deck"
        );

        let off_the_deck =
            practice_level_from(&main, (1, 5), Some(deck)).expect("the deck's next crossing");
        assert_eq!(
            off_the_deck.goal_tiles(),
            vec![(16, 5)],
            "measured from the deck, the flag is past the ride, on the ground it has not reached"
        );
        assert_eq!(
            off_the_deck.spawn_tile(),
            (1, 5),
            "the spawn stays on the ground: a deck is not a tile a spawn may be placed on"
        );
        assert_eq!(
            next_stand_from(&main, deck),
            Some((16, 5)),
            "and the crossing is read off the deck's own route, not the spawn's"
        );
    }

    #[test]
    fn a_stand_with_no_route_to_the_flag_has_no_next_stand_at_all() {
        // Two islands and an unbridgeable chasm: the flag's ground reads closer in a
        // straight line, but no crossing the game has reaches it. Ordering a practice
        // flag there would order a jump that does not exist; the honest answer is that
        // there is no move to practise.
        let main = crate::fixture::level(
            "islands",
            &[
                "..............",
                "..............",
                "..............",
                "..............",
                "..............",
                "..............",
                "##..........##",
                "##..........##",
            ],
            (0, 5),
            &[("goal", 13, 5)],
        );
        assert_eq!(
            next_stand(&main, (0, 5)),
            None,
            "the flag's island is not a crossing away — or ten"
        );
        let err = practice_level(&main, (0, 5)).expect_err("there is nothing to practise toward");
        assert!(err.contains("no crossing toward the flag"), "{err:?}");
    }

    #[test]
    fn a_practice_level_is_written_beside_the_checkpoint_under_the_runs_own_stem() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        let practice = practice_level(&main, (5, 5)).expect("tile 5,5 stands on ground");
        let dir = temp_dir("practice-file");
        let checkpoint = dir.join("hard.population.json");
        let written = save_practice_level(&practice, &checkpoint).expect("writes");
        assert_eq!(
            written,
            dir.join("hard.practice.json"),
            "the practice level takes the run's stem, not the checkpoint's last extension"
        );
        let back = Level::from_json_file(&written).expect("the practice level loads back");
        assert_eq!(back.content_hash_string(), practice.content_hash_string());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_practice_level_refuses_a_spawn_that_would_not_stand() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        // Over the pit: the spot comes from a grounded stand, so a mid-air one means
        // the level moved underneath the run and is worth failing over.
        let err = practice_level(&main, (9, 5)).expect_err("no ground under a pit");
        assert!(err.contains("does not stand on ground"), "{err:?}");
        assert!(practice_level(&main, (2, 1)).is_err(), "mid-air too");
    }

    #[test]
    fn a_stalled_run_practises_where_its_best_genome_stood() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        // A one-genome run of the always-run genome (finding 2): it walks to the
        // pit's near edge and dies there, so where it stood furthest is not a
        // question of which way a random policy wandered.
        let genome = Genome::zeros(1, crate::genome::default_shape());
        let mut trainer = Trainer::resume(tiny(), vec![genome], 0).expect("a population resumes");
        trainer.step(&main);
        let practice = practice_level_for_stall(&trainer, &main)
            .expect("a population that crossed ground has somewhere to practise");
        assert_ne!(
            practice.spawn_tile(),
            main.spawn_tile(),
            "the practice level starts where the run got to, not where it started"
        );
        assert_ne!(
            practice.content_hash_string(),
            main.content_hash_string(),
            "a different spawn is a different level"
        );
    }

    #[test]
    fn a_planned_route_splits_into_ground_segments_that_are_all_playable_starts() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../levels/l2-first-gap.json");
        let level = Level::from_json_file(&path).expect("the shipped level loads");
        let route = sim::plan::solve(
            &level,
            1,
            sim::plan::PlanConfig {
                depth: 24,
                beam: 32,
                ..sim::plan::PlanConfig::default()
            },
        )
        .expect("l2 is planned");
        let segments = plan_segments(&level, &route).expect("the route has segments");
        assert!(!segments.is_empty());
        assert_eq!(segments[0].start_block, 0);
        assert_eq!(
            segments.last().expect("non-empty").end_block,
            route.actions.len(),
            "the legs cover the whole route and leave no tail"
        );
        for segment in &segments {
            assert!(
                segment
                    .level
                    .spawn_stands_on_ground(segment.entry.0, segment.entry.1),
                "segment {} starts on ground",
                segment.index
            );
            assert_eq!(
                segment.level.goal_tiles().first().copied(),
                Some(segment.exit)
            );
            assert!(segment.blocks() > 0);
        }
    }

    fn two_specialists(seed: u64) -> (Genome, Genome) {
        let mut rng = sim::rng::rng_from_seed(seed);
        (
            Genome::random(1, crate::genome::default_shape(), &mut rng),
            Genome::random(2, crate::genome::default_shape(), &mut rng),
        )
    }

    #[test]
    fn every_fused_gene_comes_from_one_of_two_distinct_specialists() {
        let (a, b) = two_specialists(41);
        let (a_before, b_before) = (a.clone(), b.clone());
        let mut rng = sim::rng::rng_from_seed(42);
        let children = fuse_specialists(
            &[a.clone(), b.clone()],
            &FuseConfig {
                shares: 8,
                cross_rate: 1.0,
            },
            &mut rng,
        );
        assert_eq!(children.len(), 8, "one child per requested share");
        for child in &children {
            assert_eq!(
                child.shape, a.shape,
                "fusion changes parents, never the shape"
            );
            assert_eq!(child.fitness, 0.0, "a child has not been judged");
            for (i, gene) in child.weights.iter().enumerate() {
                assert!(
                    gene.to_bits() == a.weights[i].to_bits()
                        || gene.to_bits() == b.weights[i].to_bits(),
                    "weight {i} came from neither parent"
                );
            }
            for (i, gene) in child.biases.iter().enumerate() {
                assert!(
                    gene.to_bits() == a.biases[i].to_bits()
                        || gene.to_bits() == b.biases[i].to_bits(),
                    "bias {i} came from neither parent"
                );
            }
        }
        // Two different parents crossed eight times: at least one child has genes from each,
        // which is what distinguishes a cross from copying the first parent eight times.
        assert!(children.iter().any(|c| c.weights != a.weights));
        assert!(children.iter().any(|c| c.weights != b.weights));
        // And the inputs are read only.
        assert_eq!(a.weights, a_before.weights);
        assert_eq!(a.biases, a_before.biases);
        assert_eq!(b.weights, b_before.weights);
        assert_eq!(b.biases, b_before.biases);
    }

    #[test]
    fn one_specialist_has_nothing_to_compose() {
        let (a, _) = two_specialists(43);
        let mut rng = sim::rng::rng_from_seed(44);
        assert!(
            fuse_specialists(std::slice::from_ref(&a), &FuseConfig::default(), &mut rng).is_empty()
        );
        assert!(fuse_specialists(&[], &FuseConfig::default(), &mut rng).is_empty());
        assert!(fuse_specialists(
            &[a.clone(), a],
            &FuseConfig {
                shares: 0,
                ..FuseConfig::default()
            },
            &mut rng
        )
        .is_empty());
    }

    #[test]
    fn the_fused_seeding_order_is_deterministic_and_deduplicated() {
        let (a, b) = two_specialists(45);
        let mut rng = sim::rng::rng_from_seed(46);
        let whole = Genome::random(9, crate::genome::default_shape(), &mut rng);
        let campaign = SegmentCampaign {
            segments: 2,
            steps: Vec::new(),
            solved: false,
            specialists: vec![a.clone(), b.clone()],
        };
        // `a` is also offered as a whole-level genome: it must spend one slot, not two.
        let best_whole = vec![a.clone(), whole.clone()];
        let mut r1 = sim::rng::rng_from_seed(47);
        let first = fused_seeding(&campaign, &best_whole, &FuseConfig::default(), &mut r1);
        let mut r2 = sim::rng::rng_from_seed(47);
        let second = fused_seeding(&campaign, &best_whole, &FuseConfig::default(), &mut r2);
        assert_eq!(first.len(), second.len());
        assert_eq!(
            first.len(),
            8 + 1,
            "eight crosses, then the one new whole genome"
        );
        for (x, y) in first.iter().zip(&second) {
            assert_eq!(
                x.weights, y.weights,
                "the same seed must give the same order"
            );
            assert_eq!(x.biases, y.biases);
        }
        for i in 0..first.len() {
            for j in 0..i {
                assert!(
                    first[i].weights != first[j].weights || first[i].biases != first[j].biases,
                    "candidate {i} duplicates candidate {j}"
                );
            }
        }
        assert_eq!(
            first.last().expect("non-empty").weights,
            whole.weights,
            "whole-level genomes follow the fused children, in the caller's order"
        );
    }
}
