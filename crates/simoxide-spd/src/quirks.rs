//! Compat flags of the SPD quirks Q1-Q16 (`DESIGN-SPD.md` §1.5, `docs/spec/slingshot/quirks.md`).
//!
//! Slingshot's bugs are fixed by default (U5, U13); each flag reproduces one quirk. The
//! Slingshot-exact mode turns all of them on ([`Quirks::slingshot`]). Fixed behaviours marked
//! *provisional* follow a proposal of the quirks page that the user has not confirmed yet.
//!
//! Flags of quirks that live in the adjustment (Q5, Q6, Q7, Q8, Q14, Q16) belong to the
//! adjustment phase (P2) and are not here, except Q4, whose arithmetic the in-chain group-size
//! check of the fixed Q9 shares.

/// The compat flags of the interpreter. `Default` = all fixed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Quirks {
    /// Q1: a disregard does not restore the adjustor state. No filter before the adjustor
    /// changes that state, so both settings behave the same (kept for completeness).
    pub q1_disregard_keeps_state: bool,
    /// Q2: the adjustor's state change (latest adjustment, cooldown, the group's enacted
    /// policies) happens when the request is made. Fixed: it is committed only by a successful
    /// adjustment ([`crate::Spd::adjusted`]).
    pub q2_commit_on_request: bool,
    /// Q4: a relative adjustment whose percentage gives 0 ignores `minAdjustmentValue`
    /// (`min(0, min)`). Fixed (provisional): a zero change stays zero, otherwise its magnitude
    /// is at least `minAdjustmentValue` in the direction of the percentage. Used by the
    /// group-size check of the fixed Q9 (and by the adjustment in P2).
    pub q4_relative_zero: bool,
    /// Q9: only thrashing constraints are target constraints of the chain. Fixed
    /// (provisional): `TargetGroupSizeConstraint`s are checked in the chain too, so a policy
    /// whose change would be clamped to zero is disregarded.
    pub q9_no_size_check: bool,
    /// Q11: an unsupported stimulus or trigger (and any other set-up error: wrong expectation
    /// type, unsupported constraint) throws in Slingshot's set-up handler, which drops the
    /// whole SPD. Fixed: [`crate::Spd::new`] returns a configuration error naming it.
    pub q11_setup_error_drops_spd: bool,
    /// Q12: composed triggers behave as Slingshot's filters do (AND resumes after a failed
    /// child, OR never evaluates its children and parks the chain, XOR inverts after the first
    /// child), and a policy whose trigger has two measurement stimulus kinds runs its chain
    /// twice per measurement. Fixed: AND / OR / XOR (exactly one) of the children's results,
    /// every child evaluated on every event, one evaluation per event.
    pub q12_broken_composed: bool,
    /// Q13: samples of all elements of the group feed one window; `getResult` runs for every
    /// measurement that reaches the checker, also of other metrics; reading the value consumes
    /// the EMA's emit slot even if a later filter disregards. Fixed (provisional): one window
    /// per element (measuring point), the value is the aggregation method over the elements'
    /// window values; only measurements of the stimulus' metric are evaluated; the emit slot is
    /// consumed only when the policy fires.
    pub q13_merged_samples: bool,
    /// Q15: every policy of a target group receives every `SimulationTimeReached` of the group.
    /// Fixed: a policy receives only its own time events.
    pub q15_shared_time_events: bool,
}

impl Quirks {
    /// All fixed (the default).
    pub fn fixed() -> Quirks {
        Quirks::default()
    }

    /// All quirks on: Slingshot's behaviour (the Slingshot-exact mode, comparisons with the
    /// reference runner).
    pub fn slingshot() -> Quirks {
        Quirks {
            q1_disregard_keeps_state: true,
            q2_commit_on_request: true,
            q4_relative_zero: true,
            q9_no_size_check: true,
            q11_setup_error_drops_spd: true,
            q12_broken_composed: true,
            q13_merged_samples: true,
            q15_shared_time_events: true,
        }
    }

    /// Parses `fixed`, `slingshot`, or a comma-separated list of quirks to turn on (`Q2,Q12`,
    /// case-insensitive; `Q3` is accepted and changes nothing: kept behaviour).
    pub fn parse(s: &str) -> Result<Quirks, String> {
        match s {
            "fixed" | "" => return Ok(Quirks::fixed()),
            "slingshot" | "all" => return Ok(Quirks::slingshot()),
            _ => {}
        }
        let mut q = Quirks::fixed();
        for part in s.split(',') {
            match part.trim().to_ascii_uppercase().as_str() {
                "Q1" => q.q1_disregard_keeps_state = true,
                "Q2" => q.q2_commit_on_request = true,
                "Q3" => {}
                "Q4" => q.q4_relative_zero = true,
                "Q9" => q.q9_no_size_check = true,
                "Q11" => q.q11_setup_error_drops_spd = true,
                "Q12" => q.q12_broken_composed = true,
                "Q13" => q.q13_merged_samples = true,
                "Q15" => q.q15_shared_time_events = true,
                other => {
                    return Err(format!(
                        "unknown SPD quirk {other:?} (expected fixed, slingshot or a list of \
                         Q1, Q2, Q3, Q4, Q9, Q11, Q12, Q13, Q15)"
                    ));
                }
            }
        }
        Ok(q)
    }
}
