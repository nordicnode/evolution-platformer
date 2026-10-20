//! Regenerate the migration tests' fixture champion.
//!
//! The migration tests (`crates/evo/tests/migration.rs`) prove sensor growth and cadence-head
//! adoption on a real recorded episode. They used to read the author's own saved l11 champion
//! out of `champions/`, which is gitignored, so the suite failed on every clean checkout
//! (CI included). This example builds the committed replacement: a feedback-16 genome whose
//! episode on `l11-the-elements` is run through the ordinary episode runner and wrapped with
//! the ordinary `ChampionFile::new`, so the fixture is a champion file in every respect except
//! that its genome was drawn from a fixed seed instead of selected by a run.
//!
//! Run from the workspace root:
//! `cargo run -p evo --example make_migration_fixture`

use evo::champion::ChampionFile;
use evo::ga::GaConfig;
use evo::genome::NetShape;
use evo::{run_episode, EvalConfig, Genome};
use sim::rng::rng_from_seed;
use sim::Level;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let level = Level::from_json_file(root.join("levels/l11-the-elements.json"))
        .expect("the l11 level loads");
    let shape = NetShape {
        inputs: sim::sensors::N_INPUTS,
        hidden: vec![16],
        outputs: evo::genome::N_OUTPUTS,
        feedback: 16,
        commitment: false,
    };
    let eval = EvalConfig::default();
    let seed = 0x5EED_5EED_5EED_5EEDu64;
    // Scan a fixed range of genome seeds and keep the longest episode: the proof is about a
    // policy with memory surviving growth, and a genome that dies in the first second exercises
    // almost none of it. The scan is deterministic, so the fixture is reproducible byte for
    // byte from this example.
    let mut best: Option<(Genome, evo::EpisodeResult)> = None;
    for genome_seed in 0..512u64 {
        let mut rng = rng_from_seed(genome_seed);
        let candidate = Genome::random(1, shape.clone(), &mut rng);
        let result = run_episode(&level, &candidate, &eval, seed);
        let better = match &best {
            None => true,
            Some((_, r)) => result.frames > r.frames,
        };
        if better {
            best = Some((candidate, result));
        }
    }
    let (genome, result) = best.expect("the scan ran");
    let config = GaConfig {
        shape,
        action_repeat: eval.action_repeat,
        ..GaConfig::default()
    };
    let champ = ChampionFile::new(&genome, &result, &level, 0, &config);
    let out = root.join("crates/evo/tests/fixtures/l11-the-elements.champion.json");
    champ.save(&out).expect("the fixture writes");
    println!(
        "wrote {} (outcome {}, {} frames, fitness {})",
        out.display(),
        result.outcome.as_str(),
        result.frames,
        result.fitness
    );
}
