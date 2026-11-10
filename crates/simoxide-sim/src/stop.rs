//! Why a run ended, in the words of `simoxide run --report` (`stop.kind`, `stop.cause`) and of
//! the server (`stop_reason.cause`): `docs/guide/result-report.md`, "Stop reasons". The simulator's
//! own end ([`completion`]) and the errors that abort a run ([`error`]); pauses are the
//! callers' (they asked for them).

use crate::{FlavourConfig, FlavourKind, Quirk, SimConfig, SimError, SimErrorKind};

/// What ended a run: a kind and its cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    /// `completed`, `limit`, `cancelled` or `error` (the pause kinds are the callers').
    pub kind: &'static str,
    /// The cause within the kind, e.g. `max_sim_time`, `max_events`, `timeout`.
    pub cause: &'static str,
}

/// Which of the simulator's own stop conditions ended a run it finished itself (at `end_ns`,
/// after `main_count` finished usage-scenario runs): `max_measurements`, `max_sim_time`, else
/// `no_events` (no event was left). The Slingshot flavours queue their stop at `max_sim_time`
/// as an event, so they end there also when nothing else is left.
pub fn completion(end_ns: i64, main_count: i64, cfg: &SimConfig) -> Stop {
    let cause = if cfg.max_measurements > 0 && main_count >= cfg.max_measurements {
        "max_measurements"
    } else if cfg.max_sim_time > 0 && end_ns >= cfg.max_sim_time.saturating_mul(1_000_000_000) {
        "max_sim_time"
    } else {
        "no_events"
    };
    Stop {
        kind: "completed",
        cause,
    }
}

/// `cfg` as a run of `flavour` applies it: the Slingshot-exact engine and Slingshot's Q32 have
/// no maximum measurement count (`simTime` is the only stop condition), so it is `-1` there.
/// Pass this to [`completion`].
pub fn effective_config(cfg: &SimConfig, flavour: &FlavourConfig) -> SimConfig {
    let mut c = cfg.clone();
    if flavour.kind == FlavourKind::SlingshotExact
        || (flavour.kind == FlavourKind::Slingshot && flavour.quirks.has(Quirk::Q32))
    {
        c.max_measurements = -1;
    }
    c
}

/// What an error that aborted a run was: a resource limit of [`crate::Limits`]
/// (`max_events`, `max_steps`, `max_processes`, `max_stack_depth`; `limit` for the engine's own
/// bounds), the livelock guard (`livelock`, [`SimConfig::max_events_per_instant`]), the
/// deadline (`timeout`, `--timeout`), the cancellation flag (`cancelled`), or an error of the
/// model or the run (`error`, `simulation`).
pub fn error(e: &SimError) -> Stop {
    let m = e.message.as_str();
    let (kind, cause) = match e.kind {
        SimErrorKind::Cancelled if m == DEADLINE => ("limit", "timeout"),
        SimErrorKind::Cancelled => ("cancelled", "cancelled"),
        SimErrorKind::Limit => ("limit", limit_cause(m)),
        SimErrorKind::Model | SimErrorKind::ModeUnavailable => ("error", "simulation"),
    };
    Stop { kind, cause }
}

/// The message of [`SimErrorKind::Cancelled`] when [`crate::Limits::deadline`] passed.
pub(crate) const DEADLINE: &str = "deadline passed";

/// The limit a [`SimErrorKind::Limit`] message names (the messages of both engines).
fn limit_cause(m: &str) -> &'static str {
    if m.contains("(Limits::max_events)") || m.starts_with("event limit ") {
        "max_events"
    } else if m.contains("(Limits::max_steps)") {
        "max_steps"
    } else if m.contains("(Limits::max_processes)") {
        "max_processes"
    } else if m.contains("(Limits::max_stack_depth") {
        "max_stack_depth"
    } else if m.starts_with("livelock") {
        "livelock"
    } else {
        "limit"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(kind: SimErrorKind, message: &str) -> Stop {
        error(&SimError {
            message: message.into(),
            at_ns: 0,
            kind,
        })
    }

    /// The messages of the limits, as both engines word them.
    #[test]
    fn limit_messages() {
        use SimErrorKind::*;
        let limit = |c| Stop {
            kind: "limit",
            cause: c,
        };
        for (m, c) in [
            (
                "limit exceeded: more than 10 events (Limits::max_events)",
                "max_events",
            ),
            ("event limit 10 reached", "max_events"),
            (
                "limit exceeded: more than 5 interpreter steps (Limits::max_steps)",
                "max_steps",
            ),
            (
                "limit exceeded: more than 5 handler events (Limits::max_steps)",
                "max_steps",
            ),
            (
                "limit exceeded: more than 3 live processes (Limits::max_processes)",
                "max_processes",
            ),
            (
                "limit exceeded: more than 3 live users (Limits::max_processes)",
                "max_processes",
            ),
            (
                "limit exceeded: process stack deeper than 9 (Limits::max_stack_depth; unbounded recursion?)",
                "max_stack_depth",
            ),
            (
                "limit exceeded: a user's stack has 9 frames (Limits::max_stack_depth; unbounded recursion?)",
                "max_stack_depth",
            ),
            (
                "livelock: 7 events at simulation time 1 s without progress towards a stop condition (REF-7; see --max-events-per-instant)",
                "livelock",
            ),
            ("livelock guard: more than 7 events at t = 1", "livelock"),
            (
                "limit exceeded: more than 64 nested evaluation proxies (INNER characterisations passed through nested calls)",
                "limit",
            ),
        ] {
            assert_eq!(err(Limit, m), limit(c), "{m}");
        }
        assert_eq!(err(Cancelled, DEADLINE), limit("timeout"));
        assert_eq!(
            err(Cancelled, "cancelled"),
            Stop {
                kind: "cancelled",
                cause: "cancelled"
            }
        );
        assert_eq!(
            err(Model, "IllegalStateException"),
            Stop {
                kind: "error",
                cause: "simulation"
            }
        );
    }

    #[test]
    fn completions() {
        let cfg = SimConfig {
            max_sim_time: 100,
            max_measurements: 50,
            ..SimConfig::default()
        };
        let c = |end_ns, n| completion(end_ns, n, &cfg).cause;
        assert_eq!(c(100_000_000_000, 10), "max_sim_time");
        assert_eq!(c(100_500_000_000, 10), "max_sim_time");
        assert_eq!(c(99_000_000_000, 50), "max_measurements");
        assert_eq!(c(99_000_000_000, 10), "no_events");
        // the Slingshot-exact engine counts no measurements (Q32)
        let x = effective_config(&cfg, &FlavourConfig::slingshot_exact());
        assert_eq!(completion(100_000_000_000, 60, &x).cause, "max_sim_time");
        let s = effective_config(&cfg, &FlavourConfig::slingshot(crate::Quirks::NONE));
        assert_eq!(completion(99_000_000_000, 60, &s).cause, "max_measurements");
    }
}
