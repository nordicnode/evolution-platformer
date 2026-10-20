//! Behaviour cloning: turn a *walked route* into a policy.
//!
//! [`crate::route_play`] answers "can a body walk this level" by driving the oracle's own route
//! through the real sim, and it records the (observation, action) pair at every decision the body
//! got to make. That recording is a **demonstration**, and a demonstration is the one thing the
//! evolution loop has never had: every genome the trainer has ever evaluated started from noise and
//! found its own way up, so a level whose route is long and precise (the pack's `l9-wait-and-climb`
//! is a nine-move chain) has to be *rediscovered* by search even though the reachability oracle can
//! already walk it. This module closes that loop: it fits a network to the demonstration, and the
//! result is a genome that can play the level — a **warm start**.
//!
//! # It is a seed and not a solution
//!
//! A clone is not offered as an answer to a level, and the design says so on purpose:
//!
//! * it knows **only what was demonstrated**. Nothing in the objective rewards a decision the
//!   demonstration never made, so a clone cannot prefer a better route, cannot recover from a
//!   disturbance the replay never met, and cannot generalise to a variant of the level — which is
//!   exactly what a level *pack* and a level *editor* keep producing;
//! * it is fitted on the replay's own observations, so its errors compound off-distribution. The
//!   demonstration visits a corridor of states; a clone that drifts out of it has never seen what
//!   to do there. Evolution is what repairs that, and it starts from a policy that already walks
//!   the level — which is the whole point.
//!
//! So the honest claim for `--from-route` is "the population starts with somebody who can play
//! this" rather than "this level is solved", and the measurement that belongs next to it is
//! *generations to a win* against the same run from random weights, not a win/lose bit.
//!
//! # The cadence, and why a clone cannot finish the level
//!
//! The reason is **time**, and it took three measurements to stop blaming the fit for it.
//!
//! `sim::reach` sweeps its canonical moves with the sim's own stepper and presses a take-off *on the
//! tick the body reaches it*. A policy decides once per [`EvalConfig::action_repeat`] ticks — four,
//! on the shipped config — and holds that arm in between, so there is no decision of the policy's at
//! the tick the oracle presses on. What follows is that a demonstration is a controller for a
//! *different control grid* than the one the policy is allowed to act on:
//!
//! * at the shipped cadence a 4000-epoch fit reproduces **every one** of `l2-first-gap`'s 139
//!   decisions and still walks into the pit, on all eight evaluation seeds, identically; at 300
//!   epochs it reproduces 41 of 46 blocks and does the same thing;
//! * forcing the *replay* to press on a decision boundary instead breaks the walk itself — `l2`,
//!   `l4-rhythm` and `l9-wait-and-climb` die at their first gap inside 200 frames — so the
//!   tick-precise press is what the walk needs, not an artefact of how it is recorded;
//! * and at `action_repeat = 1`, where a decision *is* a tick, the same fit on the same level solves
//!   it on all eight seeds with identical frame counts: a clone that reproduces every decision is
//!   walking the route the oracle walked. `l4-rhythm` and `l5-tightrope` behave the same way.
//!
//! So "a clone walks the route and cannot finish it" is a statement about the cadence, and it is the
//! pinned claim of `crates/evo/tests/imitate.rs`, not a defect to be fitted away. It also means a
//! clone is worth *most* to a run that is already cheap to warm — the head start is a head start on
//! walking, and what it cannot do is buy the win.
//!
//! **What the fit's fidelity is worth is a separate question, and it was measured too**: sensors v10
//! — the body's sub-tile phase at indices 98 and 99 — moves a 4000-epoch fit from 18-19 missed
//! decisions to 0-2 of 551 on a tick-cadence `l2-first-gap` demonstration, which at that cadence is
//! the difference between solving and not. The fit's own *selection* criterion was also tried: keeping
//! the epoch with the fewest missed decisions instead of the lowest loss fits the demonstration more
//! closely and makes the warm start no better (the paired arms trade places level by level), so the
//! criterion stayed on the loss.
//!
//! # The objective, and why not cross-entropy
//!
//! The net's output layer is `tanh`, so the action "scores" live in `(-1, 1)` and a softmax over
//! them would be reading a squashed vector as logits. The clone is fitted by **regression on the
//! demonstrated action**: the chosen action's output is pulled toward `+1` and every other toward
//! `-1`, minimising `½·Σ(oᵢ − yᵢ)²` by full-batch gradient descent with the gradient taken through
//! the tanh. That is a multi-label fit of the policy's own score vector, it needs no extra head, and
//! the resulting `argmax` is the action the demonstration asked for.
//!
//! **The demonstration is mostly `Walk`.** A route is a long steady walk with a few presses in it,
//! so an unweighted fit learns the majority class and nothing else — which on a flat level is
//! already a winning policy and on a real one is worthless. Exemplars are therefore weighted by the
//! inverse frequency of their action (capped at [`CLASS_WEIGHT_CAP`]), so a hop the body got to make
//! once weighs as much as the two hundred idle blocks around it.
//!
//! # Memory
//!
//! A shape with `feedback > 0` is trained **free-running**: the clone's own recurrent state evolves
//! through the demonstration, and each exemplar's gradient treats that state as an input (no
//! backpropagation through time). Teacher-forcing the state instead would fit a net that is never
//! the net that runs, and full BPTT is a different module's problem; the reactive half is what a
//! replay demonstrates. A memoryless shape — the shipped default — is unaffected.
//!
//! **Determinism.** Only the initial weights are drawn, from [`ImitateConfig::init_seed`], so
//! `(play, shape, config)` determines the clone bit for bit. Nothing else in the fit is random:
//! the passes are in demonstration order and the arithmetic is `f32` in a fixed order.
//!
//! # Phase 2: teaching the leg that fails
//!
//! DAgger repairs the corridor the student leaves; it does not choose *where* to teach. A clone that
//! dies at `l11`'s first spike spent the whole whole-route pass on the states before it. So
//! [`from_certified_plan`] now measures **per-leg mastery** ([`measure_mastery`]) after every fit:
//! the plan's own legs, the crossings the clone cleared, and the leg its episode ended in — the
//! same index the telemetry reports. The first uncleared leg is the lesson, and the curriculum rolls
//! out from the leg before it (the approach contains the failure) starting at the state the student
//! actually reaches, labels off-corridor states with [`sim::plan::plan_leg`], and refits from the
//! student. Corrections are therefore **planned from where the student is**, one crossing at a
//! time, and the returned clone is the one the closed loop measured best, never the one the fit
//! liked.
//!
//! # What the fit returns
//!
//! The best epoch by training loss is kept ([`ImitationReport::best_epoch`]), because a fit this
//! small is not monotone — the last epoch is not reliably the best one — and [`agreement`] is
//! reported both for the clone and for its untrained initialisation, so "the fit learned something"
//! is a number and not a claim. [`agreement`]: ImitationReport::agreement

use sim::physics::AgentAction;
use sim::rng::rng_from_seed;
use sim::sensors::N_INPUTS;
use sim::route_plan::RoutePlan;
use sim::{Level, ObservationScratch, Outcome, Sim};

use crate::fitness::EvalConfig;
use crate::genome::{ActScratch, Genome, NetShape, GENE_CLAMP};
use crate::route_play::{replay, timed_play, Decision, RoutePlay};

/// How much a single-action exemplar may outweigh a common one.
///
/// Unbounded inverse frequency would let one `SprintJump` in a walk-heavy demonstration dominate
/// every update; six is enough to make a rare press matter and not enough to make the fit a
/// stair-climber that has forgotten how to walk.
pub const CLASS_WEIGHT_CAP: f32 = 6.0;

/// Training schedule for one clone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImitateConfig {
    /// Full-batch gradient passes over the demonstration.
    pub epochs: usize,
    /// Learning rate.
    pub lr: f32,
    /// Seed for the **initial weights only** — see the module docs on determinism.
    pub init_seed: u64,
    /// Keep the epoch that reproduces the **most decisions** rather than the one with the lowest
    /// loss. Off for a route walk (where the loss-picked clone was measured to be at least as good
    /// a warm start), on for a timed plan: a plan is a script, and one wrong block at a crossing
    /// can end the episode before evolution ever sees it. It is a selection rule, not a different
    /// objective — the epoch is still chosen from the epochs the same fit produced.
    pub prefer_agreement: bool,
    /// Outer **leg-curriculum** rounds: after each fit, measure which leg the student fails, roll
    /// it out from the leg before that one with the planner correcting it, and refit. Zero disables
    /// the curriculum and leaves the whole-route DAgger pass alone.
    pub leg_rounds: usize,
    /// Rollouts collected per curriculum round. Each starts one leg before the failing one, at the
    /// state the student actually reaches, so the lesson contains the run-up, the crossing and the
    /// failure rather than only the few blocks the student survives after it.
    pub leg_rollouts: usize,
    /// Planner expansions for a **correction** planned from a divergent state. Small on purpose:
    /// a correction only has to finish the leg the student is already standing on, and the whole
    /// level's search depth would pay for the corridor it has already walked.
    pub replan_depth: u32,
    /// Planner calls one rollout may spend before it falls back to the nearest demonstrated
    /// action. A correction route covers several blocks, and replanning is only asked for when the
    /// student is off the demonstrated corridor, so this is a budget on distinct divergences.
    pub replan_cap: usize,
    /// Whole-route DAgger rounds, before the leg curriculum. Two is deliberate: a whole-route pass
    /// teaches recovery along the route, and the curriculum is what spends the budget where the
    /// student actually stops. Six whole-route rounds were measured to cost a full-level search per
    /// divergence and to add no leg the curriculum did not reach cheaper.
    pub dagger_rounds: usize,
    /// Run the demonstration and the local corrections through the **meta-controller** ([`sim::meta`]),
    /// which owns commit / wait / ride. Off, the plan's blocks and the raw leg routes are the
    /// teacher and the report has no skill trace — the Phase 2 arm.
    pub meta: bool,
}

impl Default for ImitateConfig {
    fn default() -> Self {
        // Tuned by hand against the shipped pack (a `l2-first-gap` demonstration is a few hundred
        // exemplars over a `96>16>11` net): 300 passes cost about a second, and agreement is flat
        // well before the epoch budget is spent, so this is a ceiling rather than a dial.
        Self {
            epochs: 300,
            lr: 0.05,
            init_seed: 0x1CE_B007,
            prefer_agreement: false,
            leg_rounds: 2,
            leg_rollouts: 2,
            replan_depth: 16,
            replan_cap: 8,
            dagger_rounds: 2,
            meta: true,
        }
    }
}

/// What the fit produced.
#[derive(Clone, Debug)]
pub struct ImitationReport {
    /// The cloned policy, ready to be evaluated or to seed a population.
    pub genome: Genome,
    /// Demonstrated decisions fitted.
    pub exemplars: usize,
    pub epochs: usize,
    /// Training loss before the first update.
    pub first_loss: f32,
    /// Training loss of the returned weights.
    pub final_loss: f32,
    /// Fraction of demonstrated decisions the **clone** reproduces.
    pub agreement: f32,
    /// The same fraction for the untrained initialisation: the scale `agreement` is read against.
    pub baseline_agreement: f32,
    /// The epoch whose weights are the ones returned.
    pub best_epoch: usize,
    /// Per-leg reading of the returned clone against the certified plan, when the fit came from
    /// one. `None` for a route-walk clone, which has no legs to measure.
    pub mastery: Option<LegMastery>,
    /// What the meta-controller decided while producing the demonstration: route, commit, wait,
    /// ride, with the leg and block each first fired in. `None` when the controller was off.
    pub skills: Option<sim::meta::SkillTrace>,
}

/// What one leg of the plan did for the clone: did it get there, did it leave it, and did the
/// episode end inside it.
///
/// The index is the plan's leg index, deliberately the same number the telemetry reports as
/// `best_end_leg`: a body that has closed `k` crossings is in leg `k`. That makes "the clone died
/// in leg 1" readable against both the plan's margins and a run's death column without a
/// translation table.
#[derive(Clone, Debug, PartialEq)]
pub struct LegSkill {
    /// Zero-based plan leg.
    pub leg: u32,
    /// The clone closed enough crossings to enter this leg.
    pub reached: bool,
    /// The clone closed this leg's crossing (or, for a finish leg, reached the flag).
    pub cleared: bool,
    /// The episode ended inside this leg.
    pub ended: bool,
    /// Frames the clone spent in this leg.
    pub frames: u32,
}

/// **Per-leg mastery**: what a clone can actually do, leg by leg, rather than one agreement
/// fraction over the whole route.
///
/// Agreement is a fit statistic measured on the demonstration's own corridor; it cannot say
/// *where* a student leaves the plan. This is the game's own answer: roll the clone out from the
/// spawn on the plan's episode and read the crossing count at every block. The first leg that is
/// not cleared is the leg to spend the next round on, and [`LegMastery::line`] names it.
#[derive(Clone, Debug, PartialEq)]
pub struct LegMastery {
    /// One entry per plan leg, in order.
    pub legs: Vec<LegSkill>,
    pub outcome: Outcome,
    pub frames: u32,
    /// Horizontal progress in pixels — the reading the champion's own episode uses.
    pub progress_pixels: f32,
    /// The leg the episode ended in.
    pub end_leg: u32,
    pub won: bool,
}

impl LegMastery {
    /// Legs whose crossing the clone closed.
    pub fn cleared(&self) -> usize {
        self.legs.iter().filter(|l| l.cleared).count()
    }

    /// **The leg the curriculum should teach.** The first leg the clone did not clear — on a win,
    /// `None`. This is where the episode stopped, and the only leg a correction can be about.
    pub fn weakest(&self) -> Option<u32> {
        if self.won {
            return None;
        }
        self.legs
            .iter()
            .find(|leg| !leg.cleared)
            .map(|leg| leg.leg)
    }

    /// Ordering for "is this student better": a win first, then legs cleared, then progress.
    pub fn rank(&self) -> (u8, usize, i64) {
        (
            u8::from(self.won),
            self.cleared(),
            self.progress_pixels as i64,
        )
    }

    /// One line a report or a terminal can print: how far the clone got, and which leg stopped it.
    pub fn line(&self) -> String {
        let total = self.legs.len();
        match self.weakest() {
            Some(leg) => format!(
                "mastery {}/{} legs cleared, ended in leg {} ({:?} after {} frames, best_x {:.1})",
                self.cleared(),
                total,
                leg,
                self.outcome,
                self.frames,
                self.progress_pixels / 16.0
            ),
            None => format!(
                "mastery {}/{} legs cleared, won in {} frames",
                self.cleared(),
                total,
                self.frames
            ),
        }
    }
}

impl ImitationReport {
    /// Agreement above chance, in `0..=1`: `0` is "no better than the initialisation", `1` is
    /// "every decision in the demonstration is reproduced".
    ///
    /// Chance for 11 actions is 1/11, and an untrained net is usually near it, so the raw fraction
    /// mixes in a floor nobody wants to read.
    pub fn skill(&self) -> f32 {
        let chance = 1.0 / AgentAction::COUNT as f32;
        let span = 1.0 - chance;
        if span <= 0.0 {
            return 0.0;
        }
        ((self.agreement - chance) / span).clamp(0.0, 1.0)
    }

    /// One line for a terminal, in the trainer's own voice.
    pub fn line(&self) -> String {
        let mut out = format!(
            "clone: {} exemplars, {} epochs (best {}), agreement {:.1}% ({:.1}% untrained), loss {:.4} -> {:.4}",
            self.exemplars,
            self.epochs,
            self.best_epoch,
            self.agreement * 100.0,
            self.baseline_agreement * 100.0,
            self.first_loss,
            self.final_loss,
        );
        if let Some(mastery) = &self.mastery {
            out.push_str("; ");
            out.push_str(&mastery.line());
        }
        if let Some(skills) = &self.skills {
            // The trace is the **demonstration's**, not the clone's: `skills route 30, commit …`
            // would read as the fitted policy's habits and be wrong by a mile.
            out.push_str("; demo ");
            out.push_str(&skills.line());
        }
        out
    }
}

/// One layer's weight and bias offsets, plus its shape. The fit walks layers repeatedly, and the
/// offsets have to agree with [`Genome::forward`]'s own recurrence rather than be re-derived each
/// pass.
struct Layer {
    w_off: usize,
    b_off: usize,
    fan_in: usize,
    fan_out: usize,
}

fn layers(shape: &NetShape) -> Vec<Layer> {
    let mut out = Vec::with_capacity(shape.n_layers());
    let mut w_off = 0usize;
    let mut b_off = 0usize;
    for l in 0..shape.n_layers() {
        let (fan_in, fan_out) = shape.layer(l);
        out.push(Layer {
            w_off,
            b_off,
            fan_in,
            fan_out,
        });
        w_off += fan_in * fan_out;
        b_off += fan_out;
    }
    out
}

/// Pre-activation tape for one forward pass: `xs[l]` is layer `l`'s input (`xs[0]` is
/// observation-plus-memory) and `zs[l]` its pre-activation. Activations are `tanh(z)`, so they are
/// recovered from `zs` rather than stored twice.
///
/// Reused across the whole fit — a demonstration is thousands of passes and none of them should
/// allocate.
struct Tape {
    xs: Vec<Vec<f32>>,
    zs: Vec<Vec<f32>>,
    dz: Vec<Vec<f32>>,
}

impl Tape {
    fn new(shape: &NetShape) -> Tape {
        let ls = layers(shape);
        let mut xs: Vec<Vec<f32>> = ls.iter().map(|l| vec![0.0; l.fan_in]).collect();
        xs.push(vec![0.0; ls.last().map(|l| l.fan_out).unwrap_or(0)]);
        let zs = ls.iter().map(|l| vec![0.0; l.fan_out]).collect();
        let dz = ls.iter().map(|l| vec![0.0; l.fan_out]).collect();
        Tape { xs, zs, dz }
    }

    #[inline]
    fn act(&self, l: usize, i: usize) -> f32 {
        self.zs[l][i].tanh()
    }
}

/// One forward pass. `ctx` is the incoming recurrent state (ignored when the shape has none) and
/// the tape's first layer becomes the outgoing one, exactly as [`Genome::forward`] does it.
fn forward(g: &Genome, obs: &[f32], ctx: &[f32], ls: &[Layer], t: &mut Tape) {
    let fb = g.shape.feedback_width();
    {
        let (head, _) = t.xs.split_at_mut(1);
        head[0][..obs.len()].copy_from_slice(obs);
        if fb > 0 {
            head[0][obs.len()..obs.len() + fb].copy_from_slice(&ctx[..fb]);
        }
    }
    for (l, layer) in ls.iter().enumerate() {
        let (head, tail) = t.xs.split_at_mut(l + 1);
        let x = &head[l];
        let out = &mut tail[0];
        let row0 = g.weights[layer.w_off..layer.w_off + layer.fan_in * layer.fan_out].as_ref();
        for r in 0..layer.fan_out {
            let row = &row0[r * layer.fan_in..(r + 1) * layer.fan_in];
            let mut acc = g.biases[layer.b_off + r];
            for (w, xi) in row.iter().zip(x.iter()) {
                acc += *w * *xi;
            }
            t.zs[l][r] = acc;
            out[r] = acc.tanh();
        }
    }
}

/// The memory the clone carries out of one decision, written into `out`: its first layer's
/// activations, zero-padded — the same rule [`Genome::forward`] uses, including the one-layer case
/// where the scores themselves are the memory.
fn write_ctx(g: &Genome, t: &Tape, out: &mut [f32]) {
    let fb = g.shape.feedback_width();
    out.fill(0.0);
    if fb == 0 {
        return;
    }
    let src = &t.xs[1];
    let keep = fb.min(src.len());
    out[..keep].copy_from_slice(&src[..keep]);
}

/// One exemplar's gradient, accumulated into `gw`/`gb`, returning its weighted loss and whether the
/// clone's `argmax` matches the demonstrated action.
#[allow(clippy::too_many_arguments)]
fn backward(
    g: &Genome,
    ls: &[Layer],
    t: &mut Tape,
    target: usize,
    weight: f32,
    gw: &mut [f32],
    gb: &mut [f32],
) -> (f32, bool) {
    let last = ls.len() - 1;
    let mut loss = 0.0f32;
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for r in 0..ls[last].fan_out {
        let a = t.act(last, r);
        if a > best_v {
            best_v = a;
            best = r;
        }
        let y = if r == target { 1.0 } else { -1.0 };
        let d = a - y;
        loss += 0.5 * d * d;
        t.dz[last][r] = weight * d * (1.0 - a * a);
    }
    for l in (0..ls.len()).rev() {
        let layer = &ls[l];
        for r in 0..layer.fan_out {
            let dz = t.dz[l][r];
            gb[layer.b_off + r] += dz;
            let row = layer.w_off + r * layer.fan_in;
            for c in 0..layer.fan_in {
                gw[row + c] += dz * t.xs[l][c];
            }
        }
        if l > 0 {
            let prev_out = ls[l - 1].fan_out;
            for c in 0..prev_out {
                let mut acc = 0.0;
                for r in 0..layer.fan_out {
                    acc += t.dz[l][r] * g.weights[layer.w_off + r * layer.fan_in + c];
                }
                let a = t.act(l - 1, c);
                t.dz[l - 1][c] = acc * (1.0 - a * a);
            }
        }
    }
    (weight * loss, best == target)
}

/// Fit `shape` to a demonstration.
///
/// Errors rather than panicking when the shape or the demonstration cannot carry a fit: a run that
/// asked for a clone and got a silently untrained genome would be worse than one that refused.
pub fn clone_from_play(
    play: &RoutePlay,
    shape: NetShape,
    cfg: &ImitateConfig,
) -> Result<ImitationReport, String> {
    clone_from_play_init(play, shape, cfg, None)
}

/// [`clone_from_play`] continuing from an existing policy rather than the random initialisation.
///
/// This is what makes DAgger and the leg curriculum **refinements** instead of restarts. A refit
/// from scratch at a few hundred passes cannot reproduce the whole route *and* a handful of new
/// corrections; measured on `l11`, every such round made the closed-loop student worse (from leg 1
/// back to leg 0) and the round was discarded. Continuing from the student lets the corrections move
/// only the decisions they disagree with, and the agreement-selected epoch then keeps a round that
/// improves the demonstration's own reproduction.
pub fn clone_from_play_init(
    play: &RoutePlay,
    shape: NetShape,
    cfg: &ImitateConfig,
    init: Option<&Genome>,
) -> Result<ImitationReport, String> {
    shape.validate()?;
    if shape.inputs != N_INPUTS {
        return Err(format!(
            "a clone reads the {} -wide observation, but the shape wants {} inputs",
            N_INPUTS, shape.inputs
        ));
    }
    if play.decisions.is_empty() {
        return Err("nothing to clone: the replay made no decisions".into());
    }
    let ls = layers(&shape);
    let mut t = Tape::new(&shape);

    // Inverse-frequency exemplar weights: see the module docs on the Walk majority.
    let n = play.decisions.len();
    // **Recurrent state starts where an episode starts.** A lesson appended to a DAgger dataset
    // began from a snapshot — a fresh episode — so carrying the demo's final memory into it would
    // train the clone on a state the body never had. `segments` marks those boundaries; the fit
    // resets its context at each one, and the first decision of every run is always a reset.
    let mut resets = vec![false; n];
    if n > 0 {
        resets[0] = true;
        for &start in &play.segments {
            if start < n {
                resets[start] = true;
            }
        }
    }
    let mut counts = [0f32; AgentAction::COUNT];
    for d in &play.decisions {
        counts[d.action.index()] += 1.0;
    }
    let weights: Vec<f32> = play
        .decisions
        .iter()
        .map(|d| {
            let c = counts[d.action.index()].max(1.0);
            (n as f32 / (AgentAction::COUNT as f32 * c)).min(CLASS_WEIGHT_CAP)
        })
        .collect();
    let total_weight: f32 = weights.iter().sum();

    let mut rng = rng_from_seed(cfg.init_seed);
    let mut g = match init {
        Some(init) if init.shape == shape => init.clone(),
        _ => Genome::random(0, shape.clone(), &mut rng),
    };
    g.id = 0;
    // Xavier is the population's own initialisation, and it keeps the clone comparable with a
    // random genome as the *same kind of object*; only the fit differs.
    for v in g.weights.iter_mut().chain(g.biases.iter_mut()) {
        // `Genome::random` already draws in range; this is the ceiling the fit will respect.
        *v = v.clamp(-GENE_CLAMP, GENE_CLAMP);
    }

    let mut gw = vec![0.0f32; g.weights.len()];
    let mut gb = vec![0.0f32; g.biases.len()];

    let mut ctx = vec![0.0f32; shape.feedback_width()];
    let score_epoch = |g: &Genome, ctx: &mut [f32], t: &mut Tape| -> (f32, f32) {
        let mut loss = 0.0f32;
        let mut correct = 0usize;
        ctx.fill(0.0);
        for (i, d) in play.decisions.iter().enumerate() {
            if resets[i] {
                ctx.fill(0.0);
            }
            forward(g, &d.observation, ctx, &ls, t);
            write_ctx(g, t, ctx);
            let target = d.action.index();
            let mut best = 0usize;
            let mut best_v = f32::NEG_INFINITY;
            let mut ex_loss = 0.0f32;
            for r in 0..ls[ls.len() - 1].fan_out {
                let a = t.act(ls.len() - 1, r);
                if a > best_v {
                    best_v = a;
                    best = r;
                }
                let y = if r == target { 1.0 } else { -1.0 };
                let d = a - y;
                ex_loss += 0.5 * d * d;
            }
            if best == target {
                correct += 1;
            }
            loss += weights[i] * ex_loss;
        }
        (loss / total_weight, correct as f32 / n as f32)
    };

    // The untrained baseline, and the number the first update is measured against.
    let (mut best_loss, baseline_agreement) = score_epoch(&g, &mut ctx, &mut t);
    let first_loss = best_loss;
    let mut best = g.clone();
    let mut best_epoch = 0usize;
    let mut best_agreement = baseline_agreement;

    for epoch in 1..=cfg.epochs {
        gw.fill(0.0);
        gb.fill(0.0);
        ctx.fill(0.0);
        for (i, d) in play.decisions.iter().enumerate() {
            if resets[i] {
                ctx.fill(0.0);
            }
            forward(&g, &d.observation, &ctx, &ls, &mut t);
            write_ctx(&g, &t, &mut ctx);
            backward(
                &g,
                &ls,
                &mut t,
                d.action.index(),
                weights[i],
                &mut gw,
                &mut gb,
            );
        }
        for (w, gr) in g.weights.iter_mut().zip(gw.iter()) {
            *w = (*w - cfg.lr * gr / total_weight).clamp(-GENE_CLAMP, GENE_CLAMP);
        }
        for (b, gr) in g.biases.iter_mut().zip(gb.iter()) {
            *b = (*b - cfg.lr * gr / total_weight).clamp(-GENE_CLAMP, GENE_CLAMP);
        }
        // The epoch kept is the epoch with the lowest training loss. **A criterion of "fewest
        // missed decisions" was measured here and is not better**: it fits the demonstration more
        // closely (a 3000-epoch fit reproduces every one of `l2-first-gap`'s 139 decisions) without
        // making the clone a better *warm start* — in the paired arms of
        // `crates/evo/tests/imitate.rs`, the decision-picked clone and the loss-picked one trade
        // places level by level. What actually decides whether a clone can play is the cadence, not
        // the fit: see this module's docs.
        let (loss, agreement) = score_epoch(&g, &mut ctx, &mut t);
        let improves = if cfg.prefer_agreement {
            agreement > best_agreement
        } else {
            loss < best_loss
        };
        if improves {
            best_loss = loss;
            best_agreement = agreement;
            best = g.clone();
            best_epoch = epoch;
        }
    }

    let (_, agreement) = score_epoch(&best, &mut ctx, &mut t);
    Ok(ImitationReport {
        genome: best,
        exemplars: n,
        epochs: cfg.epochs,
        first_loss,
        final_loss: best_loss,
        agreement,
        baseline_agreement,
        best_epoch,
        mastery: None,
        skills: None,
    })
}

/// Replay a level's route and clone the demonstration in one call: the whole of what `--from-route`
/// does, minus the file I/O.
pub fn from_route(
    level: &Level,
    seed: u64,
    eval: &EvalConfig,
    shape: NetShape,
    cfg: &ImitateConfig,
) -> Result<(RoutePlay, ImitationReport), String> {
    let play = replay(level, seed, eval);
    let report = clone_from_play(&play, shape.clone(), cfg)?;
    Ok((play, report))
}

/// Plan the level, walk the plan, and clone it: the whole of what `--from-plan` does.
///
/// Unlike [`from_route`], this demonstration is written in the policy's own decision blocks. The
/// planner's scripts are already block actions, so a fit that agrees with the demonstration is a
/// fit on the control grid the policy actually has — the cadence mismatch this module documents
/// for [`from_route`] does not exist here.
///
/// A plan that the real stepper cannot walk is refused rather than cloned: a seed built from a
/// failing demonstration is the one kind of warm start that is worse than random.
pub fn from_plan(
    level: &Level,
    seed: u64,
    eval: &EvalConfig,
    shape: NetShape,
    cfg: &ImitateConfig,
    plan: sim::plan::PlanConfig,
) -> Result<(RoutePlay, ImitationReport, sim::plan::TimedRoute), String> {
    // **A plan is the only admissible demonstration, and it has to cover the whole level.** A
    // route walk is not block-aligned: the oracle changes its held action on the tick a press comes
    // due, while a policy only decides on a block boundary, so a prefix recorded from a walk can be
    // played by the walk and not by the policy (the cadence mismatch [`from_route`] documents).
    // Merging such a prefix in front of a plan was measured to produce a 95%-agreement clone that
    // still died on the walk's first spike — and a plan from a mid-level snapshot cannot be cloned
    // into a policy that starts at the spawn at all. So the search must cover the level end to end;
    // if it cannot, there is nothing here a policy can be shown.
    //
    // The compiler is the one place that certifies a route (oracle, timed plan, winning replay) and
    // cuts it into legs, and the leg curriculum needs those legs. Planning through it here rather
    // than beside it is what keeps the demonstration and the leg boundaries from being two
    // different answers to the same question.
    // No oracle here: the planner's route is replayed and must win, and that is the proof a
    // demonstration needs. `sim::route_plan::compile` adds the oracle and is what the window's
    // warm start uses, because it also decides whether the *level* is beatable at all.
    let plan = sim::route_plan::compile_route(level, seed, plan).map_err(|err| err.line())?;
    let (play, report) = from_certified_plan(level, &plan, eval, shape, cfg)?;
    Ok((play, report, plan.timed_route()))
}

/// Clone a route the planner has **already** produced (and [`crate::route_plan::RoutePlan`] has
/// already certified): replay it, refuse it if it does not win, fit a clone, run DAgger with
/// replanning corrections, then spend the remaining budget on the leg the clone fails.
///
/// Split out of [`from_plan`] so the route compiler is paid for once. A window's warm start has to
/// decide *whether* there is a plan before it decides to clone one, and planning twice to answer
/// two questions about the same search would double the startup cost for no extra information.
/// The route is still replayed here — a certified plan is checked by the same rule a fresh one is.
pub fn from_certified_plan(
    level: &Level,
    plan: &RoutePlan,
    eval: &EvalConfig,
    shape: NetShape,
    cfg: &ImitateConfig,
) -> Result<(RoutePlay, ImitationReport), String> {
    let route = plan.timed_route();
    // **The demonstration is the meta-controller's play when it wins.** The controller follows
    // the plan's blocks and overrides only what the plan has no answer for (a shot closing with no
    // route jump coming, a window the route is already holding for), and labels every decision as
    // route / commit / wait / ride for the report. A meta play that does not win is refused and the
    // certified route is the demonstration again: a skill the level does not need must not cost
    // the warm start its correctness. On `l11` the two plays are the same actions — the measured
    // gate in FINDINGS.md records that, rather than claiming a gain.
    let meta = cfg
        .meta
        .then(|| sim::meta::play(level, plan, route.seed, eval.action_repeat))
        .filter(|play| play.won());
    let skills = meta.as_ref().map(|play| play.trace.clone());
    let play = match &meta {
        Some(meta) => RoutePlay {
            decisions: meta
                .events
                .iter()
                .map(|(_, observation, action)| Decision {
                    observation: *observation,
                    action: *action,
                })
                .collect(),
            segments: vec![0],
            outcome: meta.outcome,
            frames: meta.frames,
            seed: meta.seed,
            hops_closed: plan.covered,
            best_block: 0,
            best_stand: None,
            route_hops: Some(plan.hops),
            note: meta.note(),
            trace: vec![meta.note()],
        },
        None => {
            let play = timed_play(level, &route, eval);
            if !play.won() {
                return Err(format!(
                    "the planner's route did not replay to the flag: {}",
                    play.note
                ));
            }
            play
        }
    };
    let fit = ImitateConfig {
        prefer_agreement: true,
        ..*cfg
    };
    // Refits continue from the student, so they need a refinement step rather than a from-scratch
    // one: at the fit's own learning rate a correction round was measured to overshoot the old
    // decisions and lose a leg it had already cleared.
    let refine = ImitateConfig {
        lr: (fit.lr * 0.25).max(1e-3),
        ..fit
    };
    let mut report = clone_from_play(&play, shape.clone(), &fit)?;
    let mut dataset = play.clone();
    let mut best_mastery = measure_mastery(level, &report.genome, plan, eval, route.seed);
    // **DAgger, with the planner as a replanning expert.** A closed-loop behaviour clone leaves
    // the demonstration's corridor at its first mistake and then has no label for the states it
    // actually visits. Each round rolls the *best* student out on the real sim, asks the planner
    // what it would do **from the state the student reached** (a fresh search from that snapshot,
    // not the plan's action for that frame index), appends those labels to the training set, and
    // refits. The set only grows, so a round that produces a worse student still contributes the
    // states that student had to recover from; the returned student is the best one the closed
    // loop measured, not the one the fit liked.
    for _round in 0..cfg.dagger_rounds {
        let extra = replanning_rollout(
            level,
            &report.genome,
            Sim::new(level, route.seed),
            plan,
            &play,
            eval,
            play.decisions.len() + 64,
            cfg,
            route.seed ^ 0xDA6_6E2,
        );
        if extra.is_empty() {
            break;
        }
        dataset.segments.push(dataset.decisions.len());
        dataset.decisions.extend(extra);
        let next = clone_from_play_init(&dataset, shape.clone(), &refine, Some(&report.genome))?;
        let mastery = measure_mastery(level, &next.genome, plan, eval, route.seed);
        // **Legs cleared is the selection rule, not the furthest x.** A student that clears one more
        // crossing has learned something the previous one could not do, even when it then falls back
        // — and `run_episode`'s progress alone cannot see that (it called the round that first
        // cleared leg 1 worse than the one that died inside it, because the death was further right).
        if mastery.rank() > best_mastery.rank() {
            report = next;
            best_mastery = mastery;
            if best_mastery.won {
                break;
            }
        }
    }

    // **The leg curriculum: spend the remaining fit budget where the student is weak.** The whole
    // route's DAgger pass teaches the states a mistake led to, but a clone that dies at the first
    // spike spends none of its rounds on the crossing it never made. Each round takes the first
    // leg the clone did not clear, starts rollouts on the leg **before** it from the state the
    // student actually reaches, labels every divergent state by a fresh local search, and refits
    // from the current student. The student that returns is still the best one the closed loop
    // measured, so a round that teaches nothing is discarded rather than returned.
    for _round in 0..cfg.leg_rounds {
        let Some(weak) = best_mastery.weakest() else {
            break;
        };
        // **Start one leg early, where the student actually is.** Starting exactly on the failing
        // leg gives a lesson of the few blocks the student survives there — on `l11` four, from a
        // turret it does not know is coming — which teaches the death and not the recovery. The leg
        // before it is the approach: the same rollout then contains the run-up, the crossing and
        // the failure, which is the state distribution the correction needs.
        let lesson_leg = weak.saturating_sub(1);
        let start = student_leg_start(level, &report.genome, plan, eval, route.seed, lesson_leg)
            .or_else(|| leg_start(level, plan, lesson_leg, eval));
        let Some(start) = start else {
            break;
        };
        let blocks = plan.legs[weak as usize]
            .end_block
            .saturating_sub(plan.legs[lesson_leg as usize].first_block)
            + 24;
        let mut added = 0usize;
        for k in 0..cfg.leg_rollouts.max(1) {
            let lesson = replanning_rollout(
                level,
                &report.genome,
                start.clone(),
                plan,
                &play,
                eval,
                blocks,
                cfg,
                route.seed ^ (0x1E6_0000 | (weak as u64) << 8 | k as u64),
            );
            if !lesson.is_empty() {
                added += lesson.len();
                dataset.segments.push(dataset.decisions.len());
                dataset.decisions.extend(lesson);
            }
        }
        if added == 0 {
            break;
        }
        let next = clone_from_play_init(&dataset, shape.clone(), &refine, Some(&report.genome))?;
        let mastery = measure_mastery(level, &next.genome, plan, eval, route.seed);
        if mastery.rank() > best_mastery.rank() {
            report = next;
            best_mastery = mastery;
            if best_mastery.won {
                break;
            }
        } else {
            // A round that did not clear the leg it was given is the measurement that the
            // curriculum cannot help at this budget; stop rather than spend the rest of it.
            break;
        }
    }
    report.mastery = Some(best_mastery);
    report.skills = skills;
    Ok((play, report))
}

/// The state in which **the student itself** entered a leg: roll it out from the spawn and stop at
/// the first block whose crossing count is `leg`. `None` when it never gets that far, which is the
/// case the caller answers with the plan's own leg start.
///
/// This is the snapshot a leg lesson should use: the curriculum exists to teach the crossing the
/// policy fails, from the position it fails it in.
fn student_leg_start(
    level: &Level,
    genome: &Genome,
    plan: &RoutePlan,
    eval: &EvalConfig,
    seed: u64,
    leg: u32,
) -> Option<Sim> {
    if leg == 0 {
        return Some(Sim::new(level, seed));
    }
    let repeat = eval.action_repeat.max(1);
    let mut sim = Sim::new(level, seed);
    let mut scratch = ActScratch::new();
    let mut obs = ObservationScratch::new();
    for _ in 0..plan.actions.len() + 64 {
        if sim.outcome().is_finished() {
            return None;
        }
        if sim.stats().route_hops_closed >= leg {
            return Some(sim);
        }
        let observation = sim.observation_into(&mut obs);
        let action = genome.act(&observation, &mut scratch);
        for _ in 0..repeat {
            sim.step(action);
            if sim.outcome().is_finished() {
                break;
            }
        }
    }
    None
}

/// The snapshot at the **start of a plan leg**, rebuilt by replaying the plan's own actions from
/// the spawn. `None` when the plan ends before the leg does (which a certified plan does not) or
/// the leg index is out of range.
///
/// This is the state a leg curriculum practises from: the leg's first block, with the episode
/// clock, hazard phases and mover schedules exactly as the plan met them.
fn leg_start(level: &Level, plan: &RoutePlan, leg: u32, eval: &EvalConfig) -> Option<Sim> {
    let entry = plan.legs.get(leg as usize)?;
    let repeat = eval.action_repeat.max(1);
    let mut sim = Sim::new(level, plan.seed);
    for action in plan.actions.iter().take(entry.first_block) {
        if sim.outcome().is_finished() {
            return None;
        }
        for _ in 0..repeat {
            sim.step(*action);
            if sim.outcome().is_finished() {
                return None;
            }
        }
    }
    Some(sim)
}

/// Roll a student out on the real sim, labelling each state it reaches with the action a **fresh
/// plan from that state** would take.
///
/// This is the difference between Phase 1's teacher and this one. Phase 1 labelled the student's
/// block `i` with the nearest state in the plan's demonstration, which is the plan's answer to
/// "what did the route do around here" — not to "what should I do from here". After a divergence
/// the two are different, and on `l11` they are different in the direction that matters: the
/// nearest demonstrated state behind the first spike says *walk*, while the state the student is
/// actually in needs the jump. Here the label comes from [`sim::plan::plan_from`] on the student's
/// own snapshot, so every correction is the teacher's answer to the student's state.
///
/// Replanning is bounded twice: `cfg.replan_cap` fresh searches per rollout, and a search only
/// when the student is **off the demonstrated corridor**. `nearest_d` at or below
/// [`DIVERGENCE`] means the state is one the demonstration itself visited (the observations are
/// block-aligned, so an on-route body matches exactly), and the demonstrated action is the correct
/// label there — asking the planner again at those states was measured to replan at every block of
/// the corridor while producing the same label, and to spend the whole budget before the failure.
#[allow(clippy::too_many_arguments)]
fn replanning_rollout(
    level: &Level,
    genome: &Genome,
    mut sim: Sim,
    plan: &RoutePlan,
    demonstration: &RoutePlay,
    eval: &EvalConfig,
    blocks: usize,
    cfg: &ImitateConfig,
    seed: u64,
) -> Vec<Decision> {
    let repeat = eval.action_repeat.max(1);
    let plan_cfg = sim::plan::PlanConfig {
        depth: cfg.replan_depth.max(1),
        ..sim::plan::PlanConfig::default()
    };
    let mut scratch = ActScratch::new();
    let mut obs = ObservationScratch::new();
    let mut out = Vec::with_capacity(blocks.min(plan.actions.len() + 64));
    let mut expert: Option<(sim::plan::TimedRoute, sim::meta::MetaController)> = None;
    let mut cursor = 0usize;
    let mut replans = 0usize;
    for _ in 0..blocks {
        if sim.outcome().is_finished() {
            break;
        }
        let observation = sim.observation_into(&mut obs);
        let near = nearest_distance(demonstration, &observation);
        let spent = if cfg.meta {
            expert
                .as_ref()
                .is_none_or(|(_, controller)| controller.finished())
        } else {
            expert
                .as_ref()
                .is_none_or(|(route, _)| cursor >= route.actions.len())
        };
        if near <= DIVERGENCE {
            // On the demonstration's own corridor: its action is the teacher.
            expert = None;
            cursor = 0;
        } else if spent
        {
            expert = None;
            cursor = 0;
            if replans < cfg.replan_cap {
                replans += 1;
                // **One leg, not the level.** The target is the next route crossing, so the
                // search is local even when the student is a whole deck behind the plan; asking
                // for "finish the level" from here would pay for the corridor twice and fail at
                // any realistic depth.
                let target = sim.stats().route_hops_closed + 1;
                if let Some(route) = sim::plan::plan_leg(level, sim.clone(), seed, target, plan_cfg).route
                {
                    // The local route's blocks all belong to this leg; the meta-controller owns
                    // the commit / wait / ride decision over them, so a correction at a hazard is
                    // the controller's answer from this state rather than a raw plan block.
                    let leg = sim.stats().route_hops_closed;
                    let controller = sim::meta::MetaController::from_route(route.actions.clone(), leg);
                    expert = Some((route, controller));
                }
            }
        }
        let advice = match expert.as_mut() {
            // The controller owns the sequencing over a local route when the pass asked for it;
            // otherwise the route's blocks are the teacher, which is the Phase 2 arm.
            Some((_, controller)) if cfg.meta => controller.decide(&sim).0,
            Some((route, _)) if cursor < route.actions.len() => route.actions[cursor],
            _ => nearest_action(demonstration, &observation),
        };
        out.push(Decision {
            observation,
            action: advice,
        });
        let student = genome.act(&observation, &mut scratch);
        for _ in 0..repeat {
            sim.step(student);
            if sim.outcome().is_finished() {
                break;
            }
        }
        // Both arms walk the route's blocks one per decision. The controller advances its own
        // cursor in `decide`; the Phase 2 arm advances this one. **Neither drops the route because
        // the student took a different action**: measured on `l11`, dropping it on the first
        // mismatch halved the correction labels (183 exemplars, the plan's alone) and cost a leg
        // on four of eight seeds. The route is a plan from the *student's* state, so it stays a
        // useful label even when the student ignores it for a block.
        if expert.is_some() && !cfg.meta {
            cursor += 1;
        }
    }
    out
}

/// Squared observation distance at or below which the student is **on the demonstration's own
/// corridor**. The observations are quantised sensor readings taken at block boundaries, so a body
/// following the plan reproduces them exactly (`0.0`); a body a fraction of a tile off already
/// reads several units away, and that is where a correction is worth planning.
const DIVERGENCE: f32 = 0.5;

/// The demonstrated action for the nearest demonstrated state, by the observation the policy
/// reads. The fallback when a rollout has spent its replanning budget.
fn nearest_distance(demonstration: &RoutePlay, observation: &[f32; N_INPUTS]) -> f32 {
    demonstration
        .decisions
        .iter()
        .map(|d| {
            d.observation
                .iter()
                .zip(observation.iter())
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
        })
        .fold(f32::INFINITY, f32::min)
}

fn nearest_action(demonstration: &RoutePlay, observation: &[f32; N_INPUTS]) -> AgentAction {
    let mut best = demonstration.decisions[0].action;
    let mut best_d = f32::INFINITY;
    for d in &demonstration.decisions {
        let mut dist = 0.0f32;
        for (a, b) in observation.iter().zip(d.observation.iter()) {
            let delta = a - b;
            dist += delta * delta;
        }
        if dist < best_d {
            best_d = dist;
            best = d.action;
        }
    }
    best
}

/// **Per-leg mastery of a clone**: roll it out from the spawn on the plan's episode and read the
/// route crossings at every block.
///
/// A plan leg is a span of the route, and a body that has closed `k` crossings is in leg `k`, so
/// the crossing count *is* the leg index — the same identity the telemetry's `best_end_leg` and
/// the plan's `Leg::index` rest on. The first leg without a cleared crossing is where the clone
/// stops being able to play the level, and it is the leg the curriculum gets.
pub fn measure_mastery(
    level: &Level,
    genome: &Genome,
    plan: &RoutePlan,
    eval: &EvalConfig,
    seed: u64,
) -> LegMastery {
    let repeat = eval.action_repeat.max(1);
    let n = plan.legs.len();
    let mut frames = vec![0u32; n];
    let mut sim = Sim::new(level, seed);
    let mut scratch = ActScratch::new();
    let mut obs = ObservationScratch::new();
    let mut hops = 0u32;
    let mut guard = 0usize;
    while !sim.outcome().is_finished() && guard <= plan.actions.len() + 64 {
        guard += 1;
        if n > 0 {
            let leg = (hops as usize).min(n - 1);
            let before = sim.stats().frames;
            let observation = sim.observation_into(&mut obs);
            let action = genome.act(&observation, &mut scratch);
            for _ in 0..repeat {
                sim.step(action);
                if sim.outcome().is_finished() {
                    break;
                }
            }
            frames[leg] += sim.stats().frames - before;
            hops = hops.max(sim.stats().route_hops_closed);
        } else {
            let observation = sim.observation_into(&mut obs);
            let action = genome.act(&observation, &mut scratch);
            for _ in 0..repeat {
                sim.step(action);
                if sim.outcome().is_finished() {
                    break;
                }
            }
        }
    }
    let outcome = sim.outcome();
    let won = outcome == Outcome::Won;
    let end_leg = if n == 0 {
        0
    } else {
        hops.min(n as u32 - 1)
    };
    let legs = (0..n)
        .map(|i| {
            let i32l = i as u32;
            let cleared = if plan.legs[i].hops == 0 {
                won
            } else {
                hops > i32l
            };
            LegSkill {
                leg: i32l,
                reached: hops >= i32l,
                cleared,
                ended: outcome.is_finished() && end_leg == i32l,
                frames: frames[i],
            }
        })
        .collect();
    LegMastery {
        legs,
        outcome,
        frames: sim.stats().frames,
        progress_pixels: sim.stats().progress_pixels,
        end_leg,
        won,
    }
}

/// The **warm set** a run starts from: the clone plus `siblings` mutated relatives, ids 0..=siblings.
///
/// [`warm_population`] is what a run is actually handed: this set, then random genomes for the
/// rest — and that rest is the point, not a convenience.
///
/// **A warm start is a diversity budget, not a seed.** Filling the whole population from the clone
/// (`Trainer::from_champion`, which reheats every slot around one point) measures worse than random
/// on `l5-tightrope`: a clone walks the route and cannot place the press the cadence puts between its
/// decisions (see this module's docs on the cadence), so its basin is a *high-fitness plateau with no
/// win in it*, and a population
/// standing on one plateau has nothing to left of it. The measurement is in the module docs of
/// `--from-route` and in `crates/evo/tests/imitate.rs`: at pop 64 on `l5-tightrope`, six seeds,
/// every-clone-seeded solves **0 of 6** where the random arm solves 5 of 6, while a warm set of
/// eight solves all of them as fast as random does.
///
/// So the default is deliberately small: the clone's job is to hand the run a policy that already
/// walks the level, and the random remainder's job is to keep the search out of the clone's basin.
/// `sigma` and `rate` are the population's own mutation scale, so a sibling is the same kind of step
/// an ordinary child would be.
pub fn warm_set(
    clone: &Genome,
    siblings: usize,
    sigma: f32,
    rate: f32,
    rng: &mut sim::rng::Rng,
) -> Vec<Genome> {
    let mut out = Vec::with_capacity(siblings + 1);
    let mut clone = clone.clone();
    clone.id = 0;
    out.push(clone.clone());
    for slot in 0..siblings {
        let mut sibling = clone.clone();
        sibling.id = slot as u32 + 1;
        crate::genome::mutate(&mut sibling, sigma, rate, rng);
        out.push(sibling);
    }
    out
}

/// A **whole population** for a warm start: the warm set, then random genomes up to `pop_size`.
///
/// Built here rather than left to [`crate::ga::Trainer::resume`]'s own padding for one reason: a
/// sealed trainer takes its population size *from the genomes it is handed*
/// (`GaConfig::sanitized(Some(n))` **sets** `pop_size`, it does not raise it), so handing a run eight
/// warm genomes to be padded does not make a hybrid population of sixty-four — it makes a run of
/// eight. Assembling the population here is what keeps the warm set a *fraction* of a full run, which
/// is the whole design (see [`warm_set`]).
///
pub fn warm_population(
    clone: &Genome,
    pop_size: usize,
    siblings: usize,
    sigma: f32,
    rate: f32,
    rng: &mut sim::rng::Rng,
) -> Vec<Genome> {
    let mut out = warm_set(clone, siblings, sigma, rate, rng);
    out.truncate(pop_size.max(1));
    let shape = clone.shape.clone();
    while out.len() < pop_size {
        let id = out.len() as u32;
        out.push(Genome::random(id, shape.clone(), rng));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::default_shape;

    #[test]
    fn an_empty_demonstration_is_refused_rather_than_fitted() {
        let play = RoutePlay {
            decisions: Vec::new(),
            outcome: sim::Outcome::Stuck,
            frames: 0,
            seed: 0,
            hops_closed: 0,
            best_block: 0,
            best_stand: None,
            route_hops: None,
            note: "test".into(),
            segments: vec![0],
            trace: Vec::new(),
        };
        let err = clone_from_play(&play, default_shape(), &ImitateConfig::default()).unwrap_err();
        assert!(err.contains("no decisions"), "{err}");
    }

    /// **Mastery's index is the crossing count**, and the weak leg is the first one without one.
    /// The finish-leg case is pinned too: a route that reaches the flag without closing every rung
    /// is cleared only by winning, not by standing in it.
    #[test]
    fn mastery_names_the_first_leg_without_a_crossing() {
        let skills = |crossings: u32, won: bool| LegMastery {
            legs: (0..3)
                .map(|i| LegSkill {
                    leg: i,
                    reached: crossings >= i,
                    cleared: if i == 2 {
                        won
                    } else {
                        crossings > i
                    },
                    ended: !won && crossings.min(2) == i,
                    frames: 10,
                })
                .collect(),
            outcome: if won { Outcome::Won } else { Outcome::Died },
            frames: 40,
            progress_pixels: 16.0 * crossings as f32,
            end_leg: crossings.min(2),
            won,
        };
        let none = skills(0, false);
        assert_eq!(none.weakest(), Some(0));
        assert_eq!(none.cleared(), 0);
        let one = skills(1, false);
        assert_eq!(one.weakest(), Some(1));
        assert_eq!(one.cleared(), 1);
        assert_eq!(skills(2, false).weakest(), Some(2), "the finish leg is the gap");
        assert_eq!(skills(2, true).weakest(), None);
        assert!(skills(2, true).rank() > skills(2, false).rank());
        assert!(skills(1, false).rank() > skills(0, false).rank());
        assert!(one.line().contains("1/3 legs cleared"));
        assert!(one.line().contains("leg 1"));
    }

    #[test]
    fn a_shape_that_cannot_read_the_observation_is_refused() {
        let mut shape = default_shape();
        shape.inputs = N_INPUTS - 1;
        let play = RoutePlay {
            decisions: vec![crate::route_play::Decision {
                observation: [0.0; N_INPUTS],
                action: AgentAction::Walk,
            }],
            outcome: sim::Outcome::Stuck,
            frames: 1,
            seed: 0,
            hops_closed: 0,
            best_block: 0,
            best_stand: None,
            route_hops: None,
            note: "test".into(),
            segments: vec![0],
            trace: Vec::new(),
        };
        let err = clone_from_play(&play, shape, &ImitateConfig::default()).unwrap_err();
        assert!(err.contains("inputs"), "{err}");
    }

    #[test]
    fn a_single_demonstrated_action_is_memorised() {
        // A demonstration with one repeated decision: the fit has to reproduce it, and it is the
        // cheapest possible check that the gradient points the right way.
        let mut decisions = Vec::new();
        for i in 0..24 {
            let mut observation = [0.0f32; N_INPUTS];
            observation[0] = i as f32 / 24.0;
            observation[3] = 1.0;
            decisions.push(crate::route_play::Decision {
                observation,
                action: AgentAction::SprintJump,
            });
        }
        let play = RoutePlay {
            decisions,
            outcome: sim::Outcome::Stuck,
            frames: 24,
            seed: 0,
            hops_closed: 0,
            best_block: 0,
            best_stand: None,
            route_hops: None,
            note: "test".into(),
            segments: vec![0],
            trace: Vec::new(),
        };
        let cfg = ImitateConfig {
            epochs: 400,
            lr: 0.1,
            ..ImitateConfig::default()
        };
        let report = clone_from_play(&play, default_shape(), &cfg).expect("fits");
        assert!(
            report.agreement > 0.9,
            "agreement {:.2} over one repeated action",
            report.agreement
        );
        assert!(report.final_loss < report.first_loss);
        assert!(report.skill() > 0.9, "skill {:.2}", report.skill());
    }

    #[test]
    fn the_clone_is_a_function_of_its_seed() {
        let play = RoutePlay {
            decisions: (0..16)
                .map(|i| crate::route_play::Decision {
                    observation: [i as f32 * 0.01; N_INPUTS],
                    action: if i % 2 == 0 {
                        AgentAction::Run
                    } else {
                        AgentAction::Walk
                    },
                })
                .collect(),
            outcome: sim::Outcome::Stuck,
            frames: 16,
            seed: 0,
            hops_closed: 0,
            best_block: 0,
            best_stand: None,
            route_hops: None,
            note: "test".into(),
            segments: vec![0],
            trace: Vec::new(),
        };
        let cfg = ImitateConfig {
            epochs: 20,
            ..ImitateConfig::default()
        };
        let a = clone_from_play(&play, default_shape(), &cfg).expect("fits");
        let b = clone_from_play(&play, default_shape(), &cfg).expect("fits");
        assert_eq!(a.genome.weights, b.genome.weights);
        assert_eq!(a.genome.biases, b.genome.biases);
        assert_eq!(a.agreement, b.agreement);
    }
}
