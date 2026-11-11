//! Small, dependency-free statistics helpers.
//!
//! Everything here is deterministic: random draws come from [`Rng`], a seeded
//! xoshiro256** generator, so a metric computed twice on the same log gives the
//! same interval.

use serde::{Deserialize, Serialize};

/// A two-sided interval with its nominal level and how it was obtained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub lo: f64,
    pub hi: f64,
    /// e.g. 0.95
    pub level: f64,
    /// "wilson", "jeffreys", "bootstrap percentile", "monte carlo"...
    pub method: String,
}

impl Interval {
    pub fn new(lo: f64, hi: f64, level: f64, method: &str) -> Self {
        Interval {
            lo,
            hi,
            level,
            method: method.to_string(),
        }
    }
    pub fn contains(&self, x: f64) -> bool {
        self.lo <= x && x <= self.hi
    }
    pub fn width(&self) -> f64 {
        self.hi - self.lo
    }
}

// ---------------------------------------------------------------------------
// random numbers
// ---------------------------------------------------------------------------

/// Seeded xoshiro256** generator (state filled by splitmix64).
#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix64(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        let s = [
            splitmix64(&mut x),
            splitmix64(&mut x),
            splitmix64(&mut x),
            splitmix64(&mut x),
        ];
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

    /// Uniform on [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform on the open interval (0, 1).
    pub fn open01(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0);
        // Lemire's multiply-shift; the tiny bias is irrelevant here.
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    pub fn bernoulli(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    /// Standard normal (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let u1 = self.open01();
        let u2 = self.f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// Gamma(shape, 1) by Marsaglia & Tsang (2000).
    pub fn gamma(&mut self, shape: f64) -> f64 {
        assert!(shape > 0.0);
        if shape < 1.0 {
            let g = self.gamma(shape + 1.0);
            return g * self.open01().powf(1.0 / shape);
        }
        let d = shape - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let x = self.normal();
            let v = 1.0 + c * x;
            if v <= 0.0 {
                continue;
            }
            let v = v * v * v;
            let u = self.open01();
            if u < 1.0 - 0.0331 * x * x * x * x || u.ln() < 0.5 * x * x + d * (1.0 - v + v.ln()) {
                return d * v;
            }
        }
    }

    /// Beta(a, b) via two gamma draws.
    pub fn beta(&mut self, a: f64, b: f64) -> f64 {
        let x = self.gamma(a);
        let y = self.gamma(b);
        x / (x + y)
    }

    /// Poisson(lambda) by inversion (fine for the small rates used in simulation).
    pub fn poisson(&mut self, lambda: f64) -> u32 {
        let l = (-lambda).exp();
        let mut k = 0u32;
        let mut p = 1.0;
        loop {
            p *= self.f64();
            if p <= l {
                return k;
            }
            k += 1;
        }
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

// ---------------------------------------------------------------------------
// special functions
// ---------------------------------------------------------------------------

/// ln Γ(x) for x > 0 (Lanczos, g = 7, n = 9; ~1e-15 relative error).
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // reflection
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Continued fraction for the incomplete beta function (modified Lentz).
fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const MAX_ITER: usize = 500;
    const EPS: f64 = 1e-15;
    const FPMIN: f64 = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAX_ITER {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// Regularised incomplete beta I_x(a, b), i.e. the Beta(a, b) cdf at x.
pub fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_front = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln();
    let front = ln_front.exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_cf(a, b, x) / a
    } else {
        1.0 - front * beta_cf(b, a, 1.0 - x) / b
    }
}

/// Quantile of Beta(a, b) by bisection on the cdf (robust; ~1e-12 accuracy).
pub fn beta_quantile(p: f64, a: f64, b: f64) -> f64 {
    assert!((0.0..=1.0).contains(&p) && a > 0.0 && b > 0.0);
    if p == 0.0 {
        return 0.0;
    }
    if p == 1.0 {
        return 1.0;
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if beta_cdf(mid, a, b) < p {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-14 {
            break;
        }
    }
    0.5 * (lo + hi)
}

/// Standard normal cdf (via erfc; ~1e-7 absolute accuracy).
pub fn normal_cdf(z: f64) -> f64 {
    0.5 * erfc(-z / std::f64::consts::SQRT_2)
}

/// Complementary error function (Numerical Recipes `erfcc`, fractional error < 1.2e-7).
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let ans = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87
                                        + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 { ans } else { 2.0 - ans }
}

/// Standard normal quantile (Acklam's rational approximation, ~1.2e-9 relative error).
pub fn normal_quantile(p: f64) -> f64 {
    assert!(p > 0.0 && p < 1.0, "normal_quantile needs 0 < p < 1");
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    let p_low = 0.02425;
    if p < p_low {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - p_low {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        -normal_quantile(1.0 - p)
    }
}

// ---------------------------------------------------------------------------
// intervals
// ---------------------------------------------------------------------------

/// Wilson score interval for k successes in n trials. `None` when n = 0.
pub fn wilson(k: usize, n: usize, level: f64) -> Option<Interval> {
    if n == 0 {
        return None;
    }
    let z = normal_quantile(0.5 + level / 2.0);
    let n_f = n as f64;
    let p = k as f64 / n_f;
    let z2 = z * z;
    let denom = 1.0 + z2 / n_f;
    let centre = (p + z2 / (2.0 * n_f)) / denom;
    let half = z * (p * (1.0 - p) / n_f + z2 / (4.0 * n_f * n_f)).sqrt() / denom;
    Some(Interval::new(
        (centre - half).max(0.0),
        (centre + half).min(1.0),
        level,
        "wilson",
    ))
}

/// Equal-tailed credible interval of the Jeffreys posterior Beta(k + ½, n − k + ½).
/// With n = 0 this is the prior's interval.
pub fn jeffreys(k: usize, n: usize, level: f64) -> Interval {
    let a = k as f64 + 0.5;
    let b = (n - k) as f64 + 0.5;
    let tail = (1.0 - level) / 2.0;
    Interval::new(
        beta_quantile(tail, a, b),
        beta_quantile(1.0 - tail, a, b),
        level,
        "jeffreys",
    )
}

/// Sample quantile with linear interpolation (Hyndman & Fan type 7, R's default).
/// `sorted` must be ascending and non-empty.
pub fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    assert!(!sorted.is_empty());
    let h = (sorted.len() - 1) as f64 * q.clamp(0.0, 1.0);
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

pub fn mean(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        None
    } else {
        Some(v.iter().sum::<f64>() / v.len() as f64)
    }
}

pub fn sort_f64(v: &mut [f64]) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
}

/// Percentile-bootstrap interval for the mean of `values`. `None` when empty.
pub fn bootstrap_mean(values: &[f64], resamples: usize, level: f64, seed: u64) -> Option<Interval> {
    bootstrap(
        values,
        |s| mean(s).unwrap_or(f64::NAN),
        resamples,
        level,
        seed,
    )
}

/// Percentile-bootstrap interval for an arbitrary statistic of i.i.d. units.
pub fn bootstrap<T: Clone>(
    units: &[T],
    stat: impl Fn(&[T]) -> f64,
    resamples: usize,
    level: f64,
    seed: u64,
) -> Option<Interval> {
    if units.is_empty() || resamples == 0 {
        return None;
    }
    let mut rng = Rng::new(seed);
    let mut buf: Vec<T> = Vec::with_capacity(units.len());
    let mut stats = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        buf.clear();
        for _ in 0..units.len() {
            buf.push(units[rng.below(units.len())].clone());
        }
        let s = stat(&buf);
        if s.is_finite() {
            stats.push(s);
        }
    }
    if stats.is_empty() {
        return None;
    }
    sort_f64(&mut stats);
    let tail = (1.0 - level) / 2.0;
    Some(Interval::new(
        quantile_sorted(&stats, tail),
        quantile_sorted(&stats, 1.0 - tail),
        level,
        "bootstrap percentile",
    ))
}

/// Interval from Monte Carlo draws (equal-tailed). `None` when empty.
pub fn draws_interval(draws: &mut [f64], level: f64, method: &str) -> Option<Interval> {
    if draws.is_empty() {
        return None;
    }
    sort_f64(draws);
    let tail = (1.0 - level) / 2.0;
    Some(Interval::new(
        quantile_sorted(draws, tail),
        quantile_sorted(draws, 1.0 - tail),
        level,
        method,
    ))
}

/// Ratings needed per arm to detect p1 vs p2 with a two-sided two-proportion z-test
/// (normal approximation): n = (z_{1−α/2} + z_{power})² · [p1(1−p1) + p2(1−p2)] / (p1 − p2)².
pub fn two_proportion_n_per_arm(p1: f64, p2: f64, alpha: f64, power: f64) -> usize {
    let z = normal_quantile(1.0 - alpha / 2.0) + normal_quantile(power);
    let var = p1 * (1.0 - p1) + p2 * (1.0 - p2);
    (z * z * var / ((p1 - p2) * (p1 - p2))).ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn ln_gamma_known_values() {
        assert!(close(ln_gamma(5.0), 24f64.ln(), 1e-12));
        assert!(close(
            ln_gamma(0.5),
            std::f64::consts::PI.sqrt().ln(),
            1e-12
        ));
        assert!(close(ln_gamma(10.5), 1_133_278.388_7f64.ln(), 1e-9));
    }

    #[test]
    fn beta_cdf_known_values() {
        // I_0.5(2,3) = 11/16
        assert!(close(beta_cdf(0.5, 2.0, 3.0), 0.6875, 1e-12));
        // Beta(1,1) is uniform
        assert!(close(beta_cdf(0.3, 1.0, 1.0), 0.3, 1e-12));
        // Beta(½,½) is the arcsine law: F(x) = 2/π · asin(√x)
        let x: f64 = 0.2;
        assert!(close(
            beta_cdf(x, 0.5, 0.5),
            2.0 / std::f64::consts::PI * x.sqrt().asin(),
            1e-10
        ));
    }

    #[test]
    fn beta_quantile_known_values() {
        // arcsine law quantile: sin²(πp/2)
        let p = 0.025;
        let expected = (std::f64::consts::PI * p / 2.0).sin().powi(2);
        assert!(close(beta_quantile(p, 0.5, 0.5), expected, 1e-10));
        // median of Beta(2,3) ≈ 0.385727568 (R: qbeta(0.5, 2, 3))
        assert!(close(beta_quantile(0.5, 2.0, 3.0), 0.385_727_568, 1e-8));
        // qbeta(0.975, 10.5, 10.5) = 0.7032... symmetric about 0.5
        let lo = beta_quantile(0.025, 10.5, 10.5);
        let hi = beta_quantile(0.975, 10.5, 10.5);
        assert!(close(lo + hi, 1.0, 1e-10));
    }

    #[test]
    fn normal_known_values() {
        assert!(close(normal_quantile(0.975), 1.959_963_985, 1e-8));
        assert!(close(normal_quantile(0.8), 0.841_621_234, 1e-8));
        assert!(close(normal_quantile(0.001), -3.090_232_306, 1e-7));
        assert!(close(normal_cdf(1.959_963_985), 0.975, 1e-6));
        assert!(close(normal_cdf(0.0), 0.5, 1e-7));
    }

    #[test]
    fn wilson_known_values() {
        // R: prop.test-free Wilson for 10/20 → (0.2993, 0.7007)
        let w = wilson(10, 20, 0.95).unwrap();
        assert!(
            close(w.lo, 0.299_3, 1e-4) && close(w.hi, 0.700_7, 1e-4),
            "{w:?}"
        );
        // 0/10 → (0, 0.2775)
        let w = wilson(0, 10, 0.95).unwrap();
        assert!(
            close(w.lo, 0.0, 1e-12) && close(w.hi, 0.277_5, 1e-4),
            "{w:?}"
        );
        assert!(wilson(0, 0, 0.95).is_none());
    }

    #[test]
    fn jeffreys_interval_behaves() {
        // Jeffreys for 10/20: Beta(10.5, 10.5), symmetric, approx (0.2934, 0.7066)
        let j = jeffreys(10, 20, 0.95);
        assert!(close(j.lo + j.hi, 1.0, 1e-10));
        assert!(close(j.lo, 0.293_4, 2e-3), "{j:?}");
    }

    #[test]
    fn quantile_type7() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert!(close(quantile_sorted(&v, 0.5), 2.5, 1e-12));
        assert!(close(quantile_sorted(&v, 0.25), 1.75, 1e-12));
    }

    #[test]
    fn bootstrap_is_deterministic_and_sane() {
        let v: Vec<f64> = (0..100).map(|i| (i % 2) as f64).collect();
        let a = bootstrap_mean(&v, 2000, 0.95, 7).unwrap();
        let b = bootstrap_mean(&v, 2000, 0.95, 7).unwrap();
        assert_eq!(a, b);
        assert!(a.contains(0.5));
        // standard error of a mean of Bernoulli(0.5) with n=100 is 0.05 → width ≈ 0.2
        assert!(close(a.width(), 0.196, 0.03), "{a:?}");
    }

    #[test]
    fn samplers_have_right_moments() {
        let mut rng = Rng::new(1);
        let n = 50_000;
        let m: f64 = (0..n).map(|_| rng.beta(3.0, 7.0)).sum::<f64>() / n as f64;
        assert!(close(m, 0.3, 0.005), "{m}");
        let m: f64 = (0..n).map(|_| rng.gamma(0.5)).sum::<f64>() / n as f64;
        assert!(close(m, 0.5, 0.02), "{m}");
        let m: f64 = (0..n).map(|_| rng.poisson(3.0) as f64).sum::<f64>() / n as f64;
        assert!(close(m, 3.0, 0.05), "{m}");
    }

    #[test]
    fn power_calc_matches_textbook() {
        // 70% vs 40%: ~40 per arm (~80 in total); 60% vs 40%: ~95 per arm (~190 in total)
        assert_eq!(two_proportion_n_per_arm(0.7, 0.4, 0.05, 0.8), 40);
        assert_eq!(two_proportion_n_per_arm(0.6, 0.4, 0.05, 0.8), 95);
    }
}
