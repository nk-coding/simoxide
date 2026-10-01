//! The generation algorithm (see the module docs of [`super`]).

use std::collections::BTreeMap;

use super::model::*;
use super::{Features, GenConfig, Rng};
use crate::javafmt;
use crate::runcfg::RunConfig;

/// Expected-load bookkeeping key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Proc(usize, ResType),
    Link(usize),
    /// Expected number of executed SEFF actions (run-time budget, not a resource).
    Actions,
}

type Cost = BTreeMap<Key, f64>;

fn add(c: &mut Cost, o: &Cost, f: f64) {
    for (k, v) in o {
        *c.entry(*k).or_insert(0.0) += v * f;
    }
}

/// Expected service time (resources and links).
fn total(c: &Cost) -> f64 {
    c.iter()
        .filter(|(k, _)| **k != Key::Actions)
        .map(|(_, v)| v)
        .sum()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum VarKind {
    Int,
    Bytes,
}

#[derive(Clone, Debug)]
struct Var {
    expr: String,
    mean: f64,
    max: f64,
    kind: VarKind,
}

#[derive(Clone, Debug)]
struct Coll {
    param: String,
    path: String,
    count: Var,
    inner: Var,
}

#[derive(Clone, Debug, Default)]
struct Scope {
    vars: Vec<Var>,
    colls: Vec<Coll>,
    iterating: Vec<String>,
    in_fork: bool,
    holding: bool,
    depth: u32,
}

#[derive(Clone, Debug)]
struct Callee {
    role: String,
    iface: usize,
    unit: UnitRef,
    link: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
enum UnitRef {
    Unit(usize),
    Back(usize),
}

#[derive(Clone, Debug)]
struct InfraCtx {
    role: String,
    sig: String,
    param: Option<String>,
    cost: Cost,
}

#[derive(Clone, Debug)]
struct CompCtx {
    container: usize,
    avail: Vec<ResType>,
    callees: Vec<Callee>,
    infra: Option<InfraCtx>,
    rreq: Option<String>,
    rreq_hdd: Option<String>,
    passive: Option<String>,
    comp_vars: Vec<Var>,
}

#[derive(Clone, Debug)]
struct Unit {
    /// Component assembled at system level (basic or composite).
    comp: usize,
    /// Basic component carrying the SEFFs of the unit's interface (the composite's front).
    front: usize,
    back: Option<usize>,
    back_iface: Option<usize>,
    /// Composite wrapping the back component (`nested_composite`).
    back_wrap: Option<usize>,
    iface: usize,
    requires: Vec<usize>,
    infra: bool,
    asms: Vec<usize>,
    container: usize,
    /// Expected cost per signature of `iface`.
    sig_costs: Vec<Cost>,
    back_cost: Cost,
    /// Container of the callee assembly per required unit (first assembly's connections).
    callee_container: Vec<usize>,
}

pub(super) struct Builder<'a> {
    cfg: &'a GenConfig,
    f: &'a Features,
    rng: Rng,
    counters: BTreeMap<&'static str, u32>,
    m: GenModel,
    units: Vec<Unit>,
    feats: std::collections::BTreeSet<&'static str>,
    middleware: Option<(usize, usize)>, // (component, assembly)
    // per-model switches of the newer features (decided once, at the start)
    ties: bool,
    deep: bool,
    exotic: bool,
    dresume: bool,
    heavy: bool,
    long: bool,
    poverride: bool,
}

/// Rounds to 3 significant digits and prints in Java format.
fn nice(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return "0.0".into();
    }
    let e = x.abs().log10().floor() as i32;
    let p = 10f64.powi(2 - e);
    let r = (x * p).round() / p;
    let r: f64 = format!("{r:.12e}").parse().unwrap_or(r);
    javafmt::to_string(r)
}

impl<'a> Builder<'a> {
    pub(super) fn new(cfg: &'a GenConfig) -> Self {
        Builder {
            cfg,
            f: &cfg.features,
            rng: Rng::new(cfg.seed),
            counters: BTreeMap::new(),
            m: GenModel {
                name: cfg.name.clone(),
                containers: vec![],
                links: vec![],
                interfaces: vec![],
                coll_type: None,
                components: vec![],
                assemblies: vec![],
                connectors: vec![],
                sys_roles: vec![],
                allocation: vec![],
                scenarios: vec![],
                run: RunConfig::default(),
                windows: vec![],
                asm_op_monitors: vec![],
                triggers: Triggers::Off,
                aggregations: vec![],
                reconf_monitor: None,
                container_count_monitor: false,
                nested: None,
                features: vec![],
            },
            units: vec![],
            feats: Default::default(),
            middleware: None,
            ties: false,
            deep: false,
            exotic: false,
            dresume: false,
            heavy: false,
            long: false,
            poverride: false,
        }
    }

    fn id(&mut self, kind: &'static str) -> String {
        let n = self.counters.entry(kind).or_insert(0);
        *n += 1;
        format!("_{}_{}{}", self.cfg.name, kind, n)
    }

    fn feat(&mut self, name: &'static str) {
        self.feats.insert(name);
    }

    fn size(&self) -> usize {
        self.cfg.size as usize
    }

    /// Decides a per-model switch (no draw when the feature is off, so models generated without
    /// the newer features are identical to those of the original generator).
    fn switch(&mut self, p: f64, name: &'static str) -> bool {
        let on = self.rng.chance(p);
        if on {
            self.feat(name);
        }
        on
    }

    pub(super) fn build(mut self) -> GenModel {
        let f = self.f;
        self.ties = self.switch(f.ties, "ties");
        self.deep = self.switch(f.deep_nesting, "deep_nesting");
        self.exotic = self.switch(f.stoex_exotic, "stoex_exotic");
        self.dresume = self.switch(f.double_resume, "double_resume_gen");
        self.heavy = self.switch(f.heavy_load, "heavy_load");
        self.long = self.switch(f.long_run, "long_run");
        self.poverride = self.switch(f.param_override, "param_override");
        self.containers();
        self.units();
        self.allocate();
        self.connect();
        self.seffs();
        self.usage();
        self.m.features = self.feats.iter().copied().collect();
        self.m
    }

    // ------------------------------------------------------------------ resource environment

    fn containers(&mut self) {
        let n = if self.rng.chance(self.f.multi_container) {
            self.rng.range(2, 2 + (self.size() / 4).min(1))
        } else {
            1
        };
        if n > 1 {
            self.feat("multi_container");
        }
        for i in 0..n {
            let mut res = Vec::new();
            let (sched, s) = if self.rng.chance(self.f.fcfs_cpu) {
                (Sched::Fcfs, "fcfs_cpu")
            } else {
                (Sched::Ps, "ps_cpu")
            };
            self.feat(s);
            let replicas = if self.rng.chance(self.f.multicore) {
                self.feat("multicore");
                self.rng.range(2, 4) as u32
            } else {
                1
            };
            let (rate, rate_mean) = self.rate();
            res.push(ProcRes {
                id: self.id("prs"),
                ty: ResType::Cpu,
                sched,
                rate,
                rate_mean,
                replicas,
                hdd_rates: None,
            });
            if self.rng.chance(self.f.hdd) {
                self.feat("hdd");
                let sched = if self.rng.chance(0.6) {
                    Sched::Fcfs
                } else {
                    Sched::Ps
                };
                let replicas = if self.rng.chance(self.f.multicore * 0.5) {
                    2
                } else {
                    1
                };
                let (rate, mut rate_mean) = self.rate();
                let hdd_rates = if self.rng.chance(self.f.hdd_rw) {
                    self.feat("hdd_rw");
                    let (r, w) = *self
                        .rng
                        .pick(&[(2.0, 1.0), (1.0, 0.5), (4.0, 4.0), (1.0, 2.0)]);
                    rate_mean *= r;
                    Some((javafmt::to_string(r), javafmt::to_string(w)))
                } else {
                    None
                };
                res.push(ProcRes {
                    id: self.id("prs"),
                    ty: ResType::Hdd,
                    sched,
                    rate,
                    rate_mean,
                    replicas,
                    hdd_rates,
                });
            }
            if self.rng.chance(self.f.delay_resource) {
                self.feat("delay_resource");
                res.push(ProcRes {
                    id: self.id("prs"),
                    ty: ResType::Delay,
                    sched: Sched::Delay,
                    rate: "1.0".into(),
                    rate_mean: 1.0,
                    replicas: if self.rng.chance(self.f.multicore * 0.5) {
                        2
                    } else {
                        1
                    },
                    hdd_rates: None,
                });
            }
            let id_rc = self.id("rc");
            self.m.containers.push(Container {
                id: id_rc,
                name: format!("Node{i}"),
                res,
            });
        }
        if n > 1 {
            self.feat("linking_resource");
            if n >= 3 && self.rng.chance(0.4) {
                // a partial link first: routing picks the first link connecting both ends
                self.link(vec![0, 1]);
            }
            self.link((0..n).collect());
            if self.rng.chance(self.f.no_link_throughput) {
                self.feat("no_link_throughput");
                self.m.run.simulate_throughput_of_linking_resources = false;
            }
        }
    }

    fn rate(&mut self) -> (String, f64) {
        if self.ties {
            let r = *self.rng.pick(&[1.0, 2.0]);
            return (javafmt::to_string(r), r);
        }
        if self.rng.chance(self.f.stoex_rate) {
            self.feat("stoex_rate");
            return ("UniDouble(0.9, 1.1)".into(), 1.0);
        }
        let r = *self.rng.pick(&[1.0, 1.0, 2.0, 0.5, 1000.0]);
        let s = javafmt::to_string(r);
        if r == 1000.0 {
            // demands are then read as "work units" of 1 ms
            return (s, r);
        }
        (s, r)
    }

    fn link(&mut self, containers: Vec<usize>) {
        let (latency, latency_mean) = if self.rng.chance(self.f.stoex_rate) {
            ("Exp(2000.0)".to_string(), 0.0005)
        } else {
            let l = *self.rng.pick(&[0.0, 0.001, 0.0005]);
            (javafmt::to_string(l), l)
        };
        let tp = *self.rng.pick(&[1.0e6, 1.0e7, 1.0e9]);
        let n = self.m.links.len();
        let id_link = self.id("link");
        let id_clrs = self.id("clrs");
        self.m.links.push(Link {
            id: id_link,
            spec_id: id_clrs,
            name: format!("LAN{n}"),
            latency,
            latency_mean,
            throughput: javafmt::to_string(tp),
            throughput_mean: tp,
            containers,
        });
    }

    fn link_between(&self, a: usize, b: usize) -> Option<usize> {
        if a == b {
            return None;
        }
        self.m
            .links
            .iter()
            .position(|l| l.containers.contains(&a) && l.containers.contains(&b))
    }

    // ------------------------------------------------------------------ repository structure

    fn new_iface(&mut self, name: String, allow_params: bool) -> usize {
        let nsig = if self.rng.chance(0.25 + 0.03 * self.size() as f64) {
            2
        } else {
            1
        };
        let idx = self.m.interfaces.len();
        let mut sigs = Vec::new();
        for s in 0..nsig {
            let mut params = Vec::new();
            if allow_params {
                let np = self.rng.weighted(&[0.3, 0.45, 0.25]).unwrap();
                for k in 0..np {
                    let w = [1.0, self.f.bytesize * 0.8, self.f.collection * 0.8];
                    let kind = match self.rng.weighted(&w).unwrap_or(0) {
                        0 => ParamKind::Int,
                        1 => ParamKind::Bytes,
                        _ => ParamKind::Coll,
                    };
                    let pname = match kind {
                        ParamKind::Int => format!("n{k}"),
                        ParamKind::Bytes => format!("data{k}"),
                        ParamKind::Coll => format!("items{k}"),
                    };
                    if kind == ParamKind::Coll && self.m.coll_type.is_none() {
                        self.m.coll_type = Some(self.id("dt"));
                    }
                    params.push(Param { name: pname, kind });
                }
            }
            let returns = allow_params && self.rng.chance(self.f.return_value);
            sigs.push(Signature {
                id: self.id("sig"),
                name: format!("op{idx}_{s}"),
                params,
                returns,
            });
        }
        let id_if = self.id("if");
        self.m.interfaces.push(Interface {
            id: id_if,
            name,
            infra: false,
            sigs,
        });
        idx
    }

    fn new_basic(&mut self, name: String) -> usize {
        let id = self.id("comp");
        let mut c = BasicComp {
            id,
            name,
            provides: vec![],
            requires: vec![],
            rreq: None,
            rreq_hdd: None,
            passive: vec![],
            comp_params: vec![],
            seffs: vec![],
        };
        if self.rng.chance(self.f.passive) {
            self.feat("passive_resource");
            let cap = if self.rng.chance(0.2) {
                "IntPMF[(1;0.5)(2;0.5)]".to_string()
            } else {
                self.rng.range(1, 3).to_string()
            };
            c.passive.push(Passive {
                id: self.id("pr"),
                name: "pool".into(),
                capacity: cap,
            });
        }
        if self.rng.chance(self.f.component_params) {
            self.feat("component_params");
            let v = self.rng.range(1, 5);
            let spec = if self.poverride && self.rng.chance(0.4) {
                self.feat("param_distribution");
                (*self.rng.pick(&["IntPMF[(1;0.5)(3;0.5)]", "UniInt(1, 5)"])).to_string()
            } else {
                v.to_string()
            };
            c.comp_params.push(VarUsage {
                name: "cp0".into(),
                chars: vec![(Char::Value, spec)],
            });
            if self.poverride && self.rng.chance(0.4) {
                self.feat("param_bytesize");
                let b = (*self.rng.pick(&["IntPMF[(1000;0.5)(3000;0.5)]", "2000"])).to_string();
                c.comp_params.push(VarUsage {
                    name: "cp1".into(),
                    chars: vec![(Char::Bytesize, b)],
                });
            }
        }
        if self.rng.chance(self.f.resource_call) {
            c.rreq = Some(self.id("rreq"));
        }
        if self.rng.chance(self.f.hdd_rw * self.f.resource_call * 2.0) {
            c.rreq_hdd = Some(self.id("rreq"));
        }
        self.m.components.push(Component::Basic(c));
        self.m.components.len() - 1
    }

    fn basic(&mut self, i: usize) -> &mut BasicComp {
        match &mut self.m.components[i] {
            Component::Basic(b) => b,
            _ => panic!("not basic"),
        }
    }

    fn prov_role(&mut self, comp: usize, iface: usize) -> String {
        let id = self.id("prov");
        let name = format!("Provided_{}", self.m.interfaces[iface].name);
        let r = Role {
            id: id.clone(),
            name,
            iface,
        };
        match &mut self.m.components[comp] {
            Component::Basic(b) => b.provides.push(r),
            Component::Composite(c) => c.provides.push(r),
        }
        id
    }

    fn req_role(&mut self, comp: usize, iface: usize) -> String {
        let id = self.id("req");
        let name = format!("Required_{}", self.m.interfaces[iface].name);
        let r = Role {
            id: id.clone(),
            name,
            iface,
        };
        match &mut self.m.components[comp] {
            Component::Basic(b) => b.requires.push(r),
            Component::Composite(c) => c.requires.push(r),
        }
        id
    }

    fn units(&mut self) {
        let max_units = 1 + (self.size() * 2 / 3).min(5);
        let n = if self.f.external_call > 0.0 {
            self.rng.range(1, max_units)
        } else {
            1
        };
        // call DAG: unit u requires units > u
        let mut req: Vec<Vec<usize>> = vec![vec![]; n];
        for (u, r) in req.iter_mut().enumerate() {
            for v in u + 1..n {
                let p = if v == u + 1 { 0.75 } else { 0.25 };
                if r.len() < 2 && self.rng.chance(p * self.f.external_call) {
                    r.push(v);
                }
            }
        }
        for v in 1..n {
            if !req.iter().any(|r| r.contains(&v)) {
                let u = self.rng.below(v);
                req[u].push(v);
            }
        }
        if n > 1 {
            self.feat("external_call");
        }
        for (u, requires) in req.into_iter().enumerate() {
            let iface = self.new_iface(format!("IService{u}"), true);
            let composite = self.rng.chance(self.f.composite);
            let mut back_wrap = None;
            let (comp, front, back, back_iface) = if composite {
                self.feat("composite");
                let front = self.new_basic(format!("Front{u}"));
                let (back, back_iface) = if self.rng.chance(0.5) {
                    let bi = self.new_iface(format!("IBack{u}"), true);
                    let b = self.new_basic(format!("Back{u}"));
                    if self.rng.chance(self.f.nested_composite) {
                        self.feat("nested_composite");
                        let wid = self.id("comp");
                        self.m.components.push(Component::Composite(CompositeComp {
                            id: wid,
                            name: format!("NestedBack{u}"),
                            provides: vec![],
                            requires: vec![],
                            inner: vec![],
                            conns: vec![],
                        }));
                        back_wrap = Some(self.m.components.len() - 1);
                    }
                    (Some(b), Some(bi))
                } else {
                    (None, None)
                };
                let cid = self.id("comp");
                self.m.components.push(Component::Composite(CompositeComp {
                    id: cid,
                    name: format!("Composite{u}"),
                    provides: vec![],
                    requires: vec![],
                    inner: vec![],
                    conns: vec![],
                }));
                (self.m.components.len() - 1, front, back, back_iface)
            } else {
                let c = self.new_basic(format!("Comp{u}"));
                (c, c, None, None)
            };
            self.units.push(Unit {
                comp,
                front,
                back,
                back_iface,
                back_wrap,
                iface,
                requires,
                infra: false,
                asms: vec![],
                container: 0,
                sig_costs: vec![],
                back_cost: Cost::new(),
                callee_container: vec![],
            });
        }
        // roles (after all interfaces exist)
        for u in 0..n {
            let (comp, front, back, back_iface, iface, back_wrap) = {
                let x = &self.units[u];
                (x.comp, x.front, x.back, x.back_iface, x.iface, x.back_wrap)
            };
            let req_ifaces: Vec<usize> = self.units[u]
                .requires
                .iter()
                .map(|&v| self.units[v].iface)
                .collect();
            if comp == front {
                self.prov_role(comp, iface);
                for i in req_ifaces {
                    self.req_role(comp, i);
                }
            } else {
                // composite: outer roles, inner front/back, delegation connectors
                let cprov = self.prov_role(comp, iface);
                let fprov = self.prov_role(front, iface);
                let config = self.override_config(front, true);
                let af = Assembly {
                    id: self.id("ac"),
                    name: format!("Inner_Front{u}"),
                    comp: front,
                    config,
                };
                let mut conns = vec![Conn::ProvDeleg {
                    id: self.id("conn"),
                    outer_role: cprov,
                    ac: af.id.clone(),
                    inner_role: fprov,
                }];
                let mut inner = vec![af.clone()];
                for i in req_ifaces {
                    let outer = self.req_role(comp, i);
                    let inner_role = self.req_role(front, i);
                    conns.push(Conn::ReqDeleg {
                        id: self.id("conn"),
                        inner_role,
                        ac: af.id.clone(),
                        outer_role: outer,
                    });
                }
                if let (Some(b), Some(bi)) = (back, back_iface) {
                    let mut bprov = self.prov_role(b, bi);
                    let mut bcomp = b;
                    if let Some(w) = back_wrap {
                        // NestedBack{u} provides the interface by delegation to its inner Back{u}
                        let wprov = self.prov_role(w, bi);
                        let config = self.override_config(b, true);
                        let ai = Assembly {
                            id: self.id("ac"),
                            name: format!("Inner_NestedBack{u}"),
                            comp: b,
                            config,
                        };
                        let conn = Conn::ProvDeleg {
                            id: self.id("conn"),
                            outer_role: wprov.clone(),
                            ac: ai.id.clone(),
                            inner_role: bprov,
                        };
                        if let Component::Composite(c) = &mut self.m.components[w] {
                            c.inner = vec![ai];
                            c.conns = vec![conn];
                        }
                        bprov = wprov;
                        bcomp = w;
                    }
                    let freq = self.req_role(front, bi);
                    let ab = Assembly {
                        id: self.id("ac"),
                        name: format!("Inner_Back{u}"),
                        comp: bcomp,
                        config: vec![],
                    };
                    conns.push(Conn::Assembly {
                        id: self.id("conn"),
                        req_ac: af.id.clone(),
                        req_role: freq,
                        prov_ac: ab.id.clone(),
                        prov_role: bprov,
                    });
                    inner.push(ab);
                }
                if let Component::Composite(c) = &mut self.m.components[comp] {
                    c.inner = inner;
                    c.conns = conns;
                }
            }
        }
        // infrastructure middleware
        if self.rng.chance(self.f.infra) {
            let idx = self.m.interfaces.len();
            let param = self.rng.chance(0.5).then(|| "m0".to_string());
            let sig = Signature {
                id: self.id("sig"),
                name: "marshal".into(),
                params: param
                    .iter()
                    .map(|p| Param {
                        name: p.clone(),
                        kind: ParamKind::Int,
                    })
                    .collect(),
                returns: false,
            };
            let id_if = self.id("if");
            self.m.interfaces.push(Interface {
                id: id_if,
                name: "IMiddleware".into(),
                infra: true,
                sigs: vec![sig],
            });
            let mw = self.new_basic("Middleware".into());
            // no passive resources / resource calls on the middleware (keeps it simple)
            self.basic(mw).passive.clear();
            self.prov_role(mw, idx);
            let mut any = false;
            for u in 0..n {
                if self.units[u].comp == self.units[u].front && self.rng.chance(0.6) {
                    self.units[u].infra = true;
                    any = true;
                }
            }
            if !any && self.units[0].comp == self.units[0].front {
                self.units[0].infra = true;
                any = true;
            }
            if any {
                self.feat("infrastructure_call");
                for u in 0..n {
                    if self.units[u].infra {
                        let c = self.units[u].comp;
                        let id = self.id("req");
                        self.basic(c).requires.push(Role {
                            id,
                            name: "Required_IMiddleware".into(),
                            iface: idx,
                        });
                    }
                }
                self.middleware = Some((mw, usize::MAX));
            } else {
                self.m.components.pop();
                self.m.interfaces.pop();
            }
        }
    }

    // ------------------------------------------------------------------ system + allocation

    fn allocate(&mut self) {
        let nc = self.m.containers.len();
        for u in 0..self.units.len() {
            let comp = self.units[u].comp;
            let copies = if self.units[u].comp == self.units[u].front
                && self.rng.chance(self.f.double_assembly)
            {
                self.feat("double_assembly");
                2
            } else {
                1
            };
            for k in 0..copies {
                let mut config = vec![];
                if k == 1
                    && let Component::Basic(b) = &self.m.components[comp]
                    && !b.comp_params.is_empty()
                {
                    let v = self.rng.range(6, 9);
                    config.push(VarUsage {
                        name: b.comp_params[0].name.clone(),
                        chars: vec![(Char::Value, v.to_string())],
                    });
                    self.feat("assembly_config_params");
                }
                if k == 0 {
                    config = self.override_config(comp, false);
                }
                let a = Assembly {
                    id: self.id("ac"),
                    name: format!("Assembly_{}_{k}", self.m.components[comp].name()),
                    comp,
                    config,
                };
                self.m.assemblies.push(a);
                let ai = self.m.assemblies.len() - 1;
                self.units[u].asms.push(ai);
                let c = self.rng.below(nc);
                let alc = self.id("alc");
                self.m.allocation.push((alc, ai, c));
                if k == 0 {
                    self.units[u].container = c;
                }
            }
        }
        if let Some((mw, _)) = self.middleware {
            let id_ac = self.id("ac");
            self.m.assemblies.push(Assembly {
                id: id_ac,
                name: "Assembly_Middleware".into(),
                comp: mw,
                config: vec![],
            });
            let ai = self.m.assemblies.len() - 1;
            self.middleware = Some((mw, ai));
            let c = self.rng.below(nc);
            let alc = self.id("alc");
            self.m.allocation.push((alc, ai, c));
        }
    }

    /// Assembly-context configuration overriding the component parameters (`param_override`):
    /// VALUE with a constant or a distribution, and BYTESIZE.
    fn override_config(&mut self, comp: usize, inner: bool) -> Vec<VarUsage> {
        let params = match &self.m.components[comp] {
            Component::Basic(b) => b.comp_params.clone(),
            _ => return vec![],
        };
        if !self.poverride || params.is_empty() || !self.rng.chance(0.5) {
            return vec![];
        }
        self.feat(if inner {
            "inner_assembly_config_params"
        } else {
            "assembly_config_params"
        });
        let mut v = Vec::new();
        for p in params {
            let (ch, _) = p.chars[0];
            let spec = match ch {
                Char::Bytesize => "IntPMF[(500;0.5)(4000;0.5)]".to_string(),
                _ => match self.rng.below(3) {
                    0 => "UniInt(6, 9)".to_string(),
                    _ => self.rng.range(6, 9).to_string(),
                },
            };
            if self.rng.chance(0.8) {
                v.push(VarUsage {
                    name: p.name.clone(),
                    chars: vec![(ch, spec)],
                });
            }
        }
        v
    }

    fn container_of(&self, asm: usize) -> usize {
        self.m
            .allocation
            .iter()
            .find(|(_, a, _)| *a == asm)
            .map(|x| x.2)
            .unwrap()
    }

    fn roles(&self, comp: usize) -> (Vec<Role>, Vec<Role>) {
        match &self.m.components[comp] {
            Component::Basic(b) => (b.provides.clone(), b.requires.clone()),
            Component::Composite(c) => (c.provides.clone(), c.requires.clone()),
        }
    }

    fn connect(&mut self) {
        for u in 0..self.units.len() {
            let comp = self.units[u].comp;
            let (_, reqs) = self.roles(comp);
            let asms = self.units[u].asms.clone();
            for (k, &a) in asms.iter().enumerate() {
                for r in &reqs {
                    let iface = &self.m.interfaces[r.iface];
                    if iface.infra {
                        let (mw, mwa) = self.middleware.unwrap();
                        let (mprov, _) = self.roles(mw);
                        let c = Conn::Infra {
                            id: self.id("conn"),
                            req_ac: self.m.assemblies[a].id.clone(),
                            req_role: r.id.clone(),
                            prov_ac: self.m.assemblies[mwa].id.clone(),
                            prov_role: mprov[0].id.clone(),
                        };
                        self.m.connectors.push(c);
                        continue;
                    }
                    let v = self.units.iter().position(|x| x.iface == r.iface).unwrap();
                    let target = *self.rng.pick(&self.units[v].asms.clone());
                    if k == 0 {
                        let tc = self.container_of(target);
                        self.units[u].callee_container.push(tc);
                    }
                    let (vprov, _) = self.roles(self.units[v].comp);
                    let c = Conn::Assembly {
                        id: self.id("conn"),
                        req_ac: self.m.assemblies[a].id.clone(),
                        req_role: r.id.clone(),
                        prov_ac: self.m.assemblies[target].id.clone(),
                        prov_role: vprov[0].id.clone(),
                    };
                    self.m.connectors.push(c);
                }
            }
        }
        // system provided roles: unit 0, sometimes another entry
        let mut entries = vec![0];
        if self.units.len() > 1 && self.rng.chance(0.3) {
            entries.push(self.rng.range(1, self.units.len() - 1));
            self.feat("two_system_roles");
        }
        for u in entries {
            let iface = self.units[u].iface;
            let id = self.id("prov");
            let name = format!("Provided_{}", self.m.interfaces[iface].name);
            self.m.sys_roles.push(SysRole {
                id: id.clone(),
                name,
                iface,
            });
            let (prov, _) = self.roles(self.units[u].comp);
            let a = self.units[u].asms[0];
            let c = Conn::ProvDeleg {
                id: self.id("conn"),
                outer_role: id,
                ac: self.m.assemblies[a].id.clone(),
                inner_role: prov[0].id.clone(),
            };
            self.m.connectors.push(c);
        }
    }

    // ------------------------------------------------------------------ expressions

    fn int_vars<'s>(&self, s: &'s Scope, max: f64) -> Vec<&'s Var> {
        s.vars
            .iter()
            .filter(|v| v.kind == VarKind::Int && v.max <= max && self.f.parametric > 0.0)
            .collect()
    }

    /// Demand-like expression (seconds of work at rate 1).
    fn demand_expr(&mut self, s: &Scope) -> (String, f64) {
        if self.ties && self.rng.chance(0.6) {
            let c = *self.rng.pick(&[0.0, 0.005, 0.01, 0.01, 0.02]);
            return (javafmt::to_string(c), c);
        }
        let dist = self.f.distributions;
        let par = self.f.parametric;
        let ints = self.int_vars(s, 1e9).len();
        let bytes = s.vars.iter().any(|v| v.kind == VarKind::Bytes);
        let w = [
            2.0,                                    // const
            2.0 * dist,                             // Exp
            dist,                                   // UniDouble
            dist,                                   // Gamma family
            dist,                                   // Lognorm family
            0.7 * dist,                             // Norm via Max
            dist,                                   // PMF/PDF
            0.6 * dist,                             // integer distributions scaled
            if ints > 0 { 2.0 * par } else { 0.0 }, // parametric int
            if bytes { 1.5 * par } else { 0.0 },    // parametric bytes
            0.5 * dist,                             // arithmetic mixes
            if self.exotic { 1.5 } else { 0.0 },    // unusual StoEx
        ];
        match self.rng.weighted(&w).unwrap_or(0) {
            0 => {
                let c = *self.rng.pick(&[0.001, 0.002, 0.005, 0.01, 0.02, 0.03, 0.0]);
                (javafmt::to_string(c), c)
            }
            1 => {
                self.feat("stoex_distributions");
                let r = *self.rng.pick(&[20.0, 50.0, 100.0, 200.0]);
                (format!("Exp({})", javafmt::to_string(r)), 1.0 / r)
            }
            2 => {
                self.feat("stoex_distributions");
                let a = *self.rng.pick(&[0.001, 0.005]);
                let b = a + *self.rng.pick(&[0.01, 0.02]);
                let b: f64 = format!("{b:.4}").parse().unwrap();
                (
                    format!(
                        "UniDouble({}, {})",
                        javafmt::to_string(a),
                        javafmt::to_string(b)
                    ),
                    (a + b) / 2.0,
                )
            }
            3 => {
                self.feat("stoex_distributions");
                match self.rng.below(2) {
                    0 => ("Gamma(2.0, 0.005)".into(), 0.01),
                    _ => ("GammaMoments(0.01, 0.5)".into(), 0.01),
                }
            }
            4 => {
                self.feat("stoex_distributions");
                match self.rng.below(2) {
                    0 => ("Lognorm(-4.5, 0.4)".into(), 0.012),
                    _ => ("LognormMoments(0.01, 0.005)".into(), 0.01),
                }
            }
            5 => {
                self.feat("stoex_distributions");
                if self.rng.chance(0.5) {
                    ("Max(Norm(0.01, 0.003), 0.0)".into(), 0.01)
                } else {
                    // may be negative: the demand is then skipped
                    ("Norm(0.005, 0.004)".into(), 0.005)
                }
            }
            6 => {
                self.feat("stoex_pmf_pdf");
                match self.rng.below(3) {
                    0 => ("DoublePDF[(0.005;0.5)(0.02;0.5)]".into(), 0.01),
                    1 => ("DoublePMF[(0.005;0.4)(0.02;0.6)]".into(), 0.014),
                    _ => ("IntPMF[(1;0.25)(2;0.75)] * 0.005".into(), 0.00875),
                }
            }
            7 => {
                self.feat("stoex_distributions");
                match self.rng.below(3) {
                    0 => ("UniInt(1, 3) * 0.005".into(), 0.01),
                    1 => ("Pois(2.0) * 0.004".into(), 0.008),
                    _ => ("Round(Exp(0.5)) * 0.004".into(), 0.008),
                }
            }
            8 => {
                self.feat("parametric_dependency");
                let vars = self.int_vars(s, 1e9);
                let v = (*self.rng.pick(&vars)).clone();
                let c = *self.rng.pick(&[0.001, 0.002, 0.005]);
                let plus = self.rng.chance(0.4);
                let expr = if plus {
                    format!("{} * {} + 0.001", v.expr, javafmt::to_string(c))
                } else {
                    format!("{} * {}", v.expr, javafmt::to_string(c))
                };
                (expr, v.mean * c + if plus { 0.001 } else { 0.0 })
            }
            9 => {
                self.feat("parametric_dependency");
                let vars: Vec<Var> = s
                    .vars
                    .iter()
                    .filter(|v| v.kind == VarKind::Bytes)
                    .cloned()
                    .collect();
                let v = self.rng.pick(&vars).clone();
                (format!("{} * 0.000002", v.expr), v.mean * 0.000002)
            }
            10 => {
                self.feat("stoex_arithmetic");
                match self.rng.below(5) {
                    0 => ("Min(0.02, Exp(80.0)) + 0.001".into(), 0.0125),
                    1 => ("Trunc(3.7) * 0.002 + Round(2.5) * 0.001".into(), 0.009),
                    2 => ("(2 + 3) * 0.002 - 0.01 / 2 ^ 2".into(), 0.0075),
                    3 => ("Ceil(0.2) * 0.005 + Sqrt(0.0001)".into(), 0.015),
                    _ => ("Log(10, 1.02) * 0.5".into(), 0.0043),
                }
            }
            _ => {
                self.feat("stoex_exotic_demand");
                let ints = self.int_vars(s, 1e9);
                if !ints.is_empty() && self.rng.chance(0.15) {
                    let v = self.rng.pick(&ints).expr.clone();
                    return (format!("({v} > 2 ? 0.01 : 0.002)"), 0.006);
                }
                const EXOTIC: &[(&str, f64)] = &[
                    // unsorted, sum 1 + 1e-7: normalised (delta added to every p > 0), then sorted
                    ("IntPMF[(3;0.2)(1;0.3)(2;0.5000001)] * 0.004", 0.0076),
                    // Pois is shifted by -1 in 5.2.2: -1 with probability exp(-m)
                    ("(Pois(2.5) + 1) * 0.003", 0.0075),
                    ("Pois(0.3) * 0.01", 0.0),
                    ("DoublePDF[(0.002;0.0)(0.01;0.5)(0.03;0.5)]", 0.013),
                    ("DoublePMF[(0.02;0.25)(0.005;0.75)]", 0.00875),
                    ("DoublePMF[(0.01;0.3)(0.02;0.3)(0.03;0.39999999)]", 0.021),
                    // Java int arithmetic: truncating division, remainder, wrap-around
                    ("(7 / 2) * 0.002 + (7 % 3) * 0.001", 0.007),
                    ("Max(2147483647 + 2, 3) * 0.002", 0.006),
                    ("2 ^ 3 * 0.001", 0.008),
                    ("(true ? 0.004 : 1.0)", 0.004),
                    // sub-nanosecond service times: 0 ns spans
                    ("Exp(1.0E9)", 0.0),
                    ("1.0E-10", 0.0),
                    ("-(-0.005)", 0.005),
                    ("0.03 % 0.02", 0.01),
                    ("IntPMF[(0;0.5)(2;0.5)] * 0.01", 0.01),
                    // often negative: the demand is skipped
                    ("Norm(0.005, 0.02)", 0.005),
                    ("Lognorm(-9.0, 2.0)", 0.0009),
                    ("GammaMoments(0.005, 1.5)", 0.005),
                ];
                let (e, m) = *self.rng.pick(EXOTIC);
                (e.to_string(), m)
            }
        }
    }

    /// Small non-negative integer expression, bounded by `max` (for VALUE characterisations).
    fn int_expr(&mut self, s: &Scope, max: f64) -> (String, f64, f64) {
        let vars: Vec<Var> = self.int_vars(s, max / 2.0).into_iter().cloned().collect();
        let dist = self.f.distributions;
        let w = [
            1.0,
            2.0 * dist,
            dist,
            if vars.is_empty() { 0.0 } else { 1.5 },
            if self.exotic { 0.6 } else { 0.0 },
        ];
        match self.rng.weighted(&w).unwrap_or(0) {
            0 => {
                let c = self.rng.range(0, 5);
                (c.to_string(), c as f64, c as f64)
            }
            4 => {
                self.feat("stoex_exotic_int");
                match self.rng.below(4) {
                    0 => ("IntPMF[(-1;0.5)(3;0.5)]".into(), 1.0, 3.0),
                    1 => ("Pois(2.0)".into(), 1.0, 6.0),
                    2 => ("Round(2.5)".into(), 3.0, 3.0),
                    _ => ("IntPMF[(4;0.25)(0;0.25)(2;0.5)]".into(), 2.0, 4.0),
                }
            }
            1 => match self.rng.below(3) {
                0 => ("IntPMF[(1;0.3)(2;0.4)(4;0.3)]".into(), 2.3, 4.0),
                1 => ("IntPMF[(0;0.2)(3;0.5)(5;0.3)]".into(), 3.0, 5.0),
                _ => ("IntPMF[(2;0.5)(6;0.5)]".into(), 4.0, 6.0),
            },
            2 => ("UniInt(1, 4)".into(), 2.5, 4.0),
            _ => {
                self.feat("parametric_dependency");
                let v = self.rng.pick(&vars).clone();
                if self.rng.chance(0.5) {
                    (format!("{} + 1", v.expr), v.mean + 1.0, v.max + 1.0)
                } else {
                    (format!("{} * 2", v.expr), v.mean * 2.0, v.max * 2.0)
                }
            }
        }
    }

    /// Iteration counts (0..=4).
    fn count_expr(&mut self, s: &Scope) -> (String, f64) {
        let vars: Vec<Var> = self.int_vars(s, 4.0).into_iter().cloned().collect();
        let dist = self.f.distributions;
        let w = [
            1.0,
            2.0 * dist,
            dist,
            if vars.is_empty() { 0.0 } else { 1.0 },
            if self.exotic { 0.8 } else { 0.0 },
        ];
        match self.rng.weighted(&w).unwrap_or(0) {
            0 => {
                let c = self.rng.range(1, 3);
                (c.to_string(), c as f64)
            }
            4 => {
                self.feat("stoex_exotic_count");
                const COUNTS: &[(&str, f64)] = &[
                    ("Pois(1.5)", 0.8),
                    ("Max(2147483647 + 2, 2)", 2.0),
                    ("IntPMF[(2;0.3)(0;0.7000001)]", 0.6),
                    ("7 % 4", 3.0),
                    ("UniInt(0, 0)", 0.0),
                    ("-1", 0.0),
                    ("Trunc(2.9)", 2.0),
                    ("Ceil(UniDouble(0.5, 2.5))", 2.0),
                ];
                let (e, m) = *self.rng.pick(COUNTS);
                (e.to_string(), m)
            }
            1 => match self.rng.below(2) {
                0 => ("IntPMF[(1;0.5)(2;0.3)(3;0.2)]".into(), 1.7),
                _ => ("IntPMF[(0;0.2)(1;0.5)(2;0.3)]".into(), 1.1),
            },
            2 => ("UniInt(1, 3)".into(), 2.0),
            _ => {
                self.feat("parametric_dependency");
                let v = self.rng.pick(&vars).clone();
                (v.expr, v.mean)
            }
        }
    }

    fn bytes_expr(&mut self, s: &Scope) -> (String, f64) {
        let vars: Vec<Var> = s
            .vars
            .iter()
            .filter(|v| v.kind == VarKind::Bytes && v.max <= 1e5)
            .cloned()
            .collect();
        let w = [
            1.0,
            2.0 * self.f.distributions,
            self.f.distributions,
            if vars.is_empty() {
                0.0
            } else {
                self.f.parametric
            },
        ];
        match self.rng.weighted(&w).unwrap_or(0) {
            0 => ("1000".into(), 1000.0),
            1 => ("IntPMF[(1000;0.5)(5000;0.5)]".into(), 3000.0),
            2 => ("UniInt(100, 2000)".into(), 1050.0),
            _ => {
                self.feat("parametric_dependency");
                let v = self.rng.pick(&vars).clone();
                (format!("{} * 2", v.expr), v.mean * 2.0)
            }
        }
    }

    /// Branch probabilities that do not sum to exactly 1, or with a zero-probability transition
    /// (`stoex_exotic`); `None` = the regular tenths.
    fn exotic_probs(&mut self, n: usize) -> Option<Vec<String>> {
        if !self.exotic || !self.rng.chance(0.3) {
            return None;
        }
        self.feat("exotic_branch_probabilities");
        let v: &[&str] = match (n, self.rng.below(3)) {
            (2, 0) => &["0.0", "1.0"],
            (2, 1) => &["0.5", "0.5000001"],
            (2, _) => &["0.3", "0.6999999"],
            (_, 0) => &["0.0", "0.5", "0.5"],
            (_, 1) => &["0.2", "0.3", "0.5000001"],
            (_, _) => &["0.1", "0.2", "0.6999999"],
        };
        Some(v.iter().map(|x| x.to_string()).collect())
    }

    fn nelem_expr(&mut self) -> (String, f64, f64) {
        if self.exotic && self.rng.chance(0.15) {
            self.feat("empty_collection");
            return ("0".into(), 0.0, 0.0);
        }
        match self.rng.below(3) {
            0 => ("IntPMF[(2;0.5)(4;0.5)]".into(), 3.0, 4.0),
            1 => ("UniInt(1, 3)".into(), 2.0, 3.0),
            _ => {
                let c = self.rng.range(1, 3);
                (c.to_string(), c as f64, c as f64)
            }
        }
    }

    /// Input variable usages for all parameters of a signature.
    fn inputs(&mut self, s: &Scope, params: &[Param]) -> (Vec<VarUsage>, f64) {
        let mut v = Vec::new();
        let mut bytes = 0.0;
        for p in params {
            match p.kind {
                ParamKind::Int => {
                    let (e, _, _) = self.int_expr(s, 6.0);
                    v.push(VarUsage {
                        name: p.name.clone(),
                        chars: vec![(Char::Value, e)],
                    });
                }
                ParamKind::Bytes => {
                    self.feat("bytesize");
                    let (e, m) = self.bytes_expr(s);
                    bytes += m;
                    v.push(VarUsage {
                        name: p.name.clone(),
                        chars: vec![(Char::Bytesize, e)],
                    });
                }
                ParamKind::Coll => {
                    self.feat("collection_parameter");
                    let (e, _, _) = self.nelem_expr();
                    v.push(VarUsage {
                        name: p.name.clone(),
                        chars: vec![(Char::NumberOfElements, e)],
                    });
                    let (e, _, _) = self.int_expr(s, 6.0);
                    let mut chars = vec![(Char::Value, e)];
                    if self.rng.chance(0.3 * self.f.bytesize) {
                        let (b, m) = self.bytes_expr(s);
                        bytes += m;
                        chars.push((Char::Bytesize, b));
                    }
                    v.push(VarUsage {
                        name: format!("{}.INNER", p.name),
                        chars,
                    });
                }
            }
        }
        (v, bytes)
    }

    // ------------------------------------------------------------------ SEFFs

    fn behaviour(&mut self, actions: Vec<Action>) -> Behaviour {
        Behaviour {
            id: self.id("rdb"),
            start_id: self.id("act"),
            stop_id: self.id("act"),
            actions,
        }
    }

    fn seffs(&mut self) {
        // middleware first (callers need its cost)
        let mut infra: Option<InfraCtx> = None;
        if let Some((mw, mwa)) = self.middleware {
            let c = self.container_of(mwa);
            let avail = self.avail(&[c]);
            let (prov, _) = self.roles(mw);
            let iface = prov[0].iface;
            let sig = self.m.interfaces[iface].sigs[0].clone();
            let mut scope = Scope::default();
            if let Some(p) = sig.params.first() {
                scope.vars.push(Var {
                    expr: format!("{}.VALUE", p.name),
                    mean: 2.5,
                    max: 6.0,
                    kind: VarKind::Int,
                });
            }
            let ctx = CompCtx {
                container: c,
                avail,
                callees: vec![],
                infra: None,
                rreq: None,
                rreq_hdd: None,
                passive: None,
                comp_vars: vec![],
            };
            let (seff, cost) = self.seff(&ctx, iface, 0, scope, false);
            self.basic(mw).seffs.push(seff);
            infra = Some(InfraCtx {
                role: String::new(),
                sig: sig.id.clone(),
                param: sig.params.first().map(|p| p.name.clone()),
                cost,
            });
        }
        for u in (0..self.units.len()).rev() {
            let unit = self.units[u].clone();
            let mut conts: Vec<usize> = unit.asms.iter().map(|&a| self.container_of(a)).collect();
            conts.dedup();
            let avail = self.avail(&conts);
            // back component of a composite
            if let (Some(b), Some(bi)) = (unit.back, unit.back_iface) {
                let ctx = self.comp_ctx(b, unit.container, avail.clone(), vec![], None);
                let (seffs, costs) = self.comp_seffs(&ctx, bi);
                self.basic(b).seffs = seffs;
                self.units[u].back_cost = costs[0].clone();
            }
            let mut callees = Vec::new();
            let (_, freqs) = self.roles(unit.front);
            let mut k = 0;
            for r in &freqs {
                if self.m.interfaces[r.iface].infra {
                    continue;
                }
                if Some(r.iface) == unit.back_iface {
                    callees.push(Callee {
                        role: r.id.clone(),
                        iface: r.iface,
                        unit: UnitRef::Back(u),
                        link: None,
                    });
                    continue;
                }
                let v = self.units.iter().position(|x| x.iface == r.iface).unwrap();
                let tc = unit
                    .callee_container
                    .get(k)
                    .copied()
                    .unwrap_or(unit.container);
                k += 1;
                callees.push(Callee {
                    role: r.id.clone(),
                    iface: r.iface,
                    unit: UnitRef::Unit(v),
                    link: self.link_between(unit.container, tc),
                });
            }
            let ictx = if unit.infra {
                let role = freqs
                    .iter()
                    .find(|r| self.m.interfaces[r.iface].infra)
                    .map(|r| r.id.clone());
                infra.clone().zip(role).map(|(mut i, r)| {
                    i.role = r;
                    i
                })
            } else {
                None
            };
            let ctx = self.comp_ctx(unit.front, unit.container, avail, callees, ictx);
            let (seffs, costs) = self.comp_seffs(&ctx, unit.iface);
            self.basic(unit.front).seffs = seffs;
            self.units[u].sig_costs = costs;
        }
    }

    fn avail(&self, containers: &[usize]) -> Vec<ResType> {
        let mut v = vec![ResType::Cpu];
        for t in [ResType::Hdd, ResType::Delay] {
            if containers
                .iter()
                .all(|&c| self.m.containers[c].res.iter().any(|r| r.ty == t))
            {
                v.push(t);
            }
        }
        v
    }

    fn comp_ctx(
        &mut self,
        comp: usize,
        container: usize,
        avail: Vec<ResType>,
        callees: Vec<Callee>,
        infra: Option<InfraCtx>,
    ) -> CompCtx {
        let (rreq, rreq_hdd, passive, comp_vars) = match &self.m.components[comp] {
            Component::Basic(b) => (
                b.rreq.clone(),
                b.rreq_hdd.clone(),
                b.passive.first().map(|p| p.id.clone()),
                b.comp_params
                    .iter()
                    .map(|p| match p.chars[0].0 {
                        Char::Bytesize => Var {
                            expr: format!("{}.BYTESIZE", p.name),
                            mean: 2500.0,
                            max: 4000.0,
                            kind: VarKind::Bytes,
                        },
                        _ => Var {
                            expr: format!("{}.VALUE", p.name),
                            mean: 5.0,
                            max: 9.0,
                            kind: VarKind::Int,
                        },
                    })
                    .collect(),
            ),
            _ => (None, None, None, vec![]),
        };
        CompCtx {
            container,
            avail,
            callees,
            infra,
            rreq,
            rreq_hdd,
            passive,
            comp_vars,
        }
    }

    fn comp_seffs(&mut self, ctx: &CompCtx, iface: usize) -> (Vec<Seff>, Vec<Cost>) {
        let n = self.m.interfaces[iface].sigs.len();
        let mut seffs = Vec::new();
        let mut costs = Vec::new();
        for s in 0..n {
            let sig = self.m.interfaces[iface].sigs[s].clone();
            let mut scope = Scope::default();
            scope.vars.extend(ctx.comp_vars.iter().cloned());
            for (pi, p) in sig.params.iter().enumerate() {
                match p.kind {
                    ParamKind::Int => scope.vars.push(Var {
                        expr: format!("{}.VALUE", p.name),
                        mean: 2.5,
                        max: 6.0,
                        kind: VarKind::Int,
                    }),
                    ParamKind::Bytes => scope.vars.push(Var {
                        expr: format!("{}.BYTESIZE", p.name),
                        mean: 2000.0,
                        max: 10000.0,
                        kind: VarKind::Bytes,
                    }),
                    ParamKind::Coll => {
                        let count = Var {
                            expr: format!("{}.NUMBER_OF_ELEMENTS", p.name),
                            mean: 2.5,
                            max: 4.0,
                            kind: VarKind::Int,
                        };
                        scope.vars.push(count.clone());
                        scope.colls.push(Coll {
                            param: p.name.clone(),
                            path: format!(
                                "//@interfaces__Repository.{iface}/@signatures__OperationInterface.{s}/@parameters__OperationSignature.{pi}"
                            ),
                            count,
                            inner: Var {
                                expr: format!("{}.INNER.VALUE", p.name),
                                mean: 2.5,
                                max: 6.0,
                                kind: VarKind::Int,
                            },
                        });
                    }
                }
            }
            let (seff, cost) = self.seff(ctx, iface, s, scope, sig.returns);
            seffs.push(seff);
            costs.push(cost);
        }
        (seffs, costs)
    }

    fn seff(
        &mut self,
        ctx: &CompCtx,
        iface: usize,
        s: usize,
        mut scope: Scope,
        returns: bool,
    ) -> (Seff, Cost) {
        let sig = self.m.interfaces[iface].sigs[s].id.clone();
        let mut budget = if self.deep {
            6 + 2 * self.size() as u32
        } else {
            3 + self.size() as u32
        };
        let (mut acts, cost) = self.block(ctx, &mut scope, &mut budget);
        if returns {
            self.feat("set_variable");
            let (e, _, _) = self.int_expr(&scope, 6.0);
            let e = if e == "0" { "1".to_string() } else { e };
            // every returning SEFF sets RETURN.VALUE and a second output `aux` (VALUE, BYTESIZE)
            let (a, _, _) = self.int_expr(&scope, 6.0);
            let (b, _) = self.bytes_expr(&scope);
            let id = self.id("act");
            acts.push(Action::SetVar {
                id,
                name: "setReturn".into(),
                usages: vec![
                    VarUsage {
                        name: "RETURN".into(),
                        chars: vec![(Char::Value, e)],
                    },
                    VarUsage {
                        name: "aux".into(),
                        chars: vec![(Char::Value, a), (Char::Bytesize, b)],
                    },
                ],
            });
        }
        let body = self.behaviour(acts);
        (
            Seff {
                id: self.id("seff"),
                sig,
                body,
            },
            cost,
        )
    }

    fn block(&mut self, ctx: &CompCtx, scope: &mut Scope, budget: &mut u32) -> (Vec<Action>, Cost) {
        let n = 1 + self
            .rng
            .below(if scope.depth == 0 || self.deep { 3 } else { 2 });
        let mut acts = Vec::new();
        let mut cost = Cost::new();
        for _ in 0..n {
            if *budget == 0 && !acts.is_empty() {
                break;
            }
            *budget = budget.saturating_sub(1);
            let (a, c) = self.action(ctx, scope, budget);
            *cost.entry(Key::Actions).or_insert(0.0) += a.len() as f64;
            acts.extend(a);
            add(&mut cost, &c, 1.0);
        }
        (acts, cost)
    }

    fn internal(&mut self, ctx: &CompCtx, scope: &Scope) -> (Action, Cost) {
        let mut cost = Cost::new();
        let mut demands = Vec::new();
        let first = if ctx.avail.len() > 1 && self.rng.chance(0.35) {
            *self.rng.pick(&ctx.avail[1..])
        } else {
            ResType::Cpu
        };
        let mut types = vec![first];
        if ctx.avail.len() > 1 && self.rng.chance(0.25) {
            let t = *self.rng.pick(&ctx.avail);
            if !types.contains(&t) {
                types.push(t);
            }
        }
        for t in types {
            let (e, mean) = self.demand_expr(scope);
            let rate = self.rate_of(ctx.container, t);
            *cost.entry(Key::Proc(ctx.container, t)).or_insert(0.0) += mean / rate;
            demands.push((t, e));
        }
        let mut infra = Vec::new();
        if let Some(i) = &ctx.infra
            && self.rng.chance(0.6)
        {
            let (count, cm) = if self.rng.chance(0.5) {
                ("1".to_string(), 1.0)
            } else {
                self.count_expr(scope)
            };
            let inputs = match &i.param {
                Some(p) => {
                    let (e, _, _) = self.int_expr(scope, 6.0);
                    vec![VarUsage {
                        name: p.clone(),
                        chars: vec![(Char::Value, e)],
                    }]
                }
                None => vec![],
            };
            add(&mut cost, &i.cost, cm);
            infra.push(InfraCall {
                id: self.id("ic"),
                role: i.role.clone(),
                sig: i.sig.clone(),
                count,
                inputs,
            });
            if self.rng.chance(0.3) {
                demands.clear(); // pure infrastructure action
            }
        }
        let mut rescalls = Vec::new();
        if let Some(rreq) = &ctx.rreq
            && self.rng.chance(0.5)
        {
            self.feat("resource_call");
            let (e, mean) = self.demand_expr(scope);
            let rate = self.rate_of(ctx.container, ResType::Cpu);
            *cost
                .entry(Key::Proc(ctx.container, ResType::Cpu))
                .or_insert(0.0) += mean / rate;
            let id = self.id("rcall");
            rescalls.push(ResCall {
                id,
                role: rreq.clone(),
                sig: ResSig::CpuProcess,
                count: e,
            });
        }
        if let Some(rreq) = &ctx.rreq_hdd
            && ctx.avail.contains(&ResType::Hdd)
            && self.rng.chance(0.6)
        {
            self.feat("hdd_resource_call");
            for sig in [ResSig::HddRead, ResSig::HddWrite] {
                if !self.rng.chance(0.7) {
                    continue;
                }
                let (e, mean) = self.demand_expr(scope);
                let rate = self.rate_of(ctx.container, ResType::Hdd);
                *cost
                    .entry(Key::Proc(ctx.container, ResType::Hdd))
                    .or_insert(0.0) += 2.0 * mean / rate;
                let id = self.id("rcall");
                rescalls.push(ResCall {
                    id,
                    role: rreq.clone(),
                    sig,
                    count: e,
                });
            }
        }
        (
            Action::Internal {
                id: self.id("act"),
                name: "work".into(),
                demands,
                infra,
                rescalls,
            },
            cost,
        )
    }

    fn rate_of(&self, c: usize, t: ResType) -> f64 {
        self.m.containers[c]
            .res
            .iter()
            .find(|r| r.ty == t)
            .map(|r| r.rate_mean)
            .unwrap_or(1.0)
    }

    fn action(
        &mut self,
        ctx: &CompCtx,
        scope: &mut Scope,
        budget: &mut u32,
    ) -> (Vec<Action>, Cost) {
        let f = self.f;
        let deep = scope.depth < if self.deep { 5 } else { 3 };
        let fork_ok = if self.deep {
            scope.depth < 3
        } else {
            scope.depth < 2 && !scope.in_fork
        };
        let calls_ok = !ctx.callees.is_empty();
        let w = [
            3.0,
            if calls_ok {
                2.5 * f.external_call * if scope.in_fork { f.fork_calls } else { 1.0 }
            } else {
                0.0
            },
            if deep { 0.8 * f.prob_branch } else { 0.0 },
            if deep && !self.int_vars(scope, 6.0).is_empty() {
                0.8 * f.guarded_branch
            } else {
                0.0
            },
            if deep { 0.8 * f.loops } else { 0.0 },
            if deep
                && scope
                    .colls
                    .iter()
                    .any(|c| !scope.iterating.contains(&c.param))
            {
                1.2 * f.collection
            } else {
                0.0
            },
            if fork_ok {
                0.6 * (f.fork_sync + f.fork_async)
            } else {
                0.0
            },
            if ctx.passive.is_some() && !scope.holding && !scope.in_fork && deep {
                f.passive * 1.5
            } else {
                0.0
            },
        ];
        match self.rng.weighted(&w).unwrap_or(0) {
            0 => {
                let (a, c) = self.internal(ctx, scope);
                (vec![a], c)
            }
            1 => {
                let callee = self.rng.pick(&ctx.callees).clone();
                let nsig = self.m.interfaces[callee.iface].sigs.len();
                let k = self.rng.below(nsig);
                let sig = self.m.interfaces[callee.iface].sigs[k].clone();
                if scope.in_fork {
                    self.feat("fork_with_calls");
                }
                let (inputs, bytes_in) = self.inputs(scope, &sig.params);
                let mut returns = vec![];
                let mut ret_var = None;
                if sig.returns && self.rng.chance(0.8) {
                    self.feat("return_value");
                    let n = self.counters.get("rv").copied().unwrap_or(0);
                    self.counters.insert("rv", n + 1);
                    let name = format!("r{n}");
                    returns.push(VarUsage {
                        name: name.clone(),
                        chars: vec![(Char::Value, "RETURN.VALUE".into())],
                    });
                    if self.rng.chance(0.4) {
                        self.feat("multiple_outputs");
                        returns.push(VarUsage {
                            name: format!("{name}b"),
                            chars: vec![
                                (Char::Value, "aux.VALUE + 1".into()),
                                (Char::Bytesize, "aux.BYTESIZE".into()),
                            ],
                        });
                    }
                    ret_var = Some(Var {
                        expr: format!("{name}.VALUE"),
                        mean: 3.0,
                        max: 6.0,
                        kind: VarKind::Int,
                    });
                }
                let mut cost = match callee.unit {
                    UnitRef::Unit(v) => self.units[v].sig_costs.get(k).cloned().unwrap_or_default(),
                    UnitRef::Back(u) => self.units[u].back_cost.clone(),
                };
                if let Some(l) = callee.link {
                    let link = &self.m.links[l];
                    let bytes_out = if sig.returns { 2000.0 } else { 0.0 };
                    let tp = if self.m.run.simulate_throughput_of_linking_resources {
                        (bytes_in + bytes_out) / link.throughput_mean
                    } else {
                        0.0
                    };
                    *cost.entry(Key::Link(l)).or_insert(0.0) += 2.0 * link.latency_mean + tp;
                }
                let a = Action::External {
                    id: self.id("act"),
                    name: format!("call_{}", sig.name),
                    role: callee.role.clone(),
                    sig: sig.id.clone(),
                    inputs,
                    returns,
                };
                if let Some(v) = ret_var {
                    scope.vars.push(v);
                }
                (vec![a], cost)
            }
            2 => {
                self.feat("prob_branch");
                let n = if self.rng.chance(0.3) { 3 } else { 2 };
                let parts: Vec<usize> = if n == 2 {
                    let a = self.rng.range(1, 9);
                    vec![a, 10 - a]
                } else {
                    let a = self.rng.range(1, 8);
                    let b = self.rng.range(1, 9 - a);
                    vec![a, b, 10 - a - b]
                };
                let mut trans = Vec::new();
                let mut cost = Cost::new();
                let odd = self.exotic_probs(parts.len());
                for (k, p) in parts.into_iter().enumerate() {
                    let mut s = scope.clone();
                    s.depth += 1;
                    let (acts, c) = self.block(ctx, &mut s, budget);
                    let pr = p as f64 / 10.0;
                    add(&mut cost, &c, pr);
                    let b = self.behaviour(acts);
                    let spec = match &odd {
                        Some(v) => v[k].clone(),
                        None => javafmt::to_string(pr),
                    };
                    trans.push((self.id("bt"), spec, b));
                }
                (
                    vec![Action::ProbBranch {
                        id: self.id("act"),
                        name: "branch".into(),
                        trans,
                    }],
                    cost,
                )
            }
            3 => {
                self.feat("guarded_branch");
                let vars: Vec<Var> = self.int_vars(scope, 6.0).into_iter().cloned().collect();
                let v = self.rng.pick(&vars).clone();
                let top = (v.max as usize).max(2);
                let conds: Vec<String> = if self.exotic && self.rng.chance(0.4) {
                    self.feat("stoex_exotic_guard");
                    let x = &v.expr;
                    let a = self.rng.range(1, top);
                    match self.rng.below(4) {
                        0 => vec!["BoolPMF[(true;0.4)(false;0.6)]".into(), "true".into()],
                        1 => vec![format!("NOT ({x} < {a})"), format!("{x} < {a}")],
                        2 => vec![
                            format!("{x} <> {a} AND {x} > {a}"),
                            format!("{x} == {a} OR {x} < {a}"),
                        ],
                        _ => vec![format!("{x} >= {a} XOR {x} >= {}", a + 1), "true".into()],
                    }
                } else if self.rng.chance(0.3) && top >= 3 {
                    let a = self.rng.range(1, top - 1);
                    let b = self.rng.range(a + 1, top);
                    vec![
                        format!("{} < {a}", v.expr),
                        format!("{} >= {a} AND {} < {b}", v.expr, v.expr),
                        format!("{} >= {b}", v.expr),
                    ]
                } else {
                    let a = self.rng.range(1, top);
                    vec![format!("{} < {a}", v.expr), format!("{} >= {a}", v.expr)]
                };
                let mut trans = Vec::new();
                let mut cost = Cost::new();
                for c in conds {
                    let mut s = scope.clone();
                    s.depth += 1;
                    let (acts, bc) = self.block(ctx, &mut s, budget);
                    // conservative: every transition's cost counts fully
                    for (k, x) in bc {
                        let e = cost.entry(k).or_insert(0.0);
                        *e = e.max(x);
                    }
                    let b = self.behaviour(acts);
                    trans.push((self.id("bt"), c, b));
                }
                (
                    vec![Action::GuardedBranch {
                        id: self.id("act"),
                        name: "guard".into(),
                        trans,
                    }],
                    cost,
                )
            }
            4 => {
                self.feat("loop");
                let (count, mean) = self.count_expr(scope);
                let mut s = scope.clone();
                s.depth += 1;
                let (acts, c) = self.block(ctx, &mut s, budget);
                let body = self.behaviour(acts);
                let mut cost = Cost::new();
                add(&mut cost, &c, mean);
                (
                    vec![Action::Loop {
                        id: self.id("act"),
                        name: "loop".into(),
                        count,
                        body,
                    }],
                    cost,
                )
            }
            5 => {
                self.feat("collection_iterator");
                let colls: Vec<Coll> = scope
                    .colls
                    .iter()
                    .filter(|c| !scope.iterating.contains(&c.param))
                    .cloned()
                    .collect();
                let c = self.rng.pick(&colls).clone();
                let mut s = scope.clone();
                s.depth += 1;
                s.iterating.push(c.param.clone());
                s.vars.push(c.inner.clone());
                let (acts, bc) = self.block(ctx, &mut s, budget);
                let body = self.behaviour(acts);
                let mut cost = Cost::new();
                add(&mut cost, &bc, c.count.mean);
                (
                    vec![Action::Iterate {
                        id: self.id("act"),
                        name: "iterate".into(),
                        param_path: c.path.clone(),
                        body,
                    }],
                    cost,
                )
            }
            6 if self.dresume && self.rng.chance(0.5) => {
                // sync children that finish without waiting: the parent gets one Resume note per
                // child (SIM-4.4a); the extra ones wake it early from its next wait(s)
                self.feat("double_resume");
                self.feat("fork_sync");
                let ns = self.rng.range(2, 3);
                let mut syncs = Vec::new();
                for _ in 0..ns {
                    let id = self.id("act");
                    let d = *self.rng.pick(&["0.0", "0", "Norm(-1.0, 0.1)"]);
                    let a = Action::Internal {
                        id,
                        name: "noWait".into(),
                        demands: vec![(ResType::Cpu, d.to_string())],
                        infra: vec![],
                        rescalls: vec![],
                    };
                    syncs.push(self.behaviour(vec![a]));
                }
                let sp = self.id("sp");
                (
                    vec![Action::Fork {
                        id: self.id("act"),
                        name: "fork".into(),
                        asyncs: vec![],
                        sync: Some((sp, syncs)),
                    }],
                    Cost::new(),
                )
            }
            6 => {
                let mut cost = Cost::new();
                let mut asyncs = Vec::new();
                let mut syncs = Vec::new();
                let na = if self.rng.chance(f.fork_async) {
                    self.rng.range(1, 2)
                } else {
                    0
                };
                let ns = if self.rng.chance(f.fork_sync) || (na == 0) {
                    self.rng.range(1, 2)
                } else {
                    0
                };
                for k in 0..na + ns {
                    let mut s = scope.clone();
                    s.depth += 1;
                    s.in_fork = true;
                    let (mut acts, c) = self.block(ctx, &mut s, budget);
                    add(&mut cost, &c, 1.0);
                    if k > na {
                        // Reference bug: if two sync children finish without waiting, the parent
                        // gets two Resume notes (SIM-4.4a); the second wakes it early from its
                        // next wait and the resource's later activate() throws "Tried to schedule
                        // thread which was not suspended". Every sync child after the first
                        // therefore starts with a guaranteed CPU wait.
                        let d = *self.rng.pick(&[0.001, 0.002, 0.005]);
                        let rate = self.rate_of(ctx.container, ResType::Cpu);
                        *cost
                            .entry(Key::Proc(ctx.container, ResType::Cpu))
                            .or_insert(0.0) += d / rate;
                        let id = self.id("act");
                        acts.insert(
                            0,
                            Action::Internal {
                                id,
                                name: "syncWait".into(),
                                demands: vec![(ResType::Cpu, javafmt::to_string(d))],
                                infra: vec![],
                                rescalls: vec![],
                            },
                        );
                    }
                    let b = self.behaviour(acts);
                    if k < na {
                        asyncs.push(b);
                    } else {
                        syncs.push(b);
                    }
                }
                if na > 0 {
                    self.feat("fork_async");
                }
                if ns > 0 {
                    self.feat("fork_sync");
                }
                let sync = if syncs.is_empty() {
                    None
                } else {
                    Some((self.id("sp"), syncs))
                };
                (
                    vec![Action::Fork {
                        id: self.id("act"),
                        name: "fork".into(),
                        asyncs,
                        sync,
                    }],
                    cost,
                )
            }
            _ => {
                self.feat("acquire_release");
                let pr = ctx.passive.clone().unwrap();
                let mut s = scope.clone();
                s.depth += 1;
                s.holding = true;
                let (inner, cost) = self.block(ctx, &mut s, budget);
                let mut v = vec![Action::Acquire {
                    id: self.id("act"),
                    name: "acquire".into(),
                    pr: pr.clone(),
                }];
                v.extend(inner);
                v.push(Action::Release {
                    id: self.id("act"),
                    name: "release".into(),
                    pr,
                });
                (v, cost)
            }
        }
    }

    // ------------------------------------------------------------------ usage model + run config

    fn sys_call(&mut self) -> (UAction, Cost) {
        let r = self.rng.below(self.m.sys_roles.len());
        let role = self.m.sys_roles[r].clone();
        let nsig = self.m.interfaces[role.iface].sigs.len();
        let k = self.rng.below(nsig);
        let sig = self.m.interfaces[role.iface].sigs[k].clone();
        let (inputs, _) = self.inputs(&Scope::default(), &sig.params);
        let u = self
            .units
            .iter()
            .position(|x| x.iface == role.iface)
            .unwrap();
        let cost = self.units[u].sig_costs[k].clone();
        (
            UAction::Call {
                id: self.id("ua"),
                name: format!("call_{}", sig.name),
                sys_role: role.id.clone(),
                sig: sig.id.clone(),
                inputs,
            },
            cost,
        )
    }

    fn ubehaviour(&mut self, actions: Vec<UAction>) -> UBehaviour {
        UBehaviour {
            id: self.id("sb"),
            start_id: self.id("ua"),
            stop_id: self.id("ua"),
            actions,
        }
    }

    /// Returns (actions, cost, delay time).
    fn ublock(&mut self, depth: u32) -> (Vec<UAction>, Cost, f64) {
        let n = 1 + self.rng.below((1 + self.size() / 4).min(3));
        let mut acts = Vec::new();
        let mut cost = Cost::new();
        let mut delay = 0.0;
        let f = self.f;
        for _ in 0..n {
            let nested = depth < 2;
            let w = [
                3.0,
                f.usage_delay,
                if nested { f.usage_branch } else { 0.0 },
                if nested { f.usage_loop } else { 0.0 },
            ];
            match self.rng.weighted(&w).unwrap_or(0) {
                0 => {
                    let (a, c) = self.sys_call();
                    acts.push(a);
                    add(&mut cost, &c, 1.0);
                }
                1 => {
                    self.feat("usage_delay");
                    let (spec, m) = match self.rng.below(3) {
                        _ if self.ties => {
                            let d = *self.rng.pick(&[0.0, 0.01, 0.05]);
                            (javafmt::to_string(d), d)
                        }
                        0 => ("0.05".to_string(), 0.05),
                        1 => ("Exp(20.0)".to_string(), 0.05),
                        _ => ("UniDouble(0.01, 0.1)".to_string(), 0.055),
                    };
                    delay += m;
                    acts.push(UAction::Delay {
                        id: self.id("ua"),
                        name: "userDelay".into(),
                        spec,
                    });
                }
                2 => {
                    self.feat("usage_branch");
                    let a = self.rng.range(1, 9);
                    let mut trans = Vec::new();
                    let odd = self.exotic_probs(2);
                    for (k, p) in [a, 10 - a].into_iter().enumerate() {
                        let (inner, c, d) = self.ublock(depth + 1);
                        let pr = p as f64 / 10.0;
                        add(&mut cost, &c, pr);
                        delay += d * pr;
                        let b = self.ubehaviour(inner);
                        let spec = match &odd {
                            Some(v) => v[k].clone(),
                            None => javafmt::to_string(pr),
                        };
                        trans.push((spec, b));
                    }
                    acts.push(UAction::Branch {
                        id: self.id("ua"),
                        name: "userBranch".into(),
                        trans,
                    });
                }
                _ => {
                    self.feat("usage_loop");
                    let (count, m) = match self.rng.below(3) {
                        0 => ("2".to_string(), 2.0),
                        1 => ("IntPMF[(1;0.5)(2;0.5)]".to_string(), 1.5),
                        _ => ("UniInt(1, 3)".to_string(), 2.0),
                    };
                    let (inner, c, d) = self.ublock(depth + 1);
                    add(&mut cost, &c, m);
                    delay += d * m;
                    let body = self.ubehaviour(inner);
                    acts.push(UAction::Loop {
                        id: self.id("ua"),
                        name: "userLoop".into(),
                        count,
                        body,
                    });
                }
            }
        }
        if !acts.iter().any(|a| matches!(a, UAction::Call { .. })) && depth == 0 {
            let (a, c) = self.sys_call();
            acts.push(a);
            add(&mut cost, &c, 1.0);
        }
        (acts, cost, delay)
    }

    fn capacity(&self, k: &Key) -> Option<f64> {
        match k {
            Key::Link(_) => Some(1.0),
            Key::Actions => None,
            Key::Proc(c, t) => {
                let r = self.m.containers[*c].res.iter().find(|r| r.ty == *t)?;
                if r.sched == Sched::Delay {
                    None
                } else {
                    Some(r.replicas as f64)
                }
            }
        }
    }

    fn usage(&mut self) {
        // run flags first (link cost estimation reads them)
        let seed = self.rng.range(1, 999) as i64;
        let ns = if self.rng.chance(self.f.multi_scenario) {
            self.feat("multi_scenario");
            self.rng.range(2, 3)
        } else {
            1
        };
        let target = if self.heavy {
            0.75 + 0.17 * self.rng.f64()
        } else {
            0.25 + 0.4 * self.rng.f64()
        };
        let per = target / ns as f64;
        let mut completion_rate = 0.0;
        let mut actions_per_run: f64 = 0.0;
        for i in 0..ns {
            let (acts, cost, delay) = self.ublock(0);
            actions_per_run = actions_per_run.max(cost.get(&Key::Actions).copied().unwrap_or(0.0));
            let body = self.ubehaviour(acts);
            let maxu: f64 = cost
                .iter()
                .filter_map(|(k, v)| self.capacity(k).map(|c| v / c))
                .fold(0.0, f64::max);
            let resp = total(&cost) + delay;
            let workload = if self.rng.chance(self.f.closed) {
                self.feat("closed_workload");
                let n = self.rng.range(1, 4) as u32;
                let z = (n as f64 * maxu / per - resp).max(0.0) + 0.01;
                completion_rate += n as f64 / (z + resp.max(1e-3));
                let think = match self.rng.below(3) {
                    _ if self.ties => {
                        // multiples of 10 ms: users meet at the same instants. (Think time 0
                        // livelocks at t = 0 when the scenario's demands turn out to be 0, e.g.
                        // exotic StoEx; the reference loops forever too, so never 0.)
                        let zq = if self.rng.chance(0.2) {
                            0.01
                        } else {
                            ((z * 100.0).round() / 100.0).max(0.01)
                        };
                        javafmt::to_string(zq)
                    }
                    0 => nice(z),
                    _ => format!("Exp({})", nice(1.0 / z)),
                };
                Workload::Closed {
                    population: n,
                    think,
                }
            } else {
                self.feat("open_workload");
                let lambda = if maxu > 0.0 {
                    (per / maxu).clamp(0.05, 200.0)
                } else {
                    20.0
                };
                let lambda: f64 = nice(lambda).parse().unwrap();
                completion_rate += lambda;
                let ia = match self.rng.below(5) {
                    _ if self.ties => {
                        // a multiple of 5 ms: arrivals coincide with completions and with the
                        // max-time stop instant
                        let q = ((200.0 / lambda).round().max(1.0)) / 200.0;
                        javafmt::to_string(q)
                    }
                    0 => nice(1.0 / lambda),
                    1 => format!("UniDouble(0.0, {})", nice(2.0 / lambda)),
                    _ => format!("Exp({})", javafmt::to_string(lambda)),
                };
                Workload::Open { interarrival: ia }
            };
            let id_us = self.id("us");
            self.m.scenarios.push(Scenario {
                id: id_us,
                name: format!("Scenario{i}"),
                workload,
                body,
            });
        }
        // keep reference runs short: about 4000 executed actions at most
        let m = if self.long {
            (self.rng.range(300, 3000) as f64)
                .min(60000.0 / actions_per_run.max(1.0))
                .max(100.0) as i64
        } else {
            (self.rng.range(10, 20 + 4 * self.size()) as f64)
                .min(4000.0 / actions_per_run.max(1.0))
                .max(5.0) as i64
        };
        let t_m = m as f64 / completion_rate.max(1e-3);
        // (heavy load: never a pure time stop; an underestimated load would make the run explode)
        let only_time = self.rng.chance(if self.ties { 0.5 } else { 0.1 }) && !self.heavy;
        self.m.run.seed = seed;
        if only_time {
            self.feat("stop_by_time");
            self.m.run.max_measurements = -1;
            self.m.run.max_sim_time = (t_m.ceil() as i64).clamp(1, 100_000);
        } else {
            self.m.run.max_measurements = m;
            self.m.run.max_sim_time = if self.rng.chance(0.5) {
                -1
            } else {
                ((5.0 * t_m).ceil() as i64 + 1).clamp(1, 1_000_000)
            };
        }
        self.windows(t_m);
        self.asm_op_monitors();
        self.triggers(t_m);
        self.nested_container();
        self.middleware_stream();
    }

    /// `simulate_linking_resources = true` with `stream.BYTESIZE` on every call request and
    /// reply (`middleware_stream`); sometimes without the stream (reference abort).
    fn middleware_stream(&mut self) {
        if !self.rng.chance(self.f.middleware_stream) {
            return;
        }
        self.feat("middleware_stream");
        self.m.run.simulate_linking_resources = true;
        if self.rng.chance(0.2) {
            self.feat("middleware_stream_missing");
            return;
        }
        const SIZES: [&str; 3] = ["1000", "UniInt(100, 3000)", "IntPMF[(500;0.5)(2000;0.5)]"];
        fn add_inputs(b: &mut Behaviour, rng: &mut Rng) {
            for a in &mut b.actions {
                match a {
                    Action::External { inputs, .. } => inputs.push(VarUsage {
                        name: "stream".into(),
                        chars: vec![(Char::Bytesize, (*rng.pick(&SIZES)).into())],
                    }),
                    Action::ProbBranch { trans, .. } | Action::GuardedBranch { trans, .. } => {
                        for (_, _, body) in trans {
                            add_inputs(body, rng);
                        }
                    }
                    Action::Loop { body, .. } | Action::Iterate { body, .. } => {
                        add_inputs(body, rng)
                    }
                    Action::Fork { asyncs, sync, .. } => {
                        for body in asyncs.iter_mut() {
                            add_inputs(body, rng);
                        }
                        if let Some((_, bs)) = sync {
                            for body in bs {
                                add_inputs(body, rng);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let infra_sigs: std::collections::HashSet<String> = self
            .m
            .interfaces
            .iter()
            .filter(|i| i.infra)
            .flat_map(|i| i.sigs.iter().map(|s| s.id.clone()))
            .collect();
        for ci in 0..self.m.components.len() {
            let n = match &self.m.components[ci] {
                Component::Basic(b) => b.seffs.len(),
                Component::Composite(_) => 0,
            };
            for si in 0..n {
                let id = self.id("act");
                let size = (*self.rng.pick(&SIZES)).to_string();
                let Component::Basic(b) = &mut self.m.components[ci] else {
                    unreachable!()
                };
                let seff = &mut b.seffs[si];
                add_inputs(&mut seff.body, &mut self.rng);
                if !infra_sigs.contains(&seff.sig) {
                    // a top-level SetVariable writes the reply's stream into the result frame
                    seff.body.actions.push(Action::SetVar {
                        id,
                        name: "setStream".into(),
                        usages: vec![VarUsage {
                            name: "stream".into(),
                            chars: vec![(Char::Bytesize, size)],
                        }],
                    });
                }
            }
        }
    }

    /// A nested resource container with a PS CPU (`nested_container`); sometimes an allocation
    /// context is moved to it (the reference then aborts at its first demand).
    fn nested_container(&mut self) {
        if !self.rng.chance(self.f.nested_container) {
            return;
        }
        self.feat("nested_container");
        let parent = self.rng.below(self.m.containers.len());
        let c = Container {
            id: self.id("nrc"),
            name: format!("Nested{}", self.m.containers[parent].name),
            res: vec![ProcRes {
                id: self.id("prs"),
                ty: ResType::Cpu,
                sched: Sched::Ps,
                rate: "1.0".into(),
                rate_mean: 1.0,
                replicas: 1,
                hdd_rates: None,
            }],
        };
        let alloc = if !self.m.allocation.is_empty() && self.rng.chance(0.25) {
            self.feat("nested_allocation");
            Some(self.rng.below(self.m.allocation.len()))
        } else {
            None
        };
        self.m.nested = Some((parent, c, alloc));
    }

    /// `triggersSelfAdaptations` (`triggers`) and PRM-only aggregations (`prm_aggregation`).
    fn triggers(&mut self, t_m: f64) {
        if !self.rng.chance(self.f.triggers) {
            return;
        }
        self.feat("triggers");
        self.m.triggers = if self.rng.chance(0.4) {
            Triggers::All
        } else {
            Triggers::Some {
                salt: self.rng.next_u64(),
                percent: *self.rng.pick(&[5, 20, 50]),
            }
        };
        if self.rng.chance(0.6) {
            self.feat("reconfiguration_time_monitor");
            let uri = *self
                .rng
                .pick(&["rules/scale.qvto", "rules/scale.henshin#_rule1"]);
            self.m.reconf_monitor = Some(uri.into());
        }
        if self.rng.chance(0.3) {
            self.feat("container_count_monitor");
            self.m.container_count_monitor = true;
        }
        if !self.rng.chance(self.f.prm_aggregation) {
            return;
        }
        // one per scenario at most; not where a TimeDrivenAggregation already replaces the
        // FeedThrough (two response-time calculators for one measuring point abort)
        let base = (t_m / 5.0).clamp(0.05, 50.0);
        for si in 0..self.m.scenarios.len() {
            if self
                .m
                .windows
                .iter()
                .any(|w| w.target == WindowTarget::Scenario(si))
                || !self.rng.chance(0.7)
            {
                continue;
            }
            self.feat("prm_aggregation");
            let fixed = self.rng.chance(0.5);
            let frequency = self.rng.range(1, 4) as u32;
            let number_of_measurements = self.rng.range(1, 6) as u32;
            let retrospection_length: f64 = nice(base * *self.rng.pick(&[0.2, 1.0, 3.0]))
                .parse()
                .unwrap();
            self.m.aggregations.push(Aggregation {
                scenario: si,
                fixed,
                frequency,
                number_of_measurements,
                retrospection_length,
            });
        }
    }

    /// Response-time monitors on up to three assembly operations (`extra_monitors`).
    fn asm_op_monitors(&mut self) {
        if !self.rng.chance(self.f.extra_monitors) {
            return;
        }
        self.feat("assembly_operation_monitors");
        let mut cands: Vec<(String, bool, String, String)> = Vec::new();
        // (infrastructure signatures abort the reference: the measuring point needs an
        // OperationSignature)
        let visit = |asms: &[Assembly], in_system: bool, m: &GenModel| {
            let mut v = Vec::new();
            for a in asms {
                let provides = match &m.components[a.comp] {
                    Component::Basic(b) => &b.provides,
                    Component::Composite(c) => &c.provides,
                };
                for r in provides.iter().filter(|r| !m.interfaces[r.iface].infra) {
                    for s in &m.interfaces[r.iface].sigs {
                        v.push((a.id.clone(), in_system, r.id.clone(), s.id.clone()));
                    }
                }
            }
            v
        };
        cands.extend(visit(&self.m.assemblies, true, &self.m));
        for c in &self.m.components {
            if let Component::Composite(cc) = c {
                cands.extend(visit(&cc.inner, false, &self.m));
            }
        }
        let k = self.rng.range(1, 3).min(cands.len());
        for _ in 0..k {
            let i = self.rng.below(cands.len());
            self.m.asm_op_monitors.push(cands.swap_remove(i));
        }
    }

    /// Sliding-window monitors (`windows`): utilisation of some resource replicas and a
    /// TimeDrivenAggregation on a scenario's response time.
    fn windows(&mut self, t_m: f64) {
        if !self.rng.chance(self.f.windows) {
            return;
        }
        self.feat("windows");
        // window lengths around the expected run length / 5
        let base = (t_m / 5.0).clamp(0.05, 50.0);
        let mut w = Vec::new();
        for ci in 0..self.m.containers.len() {
            for ri in 0..self.m.containers[ci].res.len() {
                if !self.rng.chance(0.6) {
                    continue;
                }
                let replicas = self.m.containers[ci].res[ri].replicas;
                let rep = self.rng.below(replicas as usize) as u32;
                let len: f64 = nice(base * *self.rng.pick(&[0.5, 1.0, 2.0]))
                    .parse()
                    .unwrap();
                let inc = if self.rng.chance(0.5) {
                    len
                } else {
                    nice(len / 2.0).parse().unwrap()
                };
                w.push(Window {
                    target: WindowTarget::Utilisation(ci, ri, rep),
                    len,
                    inc,
                });
            }
        }
        if self.rng.chance(0.4) {
            let si = self.rng.below(self.m.scenarios.len());
            let len: f64 = nice(base).parse().unwrap();
            w.push(Window {
                target: WindowTarget::Scenario(si),
                len,
                inc: nice(len / 3.0).parse().unwrap(),
            });
        }
        self.m.windows = w;
    }
}
