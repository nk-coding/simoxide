//! Interpreter listeners (`docs/spec/simulation.md` SIM-14) in every
//! engine: the process engine (SimuLizar flavour exact and fast, Slingshot flavour fast) on
//! `corpus/` and the Slingshot-exact engine on `corpus-ss/` (with its SPD cases):
//!
//! * a run with a listener is the run without (trace, tape, measurements, events, error);
//! * the callbacks are well formed: every process is created once before its callbacks and
//!   ends at most once; assembly operations and external calls nest per process; usage
//!   scenarios alternate begin/end per process; no forked processes where Slingshot's Q23
//!   holds (the Slingshot flavour with Q23, the Slingshot-exact engine's asynchronous forks).

use simoxide_model::{ActionId, RoleId, SignatureId, UserActionId};
use simoxide_sim::listener::{AssemblyOp, Listener, Pid, ProcessKind};
use simoxide_sim::{CompiledModel, FlavourConfig, Mode, Outputs, RunSpec, SimConfig};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Clone, Default)]
struct Buf(Rc<RefCell<Vec<u8>>>);

impl std::io::Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Default, Debug)]
struct Stats {
    created: u64,
    forked_sync: u64,
    forked_async: u64,
    ended: u64,
    scenarios: u64,
    entry: u64,
    sysop: u64,
    asm: u64,
    ext: u64,
    errors: Vec<String>,
}

#[derive(Default)]
struct Proc {
    ended: bool,
    in_scenario: bool,
    /// Open assembly operations (`Ok`) and external calls (`Err`).
    stack: Vec<Result<AssemblyOp, ActionId>>,
}

/// Checks the callbacks as they come.
#[derive(Default)]
struct Check {
    procs: HashMap<Pid, Proc>,
    s: Rc<RefCell<Stats>>,
    last_t: f64,
}

impl Check {
    fn err(&self, m: String) {
        let mut s = self.s.borrow_mut();
        if s.errors.len() < 20 {
            s.errors.push(m);
        }
    }
    fn time(&mut self, t: f64) {
        if t < self.last_t {
            self.err(format!("time goes back: {t} < {}", self.last_t));
        }
        self.last_t = t;
    }
    fn live(&mut self, p: Pid, what: &str) -> Option<&mut Proc> {
        match self.procs.get(&p) {
            None => {
                self.err(format!("{what} of unknown process {p}"));
                None
            }
            Some(x) if x.ended => {
                self.err(format!("{what} of ended process {p}"));
                None
            }
            _ => self.procs.get_mut(&p),
        }
    }
}

impl Listener for Check {
    fn process_created(&mut self, t: f64, p: Pid, parent: Option<Pid>, kind: ProcessKind) {
        self.time(t);
        if self.procs.contains_key(&p) {
            self.err(format!("process {p} created twice"));
        }
        if let Some(q) = parent
            && !self.procs.contains_key(&q)
        {
            self.err(format!("parent {q} of {p} unknown"));
        }
        self.procs.insert(p, Proc::default());
        let mut s = self.s.borrow_mut();
        s.created += 1;
        match kind {
            ProcessKind::Forked {
                synchronous: true, ..
            } => s.forked_sync += 1,
            ProcessKind::Forked {
                synchronous: false, ..
            } => s.forked_async += 1,
            _ => {}
        }
    }
    fn process_ended(&mut self, t: f64, p: Pid) {
        self.time(t);
        if let Some(x) = self.live(p, "end") {
            x.ended = true;
        }
        self.s.borrow_mut().ended += 1;
    }
    fn usage_scenario(&mut self, t: f64, begin: bool, p: Pid, _scenario: u32) {
        self.time(t);
        if let Some(x) = self.live(p, "scenario") {
            let ok = x.in_scenario != begin;
            x.in_scenario = begin;
            if !ok {
                self.err(format!("scenario begin/end out of order in {p}"));
            }
        }
        self.s.borrow_mut().scenarios += 1;
    }
    fn entry_call(&mut self, t: f64, _begin: bool, p: Pid, _call: UserActionId) {
        self.time(t);
        self.live(p, "entry call");
        self.s.borrow_mut().entry += 1;
    }
    fn system_operation(&mut self, t: f64, _b: bool, p: Pid, _r: RoleId, _s: SignatureId) {
        self.time(t);
        self.live(p, "system operation");
        self.s.borrow_mut().sysop += 1;
    }
    fn assembly_operation(&mut self, t: f64, begin: bool, p: Pid, op: &AssemblyOp) {
        self.time(t);
        if let Some(x) = self.live(p, "assembly operation") {
            if begin {
                x.stack.push(Ok(*op));
            } else if x.stack.pop() != Some(Ok(*op)) {
                self.err(format!("assembly operation end without its begin in {p}"));
            }
        }
        self.s.borrow_mut().asm += 1;
    }
    fn external_call(&mut self, t: f64, begin: bool, p: Pid, action: ActionId) {
        self.time(t);
        if let Some(x) = self.live(p, "external call") {
            if begin {
                x.stack.push(Err(action));
            } else if x.stack.pop() != Some(Err(action)) {
                self.err(format!("external call end without its begin in {p}"));
            }
        }
        self.s.borrow_mut().ext += 1;
    }
}

/// trace, tape, measurements, events (or the error).
type Outcome = (String, String, String);

fn run(
    cm: &CompiledModel,
    cfg: SimConfig,
    traced: bool,
    listen: bool,
    spd: bool,
) -> (Outcome, Option<Stats>) {
    let (p, t) = (Buf::default(), Buf::default());
    let stats = Rc::new(RefCell::new(Stats::default()));
    let out = Outputs {
        trace: traced.then(|| Box::new(p.clone()) as Box<dyn std::io::Write>),
        tape: None,
    };
    let listener = listen.then(|| {
        Box::new(Check {
            s: stats.clone(),
            ..Default::default()
        }) as Box<dyn Listener>
    });
    let spd_def = spd
        .then(|| simoxide_spd::sim::spd_of(&cm.model, simoxide_spd::Quirks::slingshot()).unwrap())
        .flatten();
    let r = match &spd_def {
        Some(spd) => {
            let core = simoxide_reconf::QvtoCore::new().unwrap();
            let mut adj = simoxide_reconf::QvtoAdjuster::spd(
                &core,
                &simoxide_reconf::SpdAdjust::new(&spd.quirks()),
            )
            .unwrap();
            let mut log = simoxide_spd::log::VecLog::default();
            let mut hooks = simoxide_spd::ssx::SsxSpd::new(spd, &cm.model, &mut adj, &mut log);
            let mut run = simoxide_sim::ssx::SsxRun::new(cm, cfg, out).unwrap();
            run.set_listener(listener);
            run.finish(Some(&mut hooks))
        }
        None => match listener {
            Some(l) => simoxide_sim::run_with_listener(cm, cfg, out, l),
            None => simoxide_sim::run(cm, cfg, out),
        },
    };
    let res = match r {
        Ok(r) => format!("{}\nevents {}", r.measurements.to_csv(), r.events),
        Err(e) => format!("error {e}"),
    };
    let text = |b: &Buf| String::from_utf8(b.0.borrow().clone()).unwrap();
    let st = listen.then(|| std::mem::take(&mut *stats.borrow_mut()));
    ((text(&p), text(&t), res), st)
}

/// Runs `cm` with and without a listener (traced and not): equal outcomes; the stats.
fn compare(name: &str, cm: &CompiledModel, cfg: &SimConfig, spd: bool) -> Stats {
    let mut stats = None;
    for traced in [true, false] {
        let (a, _) = run(cm, cfg.clone(), traced, false, spd);
        let (b, s) = run(cm, cfg.clone(), traced, true, spd);
        assert!(a.2.len() > 20, "{name}: {}", a.2);
        assert_eq!(a, b, "{name} (traced {traced}): a listener changed the run");
        let s = s.unwrap();
        assert!(s.errors.is_empty(), "{name}: {:?}", s.errors);
        stats = Some(s);
    }
    stats.unwrap()
}

fn process_engine(name: &str, flavour: FlavourConfig, mode: Mode) -> Stats {
    let dir = root().join("corpus").join(name);
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let model = spec.load_model(&dir).unwrap();
    let cm = CompiledModel::compile_flavour(model, flavour).unwrap();
    let mut cfg = spec.sim_config(name);
    cfg.mode = mode;
    let s = compare(
        &format!("{name} {mode:?} {:?}", flavour.kind),
        &cm,
        &cfg,
        false,
    );
    assert!(
        s.created > 0 && s.scenarios > 0 && s.asm > 0,
        "{name}: {s:?}"
    );
    s
}

const CASES: &[&str] = &[
    "h05_open_workload",
    "h06_closed_think",
    "h11_fork_sync",
    "h12_fork_async",
    "x_sl_mediastore",
];

#[test]
fn simulizar_flavour_exact_and_fast() {
    for mode in [Mode::Exact, Mode::Fast] {
        for name in CASES {
            let s = process_engine(name, FlavourConfig::SIMULIZAR, mode);
            if *name == "h12_fork_async" {
                assert!(s.forked_async > 0, "{s:?}");
            }
            if *name == "h11_fork_sync" {
                assert!(s.forked_sync > 0, "{s:?}");
            }
        }
    }
}

#[test]
fn slingshot_flavour() {
    use simoxide_sim::{Quirk, Quirks};
    for name in CASES {
        // the fixed behaviour: forks as in SimuLizar
        let s = process_engine(name, FlavourConfig::slingshot(Quirks::NONE), Mode::Fast);
        if *name == "h12_fork_async" {
            assert!(s.forked_async > 0, "{s:?}");
        }
        // Q23: no asynchronous behaviours, a synchronous one runs in the forking process
        let s = process_engine(
            name,
            FlavourConfig::slingshot(Quirks::NONE.with(Quirk::Q23)),
            Mode::Fast,
        );
        assert_eq!((s.forked_async, s.forked_sync), (0, 0), "{name}: {s:?}");
    }
}

#[test]
fn slingshot_exact_engine() {
    let mut forks = 0;
    for name in [
        "h05_open_workload",
        "h06_closed_think",
        "h11_fork_sync",
        "h12_fork_async",
        "ss_async_fork",
        "x_ss_mediastore",
        "spd_minimal",
        "spd_scale_in",
    ] {
        let dir = root().join("corpus-ss").join(name);
        let case: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        let model_dir = root().join(case["model"].as_str().unwrap());
        // models behind git submodules: skipped without them
        let Ok(model) = RunSpec::default().load_model(&model_dir) else {
            eprintln!("{name}: skipped (model does not load)");
            continue;
        };
        let cm = CompiledModel::compile_flavour(model, FlavourConfig::slingshot_exact()).unwrap();
        let cfg = SimConfig {
            seed: case["seed"].as_i64().unwrap(),
            max_sim_time: case["sim_time"].as_i64().unwrap(),
            max_measurements: -1,
            ..Default::default()
        };
        let s = compare(name, &cm, &cfg, true);
        assert!(s.created > 0 && s.scenarios > 0, "{name}: {s:?}");
        assert_eq!(s.forked_async, 0, "{name}: {s:?}");
        forks += s.forked_sync;
    }
    assert!(forks > 0);
}
