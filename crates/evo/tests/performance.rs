//! Performance probes for the smarter-AI plan (SMARTER_AI_PLAN.md, Item 2).
//!
//! These are `#[ignore]`d because they measure a machine rather than assert a result: the
//! numbers they print are the before/after readings a change has to move, and a timing
//! assertion in the ordinary suite would fail on a loaded laptop for reasons that have
//! nothing to do with the code. Run them in release mode, on an idle machine:
//!
//! ```text
//! cargo test --release -p evo --test performance -- --ignored --nocapture
//! ```
//!
//! **What "forward share" means here.** One generation is 200 episodes on the reference
//! level, evaluated in parallel. The forward pass is measured single-threaded per decision,
//! then multiplied by the decisions those episodes actually made; the share is that CPU
//! time against the generation's wall time times the core count. It is an estimate of the
//! CPU budget the forward pass owns, not a profile, and it is the number Item 2's 25 %
//! gate is about.
//!
//! The probe prints every reading it uses — episodes, decisions, microseconds per
//! decision, generation seconds — so a later run can be compared without trusting this
//! file's arithmetic.
//!
//! The batched-vs-scalar comparison that used to live here went with `Genome::forward_block`
//! itself: Item 2 was killed by its own gate (0.96x on the feedback shape that matters), and
//! the killed implementation was removed rather than kept as a measured artifact with no
//! callers. The readings that condemned it are recorded in FINDINGS.md.

use std::time::Instant;

use evo::genome::{default_shape, ActScratch, Genome, NetShape};
use evo::{evaluate_population, GaConfig, Trainer};
use sim::rng::rng_from_seed;
use sim::sensors::N_INPUTS;
use sim::Level;

/// The reference level the plan quotes: the pack's first chain, and the slowest row.
const REFERENCE: &str = "l9-wait-and-climb";

fn level(slug: &str) -> Level {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../levels")
        .join(format!("{slug}.json"));
    Level::from_json_file(&path).unwrap_or_else(|e| panic!("shipped level {slug} failed: {e}"))
}

/// A deterministic observation vector, so the timed multiplies are not all by zero.
fn observation(seed: u64) -> [f32; N_INPUTS] {
    let mut rng = rng_from_seed(seed);
    let mut obs = [0.0f32; N_INPUTS];
    for v in &mut obs {
        *v = sim::rng::next_signed_f32(&mut rng);
    }
    obs
}

/// Time `calls` forward passes of the genome set, in genome order, reusing one scratch per
/// genome as an episode does. Returns nanoseconds per call.
fn forward_ns_per_call(genomes: &[Genome], calls: usize) -> f64 {
    let obs = observation(7);
    let mut scratches: Vec<ActScratch> = genomes.iter().map(|_| ActScratch::new()).collect();
    // Warm the caches and the branch predictors before the timed loop.
    for (g, s) in genomes.iter().zip(scratches.iter_mut()) {
        g.forward(&obs, s);
    }
    let start = Instant::now();
    for i in 0..calls {
        let at = i % genomes.len();
        genomes[at].forward(&obs, &mut scratches[at]);
    }
    start.elapsed().as_nanos() as f64 / calls as f64
}

/// **Item 5's baseline.** "Stop paying for episodes that have already failed" assumes failed
/// episodes run to the frame cap. They do not: `sim::NO_PROGRESS_FRAMES` already ends a body
/// that has stopped getting closer to the flag, and the sim charges it the same one-shot score a
/// full episode would earn. This probe counts what actually ends an episode and how many frames
/// each ending costs, so the item's remaining headroom is a number rather than an assumption.
#[test]
#[ignore = "a machine measurement, not an assertion; run with --release --ignored"]
fn what_ends_an_episode_and_what_it_costs() {
    for slug in [REFERENCE, "l11-the-elements"] {
        let level = level(slug);
        let cfg = GaConfig {
            master_seed: 1,
            pop_size: 200,
            ..Default::default()
        };
        let trainer = Trainer::new(cfg.clone());
        let mut ends: std::collections::BTreeMap<&'static str, (usize, u64)> =
            std::collections::BTreeMap::new();
        let mut total_frames = 0u64;
        for generation in 0..3u32 {
            for r in evaluate_population(&level, &trainer.genomes, &cfg, generation) {
                let key = match r.outcome {
                    sim::Outcome::Won => "won",
                    sim::Outcome::Died => "died",
                    sim::Outcome::Stuck => "stuck",
                    sim::Outcome::TimedOut => "timed out",
                    sim::Outcome::Running => "running",
                };
                let e = ends.entry(key).or_default();
                e.0 += 1;
                e.1 += r.frames as u64;
                total_frames += r.frames as u64;
            }
        }
        println!(
            "{slug}: {total_frames} frames total, cap {} per episode",
            level.width.max(level.height) as u32 * sim::FRAMES_PER_TILE
        );
        for (k, (n, frames)) in ends {
            println!(
                "  {k:9}: {n:4} episodes, {frames:8} frames, {:.1} % of all frames",
                100.0 * frames as f64 / total_frames as f64
            );
        }
    }
}

#[test]
#[ignore = "a machine measurement, not an assertion; run with --release --ignored"]
fn the_forward_pass_share_of_a_generation() {
    let level = level(REFERENCE);
    let cfg = GaConfig {
        master_seed: 1,
        pop_size: 200,
        ..Default::default()
    };
    let trainer = Trainer::new(cfg.clone());
    let repeat = cfg.eval_config().action_repeat.max(1);

    // One generation as training pays for it: parallel across genomes.
    let start = Instant::now();
    let results = evaluate_population(&level, &trainer.genomes, &cfg, 0);
    let generation = start.elapsed().as_secs_f64();
    let decisions: usize = results
        .iter()
        .map(|r| (r.frames as usize).div_ceil(repeat as usize))
        .sum();
    let frames: usize = results.iter().map(|r| r.frames as usize).sum();

    let scalar = forward_ns_per_call(&trainer.genomes, decisions);
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());

    for (name, shape) in [
        ("shipped 108-16-11, no memory", default_shape()),
        (
            "feedback 16 (a timed level)",
            NetShape {
                feedback: 16,
                ..default_shape()
            },
        ),
    ] {
        let mut rng = rng_from_seed(3);
        let genomes: Vec<Genome> = (0..200)
            .map(|i| Genome::random(i as u32, shape.clone(), &mut rng))
            .collect();
        let ns = forward_ns_per_call(&genomes, decisions);
        let cpu = ns * decisions as f64 / 1e9;
        println!(
            "{name}: {ns:.1} ns/decision, {decisions} decisions, {cpu:.3} s of forward CPU, \
             {:.1} % of a {generation:.3} s generation on {cores} cores",
            100.0 * cpu / (generation * cores as f64)
        );
    }

    println!(
        "reference: {} genomes, {frames} frames, {generation:.3} s/generation, \
         {scalar:.1} ns/decision headless",
        trainer.genomes.len()
    );
}
