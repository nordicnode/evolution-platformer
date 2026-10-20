//! Population checkpoint — the "restart from where it got stuck" artifact (§12 M5).
//!
//! A champion file records one genome and the episode that earned it. A population
//! checkpoint records **the whole population at one generation**, which is what a
//! curriculum step needs: the stalled population is not a loser to be thrown away
//! but the best starting material the run has produced, and moving it to an easier
//! level — and back — is the plan's answer to a run that stalls forever (§13).
//!
//! Two deliberate differences from [`crate::champion`]:
//!
//! * **The level hash is a note, not a gate.** A champion that does not hash to the
//!   level you hand it is refused, because replaying it would be a lie. A
//!   population checkpoint is *expected* to be resumed on a different level — that
//!   is the entire point of a curriculum — so [`PopulationFile::level_note`] reports
//!   the change to the operator instead of refusing it. Everything that would make
//!   a resumed run unable to reproduce itself (versions, sensor width, action
//!   cadence, network shape) is still refused.
//! * **Genome ids are carried explicitly**, because they are part of every
//!   `episode_seed` (§5). A resume that renumbered the population would silently
//!   train on different episodes than the ones the checkpoint was stalled on.

use serde::{Deserialize, Serialize};
use sim::level::LEVEL_FORMAT_VERSION;
use sim::sensors;
use sim::{Level, SIM_VERSION};

use crate::champion::{check_reproducibility, NetworkSpec, TrainingSummary};
use crate::fitness::FITNESS_VERSION;
use crate::ga::GaConfig;
use crate::genome::Genome;

pub const POPULATION_FORMAT: &str = "evo-platformer-population";
pub const POPULATION_VERSION: u32 = 1;

/// A serialized population checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PopulationFile {
    pub format: String,
    pub version: u32,
    /// Why this checkpoint exists — the stall, or the operator's reason.
    pub note: String,
    pub generation: u32,
    pub level_name: String,
    pub level_hash: String,
    pub level_format_version: u32,
    pub sim_version: String,
    pub sensor_version: u32,
    pub fitness_version: u32,
    pub action_repeat: u32,
    pub training_config: TrainingSummary,
    /// Genome id per network, index-aligned with `networks`.
    pub genome_ids: Vec<u32>,
    pub networks: Vec<NetworkSpec>,
    /// The run's **best-ever genome**, when the file was written by a run that had one.
    ///
    /// The population alone is not enough to resume a solved run: the record can be a mutation
    /// that won once and was then lost from the cohort, and a checkpoint written without it makes
    /// the resumed run unable to breed the only lineage that ever reached the flag. The genome is
    /// re-evaluated on load ([`crate::ga::Trainer::seed_record_from_checkpoint`]), so this is a candidate rather
    /// than a recorded win. Defaulted so every checkpoint written before this field existed still
    /// loads.
    #[serde(default)]
    pub best_id: Option<u32>,
    #[serde(default)]
    pub best: Option<NetworkSpec>,
}

impl PopulationFile {
    /// Snapshot a live population.
    pub fn new(
        genomes: &[Genome],
        generation: u32,
        level: &Level,
        config: &GaConfig,
        note: impl Into<String>,
    ) -> PopulationFile {
        PopulationFile {
            format: POPULATION_FORMAT.into(),
            version: POPULATION_VERSION,
            note: note.into(),
            generation,
            level_name: level.name.clone(),
            level_hash: level.content_hash_string(),
            level_format_version: LEVEL_FORMAT_VERSION,
            sim_version: SIM_VERSION.into(),
            sensor_version: sensors::VERSION,
            fitness_version: FITNESS_VERSION,
            action_repeat: config.action_repeat.max(1),
            training_config: TrainingSummary::from(config),
            genome_ids: genomes.iter().map(|g| g.id).collect(),
            networks: genomes.iter().map(NetworkSpec::from_genome).collect(),
            best_id: None,
            best: None,
        }
    }

    /// Attach the run's best-ever genome to the checkpoint.
    ///
    /// Builder-shaped rather than a fifth argument to [`PopulationFile::new`]: a checkpoint of a
    /// population is a complete artifact on its own, and the record is extra provenance the caller
    /// has or does not.
    pub fn with_best(mut self, best: Option<&Genome>) -> PopulationFile {
        self.best_id = best.map(|g| g.id);
        self.best = best.map(NetworkSpec::from_genome);
        self
    }

    /// Rebuild the recorded best-ever genome, if the checkpoint carried one.
    pub fn best_genome(&self) -> Result<Option<Genome>, String> {
        let Some(net) = &self.best else {
            return Ok(None);
        };
        let id = self
            .best_id
            .ok_or_else(|| "checkpoint has a best network but no genome id".to_string())?;
        let genome = Genome {
            id,
            shape: net.shape(),
            weights: net.weights.clone(),
            biases: net.biases.clone(),
            fitness: 0.0,
        };
        genome.validate()?;
        Ok(Some(genome))
    }

    /// Rebuild the population, in the order it was saved.
    ///
    /// Every genome is validated, and the ids are checked to be unique here rather
    /// than trusted: duplicate ids would make two members of the population share
    /// an `episode_seed`, so they would be evaluated on the same episodes and the
    /// population would quietly lose diversity.
    pub fn to_genomes(&self) -> Result<Vec<Genome>, String> {
        if self.networks.len() != self.genome_ids.len() {
            return Err(format!(
                "checkpoint has {} networks but {} genome ids",
                self.networks.len(),
                self.genome_ids.len()
            ));
        }
        let mut genomes = Vec::with_capacity(self.networks.len());
        for (net, id) in self.networks.iter().zip(&self.genome_ids) {
            let g = Genome {
                id: *id,
                shape: net.shape(),
                weights: net.weights.clone(),
                biases: net.biases.clone(),
                // A population's members carry no fitness of their own: selection
                // re-evaluates every generation, so a stored number would be a
                // number nobody measured this generation.
                fitness: 0.0,
            };
            g.validate()?;
            genomes.push(g);
        }
        if genomes.is_empty() {
            return Err("checkpoint holds no genomes".into());
        }
        let mut ids: Vec<u32> = genomes.iter().map(|g| g.id).collect();
        ids.sort_unstable();
        if let Some(dup) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("checkpoint has duplicate genome id {}", dup[0]));
        }
        Ok(genomes)
    }

    /// Refuse anything this build could not train on (§8).
    pub fn check_compat(&self) -> Result<(), String> {
        if self.format != POPULATION_FORMAT {
            return Err(format!(
                "not a population checkpoint: format {:?} (expected {POPULATION_FORMAT:?})",
                self.format
            ));
        }
        if self.version != POPULATION_VERSION {
            return Err(format!(
                "checkpoint version {} is not supported (this build reads {POPULATION_VERSION})",
                self.version
            ));
        }
        check_reproducibility(
            "checkpoint",
            &self.sim_version,
            self.sensor_version,
            self.fitness_version,
            self.level_format_version,
            self.action_repeat,
        )?;
        for net in &self.networks {
            net.check_compat("checkpoint")?;
        }
        self.to_genomes()?;
        Ok(())
    }

    /// What the operator should know about resuming this checkpoint *here*.
    ///
    /// `None` when the level is the one the checkpoint came from. A different level
    /// is not an error — it is the curriculum — but it changes what the run means,
    /// so it is said out loud instead of being inferred from the fitness chart.
    pub fn level_note(&self, level: &Level) -> Option<String> {
        let here = level.content_hash_string();
        if self.level_hash == here {
            return None;
        }
        Some(format!(
            "checkpoint was taken on {} ({}) and is resuming on {} ({}): fitness before and after are not comparable",
            self.level_name,
            &self.level_hash[..self.level_hash.len().min(16)],
            level.name,
            &here[..here.len().min(16)]
        ))
    }

    pub fn to_json_pretty(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot serialize population checkpoint: {e}"))
    }

    pub fn from_json_str(src: &str) -> Result<PopulationFile, String> {
        serde_json::from_str(src).map_err(|e| format!("invalid population JSON: {e}"))
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
        std::fs::write(path, json + "\n")
            .map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// Load a checkpoint, refusing anything this build cannot train on.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<PopulationFile, String> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let file =
            PopulationFile::from_json_str(&src).map_err(|e| format!("{}: {e}", path.display()))?;
        file.check_compat()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(file)
    }

    /// One-line summary for CLI output.
    pub fn summary(&self) -> String {
        format!(
            "gen {} population of {} on {} ({})",
            self.generation,
            self.networks.len(),
            self.level_name,
            self.note
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::flat_runway;
    use crate::genome::default_shape;
    use sim::rng::rng_from_seed;

    fn level() -> Level {
        flat_runway("population-fixture", 24, 22)
    }

    fn population(n: u32) -> Vec<Genome> {
        let mut rng = rng_from_seed(7);
        (1..=n)
            .map(|id| Genome::random(id, default_shape(), &mut rng))
            .collect()
    }

    fn checkpoint() -> PopulationFile {
        PopulationFile::new(
            &population(4),
            38,
            &level(),
            &GaConfig::default(),
            "stalled for 25 generations",
        )
    }

    /// The record travels with the cohort, and an older file without it still loads.
    #[test]
    fn a_checkpoint_can_carry_the_runs_best_ever_genome() {
        let mut original = population(4);
        let best = original.remove(2);
        let file = checkpoint().with_best(Some(&best));
        assert_eq!(file.best_id, Some(best.id));
        let rebuilt = file
            .best_genome()
            .expect("the record parses")
            .expect("the record is present");
        assert_eq!(rebuilt.id, best.id);
        assert_eq!(rebuilt.weights, best.weights);
        assert_eq!(rebuilt.biases, best.biases);

        // Through the text form too, which is what a checkpoint file actually is.
        let json = file.to_json_pretty().expect("serializes");
        let back = PopulationFile::from_json_str(&json).expect("parses");
        assert_eq!(back.best_id, Some(best.id));

        // A file written before this field existed has no record and must still load. Removed
        // through `serde_json` rather than by stripping lines: the last key of an object cannot
        // be taken out textually without leaving a trailing comma behind.
        let mut value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let object = value.as_object_mut().expect("a checkpoint is an object");
        assert!(object.remove("best").is_some(), "best is written");
        assert!(object.remove("best_id").is_some(), "best_id is written");
        let old =
            PopulationFile::from_json_str(&value.to_string()).expect("an older checkpoint parses");
        assert_eq!(old.best_genome().expect("no record is not an error"), None);
    }

    #[test]
    fn a_population_round_trips_with_its_ids_and_order() {
        let file = checkpoint();
        let genomes = file.to_genomes().expect("loads");
        assert_eq!(genomes.len(), 4);
        assert_eq!(
            genomes.iter().map(|g| g.id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4],
            "ids must survive the round trip: they are part of every episode_seed"
        );
        // The weights have to come back bit for bit, or a resumed run is a
        // different run wearing the same ids.
        let original = population(4);
        for (a, b) in original.iter().zip(&genomes) {
            assert_eq!(a.weights, b.weights);
            assert_eq!(a.biases, b.biases);
            assert_eq!(a.shape, b.shape);
        }
        assert_eq!(file.generation, 38);
        assert!(file.summary().contains("stalled for 25 generations"));

        // And through the text form, which is what a file actually is.
        let json = file.to_json_pretty().expect("serializes");
        assert_eq!(PopulationFile::from_json_str(&json).expect("parses"), file);
    }

    #[test]
    fn a_checkpoint_refuses_what_this_build_cannot_train_on() {
        let mut stale = checkpoint();
        stale.sensor_version = sensors::VERSION + 1;
        assert!(
            stale
                .check_compat()
                .unwrap_err()
                .contains("sensor_version mismatch"),
            "{:?}",
            stale.check_compat().unwrap_err()
        );

        let mut old_fitness = checkpoint();
        old_fitness.fitness_version = FITNESS_VERSION + 1;
        assert!(old_fitness
            .check_compat()
            .unwrap_err()
            .contains("fitness_version mismatch"));

        let mut wrong_net = checkpoint();
        wrong_net.networks[2].n_inputs = 4;
        assert!(wrong_net.check_compat().unwrap_err().contains("inputs"));

        let mut other_kind = checkpoint();
        other_kind.format = "evo-platformer-champion".into();
        assert!(other_kind
            .check_compat()
            .unwrap_err()
            .contains("not a population checkpoint"));

        let mut future = checkpoint();
        future.version = POPULATION_VERSION + 1;
        assert!(future
            .check_compat()
            .unwrap_err()
            .contains("is not supported"));

        let mut zero_repeat = checkpoint();
        zero_repeat.action_repeat = 0;
        assert!(zero_repeat.check_compat().is_err());
    }

    #[test]
    fn a_checkpoint_with_missing_or_duplicated_genomes_is_refused() {
        let mut short = checkpoint();
        short.networks.pop();
        assert!(short
            .to_genomes()
            .unwrap_err()
            .contains("networks but 4 genome ids"));

        let mut dupes = checkpoint();
        dupes.genome_ids[3] = dupes.genome_ids[0];
        let err = dupes.to_genomes().unwrap_err();
        assert!(err.contains("duplicate genome id"), "{err}");

        let mut empty = checkpoint();
        empty.networks.clear();
        empty.genome_ids.clear();
        assert!(empty.to_genomes().unwrap_err().contains("no genomes"));
    }

    #[test]
    fn a_different_level_is_reported_rather_than_refused() {
        // The curriculum *is* resuming on another level, so this must not be an
        // error — but it must not be silent either, because the fitness before and
        // after are not comparable.
        let file = checkpoint();
        let same = level();
        assert_eq!(file.level_note(&same), None);

        let other = flat_runway("another-fixture", 40, 38);
        let note = file.level_note(&other).expect("a different level is noted");
        assert!(note.contains("population-fixture"), "{note}");
        assert!(note.contains("another-fixture"), "{note}");
        assert!(note.contains("not comparable"), "{note}");
        // Still a valid checkpoint for that level: only the note changed.
        assert!(file.check_compat().is_ok());
    }

    #[test]
    fn a_checkpoint_file_on_disk_loads_back() {
        let dir = std::env::temp_dir().join(format!("evo-pop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("stalled.population.json");
        let file = checkpoint();
        file.save(&path).expect("saves, creating the directory");
        let back = PopulationFile::load(&path).expect("loads");
        assert_eq!(back, file);
        assert_eq!(back.to_genomes().expect("loads").len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
