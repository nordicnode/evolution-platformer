//! What a cloned route is worth as a warm start, measured rather than asserted.
//!
//! Two claims, and they are the two a reader of `--from-route` should be able to check:
//!
//! 1. **A clone is a better start than a random genome, and far better than a typical one.** The
//!    fitness it brings is at the top of a random population's range, against a random *median* of
//!    zero (a random genome makes no progress at all, and the score is progress). See
//!    [`a_clone_starts_near_the_top_of_a_random_population`].
//! 2. **A quarter of the population warmed makes the search faster, and the whole population warmed
//!    makes it slower.** [`a_warm_quarter_learns_sooner_than_random`] is the paired measurement:
//!    same master seed, same level, same population size, the same `resume` path for both arms, the
//!    same forty-eight random genomes under the same ids, and the only difference is the sixteen
//!    slots that came from the clone. `warm = 64` — which is what `Trainer::from_champion` does with
//!    a warm genome, and therefore what `--from-route` would do if it did not build its own
//!    population — is measured in `crates/train-cli`'s docs and is the **worst** arm on three of the
//!    four levels tried: the clone's basin is a high-fitness plateau with no win on it.
//!
//! 3. **A clone plays the level only at the demonstration's own cadence.**
//!    [`a_clone_plays_the_level_only_at_the_demonstrations_own_cadence`] is the ceiling of the whole
//!    idea: the oracle presses on a tick, a policy decides once every four, and a clone that
//!    reproduces *every* decision of the walk still steps into the pit at the shipped cadence while
//!    the same fit is a solver on all eight seeds at `action_repeat = 1`. See `evo::imitate`'s docs.
//!
//! The noise floor matters here and is why both checks are *paired*: a first-win generation is a
//! draw from 64 genomes' worth of luck, so "the warm arm is faster" is only a claim if the same
//! master seed ran both arms.
//!
//! The full survey (six seeds, five warm sizes, four levels) is in the docs of
//! `route_seeded_trainer` in `crates/train-cli/src/commands.rs`; what is here is the part that has
//! to fail if any of it stops being true.

use evo::fitness::{run_episode, EvalConfig};
use evo::genome::{default_shape, Genome};
use evo::{clone_from_play, from_plan, replay, GaConfig, ImitateConfig, Trainer};
use sim::rng::rng_from_seed;
use sim::Level;

/// Population used by both arms. Small on purpose — the claim is about a *rate*, and 64 is the
/// population the survey used, so the numbers here and the table in `train-cli`'s docs are the same
/// measurement at different widths.
const POP: usize = 64;

/// The warm share the CLI defaults to: a quarter of the population.
const WARM: usize = POP / 4;

/// Generations a first win has to arrive within.
///
/// **Re-measured under the paired default** (`GaConfig::common_episodes`, on with racing) and the
/// current version trio (sensors v13, fitness v9), on the five seeds this test runs. The spread is
/// one to several hundred generations rather than the one-to-forty of the per-genome measurement
/// this cap was sized for: pairing costs the hard level the seeds it used to save (the review's
/// chain table says the same thing about `l9`), and a cap of sixty censored the l5 seed that needs
/// the long run, which fails the claim about a *rate* for a reason that has nothing to do with
/// warming. Four hundred is above the hardest measured solve here and still bounded, so a null
/// result is a censored sum, not a hang. The assertion stays on the total of the seeds rather than
/// on any one of them, for the same reason it always was: a first win is a draw, and the claim is
/// about the arm.
const CAP: u32 = 400;

fn level(slug: &str) -> Level {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../levels")
        .join(format!("{slug}.json"));
    Level::from_json_file(&path).unwrap_or_else(|e| panic!("shipped level {slug}: {e}"))
}

fn config(seed: u64) -> GaConfig {
    GaConfig {
        master_seed: seed,
        pop_size: POP,
        ..Default::default()
    }
}

/// The clone of `slug`'s route at the default seed, at the fit the CLI runs.
fn clone_of(slug: &str) -> (Level, Genome) {
    let level = level(slug);
    let play = replay(&level, 1, &EvalConfig::default());
    assert!(
        play.won(),
        "the replay has to walk {slug} for this measurement to mean anything: {}",
        play.note
    );
    let report = clone_from_play(&play, default_shape(), &ImitateConfig::default())
        .expect("the demonstration fits");
    (level, report.genome)
}

fn mean_fitness(level: &Level, g: &Genome, seeds: &[u64], cfg: &EvalConfig) -> f32 {
    seeds
        .iter()
        .map(|s| run_episode(level, g, cfg, *s).fitness)
        .sum::<f32>()
        / seeds.len() as f32
}

/// First-win generation of an arm, or `None` if it never got there inside [`CAP`].
fn first_win(level: &Level, mut trainer: Trainer) -> Option<u32> {
    (0..=CAP).find(|_| trainer.step(level).wins > 0)
}

/// The clone lands at the top of a random population's range, while a random genome's *median* is
/// zero.
///
/// `l2-first-gap` because it is the pack's first earned level: the one where walking is not the
/// solution, so "the clone gets further than random" is not a statement about a flat level. The
/// assertion is deliberately loose — above the 75th percentile of 32 genomes — because the exact
/// rank depends on the trial seeds, and the *shape* of the claim (top of the range, against a median
/// of zero) survives any draw of them.
#[test]
fn a_clone_starts_near_the_top_of_a_random_population() {
    let cfg = EvalConfig::default();
    let seeds: Vec<u64> = (0..8).map(|i| 1000 + i).collect();
    let (level, clone) = clone_of("l2-first-gap");
    let clone_f = mean_fitness(&level, &clone, &seeds, &cfg);
    let mut rng = rng_from_seed(0xC0FFEE);
    let mut random: Vec<f32> = (0..32)
        .map(|_| {
            let g = Genome::random(0, default_shape(), &mut rng);
            mean_fitness(&level, &g, &seeds, &cfg)
        })
        .collect();
    random.sort_by(f32::total_cmp);
    let median = random[random.len() / 2];
    let rank = random.iter().filter(|f| **f < clone_f).count() as f32 / random.len() as f32;
    assert!(
        rank >= 0.75,
        "the clone's mean fitness {clone_f:.2} sits at the {:.0}th percentile of 32 random \
         genomes (median {median:.2}), so it is not the warm start it is advertised as",
        rank * 100.0
    );
    assert!(
        clone_f > 0.0,
        "the clone made no progress at all over eight episodes"
    );
}

/// Five paired seeds per level: the warm arm must solve what the random arm solves and reach it in
/// fewer generations in total.
///
/// Two levels rather than the survey's four: `l2-first-gap` is where a *random* population is
/// already fast (a handful of generations, so the warm arm has to beat a good number) and
/// `l5-tightrope` is where random is slow (tens of generations), which is the shape of level a
/// warm start exists for.
#[test]
fn a_warm_quarter_learns_sooner_than_random() {
    for slug in ["l2-first-gap", "l5-tightrope"] {
        let (level, clone) = clone_of(slug);
        let (mut cold_total, mut warm_total) = (0u32, 0u32);
        let mut cold_solved = 0usize;
        let mut warm_solved = 0usize;
        for seed in [1u64, 2, 3, 4, 5] {
            // **Paired on the random part, which is what the arms are supposed to differ by.** Both
            // arms hold the *same* 48 random genomes, under the same ids, drawn once; the warm arm
            // replaces the first 16 slots with the clone and its mutants. Drawing a fresh random
            // population for the cold arm instead — which is what this did until it was caught
            // reading a wash on `l2-first-gap` — makes every arm a different draw of 64 nets, and a
            // claim about warming a quarter of the population is then a claim about two populations.
            let mut rng = rng_from_seed(seed * 7919);
            let random: Vec<Genome> = (0..POP)
                .map(|id| Genome::random(id as u32, default_shape(), &mut rng))
                .collect();
            let mut warm_pop = evo::warm_set(&clone, WARM - 1, 0.1, 0.05, &mut rng);
            warm_pop.extend_from_slice(&random[WARM..]);
            let cold = Trainer::resume(config(seed), random, 0).expect("a random population seeds");
            let warm = Trainer::resume(config(seed), warm_pop, 0).expect("a warm population seeds");
            let cold_gen = first_win(&level, cold);
            let warm_gen = first_win(&level, warm);
            if let Some(g) = cold_gen {
                cold_total += g;
                cold_solved += 1;
            } else {
                cold_total += CAP;
            }
            if let Some(g) = warm_gen {
                warm_total += g;
                warm_solved += 1;
            } else {
                warm_total += CAP;
            }
        }
        assert_eq!(
            warm_solved, 5,
            "{slug}: the warm arm failed to solve a level the random arm's seeds are chosen from \
             ({warm_solved} of 5 seeds, {warm_total} generations against random's {cold_total} \
             over {cold_solved} solves)"
        );
        assert!(
            warm_total < cold_total,
            "{slug}: the warm arm solved in {warm_total} generations against random's {cold_total} \
             ({cold_solved} of 5 seeds), so warming a quarter of the population bought nothing"
        );
    }
}

/// **A clone plays the level exactly when it can decide on the demonstration's own grid.**
///
/// This is the finding the warm start's ceiling is made of, and it is a fact about *time* rather than
/// about the observation or the fit. `sim::reach` sweeps its canonical moves with the sim's own
/// stepper and presses a take-off on the tick the body reaches it; a policy decides once per
/// [`EvalConfig::action_repeat`] ticks and holds its arm in between. At the shipped cadence of four,
/// the oracle's press falls *between* a policy's decisions — `l2-first-gap`'s pit is cleared by the
/// press made on the tick the body arrives, and a policy that may only act on every fourth tick
/// cannot place it. So the clone walks the route, reproduces almost all of the demonstration, and
/// steps into the pit — at a 300-epoch fit, at a 4000-epoch fit, and at a fit that misses *none* of
/// the demonstrated decisions.
///
/// At `action_repeat = 1` the two grids are the same grid, and the claim flips: the same fit, on the
/// same demonstration, of the same level, **solves it on all eight seeds** — with identical frame
/// counts, because a clone that reproduces every decision is walking the route the oracle walked.
///
/// Both arms are pinned here, and the second is the one that matters: a warm start is worth what the
/// cadence lets it be, which is why the module docs read "a seed and not a solution" rather than
/// "a solver with a fitting problem".
#[test]
fn a_clone_plays_the_level_only_at_the_demonstrations_own_cadence() {
    let level = level("l2-first-gap");
    // The fit budget moved with the observation: at the 108-wide v13 vector, 1 200 epochs leave
    // five to eight decisions wrong and the clone loses the pit at the one-tick cadence, where the
    // same budget on the 100-wide vector reproduced the walk. 2 500 is the smallest budget measured
    // here that recovers every decision the walk depends on; the claim is about the cadence, not
    // about the fit, so the fit is given what it needs to make the claim testable.
    let cfg = ImitateConfig {
        epochs: 2500,
        lr: 0.2,
        ..ImitateConfig::default()
    };

    // The oracle's own grid: one decision per tick, which is the grid its scripts were swept on.
    let tick = EvalConfig {
        action_repeat: 1,
        cut_stuck_frames: 0,
    };
    let play = replay(&level, 1, &tick);
    assert!(play.won(), "the route of l2-first-gap: {}", play.note);
    let fitted = clone_from_play(&play, default_shape(), &cfg).expect("the demonstration fits");
    let missed = ((1.0 - fitted.agreement) * fitted.exemplars as f32).round() as usize;
    assert!(
        missed <= 8,
        "the fit reproduces {:.1}% of the demonstration ({} of {} decisions missed), so it is not \
         close enough to the walk for the arm below to mean anything",
        fitted.agreement * 100.0,
        missed,
        fitted.exemplars
    );
    let won = (1..=8u64)
        .filter(|seed| run_episode(&level, &fitted.genome, &tick, *seed).won())
        .count();
    assert_eq!(
        won, 8,
        "a clone that decides on the oracle's own grid has to walk the route it was shown: {won} \
         of 8 seeds won, {} decisions fitted with {missed} missed",
        fitted.exemplars
    );

    // The shipped grid: the same level, the same fit budget, the same evaluation seeds, and the
    // demonstration recorded at the cadence a real run decides on. Every seed gets to the pit.
    let shipped = EvalConfig::default();
    assert!(shipped.action_repeat > 1, "the shipped cadence is the point");
    let play = replay(&level, 1, &shipped);
    assert!(play.won(), "the route of l2-first-gap: {}", play.note);
    let fitted = clone_from_play(&play, default_shape(), &cfg).expect("the demonstration fits");
    let won = (1..=8u64)
        .filter(|seed| run_episode(&level, &fitted.genome, &shipped, *seed).won())
        .count();
    assert_eq!(
        won, 0,
        "a clone of the walk at the shipped {} -tick cadence is not expected to play the level (the \
         documented ceiling of `--from-route`); {won} of 8 seeds won, so either the cadence changed \
         or the press became expressible and this module's docs need rewriting",
        shipped.action_repeat
    );
}

/// **A plan is the one demonstration written on the policy's own grid.**
///
/// This is the property `--from-plan` exists for: every decision in the plan is a whole block
/// action, so the fit is not chasing a press that happened between decisions. The plan is replayed
/// on the real stepper before it is cloned, and the test pins both halves — the route wins, and the
/// clone agrees with it at a budget a route clone would need thousands of epochs for.
#[test]
fn a_timed_plan_is_demonstrated_on_the_policys_own_block_grid() {
    // A small authored level rather than a shipped one, so the planner's search is a second of
    // debug build rather than a minute: the claim is about the shape of the demonstration, not
    // about the pack's hardest crossing (that one is the ignored `elements` benchmark).
    let src = r########"{"format":"evo-platformer-level","level_format_version":3,
        "name":"plan-gap","width":20,"height":7,"spawn":{"x":1,"y":4},
        "tiles":["....................","....................","....................",
                  "....................","....................","....................",
                  "######...###########"],
        "entities":[{"type":"goal","x":18,"y":4}]}"########;
    let level = sim::Level::from_json_str(src).expect("the fixture parses");
    let eval = EvalConfig::default();
    let icfg = ImitateConfig {
        epochs: 300,
        ..ImitateConfig::default()
    };

    let (planned, report, route) = from_plan(
        &level,
        1,
        &eval,
        default_shape(),
        &icfg,
        sim::plan::PlanConfig::default(),
    )
    .expect("l2 has a timed plan");
    assert!(
        planned.won(),
        "the plan has to replay to the flag before it is a demonstration: {}",
        planned.note
    );
    assert_eq!(
        planned.decisions.len(),
        route.actions.len(),
        "one decision per planned block: the demonstration is the plan, not a re-timing of it"
    );
    assert!(
        report.agreement >= 0.8,
        "the block-aligned plan should fit the shipped net closely at 300 epochs, got {:.1}%",
        report.agreement * 100.0
    );
}

/// The clone is a function of `(level, seed, config)`: two runs of the same invocation agree.
///
/// A warm start that is not reproducible is not a warm start — it is a free roll of the dice that a
/// benchmark cannot pair against anything.
#[test]
fn a_warm_start_is_reproducible() {
    let level = level("l2-first-gap");
    let cfg = EvalConfig::default();
    let a = replay(&level, 3, &cfg);
    let b = replay(&level, 3, &cfg);
    assert_eq!(a.decisions, b.decisions);
    let icfg = ImitateConfig {
        epochs: 40,
        ..ImitateConfig::default()
    };
    let ca = clone_from_play(&a, default_shape(), &icfg).expect("fits");
    let cb = clone_from_play(&b, default_shape(), &icfg).expect("fits");
    assert_eq!(ca.genome.weights, cb.genome.weights);
    assert_eq!(ca.genome.biases, cb.genome.biases);
    assert_eq!(ca.agreement, cb.agreement);
}

