//! Load profiles: a time-varying load per usage scenario (`docs/spec/workloads.md`, rules
//! `LP-*`; `docs/guide/load-profile.md`).
//!
//! A [`LoadProfile`] is a list of step functions `(time, factor)`, each for some usage scenarios
//! (or all of them). From a point's time on, the scenario's load is the model's times the
//! point's factor, until the next point:
//!
//! * an **open workload** arrives `factor` times as often: each inter-arrival time the model
//!   draws is divided by the factor. At a step, the pending arrival is moved so that the rest
//!   of its interval passes at the new rate (the arrival process runs in operational time
//!   `∫ factor dt`, a time change of the model's renewal process; no extra random number);
//! * a **closed workload** has `round(population · factor)` users: new users start at the step,
//!   surplus users leave after their current run, oldest first (as a population change of the
//!   model, MC-5.2).
//!
//! The profile is part of the run configuration ([`crate::SimConfig::load_profile`]);
//! [`crate::Simulation::create`] resolves it against the compiled model. The steps are events of
//! the event list. A run without a profile is unchanged.

use crate::ir::CompiledModel;
use serde_json::Value;
use std::path::Path;

/// A load profile: step functions of a load factor over time, per usage scenario.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct LoadProfile {
    /// In order; a later entry overrides an earlier one for the scenarios it names.
    pub entries: Vec<LoadEntry>,
}

/// One step function of a [`LoadProfile`].
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct LoadEntry {
    /// The usage scenarios, by id or entity name; empty: every scenario.
    pub scenarios: Vec<String>,
    /// `(time in seconds, factor)`, times strictly increasing and not negative, factors finite
    /// and positive. Before the first point the factor is 1.
    pub points: Vec<(f64, f64)>,
}

/// The file names [`LoadProfile::load_dir`] reads.
pub const LOAD_PROFILE_SUFFIX: &str = ".loadprofile.json";

impl LoadProfile {
    /// One step function for every scenario.
    pub fn uniform(points: Vec<(f64, f64)>) -> LoadProfile {
        LoadProfile {
            entries: vec![LoadEntry {
                scenarios: Vec::new(),
                points,
            }],
        }
    }

    /// Reads a profile from JSON text (`docs/guide/load-profile.md`):
    ///
    /// ```json
    /// {"profiles": [
    ///   {"points": [[0, 1.0], [600, 4.0], [1200, 1.0]]},
    ///   {"scenarios": ["login"], "points": [[0, 1.0]]}
    /// ]}
    /// ```
    ///
    /// `{"points": [...]}` alone is one profile for every scenario. Unknown fields are errors.
    pub fn from_json(text: &str) -> Result<LoadProfile, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("load profile: {e}"))?;
        LoadProfile::from_value(&v)
    }

    /// [`LoadProfile::from_json`] on a parsed value.
    pub fn from_value(v: &Value) -> Result<LoadProfile, String> {
        let obj = v
            .as_object()
            .ok_or("load profile: expected a JSON object")?;
        let entries = if let Some(list) = obj.get("profiles") {
            if let Some(k) = obj.keys().find(|k| *k != "profiles") {
                return Err(format!("load profile: unknown field `{k}`"));
            }
            list.as_array()
                .ok_or("load profile: `profiles` must be an array")?
                .iter()
                .enumerate()
                .map(|(i, e)| entry(e, &format!("profiles[{i}]")))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![entry(v, "load profile")?]
        };
        let p = LoadProfile { entries };
        p.validate()?;
        Ok(p)
    }

    /// Reads a profile file.
    pub fn load_file(path: &Path) -> Result<LoadProfile, String> {
        let t = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        LoadProfile::from_json(&t).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The profile of a model directory: its `*.loadprofile.json` files, in name order, as one
    /// profile (their entries in that order); `None` without such a file.
    pub fn load_dir(dir: &Path) -> Result<Option<LoadProfile>, String> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(LOAD_PROFILE_SUFFIX))
            })
            .collect();
        if files.is_empty() {
            return Ok(None);
        }
        files.sort();
        let mut all = LoadProfile::default();
        for f in files {
            all.entries.extend(LoadProfile::load_file(&f)?.entries);
        }
        Ok(Some(all))
    }

    /// Checks the points (see [`LoadEntry::points`]).
    pub fn validate(&self) -> Result<(), String> {
        for (i, e) in self.entries.iter().enumerate() {
            let at = |j: usize| format!("load profile entry {i}, point {j}");
            if e.points.is_empty() {
                return Err(format!("load profile entry {i}: no points"));
            }
            let mut last = f64::NEG_INFINITY;
            for (j, &(t, f)) in e.points.iter().enumerate() {
                if !t.is_finite() || t < 0.0 {
                    return Err(format!(
                        "{}: time {t} must be finite and not negative",
                        at(j)
                    ));
                }
                if t <= last {
                    return Err(format!("{}: times must increase ({t} after {last})", at(j)));
                }
                if !f.is_finite() || f <= 0.0 {
                    return Err(format!("{}: factor {f} must be finite and positive", at(j)));
                }
                last = t;
            }
        }
        Ok(())
    }

    /// The step function of each usage scenario of `cm` (index of `cm.scenarios`), in seconds;
    /// empty for a scenario no entry names. A name that matches no scenario, or more than one
    /// by name, is an error.
    pub fn resolve(&self, cm: &CompiledModel) -> Result<Vec<Vec<(f64, f64)>>, String> {
        self.validate()?;
        let mut per: Vec<Vec<(f64, f64)>> = vec![Vec::new(); cm.scenarios.len()];
        for e in &self.entries {
            let targets: Vec<usize> = if e.scenarios.is_empty() {
                (0..cm.scenarios.len()).collect()
            } else {
                let mut v = Vec::new();
                for n in &e.scenarios {
                    let by_id: Vec<usize> = (0..cm.scenarios.len())
                        .filter(|&i| *cm.model.usage_scenarios[cm.scenarios[i].id].id == **n)
                        .collect();
                    let found = if by_id.is_empty() {
                        (0..cm.scenarios.len())
                            .filter(|&i| *cm.model.usage_scenarios[cm.scenarios[i].id].name == **n)
                            .collect()
                    } else {
                        by_id
                    };
                    match found.as_slice() {
                        [i] => v.push(*i),
                        [] => {
                            return Err(format!(
                                "load profile: no usage scenario with the id or name {n:?}"
                            ));
                        }
                        _ => {
                            return Err(format!(
                                "load profile: several usage scenarios are named {n:?}; name it by its id"
                            ));
                        }
                    }
                }
                v
            };
            for i in targets {
                per[i] = e.points.clone();
            }
        }
        Ok(per)
    }
}

fn entry(v: &Value, at: &str) -> Result<LoadEntry, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{at}: expected an object"))?;
    if let Some(k) = obj.keys().find(|k| *k != "points" && *k != "scenarios") {
        return Err(format!("{at}: unknown field `{k}`"));
    }
    let scenarios = match obj.get("scenarios") {
        None => Vec::new(),
        Some(Value::String(s)) if s == "*" => Vec::new(),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(String::from)
                    .ok_or_else(|| format!("{at}: `scenarios` holds names (strings)"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(format!("{at}: `scenarios` must be a name, a list or \"*\"")),
    };
    let points = obj
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{at}: `points` must be an array of [time, factor]"))?
        .iter()
        .map(|p| match p.as_array().map(Vec::as_slice) {
            Some([t, f]) => match (t.as_f64(), f.as_f64()) {
                (Some(t), Some(f)) => Ok((t, f)),
                _ => Err(format!("{at}: a point is [time, factor] (numbers)")),
            },
            _ => Err(format!("{at}: a point is [time, factor]")),
        })
        .collect::<Result<_, _>>()?;
    Ok(LoadEntry { scenarios, points })
}

/// The run state of a profile (`RunState::lp`): resolved points, the current factors and the
/// open workloads' pending arrivals.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct LoadRun {
    /// Per usage scenario (index of `CompiledModel::scenarios`): `(time, factor)` with the time
    /// in the engine's clock; empty: no profile.
    pub points: Vec<Vec<(i64, f64)>>,
    /// Per scenario: the index of the next point not yet applied.
    pub next: Vec<u32>,
    /// Per scenario: the current factor.
    pub factor: Vec<f64>,
    /// Per open-workload scenario: its generator's slot and pending arrival time.
    pub wake: Vec<Option<(u32, i64)>>,
}

impl LoadRun {
    /// The run state of `profile` on `cm`, before the start (the points at time 0 apply).
    pub fn new(
        profile: &LoadProfile,
        cm: &CompiledModel,
        at_secs: impl Fn(f64) -> i64,
    ) -> Result<LoadRun, String> {
        let per = profile.resolve(cm)?;
        let n = per.len();
        let mut r = LoadRun {
            points: per
                .into_iter()
                .map(|v| v.into_iter().map(|(t, f)| (at_secs(t), f)).collect())
                .collect(),
            next: vec![0; n],
            factor: vec![1.0; n],
            wake: vec![None; n],
        };
        for i in 0..n {
            while let Some(&(t, f)) = r.points[i].get(r.next[i] as usize)
                && t <= 0
            {
                r.factor[i] = f;
                r.next[i] += 1;
            }
        }
        Ok(r)
    }

    /// The factor of scenario `scen` now (1 for scenarios created after the start).
    #[inline]
    pub fn factor(&self, scen: usize) -> f64 {
        self.factor.get(scen).copied().unwrap_or(1.0)
    }

    /// The distinct times of the points not applied yet, ascending.
    pub fn pending_times(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self
            .points
            .iter()
            .zip(&self.next)
            .flat_map(|(p, &n)| p[n as usize..].iter().map(|x| x.0))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Applies the points due at `now`: the scenarios whose factor changes, with the old and
    /// the new factor.
    pub fn advance(&mut self, now: i64) -> Vec<(usize, f64, f64)> {
        let mut out = Vec::new();
        for i in 0..self.points.len() {
            let old = self.factor[i];
            let mut changed = false;
            while let Some(&(t, f)) = self.points[i].get(self.next[i] as usize)
                && t <= now
            {
                self.factor[i] = f;
                self.next[i] += 1;
                changed = true;
            }
            if changed && self.factor[i] != old {
                out.push((i, old, self.factor[i]));
            }
        }
        out
    }
}

/// The population of a closed workload under `factor` (rounded half away from zero).
pub(crate) fn scaled_population(population: i64, factor: f64) -> i64 {
    if factor == 1.0 {
        return population;
    }
    (population.max(0) as f64 * factor).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates() {
        let p = LoadProfile::from_json(
            r#"{"profiles":[{"points":[[0,1],[600,4.5]]},{"scenarios":["login"],"points":[[0,2]]}]}"#,
        )
        .unwrap();
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.entries[0].scenarios, Vec::<String>::new());
        assert_eq!(p.entries[0].points, vec![(0.0, 1.0), (600.0, 4.5)]);
        assert_eq!(p.entries[1].scenarios, vec!["login".to_string()]);
        let one = LoadProfile::from_json(r#"{"scenarios":"*","points":[[10,2]]}"#).unwrap();
        assert_eq!(one, LoadProfile::uniform(vec![(10.0, 2.0)]));
        for bad in [
            r#"{"points":[[0,1],[0,2]]}"#,
            r#"{"points":[[5,1],[2,2]]}"#,
            r#"{"points":[[-1,1]]}"#,
            r#"{"points":[[0,0]]}"#,
            r#"{"points":[[0,-2]]}"#,
            r#"{"points":[]}"#,
            r#"{"points":[[0]]}"#,
            r#"{"points":[[0,1]],"rate":2}"#,
            r#"{"profiles":[],"x":1}"#,
            r#"[1]"#,
        ] {
            assert!(LoadProfile::from_json(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn steps_advance_in_time() {
        let mut r = LoadRun {
            points: vec![
                vec![(0, 2.0), (10, 3.0), (20, 3.0)],
                vec![],
                vec![(10, 0.5)],
            ],
            next: vec![1, 0, 0],
            factor: vec![2.0, 1.0, 1.0],
            wake: vec![None; 3],
        };
        assert_eq!(r.pending_times(), vec![10, 20]);
        assert_eq!(r.advance(10), vec![(0, 2.0, 3.0), (2, 1.0, 0.5)]);
        assert_eq!(r.pending_times(), vec![20]);
        // an unchanged factor is no change
        assert_eq!(r.advance(20), vec![]);
        assert_eq!(r.factor(5), 1.0);
        assert_eq!(scaled_population(10, 0.25), 3);
        assert_eq!(scaled_population(10, 0.05), 1);
        assert_eq!(scaled_population(3, 1.0), 3);
    }
}
