//! Watching the run's best agent play the level, at real time, with the population
//! ghosted beside it.
//!
//! The evolution view is the *search*: a generation at a time, at whatever speed makes a
//! generation readable. This is its answer. One episode, run once, at the speed the game
//! actually runs at — 60 ticks a second, the first rung of the speed ladder — with the
//! champion solid and the population ghosted on the same clock, the same level and the
//! same physics.
//!
//! The two halves of that come from different places, and it is worth being exact about
//! them because a viewer will watch them diverge. The champion is the run's **best-ever**
//! agent: the record, which is older than the newest generation as soon as the search
//! moves past its best idea ([`Star`] carries the generation it came from). The field
//! beside it is the **last generation the run scored** — the population as the run left
//! it, which is what the window was handed and what [`Population::generation`] names. So
//! a stopped run shows its best idea racing the population that has since moved on, which
//! is a comparison worth watching rather than an accident.
//!
//! Two things make the episodes themselves cheap, and both are already in the packet. The
//! run's best-ever genome arrives with the `EpisodeResult` it earned the record on, whose
//! `seed` is half of the episode — a replay with any other seed would be a different run
//! of the same network, which is exactly what a replay must not be. And [`Packet::agents`]
//! holds every agent of that last generation, each paired with the episode it ran, so the
//! ghosts are the population's own runs rather than fresh samples of it.
//!
//! Nothing here trains, and nothing here is a lookalike: the runners are the same
//! [`EpisodeRunner`] the trainer and the champion's own replay check use, so what the
//! viewer watches tick past is the episode the evaluator scored, and it ends on the
//! fitness that episode was recorded with.
//!
//! ```text
//!   the champion                          the ghosts
//!   (drawn last and complete: its         (bodies only, faint enough not to hide the
//!    walkers, its dust, its flag)          one that matters — and no walkers of their
//!                                          own, or a hundred marches would curtain it)
//! ```
//!
//! The field is drawn in **rank bands** rather than one flat tint ([`GHOST_COHORTS`]). One
//! colour says only "these are the others"; four say the thing a viewer actually wants to
//! know — whether the agents clearing an obstacle are the ones the run thinks are good, or
//! whether it is the stragglers getting through by luck. The bands are proportional, so
//! they fill whatever the run has, and the *rule* lives here ([`cohort_of`]) rather than in
//! the painter, because "which band is the best band" is a fact about the roster and not
//! about a colour.

use std::ops::Range;

use evo::{score, EpisodeRunner, EvalConfig};
use render::chart::format_fitness;
use sim::{Level, Outcome, Sim};

use crate::lab::{ticks_for, Agent, Packet};
use crate::view::AgentAnim;

/// Real time: one physics tick per 1/60 s, the rate the simulation itself runs at.
///
/// The replay opens here whatever speed the run was being watched at — "watch it run"
/// means the game's own clock — and the ladder still climbs from here for a viewer who
/// wants to skip ahead through a long level.
pub const REAL_TIME: u32 = 1;

/// How many ghosts one replay draws, at most.
///
/// A screen limit rather than a training limit: the default population of 200 is ghosted
/// in full. Above that the roster keeps the best [`GHOST_LIMIT`] agents, ranked by the
/// metric selection uses, and the panel says how many were left out — a thousand
/// overlapping bodies are a smear rather than a comparison, and the frame pays for every
/// one of them. The champion always runs, whatever the limit.
pub const GHOST_LIMIT: usize = 256;

/// How many rank bands the ghost field is drawn in.
///
/// Four, because the field has to stay readable *as a field*: a handful of bands answers
/// "are the good agents the ones getting through", while six blues would be a gradient
/// nobody can read off a moving sprite and twenty would be a smear with extra steps.
pub const GHOST_COHORTS: usize = 4;

/// Which rank band the agent at `position` of `roster` is drawn in, `0` being the best.
///
/// Proportional rather than fixed-size, so the bands fill whatever the run has: 200 agents
/// make four bands of fifty and 7 make 2-2-2-1. The property that matters — and the one the
/// tests hold it to — is that it never *improves* as the position gets worse: the bands are
/// non-decreasing in `position`, which is what makes "band 0 is the best of them" true of
/// the field rather than of a coincidence in the population's order.
///
/// A roster smaller than the number of bands simply wears the first bands and leaves the
/// rest empty ([`Replay::cohorts`] reports only the ones somebody is in).
pub fn cohort_of(position: usize, roster: usize, cohorts: usize) -> usize {
    if roster == 0 || cohorts <= 1 {
        return 0;
    }
    (position * cohorts / roster).min(cohorts - 1)
}

/// The agents one replay ghosts: the population's agents other than the champion, best
/// first.
///
/// A free function rather than a step inside [`Replay::with_limit`] because this is the
/// rule — *which* agents a viewer is shown beside the champion, and in what order — and a
/// rule buried in a constructor is one only an end-to-end test can reach. `agents` is
/// [`Packet::agents`] — the last generation the run scored — in population order; the
/// result is a prefix of it by `mean_fitness`, the metric selection itself ranks on, so
/// "the best 256 of them" is a statement about the run rather than about the order the
/// population happens to sit in.
///
/// The champion's own genome is left out: it is not *another* agent, and a second copy of
/// the same network running a different seed would be a ghost that means nothing.
pub(crate) fn ghost_roster(agents: &[Agent], star_id: u32, limit: usize) -> Vec<&Agent> {
    let mut pool: Vec<&Agent> = agents
        .iter()
        .filter(|agent| agent.genome.id != star_id)
        .collect();
    pool.sort_by(|a, b| b.result.mean_fitness.total_cmp(&a.result.mean_fitness));
    pool.truncate(limit);
    pool
}

/// The episode being replayed: the run's best-ever genome, and the episode that earned
/// the record.
///
/// Kept as the record itself rather than as a pointer into the packet, because it is what
/// the replay is *checked against*: the score at the end of the run has to land on
/// [`Star::fitness`], and a star that had lost its own numbers could not show anyone that
/// it had.
#[derive(Clone, Debug, PartialEq)]
pub struct Star {
    pub genome_id: u32,
    /// The generation that produced this genome. The run's record, not the newest
    /// generation the window holds: the two part company as soon as the search moves past
    /// its best idea, which is why this is printed beside the field's own generation
    /// rather than assumed to be it ([`Population`]).
    pub generation: u32,
    /// The seed the evaluator scored this episode under. The other half of the episode.
    pub seed: u64,
    /// The fitness the episode earned. The replay has to end on this number.
    pub fitness: f32,
    pub outcome: Outcome,
    pub frames: u32,
}

/// The generation the ghosts come from.
///
/// Kept as its own fact rather than read off the champion, because the two are usually
/// different numbers and a label that conflated them would be wrong in the common case:
/// the population is the run's newest scored generation, and the champion is its
/// best-ever. The viewer is told both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Population {
    pub generation: u32,
    /// Agents in the generation before the ghost limit, so the panel can say how many of
    /// them a capped roster left out.
    pub size: usize,
}

/// A pool of episodes animated together, on one clock.
///
/// `runners[0]` is the champion and the rest are the ghosts, so "the one being watched" is
/// not a flag kept somewhere that could disagree with what is drawn: it is the head of the
/// roster.
pub struct Replay {
    level: Level,
    eval: EvalConfig,
    level_name: String,
    star: Star,
    population: Population,
    runners: Vec<EpisodeRunner>,
    /// One agent's animation per runner, advanced with it.
    anims: Vec<AgentAnim>,
    /// Fractional ticks carried between frames, so a fast monitor does not run through
    /// more of the episode per second than a slow one.
    carry: f32,
    ticks: u32,
    /// Draw the ghosts.
    ///
    /// The toggle changes what is *drawn* and nothing else: every ghost is stepped whether
    /// or not it is visible, so turning them back on shows the same field the champion has
    /// been running through, not one that fell behind while nobody was looking.
    pub ghosts: bool,
    /// Hold the clock.
    pub paused: bool,
    /// Physics ticks per 1/60 s of wall clock. Opens at [`REAL_TIME`].
    pub speed: u32,
}

impl Replay {
    /// A replay of the run's best agent, with the last scored generation as ghosts.
    ///
    /// `None` when the run has not scored a generation yet: there is no best anything to
    /// watch, and an empty replay would be a mode with nothing in it.
    pub fn new(packet: &Packet, level: &Level, eval: &EvalConfig) -> Option<Replay> {
        Replay::with_limit(packet, level, eval, GHOST_LIMIT)
    }

    /// As [`Replay::new`], ghosting at most `limit` of the population.
    ///
    /// A parameter so the limit is a rule a test can reach at a size a test can afford,
    /// which is the only way a cap this generous is ever exercised.
    pub fn with_limit(
        packet: &Packet,
        level: &Level,
        eval: &EvalConfig,
        limit: usize,
    ) -> Option<Replay> {
        let best = packet.best.as_ref()?;
        let star = Star {
            genome_id: best.genome.id,
            generation: best.generation,
            seed: best.result.seed,
            fitness: best.result.fitness,
            outcome: best.result.outcome,
            frames: best.result.frames,
        };

        let roster = ghost_roster(&packet.agents, star.genome_id, limit);
        let mut runners: Vec<EpisodeRunner> = Vec::with_capacity(roster.len() + 1);
        runners.push(EpisodeRunner::new(
            level,
            best.genome.clone(),
            eval,
            star.seed,
        ));
        // Each ghost runs the episode its own result names, seed and all: the population's
        // runs, rather than fresh samples of the same genomes.
        for agent in roster {
            runners.push(EpisodeRunner::new(
                level,
                agent.genome.clone(),
                eval,
                agent.result.seed,
            ));
        }
        let anims = (0..runners.len()).map(|_| AgentAnim::new()).collect();

        Some(Replay {
            level: level.clone(),
            eval: *eval,
            level_name: packet.level_name.clone(),
            star,
            population: Population {
                generation: packet.generation,
                size: packet.agents.len(),
            },
            runners,
            anims,
            carry: 0.0,
            ticks: 0,
            ghosts: true,
            paused: false,
            speed: REAL_TIME,
        })
    }

    /// Run it again, from the first tick.
    ///
    /// The roster is rebuilt from the runners' own genomes and seeds, so running a replay
    /// twice is the same episodes twice: what the viewer sees never depends on when they
    /// pressed the key. A paused replay starts playing, because "again" is a request to
    /// watch it.
    pub fn restart(&mut self) {
        let runners: Vec<EpisodeRunner> = self
            .runners
            .iter()
            .map(|runner| {
                EpisodeRunner::new(
                    &self.level,
                    runner.genome().clone(),
                    &self.eval,
                    runner.seed(),
                )
            })
            .collect();
        self.runners = runners;
        for anim in &mut self.anims {
            *anim = AgentAnim::new();
        }
        self.carry = 0.0;
        self.ticks = 0;
        self.paused = false;
    }

    /// Advance by `dt` seconds of wall clock, at this replay's own speed.
    ///
    /// The same cadence the run's lanes use ([`ticks_for`]), so "real time" means the same
    /// thing here as it does there, and the dt clamp is shared for the same reason: a
    /// hitch must not skip the jump the viewer was watching.
    pub fn advance(&mut self, dt: f32) {
        self.carry += ticks_for(dt, self.speed);
        let whole = self.carry.floor();
        self.carry -= whole;
        let ticks = whole.max(0.0) as u32;
        for (runner, anim) in self.runners.iter_mut().zip(self.anims.iter_mut()) {
            let mut stepped = 0u32;
            for _ in 0..ticks {
                // A finished episode holds its last frame while the agents still running
                // carry on: that is the whole point of running them in parallel.
                if runner.is_finished() {
                    break;
                }
                runner.step();
                stepped += 1;
            }
            // Simulated time, not wall clock: a dead ghost's dust settles instead of
            // drifting for as long as the slowest agent in the field keeps going.
            if stepped > 0 {
                anim.advance(runner.sim(), stepped as f32 * sim::DT);
            }
        }
        self.ticks += ticks;
    }

    /// The episode being replayed.
    pub fn star(&self) -> &Star {
        &self.star
    }

    /// The generation the ghosts are drawn from: the run's last scored one.
    pub fn population(&self) -> Population {
        self.population
    }

    /// The level the episode is on: the run's, not the editor's — a replay is a claim
    /// about the terrain it was scored on.
    pub fn level(&self) -> &Level {
        &self.level
    }

    pub fn level_name(&self) -> &str {
        &self.level_name
    }

    /// The champion's live episode.
    pub fn star_sim(&self) -> &Sim {
        self.runners[0].sim()
    }

    pub fn star_anim(&self) -> &AgentAnim {
        &self.anims[0]
    }

    /// Agents in the roster, champion included.
    pub fn len(&self) -> usize {
        self.runners.len()
    }

    /// Which rank band ghost `i` is drawn in, `0` being the best of the field.
    ///
    /// `i` is a roster position rather than a runner index: ghost 0 is the best agent other
    /// than the champion, which is the only sense in which "the best of the ghosts" is well
    /// defined once the champion is out of the roster.
    pub fn ghost_cohort(&self, i: usize) -> usize {
        cohort_of(i, self.ghost_count(), GHOST_COHORTS)
    }

    /// The rank bands this field is drawn in, best band first, with the roster positions
    /// each band covers.
    ///
    /// Bands nobody is in are left out, so a field smaller than [`GHOST_COHORTS`] does not
    /// put a legend entry on screen for a colour no agent is wearing. The ranges are
    /// contiguous and cover the whole roster, which is what lets the panel describe the
    /// field it is looking at rather than one of a different size.
    pub fn cohorts(&self) -> Vec<(usize, Range<usize>)> {
        let roster = self.ghost_count();
        let mut bands: Vec<(usize, Range<usize>)> = Vec::new();
        for position in 0..roster {
            let band = self.ghost_cohort(position);
            match bands.last_mut() {
                Some((last, range)) if *last == band => range.end = position + 1,
                _ => bands.push((band, position..position + 1)),
            }
        }
        bands
    }

    /// How many agents are ghosted. Zero when the generation held no other agent.
    pub fn ghost_count(&self) -> usize {
        self.runners.len() - 1
    }

    /// One ghost's live episode. Index `0` is the first ghost, never the champion.
    pub fn ghost_sim(&self, i: usize) -> &Sim {
        self.runners[i + 1].sim()
    }

    pub fn ghost_anim(&self, i: usize) -> &AgentAnim {
        &self.anims[i + 1]
    }

    /// Seconds of the episode that have been simulated — not seconds on the wall clock:
    /// at 4x the difference is the whole point.
    pub fn clock(&self) -> f32 {
        self.ticks as f32 * sim::DT
    }

    /// The champion's score so far, by the same function the evaluator used.
    ///
    /// Live rather than predicted: while it runs it answers "how is it going", and when it
    /// stops it is the number to hold against [`Star::fitness`] — which is why the status
    /// line prints both.
    pub fn star_score(&self) -> f32 {
        let sim = self.star_sim();
        score(sim.stats(), sim.outcome())
    }

    /// The champion's episode has ended: the flag, a grave, or the sim's own caps.
    pub fn star_finished(&self) -> bool {
        self.star_sim().outcome().is_finished()
    }

    /// Ghosts still going.
    pub fn ghosts_running(&self) -> usize {
        (0..self.ghost_count())
            .filter(|i| !self.ghost_sim(*i).outcome().is_finished())
            .count()
    }

    /// Ghosts that reached the flag.
    pub fn ghosts_won(&self) -> usize {
        (0..self.ghost_count())
            .filter(|i| self.ghost_sim(*i).outcome() == Outcome::Won)
            .count()
    }

    /// Every agent has finished: the field is over and the clock has nothing left to do.
    pub fn all_finished(&self) -> bool {
        (0..self.len()).all(|i| self.runners[i].is_finished())
    }

    /// The status line: whose episode this is, how it is going, and what the ghosts are.
    ///
    /// One string built here rather than in the window for the reason `play::Session` has
    /// one: a number assembled where it is drawn cannot be tested, and this one carries the
    /// fact the replay exists for — the recorded fitness beside the score actually being
    /// produced.
    pub fn summary(&self) -> String {
        let outcome = self.star_sim().outcome();
        let headline = if outcome.is_finished() {
            format!(
                "{} in {:.1}s — score {} (recorded: {})",
                outcome.as_str(),
                self.clock(),
                format_fitness(self.star_score()),
                format_fitness(self.star.fitness)
            )
        } else {
            format!(
                "running — {:.1}s in, score {}",
                self.clock(),
                format_fitness(self.star_score())
            )
        };
        // The field's generation is printed, not assumed: the champion is the run's
        // best-ever and the ghosts are its newest generation, so "the generation this
        // came from" would be wrong on every run that has moved past its best idea.
        let ghosts = if self.ghosts {
            format!(
                "{} ghosts from generation {}: {} still going, {} at the flag",
                self.ghost_count(),
                self.population.generation + 1,
                self.ghosts_running(),
                self.ghosts_won()
            )
        } else {
            format!(
                "{} ghosts hidden (generation {})",
                self.ghost_count(),
                self.population.generation + 1
            )
        };
        format!(
            "replaying the best of generation {} (id {}) — {} — {} — {}x{}",
            self.star.generation + 1,
            self.star.genome_id,
            headline,
            ghosts,
            self.speed,
            if self.paused { " — paused" } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evo::GaConfig;

    /// A level this test owns, so its numbers are about the replay rather than about a
    /// document the author draws in.
    fn runway() -> Level {
        Level::starter("replay-probe", 40, 8)
    }

    fn eval() -> EvalConfig {
        GaConfig::default().eval_config()
    }

    fn replay_on(level: &Level, tag: &str) -> Replay {
        let packet = crate::lab::scored_packet(level, tag);
        Replay::new(&packet, level, &eval()).expect("a scored generation has a best agent")
    }

    /// The seed the evaluator used for one agent's representative episode.
    fn scored_seed(generation: u32, genome_id: u32) -> u64 {
        sim::episode_seed(GaConfig::default().master_seed, generation, genome_id, 0)
    }

    /// Run the champion to the end, with a cap so a bug cannot hang the suite.
    fn run_to_end(replay: &mut Replay, cap: u32) {
        let mut ticks = 0;
        while !replay.star_finished() && ticks < cap {
            replay.advance(1.0 / 60.0);
            ticks += 1;
        }
        assert!(ticks < cap, "the champion's episode did not end");
    }

    #[test]
    fn the_champion_replays_the_episode_the_record_was_made_on() {
        let level = runway();
        let mut replay = replay_on(&level, "fidelity");
        run_to_end(&mut replay, 4_000);

        // The three numbers that say this is *that* episode and not a fresh one: the
        // ending, the score, and the length of it. A different seed moves all three.
        let star = replay.star().clone();
        assert_eq!(
            replay.star_sim().outcome(),
            star.outcome,
            "the replay ended differently from the episode it was recorded on"
        );
        assert_eq!(
            replay.star_sim().stats().frames,
            star.frames,
            "the replay ran for a different number of frames"
        );
        assert!(
            (replay.star_score() - star.fitness).abs() < 1e-3,
            "the replay scored {} but the record says {}",
            replay.star_score(),
            star.fitness
        );
    }

    #[test]
    fn the_champion_runs_the_seed_it_was_scored_under() {
        let level = runway();
        let packet = crate::lab::scored_packet(&level, "star-seed");
        let best = packet.best.as_ref().expect("a best agent");
        assert_eq!(
            best.result.seed,
            scored_seed(best.generation, best.genome.id)
        );

        let replay = Replay::new(&packet, &level, &eval()).expect("a best agent");
        assert_eq!(replay.star().seed, best.result.seed);
        assert_eq!(replay.star().genome_id, best.genome.id);
    }

    #[test]
    fn the_ghosts_are_the_generations_other_agents_in_rank_order() {
        let level = runway();
        let packet = crate::lab::scored_packet(&level, "roster");
        let replay = Replay::new(&packet, &level, &eval()).expect("a best agent");
        let best = packet.best.as_ref().expect("a best agent");

        assert_eq!(packet.agents.len(), packet.pop_size);
        // The generation still holds the champion — elites are copied unchanged and keep
        // their ids — and it is not ghosted: it is the agent on screen, solid.
        assert!(
            packet.agents.iter().any(|a| a.genome.id == best.genome.id),
            "the champion is not in its own generation"
        );
        let roster = ghost_roster(&packet.agents, best.genome.id, GHOST_LIMIT);
        assert_eq!(roster.len(), packet.agents.len() - 1);
        assert_eq!(replay.ghost_count(), roster.len());
        assert!(
            roster.iter().all(|a| a.genome.id != best.genome.id),
            "the champion is ghosted beside itself"
        );

        // Rank order, by the metric selection itself uses.
        for pair in roster.windows(2) {
            assert!(
                pair[0].result.mean_fitness >= pair[1].result.mean_fitness,
                "the roster is not in rank order"
            );
        }
        // And every seed in it is the one the evaluator scored that agent under, which is
        // what makes a ghost a replay rather than a fresh episode.
        for agent in &roster {
            assert_eq!(
                agent.result.seed,
                scored_seed(packet.generation, agent.genome.id)
            );
            assert_ne!(agent.result.seed, best.result.seed);
        }
    }

    #[test]
    fn the_rank_bands_are_proportional_and_never_improve_as_rank_falls() {
        // The rule a viewer relies on: the band an agent is in never gets better as its
        // rank gets worse. Everything else about the field's colours rests on this.
        for roster in [0usize, 1, 2, 3, 7, 8, 199, 200, 255, 256] {
            let bands: Vec<usize> = (0..roster)
                .map(|position| cohort_of(position, roster, GHOST_COHORTS))
                .collect();
            assert!(
                bands.iter().all(|band| *band < GHOST_COHORTS),
                "a roster of {roster} left a band out of range: {bands:?}"
            );
            assert!(
                bands.windows(2).all(|pair| pair[0] <= pair[1]),
                "a roster of {roster} banded a worse agent better: {bands:?}"
            );
            if roster > 0 {
                assert_eq!(bands[0], 0, "the best of {roster} is not in the best band");
            }
            // A field at least as big as the palette wears every band, so the legend and
            // the canvas agree about what is on screen.
            if roster >= GHOST_COHORTS {
                assert_eq!(
                    *bands.last().unwrap(),
                    GHOST_COHORTS - 1,
                    "the last of {roster} does not wear the last band"
                );
            }
        }

        let sizes = |roster: usize| -> Vec<usize> {
            let mut out = vec![0usize; GHOST_COHORTS];
            for position in 0..roster {
                out[cohort_of(position, roster, GHOST_COHORTS)] += 1;
            }
            out
        };
        // Proportional, not fixed: the bands fill the run it has.
        assert_eq!(sizes(200), vec![50, 50, 50, 50]);
        assert_eq!(sizes(7), vec![2, 2, 2, 1]);
        assert_eq!(sizes(1), vec![1, 0, 0, 0]);
        assert_eq!(sizes(0), vec![0, 0, 0, 0]);
    }

    #[test]
    fn the_bands_cover_the_whole_field_in_rank_order() {
        let level = runway();
        let replay = replay_on(&level, "cohorts");
        let roster = replay.ghost_count();
        let bands = replay.cohorts();
        assert!(roster >= GHOST_COHORTS, "the fixture is too small to band");

        // Contiguous, from the first ghost to the last: nobody is left out of a band and
        // nobody is counted twice, which is what "the field by rank" has to mean.
        assert_eq!(bands.first().expect("a band").1.start, 0);
        assert_eq!(bands.last().expect("a band").1.end, roster);
        for pair in bands.windows(2) {
            assert_eq!(
                pair[0].1.end, pair[1].1.start,
                "the bands are not contiguous"
            );
            assert!(pair[0].0 < pair[1].0, "the bands are not in rank order");
        }
        assert_eq!(bands.iter().map(|(_, r)| r.len()).sum::<usize>(), roster);

        // And each ghost's band is the one its position falls in, so the legend describes
        // the field that is actually drawn.
        for (band, ranks) in &bands {
            for position in ranks.clone() {
                assert_eq!(replay.ghost_cohort(position), *band);
            }
        }
        // The band order is the roster's own: the roster is sorted best-first, so a lower
        // position is a better `mean_fitness`.
        let packet = crate::lab::scored_packet(&level, "cohorts");
        let best = packet.best.as_ref().expect("a best agent");
        let ranked = ghost_roster(&packet.agents, best.genome.id, GHOST_LIMIT);
        for pair in ranked.windows(2) {
            assert!(pair[0].result.mean_fitness >= pair[1].result.mean_fitness);
        }
    }

    #[test]
    fn every_ghost_replays_the_episode_it_was_scored_on() {
        let level = runway();
        let packet = crate::lab::scored_packet(&level, "ghost-fidelity");
        let best = packet.best.as_ref().expect("a best agent");
        let roster = ghost_roster(&packet.agents, best.genome.id, GHOST_LIMIT)
            .into_iter()
            .map(|agent| agent.result.clone())
            .collect::<Vec<_>>();
        let mut replay = Replay::new(&packet, &level, &eval()).expect("a best agent");

        // Run the whole field out, then hold every ghost's score against the episode it
        // was scored on: outcome, length and fitness. This is the claim the ghosts make —
        // "these are the population's own runs" — and it is the one that would break
        // silently if the roster ever lost its seeds.
        let mut ticks = 0;
        while !replay.all_finished() && ticks < 4_000 {
            replay.advance(1.0 / 60.0);
            ticks += 1;
        }
        assert!(ticks < 4_000, "the field never finished");
        for (i, recorded) in roster.iter().enumerate() {
            let sim = replay.ghost_sim(i);
            assert_eq!(
                sim.outcome(),
                recorded.outcome,
                "ghost {i} ended differently from the episode it was scored on"
            );
            assert_eq!(
                sim.stats().frames,
                recorded.frames,
                "ghost {i} ran a different length"
            );
            let got = score(sim.stats(), sim.outcome());
            assert!(
                (got - recorded.fitness).abs() < 1e-3,
                "ghost {i} scored {got} but its episode was recorded at {}",
                recorded.fitness
            );
        }
    }

    #[test]
    fn real_time_is_one_tick_per_sixtieth_of_a_second_whatever_the_frame_rate() {
        let level = runway();
        let mut replay = replay_on(&level, "clock");
        assert_eq!(replay.speed, REAL_TIME);

        // A second of wall clock at 60 Hz. `1/60` is a hair *above* a sixtieth in f32, so
        // every frame carries exactly one tick.
        for _ in 0..60 {
            replay.advance(1.0 / 60.0);
        }
        assert!(
            (replay.clock() - 1.0).abs() < 1e-4,
            "clock {}",
            replay.clock()
        );

        // 128 Hz — a rate whose frames are exact in binary — is the same second spread over
        // more than twice the frames, and lands on it exactly.
        let mut even = replay_on(&level, "clock-128");
        for _ in 0..128 {
            even.advance(1.0 / 128.0);
        }
        assert!((even.clock() - 1.0).abs() < 1e-6, "clock {}", even.clock());

        // 144 Hz is not a whole number of ticks a frame (60/144 = 5/12), so a second of it
        // can land a tick short: the carry keeps the fraction and the per-frame floor drops
        // the rest. "A second is a second to within a tick" is the guarantee, and it is the
        // one the run's own lanes give.
        let mut odd = replay_on(&level, "clock-144");
        for _ in 0..144 {
            odd.advance(1.0 / 144.0);
        }
        assert!(
            (odd.clock() - 1.0).abs() <= sim::DT,
            "144 Hz drifted by more than a tick: {}",
            odd.clock()
        );

        // The dt clamp is the run's own (0.25 s): a half-second hitch is worth a quarter of
        // a second, never half a level skipped past the viewer.
        let before = even.clock();
        even.advance(0.5);
        assert!(
            (even.clock() - (before + 0.25)).abs() < 1e-3,
            "a half-second frame was not clamped to a quarter: {}",
            even.clock()
        );

        // And the ladder is a multiplier on that clock, exactly as it is in the run.
        let mut faster = replay_on(&level, "clock-fast");
        faster.speed = 4;
        for _ in 0..60 {
            faster.advance(1.0 / 60.0);
        }
        assert!(
            (faster.clock() - 4.0).abs() < 1e-3,
            "clock {}",
            faster.clock()
        );
    }

    #[test]
    fn hiding_the_ghosts_does_not_change_the_episode_being_run() {
        let level = runway();
        let mut shown = replay_on(&level, "toggle-shown");
        let mut hidden = replay_on(&level, "toggle-hidden");
        hidden.ghosts = false;

        for _ in 0..90 {
            shown.advance(1.0 / 60.0);
            hidden.advance(1.0 / 60.0);
        }

        // The toggle is a drawing decision: the champion, the ghosts and the clock are all
        // untouched by it, so a viewer who hides the field and brings it back sees the run
        // they were watching rather than one that fell behind in the meantime.
        assert_eq!(shown.clock(), hidden.clock());
        assert_eq!(shown.star_score(), hidden.star_score());
        assert_eq!(
            *shown.star_sim().player(),
            *hidden.star_sim().player(),
            "the champion moved differently with the ghosts hidden"
        );
        for i in 0..shown.ghost_count() {
            assert_eq!(
                *shown.ghost_sim(i).player(),
                *hidden.ghost_sim(i).player(),
                "ghost {i} moved differently with the ghosts hidden"
            );
        }
    }

    #[test]
    fn running_a_replay_again_is_the_same_episodes_again() {
        let level = runway();
        let mut replay = replay_on(&level, "again");
        for _ in 0..40 {
            replay.advance(1.0 / 60.0);
        }
        let first: Vec<_> = (0..replay.len())
            .map(|i| {
                let sim = if i == 0 {
                    replay.star_sim()
                } else {
                    replay.ghost_sim(i - 1)
                };
                *sim.player()
            })
            .collect();

        replay.paused = true;
        replay.restart();
        assert_eq!(replay.clock(), 0.0, "the clock did not go back to zero");
        assert!(
            !replay.paused,
            "a replay that was asked for again stayed paused"
        );
        for _ in 0..40 {
            replay.advance(1.0 / 60.0);
        }
        for (i, was) in first.iter().enumerate() {
            let now = if i == 0 {
                *replay.star_sim().player()
            } else {
                *replay.ghost_sim(i - 1).player()
            };
            assert_eq!(now, *was, "agent {i} ran differently the second time");
        }
    }

    #[test]
    fn a_population_bigger_than_the_limit_is_ghosted_best_first() {
        let level = runway();
        let packet = crate::lab::scored_packet(&level, "limit");
        let best = packet.best.as_ref().expect("a best agent");

        let mut ranked: Vec<(f32, u32)> = packet
            .agents
            .iter()
            .filter(|a| a.genome.id != best.genome.id)
            .map(|a| (a.result.mean_fitness, a.genome.id))
            .collect();
        // Stable, as the roster's own sort is, so a tie breaks the same way in both.
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        assert!(
            ranked.len() > 3,
            "the fixture cannot show a cap it is not over"
        );

        // The three that made it are the three selection would have kept, in that order:
        // the roster is a prefix of the population by `mean_fitness`, not the first three
        // slots of it.
        let roster = ghost_roster(&packet.agents, best.genome.id, 3);
        let got: Vec<u32> = roster.iter().map(|a| a.genome.id).collect();
        assert_eq!(
            got,
            ranked.iter().take(3).map(|(_, id)| *id).collect::<Vec<_>>()
        );

        // And a replay built with the same cap ghosts exactly those agents.
        let replay = Replay::with_limit(&packet, &level, &eval(), 3).expect("a best agent");
        assert_eq!(replay.ghost_count(), 3);
    }

    #[test]
    fn the_status_line_names_the_episode_and_the_field() {
        let level = runway();
        let mut replay = replay_on(&level, "summary");

        let line = replay.summary();
        assert!(
            line.contains(&format!("generation {}", replay.star().generation + 1)),
            "{line}"
        );
        assert!(
            line.contains(&format!("id {}", replay.star().genome_id)),
            "{line}"
        );
        assert!(
            line.contains(&format!("{} ghosts", replay.ghost_count())),
            "{line}"
        );
        assert!(line.contains("1x"), "the clock is not named: {line}");
        assert!(
            line.contains(&format!(
                "ghosts from generation {}",
                replay.population().generation + 1
            )),
            "the field's generation is not on the line: {line}"
        );

        // Hidden ghosts say so, rather than leaving a count that reads as a field nobody
        // can see.
        replay.ghosts = false;
        assert!(replay.summary().contains("ghosts hidden"));

        // And once the champion's episode is over the line carries both numbers — the
        // score it just produced and the record it has to match — because those two being
        // equal is the whole claim a replay makes.
        run_to_end(&mut replay, 4_000);
        let done = replay.summary();
        assert!(done.contains("recorded:"), "{done}");
        assert!(done.contains(replay.star().outcome.as_str()), "{done}");
    }

    #[test]
    fn the_field_is_the_last_generation_scored_not_the_champions_own() {
        let level = runway();
        let mut packet = crate::lab::scored_packet(&level, "population");
        assert_eq!(
            packet.best.as_ref().expect("a best agent").generation,
            packet.generation,
            "a one-generation fixture cannot part the two numbers"
        );
        // The run moves past its best idea: it scores further generations while the
        // record stays where it was made, so the newest population no longer holds the
        // genome the record belongs to. That is the ordinary state of a stopped run, and
        // the field's generation has to be its own fact rather than the champion's.
        let star_id = packet.best.as_ref().expect("a best agent").genome.id;
        packet.generation += 7;
        packet.agents.retain(|agent| agent.genome.id != star_id);

        let replay = Replay::new(&packet, &level, &eval()).expect("a best agent");
        assert_eq!(replay.population().generation, packet.generation);
        assert_ne!(replay.population().generation, replay.star().generation);
        assert_eq!(replay.population().size, packet.agents.len());
        // And the field is ghosted in full: the champion is not *in* this generation, so
        // the roster has nothing to filter out of it.
        assert_eq!(replay.ghost_count(), packet.agents.len());
        let line = replay.summary();
        assert!(
            line.contains(&format!("ghosts from generation {}", packet.generation + 1)),
            "{line}"
        );
    }

    #[test]
    fn a_run_with_nothing_scored_yet_has_no_replay() {
        let level = runway();
        let mut packet = crate::lab::scored_packet(&level, "unscored");
        packet.best = None;
        assert!(Replay::new(&packet, &level, &eval()).is_none());
    }
}
