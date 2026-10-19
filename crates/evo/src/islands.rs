//! **Islands (WP7): the same diversity budget spent on independent populations.**
//!
//! The project measured two diversity mechanisms as losses on its plateau — archive children and
//! novelty ranking — and the plan's reading of that result is that diversity does not pay *inside
//! one panmictic population*, where a niche genome competes for parents against the main line and
//! destroys it. Islands are the other arrangement: several populations with their own settings,
//! each ranked only against itself, exchanging one genome every few generations.
//!
//! The spread here is deliberately small and made of knobs this project already owns:
//!
//! | island | setting |
//! |---|---|
//! | 0 | the base configuration |
//! | 1 | `mutation_sigma × 1.5` |
//! | 2 | `mutation_sigma × 0.7` |
//! | 3 | `novelty_weight` 0.5 |
//! | 4 | `rank_pessimism` 0.5 |
//! | 5 | `archive_children` 2 |
//!
//! Every island keeps the base **shape**: migration copies weights, so two islands with different
//! input widths could not exchange anything. The plan's proposed feedback (0/16) split is therefore
//! not implemented — a shape split needs either per-shape migration rules or a projection, and
//! neither is a thing this module can invent honestly.
//!
//! The default is still one population. This is an experimental arm until the plan's equal-episode
//! gate runs; `Islands` makes that arm expressible without changing `Trainer`.

use crate::ga::{BestEver, GaConfig, GenerationReport, Trainer};
use crate::genome::Genome;
use sim::Level;

/// How to arrange the islands.
#[derive(Clone, Debug)]
pub struct IslandConfig {
    /// Number of populations. Clamped to 1..=12.
    pub islands: usize,
    /// Generations between ring migrations. 0 disables migration.
    pub migration_every: u32,
    /// The base configuration every island is derived from. Its `pop_size` is each island's size.
    pub base: GaConfig,
}

impl IslandConfig {
    pub fn new(base: GaConfig, islands: usize) -> IslandConfig {
        IslandConfig {
            islands: islands.clamp(1, 12),
            migration_every: 25,
            base,
        }
    }

    /// The configuration of island `index`, derived deterministically from the base.
    ///
    /// The master seed is offset per island, so two islands are not handed the same random
    /// population; that is the one setting that must differ or the arrangement proves nothing.
    pub fn island_config(&self, index: usize) -> GaConfig {
        let mut cfg = self.base.clone();
        cfg.master_seed ^= (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        match index % 6 {
            0 => {}
            1 => cfg.mutation_sigma = (cfg.mutation_sigma * 1.5).clamp(0.05, 0.9),
            2 => cfg.mutation_sigma = (cfg.mutation_sigma * 0.7).clamp(0.05, 0.9),
            3 => cfg.novelty_weight = 0.5,
            4 => cfg.rank_pessimism = 0.5,
            _ => cfg.archive_children = cfg.archive_children.max(2),
        }
        cfg
    }
}

/// One generation across every island.
#[derive(Clone, Debug)]
pub struct IslandStep {
    pub generation: u32,
    pub reports: Vec<GenerationReport>,
    /// The islands that received a migrant this generation, and where it came from.
    pub migrations: Vec<(usize, usize)>,
}

/// A ring of independent populations.
#[derive(Clone, Debug)]
pub struct Islands {
    pub config: IslandConfig,
    pub islands: Vec<Trainer>,
    generation: u32,
}

impl Islands {
    pub fn new(config: IslandConfig) -> Islands {
        let islands = (0..config.islands)
            .map(|i| Trainer::new(config.island_config(i)))
            .collect();
        Islands {
            config,
            islands,
            generation: 0,
        }
    }

    /// Resume every island from the same-shaped populations. Each island is a normal
    /// [`Trainer::resume`], so a checkpoint from a single-population run is a legal starting point
    /// for all of them.
    pub fn resume(
        config: IslandConfig,
        populations: Vec<Vec<Genome>>,
        generation: u32,
    ) -> Result<Islands, String> {
        if populations.len() != config.islands {
            return Err(format!(
                "{} populations were handed to a ring of {} islands",
                populations.len(),
                config.islands
            ));
        }
        let islands = populations
            .into_iter()
            .enumerate()
            .map(|(i, genomes)| {
                Trainer::resume(config.island_config(i), genomes, generation)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Islands {
            config,
            islands,
            generation,
        })
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Step every island once, then migrate around the ring if this generation is a migration one.
    ///
    /// Migration is **native-best only, never a replacement for an elite**: [`Trainer::inject`]
    /// replaces the receiving island's weakest slot, so the island's own record and elites are
    /// untouched. The source genome is selected by its own generation score, which is the only
    /// comparable number across islands with different ranking knobs.
    pub fn step(&mut self, level: &Level) -> IslandStep {
        let reports: Vec<GenerationReport> = self
            .islands
            .iter_mut()
            .map(|island| island.step(level))
            .collect();
        let mut migrations = Vec::new();
        self.generation += 1;
        if self.config.migration_every > 0
            && self.generation % self.config.migration_every == 0
            && self.islands.len() > 1
        {
            let migrants: Vec<Genome> = self.islands.iter().map(source_genome).collect();
            for (from, migrant) in migrants.into_iter().enumerate() {
                let to = (from + 1) % self.islands.len();
                if self.islands[to].inject(migrant).is_ok() {
                    migrations.push((from, to));
                }
            }
        }
        IslandStep {
            generation: self.generation,
            reports,
            migrations,
        }
    }

    /// The best record any island holds, with the island it belongs to.
    pub fn best(&self) -> Option<(usize, &BestEver)> {
        self.islands
            .iter()
            .enumerate()
            .filter_map(|(i, island)| island.best_ever().map(|b| (i, b)))
            .max_by(|(_, a), (_, b)| a.result.mean_fitness.total_cmp(&b.result.mean_fitness))
    }

    pub fn solved(&self) -> bool {
        self.islands
            .iter()
            .any(|island| island.best_ever().is_some_and(|b| b.result.solved()))
    }
}

/// The genome an island contributes to the ring: its record when it has one, otherwise its
/// highest-scoring member. A record is a better migrant than a lucky generation, because it has
/// been validated under this project's protocol.
fn source_genome(island: &Trainer) -> Genome {
    if let Some(best) = island.best_ever() {
        return best.genome.clone();
    }
    island
        .genomes
        .iter()
        .max_by(|a, b| a.fitness.total_cmp(&b.fitness))
        .cloned()
        .unwrap_or_else(|| island.genomes[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GaConfig {
        GaConfig {
            pop_size: 8,
            ..GaConfig::default()
        }
    }

    #[test]
    fn every_island_gets_a_different_seed_and_the_base_keeps_its_own() {
        let base = cfg();
        let plan = IslandConfig::new(base.clone(), 4);
        assert_eq!(plan.island_config(0).master_seed, base.master_seed);
        let seeds: Vec<u64> = (0..4).map(|i| plan.island_config(i).master_seed).collect();
        assert_eq!(
            seeds.iter().collect::<std::collections::HashSet<_>>().len(),
            4,
            "islands must not share a random population"
        );
    }

    #[test]
    fn the_spread_uses_the_knobs_it_claims_to() {
        let plan = IslandConfig::new(cfg(), 6);
        assert!(plan.island_config(1).mutation_sigma > plan.island_config(0).mutation_sigma);
        assert!(plan.island_config(2).mutation_sigma < plan.island_config(0).mutation_sigma);
        assert!(plan.island_config(3).novelty_weight > 0.0);
        assert_eq!(plan.island_config(4).rank_pessimism, 0.5);
        assert!(plan.island_config(5).archive_children >= 2);
    }

    #[test]
    fn migration_replaces_the_weakest_slot_and_never_duplicates_an_id() {
        let mut trainer = Trainer::new(cfg());
        trainer.genomes[3].fitness = -100.0;
        let weak_id = trainer.genomes[3].id;
        let mut migrant = trainer.genomes[0].clone();
        migrant.id = weak_id;
        trainer.inject(migrant).expect("migrant accepted");
        assert_eq!(trainer.genomes[3].id, trainer.next_id() - 1);
    }
}
