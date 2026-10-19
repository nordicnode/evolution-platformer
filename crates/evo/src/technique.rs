//! **Technique archive and firsts (WP13).**
//!
//! The main archive keys on progress, checkpoints and finish time. A genome that stomps five
//! walkers to cross a pit but finishes slower is simply a worse genome there, and dies. That is
//! the right rule for the champion and the wrong rule for a *discovery*: real exploit lineages
//! start as useless tricks, and something has to keep them alive until a later mutation
//! weaponises them.
//!
//! This module is that "something", in two halves:
//!
//! * a **technique archive**, keyed by a coarse signature over stomp chain, wall jumps, spring
//!   launches and airtime. Its incumbent is the genome that did *the most* of that technique,
//!   never the one with the best progress — the archive is not a second championship;
//! * a **firsts detector** that reports the first run to reach a named threshold (a three-stomp
//!   chain, a spring launch, a three-tile crumble burn, a long airtime). A first is a
//!   presentation event and a retention hint, not a fitness term: the crowned score never sees
//!   any of it.
//!
//! The counters themselves live in [`sim::EpisodeStats`] and are copied onto
//! [`crate::fitness::EpisodeResult`] for the trainer and the window. They are reporting only, so
//! this module changes no version.

use std::collections::BTreeMap;

use crate::fitness::EpisodeResult;
use crate::genome::Genome;

/// Coarse bins over the technique counters. Ten stomps, ten wall jumps and ten springs per bin is
/// deliberately coarse: the archive is for "which kind of strange" a genome is, not for a second
/// scoreboard.
pub const TECHNIQUE_BINS: u32 = 10;

/// The maximum a technique counter may contribute to a signature.
pub const TECHNIQUE_CAP: u32 = 5;

/// A genome's technique signature: four capped bins, lexicographically ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TechniqueSignature {
    pub stomps: u32,
    pub wall_jumps: u32,
    pub springs: u32,
    pub airtime: u32,
}

impl TechniqueSignature {
    pub fn of(result: &EpisodeResult) -> TechniqueSignature {
        TechniqueSignature {
            stomps: (result.max_stomp_chain / TECHNIQUE_BINS).min(TECHNIQUE_CAP),
            wall_jumps: (result.wall_jumps / TECHNIQUE_BINS).min(TECHNIQUE_CAP),
            springs: (result.springs / TECHNIQUE_BINS).min(TECHNIQUE_CAP),
            airtime: (result.max_air_ticks / 60).min(TECHNIQUE_CAP),
        }
    }

    /// The archive's "most of this technique" order: total count first, then the individual
    /// counters, then the earlier genome id so a tie is stable.
    fn weight(self) -> u32 {
        self.stomps + self.wall_jumps * 2 + self.springs * 3 + self.airtime
    }
}

/// The archive's incumbent: the genome that did the most of a signature, and the result that
/// proves it.
#[derive(Clone, Debug, PartialEq)]
pub struct TechniqueEntry {
    pub genome_id: u32,
    pub signature: TechniqueSignature,
    pub max_stomp_chain: u32,
    pub wall_jumps: u32,
    pub springs: u32,
    pub crumble_burned: u32,
    pub max_air_ticks: u32,
    pub generation: u32,
}

/// A genome that is worth breeding from because of *how* it moved, not how far it got.
#[derive(Clone, Debug, PartialEq)]
pub struct TechniqueParent {
    pub genome: Genome,
    pub entry: TechniqueEntry,
}

/// A named first. Stable names are the point: a discovery a player cannot talk about is not a
/// discovery the presentation can sell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechniqueFirst {
    pub name: &'static str,
    pub generation: u32,
    pub genome_id: u32,
    /// One line of plain explanation, for a splash or a codex entry.
    pub line: String,
}

/// The two halves together.
#[derive(Clone, Debug, Default)]
pub struct TechniqueArchive {
    cells: BTreeMap<TechniqueSignature, (Genome, TechniqueEntry)>,
    firsts: Vec<TechniqueFirst>,
    seen: TechniqueTotals,
}

/// **One named technique, and the reading that fires it.**
///
/// The table is public because it is also the discovery codex's (`crate::discovery`): the
/// trainer's firsts and the window's presentation must call a trick by the same name and
/// explain it with the same sentence, and two tables would eventually disagree.
#[derive(Clone, Copy)]
pub struct TechniqueRule {
    pub name: &'static str,
    /// One line of plain explanation, for a splash or a codex entry.
    pub line: &'static str,
    pub threshold: f32,
    /// The reading this rule is measured on.
    pub read: fn(&EpisodeResult) -> f32,
}

fn stomp_chain(r: &EpisodeResult) -> f32 {
    r.max_stomp_chain as f32
}
fn wall_jumps(r: &EpisodeResult) -> f32 {
    r.wall_jumps as f32
}
fn springs(r: &EpisodeResult) -> f32 {
    r.springs as f32
}
fn crumble_burned(r: &EpisodeResult) -> f32 {
    r.crumble_burned as f32
}
fn air_ticks(r: &EpisodeResult) -> f32 {
    r.max_air_ticks as f32
}
fn peak_speed(r: &EpisodeResult) -> f32 {
    r.peak_speed
}

/// The fixed table, in the order a run reports its firsts. [`TechniqueTotals::readings`] is
/// index-aligned with this array; a new rule must be added in both places.
pub const TECHNIQUE_RULES: [TechniqueRule; 6] = [
    TechniqueRule {
        name: "Corpse Bridge",
        line: "chained three stomps without touching the ground",
        threshold: 3.0,
        read: stomp_chain,
    },
    TechniqueRule {
        name: "Shaft Requiem",
        line: "left a wall with a wall jump",
        threshold: 1.0,
        read: wall_jumps,
    },
    TechniqueRule {
        name: "Springboard Skip",
        line: "was thrown by a spring pad",
        threshold: 1.0,
        read: springs,
    },
    TechniqueRule {
        name: "Fusesurf",
        line: "rode three brittle tiles until they burned",
        threshold: 3.0,
        read: crumble_burned,
    },
    TechniqueRule {
        name: "Long Haul",
        line: "stayed airborne for two seconds",
        threshold: 120.0,
        read: air_ticks,
    },
    TechniqueRule {
        name: "Terminal Velocity",
        line: "outran a sprint at full extension",
        // `sprint_speed` is 0.15 tiles a tick. A body that is only sprinting is at 0.15, so
        // this fires on a reading the sprint itself cannot produce — a spring launch or a
        // stomp bounce carries the body past its own legs.
        threshold: 0.16,
        read: peak_speed,
    },
];

/// The largest technique reading a run has seen. This is what "first" is measured against.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TechniqueTotals {
    pub max_stomp_chain: u32,
    pub wall_jumps: u32,
    pub springs: u32,
    pub crumble_burned: u32,
    pub max_air_ticks: u32,
    pub peak_speed: f32,
}

impl TechniqueTotals {
    /// The maxima this run has seen, index-aligned with [`TECHNIQUE_RULES`].
    pub fn readings(&self) -> [f32; 6] {
        [
            self.max_stomp_chain as f32,
            self.wall_jumps as f32,
            self.springs as f32,
            self.crumble_burned as f32,
            self.max_air_ticks as f32,
            self.peak_speed,
        ]
    }

    fn absorb_max(&mut self, result: &EpisodeResult) {
        self.max_stomp_chain = self.max_stomp_chain.max(result.max_stomp_chain);
        self.wall_jumps = self.wall_jumps.max(result.wall_jumps);
        self.springs = self.springs.max(result.springs);
        self.crumble_burned = self.crumble_burned.max(result.crumble_burned);
        self.max_air_ticks = self.max_air_ticks.max(result.max_air_ticks);
        self.peak_speed = self.peak_speed.max(result.peak_speed);
    }

    fn absorb(&mut self, result: &EpisodeResult) -> Vec<TechniqueFirst> {
        let before = self.readings();
        let mut new = Vec::new();
        for (i, rule) in TECHNIQUE_RULES.iter().enumerate() {
            let now = (rule.read)(result);
            if before[i] < rule.threshold && now >= rule.threshold {
                new.push(TechniqueFirst {
                    name: rule.name,
                    generation: 0,
                    genome_id: result.genome_id,
                    line: rule.line.to_string(),
                });
            }
        }
        self.absorb_max(result);
        new
    }
}

impl TechniqueArchive {
    pub fn new() -> TechniqueArchive {
        TechniqueArchive::default()
    }

    /// Offer one scored genome. Returns the firsts this result created, in the module's fixed
    /// order, with the generation stamped on them.
    pub fn observe(
        &mut self,
        generation: u32,
        genome: &Genome,
        result: &EpisodeResult,
    ) -> Vec<TechniqueFirst> {
        let signature = TechniqueSignature::of(result);
        let entry = TechniqueEntry {
            genome_id: genome.id,
            signature,
            max_stomp_chain: result.max_stomp_chain,
            wall_jumps: result.wall_jumps,
            springs: result.springs,
            crumble_burned: result.crumble_burned,
            max_air_ticks: result.max_air_ticks,
            generation,
        };
        let weight = signature.weight();
        let replace = match self.cells.get(&signature) {
            None => true,
            Some((_, held)) => {
                let held_weight = held.signature.weight();
                weight > held_weight
                    || (weight == held_weight
                        && (result.max_stomp_chain, result.wall_jumps, result.springs)
                            > (
                                held.max_stomp_chain,
                                held.wall_jumps,
                                held.springs,
                            ))
            }
        };
        if replace {
            self.cells
                .insert(signature, (genome.clone(), entry));
        }
        let mut firsts = self.seen.absorb(result);
        for first in &mut firsts {
            first.generation = generation;
        }
        self.firsts.extend(firsts.iter().cloned());
        firsts
    }

    /// The archive's incumbents, in signature order. Deterministic, so a report is too.
    pub fn entries(&self) -> Vec<TechniqueEntry> {
        self.cells.values().map(|(_, e)| e.clone()).collect()
    }

    pub fn parents(&self) -> Vec<TechniqueParent> {
        self.cells
            .values()
            .map(|(g, e)| TechniqueParent {
                genome: g.clone(),
                entry: e.clone(),
            })
            .collect()
    }

    pub fn firsts(&self) -> &[TechniqueFirst] {
        &self.firsts
    }

    pub fn seen(&self) -> TechniqueTotals {
        self.seen
    }

    pub fn occupied(&self) -> usize {
        self.cells.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{default_shape, Genome};

    fn result(id: u32, chain: u32, wall: u32, springs: u32, air: u32) -> EpisodeResult {
        EpisodeResult {
            genome_id: id,
            fitness: 0.0,
            mean_fitness: 0.0,
            worst_trial: 0.0,
            outcome: sim::Outcome::Running,
            frames: air,
            progress_pixels: 0.0,
            best_x: 0.0,
            stomps: chain,
            max_stomp_chain: chain,
            wall_jumps: wall,
            springs,
            crumble_burned: 0,
            max_air_ticks: air,
            max_air_distance: 0.0,
            peak_speed: 0.0,
            jumps: chain,
            checkpoints: 0,
            route_hops_closed: 0,
            best_stand: None,
            suite_wins: 0,
            all_trials_won: false,
            death_pos: None,
            death_cause: None,
            cut_early: false,
            seed: 1,
            trajectory_hash: 0,
        }
    }

    #[test]
    fn the_archive_keeps_the_most_technical_genome_in_a_cell() {
        let mut archive = TechniqueArchive::new();
        let shape = default_shape();
        let quiet = Genome::zeros(1, shape.clone());
        let louder = Genome::zeros(2, shape);
        archive.observe(3, &quiet, &result(1, 3, 0, 0, 0));
        archive.observe(4, &louder, &result(2, 8, 0, 0, 0));
        assert_eq!(archive.occupied(), 1, "both are in the same coarse cell");
        assert_eq!(
            archive.entries()[0].genome_id, 2,
            "the genome that did more of the technique must be the incumbent"
        );
    }

    #[test]
    fn firsts_fire_once_per_threshold_and_carry_stable_names() {
        let mut archive = TechniqueArchive::new();
        let shape = default_shape();
        let g = Genome::zeros(7, shape);
        let first = archive.observe(1, &g, &result(7, 3, 0, 0, 0));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "Corpse Bridge");
        assert_eq!(first[0].generation, 1);
        let again = archive.observe(2, &g, &result(7, 4, 0, 0, 0));
        assert!(again.is_empty(), "a threshold fires once per run");
        let spring = archive.observe(3, &g, &result(7, 4, 0, 1, 0));
        assert_eq!(spring[0].name, "Springboard Skip");
    }
}
