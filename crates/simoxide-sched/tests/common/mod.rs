//! Rust twin of `reference/oracles/sched` (the Java scheduler oracle): parses the same scripts,
//! drives the resources with a minimal DESMO-J-like event list (integer ns, FIFO among equal
//! times) and prints the same trace format.
#![allow(dead_code)]

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use simoxide_sched::time::{SimTime, span};
use simoxide_sched::{
    ActiveResource, Delay, Fcfs, PassiveListener, PassiveResource, ProcessorSharing, PsAlgorithm,
    ResourceListener, VirtualTimeProcessorSharing, Wakeup,
};

pub fn oracle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/oracles/sched")
}

#[derive(Clone, Debug)]
pub struct Step {
    /// `None` = continue immediately ("now").
    pub think: Option<f64>,
    pub a: f64,
    pub b: f64,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub name: String,
    pub start: f64,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug)]
pub struct Script {
    pub kind: String,
    pub cores: u32,
    pub rate: f64,
    pub capacity: u64,
    pub jobs: Vec<Job>,
}

fn think(s: &str) -> Option<f64> {
    if s == "now" {
        None
    } else {
        Some(s.parse().unwrap())
    }
}

pub fn parse(text: &str) -> Script {
    let mut sc = Script {
        kind: String::new(),
        cores: 1,
        rate: 1.0,
        capacity: 1,
        jobs: vec![],
    };
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        let f = |i: usize| -> f64 { t[i].parse().unwrap() };
        match t[0] {
            "resource" => {
                sc.kind = t[1].to_string();
                sc.cores = t[2].parse().unwrap();
                sc.rate = f(3);
            }
            "passive" => {
                sc.kind = "passive".into();
                sc.capacity = t[1].parse().unwrap();
            }
            "job" => {
                let mut steps = vec![Step {
                    think: None,
                    a: f(3),
                    b: 0.0,
                }];
                let mut i = 4;
                while i + 1 < t.len() {
                    steps.push(Step {
                        think: think(t[i]),
                        a: f(i + 1),
                        b: 0.0,
                    });
                    i += 2;
                }
                sc.jobs.push(Job {
                    name: t[1].into(),
                    start: f(2),
                    steps,
                });
            }
            "pjob" => {
                let mut steps = vec![Step {
                    think: None,
                    a: f(3),
                    b: f(4),
                }];
                let mut i = 5;
                while i + 2 < t.len() {
                    steps.push(Step {
                        think: think(t[i]),
                        a: f(i + 1),
                        b: f(i + 2),
                    });
                    i += 3;
                }
                sc.jobs.push(Job {
                    name: t[1].into(),
                    start: f(2),
                    steps,
                });
            }
            other => panic!("bad line {other}"),
        }
    }
    sc
}

fn hex(d: f64) -> String {
    format!("{:x}", d.to_bits())
}

#[derive(Clone, Copy, Debug)]
enum Ev {
    Start(u32),
    Res(Wakeup<u32>),
    Resume(u32),
    Issue(u32, usize),
    Acquire(u32, usize),
    Release(u32, usize),
}

/// DESMO-J-like event list: ordered by (time, insertion sequence).
struct Queue {
    heap: BinaryHeap<Reverse<(SimTime, u64, usize)>>,
    evs: Vec<Ev>,
    seq: u64,
}

impl Queue {
    fn new() -> Self {
        Queue {
            heap: BinaryHeap::new(),
            evs: vec![],
            seq: 0,
        }
    }
    fn push(&mut self, at: SimTime, ev: Ev) {
        self.seq += 1;
        self.evs.push(ev);
        self.heap.push(Reverse((at, self.seq, self.evs.len() - 1)));
    }
    fn pop(&mut self) -> Option<(SimTime, Ev)> {
        self.heap.pop().map(|Reverse((t, _, i))| (t, self.evs[i]))
    }
}

struct Out<'a> {
    text: &'a RefCell<String>,
    now: SimTime,
    names: &'a [String],
}

impl ResourceListener<u32> for Out<'_> {
    fn state_changed(&mut self, core: u32, state: u64) {
        writeln!(self.text.borrow_mut(), "S {} {} {}", self.now, core, state).unwrap();
    }
    fn demand_completed(&mut self, job: u32) {
        writeln!(
            self.text.borrow_mut(),
            "C {} {}",
            self.now,
            self.names[job as usize]
        )
        .unwrap();
    }
}

struct POut<'a> {
    text: &'a RefCell<String>,
    now: SimTime,
    names: &'a [String],
    woken: Vec<u32>,
}

impl PassiveListener<u32> for POut<'_> {
    fn requested(&mut self, job: u32, num: u64) {
        let n = &self.names[job as usize];
        writeln!(self.text.borrow_mut(), "Q {} {} {}", self.now, n, num).unwrap();
    }
    fn acquired(&mut self, job: u32, num: u64, _available: i64) {
        let n = &self.names[job as usize];
        writeln!(self.text.borrow_mut(), "G {} {} {}", self.now, n, num).unwrap();
    }
    fn released(&mut self, job: u32, num: u64, _available: i64) {
        let n = &self.names[job as usize];
        writeln!(self.text.borrow_mut(), "L {} {} {}", self.now, n, num).unwrap();
    }
    fn wake(&mut self, job: u32) {
        // activate(): printed and scheduled immediately (in the driver, via `woken`)
        writeln!(
            self.text.borrow_mut(),
            "A {} {}",
            self.now,
            self.names[job as usize]
        )
        .unwrap();
        self.woken.push(job);
    }
}

/// What a job does when resumed after an activation.
#[derive(Clone, Copy)]
enum Cont {
    None,
    AfterStep(usize),
    Hold(usize),
}

struct Driver<'a> {
    sc: &'a Script,
    names: Vec<String>,
    order: Vec<u32>,
    text: RefCell<String>,
    q: Queue,
    now: SimTime,
    res: Option<ActiveResource<u32>>,
    pres: PassiveResource<u32>,
    cont: Vec<Cont>,
}

impl Driver<'_> {
    fn out(&self) -> Out<'_> {
        Out {
            text: &self.text,
            now: self.now,
            names: &self.names,
        }
    }

    fn line(&self, s: String) {
        let mut t = self.text.borrow_mut();
        t.push_str(&s);
        t.push('\n');
    }

    fn dump_remaining(&self) {
        let rem: Vec<(u32, f64)> = match &self.res {
            Some(ActiveResource::Ps(r)) => r.remaining().collect(),
            Some(ActiveResource::PsVirtualTime(r)) => r.remaining().collect(),
            Some(ActiveResource::Fcfs(r)) => r.remaining().collect(),
            _ => return,
        };
        let mut s = format!("R {}", self.now);
        for &j in &self.order {
            if let Some(&(_, v)) = rem.iter().find(|(k, _)| *k == j) {
                write!(s, " {}={}", self.names[j as usize], hex(v)).unwrap();
            }
        }
        self.line(s);
    }

    fn activate(&mut self, j: u32) {
        self.line(format!("A {} {}", self.now, self.names[j as usize]));
        self.dump_remaining();
        self.q.push(self.now, Ev::Resume(j));
    }

    fn issue(&mut self, j: u32, i: usize) {
        let concrete = self.sc.jobs[j as usize].steps[i].a / self.sc.rate;
        self.line(format!(
            "D {} {} {}",
            self.now,
            self.names[j as usize],
            hex(concrete)
        ));
        if concrete <= 0.0 {
            self.line(format!("Z {} {}", self.now, self.names[j as usize]));
            self.after_step(j, i);
            return;
        }
        self.cont[j as usize] = Cont::AfterStep(i);
        let mut res = self.res.take().unwrap();
        let w = res
            .process(self.now, j, concrete, &mut self.out())
            .expect("resource error");
        self.res = Some(res);
        self.q.push(w.at, Ev::Res(w));
        self.dump_remaining();
    }

    fn after_step(&mut self, j: u32, i: usize) {
        let job = &self.sc.jobs[j as usize];
        if i + 1 >= job.steps.len() {
            self.line(format!("E {} {}", self.now, job.name));
            return;
        }
        match job.steps[i + 1].think {
            None => self.issue(j, i + 1),
            Some(z) => self.q.push(self.now + span(z), Ev::Issue(j, i + 1)),
        }
    }

    fn acquire_step(&mut self, j: u32, i: usize) {
        let num = self.sc.jobs[j as usize].steps[i].a as u64;
        let mut o = POut {
            text: &self.text,
            now: self.now,
            names: &self.names,
            woken: vec![],
        };
        let ok = self.pres.acquire(j, num, &mut o);
        self.line(format!(
            "V {} {} {}",
            self.now,
            self.pres.available(),
            if ok { "granted" } else { "queued" }
        ));
        if ok {
            self.hold(j, i);
        } else {
            self.cont[j as usize] = Cont::Hold(i);
        }
    }

    fn hold(&mut self, j: u32, i: usize) {
        let h = self.sc.jobs[j as usize].steps[i].b;
        self.q.push(self.now + span(h), Ev::Release(j, i));
    }

    fn release(&mut self, j: u32, i: usize) {
        let num = self.sc.jobs[j as usize].steps[i].a as u64;
        let mut o = POut {
            text: &self.text,
            now: self.now,
            names: &self.names,
            woken: vec![],
        };
        self.pres.release(j, num, &mut o);
        // each wake schedules the resumption; nothing else is scheduled in between
        for w in o.woken {
            self.q.push(self.now, Ev::Resume(w));
        }
        self.line(format!("V {} {}", self.now, self.pres.available()));
        let job = &self.sc.jobs[j as usize];
        if i + 1 >= job.steps.len() {
            self.line(format!("E {} {}", self.now, job.name));
            return;
        }
        match job.steps[i + 1].think {
            None => self.acquire_step(j, i + 1),
            Some(z) => self.q.push(self.now + span(z), Ev::Acquire(j, i + 1)),
        }
    }

    fn run(&mut self) {
        while let Some((t, ev)) = self.q.pop() {
            self.now = t;
            match ev {
                Ev::Start(j) => {
                    if self.res.is_some() {
                        self.issue(j, 0)
                    } else {
                        self.acquire_step(j, 0)
                    }
                }
                Ev::Issue(j, i) => self.issue(j, i),
                Ev::Acquire(j, i) => self.acquire_step(j, i),
                Ev::Release(j, i) => self.release(j, i),
                Ev::Res(w) => {
                    let mut res = self.res.take().unwrap();
                    let c = res
                        .on_wakeup(self.now, &w, &mut self.out())
                        .expect("resource error");
                    self.res = Some(res);
                    if let Some(c) = c {
                        if let Some(n) = c.next {
                            self.q.push(n.at, Ev::Res(n));
                        }
                        self.activate(c.job);
                    }
                }
                Ev::Resume(j) => {
                    let c = std::mem::replace(&mut self.cont[j as usize], Cont::None);
                    match c {
                        Cont::AfterStep(i) => self.after_step(j, i),
                        Cont::Hold(i) => self.hold(j, i),
                        Cont::None => panic!("resume without continuation"),
                    }
                }
            }
        }
    }
}

/// Runs a script and returns the trace in the oracle's format.
pub fn run(sc: &Script, ps: PsAlgorithm) -> String {
    let names: Vec<String> = sc.jobs.iter().map(|j| j.name.clone()).collect();
    let mut order: Vec<u32> = (0..names.len() as u32).collect();
    order.sort_by(|a, b| names[*a as usize].cmp(&names[*b as usize]));
    let res = match sc.kind.as_str() {
        "ps" => Some(match ps {
            PsAlgorithm::Exact => ActiveResource::Ps(ProcessorSharing::new(sc.cores)),
            PsAlgorithm::VirtualTime => {
                ActiveResource::PsVirtualTime(VirtualTimeProcessorSharing::new(sc.cores))
            }
        }),
        "fcfs" => Some(ActiveResource::Fcfs(Fcfs::new())),
        "delay" => Some(ActiveResource::Delay(Delay::new())),
        "passive" => None,
        k => panic!("unknown kind {k}"),
    };
    let mut d = Driver {
        sc,
        cont: vec![Cont::None; names.len()],
        names,
        order,
        text: RefCell::new(String::new()),
        q: Queue::new(),
        now: 0,
        res,
        pres: PassiveResource::new(sc.capacity),
    };
    d.line(format!(
        "# {} cores={} rate={} capacity={}",
        sc.kind,
        sc.cores,
        hex(sc.rate),
        sc.capacity
    ));
    for (j, job) in sc.jobs.iter().enumerate() {
        d.q.push(span(job.start), Ev::Start(j as u32));
    }
    d.run();
    d.line(format!("# end {}", d.now));
    d.text.into_inner()
}

/// Completion time (ns) of every step of every job, in trace order: (job name, time).
pub fn completions(trace: &str) -> Vec<(String, SimTime)> {
    trace
        .lines()
        .filter(|l| l.starts_with("C "))
        .map(|l| {
            let mut it = l.split(' ').skip(1);
            let t = it.next().unwrap().parse().unwrap();
            (it.next().unwrap().to_string(), t)
        })
        .collect()
}

/// Open workload through one active resource: `arrivals` yields (inter-arrival time, demand) in
/// seconds; arrivals happen at accumulated `span(inter-arrival)` ns. `check` is called after every
/// resource call with the resource; `done(job, arrival, completion)` for every completion.
/// Returns the number of stale wake-ups.
pub fn run_open(
    res: &mut ActiveResource<u32>,
    arrivals: impl IntoIterator<Item = (f64, f64)>,
    mut check: impl FnMut(&ActiveResource<u32>),
    mut done: impl FnMut(u32, SimTime, SimTime),
) -> u64 {
    #[derive(Clone, Copy)]
    enum E {
        Arrive(f64),
        Res(Wakeup<u32>),
    }
    struct Q {
        heap: BinaryHeap<Reverse<(SimTime, u64, usize)>>,
        evs: Vec<E>,
        free: Vec<usize>,
        seq: u64,
    }
    impl Q {
        fn push(&mut self, at: SimTime, e: E) {
            self.seq += 1;
            let i = match self.free.pop() {
                Some(i) => {
                    self.evs[i] = e;
                    i
                }
                None => {
                    self.evs.push(e);
                    self.evs.len() - 1
                }
            };
            self.heap.push(Reverse((at, self.seq, i)));
        }
        fn pop(&mut self) -> Option<(SimTime, E)> {
            let Reverse((t, _, i)) = self.heap.pop()?;
            self.free.push(i);
            Some((t, self.evs[i]))
        }
    }
    let mut q = Q {
        heap: BinaryHeap::new(),
        evs: vec![],
        free: vec![],
        seq: 0,
    };
    let mut arrivals = arrivals.into_iter();
    let mut arrival_time: Vec<SimTime> = vec![];
    let mut stale = 0;
    let mut t_next = 0;
    if let Some((ia, d)) = arrivals.next() {
        t_next += span(ia);
        q.push(t_next, E::Arrive(d));
    }
    while let Some((now, e)) = q.pop() {
        match e {
            E::Arrive(d) => {
                let job = arrival_time.len() as u32;
                arrival_time.push(now);
                let w = res.process(now, job, d, &mut ()).expect("resource error");
                check(res);
                q.push(w.at, E::Res(w));
                if let Some((ia, d)) = arrivals.next() {
                    t_next += span(ia);
                    q.push(t_next, E::Arrive(d));
                }
            }
            E::Res(w) => match res.on_wakeup(now, &w, &mut ()).expect("resource error") {
                None => stale += 1,
                Some(c) => {
                    check(res);
                    if let Some(n) = c.next {
                        q.push(n.at, E::Res(n));
                    }
                    done(c.job, arrival_time[c.job as usize], now);
                }
            },
        }
    }
    stale
}

/// SplitMix64, for reproducible synthetic workloads in tests.
pub struct SplitMix(pub u64);

impl SplitMix {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in (0, 1].
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) + 1) as f64 / (1u64 << 53) as f64
    }
    pub fn exp(&mut self, rate: f64) -> f64 {
        -self.uniform().ln() / rate
    }
}
