//! Seeded RNG (§5.5, §5.6). No global RNG, no `thread_rng()`, ever.

// `Rng as _`: the trait is needed for method resolution on `Rng` (the type
// alias below), but the name itself must stay free for that alias.
use rand_xoshiro::rand_core::Rng as _;
use rand_xoshiro::rand_core::SeedableRng;
use rand_xoshiro::Xoshiro256PlusPlus;

/// The sim's only RNG type.
pub type Rng = Xoshiro256PlusPlus;

/// Per-episode seed derivation.
///
/// SplitMix64-style avalanche: integer-only and bijective, so distinct
/// `(generation, genome_id, trial)` triples never collide for one master seed.
///
/// This function is frozen. Changing it invalidates every champion's
/// `episode_seed` and is a `sim_version` bump.
pub fn episode_seed(master_seed: u64, generation: u32, genome_id: u32, trial: u32) -> u64 {
    let mut z = master_seed
        .wrapping_add(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(((generation as u64) << 32) | genome_id as u64)
        .wrapping_add((trial as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Construct the per-episode generator.
///
/// `rand_xoshiro` seeds `Xoshiro256PlusPlus` from `u64` via SplitMix64, so a
/// single `u64` fully determines the stream. Must be called *inside* the rayon
/// closure (Pattern A, §5.6), never shared across threads.
pub fn rng_from_seed(seed: u64) -> Rng {
    Xoshiro256PlusPlus::seed_from_u64(seed)
}

/// Uniform `f32` in `[0, 1)`.
///
/// Uses the top 24 bits so every representable value is reachable and the
/// mapping is identical everywhere (no `rand` distribution traits involved).
pub fn next_f32(rng: &mut Rng) -> f32 {
    // 24 bits is the f32 mantissa width; 1 << 24 is exactly representable.
    ((rng.next_u32() >> 8) as f32) * (1.0 / 16_777_216.0)
}

/// Uniform `f32` in `[-1, 1)`.
pub fn next_signed_f32(rng: &mut Rng) -> f32 {
    next_f32(rng) * 2.0 - 1.0
}

/// Standard normal via Box–Muller, in `(0, 1]`-safe polar-free form.
///
/// Hand-rolled instead of pulling `rand_distr` so the *exact* mutation stream is
/// pinned by this crate's own code rather than by a dependency's algorithm
/// choice. A future `rand_distr` change must not silently alter evolution.
pub fn next_gaussian(rng: &mut Rng, sigma: f32) -> f32 {
    // u1 in (0, 1] so ln() never sees zero (next_f32 can return exactly 0.0).
    let u1 = 1.0 - next_f32(rng);
    let u2 = next_f32(rng);
    let r = (-2.0 * u1.ln()).sqrt();
    r * (std::f32::consts::TAU * u2).cos() * sigma
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episode_seed_is_a_function_of_its_inputs() {
        // Stability: the same inputs must yield the same seed forever (§5.5).
        assert_eq!(episode_seed(1, 0, 0, 0), episode_seed(1, 0, 0, 0));
        assert_eq!(episode_seed(0, 7, 3, 0), episode_seed(0, 7, 3, 0));
    }

    #[test]
    fn episode_seed_separates_its_components() {
        let base = episode_seed(42, 10, 20, 0);
        assert_ne!(base, episode_seed(43, 10, 20, 0), "master seed");
        assert_ne!(base, episode_seed(42, 11, 20, 0), "generation");
        assert_ne!(base, episode_seed(42, 10, 21, 0), "genome id");
        assert_ne!(base, episode_seed(42, 10, 20, 1), "trial");
    }

    #[test]
    fn episode_seed_pins_known_values() {
        // Golden values. If these change, champion `episode_seed` fields break:
        // that is a deliberate `sim_version` bump, never a casual edit.
        assert_eq!(episode_seed(0, 0, 0, 0), 0xe220_a839_7b1d_cdaf);
        assert_eq!(episode_seed(1, 0, 1, 0), 0x9758_35de_1c97_56ce);
        assert_eq!(episode_seed(0xDEAD_BEEF, 87, 42, 0), 0xd098_d054_1ff8_24c9);
    }

    #[test]
    fn rng_stream_is_reproducible_and_not_shared() {
        let mut a = rng_from_seed(7);
        let mut b = rng_from_seed(7);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = rng_from_seed(8);
        assert_ne!(rng_from_seed(7).next_u64(), c.next_u64());
    }

    #[test]
    fn gaussian_is_bounded_and_centred() {
        let mut rng = rng_from_seed(1234);
        let mut sum = 0.0;
        let n = 20_000;
        for _ in 0..n {
            let v = next_gaussian(&mut rng, 1.0);
            assert!(v.is_finite());
            assert!(v.abs() < 12.0, "implausible sample {v}");
            sum += v;
        }
        let mean = sum / n as f32;
        assert!(mean.abs() < 0.05, "mean {mean} is not ~0");
    }
}
