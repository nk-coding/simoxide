//! The two random streams of the Slingshot-exact mode (`docs/spec/slingshot/random.md`) and the
//! tape projection (`tape-projection.jsonl`, SS-X-4).
//!
//! * StoEx: SimuCom's MT19937 seeded `{N, …, N+5}` ([`simoxide_random::SimuComStream`]), the
//!   reference runner's `-stoex mt` (SS-RNG-1.1);
//! * `Math.random()`: `java.util.Random(N + 1000003)` ([`crate::javarand`], written from the
//!   API specification), for branches and link failures (SS-RNG-1.2).
//!
//! Both are EPL code; nothing here needs an opt-in crate.

use crate::javafmt::{push_json_double, push_json_str};
use crate::javarand::JavaRandom;
use simoxide_random::{DistError, SimuComStream, UniformSource};

/// The streams plus the tape bookkeeping.
pub(crate) struct SsRng {
    mt: SimuComStream,
    /// StoEx uniforms drawn so far.
    pub stoex: u64,
    pub math: JavaRandom,
    /// `tape-projection.jsonl` records (`None`: no tape).
    pub tape: Option<String>,
    /// Purpose of the StoEx draws of the current evaluation (SS-X-4).
    pub site: &'static str,
    /// Index of the last level-E projection line at or before the event being dispatched.
    pub pos: i64,
}

impl SsRng {
    pub fn new(seed: i64, tape: bool) -> Result<SsRng, String> {
        let seeds: Vec<i64> = (0..6).map(|i| seed + i).collect();
        Ok(SsRng {
            mt: SimuComStream::from_seed(&seeds).map_err(|e| e.to_string())?,
            stoex: 0,
            math: JavaRandom::slingshot_math(seed),
            tape: tape.then(|| String::with_capacity(1 << 16)),
            site: "?",
            pos: -1,
        })
    }

    fn record(&mut self, stream: &str, i: u64, u: f64, site: &str) {
        let pos = self.pos;
        if let Some(b) = self.tape.as_mut() {
            b.push_str("{\"s\":\"");
            b.push_str(stream);
            b.push_str("\",\"i\":");
            b.push_str(&i.to_string());
            b.push_str(",\"u\":");
            push_json_double(b, u);
            b.push_str(",\"o\":");
            push_json_str(b, site);
            b.push_str(",\"p\":");
            b.push_str(&pos.to_string());
            b.push_str("}\n");
        }
    }

    /// `Math.random()` at `site` (`branch:<id>`, `link:<id>`).
    pub fn math_random(&mut self, site: &str) -> f64 {
        let u = self.math.next_double();
        let i = self.math.count - 1;
        self.record("math", i, u, site);
        u
    }
}

impl UniformSource for SsRng {
    #[inline]
    fn next_uniform(&mut self) -> f64 {
        let u = self.mt.next_uniform();
        let i = self.stoex;
        self.stoex += 1;
        if self.tape.is_some() {
            let site = self.site;
            self.record("stoex", i, u, site);
        }
        u
    }

    // Palladio 6.0's samplers (the reference runs on Palladio 6.0's probability-function
    // library, Commons Math 3): `pdist`
    fn sample_exp(&mut self, rate: f64) -> Result<f64, DistError> {
        super::pdist::sample_exp(rate, self)
    }
    fn sample_norm(&mut self, mean: f64, sd: f64) -> Result<f64, DistError> {
        super::pdist::sample_norm(mean, sd, self)
    }
    fn sample_lognorm(&mut self, mu: f64, sigma: f64) -> Result<f64, DistError> {
        super::pdist::sample_lognorm(mu, sigma, self)
    }
    fn sample_lognorm_moments(&mut self, mean: f64, stdev: f64) -> Result<f64, DistError> {
        super::pdist::sample_lognorm_moments(mean, stdev, self)
    }
    fn sample_gamma(&mut self, alpha: f64, theta: f64) -> Result<f64, DistError> {
        super::pdist::sample_gamma(alpha, theta, self)
    }
    fn sample_gamma_moments(&mut self, mean: f64, coeff_var: f64) -> Result<f64, DistError> {
        super::pdist::sample_gamma_moments(mean, coeff_var, self)
    }
    fn sample_pois(&mut self, mean: f64) -> Result<i32, DistError> {
        super::pdist::sample_pois(mean, self)
    }
    fn sample_unidouble(&mut self, a: f64, b: f64) -> Result<f64, DistError> {
        super::pdist::sample_unidouble(a, b, self)
    }
    fn sample_uniint(&mut self, a: i32, b: i32) -> Result<i32, DistError> {
        super::pdist::sample_uniint(a, b, self)
    }
}
