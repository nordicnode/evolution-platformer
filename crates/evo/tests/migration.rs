//! Item 3's proof: an append-only sensor bump cannot change a saved champion's policy.
//!
//! The arithmetic (`Genome::grown_to_inputs`) is pinned in `genome.rs`. This test is the whole
//! artifact chain: read the real saved l11 champion, grow it by the eight inputs a next bump
//! would append, and replay it against a driver that feeds the *old* vector to the *old* genome.
//! If the growth is what it claims, the two runs agree on every frame, the outcome and the
//! frontier — not approximately, exactly.
//!
//! **Why the growth is hypothetical here.** Sensors v14 was built and measured, and it does not
//! land in this pass: the plan's rule 1 puts Item 3 behind Item 2, whose batched forward pass came
//! in below its gate, and the battery levels it exists for have not moved. The machinery is the
//! part that survives, so the proof grows the current layout by eight columns instead of reading a
//! v13 file into a v14 build. When a real bump lands, `ChampionFile::migrate_sensors` is the same
//! call this test exercises by hand.
//!
//! The l11 champion is the right fixture because it already carries feedback-16 memory, so the
//! growth has to preserve the recurrent block as well as the feedforward rows.

use evo::champion::ChampionFile;
use evo::genome::ActScratch;
use evo::{run_episode, EvalConfig};

use sim::physics::AgentAction;
use sim::{Level, Sim};

/// The eight columns a next append-only bump would add, and the width the arithmetic is tested at.
const GROWTH: usize = 8;

fn path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn saved_l11() -> ChampionFile {
    let src = std::fs::read_to_string(path("champions/l11-the-elements.json"))
        .expect("the saved l11 champion is in the tree");
    ChampionFile::from_json_str(&src).expect("it parses")
}

/// The saved policy, driven by hand against its own observation width.
///
/// This is deliberately *not* `EpisodeRunner`: the runner would ask the genome for the current
/// vector, which is exactly the question the grown genome answers. The control has to be the old
/// policy reading old senses, or the comparison proves nothing.
///
/// A genome wider than the sim's vector is fed the real cells followed by zeros, which is what a
/// landed bump would feed it with (the new columns are read from the sensors); the point of the
/// comparison is that a **zero weight** on those columns leaves every decision alone.
fn replay_old(level: &Level, genome: &evo::Genome, seed: u64) -> (u32, String, f32, f32) {
    let repeat = EvalConfig::default().action_repeat.max(1);
    let mut sim = Sim::new(level, seed);
    let mut scratch = ActScratch::new();
    let mut action = AgentAction::Walk;
    let width = genome.shape.inputs;
    let mut fed = vec![0.0f32; width];
    while !sim.outcome().is_finished() {
        if sim.stats().frames % repeat == 0 {
            let obs = sim.observation();
            let real = obs.len().min(width);
            fed[..real].copy_from_slice(&obs[..real]);
            action = genome.act(&fed, &mut scratch);
        }
        sim.step(action);
    }
    let stats = sim.stats();
    (
        stats.frames,
        sim.outcome().as_str().to_string(),
        stats.best_x,
        stats.progress_pixels,
    )
}

#[test]
fn the_saved_l11_champion_survives_growth_and_replays_bit_identically() {
    let level = Level::from_json_file(path("levels/l11-the-elements.json")).expect("level loads");
    let mut champ = saved_l11();
    assert_eq!(
        champ.metadata.sensor_version,
        sim::sensors::VERSION,
        "the fixture is this build's layout"
    );
    assert_eq!(champ.network.n_inputs, sim::sensors::N_INPUTS);
    assert_eq!(
        champ.network.feedback, 16,
        "the fixture has memory to preserve"
    );

    let old = champ.to_genome().expect("the genome reads back");
    let seed = champ.metadata.episode_seed;
    let (frames, outcome, best_x, progress) = replay_old(&level, &old, seed);
    assert_eq!(
        frames, champ.metadata.frames,
        "the hand driver must reproduce the recorded episode before it can be a control"
    );
    assert_eq!(outcome, champ.metadata.outcome);

    // A file already at this build's layout has nothing to migrate.
    assert!(
        !champ.migrate_sensors().expect("a current file is a no-op"),
        "the fixture is current, so migration must report that it changed nothing"
    );

    let grown = old
        .grown_to_inputs(sim::sensors::N_INPUTS + GROWTH)
        .expect("growth to a wider observation");
    assert_eq!(grown.shape.inputs, sim::sensors::N_INPUTS + GROWTH);
    assert_eq!(grown.shape.feedback, 16, "memory width is untouched");

    let (grown_frames, grown_outcome, grown_best_x, grown_progress) =
        replay_old(&level, &grown, seed);
    assert_eq!(
        grown_frames, frames,
        "the grown policy took a different clock"
    );
    assert_eq!(grown_outcome, outcome);
    assert_eq!(grown_best_x.to_bits(), best_x.to_bits());
    assert_eq!(grown_progress.to_bits(), progress.to_bits());
}

/// Item 12's proof on the same artifact: the cadence head is one zero output row, so the
/// champion's recorded episode replays to the same clock and frontier with the head present.
#[test]
fn the_saved_champion_can_take_the_cadence_head_without_changing_its_episode() {
    let level = Level::from_json_file(path("levels/l11-the-elements.json")).expect("level loads");
    let mut champ = saved_l11();
    let plain = champ.to_genome().expect("genome");
    let seed = champ.metadata.episode_seed;
    let before = run_episode(&level, &plain, &EvalConfig::default(), seed);

    assert!(champ
        .adopt_commitment()
        .expect("the head is an append-only row"));
    champ
        .check_trainable()
        .expect("the adopted file is this build's");
    let headed = champ.to_genome().expect("genome with the head");
    assert!(headed.shape.commitment);
    assert_eq!(headed.shape.outputs, plain.shape.outputs);
    let after = run_episode(&level, &headed, &EvalConfig::default(), seed);

    assert_eq!(after.frames, before.frames);
    assert_eq!(after.outcome, before.outcome);
    assert_eq!(after.best_x.to_bits(), before.best_x.to_bits());
    assert_eq!(
        after.progress_pixels.to_bits(),
        before.progress_pixels.to_bits()
    );
    assert_eq!(after.trajectory_hash, before.trajectory_hash);
}

#[test]
fn only_the_previous_sensor_layout_is_migrated() {
    let mut champ = saved_l11();
    champ.metadata.sensor_version = sim::sensors::VERSION - 2;
    let err = champ
        .migrate_sensors()
        .expect_err("two layouts back is not a growth this build can promise");
    assert!(err.contains("only the previous layout"), "{err}");
    // And the file is left exactly as it was: a refused migration must not half-apply.
    assert_eq!(champ.metadata.sensor_version, sim::sensors::VERSION - 2);
    assert_eq!(champ.network.n_inputs, sim::sensors::N_INPUTS);
}
