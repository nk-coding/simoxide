//! The distribution samplers of the Slingshot-exact mode: Palladio 6.0's
//! ([`simoxide_random::p6`], Commons Math 3), the probability-function library the reference runs
//! on, instead of SimuLizar 5.2.2's ([`simoxide_random::dist`]).

pub(crate) use simoxide_random::p6::{
    sample_exp, sample_gamma, sample_gamma_moments, sample_lognorm, sample_lognorm_moments,
    sample_norm, sample_pois, sample_unidouble, sample_uniint,
};
