//! State hashing for the determinism contract (§5).
//!
//! FNV-1a over canonicalized little-endian bytes. Chosen over `DefaultHasher`
//! because `std`'s hasher is explicitly not stable across releases, and over
//! sha2 because the sim crate stays dependency-light for a value that only ever
//! needs to be compared against itself.

/// Streaming FNV-1a 64-bit hasher.
///
/// Every `f32` is canonicalized before hashing: `NaN` payloads are
/// nondeterministic per the WASM spec (§5.3), so all NaNs collapse to a single
/// bit pattern and `-0.0`/`0.0` are distinguished only if the sim produces them
/// (it does not — physics arithmetic is total).
#[derive(Clone, Debug)]
pub struct StateHasher {
    h: u64,
}

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHasher {
    pub const fn new() -> Self {
        StateHasher {
            h: FNV_OFFSET_BASIS,
        }
    }

    #[inline]
    pub fn u8(&mut self, v: u8) {
        self.h ^= v as u64;
        self.h = self.h.wrapping_mul(FNV_PRIME);
    }

    #[inline]
    pub fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }

    #[inline]
    pub fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }

    #[inline]
    pub fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }

    #[inline]
    pub fn i8(&mut self, v: i8) {
        self.u8(v as u8);
    }

    #[inline]
    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }

    /// Hash a float by bit pattern, canonicalizing NaN first (§5.3).
    #[inline]
    pub fn f32(&mut self, v: f32) {
        let canonical = if v.is_nan() { f32::NAN } else { v };
        self.u32(canonical.to_bits());
    }

    pub fn finish(&self) -> u64 {
        self.h
    }
}

/// One-shot convenience for hashing a slice of floats.
pub fn hash_f32_slice(values: &[f32]) -> u64 {
    let mut h = StateHasher::new();
    h.u32(values.len() as u32);
    for v in values {
        h.f32(*v);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hasher_is_order_sensitive() {
        let mut a = StateHasher::new();
        a.u32(1);
        a.u32(2);
        let mut b = StateHasher::new();
        b.u32(2);
        b.u32(1);
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn all_nans_collapse() {
        let mut a = StateHasher::new();
        a.f32(f32::NAN);
        let mut b = StateHasher::new();
        b.f32(-f32::NAN);
        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn floats_are_hashed_by_bits() {
        let mut a = StateHasher::new();
        a.f32(1.0);
        let mut b = StateHasher::new();
        b.u32(1.0f32.to_bits());
        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn empty_slice_is_stable() {
        assert_eq!(hash_f32_slice(&[]), hash_f32_slice(&[]));
    }
}
