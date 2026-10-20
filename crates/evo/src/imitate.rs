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
//! # What the fit returns
//!
//! The best epoch by training loss is kept ([`ImitationReport::best_epoch`]), because a fit this
//! small is not monotone — the last epoch is not reliably the best one — and [`agreement`] is
//! reported both for the clone and for its untrained initialisation, so "the fit learned something"
//! is a number and not a claim. [`agreement`]: ImitationReport::agreement

use sim::physics::AgentAction;
use sim::rng::rng_from_seed;
use sim::sensors::N_INPUTS;
use sim::{Level, ObservationScratch, Sim};

use crate::fitness::{run_episode, EvalConfig};
use crate::genome::{ActScratch, Genome, NetShape, GENE_CLAMP};
use crate::route_play::{replay, timed_play, RoutePlay};

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
        format!(
            "clone: {} exemplars, {} epochs (best {}), agreement {:.1}% ({:.1}% untrained), loss {:.4} -> {:.4}",
            self.exemplars,
            self.epochs,
            self.best_epoch,
            self.agreement * 100.0,
            self.baseline_agreement * 100.0,
            self.first_loss,
            self.final_loss,
        )
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
    let mut g = Genome::random(0, shape.clone(), &mut rng);
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
    let outcome = sim::plan::plan_level(level, seed, plan);
    let route = outcome
        .route
        .ok_or_else(|| format!("no timed route was found: {}", outcome.note))?;
    let play = timed_play(level, &route, eval);
    if !play.won() {
        return Err(format!(
            "the planner's route did not replay to the flag: {}",
            play.note
        ));
    }
    let fit = ImitateConfig {
        prefer_agreement: true,
        ..*cfg
    };
    let mut report = clone_from_play(&play, shape.clone(), &fit)?;
    // **DAgger, with the plan as a time-indexed expert.** A closed-loop behaviour clone leaves
    // the demonstration's corridor at its first mistake and then has no label for the states it
    // actually visits. Each round rolls the *best* student out on the real sim, labels every state
    // it reached with the plan's action for the nearest demonstrated state, appends that to the
    // training set, and refits. The set only grows, so a round that produces a worse student still
    // contributes the states that student had to recover from; the returned student is the best
    // one the *closed loop* measured, not the one the fit liked.
    let mut dataset = play.clone();
    let mut best_solo = run_episode(level, &report.genome, eval, route.seed);
    for _round in 0..6 {
        let extra = rollout_states(level, &report.genome, &dataset, eval);
        if extra.is_empty() {
            break;
        }
        dataset.decisions.extend(extra);
        let next = clone_from_play(&dataset, shape.clone(), &fit)?;
        let solo = run_episode(level, &next.genome, eval, route.seed);
        let better = solo.won()
            || (!best_solo.won()
                && (solo.progress_pixels > best_solo.progress_pixels
                    || (solo.progress_pixels == best_solo.progress_pixels
                        && solo.frames > best_solo.frames)));
        if better {
            best_solo = solo;
            report = next;
            if best_solo.won() {
                break;
            }
        }
    }
    Ok((play, report, route))
}

/// The states a student actually visits while following a plan, labelled by the **nearest**
/// expert state. Empty when the student never made a decision.
///
/// The plan is open loop, so labelling the student's block `i` with the plan's block `i` is only
/// right while the student is on the route. It is not, by definition, after the first mistake:
/// that is the compounding-error problem. Nearest-neighbour in observation space is the cheap
/// trajectory-expert answer — the label is "what the plan did when it was where you are now"
/// — and it uses the same 108-wide vector the student reads, so a body behind the route is
/// labelled at the state it is actually in rather than at the clock it should have been at.
fn rollout_states(
    level: &Level,
    genome: &Genome,
    expert: &crate::route_play::RoutePlay,
    eval: &EvalConfig,
) -> Vec<crate::route_play::Decision> {
    let repeat = eval.action_repeat.max(1);
    let mut sim = Sim::new(level, expert.seed);
    let mut scratch = ActScratch::new();
    let mut obs = ObservationScratch::new();
    let mut out = Vec::new();
    for _ in 0..expert.decisions.len() {
        if sim.outcome().is_finished() {
            break;
        }
        let observation = sim.observation_into(&mut obs);
        let action = genome.act(&observation, &mut scratch);
        // Nearest demonstrated state, by the observation the policy itself reads.
        let mut best = expert.decisions[0].action;
        let mut best_d = f32::INFINITY;
        for d in &expert.decisions {
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
        out.push(crate::route_play::Decision {
            observation,
            action: best,
        });
        for _ in 0..repeat {
            sim.step(action);
            if sim.outcome().is_finished() {
                break;
            }
        }
    }
    out
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
            trace: Vec::new(),
        };
        let err = clone_from_play(&play, default_shape(), &ImitateConfig::default()).unwrap_err();
        assert!(err.contains("no decisions"), "{err}");
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
