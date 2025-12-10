use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;
// rand 0.9: use Rng::gen for floats in [0,1)


pub enum Distribution {
    ERLANG,
    NORMAL,
    UNIFORM,
}

pub fn sample_standard_uniform(seed: u64) -> f64 {
    let mut rng = StdRng::seed_from_u64(seed);
    rng.random::<f64>()
}

pub fn sample_normal_distribution(seed: u64, mean: f64, std: f64) -> f64 {
    let mut rng = StdRng::seed_from_u64(seed);
    // Box-Muller
    let u1: f64 = rng.random::<f64>().max(f64::MIN_POSITIVE);
    let u2: f64 = rng.random::<f64>();
    let z0: f64 = (-2.0f64 * u1.ln()).sqrt() * (2.0f64 * std::f64::consts::PI * u2).cos();
    mean + std * z0
}

pub fn sample_erlang_distribution(seed: u64, mean: f64, k: i32) -> f64 {
    let mut rng: StdRng = StdRng::seed_from_u64(seed);
    let lambda: f64 = k as f64 / mean;
    let mut product: f64 = 1.0;
    for _ in 0..k {
        let u: f64 = rng.random::<f64>().max(f64::MIN_POSITIVE);
        product *= u;
    }
    -1.0 / lambda * product.ln()
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

    #[test]
    fn erlang_distribution_statistics_are_close() {
        // Target Erlang parameters
        let k = 3; // shape
        let target_mean = 15.0; // desired mean
        let lambda = k as f64 / target_mean; // rate

        // Sample many times with different seeds to avoid correlation
        let samples = 200_000;
        let mut xs = Vec::with_capacity(samples);
        for i in 0..samples {
            let seed = 0xDEADBEEFCAFEBABE_u64 ^ (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
            let x = sample_erlang_distribution(seed, target_mean, k);
            xs.push(x);
        }

        let (m, v) = mean_variance(&xs);
        let expected_mean = target_mean; // k/lambda
        let expected_var = (k as f64) / (lambda * lambda);

        // Allow relative error tolerance due to finite sampling
        let mean_rel_err = ((m - expected_mean).abs()) / expected_mean;
        let var_rel_err = ((v - expected_var).abs()) / expected_var;

        assert!(mean_rel_err < 0.02, "mean rel err too large: {:e} (m={}, expected={})", mean_rel_err, m, expected_mean);
        assert!(var_rel_err < 0.05, "var rel err too large: {:e} (v={}, expected={})", var_rel_err, v, expected_var);

        // Print histogram focused around the expected range
        let min = 0.0;
        let max = expected_mean * 3.0;
        print_histogram(&xs, min, max, 30, "Erlang(k=3)");
    }

    #[test]
    fn uniform_distribution_statistics_are_close() {
        let samples = 200_000;
        let mut xs = Vec::with_capacity(samples);
        for i in 0..samples {
            let seed = 0x1234_5678_9ABC_DEF0_u64 ^ (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
            let x = sample_standard_uniform(seed);
            xs.push(x);
        }

        let (m, v) = mean_variance(&xs);
        let expected_mean = 0.5;
        let expected_var = 1.0 / 12.0; // variance of U(0,1)

        let mean_rel_err = ((m - expected_mean).abs()) / expected_mean;
        let var_rel_err = ((v - expected_var).abs()) / expected_var;

        assert!(mean_rel_err < 0.01, "mean rel err too large: {:e} (m={}, expected={})", mean_rel_err, m, expected_mean);
        assert!(var_rel_err < 0.03, "var rel err too large: {:e} (v={}, expected={})", var_rel_err, v, expected_var);

        // Histogram over [0,1)
        print_histogram(&xs, 0.0, 1.0, 20, "Uniform(0,1)");
    }

    #[test]
    fn normal_distribution_statistics_are_close() {
        let target_mean = 10.0;
        let target_std = 2.0;
        let samples = 200_000;
        let mut xs = Vec::with_capacity(samples);
        for i in 0..samples {
            let seed = 0xCAFEBABE_DEADBEEF_u64 ^ (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
            let x = sample_normal_distribution(seed, target_mean, target_std);
            xs.push(x);
        }

        let (m, v) = mean_variance(&xs);
        let expected_mean = target_mean;
        let expected_var = target_std * target_std;

        let mean_rel_err = ((m - expected_mean).abs()) / expected_mean.max(f64::MIN_POSITIVE);
        let var_rel_err = ((v - expected_var).abs()) / expected_var.max(f64::MIN_POSITIVE);

        assert!(mean_rel_err < 0.01, "mean rel err too large: {:e} (m={}, expected={})", mean_rel_err, m, expected_mean);
        assert!(var_rel_err < 0.05, "var rel err too large: {:e} (v={}, expected={})", var_rel_err, v, expected_var);

        // Histogram around mean +/- 4 std deviations
        let min = target_mean - 4.0 * target_std;
        let max = target_mean + 4.0 * target_std;
        print_histogram(&xs, min, max, 40, "Normal(mean=10,std=2)");
    }
}