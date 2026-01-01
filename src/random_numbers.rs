use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;

pub enum Distribution {
    ERLANG,
    NORMAL,
    UNIFORM,
    CONSTANT,
}
pub trait DistributionSampler {
    type Params;
    fn sample(&mut self) -> f64;
    fn change_seed(&mut self, new_seed: u64);
    fn parametrize(&mut self, params: Self::Params);
    
}

pub struct UniformParams {min: f64,max: f64}
pub struct UniformSampler {
    params: Option<UniformParams>,
    seed: u64,
    prng: StdRng,
}
impl UniformSampler {
    pub fn new(seed: u64, params: Option<UniformParams>) -> UniformSampler {
        let prng: StdRng = StdRng::seed_from_u64(seed);
        UniformSampler { params,seed, prng}
    }
}
impl DistributionSampler for UniformSampler {
    type Params = Option<UniformParams>;
    fn sample(&mut self) -> f64 {
        match &self.params {
            Some(p) => {
                self.prng.random_range(p.min..p.max)
            },
            None => self.prng.random::<f64>(),
        }
    }
    fn change_seed(&mut self, new_seed: u64) {
        self.prng = StdRng::seed_from_u64(new_seed)
    }
    fn parametrize(&mut self, new_params: Option<UniformParams>) {
        self.params = new_params;
    }
}
pub struct ErlangParams {mean: f64,k: i32}
pub struct ErlangSampler {
    mean: f64,
    k: i32,
    lambda: f64,
    seed: u64,
    prng: StdRng,
}
impl ErlangSampler {
    pub fn new(mean: f64, k: i32, seed: u64) -> ErlangSampler {
        let prng: StdRng = StdRng::seed_from_u64(seed);
        ErlangSampler {
            mean,
            k,
            lambda: k as f64 / mean,
            seed,
            prng,
        }
    }
}
impl DistributionSampler for ErlangSampler {
    type Params = ErlangParams;
    fn sample(&mut self) -> f64 {
        let mut product: f64 = 1.0;
        for _ in 0..self.k {
            let u: f64 = self.prng.random::<f64>().max(f64::MIN_POSITIVE);
            product *= u;
        }
        -1.0 / self.lambda * product.ln()
    }
    fn change_seed(&mut self, new_seed: u64) {
        self.prng = StdRng::seed_from_u64(new_seed)
    }
    fn parametrize(&mut self, new_params: ErlangParams) {
        self.mean = new_params.mean;
        self.k = new_params.k;
        self.lambda = self.k as f64 / self.mean;
    }
}

pub struct NormalParams {mean: f64,std: f64}
pub struct NormalSampler {
    params: NormalParams,
    seed: u64,
    prng: StdRng,
}
impl NormalSampler {
    pub fn new(mean: f64, std: f64, seed: u64) -> NormalSampler {
        let prng: StdRng = StdRng::seed_from_u64(seed);
        NormalSampler {
            params: NormalParams {mean,std},
            seed,
            prng,
        }
    }
}
impl DistributionSampler for NormalSampler {
    type Params = NormalParams;
    fn sample(&mut self) -> f64 {
        // Box-Muller
        let u1: f64 = self.prng.random::<f64>().max(f64::MIN_POSITIVE);
        let u2: f64 = self.prng.random::<f64>();
        let z0: f64 = (-2.0f64 * u1.ln()).sqrt() * (2.0f64 * std::f64::consts::PI * u2).cos();
        self.params.mean + self.params.std * z0
    }
    fn change_seed(&mut self, new_seed: u64) {
        self.prng = StdRng::seed_from_u64(new_seed)
    }
    fn parametrize(&mut self, new_params: NormalParams) {
        self.params = new_params;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to compute mean and variance of a slice
    fn mean_variance(xs: &[f64]) -> (f64, f64) {
        let n = xs.len() as f64;
        let mean = xs.iter().sum::<f64>() / n;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        (mean, var)
    }

    // Simple histogram printer for debugging distributions
    fn print_histogram(xs: &[f64], min: f64, max: f64, bins: usize, title: &str) {
        let mut counts = vec![0usize; bins];
        let width = (max - min) / bins as f64;
        for &x in xs {
            if x < min || x >= max {
                continue; // skip out of range
            }
            let idx = ((x - min) / width).floor() as isize;
            let idx = idx.clamp(0, (bins as isize) - 1) as usize;
            counts[idx] += 1;
        }
        let total: usize = counts.iter().sum();
        println!("\nHistogram: {} (range [{}, {}], bins={})", title, min, max, bins);
        for i in 0..bins {
            let start = min + i as f64 * width;
            let end = start + width;
            let pct = if total > 0 { (counts[i] as f64) * 100.0 / (total as f64) } else { 0.0 };
            println!("[{:.3}, {:.3}): {:6} ({:5.2}%)", start, end, counts[i], pct);
        }
    }

    // Approximate error function (Abramowitz-Stegun) for normal CDF
    fn erf_approx(x: f64) -> f64 {
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        let x = x.abs();
        let a1 = 0.254829592;
        let a2 = -0.284496736;
        let a3 = 1.421413741;
        let a4 = -1.453152027;
        let a5 = 1.061405429;
        let p = 0.3275911;
        let t = 1.0 / (1.0 + p * x);
        let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();
        sign * y
    }

    fn normal_cdf(x: f64, mean: f64, std: f64) -> f64 {
        0.5 * (1.0 + erf_approx((x - mean) / (std * 2f64.sqrt())))
    }

    // Erlang CDF for integer k: 1 - e^{-lambda x} * sum_{i=0}^{k-1} (lambda x)^i / i!
    fn erlang_cdf(x: f64, k: i32, lambda: f64) -> f64 {
        if x <= 0.0 {
            return 0.0;
        }
        let lx = lambda * x;
        let mut sum = 0.0;
        let mut term = 1.0;
        for i in 0..k {
            if i == 0 {
                term = 1.0;
            } else {
                term *= lx / (i as f64);
            }
            sum += term;
        }
        1.0 - (-lx).exp() * sum
    }

    // One-sample Kolmogorov-Smirnov test (two-sided) using asymptotic p-value approximation
    fn ks_test(samples: &mut [f64], cdf: impl Fn(f64) -> f64) -> f64 {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = samples.len() as f64;
        let mut d_plus: f64 = 0.0;
        let mut d_minus: f64 = 0.0;
        for (i, &x) in samples.iter().enumerate() {
            let i1 = (i + 1) as f64;
            let fx = cdf(x).clamp(0.0, 1.0);
            d_plus = d_plus.max(i1 / n - fx);
            d_minus = d_minus.max(fx - (i as f64) / n);
        }
        let d = d_plus.max(d_minus);

        // Compute p-value using asymptotic expansion
        let en = (n).sqrt();
        let z = (en + 0.12 + 0.11 / en) * d;
        if z < 1.18 {
            return 1.0; // p is ~1 for very small z (cannot reject)
        }
        let mut j = 1u64;
        let mut sum = 0.0;
        loop {
            let term = 2.0 * (-2.0 * (j as f64) * (j as f64) * z * z).exp() * if j % 2 == 1 { 1.0 } else { -1.0 };
            sum += term;
            if term.abs() < 1e-12 || j > 1000 {
                break;
            }
            j += 1;
        }
        // clamp p-value
        sum.clamp(0.0, 1.0)
    }

    #[test]
    fn erlang_distribution_statistics_are_close() {
        // Target Erlang parameters
        let k = 3; // shape
        let target_mean = 15.0; // desired mean
        let lambda = k as f64 / target_mean; // rate

        // Sample many times using an `ErlangSampler` instance
        let samples = 200_000;
        let mut xs = Vec::with_capacity(samples);
        let mut sampler = ErlangSampler::new(target_mean, k, 0xDEADBEEFCAFEBABE_u64);
        for _ in 0..samples {
            xs.push(sampler.sample());
        }

        // Kolmogorov-Smirnov test against Erlang CDF
        let p = ks_test(&mut xs, |x| erlang_cdf(x, k, lambda));
        assert!(p > 1e-6, "Erlang KS p-value too small: {}", p);
        let min = 0.0;
        let max = target_mean * 3.0;
        print_histogram(&xs, min, max, 30, "Erlang(k=3)");
    }

    #[test]
    fn uniform_distribution_statistics_are_close() {
        let samples = 200_000;
        let mut xs = Vec::with_capacity(samples);
        let mut sampler = UniformSampler::new(0x1234_5678_9ABC_DEF0_u64, None);
        for _ in 0..samples {
            xs.push(sampler.sample());
        }

        // KS test vs Uniform(0,1)
        let p = ks_test(&mut xs, |x| if x <= 0.0 { 0.0 } else if x >= 1.0 { 1.0 } else { x });
        assert!(p > 1e-6, "Uniform KS p-value too small: {}", p);
        print_histogram(&xs, 0.0, 1.0, 20, "Uniform(0,1)");
    }

    #[test]
    fn normal_distribution_statistics_are_close() {
        let target_mean = 10.0;
        let target_std = 2.0;
        let samples = 200_000;
        let mut xs = Vec::with_capacity(samples);
        let mut sampler = NormalSampler::new(target_mean, target_std, 0xCAFEBABE_DEADBEEF_u64);
        for _ in 0..samples {
            xs.push(sampler.sample());
        }

        // KS test vs Normal(target_mean, target_std)
        let p = ks_test(&mut xs, |x| normal_cdf(x, target_mean, target_std));
        assert!(p > 1e-6, "Normal KS p-value too small: {}", p);
        let min = target_mean - 4.0 * target_std;
        let max = target_mean + 4.0 * target_std;
        print_histogram(&xs, min, max, 40, "Normal(mean=10,std=2)");
    }
}