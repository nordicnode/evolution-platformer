//! **Discovery detection and the trick codex (WP16).**
//!
//! A run that gets better is not yet a story. This module is the part that knows *what* was
//! found: it turns the technique counters (WP13) and the shortcut trace (WP14) into named
//! [`Discovery`] values, keeps them in a per-level [`Codex`], and gives the window something
//! concrete to say — a name, one line of explanation, and a replay.
//!
//! **Names come from a fixed table, not from a generator.** A discovered trick is something
//! players talk about, and the only way a name can be talked about is if the same trick is
//! called the same thing every time it is found. The technique rules live in
//! [`crate::technique::TECHNIQUE_RULES`], so the firsts a trainer reports and the discoveries
//! this module presents cannot drift apart; shortcut names are the small ladder below.
//!
//! **A discovery is not a champion.** Nothing here enters [`crate::fitness::score`], and a
//! discovery's replay is a [`sim::AgentAction`] tick list under one seed, exactly like a TAS
//! route — a claim that can be replayed bit-for-bit, not a policy.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sim::{AgentAction, Level};

use crate::fitness::{EpisodeResult, EvalConfig};
use crate::genome::Genome;
use crate::technique::TECHNIQUE_RULES;

/// The codex file's format tag, so a file from a future build is refused by name.
pub const CODEX_FORMAT: &str = "evo-discovery-codex";
/// The codex file's version. Bumped when an entry's meaning changes, not when a new name is
/// added: an old codex is still a true journal, and a new build can append to it.
pub const CODEX_VERSION: u32 = 1;
/// The directory a codex lives in, beside the champions.
pub const CODEX_DIR: &str = "codex";

/// What kind of find an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoveryKind {
    /// A named technique the episode performed (a stomp chain, a spring launch, a burn).
    Technique,
    /// A winning route that skipped part of the intended route.
    Shortcut,
}

/// One named find, with everything a codex or a splash needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Discovery {
    /// Stable display name.
    pub name: String,
    /// One line of plain explanation, in the game's voice.
    pub line: String,
    pub kind: DiscoveryKind,
    /// The identity a codex de-duplicates on: a find is one entry per level, however many runs
    /// rediscover it. It deliberately excludes the generation, the genome and the clock.
    pub signature: String,
    pub level_name: String,
    pub level_hash: String,
    /// Generation that first produced it.
    pub generation: u32,
    pub genome_id: u32,
    /// The winning episode's clock, when the find came from one.
    pub frames: u32,
    pub seed: u64,
    /// Percentage of the intended route skipped, for a shortcut.
    #[serde(default)]
    pub skip_percent: Option<f32>,
    /// One action per physics tick, so the find can be replayed. Empty when the caller asked
    /// only for detection (`detect(..., false)`).
    #[serde(default)]
    pub replay: Vec<AgentAction>,
}

/// The shortcut names, as a small ladder over how much of the intended route was skipped.
///
/// Thresholds rather than a formula: the number a codex entry carries is the exact percentage,
/// and the name is there to make three finds distinguishable in conversation. `Grand Theft
/// Flag` is deliberately a little ridiculous — it is the name for "the route the level was built
/// around is not the route this policy takes".
pub fn shortcut_name(percent: f32) -> &'static str {
    if percent >= 50.0 {
        "Grand Theft Flag"
    } else if percent >= 25.0 {
        "Shortcut"
    } else {
        "Detour"
    }
}

/// **Detect what a winning episode found.** Pure except for the optional replay capture, which
/// re-runs the episode once at tick resolution.
///
/// `replays` is off for a per-generation caller — a record can be re-detected at any time, and
/// paying a full tick-by-tick episode for every candidate would make a watched run slower than
/// the search it is showing. The codex writer turns it on, because a codex entry without its
/// replay is a claim without its evidence.
pub fn detect(
    level: &Level,
    genome: &Genome,
    result: &EpisodeResult,
    eval: &EvalConfig,
    generation: u32,
    replays: bool,
) -> Vec<Discovery> {
    let won = result.won();
    // **Evidence is checked before it is attached.** The recorded tick actions are replayed once
    // more, and a route that does not itself win leaves the entry without a replay rather than
    // with a replay that contradicts the claim it is meant to support. The extra episode is paid
    // only by a caller that asked for evidence, and only for a winning route.
    let replay: Vec<AgentAction> = if replays && won {
        let actions = crate::tas::route_from_genome(level, genome, eval, result.seed).actions;
        if !actions.is_empty() && crate::tas::play(level, result.seed, &actions).won() {
            actions
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let base = |name: &str, line: String, kind: DiscoveryKind, signature: String| Discovery {
        name: name.to_string(),
        line,
        kind,
        signature,
        level_name: level.name.clone(),
        level_hash: level.content_hash_string(),
        generation,
        genome_id: genome.id,
        frames: result.frames,
        seed: result.seed,
        skip_percent: None,
        replay: replay.clone(),
    };

    let mut found = Vec::new();
    // **Only a winning route is a discovery.** The trainer's firsts fire on losses on purpose —
    // the technique archive exists to keep a trick alive until a later mutation weaponises it —
    // but a codex entry is a claim about a route to the flag. Without this rule a body that fell
    // off the level edge for seven seconds would be presented as *Long Haul* and *Terminal
    // Velocity*, which is a true reading of the counters and a false story about the level.
    if won {
        // Technique firsts, read through the same table the trainer's archive uses.
        for rule in TECHNIQUE_RULES {
            let reading = (rule.read)(result);
            if reading >= rule.threshold {
                found.push(base(
                    rule.name,
                    rule.line.to_string(),
                    DiscoveryKind::Technique,
                    format!("technique:{}", rule.name),
                ));
            }
        }
    }

    // A route that skips the intended one. Only a winning route can: a corpse that fell past a
    // hop did not find a shortcut, it failed the level.
    if won {
        let report = crate::shortcut::trace(level, genome, eval, result.seed);
        if report.skip_fraction > 0.0 {
            let percent = report.skip_percent();
            let mut entry = base(
                shortcut_name(percent),
                format!(
                    "crossed {} of the {} intended hops — {:.0}% of the route skipped",
                    report.visited.len().saturating_sub(1),
                    report.intended.len().saturating_sub(1),
                    percent
                ),
                DiscoveryKind::Shortcut,
                format!("shortcut:{}", shortcut_name(percent)),
            );
            entry.skip_percent = Some(percent);
            found.push(entry);
        }
    }
    found
}

/// A per-level journal of finds.
///
/// Ordered by first discovery, because a codex is read as a history. `record` keeps the first
/// entry for a signature — the codex answers "this level has produced a Corpse Bridge", not
/// "the fastest Corpse Bridge on record" — and silently upgrades the stored replay when a later
/// find is quicker, so the attached evidence is the best one available.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Codex {
    pub format: String,
    pub version: u32,
    pub level_name: String,
    pub level_hash: String,
    #[serde(default)]
    pub entries: Vec<Discovery>,
}

impl Codex {
    pub fn empty(level: &Level) -> Codex {
        Codex {
            format: CODEX_FORMAT.to_string(),
            version: CODEX_VERSION,
            level_name: level.name.clone(),
            level_hash: level.content_hash_string(),
            entries: Vec::new(),
        }
    }

    /// Load a codex for `level`, or start an empty one when the file does not exist.
    ///
    /// A file that belongs to a different level is refused rather than merged: a codex is a
    /// claim about one level's geometry, and carrying an entry across a content change would
    /// make the journal lie about what was found where.
    pub fn load(level: &Level, path: &Path) -> Result<Codex, String> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Codex::empty(level))
            }
            Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
        };
        let codex: Codex =
            serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        if codex.format != CODEX_FORMAT {
            return Err(format!(
                "{}: format {:?} is not {CODEX_FORMAT}",
                path.display(),
                codex.format
            ));
        }
        if codex.version > CODEX_VERSION {
            return Err(format!(
                "{}: codex version {} is newer than this build's {CODEX_VERSION}",
                path.display(),
                codex.version
            ));
        }
        if !codex.level_hash.is_empty() && codex.level_hash != level.content_hash_string() {
            return Err(format!(
                "{}: this codex belongs to {} ({}) and not to {} ({})",
                path.display(),
                codex.level_name,
                codex.level_hash,
                level.name,
                level.content_hash_string()
            ));
        }
        Ok(codex)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot encode the codex: {e}"))?;
        std::fs::write(path, raw).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// Record one find. Returns whether it was **new** to the codex.
    pub fn record(&mut self, discovery: Discovery) -> bool {
        if let Some(held) = self
            .entries
            .iter_mut()
            .find(|e| e.signature == discovery.signature)
        {
            if discovery.frames < held.frames
                && (discovery.skip_percent.unwrap_or(0.0) >= held.skip_percent.unwrap_or(0.0))
            {
                *held = discovery;
            }
            return false;
        }
        self.entries.push(discovery);
        true
    }

    /// Record a whole detection pass, returning the names that were new.
    pub fn record_all(&mut self, found: Vec<Discovery>) -> Vec<String> {
        let mut new = Vec::new();
        for discovery in found {
            let name = discovery.name.clone();
            if self.record(discovery) {
                new.push(name);
            }
        }
        new
    }

    pub fn entries(&self) -> &[Discovery] {
        &self.entries
    }

    /// Does the journal already hold this find? Used by a caller that wants to know whether a
    /// replay is worth capturing **before** paying for the tick-resolution re-run.
    pub fn contains(&self, signature: &str) -> bool {
        self.entries.iter().any(|e| e.signature == signature)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Where a level's codex lives under a champion output directory: `<dir>/codex/<stem>.json`.
    pub fn path_for(dir: &Path, stem: &str) -> PathBuf {
        dir.join(CODEX_DIR).join(format!("{stem}.json"))
    }

    /// Where a level's codex lives when the caller already names the codex directory itself.
    ///
    /// Distinct from [`Codex::path_for`] on purpose: `--out champions` means `champions/codex/`,
    /// while `--codex-dir champions/codex` must not become `champions/codex/codex/`.
    pub fn path_in(dir: &Path, stem: &str) -> PathBuf {
        dir.join(format!("{stem}.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::Level;

    fn level() -> Level {
        Level::from_json_str(include_str!("../../../levels/l1-flat-run.json"))
            .expect("the shipped level parses")
    }

    fn discovery(name: &str, frames: u32, skip: Option<f32>) -> Discovery {
        Discovery {
            name: name.to_string(),
            line: format!("{name} line"),
            kind: if skip.is_some() {
                DiscoveryKind::Shortcut
            } else {
                DiscoveryKind::Technique
            },
            signature: format!("technique:{name}"),
            level_name: "test".to_string(),
            level_hash: "fnv1a64:test".to_string(),
            generation: 1,
            genome_id: 7,
            frames,
            seed: 3,
            skip_percent: skip,
            replay: Vec::new(),
        }
    }

    #[test]
    fn a_shortcut_name_climbs_the_three_rungs() {
        assert_eq!(shortcut_name(0.5), "Detour");
        assert_eq!(shortcut_name(25.0), "Shortcut");
        assert_eq!(shortcut_name(50.0), "Grand Theft Flag");
    }

    #[test]
    fn a_technique_fires_at_its_threshold_and_nowhere_else() {
        let level = level();
        let genome = crate::genome::Genome::zeros(1, crate::genome::default_shape());
        let mut result = crate::fitness::run_episode(&level, &genome, &EvalConfig::default(), 1);
        // A synthetic reading, because the point is the table and not the episode: a genome
        // that has not learned the level cannot be asked to stomp three walkers on demand.
        result.max_stomp_chain = 2;
        let none = detect(&level, &genome, &result, &EvalConfig::default(), 1, false);
        assert!(
            none.iter().all(|d| d.name != "Corpse Bridge"),
            "two stomps is below the three-stomp threshold"
        );
        // The same reading on a *lost* episode is not a discovery: the counters are a true
        // reading and the story would be false.
        result.max_stomp_chain = 3;
        result.outcome = sim::Outcome::Stuck;
        assert!(
            detect(&level, &genome, &result, &EvalConfig::default(), 1, false).is_empty(),
            "a route that did not finish is not a find"
        );
        result.outcome = sim::Outcome::Won;
        let found = detect(&level, &genome, &result, &EvalConfig::default(), 1, false);
        let entry = found
            .iter()
            .find(|d| d.name == "Corpse Bridge")
            .expect("three stomps is the threshold");
        assert_eq!(entry.kind, DiscoveryKind::Technique);
        assert_eq!(entry.signature, "technique:Corpse Bridge");
        assert_eq!(entry.genome_id, genome.id);
    }

    #[test]
    fn the_codex_keeps_one_entry_per_find_and_upgrades_its_replay() {
        let level = level();
        let mut codex = Codex::empty(&level);
        assert!(codex.record(discovery("Corpse Bridge", 400, None)));
        assert_eq!(codex.len(), 1);
        assert!(
            !codex.record(discovery("Corpse Bridge", 300, None)),
            "a rediscovery is not a new entry"
        );
        assert_eq!(
            codex.entries()[0].frames,
            300,
            "the evidence attached is the quicker run"
        );
        assert_eq!(
            codex.record_all(vec![
                discovery("Corpse Bridge", 200, None),
                discovery("Fusesurf", 250, None),
            ]),
            vec!["Fusesurf".to_string()]
        );
        assert_eq!(codex.len(), 2);
    }

    #[test]
    fn a_codex_file_round_trips_and_refuses_another_level() {
        let level = level();
        let dir = std::env::temp_dir().join(format!(
            "evo-codex-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = Codex::path_for(&dir, "l1-flat-run");
        let mut codex = Codex::empty(&level);
        codex.record(discovery("Springboard Skip", 120, None));
        codex.save(&path).expect("the codex writes");
        let back = Codex::load(&level, &path).expect("the codex reads");
        assert_eq!(back, codex);

        let other = Level::from_json_str(include_str!("../../../levels/l2-first-gap.json"))
            .expect("the second shipped level parses");
        let err = Codex::load(&other, &path).expect_err("a different level is refused");
        assert!(err.contains("belongs to"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
