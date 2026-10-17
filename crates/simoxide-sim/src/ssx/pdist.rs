//! Distribution samplers of the Slingshot-exact mode where Palladio 6.0 (the reference's
//! probability-function library) differs from SimuLizar 5.2.2's (`simoxide_random::dist`).

use simoxide_random::{DistError, UniformSource, dist};

pub(crate) fn sample_gamma<R: UniformSource + ?Sized>(
    alpha: f64,
    theta: f64,
    r: &mut R,
) -> Result<f64, DistError> {
    dist::sample_gamma(alpha, theta, r)
}

pub(crate) fn sample_gamma_moments<R: UniformSource + ?Sized>(
    mean: f64,
    coeff_var: f64,
    r: &mut R,
) -> Result<f64, DistError> {
    dist::sample_gamma_moments(mean, coeff_var, r)
}

pub(crate) fn sample_lognorm<R: UniformSource + ?Sized>(
    mu: f64,
    sigma: f64,
    r: &mut R,
) -> Result<f64, DistError> {
    dist::sample_lognorm(mu, sigma, r)
}

pub(crate) fn sample_lognorm_moments<R: UniformSource + ?Sized>(
    mean: f64,
    stdev: f64,
    r: &mut R,
) -> Result<f64, DistError> {
    dist::sample_lognorm_moments(mean, stdev, r)
}
