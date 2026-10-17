//! Fixed-topology genome (§7.1) and its forward pass.
//!
//! A genome is a flat `Vec<f32>` of weights (row-major per layer, layers
//! concatenated) plus a flat `Vec<f32>` of biases, and a [`NetShape`] describing
//! how to slice them. Everything is `f32` and every summation order is fixed, so
//! a genome plus a level plus a seed fully determines an episode.

use serde::{Deserialize, Serialize};
use sim::physics::AgentAction;
use sim::rng::{next_f32, next_gaussian, next_signed_f32, Rng};

/// Observation size, straight from the sensor module so the two can never drift
/// apart: changing it is a `sensor_version` bump in `sim`, which propagates here.
pub const N_INPUTS: usize = sim::sensors::N_INPUTS;

/// Number of discrete macro-actions (§7.3).
pub const N_OUTPUTS: usize = AgentAction::COUNT;

/// Hard clamp on every gene. Without it, a lucky large weight can pin every
/// hidden unit into tanh saturation and the lineage becomes unmutatable.
pub const GENE_CLAMP: f32 = 4.0;

/// Network topology. `hidden` holds one entry per hidden layer.
///
/// `feedback` is the width of the network's **memory**: that many values of its own first
/// hidden layer are fed back into the next decision's input vector, and they are zeroed at
/// the start of an episode. `0` — the default, and the shape every champion trained before
/// this field existed has — is a memoryless feedforward net, whose action is a function of
/// the current observation alone.
///
/// # What memory is for, and what it costs
///
/// With `feedback = 0` the policy cannot count, cannot wait, and cannot commit: two visits to
/// the same body-and-world state with different histories are the same decision, so a genome
/// that must hold a jump for three decision blocks, or stand still until a patrolling walker
/// turns around, can only ever express that as a reflex keyed on something visible *right now*.
/// Feeding the first hidden layer's own activations back in is the smallest change that makes
/// history visible, and it is the same trick at two scales: at `feedback = 1` the net has a
/// leaky integrator (a timer, a filter, a one-bit latch), and at `feedback = hidden[0]` it has
/// the full state of a simple recurrent net.
///
/// It is *not* free. The feedback matrix is `feedback × hidden[0]` genes that have to be
/// searched, an episode's decisions are no longer interchangeable, and a policy that is wrong
/// about its own state can be wrong *repeatedly*. The shipped default is `0` because this pack
/// is solved without it (see `REVIEW.md` §4), and the flag exists so that a level whose
/// observation is genuinely insufficient — one that needs a count or a wait — can be attacked
/// with memory rather than with more reward shaping.
///
/// Determinism is preserved exactly: the state is a pure function of the episode's own
/// decisions, so (genome, level, seed) still determines the episode bit for bit, and an elite
/// re-evaluated next generation starts from a clean state like every other member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetShape {
    pub inputs: usize,
    pub hidden: Vec<usize>,
    pub outputs: usize,
    /// Width of the recurrent state fed back into the next decision. `0` = memoryless.
    /// Defaulted on load so a shape written before this field existed reads back as it was.
    #[serde(default)]
    pub feedback: usize,
}

/// The v1 recipe, at the sensor layout's current width: 96 → 16 (tanh) → 11 (tanh, argmax),
/// memoryless (§7.5).
pub fn default_shape() -> NetShape {
    NetShape {
        inputs: N_INPUTS,
        hidden: vec![16],
        outputs: N_OUTPUTS,
        feedback: 0,
    }
}

/// The widest recurrent state this build will build: the whole first hidden layer, twice
/// over.
///
/// A bound rather than a rule of the mathematics, because `feedback` is `feedback × hidden[0]`
/// extra genes and a command-line typo (`--feedback 100000`) would otherwise be an allocation
/// failure instead of a clamped run. It is enforced twice, on purpose and in two different
/// directions: a *live* shape is clamped to it by [`NetShape::feedback_width`] (a config cannot
/// ask for more than the build will do), and a *file* claiming more is refused by
/// `champion::NetworkSpec::check_compat` (a spec is a claim about genes that exist, and
/// silently shrinking it would reinterpret those genes).
pub const MAX_FEEDBACK: usize = 64;

/// The widest single hidden layer either front-end will let an operator ask for.
///
/// A bound on a *live* shape only, applied by `GaConfig::sanitized`, exactly as
/// [`MAX_FEEDBACK`] is: a file that claims a wider net is refused by
/// `champion::NetworkSpec::check_compat` rather than clamped, because a spec is a claim about
/// genes that exist and shrinking it would reinterpret them. 64 is twice the shipped width and
/// the same size as the memory ceiling, which is where the arithmetic stops being cheap: layer 0
/// of the shipped sensor vector is 96 inputs, so a 64-wide first layer is ~6 400 genes per
/// layer — five times the whole 1 739-gene genome this project trains by default, and no more
/// than a typo away from a sweep that costs ten times what it reads as.
pub const MAX_HIDDEN_WIDTH: usize = 64;

/// The most hidden layers a single command line can ask for.
///
/// The project's whole argument for fixed topology is that a small net wins here (`lib.rs`: the
/// plan's 90 → 16 → 6, and MarI/O's winning network had under a dozen neurons), so the flag this
/// bounds exists to widen a layer, not to grow a tower. Four is the point past which "deep"
/// becomes an experiment of its own, with its own measurement to make, rather than a capacity
/// question — and a fourth layer through `tanh` is already a harder thing to search than the
/// 1 739-gene flat genome every number in this repository was measured on.
pub const MAX_HIDDEN_LAYERS: usize = 4;

impl NetShape {
    /// Number of layers with weights, hidden layers plus the output layer.
    pub fn n_layers(&self) -> usize {
        self.hidden.len() + 1
    }

    /// `(fan_in, fan_out)` for layer `l`.
    ///
    /// Layer 0's fan-in is the observation **plus** the recurrent state: the feedback vector is
    /// concatenated onto the observation rather than mixed into it, so the first hidden layer is
    /// a function of `[obs; memory]` and nothing else has to know memory exists.
    pub fn layer(&self, l: usize) -> (usize, usize) {
        let fan_in = if l == 0 {
            // Clamped, so a shape asking for more memory than this build builds still *slices*
            // its own gene vector consistently: the clamp has to live in one place or the
            // weights and the scratch buffer would disagree about where layer 1 starts.
            self.inputs + self.feedback_width()
        } else {
            self.hidden[l - 1]
        };
        let fan_out = if l < self.hidden.len() {
            self.hidden[l]
        } else {
            self.outputs
        };
        (fan_in, fan_out)
    }

    pub fn n_weights(&self) -> usize {
        (0..self.n_layers()).map(|l| {
            let (i, o) = self.layer(l);
            i * o
        }).sum()
    }

    pub fn n_biases(&self) -> usize {
        (0..self.n_layers()).map(|l| self.layer(l).1).sum()
    }

    pub fn n_genes(&self) -> usize {
        self.n_weights() + self.n_biases()
    }

    /// Widest layer, i.e. the scratch buffer size the forward pass needs.
    ///
    /// The layer-0 input buffer is `inputs + feedback` wide, so the state has to be counted here
    /// as well as in [`NetShape::layer`].
    pub fn max_width(&self) -> usize {
        self.hidden
            .iter()
            .copied()
            .chain(std::iter::once(self.outputs))
            .chain(std::iter::once(self.inputs + self.feedback))
            .chain(std::iter::once(self.feedback))
            .max()
            .unwrap_or(0)
    }

    /// The width of the recurrent state, clamped to what this build will build.
    pub fn feedback_width(&self) -> usize {
        self.feedback.min(MAX_FEEDBACK)
    }

    /// One-line description of the topology: `96>16>11`, with the memory width appended when
    /// there is one.
    ///
    /// Lives beside the shape rather than in a front-end because it is the *only* spelling of a
    /// net that both the trainer's own refusals and the terminal's run header use, and a program
    /// that prints one net and refuses another is a program nobody can debug. It is what makes
    /// "this champion is `96>8>11` and this run asks for `96>32>11`" a sentence an operator can
    /// act on.
    pub fn label(&self) -> String {
        let mut out = self.inputs.to_string();
        for width in &self.hidden {
            out.push('>');
            out.push_str(&width.to_string());
        }
        out.push('>');
        out.push_str(&self.outputs.to_string());
        if self.feedback > 0 {
            out.push_str(&format!(" memory {}", self.feedback));
        }
        out
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.inputs == 0 || self.outputs == 0 {
            return Err("network must have at least one input and one output".into());
        }
        if self.hidden.contains(&0) {
            return Err("hidden layers must be non-empty".into());
        }
        Ok(())
    }
}

/// Reusable forward-pass scratch space.
///
/// One per evaluated genome per thread is enough: the evaluator allocates it
/// once and reuses it for every decision in an episode, so a decision makes no
/// allocations at all.
///
/// It also owns the network's **memory**: `ctx` holds the recurrent state carried from one
/// decision to the next. It starts zeroed ([`ActScratch::new`]) and an episode boundary is a
/// fresh [`ActScratch::reset_context`] — the evaluator gets that for free because
/// `EpisodeRunner` builds one scratch per episode, which is exactly the property that keeps a
/// recurrent policy as reproducible as a memoryless one.
#[derive(Clone, Debug, Default)]
pub struct ActScratch {
    input: Vec<f32>,
    a: Vec<f32>,
    b: Vec<f32>,
    out: Vec<f32>,
    ctx: Vec<f32>,
}

impl ActScratch {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure(&mut self, shape: &NetShape) {
        let w = shape.max_width();
        let fan0 = shape.inputs + shape.feedback_width();
        if self.input.len() < fan0 {
            self.input.resize(fan0, 0.0);
        }
        if self.a.len() < w {
            self.a.resize(w, 0.0);
        }
        if self.b.len() < w {
            self.b.resize(w, 0.0);
        }
        if self.out.len() < shape.outputs {
            self.out.resize(shape.outputs, 0.0);
        }
        // Grown from zero, so a scratch that has never been reset starts with no memory: a
        // `resize` fills with zeros, and an already-live vector is deliberately left alone —
        // the state is the previous decision's, and wiping it here would erase the memory on
        // the first call of every decision after the first.
        // Grown with zeros, so a scratch that has never been reset holds no memory — and an
        // already-live vector is deliberately left alone, because the state *is* the previous
        // decision's; wiping it here would erase the memory on every decision after the first.
        if self.ctx.len() < shape.feedback_width() {
            self.ctx.resize(shape.feedback_width(), 0.0);
        }
    }

    /// Forget everything carried from the previous decision.
    ///
    /// What an episode boundary is, for a network with memory. Callers that run several
    /// episodes through one scratch (a demo loop, a replay that restarts) must call this where
    /// they restart, or the second episode begins holding the tail of the first one's thoughts.
    pub fn reset_context(&mut self) {
        self.ctx.fill(0.0);
    }

    /// The recurrent state as the network last left it. Read by tests and by the network
    /// inspector; never by the policy itself.
    pub fn context(&self) -> &[f32] {
        &self.ctx
    }
}

/// One member of the population.
#[derive(Clone, Debug, PartialEq)]
pub struct Genome {
    pub id: u32,
    pub shape: NetShape,
    /// Row-major per layer, layers concatenated.
    pub weights: Vec<f32>,
    /// One per output of each layer, layers concatenated.
    pub biases: Vec<f32>,
    /// Filled in by the evaluator. Never read by the network.
    pub fitness: f32,
}

impl Genome {
    /// Random genome with Xavier-uniform initialisation.
    ///
    /// Xavier rather than plain uniform: with 96 inputs into 16 tanh units, a
    /// uniform `[-1, 1]` start drives most hidden units straight into saturation
    /// on the very first generation, and a GA with no gradients has a hard time
    /// walking back out of that.
    pub fn random(id: u32, shape: NetShape, rng: &mut Rng) -> Genome {
        let n_w = shape.n_weights();
        let n_b = shape.n_biases();
        let mut weights = Vec::with_capacity(n_w);
        let mut biases = Vec::with_capacity(n_b);
        for l in 0..shape.n_layers() {
            let (fan_in, fan_out) = shape.layer(l);
            let limit = (6.0 / (fan_in + fan_out) as f32).sqrt();
            for _ in 0..fan_in * fan_out {
                weights.push(next_signed_f32(rng) * limit);
            }
            for _ in 0..fan_out {
                biases.push(next_signed_f32(rng) * limit);
            }
        }
        Genome {
            id,
            shape,
            weights,
            biases,
            fitness: 0.0,
        }
    }

    /// All-zero genome. Useful as a control: it must still produce valid actions.
    pub fn zeros(id: u32, shape: NetShape) -> Genome {
        let n_w = shape.n_weights();
        let n_b = shape.n_biases();
        Genome {
            id,
            shape,
            weights: vec![0.0; n_w],
            biases: vec![0.0; n_b],
            fitness: 0.0,
        }
    }

    pub fn n_genes(&self) -> usize {
        self.weights.len() + self.biases.len()
    }

    /// Check that every slice matches the declared shape.
    pub fn validate(&self) -> Result<(), String> {
        self.shape.validate()?;
        let n_w = self.shape.n_weights();
        let n_b = self.shape.n_biases();
        if self.weights.len() != n_w {
            return Err(format!(
                "genome {} has {} weights, shape needs {}",
                self.id,
                self.weights.len(),
                n_w
            ));
        }
        if self.biases.len() != n_b {
            return Err(format!(
                "genome {} has {} biases, shape needs {}",
                self.id,
                self.biases.len(),
                n_b
            ));
        }
        if self.weights.iter().any(|w| !w.is_finite())
            || self.biases.iter().any(|b| !b.is_finite())
        {
            return Err(format!("genome {} contains a non-finite gene", self.id));
        }
        Ok(())
    }

    /// Full forward pass. Results land in the scratch's output buffer; read them
    /// with [`Genome::last_outputs`]. Loop order is fixed, so the summation order
    /// is fixed, so the result is bit-reproducible.
    ///
    /// With `shape.feedback > 0` the pass also *reads and writes the scratch's recurrent
    /// state*: the observation is concatenated with the state carried from the previous
    /// decision, and the first layer's own activations become the state for the next one. The
    /// state is copied from the first layer as soon as that layer is computed rather than at
    /// the end, because a three-or-more-hidden-layer net reuses its scratch buffers and the
    /// first layer's activations are gone by then.
    pub fn forward(&self, inputs: &[f32], s: &mut ActScratch) {
        debug_assert_eq!(inputs.len(), self.shape.inputs);
        s.ensure(&self.shape);
        let ActScratch {
            input,
            a,
            b,
            out,
            ctx,
        } = s;
        let ins = self.shape.inputs;
        let fb = self.shape.feedback_width();
        input[..ins].copy_from_slice(&inputs[..ins]);
        input[ins..ins + fb].copy_from_slice(&ctx[..fb]);

        let n_hidden = self.shape.hidden.len();
        let mut w_off = 0usize;
        let mut b_off = 0usize;
        // `fresh_a` tracks which buffer holds the most recent activations.
        let mut fresh_a = false;

        for l in 0..=n_hidden {
            let (fan_in, fan_out) = self.shape.layer(l);
            let is_last = l == n_hidden;
            if l == 0 {
                // The memory is the first layer's activations — the net's own summary of
                // `[obs; memory]` — so it is the layer whose output is kept. A shape with no
                // hidden layer at all has nothing but its scores to remember, and those are
                // what is fed back.
                let src: &[f32] = if is_last {
                    dense(&input[..fan_in], out, self, w_off, b_off, fan_in, fan_out);
                    &out[..fan_out]
                } else {
                    dense(&input[..fan_in], &mut a[..fan_out], self, w_off, b_off, fan_in, fan_out);
                    fresh_a = true;
                    &a[..fan_out]
                };
                let keep = fb.min(src.len());
                ctx[..keep].copy_from_slice(&src[..keep]);
                for v in &mut ctx[keep..] {
                    *v = 0.0;
                }
            } else if fresh_a {
                if is_last {
                    dense(&a[..fan_in], out, self, w_off, b_off, fan_in, fan_out);
                } else {
                    dense(&a[..fan_in], &mut b[..fan_out], self, w_off, b_off, fan_in, fan_out);
                    fresh_a = false;
                }
            } else if is_last {
                dense(&b[..fan_in], out, self, w_off, b_off, fan_in, fan_out);
            } else {
                dense(&b[..fan_in], &mut a[..fan_out], self, w_off, b_off, fan_in, fan_out);
                fresh_a = true;
            }
            w_off += fan_in * fan_out;
            b_off += fan_out;
        }
    }

    /// Raw outputs of the most recent [`Genome::forward`]. Exposed for the
    /// network inspector (§10 Tier 3) and for tests.
    pub fn last_outputs<'a>(&self, s: &'a ActScratch) -> &'a [f32] {
        &s.out[..self.shape.outputs]
    }

    /// Argmax over the action scores (§7.3). Ties go to the lowest index, so the
    /// mapping is deterministic without needing a tie-break RNG.
    pub fn act(&self, inputs: &[f32], s: &mut ActScratch) -> AgentAction {
        self.forward(inputs, s);
        let outputs = &s.out[..self.shape.outputs];
        let mut best = 0usize;
        for (i, v) in outputs.iter().enumerate().skip(1) {
            if *v > outputs[best] {
                best = i;
            }
        }
        AgentAction::from_index(best)
    }

    /// Action scores as a plain vector. Convenience for tests and tooling.
    pub fn score_actions(&self, inputs: &[f32]) -> Vec<f32> {
        let mut s = ActScratch::new();
        self.forward(inputs, &mut s);
        s.out[..self.shape.outputs].to_vec()
    }
}

/// One dense layer: `out[r] = tanh(b[r] + sum_c W[r][c] * x[c])`.
#[inline]
#[allow(clippy::too_many_arguments)]
fn dense(
    x: &[f32],
    out: &mut [f32],
    g: &Genome,
    w_off: usize,
    b_off: usize,
    fan_in: usize,
    fan_out: usize,
) {
    debug_assert_eq!(x.len(), fan_in);
    debug_assert_eq!(out.len(), fan_out);
    for (r, o) in out.iter_mut().enumerate() {
        let row = &g.weights[w_off + r * fan_in..w_off + (r + 1) * fan_in];
        let mut acc = g.biases[b_off + r];
        for (w_rc, xi) in row.iter().zip(x.iter()) {
            acc += *w_rc * *xi;
        }
        *o = acc.tanh();
    }
}

/// Per-gene Gaussian mutation (§7.5).
///
/// A uniform draw happens for every gene whether or not it mutates, so the RNG
/// consumption is a pure function of the genome length.
pub fn mutate(g: &mut Genome, sigma: f32, rate: f32, rng: &mut Rng) {
    for w in g.weights.iter_mut() {
        if next_f32(rng) < rate {
            *w = (*w + next_gaussian(rng, sigma)).clamp(-GENE_CLAMP, GENE_CLAMP);
        }
    }
    for b in g.biases.iter_mut() {
        if next_f32(rng) < rate {
            *b = (*b + next_gaussian(rng, sigma)).clamp(-GENE_CLAMP, GENE_CLAMP);
        }
    }
}

/// Mutate at most `genes` genes, chosen uniformly over the whole genome.
///
/// **The fine half of a two-scale search.** [`mutate`] is a *coarse* move by
/// construction: at the default rate it redraws five percent of a 1 739-gene genome, so
/// every child is about eighty-six genes away from its parents — a jump, not a step. That
/// is the right shape early, when the policy is nothing like a solution, and the wrong
/// shape at the end, when a lineage is one gesture away from clearing the level: a
/// coarse child of a nearly-good genome is a fresh lottery ticket rather than an
/// improvement on it. This is the same Gaussian step applied to a handful of genes.
///
/// The picks are *with* replacement, so a gene can be hit twice in one call (a double
/// step, which is a legal mutation) and the number of genes actually changed can be
/// smaller than `genes`. Sampling without replacement would cost a shuffle and buy a
/// guarantee nobody needs; what matters here is that the expected number of touched
/// genes is `genes`, not 86.
///
/// RNG consumption is fixed by `genes` and the genome's length, so a caller cannot make
/// the stream depend on which genes happened to be picked.
///
/// Returns the number of genes whose value actually changed, for tests and reporting.
pub fn mutate_few(g: &mut Genome, genes: usize, sigma: f32, rng: &mut Rng) -> usize {
    let n = g.weights.len() + g.biases.len();
    if n == 0 || genes == 0 {
        return 0;
    }
    let mut changed = 0usize;
    for _ in 0..genes {
        let slot = (next_f32(rng) * n as f32) as usize;
        let delta = next_gaussian(rng, sigma);
        let gene = if slot < g.weights.len() {
            &mut g.weights[slot]
        } else {
            &mut g.biases[slot - g.weights.len()]
        };
        let before = *gene;
        *gene = (*gene + delta).clamp(-GENE_CLAMP, GENE_CLAMP);
        if *gene != before {
            changed += 1;
        }
    }
    changed
}

/// The genes that decide *when* the body jumps, in [`mutate_few`]'s slot space (all
/// weights first, then all biases).
///
/// The policy is an argmax over eleven action scores, so a jump begins on the tick the
/// jump actions out-score the moving ones — a **boundary in output space**, and these are
/// the genes that draw it: the output layer's row and bias for every action that holds
/// jump ([`AgentAction::wants_jump`]). Everything upstream decides what the body believes
/// is in front of it; these decide what it does about it, and when. A near-miss at a
/// crossing is a body that jumped a tick early or a hair short — a boundary misplaced by
/// a step — which is why the search that polishes a crossing is a search over these genes
/// while the rest of the genome stays where the run left it. A uniform fine draw reaches
/// them one time in nine; [`mutate_jump_timing`] reaches them every time.
///
/// Derived from the shape and the sim's own action table, never hardcoded: a shape with
/// no hidden layer (or four of them) moves the output rows, and a re-labelled action table
/// moves the set with it.
///
pub fn jump_timing_slots(shape: &NetShape) -> Vec<usize> {
    let n_hidden = shape.hidden.len();
    let mut w_base = 0usize;
    let mut b_base = 0usize;
    for l in 0..n_hidden {
        let (fan_in, fan_out) = shape.layer(l);
        w_base += fan_in * fan_out;
        b_base += fan_out;
    }
    let (fan_in, fan_out) = shape.layer(n_hidden);
    let n_weights = w_base + fan_in * fan_out;
    let mut slots = Vec::new();
    for (j, action) in AgentAction::ALL.iter().enumerate().take(fan_out) {
        if !action.wants_jump() {
            continue;
        }
        for k in 0..fan_in {
            slots.push(w_base + j * fan_in + k);
        }
        slots.push(n_weights + b_base + j);
    }
    slots
}

/// A **jump-timing** child: `g` with at most `genes` of its jump-decision genes
/// ([`jump_timing_slots`]) moved by their own Gaussian step of `sigma`, and every other
/// gene copied exactly.
///
/// The fine step of [`mutate_few`], aimed at the boundary that the crossing is actually
/// stuck on. A run that has reached a ledge and jumps too late does not need a new
/// strategy — it needs the same strategy with the take-off a tick earlier, and three
/// genes moved in the jump rows is the smallest mutation that can say that. Nothing else
/// about the approach is risked, which is the point: the parent earned its place up to
/// the hop, and the child is that parent's attempt at the hop's timing.
///
/// The genome's `id` is carried over and the fitness cleared; the caller owns ids, exactly
/// as for [`reheat`] and [`crossover`].
///
pub fn mutate_jump_timing(g: &Genome, genes: usize, sigma: f32, rng: &mut Rng) -> Genome {
    let mut child = g.clone();
    child.fitness = 0.0;
    let slots = jump_timing_slots(&child.shape);
    if slots.is_empty() || genes == 0 {
        return child;
    }
    for _ in 0..genes {
        let pick = (next_f32(rng) * slots.len() as f32) as usize;
        let slot = slots[pick.min(slots.len() - 1)];
        let delta = next_gaussian(rng, sigma);
        if slot < child.weights.len() {
            child.weights[slot] = (child.weights[slot] + delta).clamp(-GENE_CLAMP, GENE_CLAMP);
        } else {
            let b = slot - child.weights.len();
            child.biases[b] = (child.biases[b] + delta).clamp(-GENE_CLAMP, GENE_CLAMP);
        }
    }
    child
}

/// A **reheated** copy of a genome: every gene moved by its own Gaussian step of `sigma`.
///
/// **The third scale of mutation, and the only one that is not a descendant.** [`mutate`] is
/// a coarse step from a parent *in the population*, [`mutate_few`] is a fine step from the
/// generation's best, and both stay where the population already is. A reheat is the move for
/// a plateau: it walks a long way from the best point the run has found while keeping that
/// point's *shape* — the signs and relative magnitudes of 1 739 genes survive a step of a few
/// tenths, where a fresh random genome throws all of it away. The population gets a batch of
/// these when the trainer decides it is stuck (see `ga::Trainer::breed`), so the run explores
/// the neighbourhood of its champion at a radius it could not reach with its ordinary step,
/// instead of restarting from noise.
///
/// Every gene moves, which is what separates it from [`mutate`]: at the default rate a coarse
/// child touches five percent of the genome, so most of it is copied verbatim from one parent.
/// The intent here is a broad shake, not a sparse one.
///
/// The genome's `id` is carried over and the fitness is cleared; the caller owns ids (they are
/// part of every `episode_seed`), exactly as it does for [`crossover`].
///
pub fn reheat(g: &Genome, sigma: f32, rng: &mut Rng) -> Genome {
    let mut child = g.clone();
    for w in child.weights.iter_mut() {
        *w = (*w + next_gaussian(rng, sigma)).clamp(-GENE_CLAMP, GENE_CLAMP);
    }
    for b in child.biases.iter_mut() {
        *b = (*b + next_gaussian(rng, sigma)).clamp(-GENE_CLAMP, GENE_CLAMP);
    }
    child.fitness = 0.0;
    child
}

/// Uniform per-weight crossover (§7.5). `a` and `b` must share a shape.
pub fn crossover(a: &Genome, b: &Genome, rng: &mut Rng) -> Genome {
    debug_assert_eq!(a.shape, b.shape);
    let mut child = a.clone();
    for i in 0..child.weights.len() {
        if next_f32(rng) < 0.5 {
            child.weights[i] = b.weights[i];
        }
    }
    for i in 0..child.biases.len() {
        if next_f32(rng) < 0.5 {
            child.biases[i] = b.biases[i];
        }
    }
    child.fitness = 0.0;
    child
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::rng::rng_from_seed;

    #[test]
    fn default_shape_matches_the_plan() {
        let s = default_shape();
        // v10: 100 inputs (the plan's 90, plus wall side/wall slide/sprint charge from v2,
        // plus the goal's vertical component and the leftward gap probe, plus the nearest
        // walker's heading, plus the route's next waypoint, plus the body's sub-tile phase)
        // and eleven outputs
        // (the plan's six, plus sprint, sprint_jump, retreat_jump and the two leftward
        // sprinting arms). The pin is the *recipe* — one hidden layer of sixteen,
        // argument-max output — with the two ends read from the sim so this test cannot
        // disagree with the action table or the sensor layout.
        assert_eq!(s.inputs, N_INPUTS);
        // The recipe's own width, written down: 81 patch cells plus nineteen scalars.
        assert_eq!(s.inputs, 100);
        assert_eq!(s.hidden, vec![16]);
        assert_eq!(s.outputs, N_OUTPUTS);
        assert_eq!(s.outputs, 11);
        // Memoryless, which is the recipe the plan describes and what every champion trained
        // before `feedback` existed is: the flag exists, the default does not use it.
        assert_eq!(s.feedback, 0);
        assert_eq!(s.feedback_width(), 0);
        assert_eq!(s.n_weights(), N_INPUTS * 16 + 16 * 11);
        assert_eq!(s.n_biases(), 16 + 11);
        assert_eq!(s.n_genes(), N_INPUTS * 16 + 16 * 11 + 16 + 11);
        assert_eq!(s.n_genes(), 1803, "the recipe's own gene count, written down");
        s.validate().expect("shape is valid");
    }

    /// A shape that asks for memory buys exactly `feedback x hidden[0]` genes, and the width is
    /// clamped to what the build builds rather than refusing the shape.
    #[test]
    fn memory_costs_a_feedback_matrix_of_genes_and_the_width_is_bounded() {
        let mut s = default_shape();
        s.feedback = 8;
        assert_eq!(
            s.layer(0),
            (N_INPUTS + 8, 16),
            "layer 0 reads the observation and the state"
        );
        assert_eq!(s.n_weights(), (N_INPUTS + 8) * 16 + 16 * 11);
        assert_eq!(s.n_biases(), 16 + 11);
        assert_eq!(s.n_genes(), (N_INPUTS + 8) * 16 + 16 * 11 + 27);
        assert_eq!(
            s.max_width(),
            N_INPUTS + 8,
            "the layer-0 input buffer is the widest thing"
        );
        assert_eq!(s.feedback_width(), 8);
        s.validate().expect("a shape with memory is valid");

        s.feedback = MAX_FEEDBACK + 1000;
        assert_eq!(s.feedback_width(), MAX_FEEDBACK, "a live shape is clamped");
        assert_eq!(s.layer(0).0, N_INPUTS + MAX_FEEDBACK);
        // The weights and the scratch agree, which is the property the clamp exists for: a
        // genome built from this shape must still be sliceable by it.
        let mut rng = rng_from_seed(21);
        let g = Genome::random(1, s.clone(), &mut rng);
        g.validate().expect("a clamped shape still builds a consistent genome");
    }

    /// The whole point: an observation repeated is not necessarily the same decision, and a
    /// memoryless net cannot tell the two apart.
    #[test]
    fn memory_makes_the_second_visit_to_the_same_state_a_different_decision() {
        let obs = [0.35f32; N_INPUTS];

        let mut rng = rng_from_seed(23);
        let memoryless = Genome::random(1, default_shape(), &mut rng);
        let mut s = ActScratch::new();
        let first = memoryless.score_actions(&obs);
        let second = memoryless.act(&obs, &mut s);
        assert_eq!(
            memoryless.score_actions(&obs),
            first,
            "without memory the same observation is the same decision, however it was reached"
        );
        assert_eq!(second, memoryless.act(&obs, &mut s));

        let mut shape = default_shape();
        shape.feedback = 8;
        let mut rng = rng_from_seed(24);
        let remembering = Genome::random(2, shape, &mut rng);
        let mut s = ActScratch::new();
        let cold = remembering.score_actions(&obs);
        assert!(
            s.context().is_empty(),
            "a fresh scratch holds no thoughts"
        );
        remembering.forward(&obs, &mut s);
        let state_a = s.context().to_vec();
        assert_eq!(state_a.len(), 8);
        assert!(state_a.iter().any(|v| *v != 0.0), "the state is the layer's output");
        remembering.forward(&obs, &mut s);
        let warm = s.context().to_vec();
        assert_ne!(warm, state_a, "the state moved, so the next decision differs");
        assert_ne!(
            s.out[..N_OUTPUTS].to_vec(),
            cold,
            "the same observation, one state later, scores differently"
        );

        // And forgetting puts it back exactly where it started.
        s.reset_context();
        assert!(s.context().iter().all(|v| *v == 0.0));
        remembering.forward(&obs, &mut s);
        assert_eq!(
            s.out[..N_OUTPUTS].to_vec(),
            cold,
            "a reset episode is the episode it was the first time"
        );
    }

    /// Replay fidelity, which is the invariant the whole project rests on: a sequence of
    /// observations through a *fresh* scratch must give the sequence it gave before, whatever
    /// the first scratch did in between.
    #[test]
    fn a_recurrent_policy_replays_its_episode_identically_from_a_clean_scratch() {
        let mut shape = default_shape();
        shape.feedback = 16;
        let mut rng = rng_from_seed(25);
        let g = Genome::random(4, shape, &mut rng);
        let episode: Vec<[f32; N_INPUTS]> = (0..40)
            .map(|i| {
                let mut o = [0.0f32; N_INPUTS];
                for (j, v) in o.iter_mut().enumerate() {
                    *v = ((i * 31 + j * 7) % 13) as f32 / 13.0 - 0.5;
                }
                o
            })
            .collect();

        let mut a = ActScratch::new();
        let run_a: Vec<AgentAction> = episode.iter().map(|o| g.act(o, &mut a)).collect();

        // A scratch that has been used for something else entirely, then reset, must behave
        // like a new one — the difference between remembering and remembering the wrong thing.
        let mut b = ActScratch::new();
        for o in episode.iter().take(5) {
            g.act(o, &mut b);
        }
        b.reset_context();
        let run_b: Vec<AgentAction> = episode.iter().map(|o| g.act(o, &mut b)).collect();
        assert_eq!(run_a, run_b);

        // A scratch that was *not* reset is a different policy, which is what makes the reset
        // meaningful rather than decorative — and the claim is made on the **scores**, not on
        // the action sequence, because that is where the mechanism acts. An eleven-way argmax
        // can be robust to a large perturbation of the scores underneath it (this fixture's is:
        // every score moves by more than half a tenth and the winner does not change), so a
        // test that pinned the *actions* would be testing the draw rather than the memory —
        // which is exactly how this assertion broke when v8 widened the observation from 96
        // slots to 98 and re-rolled the genome.
        let mut warm = ActScratch::new();
        for _ in 0..8 {
            g.act(&[1.0f32; N_INPUTS], &mut warm);
        }
        assert!(
            warm.context().iter().any(|v| *v != 0.0),
            "eight decisions must leave something in memory"
        );
        let mut cold = ActScratch::new();
        assert!(cold.context().iter().all(|v| *v == 0.0));

        g.forward(&episode[0], &mut warm);
        let warm_scores = warm.out[..N_OUTPUTS].to_vec();
        g.forward(&episode[0], &mut cold);
        let cold_scores = cold.out[..N_OUTPUTS].to_vec();

        // The same observation, two histories. Every score differs (the recurrent block of the
        // first layer is real weights, not zeros, so a nonzero state reaches every output) and
        // most of them move far enough to matter to a decision rather than only in the last
        // bit of a float.
        assert!(
            warm_scores
                .iter()
                .zip(&cold_scores)
                .all(|(w, c)| w != c),
            "a carried state of {:?} left some output score untouched: {warm_scores:?}\n{cold_scores:?}",
            warm.context()
        );
        let moved = warm_scores
            .iter()
            .zip(&cold_scores)
            .filter(|(w, c)| (*w - *c).abs() > 0.05)
            .count();
        assert!(
            moved >= N_OUTPUTS / 2,
            "only {moved} of {N_OUTPUTS} decision scores moved by more than 0.05; \
             warm {warm_scores:?} against cold {cold_scores:?}"
        );
    }

    #[test]
    fn a_random_genome_validates_and_acts() {
        let mut rng = rng_from_seed(1);
        let g = Genome::random(7, default_shape(), &mut rng);
        g.validate().expect("valid");
        let mut s = ActScratch::new();
        let obs = [0.0f32; N_INPUTS];
        let a = g.act(&obs, &mut s);
        assert!(AgentAction::ALL.contains(&a));
        assert!(g.last_outputs(&s).iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }

    #[test]
    fn the_zero_genome_is_a_valid_but_arbitrary_policy() {
        let g = Genome::zeros(0, default_shape());
        g.validate().expect("valid");
        let outputs = g.score_actions(&[0.0f32; N_INPUTS]);
        // All scores are tanh(0) == 0, so the tie-break decides: action index 0.
        assert_eq!(outputs, vec![0.0; N_OUTPUTS]);
        let mut s = ActScratch::new();
        assert_eq!(g.act(&[0.0f32; N_INPUTS], &mut s), AgentAction::Run);
    }

    #[test]
    fn jump_timing_mutations_touch_only_the_jump_decision_genes() {
        // The slot set pinned by hand against the action table: the output layer of the
        // default recipe starts at 16 × 100 = 1 600 weights and its row for action `j` is
        // sixteen weights at `1 600 + 16j`, with biases at `1 776 + 16 + j` after the layer's
        // 176 weights and the hidden layer's 16 biases. Jump actions are 1, 3, 5, 7 and 10
        // (run_jump, sprint_jump, retreat_jump, jump_in_place, retreat_sprint_jump).
        let shape = default_shape();
        let mut expect: Vec<usize> = Vec::new();
        for j in [1usize, 3, 5, 7, 10] {
            for k in 0..16 {
                expect.push(16 * N_INPUTS + j * 16 + k);
            }
            expect.push(16 * N_INPUTS + 176 + 16 + j);
        }
        expect.sort_unstable();
        let mut slots = jump_timing_slots(&shape);
        slots.sort_unstable();
        assert_eq!(slots, expect, "the jump rows, by hand");

        // A child moves one to three genes and every one of them is in that set.
        let mut rng = rng_from_seed(5);
        let anchor = Genome::random(1, shape.clone(), &mut rng);
        let mut rng = rng_from_seed(6);
        for genes in 1..=3usize {
            let child = mutate_jump_timing(&anchor, genes, 0.30, &mut rng);
            let mut moved: Vec<usize> = (0..anchor.weights.len())
                .filter(|&i| anchor.weights[i] != child.weights[i])
                .collect();
            let n_w = anchor.weights.len();
            moved.extend(
                (0..anchor.biases.len())
                    .filter(|&i| anchor.biases[i] != child.biases[i])
                    .map(|i| n_w + i),
            );
            assert!(!moved.is_empty(), "a timing child moves something");
            assert!(moved.len() <= genes, "at most {genes} genes moved");
            for m in &moved {
                assert!(expect.contains(m), "gene {m} is not a jump-decision gene");
            }
        }
    }

    #[test]
    fn forward_is_reproducible_across_scratch_reuse() {
        let mut rng = rng_from_seed(9);
        let g = Genome::random(1, default_shape(), &mut rng);
        let obs: Vec<f32> = (0..N_INPUTS).map(|i| (i as f32 * 0.07).sin()).collect();
        let mut s = ActScratch::new();
        let first = g.score_actions(&obs);
        // Reuse the same scratch for a different input, then come back.
        g.forward(&[1.0f32; N_INPUTS], &mut s);
        g.forward(&obs, &mut s);
        assert_eq!(first, s.out[..N_OUTPUTS].to_vec());
    }

    #[test]
    fn mutation_consumes_rng_uniformly_and_stays_bounded() {
        let mut rng = rng_from_seed(3);
        let mut g = Genome::random(1, default_shape(), &mut rng);
        for _ in 0..200 {
            mutate(&mut g, 5.0, 0.5, &mut rng);
        }
        g.validate().expect("still valid");
        assert!(
            g.weights.iter().all(|w| w.abs() <= GENE_CLAMP),
            "clamp must hold"
        );
        assert!(g.weights.iter().any(|w| *w != 0.0));
    }

    #[test]
    fn mutation_is_a_no_op_at_zero_rate_but_still_advances_the_rng() {
        let mut rng = rng_from_seed(4);
        let mut g = Genome::random(1, default_shape(), &mut rng);
        let before = g.clone();
        mutate(&mut g, 0.3, 0.0, &mut rng);
        assert_eq!(g.weights, before.weights);
        assert_eq!(g.biases, before.biases);
        // The RNG must still have moved, or two children of the same parent would
        // be clones of each other forever.
        let probe = rng.clone();
        let mut r2 = rng_from_seed(4);
        let mut g2 = Genome::random(1, default_shape(), &mut r2);
        mutate(&mut g2, 0.0, 1.0, &mut r2);
        assert!(g2.weights.iter().any(|w| *w != 0.0));
        let _ = probe;
    }

    #[test]
    fn a_reheat_moves_every_gene_and_keeps_the_genomes_shape() {
        // Every gene, unlike `mutate` which touches five percent of them at the default rate:
        // the point of a reheat is a broad shake, not a sparse one.
        let mut rng = rng_from_seed(17);
        let parent = Genome::random(3, default_shape(), &mut rng);
        let mut child_rng = rng_from_seed(18);
        let child = reheat(&parent, 0.30, &mut child_rng);

        assert_eq!(child.id, parent.id, "the caller owns ids");
        assert_eq!(child.shape, parent.shape);
        assert_eq!(child.fitness, 0.0, "a reheat is unevaluated");
        let moved = child
            .weights
            .iter()
            .zip(&parent.weights)
            .chain(child.biases.iter().zip(&parent.biases))
            .filter(|(c, p)| c != p)
            .count();
        assert_eq!(
            moved,
            child.weights.len() + child.biases.len(),
            "a reheat must touch every gene"
        );
        child.validate().expect("still a valid genome");

        // And it keeps the parent's shape: the sign of each gene survives at a rate far above
        // chance, where a fresh random genome sits at chance. This *is* the mechanism — the
        // reason to walk away from the best point found rather than discard it.
        let shape_kept = |of: &Genome| {
            let mut same = 0usize;
            let mut total = 0usize;
            for (a, b) in of
                .weights
                .iter()
                .zip(&parent.weights)
                .chain(of.biases.iter().zip(&parent.biases))
            {
                total += 1;
                if a.signum() == b.signum() {
                    same += 1;
                }
            }
            same as f32 / total as f32
        };
        let stranger = Genome::random(4, default_shape(), &mut rng_from_seed(19));
        assert!(
            shape_kept(&child) > shape_kept(&stranger) + 0.1,
            "a reheat ({:.3}) is no more like its parent than a stranger is ({:.3})",
            shape_kept(&child),
            shape_kept(&stranger)
        );

        // The same seed and the same parent must give the same child (§5.6).
        let again = reheat(&parent, 0.30, &mut rng_from_seed(18));
        assert_eq!(again.weights, child.weights);
        assert_eq!(again.biases, child.biases);
    }

    #[test]
    fn crossover_takes_every_gene_from_one_parent_or_the_other() {
        let mut rng = rng_from_seed(5);
        let a = Genome::random(1, default_shape(), &mut rng);
        let b = Genome::random(2, default_shape(), &mut rng);
        let child = crossover(&a, &b, &mut rng);
        // Same id/shape scaffolding as `a`, but genes may come from either.
        assert_eq!(child.id, a.id);
        assert_eq!(child.weights.len(), a.weights.len());
        assert!(child
            .weights
            .iter()
            .enumerate()
            .all(|(i, w)| *w == a.weights[i] || *w == b.weights[i]));
        assert!(child
            .weights
            .iter()
            .enumerate()
            .any(|(i, w)| *w == b.weights[i]));
    }

    #[test]
    fn validate_rejects_a_truncated_genome() {
        let mut rng = rng_from_seed(6);
        let mut g = Genome::random(1, default_shape(), &mut rng);
        g.weights.pop();
        assert!(g.validate().is_err());
    }

    #[test]
    fn a_genome_can_be_used_as_a_replayable_policy_from_its_own_clone() {
        let mut rng = rng_from_seed(11);
        let g = Genome::random(1, default_shape(), &mut rng);
        let obs: Vec<f32> = (0..N_INPUTS).map(|i| ((i * 13) % 7) as f32 / 7.0).collect();
        let clone = g.clone();
        let mut sa = ActScratch::new();
        let mut sb = ActScratch::new();
        assert_eq!(g.act(&obs, &mut sa), clone.act(&obs, &mut sb));
    }
}
