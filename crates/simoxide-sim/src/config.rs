//! `run.json` (`docs/guide/formats.md` §5) and model loading.

use crate::sim::SimConfig;
use std::path::{Path, PathBuf};

/// A run specification as stored in `corpus/<model>/run.json`.
#[derive(Clone, Debug)]
pub struct RunSpec {
    pub seed: i64,
    pub max_measurements: i64,
    pub max_sim_time: i64,
    pub simulate_linking_resources: bool,
    pub simulate_throughput_of_linking_resources: bool,
    /// Optional entry files (relative to the model directory).
    pub usagemodel: Option<String>,
    pub allocation: Vec<String>,
    pub monitorrepository: Option<String>,
}

impl Default for RunSpec {
    fn default() -> Self {
        RunSpec {
            seed: 0,
            max_measurements: -1,
            max_sim_time: -1,
            simulate_linking_resources: false,
            simulate_throughput_of_linking_resources: true,
            usagemodel: None,
            allocation: Vec::new(),
            monitorrepository: None,
        }
    }
}

impl RunSpec {
    pub fn parse(text: &str) -> Result<RunSpec, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut r = RunSpec::default();
        let int = |k: &str, d: i64| v.get(k).and_then(|x| x.as_i64()).unwrap_or(d);
        let boolean = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
        r.seed = int("seed", 0);
        r.max_measurements = int("max_measurements", -1);
        r.max_sim_time = int("max_sim_time", -1);
        r.simulate_linking_resources = boolean("simulate_linking_resources", false);
        r.simulate_throughput_of_linking_resources =
            boolean("simulate_throughput_of_linking_resources", true);
        r.usagemodel = v
            .get("usagemodel")
            .and_then(|x| x.as_str())
            .map(String::from);
        r.monitorrepository = v
            .get("monitorrepository")
            .and_then(|x| x.as_str())
            .map(String::from);
        match v.get("allocation") {
            Some(serde_json::Value::String(s)) => r.allocation.push(s.clone()),
            Some(serde_json::Value::Array(a)) => {
                r.allocation = a
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            }
            _ => {}
        }
        Ok(r)
    }

    pub fn load(path: &Path) -> Result<RunSpec, String> {
        let t = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        RunSpec::parse(&t)
    }

    /// The simulation configuration of this run spec (own RNG).
    pub fn sim_config(&self, run_name: &str) -> SimConfig {
        SimConfig {
            run_name: run_name.to_string(),
            seed: self.seed,
            max_sim_time: self.max_sim_time,
            max_measurements: self.max_measurements,
            simulate_linking_resources: self.simulate_linking_resources,
            simulate_throughput_of_linking_resources: self.simulate_throughput_of_linking_resources,
            ..Default::default()
        }
    }

    /// Loads a model from memory, without touching the file system: `files` are (file name,
    /// XMI content) pairs, file names relative to one model directory (as in a model
    /// directory; `href`s between them resolve by name). Entry files are chosen like
    /// [`RunSpec::load_model`] does for a directory. `href`s to files that are not given fail to
    /// resolve (model error); the bundled `pathmap://PCM_MODELS/...` models are available.
    pub fn load_model_memory<N: AsRef<str>, C: AsRef<[u8]>>(
        &self,
        files: &[(N, C)],
    ) -> Result<simoxide_model::Model, String> {
        let mut names: Vec<&str> = files.iter().map(|(n, _)| n.as_ref()).collect();
        names.sort_unstable();
        let with_ext = |ext: &str| -> Vec<String> {
            names
                .iter()
                .filter(|n| Path::new(n).extension().is_some_and(|x| x == ext))
                .map(|n| n.to_string())
                .collect()
        };
        let mut entries: Vec<String> = Vec::new();
        match &self.usagemodel {
            Some(u) => entries.push(u.clone()),
            None => {
                let u = with_ext("usagemodel");
                if u.len() != 1 {
                    return Err(format!(
                        "expected exactly one .usagemodel, found {}",
                        u.len()
                    ));
                }
                entries.extend(u);
            }
        }
        if self.allocation.is_empty() {
            entries.extend(with_ext("allocation"));
        } else {
            entries.extend(self.allocation.iter().cloned());
        }
        match &self.monitorrepository {
            Some(m) => entries.push(m.clone()),
            None => entries.extend(with_ext("monitorrepository")),
        }
        let m = simoxide_model::load_memory_entries(
            files
                .iter()
                .map(|(n, c)| (n.as_ref().to_string(), c.as_ref().to_vec())),
            &entries,
        );
        model_errors(m)
    }

    /// Loads the model of a directory: the entry files named in the spec, or the directory's
    /// single `.usagemodel`, its `.allocation` files and optional `.monitorrepository`.
    pub fn load_model(&self, dir: &Path) -> Result<simoxide_model::Model, String> {
        let mut files: Vec<PathBuf> = Vec::new();
        let list = |ext: &str| -> Result<Vec<PathBuf>, String> {
            let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == ext))
                .collect();
            v.sort();
            Ok(v)
        };
        match &self.usagemodel {
            Some(u) => files.push(dir.join(u)),
            None => {
                let u = list("usagemodel")?;
                if u.len() != 1 {
                    return Err(format!(
                        "{}: expected exactly one .usagemodel, found {}",
                        dir.display(),
                        u.len()
                    ));
                }
                files.extend(u);
            }
        }
        if self.allocation.is_empty() {
            files.extend(list("allocation")?);
        } else {
            files.extend(self.allocation.iter().map(|a| dir.join(a)));
        }
        match &self.monitorrepository {
            Some(m) => files.push(dir.join(m)),
            None => files.extend(list("monitorrepository")?),
        }
        model_errors(simoxide_model::load_files(&files))
    }
}

/// The model, or its first error diagnostic.
fn model_errors(m: simoxide_model::Model) -> Result<simoxide_model::Model, String> {
    if let Some(e) = m
        .diagnostics
        .0
        .iter()
        .find(|d| d.level == simoxide_model::Level::Error)
    {
        return Err(format!("model error: {e}"));
    }
    Ok(m)
}
