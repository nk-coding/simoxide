//! The HPA configuration file (`docs/guide/hpa.md`): Kubernetes `autoscaling/v2`
//! `HorizontalPodAutoscaler` objects (the subset SimOxide models), the controller's settings
//! (the `kube-controller-manager` flags) and SimOxide's additions per autoscaler.
//!
//! ```json
//! {
//!   "controller": {"syncPeriod": "15s", "tolerance": 0.1, "downscaleStabilization": "5m"},
//!   "items": [{
//!     "apiVersion": "autoscaling/v2", "kind": "HorizontalPodAutoscaler",
//!     "metadata": {"name": "app"},
//!     "spec": {
//!       "scaleTargetRef": {"kind": "ElasticInfrastructure", "name": "AppServer group"},
//!       "minReplicas": 1, "maxReplicas": 8,
//!       "metrics": [{"type": "Resource", "resource": {"name": "cpu",
//!                    "target": {"type": "Utilization", "averageUtilization": 60}}}],
//!       "behavior": {"scaleDown": {"stabilizationWindowSeconds": 120}}
//!     },
//!     "simoxide": {"startupDelay": "20s"}
//!   }]
//! }
//! ```
//!
//! A file may also hold one `HorizontalPodAutoscaler` or a JSON array of them. Durations are
//! seconds (numbers) or Go duration strings (`"90s"`, `"1m30s"`, `"5m0s"`).

use super::controller::{Behavior, PolicyKind, Rules, SEC, ScalingPolicy, Select, Spec};
use serde_json::{Map, Value};

/// The controller's settings (`kube-controller-manager --horizontal-pod-autoscaler-*`).
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerConfig {
    /// `--horizontal-pod-autoscaler-sync-period` (15 s).
    pub sync_period_ns: i64,
    /// `--horizontal-pod-autoscaler-tolerance` (0.1).
    pub tolerance: f64,
    /// `--horizontal-pod-autoscaler-downscale-stabilization` (5 min).
    pub downscale_stabilization_ns: i64,
    /// `--horizontal-pod-autoscaler-cpu-initialization-period` (5 min).
    pub cpu_initialization_period_ns: i64,
    /// SimOxide: the most pods all autoscalers of a run may create (default 20 000). Every
    /// created pod stays in the run's model as a tombstone after it is removed (the change path
    /// keeps indices stable): about 45 KB, and every later model change rebuilds it
    /// (`DESIGN-SPD.md` §5.10). The run stops with an error beyond the limit.
    pub max_created_pods: u64,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        ControllerConfig {
            sync_period_ns: 15 * SEC,
            tolerance: 0.1,
            downscale_stabilization_ns: 300 * SEC,
            cpu_initialization_period_ns: 300 * SEC,
            max_created_pods: 20_000,
        }
    }
}

/// `spec.scaleTargetRef`: an SPD target group by name or id (`kind`, if given, its class:
/// `ElasticInfrastructure` or `ServiceGroup`).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetRef {
    pub kind: Option<String>,
    pub name: String,
}

/// One autoscaler.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoscalerConfig {
    /// `metadata.name`.
    pub name: String,
    pub target: TargetRef,
    /// The controller's view of the spec (HPA-2 to HPA-5).
    pub spec: Spec,
    /// SimOxide: time from a pod's creation until it is ready and takes requests (HPA-6;
    /// default 0: at once).
    pub startup_delay_ns: i64,
    /// SimOxide: the CPU metric's window, ending at the sync (HPA-4; default: the sync period).
    pub metric_window_ns: i64,
    /// SimOxide: the first sync (default: one sync period after the start).
    pub first_sync_ns: i64,
    /// SimOxide: the CPU request per pod in cores (default: the pod's CPU cores, its
    /// processing resources' `numberOfReplicas`).
    pub cpu_request: Option<f64>,
    pub sync_period_ns: i64,
    pub cpu_initialization_period_ns: i64,
    /// The controller's [`ControllerConfig::max_created_pods`].
    pub max_created_pods: u64,
}

/// A parsed configuration file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HpaConfig {
    pub controller: ControllerConfig,
    pub autoscalers: Vec<AutoscalerConfig>,
}

fn err<T>(at: &str, m: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("{at}: {m}"))
}

/// A duration: seconds, or a Go duration string (`1h2m3.5s`, `300ms`).
pub fn duration_ns(v: &Value, at: &str) -> Result<i64, String> {
    match v {
        Value::Number(n) => {
            let s = n.as_f64().unwrap_or(f64::NAN);
            if !(s.is_finite() && s >= 0.0) {
                return err(at, format!("not a duration: {n}"));
            }
            Ok((s * 1e9).round() as i64)
        }
        Value::String(s) => {
            parse_go_duration(s).ok_or_else(|| format!("{at}: not a duration: {s:?}"))
        }
        _ => err(
            at,
            "a duration is a number of seconds or a string such as \"15s\"",
        ),
    }
}

fn parse_go_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s == "0" {
        return Some(0);
    }
    let mut total = 0f64;
    let mut rest = s;
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        let num_end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if num_end == 0 {
            return None;
        }
        let n: f64 = rest[..num_end].parse().ok()?;
        rest = &rest[num_end..];
        let unit_end = rest
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(rest.len());
        let scale = match &rest[..unit_end] {
            "h" => 3600e9,
            "m" => 60e9,
            "s" => 1e9,
            "ms" => 1e6,
            "us" | "µs" => 1e3,
            "ns" => 1.0,
            _ => return None,
        };
        total += n * scale;
        rest = &rest[unit_end..];
    }
    Some(total.round() as i64)
}

fn obj<'a>(v: &'a Value, at: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object()
        .ok_or_else(|| format!("{at}: expected an object"))
}

fn known(m: &Map<String, Value>, at: &str, keys: &[&str]) -> Result<(), String> {
    for k in m.keys() {
        if !keys.contains(&k.as_str()) {
            return err(
                at,
                format!(
                    "unknown or unsupported field {k:?} (known: {})",
                    keys.join(", ")
                ),
            );
        }
    }
    Ok(())
}

fn int(v: &Value, at: &str) -> Result<i32, String> {
    v.as_i64()
        .and_then(|i| i32::try_from(i).ok())
        .ok_or_else(|| format!("{at}: expected an integer"))
}

fn num(v: &Value, at: &str) -> Result<f64, String> {
    v.as_f64().ok_or_else(|| format!("{at}: expected a number"))
}

fn string(v: &Value, at: &str) -> Result<String, String> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{at}: expected a string"))
}

impl HpaConfig {
    /// Parses a configuration file's text (see the module documentation).
    pub fn from_json(text: &str) -> Result<HpaConfig, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("HPA configuration: {e}"))?;
        Self::from_value(&v)
    }

    pub fn from_value(v: &Value) -> Result<HpaConfig, String> {
        let mut controller = ControllerConfig::default();
        let items: Vec<&Value> = match v {
            Value::Array(a) => a.iter().collect(),
            Value::Object(m) if m.contains_key("items") || m.contains_key("controller") => {
                known(
                    m,
                    "HPA configuration",
                    &["apiVersion", "kind", "controller", "items"],
                )?;
                if let Some(c) = m.get("controller") {
                    controller = parse_controller(c)?;
                }
                match m.get("items") {
                    None => Vec::new(),
                    Some(Value::Array(a)) => a.iter().collect(),
                    Some(_) => return err("items", "expected an array"),
                }
            }
            Value::Object(_) => vec![v],
            _ => return err("HPA configuration", "expected an object or an array"),
        };
        let mut autoscalers = Vec::with_capacity(items.len());
        for (i, it) in items.into_iter().enumerate() {
            let a = parse_hpa(it, &controller, &format!("items[{i}]"))?;
            if autoscalers
                .iter()
                .any(|b: &AutoscalerConfig| b.name == a.name)
            {
                return err(
                    &format!("items[{i}]"),
                    format!("two autoscalers named {:?}", a.name),
                );
            }
            autoscalers.push(a);
        }
        Ok(HpaConfig {
            controller,
            autoscalers,
        })
    }

    /// The configuration files of a model directory: `*.hpa` and `*.hpa.json`, in name order,
    /// merged (the controller settings of the first file that has them).
    pub fn load_dir(dir: &std::path::Path) -> Result<Option<HpaConfig>, String> {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".hpa") || n.ends_with(".hpa.json"))
            })
            .collect();
        files.sort();
        if files.is_empty() {
            return Ok(None);
        }
        let mut all: Option<HpaConfig> = None;
        for f in files {
            let c = Self::load_file(&f)?;
            match &mut all {
                None => all = Some(c),
                Some(a) => {
                    for x in c.autoscalers {
                        if a.autoscalers.iter().any(|b| b.name == x.name) {
                            return Err(format!(
                                "{}: two autoscalers named {:?}",
                                f.display(),
                                x.name
                            ));
                        }
                        a.autoscalers.push(x);
                    }
                }
            }
        }
        Ok(all)
    }

    /// One configuration file.
    pub fn load_file(path: &std::path::Path) -> Result<HpaConfig, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn parse_controller(v: &Value) -> Result<ControllerConfig, String> {
    let at = "controller";
    let m = obj(v, at)?;
    known(
        m,
        at,
        &[
            "syncPeriod",
            "tolerance",
            "downscaleStabilization",
            "cpuInitializationPeriod",
            "initialReadinessDelay",
            "maxCreatedPods",
        ],
    )?;
    let mut c = ControllerConfig::default();
    if let Some(x) = m.get("syncPeriod") {
        c.sync_period_ns = duration_ns(x, "controller.syncPeriod")?;
        if c.sync_period_ns <= 0 {
            return err("controller.syncPeriod", "must be positive");
        }
    }
    if let Some(x) = m.get("tolerance") {
        c.tolerance = num(x, "controller.tolerance")?;
        if c.tolerance.is_nan() || c.tolerance < 0.0 {
            return err("controller.tolerance", "must be >= 0");
        }
    }
    if let Some(x) = m.get("downscaleStabilization") {
        c.downscale_stabilization_ns = duration_ns(x, "controller.downscaleStabilization")?;
    }
    if let Some(x) = m.get("cpuInitializationPeriod") {
        c.cpu_initialization_period_ns = duration_ns(x, "controller.cpuInitializationPeriod")?;
    }
    if let Some(x) = m.get("maxCreatedPods") {
        c.max_created_pods = x
            .as_u64()
            .ok_or("controller.maxCreatedPods: expected a non-negative integer")?;
    }
    if let Some(x) = m.get("initialReadinessDelay") {
        // accepted for completeness: a starting pod is never ready before its start-up delay,
        // so the delay never matters (docs/spec/hpa.md HPA-6.3)
        duration_ns(x, "controller.initialReadinessDelay")?;
    }
    Ok(c)
}

fn parse_hpa(v: &Value, c: &ControllerConfig, at: &str) -> Result<AutoscalerConfig, String> {
    let m = obj(v, at)?;
    known(
        m,
        at,
        &[
            "apiVersion",
            "kind",
            "metadata",
            "spec",
            "simoxide",
            "status",
        ],
    )?;
    if let Some(k) = m.get("kind")
        && k.as_str() != Some("HorizontalPodAutoscaler")
    {
        return err(at, format!("kind {k} is not HorizontalPodAutoscaler"));
    }
    if let Some(a) = m.get("apiVersion")
        && a.as_str() != Some("autoscaling/v2")
    {
        return err(at, format!("apiVersion {a} is not autoscaling/v2"));
    }
    let name = m
        .get("metadata")
        .and_then(|md| md.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{at}: metadata.name is missing"))?
        .to_string();
    let at = format!("HorizontalPodAutoscaler {name:?}");
    let at = at.as_str();
    let spec = obj(
        m.get("spec")
            .ok_or_else(|| format!("{at}: spec is missing"))?,
        &format!("{at}: spec"),
    )?;
    known(
        spec,
        &format!("{at}: spec"),
        &[
            "scaleTargetRef",
            "minReplicas",
            "maxReplicas",
            "metrics",
            "behavior",
        ],
    )?;
    let tr = obj(
        spec.get("scaleTargetRef")
            .ok_or_else(|| format!("{at}: spec.scaleTargetRef is missing"))?,
        &format!("{at}: spec.scaleTargetRef"),
    )?;
    known(
        tr,
        &format!("{at}: spec.scaleTargetRef"),
        &["apiVersion", "kind", "name"],
    )?;
    let target = TargetRef {
        kind: tr.get("kind").and_then(Value::as_str).map(str::to_string),
        name: string(
            tr.get("name")
                .ok_or_else(|| format!("{at}: spec.scaleTargetRef.name is missing"))?,
            &format!("{at}: spec.scaleTargetRef.name"),
        )?,
    };
    let min_replicas = match spec.get("minReplicas") {
        Some(v) => int(v, &format!("{at}: spec.minReplicas"))?,
        None => 1,
    };
    let max_replicas = int(
        spec.get("maxReplicas")
            .ok_or_else(|| format!("{at}: spec.maxReplicas is missing"))?,
        &format!("{at}: spec.maxReplicas"),
    )?;
    if min_replicas < 1 {
        return err(
            at,
            "minReplicas must be at least 1 (scaling to zero needs object or external metrics)",
        );
    }
    if max_replicas < min_replicas {
        return err(at, "maxReplicas must be at least minReplicas");
    }
    // `autoscaling.DefaultCPUUtilization`: 80 % when no metric is given
    let mut target_utilization = 80;
    if let Some(ms) = spec.get("metrics") {
        let ms = ms
            .as_array()
            .ok_or_else(|| format!("{at}: spec.metrics: expected an array"))?;
        if ms.len() != 1 {
            return err(
                at,
                "spec.metrics: exactly one metric (CPU utilisation) is supported",
            );
        }
        target_utilization = parse_metric(&ms[0], &format!("{at}: spec.metrics[0]"))?;
    }
    let behavior = match spec.get("behavior") {
        None | Some(Value::Null) => None,
        Some(b) => Some(parse_behavior(b, c, &format!("{at}: spec.behavior"))?),
    };
    let mut a = AutoscalerConfig {
        name: name.clone(),
        target,
        spec: Spec {
            min_replicas,
            max_replicas,
            target_utilization,
            behavior,
            tolerance: c.tolerance,
            downscale_stabilization_ns: c.downscale_stabilization_ns,
        },
        startup_delay_ns: 0,
        metric_window_ns: c.sync_period_ns,
        first_sync_ns: c.sync_period_ns,
        cpu_request: None,
        sync_period_ns: c.sync_period_ns,
        cpu_initialization_period_ns: c.cpu_initialization_period_ns,
        max_created_pods: c.max_created_pods,
    };
    if let Some(x) = m.get("simoxide") {
        let at = format!("{at}: simoxide");
        let sx = obj(x, &at)?;
        known(
            sx,
            &at,
            &["startupDelay", "metricWindow", "firstSync", "cpuRequest"],
        )?;
        if let Some(v) = sx.get("startupDelay") {
            a.startup_delay_ns = duration_ns(v, &format!("{at}.startupDelay"))?;
        }
        if let Some(v) = sx.get("metricWindow") {
            a.metric_window_ns = duration_ns(v, &format!("{at}.metricWindow"))?;
        }
        if let Some(v) = sx.get("firstSync") {
            a.first_sync_ns = duration_ns(v, &format!("{at}.firstSync"))?;
        }
        if let Some(v) = sx.get("cpuRequest") {
            let r = num(v, &format!("{at}.cpuRequest"))?;
            if !(r > 0.0 && r.is_finite()) {
                return err(&at, "cpuRequest must be a positive number of cores");
            }
            a.cpu_request = Some(r);
        }
    }
    a.validate()?;
    Ok(a)
}

impl AutoscalerConfig {
    /// The constraints between the settings (also after a change file changed them).
    pub fn validate(&self) -> Result<(), String> {
        let at = format!("HorizontalPodAutoscaler {:?}", self.name);
        if self.metric_window_ns <= 0 || self.metric_window_ns > self.sync_period_ns {
            return err(
                &at,
                "the metric window must be positive and at most the sync period",
            );
        }
        if self.first_sync_ns < self.metric_window_ns {
            return err(
                &at,
                "the first sync must be at least one metric window after the start",
            );
        }
        if self.spec.min_replicas < 1 || self.spec.max_replicas < self.spec.min_replicas {
            return err(&at, "need 1 <= minReplicas <= maxReplicas");
        }
        if self.spec.target_utilization <= 0 {
            return err(&at, "the target utilisation must be positive");
        }
        Ok(())
    }
}

fn parse_metric(v: &Value, at: &str) -> Result<i32, String> {
    let m = obj(v, at)?;
    if m.get("type").and_then(Value::as_str) != Some("Resource") {
        return err(at, "only a Resource metric (cpu) is supported");
    }
    let r = obj(
        m.get("resource")
            .ok_or_else(|| format!("{at}: resource is missing"))?,
        &format!("{at}.resource"),
    )?;
    if r.get("name").and_then(Value::as_str) != Some("cpu") {
        return err(at, "only the resource cpu is supported");
    }
    let t = obj(
        r.get("target")
            .ok_or_else(|| format!("{at}: resource.target is missing"))?,
        &format!("{at}.resource.target"),
    )?;
    if t.get("type").and_then(Value::as_str) != Some("Utilization") {
        return err(at, "only a target of type Utilization is supported");
    }
    let u = int(
        t.get("averageUtilization")
            .ok_or_else(|| format!("{at}: averageUtilization is missing"))?,
        &format!("{at}.resource.target.averageUtilization"),
    )?;
    if u <= 0 {
        return err(at, "averageUtilization must be positive");
    }
    Ok(u)
}

fn parse_behavior(v: &Value, c: &ControllerConfig, at: &str) -> Result<Behavior, String> {
    let m = obj(v, at)?;
    known(m, at, &["scaleUp", "scaleDown"])?;
    let up = parse_rules(
        m.get("scaleUp"),
        Rules::default_up(),
        &format!("{at}.scaleUp"),
    )?;
    let down = parse_rules(
        m.get("scaleDown"),
        Rules::default_down(c.downscale_stabilization_ns),
        &format!("{at}.scaleDown"),
    )?;
    Ok(Behavior { up, down })
}

/// `GenerateHPAScale{Up,Down}Rules`: the given fields over the defaults.
fn parse_rules(v: Option<&Value>, mut r: Rules, at: &str) -> Result<Rules, String> {
    let Some(v) = v else {
        return Ok(r);
    };
    if v.is_null() {
        return Ok(r);
    }
    let m = obj(v, at)?;
    known(
        m,
        at,
        &[
            "stabilizationWindowSeconds",
            "selectPolicy",
            "policies",
            "tolerance",
        ],
    )?;
    if let Some(w) = m.get("stabilizationWindowSeconds") {
        let s = int(w, &format!("{at}.stabilizationWindowSeconds"))?;
        if !(0..=3600).contains(&s) {
            return err(at, "stabilizationWindowSeconds must be in 0..=3600");
        }
        r.stabilization_ns = i64::from(s) * SEC;
    }
    if let Some(s) = m.get("selectPolicy") {
        r.select = match s.as_str() {
            Some("Max") => Select::Max,
            Some("Min") => Select::Min,
            Some("Disabled") => Select::Disabled,
            _ => return err(at, "selectPolicy is Max, Min or Disabled"),
        };
    }
    if let Some(t) = m.get("tolerance") {
        let t = num(t, &format!("{at}.tolerance"))?;
        if t.is_nan() || t < 0.0 {
            return err(at, "tolerance must be >= 0");
        }
        r.tolerance = Some(t);
    }
    if let Some(ps) = m.get("policies") {
        let ps = ps
            .as_array()
            .ok_or_else(|| format!("{at}.policies: expected an array"))?;
        let mut v = Vec::new();
        for (i, p) in ps.iter().enumerate() {
            let at = format!("{at}.policies[{i}]");
            let pm = obj(p, &at)?;
            known(pm, &at, &["type", "value", "periodSeconds"])?;
            let kind = match pm.get("type").and_then(Value::as_str) {
                Some("Pods") => PolicyKind::Pods,
                Some("Percent") => PolicyKind::Percent,
                _ => return err(&at, "type is Pods or Percent"),
            };
            let value = int(
                pm.get("value").unwrap_or(&Value::Null),
                &format!("{at}.value"),
            )?;
            let period = int(
                pm.get("periodSeconds").unwrap_or(&Value::Null),
                &format!("{at}.periodSeconds"),
            )?;
            if value <= 0 || !(1..=1800).contains(&period) {
                return err(&at, "value must be positive and periodSeconds in 1..=1800");
            }
            v.push(ScalingPolicy {
                kind,
                value,
                period_ns: i64::from(period) * SEC,
            });
        }
        if v.is_empty() {
            return err(at, "policies must not be empty");
        }
        r.policies = v;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_go_duration("15s"), Some(15 * SEC));
        assert_eq!(parse_go_duration("5m0s"), Some(300 * SEC));
        assert_eq!(parse_go_duration("1m30s"), Some(90 * SEC));
        assert_eq!(parse_go_duration("1.5h"), Some(5400 * SEC));
        assert_eq!(parse_go_duration("300ms"), Some(300_000_000));
        assert_eq!(parse_go_duration("15"), None);
        assert_eq!(duration_ns(&serde_json::json!(2.5), "x"), Ok(2_500_000_000));
    }

    #[test]
    fn a_kubernetes_manifest_with_defaults() {
        let c = HpaConfig::from_json(
            r#"{"apiVersion":"autoscaling/v2","kind":"HorizontalPodAutoscaler",
                "metadata":{"name":"web"},
                "spec":{"scaleTargetRef":{"kind":"ElasticInfrastructure","name":"g"},
                        "maxReplicas":5,
                        "behavior":{"scaleDown":{"policies":[{"type":"Pods","value":1,"periodSeconds":60}]}}}}"#,
        )
        .unwrap();
        let a = &c.autoscalers[0];
        assert_eq!((a.spec.min_replicas, a.spec.max_replicas), (1, 5));
        assert_eq!(a.spec.target_utilization, 80);
        let b = a.spec.behavior.as_ref().unwrap();
        assert_eq!(b.up, Rules::default_up());
        assert_eq!(
            b.down.stabilization_ns,
            300 * SEC,
            "the controller's window"
        );
        assert_eq!(b.down.policies.len(), 1);
        assert_eq!(b.down.policies[0].kind, PolicyKind::Pods);
        assert_eq!((a.metric_window_ns, a.first_sync_ns), (15 * SEC, 15 * SEC));
    }

    #[test]
    fn errors_name_the_field() {
        let e = HpaConfig::from_json(
            r#"[{"metadata":{"name":"x"},"spec":{"scaleTargetRef":{"name":"g"},"maxReplicas":3,
                 "metrics":[{"type":"Pods"}]}}]"#,
        )
        .unwrap_err();
        assert!(e.contains("Resource metric"), "{e}");
        let e = HpaConfig::from_json(r#"{"controller":{"syncPeriod":"15s","bogus":1},"items":[]}"#)
            .unwrap_err();
        assert!(e.contains("bogus"), "{e}");
        let e = HpaConfig::from_json(
            r#"{"items":[{"metadata":{"name":"x"},"spec":{"scaleTargetRef":{"name":"g"},"maxReplicas":3},
                "simoxide":{"metricWindow":"30s"}}]}"#,
        )
        .unwrap_err();
        assert!(e.contains("metric window"), "{e}");
    }
}
