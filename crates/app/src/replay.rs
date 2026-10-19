//! Watching the run's quickest win play the level, at real time, with the population
//! ghosted beside it.
//!
//! The evolution view is the *search*: a generation at a time, at whatever speed makes a
//! generation readable. This is its answer. One episode, run once, at the speed the game
//! actually runs at — 60 ticks a second, the first rung of the speed ladder — with the
//! star solid and the population ghosted on the same clock, the same level and the
//! same physics.
//!
//! The two halves of that come from different places, and it is worth being exact about
//! them because a viewer will watch them diverge. The star is the run's **quickest win**:
//! the episode the clock in [`Packet::fastest`] was measured on, which is older than the
//! newest generation as soon as the search moves past that idea ([`Star`] carries the
//! generation it came from). A run that has never won has no clock to race, and then the
//! star is the best-scoring attempt it does have ([`StarBasis`]) — the most a replay can
//! show of a run that is still learning. The field beside it is the **last generation the
//! run scored** — the population as the run left it, which is what the window was handed
//! and what [`Population::generation`] names. So a stopped run shows the idea that got
//! there quickest racing the population that has since moved on, which is a comparison
//! worth watching rather than an accident.
//!
//! **Not the champion.** The two records are real and they are different agents, which is
//! the whole reason this distinction is spelled out here: the champion is the highest-*
//! fitness* genome, and since fitness v8 the clock is only part of that score (progress,
//! jumps, checkpoints and route hops are the rest), so a run with two winners can have a
//! champion that is slower than its quickest win. Everywhere a viewer races a *time* — this
//! replay, the playtest's ghost, the dock that quotes the number — the record that carries
//! the time is the one that runs. The champion is still the champion; it is just not what a
//! stopwatch is about.
//!
//! Two things make the episodes themselves cheap, and both are already in the packet. The
//! starring genome arrives with the `EpisodeResult` it earned the record on, whose `seed` is
//! half of the episode — a replay with any other seed would be a different run of the same
//! network, which is exactly what a replay must not be. And [`Packet::agents`] holds every
//! agent of that last generation, each paired with the episode it ran, so the ghosts are the
//! population's own runs rather than fresh samples of it.
//!
//! Nothing here trains, and nothing here is a lookalike: the runners are the same
//! [`EpisodeRunner`] the trainer and the champion's own replay check use, so what the
//! viewer watches tick past is the episode the evaluator scored, and it ends on the
//! fitness that episode was recorded with.
//!
//! ```text
//!   the star                              the ghosts
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

/// The agents one replay ghosts: the population's agents other than the star, best
/// first.
///
/// A free function rather than a step inside [`Replay::with_limit`] because this is the
/// rule — *which* agents a viewer is shown beside the star, and in what order — and a
/// rule buried in a constructor is one only an end-to-end test can reach. `agents` is
/// [`Packet::agents`] — the last generation the run scored — in population order; the
/// result is a prefix of it by `mean_fitness`, the metric selection itself ranks on, so
/// "the best 256 of them" is a statement about the run rather than about the order the
/// population happens to sit in.
///
/// The star's own genome is left out: it is not *another* agent, and a second copy of
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

/// Which of the run's two records a replay is showing.
///
/// Two records, and they are two *agents* the moment a run has won more than once. Written
/// down rather than inferred from the star's outcome, because the inference is exactly the
/// one a viewer gets wrong: a run that has not won yet still has a best-scoring agent, and a
/// window that showed it without saying so would be quoting a time nobody has run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarBasis {
    /// The run's quickest winning episode: the clock in [`Packet::fastest`], and the agent
    /// that ran it. What a replay is of whenever the run has won at all, and the record the
    /// playtest's ghost and the dock's time-to-beat come from too.
    QuickestWin,
    /// The highest-fitness agent of a run that has **never** won on this level: there is no
    /// time to race, so the replay shows the best attempt there is — and says so, because
    /// "the agent it is proudest of" and "the fastest agent" are different claims.
    BestScore,
}

impl StarBasis {
    /// What the strip, the panel and the details board call the agent on screen.
    ///
    /// One phrase rather than three, so the three places a viewer reads it cannot disagree
    /// about whose episode they are looking at.
    pub fn label(self) -> &'static str {
        match self {
            StarBasis::QuickestWin => "the run's quickest win",
            StarBasis::BestScore => "the run's best attempt",
        }
    }
}

/// The episode being replayed: the run's quickest win, and the episode that earned it — or, on a
/// run that has never won, the best-scoring attempt it has ([`StarBasis`]).
///
/// Kept as the record itself rather than as a pointer into the packet, because it is what
/// the replay is *checked against*: the score at the end of the run has to land on
/// [`Star::fitness`], and a star that had lost its own numbers could not show anyone that
/// it had.
#[derive(Clone, Debug, PartialEq)]
pub struct Star {
    pub genome_id: u32,
    /// Which record this is ([`StarBasis`]).
    pub basis: StarBasis,
    /// The generation that produced this genome. The run's record, not the newest
    /// generation the window holds: the two part company as soon as the search moves past
    /// that idea, which is why this is printed beside the field's own generation
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
/// Kept as its own fact rather than read off the star, because the two are usually
/// different numbers and a label that conflated them would be wrong in the common case:
/// the population is the run's newest scored generation, and the star is a record set
/// somewhere behind it. The viewer is told both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Population {
    pub generation: u32,
    /// Agents in the generation before the ghost limit, so the panel can say how many of
    /// them a capped roster left out.
    pub size: usize,
}

/// A pool of episodes animated together, on one clock.
///
/// `runners[0]` is the star and the rest are the ghosts, so "the one being watched" is
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
    /// or not it is visible, so turning them back on shows the same field the star has
    /// been running through, not one that fell behind while nobody was looking.
    pub ghosts: bool,
    /// Hold the clock.
    pub paused: bool,
    /// Physics ticks per 1/60 s of wall clock. Opens at [`REAL_TIME`].
    pub speed: u32,
    /// The star's ears (`crate::sound`). Only the star is heard: a field of
    /// thirty ghosts jumping at once is a noise, and the star is the episode
    /// being watched.
    star_ear: crate::sound::Ear,
}

impl Replay {
    /// A replay of the run's quickest win, with the last scored generation as ghosts.
    ///
    /// `None` only when the packet holds no episode at all — no record and no evaluated
    /// genome — which is a run that has scored nothing: there is nothing to watch, and an
    /// empty replay would be a mode with nothing in it.
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
        // The star is the run's **quickest win** when it has one, and its best-scoring attempt
        // when it has not.
        //
        // It used to be `packet.best` — the highest-*fitness* genome — which is a different agent
        // from the record-holder as soon as a run has two winners, because the clock is only part
        // of the score (see [`evo::FastWin`]). The window was then racing one record and following
        // another: the dock quoted the quickest time, the field behind it was the population's own
        // runs, and the solid agent in the middle was frequently *slower* than ghosts a viewer
        // could watch reach the flag ahead of it — which is the one thing a race must not do.
        // `Packet::fastest_agent` is the episode `fastest` was measured on, written beside it by
        // the trainer, so the star now reaches the flag on exactly the tick the dock quotes, and no
        // winning ghost on screen can be quicker than the agent being watched. The playtest's ghost
        // was moved to this same record first and for the same reason (`App::attach_champion_ghost`);
        // this is the replay catching up with it.
        let (record, basis) = match packet.fastest_agent.as_ref() {
            Some(win) => (win, StarBasis::QuickestWin),
            None => (packet.best.as_ref()?, StarBasis::BestScore),
        };
        let star = Star {
            genome_id: record.genome.id,
            basis,
            generation: record.generation,
            seed: record.result.seed,
            fitness: record.result.fitness,
            outcome: record.result.outcome,
            frames: record.result.frames,
        };

        let roster = ghost_roster(&packet.agents, star.genome_id, limit);
        let mut runners: Vec<EpisodeRunner> = Vec::with_capacity(roster.len() + 1);
        runners.push(EpisodeRunner::new(
            level,
            record.genome.clone(),
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
            star_ear: crate::sound::Ear::new(),
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
        if let Some(star) = self.runners.first() {
            self.star_ear.reset(star.sim());
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
        for (index, (runner, anim)) in self
            .runners
            .iter_mut()
            .zip(self.anims.iter_mut())
            .enumerate()
        {
            let mut stepped = 0u32;
            for _ in 0..ticks {
                // A finished episode holds its last frame while the agents still running
                // carry on: that is the whole point of running them in parallel.
                if runner.is_finished() {
                    break;
                }
                runner.step();
                stepped += 1;
                if index == 0 {
                    // The star is heard at the replay's own speed: at 8x its
                    // jumps are 8x as frequent, which is the honest sound of
                    // watching at 8x.
                    self.star_ear.observe(runner.sim(), 0.9);
                }
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

    /// The star's live episode.
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

    /// One ghost's live episode. Index `0` is the first ghost, never the star.
    pub fn ghost_sim(&self, i: usize) -> &Sim {
        self.runners[i + 1].sim()
    }

    pub fn ghost_anim(&self, i: usize) -> &AgentAnim {
        &self.anims[i + 1]
    }

    /// Seconds of the **field** simulated — not seconds on the wall clock: at 4x the difference
    /// is the whole point.
    ///
    /// The field's clock, not the star's: it is what the level's scenery and the walkers are
    /// animated on, so the world keeps moving while the slower agents behind the star are still
    /// out there. Nothing a viewer reads a *time* from uses it — [`Replay::star_elapsed`] is the
    /// number on the strip.
    pub fn clock(&self) -> f32 {
        self.ticks as f32 * sim::DT
    }

    /// The star's own episode length, in frames: the unit every time in this project is kept in
    /// ([`evo::FastWin::frames`], [`crate::play::Session::frames`]).
    ///
    /// Off the star's own simulation, which stops counting the moment its episode ends — so this
    /// holds at the frame the agent reached the flag (or died) and never climbs after it.
    pub fn star_frames(&self) -> u32 {
        self.star_sim().stats().frames
    }

    /// The star's own episode time in seconds: the number to read a time from, and the one the
    /// strip and the panel quote.
    ///
    /// A finished episode holds its last frame, so this stops where the star stopped — a win in
    /// 5.2 s keeps reading 5.2 s while the ghosts behind it are still running and the scenery is
    /// still waving. The field's clock ([`Replay::clock`]) is the one that keeps going, and it is
    /// for animating the world rather than for timing an episode. Both the strip and the panel
    /// used the field's clock here, which is what made a won race read like a stopwatch nobody had
    /// stopped: the agent stood at the flag while the seconds it had already spent kept ticking up
    /// beside it.
    pub fn star_elapsed(&self) -> f32 {
        self.star_frames() as f32 * sim::DT
    }

    /// The star's score so far, by the same function the evaluator used.
    ///
    /// Live rather than predicted: while it runs it answers "how is it going", and when it
    /// stops it is the number to hold against [`Star::fitness`] — which is why the status
    /// line prints both.
    pub fn star_score(&self) -> f32 {
        let sim = self.star_sim();
        score(sim.stats(), sim.outcome())
    }

    /// The star's episode has ended: the flag, a grave, or the sim's own caps.
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

    /// The status line: whose episode this is, how it is going, and whether the field is on screen.
    ///
    /// One string built here rather than in the window for the reason `play::Session` has
    /// one: a number assembled where it is drawn cannot be tested, and this one carries the
    /// fact the replay exists for — the recorded fitness beside the score actually being
    /// produced.
    ///
    /// **A ledger, not a report.** It used to carry the field in full as well — how many
    /// ghosts came from which generation, how many were still going and how many were at the
    /// flag — which is a paragraph about a picture the canvas is drawing in colour and the
    /// Details window states in numbers ([`crate::panel::replay_details`]). On a strip along
    /// the bottom of the window those three clauses pushed the line past the edge of the
    /// window, and what a strip is for is the number that changes.
    pub fn summary(&self) -> String {
        // The clock on the strip is the **star's own** ([`Replay::star_elapsed`]), not the field's.
        // The field's keeps running while the slower agents behind it are still out there, so a
        // finished episode read as one that was still costing time after it had won — the number
        // beside "won" climbed for as long as the viewer left the replay open. Both halves of this
        // line are now the star's: the time it took, and the score it made.
        let outcome = self.star_sim().outcome();
        let headline = if outcome.is_finished() {
            format!(
                "{} in {:.1}s — score {} (recorded {})",
                outcome.as_str(),
                self.star_elapsed(),
                format_fitness(self.star_score()),
                format_fitness(self.star.fitness)
            )
        } else {
            format!(
                "running — {:.1}s in, score {}",
                self.star_elapsed(),
                format_fitness(self.star_score())
            )
        };
        // Only a *hidden* field is worth a word here. Ghosts that are showing explain
        // themselves, and a generation switched off has to say so, or an empty canvas with no
        // explanation reads as a replay that is broken rather than one that is quiet.
        let ghosts = if self.ghosts {
            String::new()
        } else {
            format!(" — {} ghosts hidden", self.ghost_count())
        };
        format!(
            "replaying {} of generation {} (id {}) — {headline}{ghosts} — {}x{}",
            self.star.basis.label(),
            self.star.generation + 1,
            self.star.genome_id,
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

    /// A packet whose run recorded a **quickest win** by one agent while its best *score* belongs
    /// to another — the state a real run reaches the moment it has two winners, and the one the
    /// choice of star is about.
    ///
    /// `scored_packet` gives one generation of a run that has won nothing, so the two records are
    /// raised here by hand out of that generation's own agents. Only the two records are synthetic:
    /// each still carries the genome, seed and result of an episode that really was evaluated, so
    /// the fidelity claims the replay makes about the star still mean something on this packet.
    fn packet_with_two_records(level: &Level, tag: &str) -> Packet {
        let mut packet = crate::lab::scored_packet(level, tag);
        let quickest = packet
            .agents
            .iter()
            .min_by_key(|agent| agent.result.frames)
            .cloned()
            .expect("a generation");
        // The *slowest* agent of the generation is the champion, so the two records are as far
        // apart as one generation allows and nothing here can pass by accident.
        let champion = packet
            .agents
            .iter()
            .filter(|agent| agent.genome.id != quickest.genome.id)
            .max_by_key(|agent| agent.result.frames)
            .cloned()
            .expect("a second agent");
        assert_ne!(quickest.genome.id, champion.genome.id);

        packet.fastest = Some(evo::FastWin {
            generation: packet.generation,
            genome_id: quickest.genome.id,
            frames: quickest.result.frames,
            budget_frames: sim::frame_budget(level),
        });
        packet.fastest_agent = Some(evo::BestEver {
            generation: packet.generation,
            result: quickest.result.clone(),
            genome: quickest.genome.clone(),
            // A clock is one episode: the seeded validation set answers a different question than
            // the race this record is quoted in.
            validated: None,
        });
        packet.best = Some(evo::BestEver {
            generation: packet.generation,
            result: champion.result.clone(),
            genome: champion.genome.clone(),
            // The run's validated reading belongs to the trainer that measured it; a replay
            // rebuilt from a champion file is one episode and says so.
            validated: None,
        });
        packet
    }

    /// A level whose flag stands two tiles from the spawn, so a genome that walks right wins in a
    /// handful of frames and a generation of eight contains winners. The runway cannot promise
    /// that, and a test about the *quickest win* needs a run that has one.
    fn dash() -> Level {
        let mut level = Level::starter("replay-dash", 40, 8);
        level.set_goal_tile(3, 6);
        level
    }

    /// The seed the evaluator used for one agent's representative episode.
    ///
    /// Asked of the trainer rather than derived here, so a test that pins the replays of a run is
    /// pinned to the run's own episode policy ([`GaConfig::common_episodes`]) instead of to a copy of
    /// it that would silently stop agreeing with the trainer the day the policy moved.
    fn scored_seed(generation: u32, genome_id: u32) -> u64 {
        evo::ga::episode_seed_for(&GaConfig::default(), generation, genome_id, 0)
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
    fn the_star_replays_the_episode_the_record_was_made_on() {
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
    fn the_star_runs_the_seed_it_was_scored_under() {
        let level = runway();
        // The record path rather than the fallback: this is the episode a viewer races, so its seed
        // is the one the evaluator used for that genome's trial 0 and not a fresh sample of the same
        // network.
        let packet = packet_with_two_records(&level, "star-seed");
        let replay = Replay::new(&packet, &level, &eval()).expect("a record");

        let star = replay.star();
        assert_eq!(
            star.seed,
            scored_seed(star.generation, star.genome_id),
            "the star is not running the seed it was scored under"
        );
        assert_eq!(
            star.seed,
            packet.fastest_agent.as_ref().expect("a record").result.seed
        );
    }

    #[test]
    fn the_star_is_the_runs_quickest_win_rather_than_its_best_score() {
        let level = runway();
        let packet = packet_with_two_records(&level, "two-records");
        let win = packet.fastest_agent.as_ref().expect("a record");
        let champion = packet.best.as_ref().expect("a champion");
        let time = packet.fastest.expect("the time beside it");
        assert_ne!(
            win.genome.id, champion.genome.id,
            "the fixture needs two agents"
        );

        let mut replay = Replay::new(&packet, &level, &eval()).expect("a record");
        // The agent the clock was measured on, and that agent's own episode. The champion is a
        // *different* agent here — that is the whole point of the fixture — and it is not the one
        // on screen.
        assert_eq!(replay.star().basis, StarBasis::QuickestWin);
        assert_eq!(replay.star().genome_id, win.genome.id);
        assert_eq!(replay.star().generation, win.generation);
        assert_eq!(replay.star().seed, win.result.seed);
        assert_eq!(replay.star().frames, time.frames);
        assert_eq!(replay.star().fitness, win.result.fitness);
        assert_ne!(replay.star().genome_id, champion.genome.id);

        // And the champion did not vanish: it is in the field behind the star, which is what the
        // viewer was watching — the run's best-scoring agent running *behind* the one being raced.
        let roster = ghost_roster(&packet.agents, win.genome.id, GHOST_LIMIT);
        assert_eq!(replay.ghost_count(), roster.len());
        assert_eq!(
            roster.len(),
            packet.agents.len() - 1,
            "the star's own slot is the only one the roster drops"
        );
        assert!(
            roster.iter().any(|a| a.genome.id == champion.genome.id),
            "the champion is not ghosted beside the star"
        );

        // The record alone is enough to build one: a packet with no champion in it still has an
        // episode a viewer can be shown, which is the precedence the choice makes explicit rather
        // than leaving to whichever field happened to be `Some`.
        let mut record_only = packet.clone();
        record_only.best = None;
        let without_champion = Replay::new(&record_only, &level, &eval()).expect("a record");
        assert_eq!(without_champion.star().basis, StarBasis::QuickestWin);
        assert_eq!(without_champion.star().genome_id, win.genome.id);

        // The star's episode is the record's episode, replayed: it ends the same way, in the same
        // number of frames, which is the claim the strip's time rests on.
        run_to_end(&mut replay, 4_000);
        assert_eq!(replay.star_sim().outcome(), win.result.outcome);
        assert_eq!(replay.star_frames(), time.frames);
    }

    #[test]
    fn a_run_that_has_never_won_replays_its_best_attempt_and_says_so() {
        let level = runway();
        let mut packet = crate::lab::scored_packet(&level, "no-win");
        packet.fastest = None;
        packet.fastest_agent = None;
        let best = packet.best.clone().expect("a best agent");

        let replay = Replay::new(&packet, &level, &eval()).expect("a best agent");
        assert_eq!(replay.star().basis, StarBasis::BestScore);
        assert_eq!(replay.star().genome_id, best.genome.id);
        assert_eq!(replay.star().seed, best.result.seed);

        // No time to race is not the same claim as the run's own record, and the strip says which
        // one it is making rather than quoting a clock nobody has run.
        let line = replay.summary();
        assert!(
            line.contains(StarBasis::BestScore.label()),
            "the strip does not say what this agent is: {line}"
        );
        assert!(
            !line.contains(StarBasis::QuickestWin.label()),
            "an unwon run's replay advertises a win: {line}"
        );
    }

    #[test]
    fn the_clock_stops_where_the_star_stopped() {
        let level = runway();
        let mut replay = replay_on(&level, "stopwatch");
        run_to_end(&mut replay, 4_000);

        // The time is the star's own episode, in the unit the episode was scored in.
        assert_eq!(replay.star_frames(), replay.star().frames);
        let stopped = replay.star_elapsed();
        assert!(
            (stopped - replay.star_frames() as f32 * sim::DT).abs() < 1e-6,
            "the episode time is not the episode's length: {stopped}"
        );
        let line = replay.summary();

        // Ten more seconds of wall clock, with the field's own clock running on. The number a viewer
        // reads a time from does not move again: it is the time the agent *arrived* in, not how long
        // the replay has been open, and the strip that quoted a climbing clock beside "won" was the
        // bug this pins.
        for _ in 0..600 {
            replay.advance(1.0 / 60.0);
        }
        assert!(
            replay.clock() > stopped,
            "the field's clock stopped too: {} vs {stopped}",
            replay.clock()
        );
        assert_eq!(
            replay.star_elapsed(),
            stopped,
            "the episode time kept counting after the episode ended"
        );
        assert_eq!(
            replay.summary(),
            line,
            "the strip's time moved after the finish"
        );
    }

    #[test]
    fn no_ghost_reaches_the_flag_before_the_star() {
        // A real run that really won, rather than the fabricated records above: the claim is about
        // the field a viewer watches, and the field is the population's own episodes.
        let level = dash();
        let packet = crate::lab::scored_packet(&level, "quickest");
        let win = packet
            .fastest_agent
            .as_ref()
            .expect("a generation of dash-runners holds a winner");
        let time = packet.fastest.as_ref().expect("the time beside it");
        assert!(win.result.won(), "the record's episode is not a win");
        assert_eq!(
            win.result.frames, time.frames,
            "the run's time and the episode it came from have parted"
        );

        let mut replay = Replay::new(&packet, &level, &eval()).expect("a record");
        assert_eq!(replay.star().basis, StarBasis::QuickestWin);
        assert_eq!(replay.star().frames, time.frames);
        assert_eq!(replay.star().genome_id, time.genome_id);

        let mut ticks = 0;
        while !replay.all_finished() && ticks < 4_000 {
            replay.advance(1.0 / 60.0);
            ticks += 1;
        }
        assert!(ticks < 4_000, "the field never finished");
        assert!(
            replay.ghosts_won() > 0,
            "no ghost won, so the field proved nothing about the race"
        );

        // The rule the whole change is for: on the replays a viewer races, nobody reaches the flag
        // ahead of the agent being followed. A quick win by the star and a quick win by a ghost were
        // what the old star produced — the champion on screen while the field overtook it.
        let star = replay.star_frames();
        for i in 0..replay.ghost_count() {
            let ghost = replay.ghost_sim(i);
            if ghost.outcome() == Outcome::Won {
                assert!(
                    ghost.stats().frames >= star,
                    "ghost {i} reached the flag in {} frames, ahead of the star's {star}",
                    ghost.stats().frames
                );
            }
        }
    }

    #[test]
    fn the_ghosts_are_the_generations_other_agents_in_rank_order() {
        let level = runway();
        let packet = crate::lab::scored_packet(&level, "roster");
        let replay = Replay::new(&packet, &level, &eval()).expect("a best agent");
        let star = replay.star().genome_id;

        assert_eq!(packet.agents.len(), packet.pop_size);
        // The generation still holds the star — elites are copied unchanged and keep
        // their ids — and it is not ghosted: it is the agent on screen, solid.
        assert!(
            packet.agents.iter().any(|a| a.genome.id == star),
            "the star is not in its own generation"
        );
        let roster = ghost_roster(&packet.agents, star, GHOST_LIMIT);
        assert_eq!(roster.len(), packet.agents.len() - 1);
        assert_eq!(replay.ghost_count(), roster.len());
        assert!(
            roster.iter().all(|a| a.genome.id != star),
            "the star is ghosted beside itself"
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
        }
        // Under `common_episodes` — the default — those seeds are *all the same one*, the star's
        // included: the generation is one world and every lane in it is a policy played on that
        // world, which is what makes the panel's side-by-side comparison a comparison. The trainer
        // is asked rather than assumed, so this test follows the policy instead of pinning one:
        // with pairing off the lanes are each agent's own episode and this block reads the other way.
        let seeds: Vec<u64> = packet.agents.iter().map(|a| a.result.seed).collect();
        let paired = GaConfig::default().common_episodes;
        assert!(
            if paired {
                seeds.windows(2).all(|w| w[0] == w[1])
            } else {
                seeds.windows(2).all(|w| w[0] != w[1])
            },
            "the lanes do not follow the episode policy (paired = {paired}): {seeds:?}"
        );
        assert_eq!(
            replay.star().seed,
            scored_seed(packet.generation, star),
            "and the star replays the episode its own generation was judged on"
        );
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
        // position is a better `mean_fitness`. It is built around the same agent the replay
        // starred — this fixture wins nothing, so that is the run's best-scoring agent.
        let packet = crate::lab::scored_packet(&level, "cohorts");
        let star = packet.best.as_ref().expect("a best agent").genome.id;
        let ranked = ghost_roster(&packet.agents, star, GHOST_LIMIT);
        for pair in ranked.windows(2) {
            assert!(pair[0].result.mean_fitness >= pair[1].result.mean_fitness);
        }
    }

    #[test]
    fn every_ghost_replays_the_episode_it_was_scored_on() {
        let level = runway();
        let packet = crate::lab::scored_packet(&level, "ghost-fidelity");
        let star = packet.best.as_ref().expect("a best agent").genome.id;
        let roster = ghost_roster(&packet.agents, star, GHOST_LIMIT)
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
        let star = packet.best.as_ref().expect("a best agent").genome.id;

        let mut ranked: Vec<(f32, u32)> = packet
            .agents
            .iter()
            .filter(|a| a.genome.id != star)
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
        let roster = ghost_roster(&packet.agents, star, 3);
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
    fn the_status_line_names_the_episode_and_keeps_the_field_off_it() {
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
        assert!(line.contains("1x"), "the clock is not named: {line}");
        // Whose episode this is, in the words the panel and the details board use: the star of a run
        // that has won is its quickest win, and the strip says so rather than leaving a viewer to
        // read it off the id.
        assert!(
            line.contains(replay.star().basis.label()),
            "the strip does not say what is being replayed: {line}"
        );
        // The field is not on the strip while it is on the canvas: its roster is three readings
        // the Details window states in full, and a strip that tried to say them as well ran off
        // the end of the window (see `summary`).
        assert!(
            !line.contains("ghosts"),
            "a field nobody has hidden is described on the strip: {line}"
        );

        // A hidden field says so — and how big it is, because nothing else on screen says what
        // the viewer has switched off.
        replay.ghosts = false;
        let hidden = replay.summary();
        assert!(hidden.contains("ghosts hidden"), "{hidden}");
        assert!(
            hidden.contains(&format!("{} ghosts", replay.ghost_count())),
            "{hidden}"
        );

        // And once the champion's episode is over the line carries both numbers — the
        // score it just produced and the record it has to match — because those two being
        // equal is the whole claim a replay makes.
        run_to_end(&mut replay, 4_000);
        let done = replay.summary();
        assert!(
            done.contains(&format!("recorded {}", format_fitness(replay.star().fitness))),
            "the record the replay has to match is not on the line: {done}"
        );
        assert!(done.contains(replay.star().outcome.as_str()), "{done}");
    }

    #[test]
    fn the_field_is_the_last_generation_scored_not_the_stars_own() {
        let level = runway();
        let mut packet = crate::lab::scored_packet(&level, "population");
        assert_eq!(
            packet.best.as_ref().expect("a best agent").generation,
            packet.generation,
            "a one-generation fixture cannot part the two numbers"
        );
        // The run moves past its record: it scores further generations while the
        // record stays where it was made, so the newest population no longer holds the
        // genome the record belongs to. That is the ordinary state of a stopped run, and
        // the field's generation has to be its own fact rather than the star's.
        let star_id = packet.best.as_ref().expect("a best agent").genome.id;
        packet.generation += 7;
        packet.agents.retain(|agent| agent.genome.id != star_id);

        let replay = Replay::new(&packet, &level, &eval()).expect("a best agent");
        assert_eq!(replay.population().generation, packet.generation);
        assert_ne!(replay.population().generation, replay.star().generation);
        assert_eq!(replay.population().size, packet.agents.len());
        // And the field is ghosted in full: the star is not *in* this generation, so
        // the roster has nothing to filter out of it. The generation the ghosts belong to is
        // reported where a viewer looks for it — the details board — rather than on the strip, so
        // what the *line* owes is that it does not claim the star's own generation instead.
        assert_eq!(replay.ghost_count(), packet.agents.len());
        let line = replay.summary();
        assert!(
            !line.contains(&format!("generation {}", replay.population().generation + 1)),
            "the strip names the field's own generation: {line}"
        );
    }

    #[test]
    fn a_run_with_nothing_scored_yet_has_no_replay() {
        let level = runway();
        let mut packet = crate::lab::scored_packet(&level, "unscored");
        // Both records, because either one alone is enough to replay: the run's quickest win comes
        // first and the best score is what is left when there is no win.
        packet.best = None;
        packet.fastest = None;
        packet.fastest_agent = None;
        assert!(Replay::new(&packet, &level, &eval()).is_none());
    }
}
