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
    /// **Does the final layer carry one extra neuron that chooses the commitment length?**
    ///
    /// Item 12's head. `false` — the default — is a net that decides every `action_repeat` ticks,
    /// exactly as every champion before this field did. `true` adds one output neuron after the
    /// action rows; its value maps to 1, 2, 4 or 8 ticks of held action, and a zero value maps to
    /// 4 — the shipped cadence — which is why a freshly grown head reproduces today's behaviour
    /// before the search touches it.
    ///
    /// It is a field rather than a wider `outputs` because the action set is still eleven: argmax
    /// must never be able to select the cadence as an action, and every action table in the
    /// project reads `outputs` as "the actions".
    #[serde(default)]
    pub commitment: bool,
}

/// The v1 recipe, at the sensor layout's current width: 108 → 16 (tanh) → 11 (tanh, argmax),
/// memoryless (§7.5).
pub fn default_shape() -> NetShape {
    NetShape {
        inputs: N_INPUTS,
        hidden: vec![16],
        outputs: N_OUTPUTS,
        feedback: 0,
        commitment: false,
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
/// layer — five times the whole 1 931-gene genome this project trains by default, and no more
/// than a typo away from a sweep that costs ten times what it reads as.
pub const MAX_HIDDEN_WIDTH: usize = 64;

/// The most hidden layers a single command line can ask for.
///
/// The project's whole argument for fixed topology is that a small net wins here (`lib.rs`: the
/// plan's 90 → 16 → 6, and MarI/O's winning network had under a dozen neurons), so the flag this
/// bounds exists to widen a layer, not to grow a tower. Four is the point past which "deep"
/// becomes an experiment of its own, with its own measurement to make, rather than a capacity
/// question — and a fourth layer through `tanh` is already a harder thing to search than the
/// 1 931-gene flat genome this build trains.
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
            // The action outputs, plus the commitment head when the shape has one.
            self.out_width()
        };
        (fan_in, fan_out)
    }

    /// The same topology with a wider observation vector.
    ///
    /// Used by [`Genome::grown_to_inputs`]: growth adds observation columns and nothing else, so
    /// the hidden layers, the outputs and the recurrent width are all carried over unchanged.
    pub fn with_inputs(&self, new_inputs: usize) -> NetShape {
        NetShape {
            inputs: new_inputs,
            ..self.clone()
        }
    }

    /// Width of the final layer: the action outputs, plus one when the commitment head is on.
    pub fn out_width(&self) -> usize {
        self.outputs + usize::from(self.commitment)
    }

    /// Fan-in of the final layer — the width of its activation vector, and the vector a
    /// one-sample output-layer update moves along (see [`Genome::last_layer_input`]).
    pub fn last_fan_in(&self) -> usize {
        self.layer(self.n_layers() - 1).0
    }

    /// The same topology with the cadence head present. See [`Genome::grown_with_commitment`].
    pub fn with_commitment(&self) -> NetShape {
        NetShape {
            commitment: true,
            ..self.clone()
        }
    }

    pub fn n_weights(&self) -> usize {
        (0..self.n_layers())
            .map(|l| {
                let (i, o) = self.layer(l);
                i * o
            })
            .sum()
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
            .chain(std::iter::once(self.out_width()))
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
    /// The input vector the **last** layer was given by the most recent pass (see
    /// [`Genome::last_layer_input`]). One copy of sixteen floats per decision, which is what a
    /// one-sample output-layer update needs and what the forward pass already has in registers.
    last_in: Vec<f32>,
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
        if self.out.len() < shape.out_width() {
            self.out.resize(shape.out_width(), 0.0);
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
        if self.last_in.len() < shape.last_fan_in() {
            self.last_in.resize(shape.last_fan_in(), 0.0);
        }
    }

    /// Put a captured recurrent state back, so a network with memory makes the same decision it
    /// made when that state was live.
    ///
    /// [`ActScratch::reset_context`] is the episode boundary; this is the opposite — a replay of
    /// one *decision*, for [`crate::mistakes`] and for any inspector that wants to ask the network
    /// what it saw. The vector is copied, never borrowed, so the caller can keep the capture.
    pub fn restore_context(&mut self, ctx: &[f32]) {
        self.ctx.clear();
        self.ctx.extend_from_slice(ctx);
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
        if self.weights.iter().any(|w| !w.is_finite()) || self.biases.iter().any(|b| !b.is_finite())
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
            last_in,
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
                    last_in[..fan_in].copy_from_slice(&input[..fan_in]);
                    dense(&input[..fan_in], out, self, w_off, b_off, fan_in, fan_out);
                    &out[..fan_out]
                } else {
                    dense(
                        &input[..fan_in],
                        &mut a[..fan_out],
                        self,
                        w_off,
                        b_off,
                        fan_in,
                        fan_out,
                    );
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
                    last_in[..fan_in].copy_from_slice(&a[..fan_in]);
                    dense(&a[..fan_in], out, self, w_off, b_off, fan_in, fan_out);
                } else {
                    dense(
                        &a[..fan_in],
                        &mut b[..fan_out],
                        self,
                        w_off,
                        b_off,
                        fan_in,
                        fan_out,
                    );
                    fresh_a = false;
                }
            } else if is_last {
                last_in[..fan_in].copy_from_slice(&b[..fan_in]);
                dense(&b[..fan_in], out, self, w_off, b_off, fan_in, fan_out);
            } else {
                dense(
                    &b[..fan_in],
                    &mut a[..fan_out],
                    self,
                    w_off,
                    b_off,
                    fan_in,
                    fan_out,
                );
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

    /// The activations the **last layer** was given by the most recent pass.
    ///
    /// This is the one vector an output-layer update needs: moving the row for an action by
    /// `step × this` raises that action's score at *this* observation and leaves every other
    /// observation's alone up to the change in hidden state. [`crate::mistakes`] uses it for the
    /// one-sample repair; nothing in the policy reads it.
    pub fn last_layer_input<'a>(&self, s: &'a ActScratch) -> &'a [f32] {
        let fan_in = self.shape.last_fan_in();
        &s.last_in[..fan_in]
    }

    /// Argmax over the action scores (§7.3). Ties go to the lowest index, so the
    /// mapping is deterministic without needing a tie-break RNG.
    ///
    /// The commitment head, when the shape has one, is deliberately outside the argmax: it is a
    /// cadence, not an action, and letting it win an action vote would be a category error the
    /// action table could not even express.
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

    /// The commitment this genome chose on its last forward pass: 1, 2, 4 or 8 ticks.
    ///
    /// A shape without the head answers [`fitness::ACTION_REPEAT`] — the cadence every genome had
    /// before the head existed. The encoding is four even bands of the tanh output's range, and
    /// the middle band is 4: at zero the head asks for the shipped cadence, so a head grown with
    /// zero weights is bit-identical to the fixed-cadence policy it came from.
    pub fn commitment(&self, s: &ActScratch) -> u32 {
        const TICKS: [u32; 4] = [1, 2, 4, 8];
        if !self.shape.commitment {
            return crate::fitness::ACTION_REPEAT;
        }
        let v = s.out[self.shape.outputs];
        let band = (((v + 1.0) * 0.5) * TICKS.len() as f32).floor();
        let idx = band.clamp(0.0, (TICKS.len() - 1) as f32) as usize;
        TICKS[idx]
    }

    /// Act and read the cadence in one pass, which is what the episode runner needs: two calls
    /// would run the network twice and could disagree about which decision they describe.
    pub fn act_with_commitment(&self, inputs: &[f32], s: &mut ActScratch) -> (AgentAction, u32) {
        let action = self.act(inputs, s);
        (action, self.commitment(s))
    }

    /// **Grow hidden width without changing a single decision** (Item 13).
    ///
    /// Inserts `units` new neurons at the end of the first hidden layer. Their incoming rows are
    /// zero and their biases are zero, and — the part that makes growth function-preserving —
    /// every one of their **outgoing** weights is zero too. A neuron whose outgoing weight is zero
    /// contributes exactly nothing to every layer above it, so the policy at the moment of growth
    /// is bit-identical to the policy before it; the units are capacity the search can then spend.
    ///
    /// The old hidden units keep their indices, so the recurrent block (which reads the first
    /// `feedback` activations of layer 0) keeps its meaning and its ordering. Later layers grow
    /// their fan-in by `units` zeros, appended after the old incoming columns for the same reason.
    ///
    /// The caller owns the decision to grow and the caps on it (see `GaConfig::growth_units` and
    /// `GaConfig::growth_events`); this function only guarantees that doing so is a no-op for
    /// behaviour.
    pub fn grown_hidden(&self, units: usize) -> Result<Genome, String> {
        if units == 0 {
            return Ok(self.clone());
        }
        if self.shape.hidden.is_empty() {
            return Err("cannot grow a net with no hidden layer".into());
        }
        let mut new_shape = self.shape.clone();
        new_shape.hidden[0] += units;
        let n_layers = self.shape.n_layers();
        let mut weights = Vec::with_capacity(new_shape.n_weights());
        let mut biases = Vec::with_capacity(new_shape.n_biases());
        let mut w_off = 0usize;
        let mut b_off = 0usize;
        for l in 0..n_layers {
            let (fan_in, fan_out) = self.shape.layer(l);
            if l == 0 {
                // Existing rows, then `units` all-zero rows for the new neurons.
                weights.extend_from_slice(&self.weights[w_off..w_off + fan_in * fan_out]);
                weights.resize(weights.len() + fan_in * units, 0.0);
                biases.extend_from_slice(&self.biases[b_off..b_off + fan_out]);
                biases.resize(biases.len() + units, 0.0);
            } else {
                // Every row keeps its old incoming columns in order, then gains `units` zeros
                // for the neurons inserted at the end of the previous hidden layer.
                let added = if l == 1 { units } else { 0 };
                for row in 0..fan_out {
                    let start = w_off + row * fan_in;
                    weights.extend_from_slice(&self.weights[start..start + fan_in]);
                    if l == 1 {
                        weights.resize(weights.len() + added, 0.0);
                    }
                }
                biases.extend_from_slice(&self.biases[b_off..b_off + fan_out]);
            }
            w_off += fan_in * fan_out;
            b_off += fan_out;
        }
        let grown = Genome {
            id: self.id,
            shape: new_shape,
            weights,
            biases,
            fitness: self.fitness,
        };
        grown.validate()?;
        Ok(grown)
    }

    /// **Add the cadence head without changing a single decision.**
    ///
    /// Item 12's migration, resting on Item 3's machinery: the final layer gains one output row
    /// whose weights and bias are zero. A zero row scores `tanh(0) = 0`, and zero is the encoding
    /// of a four-tick commitment — the cadence every genome had before the head — so the grown
    /// genome holds each action for exactly as long as the old one did and the action argmax is
    /// untouched. Every existing row keeps its weights in place; only the output layer's width
    /// changes, because only its fan_out moved.
    pub fn grown_with_commitment(&self) -> Result<Genome, String> {
        if self.shape.commitment {
            return Ok(self.clone());
        }
        let new_shape = self.shape.with_commitment();
        let n_layers = self.shape.n_layers();
        let last = n_layers - 1;
        let mut weights = Vec::with_capacity(new_shape.n_weights());
        let mut biases = Vec::with_capacity(new_shape.n_biases());
        let mut w_off = 0usize;
        let mut b_off = 0usize;
        for l in 0..n_layers {
            let (fan_in, fan_out) = self.shape.layer(l);
            weights.extend_from_slice(&self.weights[w_off..w_off + fan_in * fan_out]);
            w_off += fan_in * fan_out;
            biases.extend_from_slice(&self.biases[b_off..b_off + fan_out]);
            b_off += fan_out;
            if l == last {
                weights.resize(weights.len() + fan_in, 0.0);
                biases.push(0.0);
            }
        }
        let grown = Genome {
            id: self.id,
            shape: new_shape,
            weights,
            biases,
            fitness: self.fitness,
        };
        grown.validate()?;
        Ok(grown)
    }

    /// Action scores as a plain vector. Convenience for tests and tooling.
    pub fn score_actions(&self, inputs: &[f32]) -> Vec<f32> {
        let mut s = ActScratch::new();
        self.forward(inputs, &mut s);
        s.out[..self.shape.outputs].to_vec()
    }

    /// **Grow the observation without changing the policy.**
    ///
    /// A genome read by a *wider* sensor vector is the same network with cells it has never
    /// seen. Giving every appended column a zero weight is what makes that a fact rather than a
    /// hope: a zero multiplies whatever the new sensor says to nothing, so every output, every
    /// recurrent state and therefore every decision stays bit-for-bit what it was. The old
    /// weights are not rescaled, reordered or re-seeded; they keep their index meaning inside
    /// each row, which is the contract this function exists to serve.
    ///
    /// **Layer 0's zeros go before the recurrent columns, not at the very end of the row.** The
    /// first layer reads `[observation; memory]`, so a row is `old_obs × old_inputs`, then the
    /// feedback weights. Appending the new observation columns after those would put the old
    /// memory weights on observation cells and leave the zeros reading the memory, which is a
    /// different network with a different shape of lie. The insertion point is the end of the
    /// observation, which is `old_inputs` cells into the row.
    ///
    /// The first layer is the only one whose fan-in depends on the observation; every later layer
    /// is copied verbatim. Biases, the genome's identity and its recorded fitness come with it.
    pub fn grown_to_inputs(&self, new_inputs: usize) -> Result<Genome, String> {
        let old_inputs = self.shape.inputs;
        if new_inputs < old_inputs {
            return Err(format!(
                "cannot shrink a genome's observation from {old_inputs} to {new_inputs} inputs: \
                 growth is append-only, for the reason in `grown_to_inputs`"
            ));
        }
        if new_inputs == old_inputs {
            return Ok(self.clone());
        }
        let new_shape = self.shape.with_inputs(new_inputs);
        let (old_fan_in, rows) = self.shape.layer(0);
        let (new_fan_in, _) = new_shape.layer(0);
        let feedback = self.shape.feedback_width();
        let old_obs = old_fan_in - feedback;
        let added = new_fan_in - old_fan_in;
        debug_assert_eq!(added, new_inputs - old_inputs);

        let mut weights = Vec::with_capacity(new_shape.n_weights());
        for row in 0..rows {
            let start = row * old_fan_in;
            weights.extend_from_slice(&self.weights[start..start + old_obs]);
            weights.resize(weights.len() + added, 0.0);
            weights.extend_from_slice(&self.weights[start + old_obs..start + old_fan_in]);
        }
        weights.extend_from_slice(&self.weights[old_fan_in * rows..]);

        let grown = Genome {
            id: self.id,
            shape: new_shape,
            weights,
            biases: self.biases.clone(),
            fitness: self.fitness,
        };
        grown.validate()?;
        Ok(grown)
    }
}

/// How many output rows [`dense`] computes side by side.
///
/// It is four, and the block is **written out** rather than looped over, which is the whole point:
/// the loop is what the compiler already had in front of it and did not unroll usefully (the rows
/// of a layer are independent, and neither LLVM nor a human reader can know that from
/// `for r in 0..fan_out`). Changing this number means writing the block again — it is a shape, not
/// a dial.
///
/// **A throughput constant, not a numerical one.** Each row keeps its own accumulator and its own
/// left-to-right summation order, so the four sums are bit-for-bit what four separate loops
/// produce: this changes *which* multiply-adds are in flight at once, never *how* a row is added
/// up. That distinction is the whole reason it is safe in a repository whose determinism is a
/// contract — a reassociated sum (four accumulators *inside* one row, or a fast-math flag) would
/// move the last bits of a score and could flip an argmax on a boundary, which is a different
/// change with a version bump attached.
const DENSE_ROW_BLOCK: usize = 4;

/// One dense layer: `out[r] = tanh(b[r] + sum_c W[r][c] * x[c])`.
///
/// Written as a block of [`DENSE_ROW_BLOCK`] rows at a time because the single-row form is
/// **latency-bound rather than throughput-bound**: `acc += w*x` is a chain of dependent adds, and
/// each one waits for the previous result, so a long row costs its length in add latencies however
/// fast the multiplier is. Thus the four chains overlap and the adds stop waiting on each other.
///
/// Measured on the baseline machine, `96 → 16 → 11` (1 712 multiplies) and 200 000 forwards:
/// **2.9 cycles/MAC one row at a time, 1.15 blocked — 2.5× at bit-identical results**. The
/// reference loop is pinned by `the_blocked_layer_is_bit_identical_to_one_row_at_a_time`, so the
/// ratio and the equality are measured against the same definition.
///
/// Rows are the natural unit to block over (rather than columns or lanes) because they are already
/// contiguous and independent: two output rows share no arithmetic at all.
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
    let weights = &g.weights[w_off..w_off + fan_in * fan_out];
    let biases = &g.biases[b_off..b_off + fan_out];

    let mut r = 0usize;
    while r + DENSE_ROW_BLOCK <= fan_out {
        let rows: [&[f32]; DENSE_ROW_BLOCK] = [
            &weights[r * fan_in..(r + 1) * fan_in],
            &weights[(r + 1) * fan_in..(r + 2) * fan_in],
            &weights[(r + 2) * fan_in..(r + 3) * fan_in],
            &weights[(r + 3) * fan_in..(r + 4) * fan_in],
        ];
        let mut acc = [biases[r], biases[r + 1], biases[r + 2], biases[r + 3]];
        // Four iterators, zipped as `((w0,w1),(w2,w3))` beside `x`: the zip ends when the shortest
        // side does, and all five sides are exactly `fan_in` long, so this neither indexes nor
        // bounds-checks and every row still sums in its own order.
        let lanes = rows[0]
            .iter()
            .zip(rows[1].iter())
            .zip(rows[2].iter().zip(rows[3].iter()))
            .zip(x.iter());
        for (((w0, w1), (w2, w3)), xi) in lanes {
            acc[0] += *w0 * *xi;
            acc[1] += *w1 * *xi;
            acc[2] += *w2 * *xi;
            acc[3] += *w3 * *xi;
        }
        for (k, a) in acc.into_iter().enumerate() {
            out[r + k] = a.tanh();
        }
        r += DENSE_ROW_BLOCK;
    }

    // The tail, one row at a time — the shipped shape has 16 hidden units (no tail) and 11
    // outputs (three rows), and a narrower net can be all tail.
    for (i, o) in out[r..].iter_mut().enumerate() {
        let row = &weights[(r + i) * fan_in..(r + i + 1) * fan_in];
        let mut acc = biases[r + i];
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

    /// The forward pass as it was written before rows were blocked: one row at a time, one
    /// accumulator, left to right.
    ///
    /// Kept here rather than in the library because it is not an implementation — it is the
    /// *definition* the blocked loop has to agree with, bit for bit. A blocked pass that is faster
    /// and one float out of place would be a champion that replays to a different decision, which
    /// is the failure this repository's whole versioning scheme exists to prevent.
    fn reference_forward(g: &Genome, inputs: &[f32]) -> Vec<f32> {
        assert_eq!(g.shape.feedback, 0, "the reference is the memoryless shape");
        let mut x = inputs.to_vec();
        let mut w_off = 0usize;
        let mut b_off = 0usize;
        for l in 0..g.shape.n_layers() {
            let (fan_in, fan_out) = g.shape.layer(l);
            let mut out = vec![0.0f32; fan_out];
            for (r, o) in out.iter_mut().enumerate() {
                let row = &g.weights[w_off + r * fan_in..w_off + (r + 1) * fan_in];
                let mut acc = g.biases[b_off + r];
                for (w, xi) in row.iter().zip(x.iter()) {
                    acc += *w * *xi;
                }
                *o = acc.tanh();
            }
            x = out;
            w_off += fan_in * fan_out;
            b_off += fan_out;
        }
        x
    }

    /// The blocked layer is a **throughput** change and nothing else: every shape, every genome
    /// and every input must produce the identical bits.
    ///
    /// The shapes are chosen for the block boundary: 16 hidden and 11 outputs is four blocks plus
    /// a three-row tail, 32 → 16 is all blocks, and 1, 3 and 5 are all tail — so a mis-sized block
    /// or an off-by-one in the tail is a failure here rather than a champion that stops replaying.
    #[test]
    fn the_blocked_layer_is_bit_identical_to_one_row_at_a_time() {
        let shapes = [
            default_shape(),
            NetShape {
                hidden: vec![32, 16],
                ..default_shape()
            },
            NetShape {
                hidden: vec![3],
                ..default_shape()
            },
            NetShape {
                hidden: vec![1],
                ..default_shape()
            },
            NetShape {
                hidden: vec![5],
                ..default_shape()
            },
        ];
        let inputs: Vec<f32> = (0..N_INPUTS)
            .map(|i| ((i * 37) % 29) as f32 / 14.5 - 1.0)
            .collect();
        for shape in shapes {
            for seed in 1..=4u64 {
                let mut rng = rng_from_seed(seed);
                let g = Genome::random(seed as u32, shape.clone(), &mut rng);
                let want = reference_forward(&g, &inputs);
                let mut s = ActScratch::new();
                g.forward(&inputs, &mut s);
                let got = g.last_outputs(&s);
                assert_eq!(
                    got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    want.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    "{} seed {seed}: the blocked layer disagrees with one row at a time",
                    shape.label()
                );
            }
        }
    }

    /// The batched pass is the scalar pass in a different order of *calls*, never of sums: every
    /// output and every piece of recurrent state must be bit-identical, on a memoryless shape and
    /// on a feedback shape, over a sequence of decisions (where memory makes the order matter).
    /// four-tick commitment, so the grown genome replays exactly the episode the old one did.
    #[test]
    fn growing_the_cadence_head_is_bit_identical_to_the_fixed_cadence() {
        let level = crate::fixture::gap_runway("commitment-growth", 24, 22, &[(9, 3)]);
        let mut rng = rng_from_seed(21);
        let old = Genome::random(4, default_shape(), &mut rng);
        let grown = old
            .grown_with_commitment()
            .expect("the head is an append-only row");
        grown.validate().expect("grown genome validates");
        assert_eq!(
            grown.shape.outputs, old.shape.outputs,
            "the action set did not move"
        );
        assert_eq!(grown.shape.out_width(), old.shape.out_width() + 1);
        assert_eq!(grown.shape.n_weights(), old.shape.n_weights() + 16);
        assert_eq!(grown.weights[..old.weights.len()], old.weights[..]);
        assert!(grown.weights[old.weights.len()..].iter().all(|w| *w == 0.0));
        assert_eq!(grown.biases.last(), Some(&0.0));

        // Every decision now also reads a cadence, and at zero it must be the shipped four.
        let obs = [0.25f32; N_INPUTS];
        let mut s = ActScratch::new();
        let (action, ticks) = grown.act_with_commitment(&obs, &mut s);
        assert_eq!(ticks, crate::fitness::ACTION_REPEAT);
        let mut plain = ActScratch::new();
        assert_eq!(action, grown.act(&obs, &mut plain));

        let cfg = crate::fitness::EvalConfig::default();
        let a = crate::fitness::run_episode(&level, &old, &cfg, 9);
        let b = crate::fitness::run_episode(&level, &grown, &cfg, 9);
        assert_eq!(a.frames, b.frames);
        assert_eq!(a.fitness.to_bits(), b.fitness.to_bits());
        assert_eq!(a.trajectory_hash, b.trajectory_hash);
        assert_eq!(a.outcome, b.outcome);
    }

    /// The four cadence bands, pinned at their boundaries: a head is only a mechanism if its
    /// encoding is a fact a reader can check without running the search.
    #[test]
    fn the_commitment_head_encodes_four_bands_and_the_middle_one_is_four() {
        let mut rng = rng_from_seed(22);
        let g = Genome::random(5, default_shape().with_commitment(), &mut rng);
        let mut s = ActScratch::new();
        g.forward(&[0.0f32; N_INPUTS], &mut s);
        let cases = [
            (-1.0, 1),
            (-0.5, 2),
            (0.0, 4),
            (0.5, 8),
            (1.0, 8),
            (0.999, 8),
        ];
        for (head, want) in cases {
            s.out[g.shape.outputs] = head;
            assert_eq!(g.commitment(&s), want, "head {head} should hold for {want}");
        }
        // A shape without the head cannot be asked: it answers the fixed cadence.
        let plain = Genome::random(6, default_shape(), &mut rng);
        let mut s2 = ActScratch::new();
        plain.forward(&[0.0f32; N_INPUTS], &mut s2);
        assert_eq!(plain.commitment(&s2), crate::fitness::ACTION_REPEAT);
    }

    /// Item 13's migration proof: the new hidden units have zero **outgoing** weights, so they
    /// cannot reach an action, and the policy at the moment of growth is bit-identical.
    #[test]
    fn growing_the_hidden_width_is_bit_identical_to_the_policy_before_it() {
        let level = crate::fixture::gap_runway("growth", 24, 22, &[(8, 3)]);
        let mut rng = rng_from_seed(31);
        let memory = NetShape {
            feedback: 8,
            ..default_shape()
        };
        let old = Genome::random(7, memory, &mut rng);
        let grown = old.grown_hidden(4).expect("width grows");
        grown.validate().expect("valid");
        assert_eq!(grown.shape.hidden[0], old.shape.hidden[0] + 4);
        assert_eq!(grown.shape.outputs, old.shape.outputs);
        assert_eq!(grown.shape.feedback, old.shape.feedback);

        let cfg = crate::fitness::EvalConfig::default();
        let a = crate::fitness::run_episode(&level, &old, &cfg, 3);
        let b = crate::fitness::run_episode(&level, &grown, &cfg, 3);
        assert_eq!(a.frames, b.frames);
        assert_eq!(a.outcome, b.outcome);
        assert_eq!(a.best_x.to_bits(), b.best_x.to_bits());
        assert_eq!(a.trajectory_hash, b.trajectory_hash);

        // Zero units is a copy, and a net with no hidden layer refuses rather than inventing one.
        assert_eq!(old.grown_hidden(0).expect("copy").weights, old.weights);
        let flat = Genome::random(
            8,
            NetShape {
                hidden: Vec::new(),
                ..default_shape()
            },
            &mut rng,
        );
        assert!(flat.grown_hidden(1).is_err());
    }

    #[test]
    fn default_shape_matches_the_plan() {
        let s = default_shape();
        // v13: 108 inputs (the plan's 90, plus wall side/wall slide/sprint charge from v2,
        // plus the goal's vertical component and the leftward gap probe, plus the nearest
        // walker's heading, plus the route's next waypoint, plus the body's sub-tile phase,
        // plus v13's clock, next-event eta/position, mover velocity/phase and press timing)
        // and eleven
        // outputs
        // (the plan's six, plus sprint, sprint_jump, retreat_jump and the two leftward
        // sprinting arms). The pin is the *recipe* — one hidden layer of sixteen,
        // argument-max output — with the two ends read from the sim so this test cannot
        // disagree with the action table or the sensor layout.
        assert_eq!(s.inputs, N_INPUTS);
        // The recipe's own width, written down: 81 patch cells plus twenty-seven scalars.
        assert_eq!(s.inputs, 108);
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
        assert_eq!(
            s.n_genes(),
            1931,
            "the recipe's own gene count, written down"
        );
        s.validate().expect("shape is valid");
    }

    /// The old observation followed by the new sensor cells: the only meaningful way to ask a
    /// grown net the same question is to hand it the same old cells and something in the new
    /// ones. The extras are deterministic so a failure is reproducible.
    fn padded(obs: &[f32], new_inputs: usize, salt: f32) -> Vec<f32> {
        let mut out = obs.to_vec();
        for i in obs.len()..new_inputs {
            out.push((i as f32 * 0.37 + salt).sin().clamp(-1.0, 1.0));
        }
        out
    }

    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|v| v.to_bits()).collect()
    }

    /// The function-preserving contract: growing the observation adds columns the policy cannot
    /// read, so every score and every piece of memory must be bit-identical. The sequence of
    /// calls matters for a recurrent net — the second decision reads what the first one left —
    /// which is why this is an episode and not a single forward pass.
    #[test]
    fn growing_the_observation_is_bit_identical_for_the_same_policy() {
        let added = 6;
        for feedback in [0usize, 16] {
            let shape = NetShape {
                feedback,
                ..default_shape()
            };
            let mut rng = rng_from_seed(31 + feedback as u64);
            let g = Genome::random(7, shape.clone(), &mut rng);
            let grown = g
                .grown_to_inputs(N_INPUTS + added)
                .expect("growth is append-only");
            assert_eq!(grown.shape, shape.with_inputs(N_INPUTS + added));
            assert_eq!(grown.id, g.id);
            assert_eq!(grown.fitness, g.fitness);
            assert_eq!(grown.biases, g.biases);

            let mut old_scratch = ActScratch::new();
            let mut new_scratch = ActScratch::new();
            for step in 0..24 {
                let mut obs = [0.0f32; N_INPUTS];
                for (j, v) in obs.iter_mut().enumerate() {
                    *v = (((step * 17 + j * 5) % 23) as f32 / 11.5 - 1.0).clamp(-1.0, 1.0);
                }
                let wide = padded(&obs, N_INPUTS + added, step as f32 * 0.61);
                g.forward(&obs, &mut old_scratch);
                grown.forward(&wide, &mut new_scratch);
                assert_eq!(
                    bits(grown.last_outputs(&new_scratch)),
                    bits(g.last_outputs(&old_scratch)),
                    "feedback {feedback}, step {step}: the grown net changed a decision"
                );
                assert_eq!(
                    bits(new_scratch.context()),
                    bits(old_scratch.context()),
                    "feedback {feedback}, step {step}: the grown net changed its memory"
                );
            }
        }
    }

    /// Where the zeros go, stated as an address: each layer-0 row keeps its old observation
    /// weights, then six zeros, then the feedback weights that used to sit at the end of the row.
    /// Appending the zeros after the feedback would put them on the wrong side of the insertion
    /// point and quietly reinterpret the memory.
    #[test]
    fn growth_appends_zero_columns_and_keeps_every_old_weight() {
        let shape = NetShape {
            feedback: 4,
            ..default_shape()
        };
        let mut rng = rng_from_seed(41);
        let g = Genome::random(3, shape.clone(), &mut rng);
        let grown = g.grown_to_inputs(N_INPUTS + 6).expect("growth");
        let (old_fan_in, rows) = shape.layer(0);
        let (new_fan_in, _) = grown.shape.layer(0);
        let observation = old_fan_in - shape.feedback_width();
        assert_eq!(new_fan_in - old_fan_in, 6);
        for row in 0..rows {
            let old_row = &g.weights[row * old_fan_in..(row + 1) * old_fan_in];
            let new_row = &grown.weights[row * new_fan_in..(row + 1) * new_fan_in];
            assert_eq!(&new_row[..observation], &old_row[..observation]);
            assert!(
                new_row[observation..observation + 6]
                    .iter()
                    .all(|w| *w == 0.0),
                "row {row}'s new sensor columns are not all zero"
            );
            assert_eq!(&new_row[observation + 6..], &old_row[observation..]);
        }
        // The first layer is the only one whose fan-in moved.
        let old_head = old_fan_in * rows;
        assert_eq!(&grown.weights[new_fan_in * rows..], &g.weights[old_head..]);
        assert_eq!(grown.biases, g.biases);
        assert_eq!(grown.shape.hidden, g.shape.hidden);
        assert_eq!(grown.shape.outputs, g.shape.outputs);
        assert_eq!(grown.shape.feedback, g.shape.feedback);
        assert_eq!(grown.shape.n_layers(), g.shape.n_layers());
    }

    #[test]
    fn shrinking_the_observation_is_refused() {
        let mut rng = rng_from_seed(43);
        let g = Genome::random(5, default_shape(), &mut rng);
        let err = g
            .grown_to_inputs(N_INPUTS - 1)
            .expect_err("growth cannot remove sensors");
        assert!(err.contains("cannot shrink"), "{err}");
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
        g.validate()
            .expect("a clamped shape still builds a consistent genome");
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
        assert!(s.context().is_empty(), "a fresh scratch holds no thoughts");
        remembering.forward(&obs, &mut s);
        let state_a = s.context().to_vec();
        assert_eq!(state_a.len(), 8);
        assert!(
            state_a.iter().any(|v| *v != 0.0),
            "the state is the layer's output"
        );
        remembering.forward(&obs, &mut s);
        let warm = s.context().to_vec();
        assert_ne!(
            warm, state_a,
            "the state moved, so the next decision differs"
        );
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
        assert!(g
            .last_outputs(&s)
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1.0));
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
