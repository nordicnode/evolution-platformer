//! **Winning route variants for the awe phase.**
//!
//! A reliable champion is kept as the solver of record. This module searches nearby tick routes,
//! accepts only full wins, and keeps routes that trade speed for meaningful divergence from the
//! source. It is deliberately post-solve: exploration cannot damage the policy that already beat
//! the level.

use serde::{Deserialize, Serialize};
use sim::physics::AgentAction;
use sim::rng::{next_f32, rng_from_seed, Rng};
use sim::Level;

use crate::tas::{play, TasRoute};

pub const VARIANTS_FORMAT: &str = "evo-platformer-winning-variants";
pub const VARIANTS_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VariantConfig {
    pub candidates: usize,
    pub mutations: usize,
    pub seed: u64,
    pub keep: usize,
}

impl Default for VariantConfig {
    fn default() -> Self {
        VariantConfig {
            candidates: 2_000,
            mutations: 3,
            seed: 0xA0E0_5EED,
            keep: 8,
        }
    }
}

impl VariantConfig {
    pub fn sanitized(mut self) -> Self {
        self.candidates = self.candidates.clamp(1, 100_000);
        self.mutations = self.mutations.clamp(1, 16);
        self.keep = self.keep.clamp(1, 64);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariantObjectives {
    pub frames: u32,
    pub divergence: u32,
    pub technique: u32,
    pub shortcut_percent: f32,
    pub jumps: u32,
    pub action_changes: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WinningVariant {
    pub frames: u32,
    /// Number of tick actions changed from the source route.
    pub divergence: u32,
    pub seed: u64,
    pub actions: Vec<AgentAction>,
    pub final_state_hash: u64,
    pub objectives: VariantObjectives,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariantReport {
    pub format: String,
    pub version: u32,
    pub level_name: String,
    pub level_hash: String,
    pub source_frames: u32,
    pub source_actions: usize,
    pub source_valid: bool,
    pub variants: Vec<WinningVariant>,
}

impl VariantReport {
    pub fn to_json_pretty(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot encode winning variants: {e}"))
    }
}

/// A small deterministic Pareto archive. The reliable champion remains outside this archive; each
/// entry is a full winning route that is non-dominated across speed, divergence, mechanics,
/// shortcutting, and clean-input style.
#[derive(Clone, Debug, PartialEq)]
pub struct ParetoArchive {
    capacity: usize,
    entries: Vec<WinningVariant>,
}

impl ParetoArchive {
    pub fn new(capacity: usize) -> Self {
        ParetoArchive {
            capacity: capacity.clamp(1, 64),
            entries: Vec::new(),
        }
    }

    pub fn entries(&self) -> &[WinningVariant] {
        &self.entries
    }

    pub fn insert(&mut self, candidate: WinningVariant) {
        if self
            .entries
            .iter()
            .any(|held| held.actions == candidate.actions)
            || self.entries.iter().any(|held| dominates(held, &candidate))
        {
            return;
        }
        self.entries.retain(|held| !dominates(&candidate, held));
        self.entries.push(candidate);
        if self.entries.len() > self.capacity {
            self.entries.sort_by(archive_order);
            self.entries.truncate(self.capacity);
        }
    }

    pub fn into_entries(mut self) -> Vec<WinningVariant> {
        self.entries.sort_by(archive_order);
        self.entries
    }
}

/// Search a winning route's local neighbourhood and retain a Pareto archive of fast, different,
/// mechanical, shortcut, and stylistic wins. The source route itself is not included in `variants`.
pub fn explore(level: &Level, source: &TasRoute, cfg: VariantConfig) -> VariantReport {
    let cfg = cfg.sanitized();
    let mut rng = rng_from_seed(cfg.seed);
    let source_play = play(level, source.seed, &source.actions);
    let source_valid = source_play.won();
    let mut archive = ParetoArchive::new(cfg.keep);
    if !source_valid {
        return VariantReport {
            format: VARIANTS_FORMAT.into(),
            version: VARIANTS_VERSION,
            level_name: level.name.clone(),
            level_hash: level.content_hash_string(),
            source_frames: source.frames,
            source_actions: source.actions.len(),
            source_valid,
            variants: Vec::new(),
        };
    }
    for _ in 0..cfg.candidates {
        let mut actions = source.actions.clone();
        if actions.is_empty() {
            break;
        }
        let changes = 1 + (next_f32(&mut rng) * cfg.mutations as f32) as usize;
        for _ in 0..changes {
            let index = ((next_f32(&mut rng) * actions.len() as f32) as usize)
                .min(actions.len().saturating_sub(1));
            actions[index] = action(&mut rng);
        }
        let played = play(level, source.seed, &actions);
        if !played.won() {
            continue;
        }
        let divergence = actions
            .iter()
            .zip(source.actions.iter())
            .filter(|(a, b)| a != b)
            .count() as u32;
        if divergence == 0 {
            continue;
        }
        let total_hops = sim::spawn_route_stand(level)
            .and_then(|stand| level.routes().hops_to_flag(stand))
            .unwrap_or(0);
        let shortcut_percent = if total_hops == 0 {
            0.0
        } else {
            100.0 * (1.0 - (played.route_hops_closed.min(total_hops) as f32 / total_hops as f32))
        };
        let technique = played.stomps
            + played.wall_jumps
            + played.springs
            + played.max_stomp_chain.saturating_sub(1)
            + played.max_air_ticks / 8;
        let objectives = VariantObjectives {
            frames: played.frames,
            divergence,
            technique,
            shortcut_percent,
            jumps: played.jumps,
            action_changes: played.action_changes,
        };
        let candidate = WinningVariant {
            frames: played.frames,
            divergence,
            seed: source.seed,
            actions,
            final_state_hash: played.final_state_hash,
            objectives,
            note: format!(
                "winning variant: {} frames, {} changed actions, {:.0}% shortcut, technique {}",
                played.frames, divergence, shortcut_percent, technique
            ),
        };
        archive.insert(candidate);
    }
    VariantReport {
        format: VARIANTS_FORMAT.into(),
        version: VARIANTS_VERSION,
        level_name: level.name.clone(),
        level_hash: level.content_hash_string(),
        source_frames: source.frames,
        source_actions: source.actions.len(),
        source_valid,
        variants: archive.into_entries(),
    }
}

fn dominates(a: &WinningVariant, b: &WinningVariant) -> bool {
    let x = &a.objectives;
    let y = &b.objectives;
    let no_worse = x.frames <= y.frames
        && x.divergence >= y.divergence
        && x.technique >= y.technique
        && x.shortcut_percent >= y.shortcut_percent
        && x.jumps <= y.jumps
        && x.action_changes <= y.action_changes;
    let strictly_better = x.frames < y.frames
        || x.divergence > y.divergence
        || x.technique > y.technique
        || x.shortcut_percent > y.shortcut_percent
        || x.jumps < y.jumps
        || x.action_changes < y.action_changes;
    no_worse && strictly_better
}

fn archive_order(a: &WinningVariant, b: &WinningVariant) -> std::cmp::Ordering {
    a.objectives
        .frames
        .cmp(&b.objectives.frames)
        .then(b.objectives.divergence.cmp(&a.objectives.divergence))
        .then(b.objectives.technique.cmp(&a.objectives.technique))
        .then(
            b.objectives
                .shortcut_percent
                .total_cmp(&a.objectives.shortcut_percent),
        )
        .then(a.objectives.jumps.cmp(&b.objectives.jumps))
        .then(
            a.objectives
                .action_changes
                .cmp(&b.objectives.action_changes),
        )
        .then_with(|| action_key(&a.actions).cmp(&action_key(&b.actions)))
}

fn action_key(actions: &[AgentAction]) -> Vec<usize> {
    actions.iter().map(|action| action.index()).collect()
}

fn action(rng: &mut Rng) -> AgentAction {
    let index =
        ((next_f32(rng) * AgentAction::ALL.len() as f32) as usize).min(AgentAction::ALL.len() - 1);
    AgentAction::ALL[index]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tas::route_from_actions;

    #[test]
    fn variants_are_deterministic_and_only_keep_wins() {
        let level = Level::from_json_str(include_str!("../../../levels/l1-flat-run.json"))
            .expect("the flat level parses");
        let source = route_from_actions(&level, 3, &[AgentAction::Run; 400], "source".into());
        let cfg = VariantConfig {
            candidates: 100,
            mutations: 2,
            keep: 4,
            seed: 7,
        };
        let a = explore(&level, &source, cfg);
        let b = explore(&level, &source, cfg);
        assert_eq!(a, b);
        assert!(a.variants.iter().all(|v| v.divergence > 0));
        assert!(a
            .variants
            .iter()
            .all(|v| play(&level, v.seed, &v.actions).won()));
    }
}
