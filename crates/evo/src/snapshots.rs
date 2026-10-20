//! **Snapshot archive (WP8): start a fraction of exploration from a frontier state.**
//!
//! Fitness measures a whole episode from spawn, so a level's late crossings are practised only by
//! genomes that already solved everything before them. The stall detour is the one-cell version of
//! the answer — return to a promising state and explore from there — and this module generalises
//! it to a keyed archive:
//!
//! * **the key** is `(route stand, clock bucket, mover-phase bucket)`. The same stand at a
//!   different patrol or shutter phase is a different problem, and mixing them would archive a
//!   state that can never be met again;
//! * **admission** keeps the state with the most remaining budget in a cell, so a later, fresher
//!   frontier replaces an earlier one;
//! * **sampling** takes the least-visited cell, ties broken by key, so exploration is spread over
//!   the frontier rather than stuck on whichever state was found first.
//!
//! A snapshot is a whole [`Sim`] clone. That is the existence proof the plan cites: the planner
//! already forks episodes this way, and the memory cost of thousands of cells is megabytes.
//!
//! **A snapshot-started episode never crowns a record.** Its result is scored from the snapshot,
//! so it is not replayable from spawn; the trainer explores with it and validates any candidate
//! through the ordinary spawn-started protocol before it can become the champion.

use std::collections::BTreeMap;

use sim::Sim;

/// A coarse description of the world a snapshot was taken in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SnapshotKey {
    /// Route stand the body was on (or had last stood on).
    pub stand: Option<usize>,
    /// World clock in quarter-second buckets, so a patrol's phase is visible.
    pub phase: u8,
    /// First moving platform's cycle phase, in quarter buckets.
    pub mover: u8,
}

impl SnapshotKey {
    pub fn of(sim: &Sim) -> SnapshotKey {
        let frames = sim.stats().frames;
        let mover = sim
            .platforms()
            .first()
            .map(|m| (m.x * 4.0).floor() as i32)
            .unwrap_or(0)
            .rem_euclid(4) as u8;
        SnapshotKey {
            stand: sim.route_stand(),
            phase: ((frames / 15) % 4) as u8,
            mover,
        }
    }
}

/// One archived state.
#[derive(Clone, Debug)]
pub struct SnapshotCell {
    pub sim: Sim,
    /// How many exploration episodes have been started here.
    pub visits: u32,
    /// Generation that admitted it.
    pub generation: u32,
}

/// The archive itself.
#[derive(Clone, Debug, Default)]
pub struct SnapshotArchive {
    cells: BTreeMap<SnapshotKey, SnapshotCell>,
    admitted: u64,
}

impl SnapshotArchive {
    pub fn new() -> SnapshotArchive {
        SnapshotArchive::default()
    }

    /// Offer a state. Returns true when it was admitted (new cell, or fresher than the incumbent).
    ///
    /// “Fresher” is **more remaining budget**, measured against the state's own frame cap, which
    /// is what makes the archive a frontier rather than a museum: a late state with two seconds
    /// left is worse to explore from than an early one with ten, even though it is further along.
    pub fn observe(&mut self, sim: &Sim, generation: u32) -> bool {
        let key = SnapshotKey::of(sim);
        let remaining = sim.max_frames().saturating_sub(sim.stats().frames);
        let better = match self.cells.get(&key) {
            None => true,
            Some(held) => {
                let held_remaining = held
                    .sim
                    .max_frames()
                    .saturating_sub(held.sim.stats().frames);
                remaining > held_remaining
            }
        };
        if better {
            self.cells.insert(
                key,
                SnapshotCell {
                    sim: sim.clone(),
                    visits: 0,
                    generation,
                },
            );
            self.admitted += 1;
        }
        better
    }

    /// Take the least-visited cell as a fresh episode start, and count the visit.
    ///
    /// The clone is returned rather than a reference so the caller can hand it to
    /// [`sim::Sim::restart_from_snapshot`] while still mutating the archive on the next probe.
    pub fn sample(&mut self) -> Option<(SnapshotKey, Sim)> {
        let key = self
            .cells
            .iter()
            .min_by(|(ka, a), (kb, b)| a.visits.cmp(&b.visits).then(ka.cmp(kb)))
            .map(|(k, _)| *k)?;
        let cell = self.cells.get_mut(&key)?;
        cell.visits = cell.visits.saturating_add(1);
        Some((key, cell.sim.clone()))
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Every cell's key and visit count, in key order: a report, not the archive.
    pub fn cells(&self) -> Vec<(SnapshotKey, u32, u32)> {
        self.cells
            .iter()
            .map(|(k, c)| (*k, c.visits, c.generation))
            .collect()
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

    #[test]
    fn a_snapshot_is_admitted_and_can_be_sampled() {
        let level = level();
        let sim = Sim::new(&level, 1);
        let mut archive = SnapshotArchive::new();
        assert!(archive.observe(&sim, 0));
        assert_eq!(archive.len(), 1);
        let (_, start) = archive.sample().expect("a cell");
        assert_eq!(start.stats().frames, sim.stats().frames);
        assert_eq!(archive.cells()[0].1, 1, "sampling counts the visit");
    }

    #[test]
    fn sampling_spreads_over_the_least_visited_cell() {
        let level = level();
        let mut archive = SnapshotArchive::new();
        let a = Sim::new(&level, 1);
        let mut b = Sim::new(&level, 1);
        for _ in 0..20 {
            b.step(sim::physics::AgentAction::Run);
        }
        archive.observe(&a, 0);
        archive.observe(&b, 0);
        assert!(
            archive.len() >= 2,
            "different world clocks are different cells"
        );
        let first = archive.sample().expect("a cell");
        let second = archive.sample().expect("a cell");
        assert_ne!(first.0, second.0, "the sampler moves on after a visit");
    }

    #[test]
    fn a_snapshot_restarts_with_fresh_episode_progress_and_the_same_clock() {
        let level = level();
        let mut sim = Sim::new(&level, 1);
        sim.step(sim::physics::AgentAction::Run);
        let clock = sim.stats().frames;
        let restarted = sim.restart_from_snapshot();
        assert_eq!(
            restarted.stats().frames,
            clock,
            "the world clock is preserved"
        );
        assert_eq!(restarted.stats().progress_pixels, 0.0);
        assert_eq!(
            restarted.outcome(),
            sim::Outcome::Running,
            "a snapshot starts a new episode"
        );
    }
}
