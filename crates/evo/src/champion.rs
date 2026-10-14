//! Champion file format (§7.6) — the save/load/share loop.
//!
//! Every field the file carries is required for someone else to re-simulate the
//! episode and get the same trajectory, which is why the version trio
//! (`sim_version`, `sensor_version`, `fitness_version`) and the action-repeat
//! cadence are all recorded. Loading refuses a file whose versions do not match
//! the running build (§8), so an old champion can never silently misbehave.
//!
//! One deliberate deviation from the plan's sketch: `level_hash` is
//! `"fnv1a64:<hex>"` rather than `"sha256:<hex>"`, keeping the workspace free of
//! a hashing dependency for a value whose only job is integrity checking.

use serde::{Deserialize, Serialize};
use sim::level::LEVEL_FORMAT_VERSION;
use sim::sensors;
use sim::{Level, SIM_VERSION};

use crate::fitness::{EpisodeResult, FITNESS_VERSION};
use crate::ga::GaConfig;
use crate::genome::{ActScratch, Genome, NetShape};

pub const CHAMPION_FORMAT: &str = "evo-platformer-champion";
pub const CHAMPION_VERSION: u32 = 1;

/// The network, flattened for serialization.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetworkSpec {
    pub n_inputs: usize,
    pub n_hidden: Vec<usize>,
    pub n_outputs: usize,
    pub activation_hidden: String,
    pub activation_output: String,
    pub weights: Vec<f32>,
    pub biases: Vec<f32>,
}

impl NetworkSpec {
    pub fn from_genome(g: &Genome) -> NetworkSpec {
        NetworkSpec {
            n_inputs: g.shape.inputs,
            n_hidden: g.shape.hidden.clone(),
            n_outputs: g.shape.outputs,
            activation_hidden: "tanh".into(),
            activation_output: "tanh".into(),
            weights: g.weights.clone(),
            biases: g.biases.clone(),
        }
    }

    pub fn shape(&self) -> NetShape {
        NetShape {
            inputs: self.n_inputs,
            hidden: self.n_hidden.clone(),
            outputs: self.n_outputs,
        }
    }

    /// The dimension and activation rules a loadable network has to satisfy.
    ///
    /// `kind` names the artifact the caller is loading, so the message says what to
    /// look at; the rules themselves are shared with the population checkpoint, so a
    /// resumed population cannot be validated more loosely than a champion.
    pub fn check_compat(&self, kind: &str) -> Result<(), String> {
        if self.n_inputs != sensors::N_INPUTS {
            return Err(format!(
                "{kind} network takes {} inputs but the sensor version expects {}",
                self.n_inputs,
                sensors::N_INPUTS
            ));
        }
        if self.n_outputs != sim::physics::AgentAction::COUNT {
            return Err(format!(
                "{kind} network has {} outputs but the action set has {}",
                self.n_outputs,
                sim::physics::AgentAction::COUNT
            ));
        }
        if self.activation_hidden != "tanh" || self.activation_output != "tanh" {
            return Err(format!(
                "unsupported activations ({:?} / {:?}); this build only implements tanh",
                self.activation_hidden, self.activation_output
            ));
        }
        Ok(())
    }
}

/// The guards that make a saved artifact reproducible (§8).
///
/// Extracted so the champion file and the population checkpoint enforce *the same*
/// rules rather than two similar sets: a version bump that is refused for a champion
/// must not slip through as a resumable population.
pub fn check_reproducibility(
    kind: &str,
    sim_version: &str,
    sensor_version: u32,
    fitness_version: u32,
    level_format_version: u32,
    action_repeat: u32,
) -> Result<(), String> {
    if sim_version != SIM_VERSION {
        return Err(format!(
            "sim_version mismatch: {kind} {sim_version} vs build {SIM_VERSION}"
        ));
    }
    if sensor_version != sensors::VERSION {
        return Err(format!(
            "sensor_version mismatch: {kind} {sensor_version} vs build {}",
            sensors::VERSION
        ));
    }
    if fitness_version != FITNESS_VERSION {
        return Err(format!(
            "fitness_version mismatch: {kind} {fitness_version} vs build {FITNESS_VERSION}"
        ));
    }
    if level_format_version != LEVEL_FORMAT_VERSION {
        return Err(format!(
            "level_format_version mismatch: {kind} {level_format_version} vs build {LEVEL_FORMAT_VERSION}"
        ));
    }
    if action_repeat == 0 {
        return Err(format!("{kind} records action_repeat = 0"));
    }
    Ok(())
}

/// Training provenance. Everything needed to reproduce, or to explain why you cannot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChampionMetadata {
    pub generation: u32,
    pub genome_id: u32,
    pub level_name: String,
    pub level_hash: String,
    pub level_format_version: u32,
    pub sim_version: String,
    pub sensor_version: u32,
    pub fitness_version: u32,
    pub action_repeat: u32,
    pub episode_seed: u64,
    pub frames: u32,
    pub progress_pixels: f32,
    pub best_x: f32,
    /// Jumps the recorded episode took. Provenance, not a guard — nothing about
    /// reproducing the trajectory depends on it — which is why it is one of the two fields
    /// with a serde default: a champion file written before this field existed still
    /// *parses*, so it is refused by the version trio with the message that explains why
    /// (§8) instead of failing as malformed JSON.
    #[serde(default)]
    pub jumps: u32,
    /// Safe landing zones the recorded episode stood on past a hazard (fitness v4). The
    /// second provenance field, and defaulted for the same reason: it tells a reader how
    /// far the genome got that the frontier alone does not say.
    #[serde(default)]
    pub checkpoints: u32,
    pub outcome: String,
    pub trajectory_hash: String,
    pub training_config: TrainingSummary,
}

/// The subset of [`GaConfig`] that shaped this champion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrainingSummary {
    pub pop_size: usize,
    /// The mutation size the run *started* from. With `adaptive_sigma` the search moves it
    /// from here as it goes, so this is the recipe rather than a constant of the champion.
    pub mutation_sigma: f32,
    pub mutation_rate: f32,
    pub elite: usize,
    pub tournament: usize,
    pub crossover_rate: f32,
    pub master_seed: u64,
    /// Offspring per generation bred as a one-to-three-gene perturbation of the best genome.
    /// Provenance, and defaulted for the same reason [`ChampionMetadata::jumps`] is: a
    /// champion written before this field existed still parses, and is refused by the
    /// version trio with the message that explains why instead of as malformed JSON.
    #[serde(default)]
    pub fine_children: usize,
    /// Whether the run tuned its own mutation size. Defaulted to `false`, which is the
    /// conservative reading: a file that does not say cannot claim the extra search.
    #[serde(default)]
    pub adaptive_sigma: bool,
    /// Generations without a new best before immigrants were bred in; 0 disabled.
    #[serde(default)]
    pub stagnation_generations: u32,
    /// How many arrived when that happened.
    #[serde(default)]
    pub immigrants: usize,
    /// How hot that batch was, as a multiple of the run's mutation size.
    ///
    /// Defaulted to 1.0 rather than to `serde`'s zero for floats, and the difference matters:
    /// 1.0 is what the mechanism shipped with, while 0.0 would claim the run was configured to
    /// reheat at the sigma floor, which no old champion ever was.
    #[serde(default = "default_reheat_factor")]
    pub reheat_factor: f32,
}

/// The value a champion file written before `reheat_factor` existed is read with.
fn default_reheat_factor() -> f32 {
    1.0
}

impl From<&GaConfig> for TrainingSummary {
    fn from(c: &GaConfig) -> Self {
        TrainingSummary {
            pop_size: c.pop_size,
            mutation_sigma: c.mutation_sigma,
            mutation_rate: c.mutation_rate,
            elite: c.elite,
            tournament: c.tournament,
            crossover_rate: c.crossover_rate,
            master_seed: c.master_seed,
            fine_children: c.fine_children,
            adaptive_sigma: c.adaptive_sigma,
            stagnation_generations: c.stagnation_generations,
            immigrants: c.immigrants,
            reheat_factor: c.reheat_factor,
        }
    }
}

/// A serialized champion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChampionFile {
    pub format: String,
    pub version: u32,
    pub network: NetworkSpec,
    pub fitness: f32,
    pub metadata: ChampionMetadata,
}

impl ChampionFile {
    /// Build from the generation report's best genome plus its episode result.
    pub fn new(
        genome: &Genome,
        result: &EpisodeResult,
        level: &Level,
        generation: u32,
        config: &GaConfig,
    ) -> ChampionFile {
        ChampionFile {
            format: CHAMPION_FORMAT.into(),
            version: CHAMPION_VERSION,
            network: NetworkSpec::from_genome(genome),
            fitness: result.fitness,
            metadata: ChampionMetadata {
                generation,
                genome_id: genome.id,
                level_name: level.name.clone(),
                level_hash: level.content_hash_string(),
                level_format_version: LEVEL_FORMAT_VERSION,
                sim_version: SIM_VERSION.into(),
                sensor_version: sensors::VERSION,
                fitness_version: FITNESS_VERSION,
                action_repeat: config.action_repeat,
                episode_seed: result.seed,
                frames: result.frames,
                progress_pixels: result.progress_pixels,
                best_x: result.best_x,
                jumps: result.jumps,
                checkpoints: result.checkpoints,
                outcome: result.outcome.as_str().into(),
                trajectory_hash: format!("{:016x}", result.trajectory_hash),
                training_config: TrainingSummary::from(config),
            },
        }
    }

    /// Rebuild the genome. The declared shape is validated against the flat
    /// vectors before anything runs.
    pub fn to_genome(&self) -> Result<Genome, String> {
        let g = Genome {
            id: self.metadata.genome_id,
            shape: self.network.shape(),
            weights: self.network.weights.clone(),
            biases: self.network.biases.clone(),
            fitness: self.fitness,
        };
        g.validate()?;
        if self.network.activation_hidden != "tanh" || self.network.activation_output != "tanh" {
            return Err(format!(
                "unsupported activations ({:?} / {:?}); this build only implements tanh",
                self.network.activation_hidden, self.network.activation_output
            ));
        }
        Ok(g)
    }

    /// Refuse anything this build cannot reproduce (§8).
    pub fn check_compat(&self, level: &Level) -> Result<(), String> {
        if self.format != CHAMPION_FORMAT {
            return Err(format!(
                "not a champion file: format {:?} (expected {CHAMPION_FORMAT:?})",
                self.format
            ));
        }
        if self.version != CHAMPION_VERSION {
            return Err(format!(
                "champion version {} is not supported (this build reads {CHAMPION_VERSION})",
                self.version
            ));
        }
        check_reproducibility(
            "champion",
            &self.metadata.sim_version,
            self.metadata.sensor_version,
            self.metadata.fitness_version,
            self.metadata.level_format_version,
            self.metadata.action_repeat,
        )?;
        self.network.check_compat("champion")?;
        if self.metadata.level_hash != level.content_hash_string() {
            return Err(format!(
                "level_hash mismatch: champion was trained on {} ({}) but this level hashes to {}",
                self.metadata.level_name,
                self.metadata.level_hash,
                level.content_hash_string()
            ));
        }
        Ok(())
    }

    pub fn to_json_pretty(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("cannot serialize champion: {e}"))
    }

    pub fn from_json_str(src: &str) -> Result<ChampionFile, String> {
        serde_json::from_str(src).map_err(|e| format!("invalid champion JSON: {e}"))
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
        }
        let json = self.to_json_pretty()?;
        std::fs::write(path, json + "\n").map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// Load *and* validate against a level. Prefer this over [`from_json_str`]:
    /// it is the only path that guarantees the champion can be replayed honestly.
    pub fn load_for_level(
        path: impl AsRef<std::path::Path>,
        level: &Level,
    ) -> Result<ChampionFile, String> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let champ = ChampionFile::from_json_str(&src)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        champ
            .check_compat(level)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        champ.to_genome()?;
        Ok(champ)
    }

    /// One-line summary for CLI output.
    ///
    /// Carries the checkpoint count as well as the frontier, because they answer different
    /// questions and the pair is what a hazard level's progress actually looks like: on a
    /// floor of spikes a genome can have all the `best_x` in the world by leaping over them
    /// and none of the landings that went with it.
    pub fn summary(&self) -> String {
        format!(
            "gen {} id {} fitness {:.2} ({}, {} frames, best_x {:.1}, checkpoints {})",
            self.metadata.generation,
            self.metadata.genome_id,
            self.fitness,
            self.metadata.outcome,
            self.metadata.frames,
            self.metadata.best_x,
            self.metadata.checkpoints
        )
    }
}

/// Drive a loaded champion through the shared episode driver, so "watch champion
/// replay" (§10 Tier 2.8) and the round-trip test cannot drift from training.
pub fn replay(
    level: &Level,
    champion: &ChampionFile,
) -> Result<(Genome, EpisodeResult), String> {
    champion.check_compat(level)?;
    let genome = champion.to_genome()?;
    let cfg = crate::fitness::EvalConfig {
        action_repeat: champion.metadata.action_repeat,
    };
    let result = crate::fitness::run_episode(level, &genome, &cfg, champion.metadata.episode_seed);
    Ok((genome, result))
}

/// Convenience used by tests and tooling: does this genome's argmax pick the same
/// action on a fixed observation as it did before serialization?
pub fn actions_agree(a: &Genome, b: &Genome, observations: &[Vec<f32>]) -> bool {
    let mut sa = ActScratch::new();
    let mut sb = ActScratch::new();
    observations
        .iter()
        .all(|o| a.act(o, &mut sa) == b.act(o, &mut sb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitness::EvalConfig;
    use crate::genome::default_shape;
    use sim::rng::rng_from_seed;

    fn level() -> Level {
        crate::fixture::flat_runway("champion-fixture", 24, 22)
    }

    fn champion_for(level: &Level, seed: u64) -> ChampionFile {
        let mut rng = rng_from_seed(seed);
        let g = Genome::random(1, default_shape(), &mut rng);
        let cfg = GaConfig::default();
        let r = crate::fitness::run_episode(level, &g, &cfg.eval_config(), 99);
        ChampionFile::new(&g, &r, level, 3, &cfg)
    }

    #[test]
    fn champion_round_trip_survives_json_and_reproduces_the_same_trajectory() {
        let lv = level();
        let champ = champion_for(&lv, 1);
        let json = champ.to_json_pretty().expect("serializes");
        let back = ChampionFile::from_json_str(&json).expect("parses");
        assert_eq!(champ, back);

        back.check_compat(&lv).expect("compatible");
        let (genome, replayed) = replay(&lv, &back).expect("replayable");

        // Re-simulate from the champion's own recorded seed: identical trajectory.
        let direct = crate::fitness::run_episode(
            &lv,
            &genome,
            &EvalConfig {
                action_repeat: back.metadata.action_repeat,
            },
            back.metadata.episode_seed,
        );
        assert_eq!(replayed.trajectory_hash, direct.trajectory_hash);
        assert_eq!(replayed.frames, direct.frames);
        assert_eq!(replayed.outcome, direct.outcome);
        assert_eq!(
            format!("{:016x}", replayed.trajectory_hash),
            back.metadata.trajectory_hash
        );
        assert_eq!(replayed.fitness, back.fitness);
    }

    #[test]
    fn the_loaded_genome_picks_the_same_actions_as_the_original() {
        let lv = level();
        let mut rng = rng_from_seed(5);
        let g = Genome::random(9, default_shape(), &mut rng);
        let cfg = GaConfig::default();
        let r = crate::fitness::run_episode(&lv, &g, &cfg.eval_config(), 7);
        let champ = ChampionFile::new(&g, &r, &lv, 0, &cfg);
        let back = champ.to_genome().expect("genome rebuilds");
        let obs: Vec<Vec<f32>> = (0..32)
            .map(|i| (0..sensors::N_INPUTS).map(|j| ((i * 31 + j) % 11) as f32 / 11.0 - 0.5).collect())
            .collect();
        assert!(actions_agree(&g, &back, &obs));
        assert_eq!(g.weights, back.weights);
        assert_eq!(g.biases, back.biases);
    }

    /// An older champion file is *refused for the right reason*.
    ///
    /// Version bumps are the one place a parse error and a version error could be confused:
    /// if a new metadata field were required, a pre-v2 champion would fail as malformed JSON
    /// and the reader would go looking for a corrupted file instead of the version trio that
    /// is actually doing its job.
    #[test]
    fn a_champion_file_without_the_new_provenance_field_still_parses() {
        let lv = level();
        let champ = champion_for(&lv, 8);
        let json = champ.to_json_pretty().expect("serializes");
        for field in ["jumps", "checkpoints"] {
            assert!(
                json.contains(&format!("\"{field}\"")),
                "{field} is written: {json}"
            );
        }

        // Every provenance field at once, which is what a file written before either of
        // them existed actually looks like. Stripping one and not the other would leave
        // this test passing for a file that never existed.
        let older: String = json
            .lines()
            .filter(|line| {
                let t = line.trim_start();
                !(t.starts_with("\"jumps\"") || t.starts_with("\"checkpoints\""))
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!older.contains("\"jumps\"") && !older.contains("\"checkpoints\""));
        let back = ChampionFile::from_json_str(&older).expect("an older file still parses");
        assert_eq!(back.metadata.jumps, 0, "and defaults to zero jumps");
        assert_eq!(
            back.metadata.checkpoints, 0,
            "and to zero checkpoints"
        );
    }

    /// The same guarantee as the provenance fields above, for the search knobs: a champion written
    /// before they existed must *parse*, so that the version trio is what refuses it rather than a
    /// JSON error.
    ///
    /// Removed through `serde_json` rather than by stripping lines the way the test above does,
    /// because the last key of an object cannot be taken out textually without leaving a trailing
    /// comma behind and making the file malformed for the wrong reason.
    #[test]
    fn a_champion_file_without_the_search_knobs_still_parses() {
        let champ = champion_for(&level(), 8);
        let json = champ.to_json_pretty().expect("serializes");
        let mut value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let config = value["metadata"]["training_config"]
            .as_object_mut()
            .expect("training_config is an object");
        for field in [
            "fine_children",
            "adaptive_sigma",
            "stagnation_generations",
            "immigrants",
            "reheat_factor",
        ] {
            assert!(config.remove(field).is_some(), "{field} is written");
        }

        let back = ChampionFile::from_json_str(&value.to_string()).expect("an older file parses");
        let run = back.metadata.training_config;
        // The fields that were always there survive the edit, so this is not passing because the
        // whole block went missing.
        assert_eq!(run.pop_size, GaConfig::default().pop_size);
        assert_eq!(run.mutation_sigma, GaConfig::default().mutation_sigma);
        assert_eq!(run.elite, GaConfig::default().elite);
        // And the absent ones land on the values the mechanism shipped with, not on serde's zero:
        // four honest zeros, and a reheat temperature of 1.0 — the factor the random source was
        // replaced with, where 0.0 would claim a run configured to reheat at the sigma floor.
        assert_eq!(run.fine_children, 0);
        assert!(!run.adaptive_sigma);
        assert_eq!(run.stagnation_generations, 0);
        assert_eq!(run.immigrants, 0);
        assert_eq!(run.reheat_factor, 1.0);
    }

    #[test]
    fn a_champion_from_a_different_level_is_refused() {
        let lv = level();
        let champ = champion_for(&lv, 2);
        let mut other = lv.clone();
        other.name = "different".into();
        let err = champ.check_compat(&other).expect_err("must refuse");
        assert!(err.contains("level_hash mismatch"), "{err}");
    }

    #[test]
    fn a_champion_with_a_stale_sensor_version_is_refused() {
        let lv = level();
        let mut champ = champion_for(&lv, 3);
        champ.metadata.sensor_version = sensors::VERSION + 1;
        let err = champ.check_compat(&lv).expect_err("must refuse");
        assert!(err.contains("sensor_version mismatch"), "{err}");
    }

    #[test]
    fn a_champion_with_a_stale_fitness_or_sim_version_is_refused() {
        let lv = level();
        let mut champ = champion_for(&lv, 4);
        champ.metadata.fitness_version = FITNESS_VERSION + 1;
        assert!(champ
            .check_compat(&lv)
            .expect_err("must refuse")
            .contains("fitness_version mismatch"));

        let mut champ = champion_for(&lv, 4);
        champ.metadata.sim_version = "0.0.0-old".into();
        assert!(champ
            .check_compat(&lv)
            .expect_err("must refuse")
            .contains("sim_version mismatch"));
    }

    #[test]
    fn a_champion_with_mismatched_network_dimensions_is_refused() {
        let lv = level();
        let mut champ = champion_for(&lv, 5);
        champ.network.n_inputs = sensors::N_INPUTS + 1;
        assert!(champ
            .check_compat(&lv)
            .expect_err("must refuse")
            .contains("inputs"));
    }

    #[test]
    fn a_truncated_champion_is_refused_before_it_can_run() {
        let lv = level();
        let mut champ = champion_for(&lv, 6);
        champ.network.weights.pop();
        champ.check_compat(&lv).expect("versions still agree");
        assert!(champ.to_genome().is_err(), "shape validation must catch it");
    }

    #[test]
    fn a_non_champion_json_is_refused() {
        assert!(ChampionFile::from_json_str("{\"hello\":1}").is_err());
    }

    #[test]
    fn the_round_trip_holds_for_a_genome_that_solves_the_level() {
        // A hand-biased "always run" genome guarantees a Won outcome, so the
        // round trip is exercised on the interesting branch.
        let lv = level();
        let mut g = Genome::zeros(3, default_shape());
        let n_out = g.shape.outputs;
        let out_b_off = g.shape.n_biases() - n_out;
        g.biases[out_b_off] = 1.0;
        let cfg = GaConfig::default();
        let r = crate::fitness::run_episode(&lv, &g, &cfg.eval_config(), 1);
        assert!(r.won(), "fixture genome should win, got {:?}", r.outcome);

        let champ = ChampionFile::new(&g, &r, &lv, 0, &cfg);
        assert_eq!(champ.metadata.outcome, sim::Outcome::Won.as_str());
        let (_, replayed) = replay(&lv, &champ).expect("replay");
        assert!(replayed.won());
        assert_eq!(replayed.trajectory_hash, r.trajectory_hash);
    }
}
