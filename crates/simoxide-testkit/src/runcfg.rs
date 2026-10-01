//! `run.json` (`docs/guide/formats.md` §5).

use std::path::Path;

use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunConfig {
    pub seed: i64,
    /// -1 = off.
    pub max_measurements: i64,
    /// Seconds, -1 = off.
    pub max_sim_time: i64,
    pub simulate_linking_resources: bool,
    pub simulate_throughput_of_linking_resources: bool,
    pub usagemodel: Option<String>,
    pub allocation: Option<Vec<String>>,
    pub monitorrepository: Option<String>,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            seed: 1,
            max_measurements: 100,
            max_sim_time: -1,
            simulate_linking_resources: false,
            simulate_throughput_of_linking_resources: true,
            usagemodel: None,
            allocation: None,
            monitorrepository: None,
        }
    }
}

impl RunConfig {
    pub fn parse(text: &str) -> Result<RunConfig, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let o = v.as_object().ok_or("run.json: not an object")?;
        let int = |k: &str, d: i64| -> Result<i64, String> {
            match o.get(k) {
                None | Some(Value::Null) => Ok(d),
                Some(v) => v.as_i64().ok_or(format!("run.json: {k} is not an integer")),
            }
        };
        let boolean = |k: &str, d: bool| -> Result<bool, String> {
            match o.get(k) {
                None | Some(Value::Null) => Ok(d),
                Some(v) => v.as_bool().ok_or(format!("run.json: {k} is not a boolean")),
            }
        };
        let s = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
        Ok(RunConfig {
            seed: int("seed", 1)?,
            max_measurements: int("max_measurements", -1)?,
            max_sim_time: int("max_sim_time", -1)?,
            simulate_linking_resources: boolean("simulate_linking_resources", false)?,
            simulate_throughput_of_linking_resources: boolean(
                "simulate_throughput_of_linking_resources",
                true,
            )?,
            usagemodel: s("usagemodel"),
            allocation: o.get("allocation").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            }),
            monitorrepository: s("monitorrepository"),
        })
    }

    pub fn load(path: impl AsRef<Path>) -> Result<RunConfig, String> {
        let p = path.as_ref();
        let t = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        Self::parse(&t)
    }

    /// Serialized like the reference's hand-made `run.json` (optional keys appended).
    pub fn to_json(&self) -> String {
        let mut s = format!(
            "{{\n  \"seed\": {},\n  \"max_measurements\": {},\n  \"max_sim_time\": {},\n  \
             \"simulate_linking_resources\": {},\n  \"simulate_throughput_of_linking_resources\": {}",
            self.seed,
            self.max_measurements,
            self.max_sim_time,
            self.simulate_linking_resources,
            self.simulate_throughput_of_linking_resources
        );
        let q = |x: &str| Value::String(x.to_string()).to_string();
        if let Some(u) = &self.usagemodel {
            s.push_str(&format!(",\n  \"usagemodel\": {}", q(u)));
        }
        if let Some(a) = &self.allocation {
            let items: Vec<String> = a.iter().map(|x| q(x)).collect();
            s.push_str(&format!(",\n  \"allocation\": [{}]", items.join(", ")));
        }
        if let Some(m) = &self.monitorrepository {
            s.push_str(&format!(",\n  \"monitorrepository\": {}", q(m)));
        }
        s.push_str("\n}\n");
        s
    }

    /// As a JSON object (for embedding).
    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("seed".into(), self.seed.into());
        m.insert("max_measurements".into(), self.max_measurements.into());
        m.insert("max_sim_time".into(), self.max_sim_time.into());
        m.insert(
            "simulate_linking_resources".into(),
            self.simulate_linking_resources.into(),
        );
        m.insert(
            "simulate_throughput_of_linking_resources".into(),
            self.simulate_throughput_of_linking_resources.into(),
        );
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let t = "{\n  \"seed\": 10,\n  \"max_measurements\": 50,\n  \"max_sim_time\": -1,\n  \
                 \"simulate_linking_resources\": false,\n  \"simulate_throughput_of_linking_resources\": true\n}\n";
        let c = RunConfig::parse(t).unwrap();
        assert_eq!(c.seed, 10);
        assert_eq!(c.to_json(), t);
        let c2 = RunConfig {
            allocation: Some(vec!["a.allocation".into()]),
            ..c
        };
        assert_eq!(RunConfig::parse(&c2.to_json()).unwrap(), c2);
    }
}
