//! **The player's own playthroughs, recorded and fitted** (SMARTER_AI_PLAN.md, Item 7).
//!
//! The strongest teaching signal this project can get is a person completing the level, and
//! until this module it was thrown away: the playtest already runs every tick through the same
//! [`sim::Sim`] and the same action mapping the AI uses, so the run existed — it was just never
//! written down.
//!
//! A [`HumanDemo`] is that run as data: the observation the sim exposed at each decision block and
//! the action the player was holding. The observation is recorded, not only the key, because a
//! clone has to be fitted to the state the human actually saw; a key press alone says nothing about
//! the pit in front of it. Nothing here filters the demonstration through planner agreement — a
//! human run that used a technique the planner does not know is worth *more* than a planner route,
//! not less, which is the plan's own instruction and the reason [`fit`] consumes the recorded
//! decisions directly.
//!
//! **Determinism.** A demo is a function of the level, the seed and the keys held; the replay of
//! its decisions is exact because the sim is. The version trio and the level hash are recorded and
//! refused on load for the same reason a champion's are: a clone fitted to a run under different
//! physics or senses is a clone of a different game.
//!
//! The file lives beside the other per-level artifacts, in [`DEMO_DIR`], named
//! `<level-stem>.demo.json`; the app writes it on the tick a human playthrough touches the flag and
//! never for a death or an abandoned run.

use serde::{Deserialize, Serialize};
use sim::physics::AgentAction;
use sim::{Level, SIM_VERSION};

use crate::genome::NetShape;
use crate::imitate::{clone_from_play, ImitateConfig, ImitationReport};
use crate::route_play::{Decision, RoutePlay};
use sim::sensors::{self, N_INPUTS};

/// The artifact's own identity: a file that says this is a demo is one this build can read.
pub const DEMO_FORMAT: &str = "evo-platformer-demo";
pub const DEMO_VERSION: u32 = 1;

/// Where demos are written, beside `champions/`. A constant rather than a parameter because the
/// playtest has no out-directory notion of its own — it plays the level it was handed — and a file
/// nobody can find is a demonstration nobody can fit.
pub const DEMO_DIR: &str = "demos";

/// One decision of a human playthrough: the state, and the action held through it.
///
/// The action is stored as its index rather than as the enum, so the file is readable across a
/// build that re-labels an action without renumbering the table — the same convention the action
/// table itself uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DemoDecision {
    /// The physics frame the decision was made on (always a multiple of the cadence).
    pub tick: u32,
    /// Index into [`AgentAction::ALL`].
    pub action: u8,
    /// The observation **before** the decision's first tick.
    pub observation: Vec<f32>,
}

/// A recorded human playthrough of one level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HumanDemo {
    pub format: String,
    pub version: u32,
    pub level_name: String,
    pub level_hash: String,
    pub sim_version: String,
    pub sensor_version: u32,
    pub action_repeat: u32,
    pub seed: u64,
    /// Whether the run touched the flag. Only a completed run is written by the playtest; the
    /// field is recorded so a file assembled by hand is honest about what it is.
    pub won: bool,
    pub decisions: Vec<DemoDecision>,
}

impl HumanDemo {
    /// Build a demo from a walked [`RoutePlay`] — the shape the playtest and the route replay both
    /// produce.
    pub fn from_play(level: &Level, play: &RoutePlay, action_repeat: u32) -> HumanDemo {
        let decisions = play
            .decisions
            .iter()
            .enumerate()
            .map(|(i, d)| DemoDecision {
                tick: i as u32 * action_repeat.max(1),
                action: d.action.index() as u8,
                observation: d.observation.to_vec(),
            })
            .collect();
        HumanDemo {
            format: DEMO_FORMAT.into(),
            version: DEMO_VERSION,
            level_name: level.name.clone(),
            level_hash: level.content_hash_string(),
            sim_version: SIM_VERSION.into(),
            sensor_version: sensors::VERSION,
            action_repeat: action_repeat.max(1),
            seed: play.seed,
            won: play.won(),
            decisions,
        }
    }

    /// Build a demo from decisions collected directly by a front-end (the playtest's per-tick
    /// recorder).
    pub fn from_decisions(
        level: &Level,
        seed: u64,
        won: bool,
        action_repeat: u32,
        decisions: Vec<DemoDecision>,
    ) -> HumanDemo {
        HumanDemo {
            format: DEMO_FORMAT.into(),
            version: DEMO_VERSION,
            level_name: level.name.clone(),
            level_hash: level.content_hash_string(),
            sim_version: SIM_VERSION.into(),
            sensor_version: sensors::VERSION,
            action_repeat: action_repeat.max(1),
            seed,
            won,
            decisions,
        }
    }

    /// The demo as the decision sequence the imitation fit consumes.
    ///
    /// Errors rather than panicking on a malformed file: a demonstration with an observation of
    /// the wrong width is a file from a different sensor layout that slipped past the version
    /// check, and a clone fitted to it would be arithmetic on the wrong cells.
    pub fn to_play(&self) -> Result<RoutePlay, String> {
        if self.decisions.is_empty() {
            return Err("the demonstration has no decisions".into());
        }
        let mut decisions = Vec::with_capacity(self.decisions.len());
        for (i, d) in self.decisions.iter().enumerate() {
            if d.observation.len() != N_INPUTS {
                return Err(format!(
                    "decision {i} carries a {}-wide observation, but this build's sensors produce \
                     {N_INPUTS}",
                    d.observation.len()
                ));
            }
            let action = AgentAction::ALL
                .get(d.action as usize)
                .copied()
                .ok_or_else(|| {
                    format!(
                        "decision {i} names action {} of {}",
                        d.action,
                        AgentAction::COUNT
                    )
                })?;
            let mut observation = [0.0f32; N_INPUTS];
            observation.copy_from_slice(&d.observation);
            decisions.push(Decision {
                observation,
                action,
            });
        }
        let last_tick = self.decisions.last().map_or(0, |d| d.tick);
        Ok(RoutePlay {
            decisions,
            segments: vec![0],
            outcome: if self.won {
                sim::Outcome::Won
            } else {
                sim::Outcome::Running
            },
            frames: last_tick + self.action_repeat,
            seed: self.seed,
            hops_closed: 0,
            best_block: 0,
            best_stand: None,
            route_hops: None,
            note: format!("human demonstration on {}", self.level_name),
            trace: Vec::new(),
        })
    }

    /// Write the demo, creating [`DEMO_DIR`] if it does not exist.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot serialise the demonstration: {e}"))?;
        std::fs::write(path, json + "\n")
            .map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// Path the playtest uses for this level's demonstration.
    pub fn path_for(level: &Level) -> std::path::PathBuf {
        std::path::Path::new(DEMO_DIR).join(format!("{}.demo.json", stem(&level.name)))
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> Result<HumanDemo, String> {
        let path = path.as_ref();
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        HumanDemo::from_json_str(&src).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_json_str(src: &str) -> Result<HumanDemo, String> {
        serde_json::from_str(src).map_err(|e| format!("malformed demonstration: {e}"))
    }

    /// Refuse anything this build cannot honestly fit.
    ///
    /// The same guards a champion loader applies, minus the ones that are about *scoring* a
    /// genome: a demonstration is not a claim about a level, it is a record of one, so the level
    /// hash still has to match but the fitness version does not exist to check.
    pub fn check_compat(&self, level: &Level) -> Result<(), String> {
        if self.format != DEMO_FORMAT {
            return Err(format!(
                "not a demonstration: format {:?} (expected {DEMO_FORMAT:?})",
                self.format
            ));
        }
        if self.version != DEMO_VERSION {
            return Err(format!(
                "demonstration version {} is not supported (this build reads {DEMO_VERSION})",
                self.version
            ));
        }
        if self.sim_version != SIM_VERSION {
            return Err(format!(
                "sim_version mismatch: demonstration {} vs build {SIM_VERSION}",
                self.sim_version
            ));
        }
        if self.sensor_version != sensors::VERSION {
            return Err(format!(
                "sensor_version mismatch: demonstration {} vs build {}",
                self.sensor_version,
                sensors::VERSION
            ));
        }
        if self.level_hash != level.content_hash_string() {
            return Err(format!(
                "level_hash mismatch: demonstration was recorded on {} ({}) but this level hashes \
                 to {}",
                self.level_name,
                self.level_hash,
                level.content_hash_string()
            ));
        }
        if self.action_repeat == 0 {
            return Err("demonstration records action_repeat = 0".into());
        }
        if self.decisions.is_empty() {
            return Err("demonstration has no decisions".into());
        }
        Ok(())
    }
}

/// Fit a clone to a human demonstration.
///
/// Deliberately no planner in the loop: the recorded decisions are the labels, whatever technique
/// produced them. `shape` is the run's own shape, so the clone is an ordinary member of the
/// population and nothing downstream has to know it came from a person.
pub fn fit(
    demo: &HumanDemo,
    shape: NetShape,
    cfg: &ImitateConfig,
) -> Result<ImitationReport, String> {
    let play = demo.to_play()?;
    clone_from_play(&play, shape, cfg)
}

/// A level name as a file stem: lower-case, runs of anything else collapsed to one dash.
fn stem(name: &str) -> String {
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
    if out.is_empty() {
        "level".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::default_shape;
    use crate::imitate::ImitateConfig;
    use sim::Sim;

    fn corridor() -> Level {
        crate::fixture::flat_runway("demo-corridor", 24, 22)
    }

    /// A short constant-action walk: the fixture a fit can learn in a few dozen epochs.
    fn walk(level: &Level, action: AgentAction, decisions: usize) -> HumanDemo {
        let repeat = sim::DECISION_TICKS;
        let mut sim = Sim::new(level, 3);
        let mut recorded = Vec::with_capacity(decisions);
        for i in 0..decisions {
            if sim.outcome().is_finished() {
                break;
            }
            recorded.push(DemoDecision {
                tick: i as u32 * repeat,
                action: action.index() as u8,
                observation: sim.observation().to_vec(),
            });
            for _ in 0..repeat {
                sim.step(action);
            }
        }
        HumanDemo::from_decisions(
            level,
            3,
            sim.outcome() == sim::Outcome::Won,
            repeat,
            recorded,
        )
    }

    #[test]
    fn a_demonstration_round_trips_through_json() {
        let level = corridor();
        let demo = walk(&level, AgentAction::Run, 30);
        demo.check_compat(&level)
            .expect("a fresh demo is compatible");
        let json = serde_json::to_string_pretty(&demo).expect("serialises");
        let back = HumanDemo::from_json_str(&json).expect("parses");
        assert_eq!(back, demo);
        assert_eq!(
            back.to_play().expect("converts").decisions.len(),
            demo.decisions.len()
        );
    }

    #[test]
    fn a_stale_demonstration_is_refused_with_the_reason() {
        let level = corridor();
        let mut demo = walk(&level, AgentAction::Run, 10);
        demo.sensor_version = sensors::VERSION + 1;
        let err = demo
            .check_compat(&level)
            .expect_err("stale senses are refused");
        assert!(err.contains("sensor_version mismatch"), "{err}");

        let mut demo = walk(&level, AgentAction::Run, 10);
        demo.sim_version = "0.0.0".into();
        let err = demo
            .check_compat(&level)
            .expect_err("stale physics is refused");
        assert!(err.contains("sim_version mismatch"), "{err}");

        let mut demo = walk(&level, AgentAction::Run, 10);
        demo.action_repeat = 0;
        let err = demo
            .check_compat(&level)
            .expect_err("a zero cadence is refused");
        assert!(err.contains("action_repeat"), "{err}");
    }

    #[test]
    fn a_demonstration_from_another_level_is_refused() {
        let level = corridor();
        let other = crate::fixture::flat_runway("demo-other", 24, 20);
        let demo = walk(&level, AgentAction::Run, 10);
        let err = demo
            .check_compat(&other)
            .expect_err("a demonstration is about one level");
        assert!(err.contains("level_hash mismatch"), "{err}");
    }

    /// **The point of the module.** A human walk is a set of (state, action) pairs a fit can learn:
    /// agreement has to beat the 1/11 chance floor by a clear margin, or the demonstration is not
    /// a teaching signal.
    #[test]
    fn a_corridor_demonstration_fits_a_clone_above_chance() {
        let level = corridor();
        let demo = walk(&level, AgentAction::Run, 40);
        let report = fit(
            &demo,
            default_shape(),
            &ImitateConfig {
                epochs: 60,
                ..ImitateConfig::default()
            },
        )
        .expect("the demonstration fits");
        assert!(
            report.agreement > 0.5,
            "agreement {:.2} is not above chance: {}",
            report.agreement,
            report.line()
        );
        assert!(report.genome.validate().is_ok());
    }
}
