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

use crate::fitness::{run_episode, EpisodeResult, FITNESS_VERSION};
use crate::ga::{episode_seed_for, GaConfig, Validation};
use crate::genome::{ActScratch, Genome, NetShape, MAX_FEEDBACK};

pub const CHAMPION_FORMAT: &str = "evo-platformer-champion";
pub const CHAMPION_VERSION: u32 = 1;

/// The network, flattened for serialization.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetworkSpec {
    pub n_inputs: usize,
    pub n_hidden: Vec<usize>,
    pub n_outputs: usize,
    /// Width of the network's recurrent state. `0` is a memoryless feedforward net, which is
    /// what every file written before this field existed holds; defaulted on load for exactly
    /// that reason, so an older champion reads back as the net it was rather than as a
    /// malformed one.
    #[serde(default)]
    pub feedback: usize,
    /// Item 12's cadence head: one extra neuron after the action rows. Defaulted so every file
    /// written before the head existed reads back as a fixed-cadence policy.
    #[serde(default)]
    pub commitment: bool,
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
            feedback: g.shape.feedback,
            commitment: g.shape.commitment,
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
            feedback: self.feedback,
            commitment: self.commitment,
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
        if self.feedback > MAX_FEEDBACK {
            return Err(format!(
                "{kind} network feeds back {} values but this build builds at most {MAX_FEEDBACK}",
                self.feedback
            ));
        }
        // The weight count has to match the shape, memory included: the feedback matrix is
        // `feedback x hidden[0]` weights that live at the front of the first layer, so a file
        // whose genes were written for a memoryless net but whose spec claims memory would
        // drive a network that read its own genes one offset out.
        let shape = self.shape();
        let n_w = shape.n_weights();
        let n_b = shape.n_biases();
        if self.weights.len() != n_w || self.biases.len() != n_b {
            return Err(format!(
                "{kind} network has {} weights and {} biases but its shape needs {n_w} and {n_b}",
                self.weights.len(),
                self.biases.len()
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
    /// **Item 8, step 3: the training/held-out gap, per champion.**
    ///
    /// The two win rates a phase-memorising policy separates: how often the genome won the
    /// episodes it was *trained* on, against how often it won the run's fixed validation set. A
    /// positive reading means it wins more on the phases it saw than on the ones it did not, which
    /// is the failure the narrow draw was suspected of and the thing a difficulty table could not
    /// show before this field.
    ///
    /// `None` when there is nothing to compare against: a run with validation switched off, or a
    /// level with nothing stochastic in it (where the generation's own episode already *is* the
    /// held-out reading, so the gap is zero by construction and quoting it would be a number that
    /// cannot move). Defaulted, so every champion written before the reading existed still loads.
    #[serde(default)]
    pub phase_gap: Option<PhaseGap>,
}

/// A champion's win rate on the phases it trained on against its rate on held-out phases.
///
/// Rates rather than counts at the comparison site, counts at the storage site: a reader needs
/// `wins / episodes` to compare a 3-episode training draw with a 12-episode validation set, and a
/// stored rate would round away the arithmetic that makes the number reproducible.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhaseGap {
    /// Episodes of the training draw that reached the flag.
    pub train_wins: u32,
    /// Episodes in the training draw.
    pub train_episodes: u32,
    /// Episodes of the held-out set that reached the flag.
    pub held_wins: u32,
    /// Episodes in the held-out set.
    pub held_episodes: u32,
}

impl PhaseGap {
    /// The record's win rate on the episodes it was selected on.
    pub fn training_rate(&self) -> f32 {
        ratio(self.train_wins, self.train_episodes)
    }

    /// The record's win rate on the run's fixed validation set.
    pub fn held_rate(&self) -> f32 {
        ratio(self.held_wins, self.held_episodes)
    }

    /// Positive when the policy wins more on the phases it saw. The reading itself.
    pub fn gap(&self) -> f32 {
        self.training_rate() - self.held_rate()
    }
}

fn ratio(wins: u32, episodes: u32) -> f32 {
    if episodes == 0 {
        0.0
    } else {
        wins as f32 / episodes as f32
    }
}

/// **Measure a record's training/held-out gap** (Item 8, step 3).
///
/// The training half is *replayed*, not remembered: the record's own generation and id derive the
/// seeds it was judged on ([`episode_seed_for`]), so the same genome on the same trials is the
/// honest training reading even for a champion loaded from a file. The held-out half is the
/// trainer's own [`Validation`], which is the only place the validation seeds exist.
///
/// Costs `effective_trials` episodes, once, when a champion is written — never per generation.
/// Returns `None` when there is no held-out reading to compare with.
pub fn measure_phase_gap(
    level: &Level,
    genome: &Genome,
    config: &GaConfig,
    generation: u32,
    held: Option<Validation>,
) -> Option<PhaseGap> {
    let held = held?;
    let eval = config.eval_config();
    let trials = config.effective_trials(level);
    let mut train_wins = 0;
    for trial in 0..trials {
        let seed = episode_seed_for(config, generation, genome.id, trial);
        if run_episode(level, genome, &eval, seed).won() {
            train_wins += 1;
        }
    }
    Some(PhaseGap {
        train_wins,
        train_episodes: trials,
        held_wins: held.wins,
        held_episodes: held.episodes,
    })
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
    /// Share of the fine children bred with the jump-timing operator (WP1). Defaulted to 0.0 for
    /// a file written before the operator existed: that run had none, and claiming half would
    /// describe a search it did not run.
    #[serde(default)]
    pub jump_timing_share: f32,
    /// Whether the run was **configured for the mastery league** (WP11). The league itself arms
    /// only once the record solves, so `true` here means the second phase was available, not that
    /// it necessarily ran. Defaulted to `false` for a file written before the mode existed: that
    /// run ranked on the crowned fitness throughout, which is what "no league" means.
    #[serde(default)]
    pub mastery: bool,
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
    /// **The early cut the run trained with** (Item 5), in no-progress ticks; 0 is off.
    /// Provenance with teeth: a champion's recorded fitness is the run's own result, and a cut
    /// run's results are scored on episodes that were ended early, while every replay evaluates
    /// uncut. A file that recorded the fact but let the mismatch through would be worse than one
    /// that refused, so [`ChampionFile::save`] refuses a summary with this set. Defaulted to 0:
    /// a file written before the cut existed never trained with it.
    #[serde(default)]
    pub cut_stuck_frames: u32,
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
            jump_timing_share: c.jump_timing_share,
            mastery: c.mastery,
            adaptive_sigma: c.adaptive_sigma,
            stagnation_generations: c.stagnation_generations,
            immigrants: c.immigrants,
            reheat_factor: c.reheat_factor,
            cut_stuck_frames: c.cut_stuck_frames,
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
                phase_gap: None,
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

    /// **Carry a champion across an append-only sensor bump without changing its policy.**
    ///
    /// Item 3's migration, in the artifact rather than in the arithmetic: a champion whose
    /// `sensor_version` is exactly the one before this build's is grown to the current vector by
    /// [`Genome::grown_to_inputs`], which appends zero weights so every old input keeps its index
    /// and every new input is unreadable. The metadata's `sensor_version` moves with it; the
    /// recorded episode and its hash stay valid because the policy is unchanged.
    ///
    /// Deliberately narrow. Only the **previous** layout is migrated, and only in the growth
    /// direction: a champion two bumps old would need two re-readings composed, and a champion
    /// whose patch semantics changed under it (a bump that draws a new entity into the grid) is
    /// not function-preserving on a level with one in the 9×9 patch. Refusing those is the whole
    /// reason this returns `Err` rather than silently growing whatever it is handed.
    ///
    /// Returns `true` when the file was migrated, `false` when it was already current. The file
    /// on disk is not rewritten: migration is a load-time reading, and a caller that wants the new
    /// shape saved says so.
    pub fn migrate_sensors(&mut self) -> Result<bool, String> {
        if self.metadata.sensor_version == sensors::VERSION {
            return Ok(false);
        }
        if self.metadata.sensor_version.checked_add(1) != Some(sensors::VERSION) {
            return Err(format!(
                "sensor_version mismatch: champion {} vs build {}; only the previous layout is \
                 migrated, and this build cannot grow a v{} network",
                self.metadata.sensor_version,
                sensors::VERSION,
                self.metadata.sensor_version
            ));
        }
        if self.network.n_inputs > sensors::N_INPUTS {
            return Err(format!(
                "champion takes {} inputs but this build's sensors produce {}: a migration cannot \
                 shrink an observation",
                self.network.n_inputs,
                sensors::N_INPUTS
            ));
        }
        let old = self.to_genome()?;
        let grown = old.grown_to_inputs(sensors::N_INPUTS)?;
        self.network = NetworkSpec::from_genome(&grown);
        self.metadata.sensor_version = sensors::VERSION;
        Ok(true)
    }

    /// **Give a champion the cadence head without changing a decision.**
    ///
    /// Item 12's artifact-side migration: the network is grown by
    /// [`Genome::grown_with_commitment`] (a zero output row, which reads as a four-tick
    /// commitment), the spec is rewritten and the metadata's versions are left alone because
    /// neither the senses nor the fitness moved. The recorded episode stays valid, and the test
    /// in `tests/migration.rs` replays it to the same clock and frontier.
    pub fn adopt_commitment(&mut self) -> Result<bool, String> {
        if self.network.commitment {
            return Ok(false);
        }
        let old = self.to_genome()?;
        let grown = old.grown_with_commitment()?;
        self.network = NetworkSpec::from_genome(&grown);
        Ok(true)
    }

    /// Refuse anything this build cannot reproduce (§8).
    pub fn check_compat(&self, level: &Level) -> Result<(), String> {
        self.check_trainable()?;
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

    /// Refuse a file this build could not **train on**, ignoring which level it came from.
    ///
    /// The whole of [`ChampionFile::check_compat`] except the level hash, and the pair exists
    /// because the hash is answering two different questions. As a *claim* ("this genome scores
    /// 1 240 on this level") the hash is the point: a champion replayed on another level would be
    /// a lie, which is why `replay` and the window's view of a champion go through
    /// `check_compat`. As *starting material* it is the wrong question entirely: the interesting
    /// seed is a champion from a level that is not this one — the collision the designer just
    /// moved, or the harder level the same policy should face — and refusing it would refuse
    /// exactly the case [`crate::ga::Trainer::from_champion`] exists for. What is *not* relaxed is
    /// anything that would make the seeded run unreproducible or the genes unreadable: the format,
    /// this build's version, the version trio, the action cadence, the activations and the gene
    /// counts against the declared shape are all still refusals.
    pub fn check_trainable(&self) -> Result<(), String> {
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
        if self.network.activation_hidden != "tanh" || self.network.activation_output != "tanh" {
            return Err(format!(
                "unsupported activations ({:?} / {:?}); this build only implements tanh",
                self.network.activation_hidden, self.network.activation_output
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
        // A champion is a claim that its recorded episode replays exactly, and replay always
        // evaluates uncut. A run trained with the early cut scored its record on episodes that
        // were ended early, so its numbers cannot keep that promise: refuse the file rather
        // than write a record the replay screen would contradict. Training with the cut stays
        // available for measurement (bench arms record themselves); it just cannot crown.
        if self.metadata.training_config.cut_stuck_frames > 0 {
            return Err(format!(
                "refusing to write a champion trained with --cut-stuck {}: its recorded \
                 fitness comes from cut episodes and would not reproduce on replay; \
                 re-run without the cut to crown a champion",
                self.metadata.training_config.cut_stuck_frames
            ));
        }
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

    /// Load *and* validate against a level. Prefer this over [`from_json_str`]:
    /// it is the only path that guarantees the champion can be replayed honestly.
    pub fn load_for_level(
        path: impl AsRef<std::path::Path>,
        level: &Level,
    ) -> Result<ChampionFile, String> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut champ =
            ChampionFile::from_json_str(&src).map_err(|e| format!("{}: {e}", path.display()))?;
        champ
            .migrate_sensors()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        champ
            .check_compat(level)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        champ.to_genome()?;
        Ok(champ)
    }

    /// Load a champion as **starting material** rather than as a claim about a level.
    ///
    /// [`ChampionFile::load_for_level`] with the level hash left out of the check (see
    /// [`ChampionFile::check_trainable`] for why that is the right half to drop when the question
    /// is "what should this run start from" rather than "what does this genome score"). The
    /// caller is expected to say out loud which level the file came from — [`ChampionFile::level_note`]
    /// is the sentence, and it is written here rather than in a front-end for the same reason
    /// `PopulationFile::level_note` is: three front-ends each inventing their own wording is how
    /// one of them ends up not saying it at all.
    pub fn load_seed(path: impl AsRef<std::path::Path>) -> Result<ChampionFile, String> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut champ =
            ChampionFile::from_json_str(&src).map_err(|e| format!("{}: {e}", path.display()))?;
        champ
            .migrate_sensors()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        champ
            .check_trainable()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        champ.to_genome()?;
        Ok(champ)
    }

    /// What the operator should know about starting a run *here* from this champion.
    ///
    /// `None` when the level is the one the champion was trained on — a fine-tune, where the
    /// fitness before and after are comparable. A different level is not an error (it is the point
    /// of seeding), but it does change what the run means, so it is said out loud instead of being
    /// left to be inferred from two files in two directories.
    pub fn level_note(&self, level: &Level) -> Option<String> {
        let here = level.content_hash_string();
        if self.metadata.level_hash == here {
            return None;
        }
        Some(format!(
            "champion was trained on {} ({}) and is seeding a run on {} ({}): fitness before and after are not comparable",
            self.metadata.level_name,
            &self.metadata.level_hash[..self.metadata.level_hash.len().min(16)],
            level.name,
            &here[..here.len().min(16)]
        ))
    }

    /// One-line summary for CLI output.
    ///
    /// Carries the checkpoint count as well as the frontier, because they answer different
    /// questions and the pair is what a hazard level's progress actually looks like: on a
    /// floor of spikes a genome can have all the `best_x` in the world by leaping over them
    /// and none of the landings that went with it.
    /// One line about the file, for the terminal.
    ///
    /// The clock is written as a **time** and the frame count beside it, rather than the frame count
    /// alone: 505 frames is the number that reproduces the episode and 8.42 s is the number a person
    /// races, and every front-end's champion line goes through here (train, watch, replay) so the two
    /// are never two different answers. `sim::format_time` is the same formatter the window's panel
    /// and the playtest's stopwatch use, which is the point of it being one function.
    pub fn summary(&self) -> String {
        format!(
            "gen {} id {} fitness {:.2} ({} in {}, {} frames, best_x {:.1}, checkpoints {})",
            self.metadata.generation,
            self.metadata.genome_id,
            self.fitness,
            self.metadata.outcome,
            sim::format_time(self.metadata.frames),
            self.metadata.frames,
            self.metadata.best_x,
            self.metadata.checkpoints
        )
    }
}

/// Drive a loaded champion through the shared episode driver, so "watch champion
/// replay" (§10 Tier 2.8) and the round-trip test cannot drift from training.
pub fn replay(level: &Level, champion: &ChampionFile) -> Result<(Genome, EpisodeResult), String> {
    champion.check_compat(level)?;
    let genome = champion.to_genome()?;
    let cfg = crate::fitness::EvalConfig {
        action_repeat: champion.metadata.action_repeat,
        cut_stuck_frames: 0,
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

    /// **A cut run cannot crown.** The champion's recorded fitness is the run's own result and
    /// replay evaluates uncut, so a file written from a `--cut-stuck` run would promise an
    /// episode its numbers did not play. `save` refuses, and the refusal names the knob.
    #[test]
    fn a_champion_trained_with_the_cut_refuses_to_be_written() {
        let lv = level();
        let mut rng = rng_from_seed(5);
        let g = Genome::random(1, default_shape(), &mut rng);
        let cfg = GaConfig {
            cut_stuck_frames: 60,
            ..GaConfig::default()
        };
        let r = crate::fitness::run_episode(&lv, &g, &cfg.eval_config(), 99);
        let champ = ChampionFile::new(&g, &r, &lv, 3, &cfg);
        assert_eq!(champ.metadata.training_config.cut_stuck_frames, 60);
        let path =
            std::env::temp_dir().join(format!("evo-cut-champion-{}.json", std::process::id()));
        let err = champ.save(&path).expect_err("a cut run must not crown");
        assert!(err.contains("--cut-stuck"), "{err}");
        assert!(!path.exists(), "the refused champion must not reach disk");

        // The same genome crowned by an uncut run writes fine.
        let clean = champion_for(&lv, 5);
        clean.save(&path).expect("an uncut champion writes");
        std::fs::remove_file(&path).ok();
    }

    /// **Item 8's reading is arithmetic, and it round-trips through the file.** A champion written
    /// without a held-out reading says so (`None`), the field is optional on load, and when both
    /// halves exist the gap is exactly `training − held-out` and the training half was measured on
    /// the trials the run judged the record on.
    #[test]
    fn the_phase_gap_is_the_difference_between_the_two_win_rates_and_is_optional() {
        let lv = level();
        let mut rng = rng_from_seed(17);
        let genome = Genome::random(3, default_shape(), &mut rng);
        let cfg = GaConfig::default();

        // No validation reading means no gap: a number over one half of the comparison would be
        // a claim the run cannot make.
        assert_eq!(measure_phase_gap(&lv, &genome, &cfg, 4, None), None);

        let held = Validation {
            wins: 2,
            episodes: 4,
            mean: 100.0,
        };
        let gap = measure_phase_gap(&lv, &genome, &cfg, 4, Some(held)).expect("both halves exist");
        assert_eq!(gap.train_episodes, cfg.effective_trials(&lv));
        assert_eq!(gap.held_wins, 2);
        assert_eq!(gap.held_episodes, 4);
        assert_eq!(gap.gap(), gap.training_rate() - gap.held_rate());
        assert!((gap.held_rate() - 0.5).abs() < 1e-6);

        // The reading survives a save and a load, and a file written before the field existed
        // still parses with `None`.
        let mut champ = champion_for(&lv, 2);
        champ.metadata.phase_gap = Some(gap);
        let text = champ.to_json_pretty().expect("serialises");
        let back = ChampionFile::from_json_str(&text).expect("parses");
        assert_eq!(back.metadata.phase_gap, Some(gap));
        let mut value: serde_json::Value = serde_json::from_str(&text).expect("json");
        let removed = value["metadata"]
            .as_object_mut()
            .expect("metadata is an object")
            .remove("phase_gap");
        assert!(removed.is_some(), "the field really is in the JSON");
        let older = serde_json::to_string_pretty(&value).expect("serialises");
        let old = ChampionFile::from_json_str(&older).expect("an older file still parses");
        assert_eq!(old.metadata.phase_gap, None);
    }

    /// The one line every front-end prints about a file says the clock as a **time** as well as a
    /// frame count, and both are that episode's own numbers.
    ///
    /// The frame count is what reproduces the episode and the time is what a person races, and a
    /// terminal that printed only the count left the reader to divide by sixty. One function writes
    /// both (`sim::format_time`), so a champion line and the window's panel cannot disagree.
    #[test]
    fn a_champions_summary_says_its_clock_as_a_time_and_a_frame_count() {
        let lv = level();
        let champ = champion_for(&lv, 1);
        let line = champ.summary();
        let frames = champ.metadata.frames;
        assert!(
            line.contains(&format!("{} frames", frames)),
            "the frame count is what reproduces the episode: {line}"
        );
        assert!(
            line.contains(&sim::format_time(frames)),
            "and the time is what a person races: {line}"
        );
        assert_eq!(
            sim::format_time(frames),
            format!("{:.2}s", frames as f64 / 60.0)
        );
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
                cut_stuck_frames: 0,
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
            .map(|i| {
                (0..sensors::N_INPUTS)
                    .map(|j| ((i * 31 + j) % 11) as f32 / 11.0 - 0.5)
                    .collect()
            })
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
        assert_eq!(back.metadata.checkpoints, 0, "and to zero checkpoints");
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
            "jump_timing_share",
            "mastery",
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
        assert_eq!(run.jump_timing_share, 0.0);
        assert!(!run.mastery);
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
        // The structural check catches this one now, and it is the stronger place to catch it:
        // nothing about the build changed (the version trio agrees), while the genes and the
        // declared shape disagree, which is a corrupt file rather than a stale one. Still
        // refused before anything can run, which is the property the test is about.
        let err = champ.check_compat(&lv).expect_err("must refuse");
        assert!(err.contains("weights"), "{err}");
        assert!(
            champ.to_genome().is_err(),
            "and the shape check would catch it too"
        );
    }

    /// A network with memory survives the file, and a file written before memory existed
    /// reads back as the memoryless net it was.
    #[test]
    fn memory_is_recorded_in_the_file_and_replays_identically() {
        let lv = level();
        let mut shape = default_shape();
        shape.feedback = 8;
        let mut rng = rng_from_seed(31);
        let g = Genome::random(9, shape.clone(), &mut rng);
        let cfg = GaConfig::default();
        let r = crate::fitness::run_episode(&lv, &g, &cfg.eval_config(), 3);
        let champ = ChampionFile::new(&g, &r, &lv, 0, &cfg);
        assert_eq!(
            champ.network.feedback, 8,
            "the file records the state width"
        );

        let json = champ.to_json_pretty().expect("serializes");
        let back = ChampionFile::from_json_str(&json).expect("parses");
        assert_eq!(back.network.feedback, 8);
        assert_eq!(back.network.shape(), shape);
        back.check_compat(&lv).expect("compatible");

        // The replay builds its own scratch, so a recurrent policy has to replay from a clean
        // state to reproduce: this is the invariant the whole project rests on.
        let (_, replayed) = replay(&lv, &back).expect("replay");
        assert_eq!(replayed.trajectory_hash, r.trajectory_hash);
        assert_eq!(replayed.outcome, r.outcome);

        // A file from before the field existed: gone from the JSON, memoryless on load.
        let mut old: serde_json::Value = serde_json::from_str(&json).expect("json");
        old["network"]
            .as_object_mut()
            .expect("network object")
            .remove("feedback");
        let old = ChampionFile::from_json_str(&old.to_string()).expect("an older file parses");
        assert_eq!(
            old.network.feedback, 0,
            "absence means memoryless, not unreadable"
        );
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
