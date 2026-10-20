//! **Learning telemetry (Phase 0).** What a generation did, in the terms that separate *learning*
//! from *random guesswork*: how many solved, how quickly, which leg of the route the policy was
//! on when it ended, and at which frame and decision.
//!
//! The trainer already reports a fitness distribution, and that is exactly the number that
//! cannot answer the question a person watching a stalled run asks. A maximum that moves while
//! every agent dies at the same ledge is not progress; a maximum that is flat while the
//! population's deaths move one leg deeper is. So this module reports **where on the level** the
//! generation got, not only how much score it accumulated:
//!
//! * `solved` / `wins` — the conservative and the generous census, already computed by the
//!   trainer, carried here so one record is the whole generation;
//! * `best_frames` / `median_frames` — the clock among this generation's winners, in frames
//!   (a level that has never been won has no clock, not a zero);
//! * `best_end_leg`, `best_end_frame`, `best_end_decision` — where the generation's **best**
//!   genome stopped, so a near-miss is a place rather than a score;
//! * `ended_legs` / `died_legs` — a census over the whole population, per leg. This is the
//!   reading that makes a wall visible: forty agents ending on leg 2 is a different run from
//!   forty agents spread over legs 0-6;
//! * `reached_legs` — how many agents completed each leg, the complement that says whether the
//!   population is *moving through* the level or piling up against it;
//! * `first_actions` / `first_action_entropy` — the spread of the population's very first
//!   decisions at a fixed spawn probe. A converged population answers with one action; one that
//!   is still exploring spreads over the eleven. It is a cheap proxy for policy diversity, not a
//!   claim about the whole trajectory, and it is named that way.
//!
//! **A leg is a route crossing, not a checkpoint.** `route_hops_closed` is the sim's own geodesic
//! count ([`sim::EpisodeStats::route_hops_closed`]): one leg is one canonical crossing from the
//! spawn stand toward the flag. On a level with no route (`legs == 0`) every episode is in leg 0
//! and the census says so rather than inventing a distance. This is the unit the plan's later
//! phases need (a route compiler's legs are the same crossings), so the telemetry and the
//! curriculum will not disagree about what "leg 3" means.
//!
//! **Reporting only.** Nothing here is scored, nothing here is hashed, and nothing here draws
//! from the run's RNG. The only cost is one forward pass per genome at spawn (200 passes at the
//! default population, well under a millisecond), which is why it can be paid on every
//! generation of every run rather than behind a flag.

use serde::Serialize;
use sim::physics::AgentAction;
use sim::{Level, ObservationScratch, Outcome};

use crate::fitness::EpisodeResult;
use crate::genome::{ActScratch, Genome};

/// One generation's learning readings.
///
/// `generation` is the trainer's own number, so a JSONL file written by a run can be joined to
/// the run's chart without a second index.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Telemetry {
    pub generation: u32,
    /// Episodes in the generation's census (the population, not the trial count).
    pub pop: usize,
    /// Genomes that won **every** episode they were judged on.
    pub solved: usize,
    /// Episodes that touched the flag.
    pub wins: usize,
    /// Route crossings from the spawn stand to the flag. `0` when the level has no route.
    pub legs: u32,
    /// The quickest winning episode this generation, in frames.
    pub best_frames: Option<u32>,
    /// The median winning episode this generation, in frames.
    pub median_frames: Option<u32>,
    /// The leg the generation's best genome was on when it ended (`route_hops_closed`, capped at
    /// [`Telemetry::legs`]).
    pub best_end_leg: u32,
    /// Whether the best genome's episode ended in a death rather than a timeout or a stall.
    pub best_died: bool,
    /// The frame the best genome's episode ended on, when it did not win.
    pub best_end_frame: Option<u32>,
    /// The decision block that frame belongs to (`frame / action_repeat`).
    pub best_end_decision: Option<u32>,
    /// Episodes that did not win, counted by the leg they ended on. Index = leg.
    pub ended_legs: Vec<u32>,
    /// The subset of [`Telemetry::ended_legs`] the sim called a death. Index = leg.
    pub died_legs: Vec<u32>,
    /// Episodes that completed each leg. Index = leg; `reached_legs[L]` counts episodes with
    /// `route_hops_closed > L`.
    pub reached_legs: Vec<u32>,
    /// The action each genome's **first** decision took at a fixed spawn probe, indexed by
    /// [`AgentAction::index`].
    pub first_actions: [u32; AgentAction::COUNT],
    /// Shannon entropy of [`Telemetry::first_actions`], in bits. Zero means every genome answered
    /// the probe with the same action.
    pub first_action_entropy: f32,
}

impl Default for Telemetry {
    fn default() -> Self {
        Telemetry {
            generation: 0,
            pop: 0,
            solved: 0,
            wins: 0,
            legs: 0,
            best_frames: None,
            median_frames: None,
            best_end_leg: 0,
            best_died: false,
            best_end_frame: None,
            best_end_decision: None,
            ended_legs: Vec::new(),
            died_legs: Vec::new(),
            reached_legs: Vec::new(),
            first_actions: [0; AgentAction::COUNT],
            first_action_entropy: 0.0,
        }
    }
}

impl Telemetry {
    /// Read one generation. `winner` indexes [`Telemetry::best_end_leg`]'s subject in `results`;
    /// `genomes` is the population that produced those results, and is only read by the spawn
    /// probe.
    ///
    /// `action_repeat` is the cadence the episode ran at, so a decision index is the frame the
    /// policy actually chose on rather than a tick it was coasting through.
    pub fn measure(
        generation: u32,
        level: &Level,
        results: &[EpisodeResult],
        winner: usize,
        genomes: &[Genome],
        action_repeat: u32,
    ) -> Telemetry {
        let mut out = Telemetry {
            generation,
            pop: results.len(),
            ..Telemetry::default()
        };
        if results.is_empty() {
            return out;
        }
        out.legs = level_legs(level);
        out.ended_legs = vec![0; out.legs as usize];
        out.died_legs = vec![0; out.legs as usize];
        out.reached_legs = vec![0; out.legs as usize];

        let repeat = action_repeat.max(1);
        let mut winning_frames: Vec<u32> = Vec::new();
        for r in results {
            // Counted for winners too: a win completed every leg, and a `reached_legs` table
            // that excluded them would say the last leg was never crossed on a solved level.
            for (index, count) in out.reached_legs.iter_mut().enumerate() {
                if r.route_hops_closed > index as u32 {
                    *count += 1;
                }
            }
            if r.won() {
                out.wins += 1;
                winning_frames.push(r.frames);
                continue;
            }
            let leg = r.route_hops_closed.min(out.legs);
            if let Some(count) = out.ended_legs.get_mut(leg as usize) {
                *count += 1;
                if r.outcome == Outcome::Died {
                    out.died_legs[leg as usize] += 1;
                }
            }
        }
        out.solved = results.iter().filter(|r| r.solved()).count();
        winning_frames.sort_unstable();
        if !winning_frames.is_empty() {
            out.best_frames = winning_frames.first().copied();
            out.median_frames = Some(winning_frames[winning_frames.len() / 2]);
        }

        if let Some(best) = results.get(winner) {
            out.best_end_leg = best.route_hops_closed.min(out.legs);
            if !best.won() {
                out.best_died = best.outcome == Outcome::Died;
                out.best_end_frame = Some(best.frames);
                out.best_end_decision = Some(best.frames / repeat);
            }
        }

        out.first_actions = spawn_actions(level, genomes);
        out.first_action_entropy = entropy(&out.first_actions);
        out
    }

    /// `true` when the level has a route to count legs on.
    pub fn has_route(&self) -> bool {
        self.legs > 0
    }

    /// The clock among this generation's winners, `best / median`, or an em dash when it won
    /// nothing. One formatter so the terminal and the panel cannot disagree about what "no
    /// clock" looks like.
    pub fn clock_line(&self) -> String {
        match (self.best_frames, self.median_frames) {
            (Some(best), Some(median)) => {
                format!("{} / {}", sim::format_time(best), sim::format_time(median))
            }
            _ => "—".to_string(),
        }
    }

    /// Where the generation's best genome stopped, in one clause: `won in 4.20 s`,
    /// `died in leg 3 at frame 412 (decision 103)`, or the timeout/stall variant of it.
    pub fn best_end_line(&self) -> String {
        if let Some(frames) = self.best_frames.filter(|_| self.best_end_frame.is_none()) {
            // The best genome won: there is no ending place to report, only the clock.
            return format!("won in {}", sim::format_time(frames));
        }
        let Some(frame) = self.best_end_frame else {
            return "no episode".to_string();
        };
        let verb = if self.best_died { "died" } else { "ended" };
        let mut line = format!(
            "{verb} in leg {} at frame {frame} (decision {})",
            self.best_end_leg,
            self.best_end_decision.unwrap_or(frame)
        );
        if self.legs > 0 {
            line.push_str(&format!(" of {}", self.legs));
        }
        line
    }

    /// The per-leg census as `(leg, ended, died, through)` rows, one per leg.
    ///
    /// `through` is the number of episodes that completed the leg, so a row reads
    /// "3 ended here, 41 were still alive" only when those add up over the generation — the
    /// last leg's `through` is the winners on a route that ends at the flag.
    pub fn leg_rows(&self) -> Vec<(u32, u32, u32, u32)> {
        (0..self.legs)
            .map(|leg| {
                (
                    leg,
                    self.ended_legs.get(leg as usize).copied().unwrap_or(0),
                    self.died_legs.get(leg as usize).copied().unwrap_or(0),
                    self.reached_legs.get(leg as usize).copied().unwrap_or(0),
                )
            })
            .collect()
    }

    /// The spawn probe's spread as a fraction of what a uniform population would have, in
    /// `0..=1`. The raw bits depend on the action count; this is the number a panel can say
    /// "3 % of the action table" about without a footnote.
    pub fn spread(&self) -> f32 {
        let max = (AgentAction::COUNT as f32).log2();
        if max <= 0.0 {
            0.0
        } else {
            self.first_action_entropy / max
        }
    }
}

/// Route crossings from the spawn stand to the flag, or `0` for a level with no route.
fn level_legs(level: &Level) -> u32 {
    sim::spawn_route_stand(level)
        .and_then(|stand| level.routes().hops_to_flag(stand))
        .unwrap_or(0)
}

/// The action every genome takes at the spawn observation, with a fresh recurrent state.
///
/// The probe is one fixed frame (`seed 0`) rather than each genome's own episode: the point is
/// to compare the population's *dispositions* at the same state, and a per-episode probe would
/// put them at different states on a level whose patrol phase is drawn from the seed.
fn spawn_actions(level: &Level, genomes: &[Genome]) -> [u32; AgentAction::COUNT] {
    let mut counts = [0u32; AgentAction::COUNT];
    if genomes.is_empty() {
        return counts;
    }
    let sim = sim::Sim::new(level, 0);
    let mut obs = ObservationScratch::new();
    let observation = sim.observation_into(&mut obs);
    let mut scratch = ActScratch::new();
    for genome in genomes {
        scratch.reset_context();
        let action = genome.act(&observation, &mut scratch);
        counts[action.index()] += 1;
    }
    counts
}

/// Shannon entropy of an action histogram, in bits.
fn entropy(counts: &[u32; AgentAction::COUNT]) -> f32 {
    let total: u32 = counts.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let total = total as f32;
    -counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let p = *c as f32 / total;
            p * p.log2()
        })
        .sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::genome::default_shape;
    use sim::rng::rng_from_seed;

    fn result(outcome: Outcome, frames: u32, leg: u32) -> EpisodeResult {
        EpisodeResult {
            genome_id: 0,
            fitness: 0.0,
            mean_fitness: 0.0,
            worst_trial: 0.0,
            outcome,
            frames,
            progress_pixels: 0.0,
            best_x: 0.0,
            stomps: 0,
            max_stomp_chain: 0,
            wall_jumps: 0,
            springs: 0,
            crumble_burned: 0,
            max_air_ticks: 0,
            max_air_distance: 0.0,
            peak_speed: 0.0,
            jumps: 0,
            checkpoints: 0,
            route_hops_closed: leg,
            best_stand: Some(leg as usize),
            suite_wins: 0,
            all_trials_won: outcome == Outcome::Won,
            death_pos: None,
            death_cause: None,
            cut_early: false,
            seed: 1,
            trajectory_hash: 0,
        }
    }

    #[test]
    fn the_census_names_the_leg_and_the_decision_a_near_miss_ended_on() {
        // Two gaps: two route legs, so a near-miss in leg 1 is a place with a next leg after it.
        let level = fixture::gap_runway("telemetry-gap", 48, 46, &[(20, 3), (32, 3)]);
        let mut rng = rng_from_seed(7);
        let genomes: Vec<Genome> = (0..3)
            .map(|id| Genome::random(id, default_shape(), &mut rng))
            .collect();
        let results = vec![
            result(Outcome::Won, 300, 1),
            result(Outcome::Died, 412, 1),
            result(Outcome::Died, 90, 0),
        ];

        let t = Telemetry::measure(4, &level, &results, 1, &genomes, 4);
        assert_eq!(t.generation, 4);
        assert_eq!(t.pop, 3);
        assert_eq!(t.wins, 1);
        assert_eq!(t.solved, 1);
        // One winner: best and median are the same clock, not zero.
        assert_eq!(t.best_frames, Some(300));
        assert_eq!(t.median_frames, Some(300));
        assert_eq!(t.best_end_leg, 1);
        assert!(t.best_died);
        assert_eq!(t.best_end_frame, Some(412));
        assert_eq!(t.best_end_decision, Some(103));
        assert_eq!(t.ended_legs[0], 1);
        assert_eq!(t.ended_legs[1], 1);
        assert_eq!(t.died_legs[1], 1);
        assert_eq!(
            t.reached_legs[0], 2,
            "the winner and the near-miss crossed leg 0"
        );
        assert!(t.best_end_line().contains("leg 1"));
        assert!(t.best_end_line().contains("frame 412"));
        assert!(t.clock_line().contains("5.00s"));
    }

    #[test]
    fn a_generation_that_won_nothing_has_no_clock_and_says_so() {
        let level = fixture::flat_runway("telemetry-flat", 20, 18);
        let results = vec![result(Outcome::Stuck, 500, 0)];
        let t = Telemetry::measure(0, &level, &results, 0, &[], 4);
        assert_eq!(t.best_frames, None);
        assert_eq!(t.median_frames, None);
        assert_eq!(t.clock_line(), "—");
        assert_eq!(t.legs, 0);
        assert_eq!(t.best_end_frame, Some(500));
        assert_eq!(t.best_end_decision, Some(125));
        assert!(!t.best_died, "a stall is not a death");
        assert!(t.best_end_line().starts_with("ended"));
    }

    #[test]
    fn identical_genomes_have_no_first_action_spread_and_distinct_ones_can() {
        let level = fixture::flat_runway("telemetry-spread", 30, 28);
        let mut rng = rng_from_seed(11);
        let genome = Genome::random(1, default_shape(), &mut rng);
        let clones = vec![genome.clone(), genome.clone(), genome.clone()];
        let same = Telemetry::measure(
            0,
            &level,
            &vec![result(Outcome::Running, 1, 0); 3],
            0,
            &clones,
            4,
        );
        assert_eq!(same.first_action_entropy, 0.0);
        assert_eq!(same.first_actions.iter().sum::<u32>(), 3);
        assert_eq!(same.spread(), 0.0);

        let distinct: Vec<Genome> = (0..64)
            .map(|i| Genome::random(i, default_shape(), &mut rng))
            .collect();
        let mixed = Telemetry::measure(
            0,
            &level,
            &vec![result(Outcome::Running, 1, 0); 64],
            0,
            &distinct,
            4,
        );
        assert_eq!(mixed.first_actions.iter().sum::<u32>(), 64);
        assert!(
            mixed.first_action_entropy > 0.0,
            "64 random genomes do not all answer the spawn probe the same way"
        );
        assert!(mixed.spread() <= 1.0);
    }
}
