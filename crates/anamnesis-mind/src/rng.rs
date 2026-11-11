//! Small deterministic random number generation.
//!
//! Everything random in this crate (layout negative sampling, hold-out arms,
//! resurfacing arms) is driven by an explicit `u64` seed so runs and
//! evaluations are reproducible. We avoid a `rand` dependency on purpose:
//! a SplitMix64 hash plus a xoshiro256** stream is all we need.

/// SplitMix64 finaliser: a good 64-bit mixing function.
pub fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Stable 64-bit FNV-1a hash of bytes (independent of Rust's randomised hasher).
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// Hash a seed together with a list of string parts into a uniform `[0, 1)` float.
pub fn unit_from(seed: u64, parts: &[&str]) -> f64 {
    let mut h = mix64(seed);
    for p in parts {
        h = mix64(h ^ hash_bytes(p.as_bytes()));
        // separator so ("ab","c") != ("a","bc")
        h = mix64(h ^ 0x1F);
    }
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// xoshiro256** generator seeded through SplitMix64.
#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        let mut s = [0u64; 4];
        for v in s.iter_mut() {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            *v = mix64(x);
        }
        Rng { s }
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, 1)`.
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn f32(&mut self) -> f32 {
        self.f64() as f32
    }

    /// Uniform integer in `[0, n)`. `n` must be > 0.
    pub fn below(&mut self, n: usize) -> usize {
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    /// Standard normal (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let u1 = self.f64().max(1e-300);
        let u2 = self.f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_uniformish() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut r = Rng::new(1);
        let mean: f64 = (0..10_000).map(|_| r.f64()).sum::<f64>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02);
        assert_eq!(unit_from(3, &["a", "b"]), unit_from(3, &["a", "b"]));
        assert_ne!(unit_from(3, &["ab", "c"]), unit_from(3, &["a", "bc"]));
    }
}
