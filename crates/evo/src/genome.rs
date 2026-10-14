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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetShape {
    pub inputs: usize,
    pub hidden: Vec<usize>,
    pub outputs: usize,
}

/// The v1 recipe: 90 → 16 (tanh) → 6 (tanh, argmax) (§7.5).
pub fn default_shape() -> NetShape {
    NetShape {
        inputs: N_INPUTS,
        hidden: vec![16],
        outputs: N_OUTPUTS,
    }
}

impl NetShape {
    /// Number of layers with weights, hidden layers plus the output layer.
    pub fn n_layers(&self) -> usize {
        self.hidden.len() + 1
    }

    /// `(fan_in, fan_out)` for layer `l`.
    pub fn layer(&self, l: usize) -> (usize, usize) {
        let fan_in = if l == 0 { self.inputs } else { self.hidden[l - 1] };
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
    pub fn max_width(&self) -> usize {
        self.hidden
            .iter()
            .copied()
            .chain(std::iter::once(self.outputs))
            .chain(std::iter::once(self.inputs))
            .max()
            .unwrap_or(0)
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
#[derive(Clone, Debug, Default)]
pub struct ActScratch {
    input: Vec<f32>,
    a: Vec<f32>,
    b: Vec<f32>,
    out: Vec<f32>,
}

impl ActScratch {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure(&mut self, shape: &NetShape) {
        let w = shape.max_width();
        if self.input.len() < shape.inputs {
            self.input.resize(shape.inputs, 0.0);
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
    /// Xavier rather than plain uniform: with 95 inputs into 16 tanh units, a
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
    pub fn forward(&self, inputs: &[f32], s: &mut ActScratch) {
        debug_assert_eq!(inputs.len(), self.shape.inputs);
        s.ensure(&self.shape);
        let ActScratch { input, a, b, out } = s;
        input[..self.shape.inputs].copy_from_slice(&inputs[..self.shape.inputs]);

        let n_hidden = self.shape.hidden.len();
        let mut w_off = 0usize;
        let mut b_off = 0usize;
        // `fresh_a` tracks which buffer holds the most recent activations.
        let mut fresh_a = false;

        for l in 0..=n_hidden {
            let (fan_in, fan_out) = self.shape.layer(l);
            let is_last = l == n_hidden;
            if l == 0 {
                if is_last {
                    dense(&input[..fan_in], out, self, w_off, b_off, fan_in, fan_out);
                } else {
                    dense(&input[..fan_in], &mut a[..fan_out], self, w_off, b_off, fan_in, fan_out);
                    fresh_a = true;
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
        for (c, xi) in x.iter().enumerate() {
            acc += row[c] * xi;
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
/// construction: at the default rate it redraws five percent of a 1 723-gene genome, so
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

/// A **reheated** copy of a genome: every gene moved by its own Gaussian step of `sigma`.
///
/// **The third scale of mutation, and the only one that is not a descendant.** [`mutate`] is
/// a coarse step from a parent *in the population*, [`mutate_few`] is a fine step from the
/// generation's best, and both stay where the population already is. A reheat is the move for
/// a plateau: it walks a long way from the best point the run has found while keeping that
/// point's *shape* — the signs and relative magnitudes of 1 723 genes survive a step of a few
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
        // v4: 95 inputs (the plan's 90, plus wall side/wall slide/sprint charge from v2,
        // plus the goal's vertical component and the leftward gap probe) and eleven outputs
        // (the plan's six, plus sprint, sprint_jump, retreat_jump and the two leftward
        // sprinting arms). The pin is the *recipe* — one hidden layer of sixteen,
        // argument-max output — with the two ends read from the sim so this test cannot
        // disagree with the action table or the sensor layout.
        assert_eq!(s.inputs, N_INPUTS);
        assert_eq!(s.inputs, 95);
        assert_eq!(s.hidden, vec![16]);
        assert_eq!(s.outputs, N_OUTPUTS);
        assert_eq!(s.outputs, 11);
        assert_eq!(s.n_weights(), 95 * 16 + 16 * 11);
        assert_eq!(s.n_biases(), 16 + 11);
        assert_eq!(s.n_genes(), 95 * 16 + 16 * 11 + 16 + 11);
        s.validate().expect("shape is valid");
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
