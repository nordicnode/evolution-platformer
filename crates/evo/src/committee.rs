//! A committee of genomes, for **presentation only**.
//!
//! SMARTER_AI_PLAN.md Item 10. The window, the replay and the title-screen demo all show one
//! genome, which makes that genome a single point of failure: its mistakes are idiosyncratic, and
//! five top genomes rarely make the same mistake on the same tick. This module is the vote — each
//! member evaluates the same observation, and the action with the most votes is taken.
//!
//! **Why it is not a training artifact.** A committee is not a genome: it has no genes, no id and
//! no single trajectory, so it cannot be scored as one, cannot be saved as a champion, and must
//! never be smuggled into a difficulty-table row as if it were a genome. The record stays the
//! single champion the run certified; the committee is a way to *show* the run's result. A caller
//! that wants to present a committee as the level's solver has to validate it first, the same way
//! a champion is validated, or the claim is a picture rather than a measurement.
//!
//! **Determinism is the other hard requirement.** The vote is over the action indices, the
//! members are kept in the order the caller gave them (the leaderboard's order), and a tie is
//! broken by the *earliest member that voted for a tied action* — not by hash-map order, not by
//! the action's own index. Two runs of the same committee on the same observation therefore make
//! the same decision, which is what lets a saved replay reproduce the picture.
//!
//! Each member owns its own [`ActScratch`], so a recurrent member carries its own memory between
//! decisions; [`Committee::reset_context`] is the episode boundary for all of them at once.
//!
//! **The gate.** [`Committee::validate`] plays the committee and its best member on the same
//! fixed seeds and reports both. [`CommitteeReport::passes`] is the plan's rule — the committee
//! ships only if it is never worse than its best member on completion — and an equality passes,
//! because a committee that ties its best member has not lost the skill it is presenting. A
//! caller that shows a committee without this report is showing a picture, not a measurement;
//! `crates/app/src/replay.rs`'s `committee_showcase` is the presentation helper and says so.

use serde::{Deserialize, Serialize};
use sim::physics::AgentAction;
use sim::{Level, Outcome, Sim};

use crate::champion::NetworkSpec;
use crate::fitness::{run_episode, EvalConfig};
use crate::ga::validation_seed;
use crate::genome::{ActScratch, Genome, NetShape};

/// A level name as a file stem: lower case, one dash for any run of other characters.
///
/// The same rule as the app's `lab::slug` and the demo writer's stem, and deliberately repeated
/// rather than shared across crates: the file name is a contract between the writer (`train`)
/// and the reader (the title screen), so both sides spell it the same way and a test pins it.
fn level_slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// What a committee file is called. A distinct format rather than a champion or a population:
/// a committee is ranked members and no single score, and reading it as either of the others
/// would invite exactly the category error the module docs warn about.
pub const COMMITTEE_FORMAT: &str = "evo-committee";
pub const COMMITTEE_VERSION: u32 = 1;
/// Directory beside `champions/` that a run writes its committee into.
pub const COMMITTEE_DIR: &str = "committees";

/// **The persisted form of a committee (Item 10).**
///
/// Members are in leaderboard order and carry their own network spec, so the file is
/// self-describing: the vote and its tie-break can be reconstructed without the run that made
/// it. The level hash, sensor version and cadence are checked on load for the same reason a
/// champion's are — a committee assembled from another layout would vote on the wrong cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitteeFile {
    pub format: String,
    pub version: u32,
    pub level_name: String,
    pub level_hash: String,
    /// The run's master seed, so [`Self::validate`] can play the run's own fixed validation
    /// episodes rather than an invented set: the exam has to be the one the champion faced.
    pub master_seed: u64,
    pub validation_episodes: u32,
    pub action_repeat: u32,
    pub sensor_version: u32,
    /// Leaderboard order: index 0 is the run's best member and the tie-break's first voice.
    pub members: Vec<NetworkSpec>,
}

impl CommitteeFile {
    /// Build from already-ranked genomes (`members[0]` is the best) and the run that produced
    /// them. The file stores what validation needs and nothing that could be mistaken for a
    /// fitness claim.
    pub fn new(
        level: &Level,
        master_seed: u64,
        validation_episodes: u32,
        action_repeat: u32,
        members: &[Genome],
    ) -> Result<CommitteeFile, String> {
        if members.is_empty() {
            return Err("a committee needs at least one member".into());
        }
        Committee::new(members.to_vec())?;
        Ok(CommitteeFile {
            format: COMMITTEE_FORMAT.into(),
            version: COMMITTEE_VERSION,
            level_name: level.name.clone(),
            level_hash: level.content_hash_string(),
            master_seed,
            validation_episodes: validation_episodes.clamp(1, 16),
            action_repeat: action_repeat.max(1),
            sensor_version: sim::sensors::VERSION,
            members: members.iter().map(NetworkSpec::from_genome).collect(),
        })
    }

    /// The committee the file describes, in file order.
    pub fn to_committee(&self) -> Result<Committee, String> {
        let genomes: Vec<Genome> = self
            .members
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                let shape = spec.shape();
                Genome {
                    id: i as u32,
                    shape,
                    weights: spec.weights.clone(),
                    biases: spec.biases.clone(),
                    fitness: 0.0,
                }
            })
            .collect();
        Committee::new(genomes)
    }

    /// Refuse a file this build cannot honestly present beside a level.
    pub fn check_compat(&self, level: &Level) -> Result<(), String> {
        if self.format != COMMITTEE_FORMAT {
            return Err(format!(
                "not a committee: format {:?} (expected {COMMITTEE_FORMAT:?})",
                self.format
            ));
        }
        if self.version != COMMITTEE_VERSION {
            return Err(format!(
                "committee version {} is not supported (this build reads {COMMITTEE_VERSION})",
                self.version
            ));
        }
        if self.sensor_version != sim::sensors::VERSION {
            return Err(format!(
                "sensor_version mismatch: committee {} vs build {}",
                self.sensor_version,
                sim::sensors::VERSION
            ));
        }
        if self.level_hash != level.content_hash_string() {
            return Err(format!(
                "level_hash mismatch: committee was built on {} ({}) but this level hashes to {}",
                self.level_name,
                self.level_hash,
                level.content_hash_string()
            ));
        }
        if self.members.is_empty() {
            return Err("committee file has no members".into());
        }
        Ok(())
    }

    /// **The Item 10 gate, played out on the run's own validation set.**
    ///
    /// The seeds are [`validation_seed`]'s — the fixed episodes no generation drew — so the
    /// answer is comparable to the champion's held-out reading. A caller that wants a wider
    /// exam can pass its own seeds to [`Committee::validate`] instead.
    pub fn validate(&self, level: &Level) -> Result<CommitteeReport, String> {
        self.check_compat(level)?;
        let cfg = crate::ga::GaConfig {
            master_seed: self.master_seed,
            action_repeat: self.action_repeat,
            ..crate::ga::GaConfig::default()
        };
        let seeds: Vec<u64> = (0..self.validation_episodes)
            .map(|i| validation_seed(&cfg, i))
            .collect();
        let eval = EvalConfig {
            action_repeat: self.action_repeat,
            cut_stuck_frames: 0,
        };
        Ok(self.to_committee()?.validate(level, &eval, &seeds))
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot serialise the committee: {e}"))?;
        std::fs::write(path, json + "\n")
            .map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> Result<CommitteeFile, String> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        serde_json::from_str(&src).map_err(|e| format!("{}: malformed committee: {e}", path.display()))
    }

    /// Where a run on `level` writes its committee: beside `champions/`, named after the level.
    ///
    /// The slug collapses runs of separators and trims both ends, which is the same rule the
    /// playtest's demo names and the app's champion lookup use. It has to be: the title screen
    /// rebuilds this name from the level it opened, so two different slug rules would write a
    /// file the window never finds.
    pub fn path_for(level: &Level) -> std::path::PathBuf {
        std::path::Path::new(COMMITTEE_DIR).join(format!(
            "{}.committee.json",
            level_slug(&level.name)
        ))
    }
}

/// The top `k` genomes of a run, voting on one action at a time.
#[derive(Clone, Debug)]
pub struct Committee {
    /// In leaderboard order: index 0 is the run's best genome. The order is the tie-break.
    members: Vec<Genome>,
    /// One scratch per member, so each recurrent member remembers its own previous decision.
    scratches: Vec<ActScratch>,
    /// The shared shape. A committee is one observation layout, voted on by every member.
    shape: NetShape,
}

impl Committee {
    /// Build a committee from genomes already in leaderboard order.
    ///
    /// Refuses an empty list (there is nothing to vote) and members of different shapes (a
    /// committee is one observation layout, not a parliament of different senses). Every genome is
    /// validated here rather than on the first decision, so a corrupt member is a construction
    /// error instead of a mid-replay surprise.
    pub fn new(members: Vec<Genome>) -> Result<Committee, String> {
        let Some(first) = members.first() else {
            return Err("a committee needs at least one member".to_string());
        };
        let shape = first.shape.clone();
        for (i, genome) in members.iter().enumerate() {
            genome
                .validate()
                .map_err(|e| format!("committee member {i}: {e}"))?;
            if genome.shape != shape {
                return Err(format!(
                    "committee member {i} is {} but the first is {}: a committee votes on one observation",
                    genome.shape.label(),
                    shape.label()
                ));
            }
        }
        let scratches = members.iter().map(|_| ActScratch::new()).collect();
        Ok(Committee {
            members,
            scratches,
            shape,
        })
    }

    /// Build from a **ranked** population: the first `k` genomes, in the order given.
    ///
    /// The order is the leaderboard's, and it is the committee's tie-break, so this is the
    /// constructor a run's ending should use: `Trainer` hands out its ranking and the caller does
    /// not have to sort again. `k` is clamped to at least one and to the population's size — a
    /// call that asks for five members of a three-genome population gets the three, because a
    /// committee is what the run actually produced rather than a padded fiction.
    pub fn from_ranked(genomes: &[Genome], k: usize) -> Result<Committee, String> {
        if genomes.is_empty() {
            return Err("a committee needs at least one genome to rank".to_string());
        }
        let take = k.max(1).min(genomes.len());
        Committee::new(genomes[..take].to_vec())
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// The members, in leaderboard order. Index 0 is the best member and the tie-break's first
    /// voice; the slice is read-only because a committee whose order moved would not be the same
    /// committee.
    pub fn members(&self) -> &[Genome] {
        &self.members
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The observation width every member expects. Exposed so a caller can size the vector without
    /// reaching into a member's shape.
    pub fn inputs(&self) -> usize {
        self.shape.inputs
    }

    /// Forget every member's recurrent state. Call this at an episode boundary, or a second
    /// episode begins holding the tail of the first one's thoughts.
    pub fn reset_context(&mut self) {
        for scratch in &mut self.scratches {
            scratch.reset_context();
        }
    }

    /// One decision: every member's argmax, then the majority.
    ///
    /// Ties go to the action supported by the earliest member among the tied actions. A committee
    /// of five splitting 2–2–1 therefore returns the action whose two supporters include the
    /// higher-ranked member, which is the rule the plan states and the one that keeps the picture
    /// reproducible.
    pub fn act(&mut self, inputs: &[f32]) -> AgentAction {
        let mut votes = [0u32; AgentAction::COUNT];
        let mut earliest = [usize::MAX; AgentAction::COUNT];
        for (i, (genome, scratch)) in self
            .members
            .iter()
            .zip(self.scratches.iter_mut())
            .enumerate()
        {
            let index = genome.act(inputs, scratch).index();
            votes[index] += 1;
            if earliest[index] == usize::MAX {
                earliest[index] = i;
            }
        }

        let mut best = 0usize;
        for action in 1..AgentAction::COUNT {
            let ahead = votes[action] > votes[best]
                || (votes[action] == votes[best] && earliest[action] < earliest[best]);
            if ahead {
                best = action;
            }
        }
        AgentAction::from_index(best)
    }

    /// **The plan's gate, played out.**
    ///
    /// Every seed is played twice on the same level: once by the committee, driving a `Sim`
    /// directly at the evaluation cadence and resetting the members' memory at each episode start,
    /// and once by the committee's first member as an ordinary genome through
    /// [`crate::fitness::run_episode`]. The second reading is the baseline the plan names — "the
    /// committee against its own best member" — and it goes through the same episode runner the
    /// trainer and a champion replay use, so the baseline is not a second implementation of what
    /// an episode is.
    ///
    /// `&self` rather than `&mut self`: validation is a question, not a state change. The vote
    /// needs mutable scratches, so the method plays a private clone and leaves the committee the
    /// caller holds untouched — which is what lets a window validate a committee and then show it
    /// from the same object without a reset in between.
    pub fn validate(&self, level: &Level, cfg: &EvalConfig, seeds: &[u64]) -> CommitteeReport {
        let repeat = cfg.action_repeat.max(1);
        let mut committee = self.clone();
        let mut won = 0u32;
        let mut frames = 0u32;
        for &seed in seeds {
            committee.reset_context();
            let mut sim = Sim::new(level, seed);
            let mut action = AgentAction::Walk;
            while !sim.outcome().is_finished() {
                if sim.stats().frames % repeat == 0 {
                    action = committee.act(&sim.observation());
                }
                sim.step(action);
            }
            if sim.outcome() == Outcome::Won {
                won += 1;
                if frames == 0 {
                    frames = sim.stats().frames;
                }
            }
        }

        let best = &self.members[0];
        let mut best_member_won = 0u32;
        let mut best_member_frames = 0u32;
        for &seed in seeds {
            let result = run_episode(level, best, cfg, seed);
            if result.won() {
                best_member_won += 1;
                if best_member_frames == 0 {
                    best_member_frames = result.frames;
                }
            }
        }

        CommitteeReport {
            won,
            episodes: seeds.len() as u32,
            frames,
            best_member_won,
            best_member_frames,
        }
    }
}

/// What a committee did, beside what its best member did, on the same seeds.
///
/// Deliberately counts and clocks rather than a boolean: [`CommitteeReport::passes`] is the
/// shipping rule, but the numbers are what a reader needs to see whether a committee tied its
/// member on every episode or merely lost none of the ones it happened to be asked. Frames are
/// the first winning episode's, `0` when that side won nothing — the same choice a champion's
/// metadata makes, where a clock only exists for a win.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitteeReport {
    pub won: u32,
    pub episodes: u32,
    pub frames: u32,
    pub best_member_won: u32,
    pub best_member_frames: u32,
}

impl CommitteeReport {
    /// The plan's gate: the committee ships only if it is never worse than its best member on
    /// completion. Equality passes — a committee that ties has not lost what it presents — and a
    /// validation with no episodes at all passes vacuously, which is why `episodes` is reported
    /// beside the rule: a caller that wants evidence must supply seeds.
    pub fn passes(&self) -> bool {
        self.won >= self.best_member_won
    }
}

/// **The committee's actions for one episode, as a stream a presentation could replay.**
///
/// Item 10's presentation half, kept in the library rather than in a front-end because it is the
/// mechanism and not a drawing: a `Replay` replays a *recorded* episode whose star is an
/// `EpisodeRunner` owning one genome, and a committee has no such episode, so this records the
/// committee's own decision stream from the ordinary evaluation loop — decide every
/// `EvalConfig::action_repeat` ticks, step the sim once per tick. A viewer that wants to show a
/// committee replays this stream, and only after [`Committee::validate`] has certified it.
///
/// The stream is deterministic: the committee is built in the order given (the leaderboard's),
/// its tie-break is that order, and its memory is reset at the start of the episode.
pub struct CommitteeShowcase {
    /// One decision per cadence boundary, in the order the committee made them.
    pub actions: Vec<AgentAction>,
    /// `actions.len()`: the episode's decision count, which is what a cadence change moves.
    pub decisions: usize,
    pub frames: u32,
    pub outcome: Outcome,
}

impl CommitteeShowcase {
    /// Did the committee reach the flag? The same reading a champion's recorded episode carries.
    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
    }
}

/// Drive `members`' top `k` as a committee for one episode on `level`, and return its decisions.
///
/// `members` must already be in leaderboard order; [`Committee::from_ranked`] takes the prefix
/// and refuses an empty run.
pub fn showcase(
    members: &[Genome],
    level: &Level,
    eval: &EvalConfig,
    seed: u64,
    k: usize,
) -> Result<CommitteeShowcase, String> {
    let mut committee = Committee::from_ranked(members, k)?;
    let repeat = eval.action_repeat.max(1);
    let mut sim = Sim::new(level, seed);
    let mut action = AgentAction::Walk;
    let mut actions = Vec::new();
    while !sim.outcome().is_finished() {
        if sim.stats().frames % repeat == 0 {
            action = committee.act(&sim.observation());
            actions.push(action);
        }
        sim.step(action);
    }
    Ok(CommitteeShowcase {
        decisions: actions.len(),
        actions,
        frames: sim.stats().frames,
        outcome: sim.outcome(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::default_shape;
    use sim::rng::rng_from_seed;

    /// A genome whose output layer votes for `action` whatever the observation: zero weights, and
    /// one large output bias. `tanh` saturates the chosen output above the others.
    fn always(id: u32, action: AgentAction) -> Genome {
        let shape = default_shape();
        let mut genome = Genome::zeros(id, shape.clone());
        let output = genome.biases.len() - shape.outputs;
        genome.biases[output + action.index()] = 8.0;
        genome
    }

    fn observation() -> Vec<f32> {
        vec![0.0; default_shape().inputs]
    }

    #[test]
    fn a_showcase_stream_is_deterministic_and_refuses_an_empty_roster() {
        let level = crate::fixture::flat_runway("committee-showcase", 24, 22);
        let members: Vec<Genome> = (0..5).map(|i| always(i, AgentAction::Run)).collect();
        let eval = EvalConfig::default();
        let a = showcase(&members, &level, &eval, 7, 5).expect("a committee");
        let b = showcase(&members, &level, &eval, 7, 5).expect("a committee");
        assert_eq!(a.actions, b.actions);
        assert_eq!(a.decisions, a.actions.len());
        assert!(a.decisions > 0);
        assert!(a.frames > 0);
        assert!(showcase(&[], &level, &eval, 7, 5).is_err());
    }

    #[test]
    fn the_majority_wins() {
        let chosen = AgentAction::Run;
        let other = AgentAction::JumpInPlace;
        let mut committee =
            Committee::new(vec![always(0, chosen), always(1, chosen), always(2, other)])
                .expect("a three-member committee");
        assert_eq!(committee.act(&observation()), chosen);
    }

    #[test]
    fn a_tie_goes_to_the_earliest_member_who_voted_for_it() {
        // Votes read B, A, A, B, C: two each for A and B. A's first supporter is member 1, B's is
        // member 0, so the leaderboard order breaks the tie toward B.
        let a = AgentAction::Run;
        let b = AgentAction::Brake;
        let c = AgentAction::Walk;
        let mut committee = Committee::new(vec![
            always(0, b),
            always(1, a),
            always(2, a),
            always(3, b),
            always(4, c),
        ])
        .expect("a five-member committee");
        assert_eq!(committee.act(&observation()), b);
    }

    #[test]
    fn a_committee_of_one_is_exactly_its_genome() {
        let shape = default_shape();
        let mut rng = rng_from_seed(11);
        let genome = Genome::random(0, shape.clone(), &mut rng);
        let obs = observation();

        let mut scratch = ActScratch::new();
        let alone = genome.act(&obs, &mut scratch);

        let mut committee = Committee::new(vec![genome]).expect("one member");
        assert_eq!(committee.act(&obs), alone);
        assert_eq!(committee.len(), 1);
        assert!(!committee.is_empty());
        assert_eq!(committee.inputs(), obs.len());
    }

    /// The run-ending constructor: it takes the prefix in the order given, and refuses a list with
    /// nothing in it. A `k` larger than the population is the population, not an error.
    #[test]
    fn from_ranked_takes_the_prefix_and_refuses_an_empty_population() {
        assert!(Committee::from_ranked(&[], 5).is_err());

        let mut rng = rng_from_seed(13);
        let genomes: Vec<Genome> = (0..3)
            .map(|i| Genome::random(i, default_shape(), &mut rng))
            .collect();
        let committee = Committee::from_ranked(&genomes, 5).expect("three is a committee");
        assert_eq!(committee.len(), 3);
        assert_eq!(committee.members(), genomes.as_slice());

        let one = Committee::from_ranked(&genomes, 0).expect("at least one member");
        assert_eq!(one.members()[0].id, genomes[0].id);
    }

    /// The gate's arithmetic, and the case it is there to refuse: a committee whose vote loses an
    /// episode its own best member wins must not be shown as the level's solver.
    #[test]
    fn the_committee_must_not_lose_to_its_best_member_to_pass() {
        let level = crate::fixture::flat_runway("committee-gate", 20, 18);
        let eval = EvalConfig::default();
        let seeds = [3u64, 5, 7];

        // A constant `Run` policy solves this runway; a committee that votes to jump in place does
        // not, while its first member still does. That is the loss the gate exists for.
        let losing = Committee::new(vec![
            always(0, AgentAction::Run),
            always(1, AgentAction::JumpInPlace),
            always(2, AgentAction::JumpInPlace),
        ])
        .expect("three members");
        let report = losing.validate(&level, &eval, &seeds);
        assert_eq!(report.episodes, 3);
        assert_eq!(
            report.best_member_won, 3,
            "the best member solves the runway"
        );
        assert_eq!(report.won, 0, "the vote never runs");
        assert_eq!(report.frames, 0, "no winning episode, no clock");
        assert!(!report.passes());

        // One member is exactly its genome, so the two readings agree and equality passes.
        let alone = Committee::new(vec![always(0, AgentAction::Run)]).expect("one member");
        let report = alone.validate(&level, &eval, &seeds);
        assert_eq!(report.won, report.best_member_won);
        assert_eq!(report.frames, report.best_member_frames);
        assert!(report.passes());
    }

    /// Item 10's artifact: the file round-trips to the same vote and refuses a level it was not
    /// built for. Without this the presentation half is an in-memory object that no viewer can
    /// load, which is the gap the splash screen's docs used to record.
    #[test]
    fn a_committee_file_round_trips_and_refuses_another_level() {
        let level = crate::fixture::flat_runway("committee-file", 20, 18);
        let members: Vec<Genome> = (0..3)
            .map(|i| always(i, AgentAction::Run))
            .collect();
        let file = CommitteeFile::new(&level, 7, 12, 4, &members).expect("a file");
        assert_eq!(
            CommitteeFile::path_for(&level).file_name().unwrap(),
            "committee-file.committee.json",
            "the train side and the title screen must spell the name the same way"
        );
        assert_eq!(file.members.len(), 3);
        assert_eq!(file.master_seed, 7);

        let json = serde_json::to_string(&file).expect("serialises");
        let back: CommitteeFile = serde_json::from_str(&json).expect("reads back");
        assert_eq!(back, file);
        let committee = back.to_committee().expect("the members rebuild");
        assert_eq!(committee.len(), 3);
        back.check_compat(&level).expect("same level");

        let other = crate::fixture::flat_runway("other-level", 20, 18);
        let err = back
            .check_compat(&other)
            .expect_err("a committee is a claim about one level");
        assert!(err.contains("level_hash"), "{err}");
    }

    #[test]
    fn a_committee_needs_members_of_one_shape() {
        assert!(Committee::new(Vec::new()).is_err());

        let wide = Genome::zeros(0, default_shape());
        let narrow_shape = NetShape {
            hidden: vec![8],
            ..default_shape()
        };
        let narrow = Genome::zeros(1, narrow_shape);
        let err = Committee::new(vec![wide, narrow]).expect_err("different shapes");
        assert!(err.contains("committee"), "{err}");
    }
}
