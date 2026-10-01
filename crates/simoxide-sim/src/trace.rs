//! Writer of the `palladio-trace/1` event trace (`docs/guide/formats.md` §2).
//!
//! Lines are assembled in a reusable buffer; field order and number formatting follow the
//! reference writer (`reference/src/refsim/trace/Trace.java`) exactly.

use crate::javafmt::{push_json_double, push_json_str};
use std::io::Write;

pub(crate) struct TraceOut {
    buf: String,
    out: Box<dyn Write>,
    /// Current time as printed (`ns / 1e9`), cached per ns value.
    t_ns: i64,
    t_str: String,
}

impl TraceOut {
    pub fn new(out: Box<dyn Write>) -> TraceOut {
        TraceOut {
            buf: String::with_capacity(1 << 17),
            out,
            t_ns: -1,
            t_str: String::new(),
        }
    }

    pub fn finish(&mut self) {
        let _ = self.out.write_all(self.buf.as_bytes());
        self.buf.clear();
        let _ = self.out.flush();
    }

    #[inline]
    fn start(&mut self, ev: &str, now: i64) {
        if now != self.t_ns {
            self.t_ns = now;
            self.t_str.clear();
            push_json_double(&mut self.t_str, now as f64 / 1e9);
        }
        self.buf.push_str("{\"ev\":\"");
        self.buf.push_str(ev);
        self.buf.push_str("\",\"t\":");
        self.buf.push_str(&self.t_str);
    }

    #[inline]
    fn s(&mut self, k: &str, v: &str) {
        self.buf.push_str(",\"");
        self.buf.push_str(k);
        self.buf.push_str("\":");
        push_json_str(&mut self.buf, v);
    }

    #[inline]
    fn i(&mut self, k: &str, v: i64) {
        self.buf.push_str(",\"");
        self.buf.push_str(k);
        self.buf.push_str("\":");
        self.buf.push_str(&v.to_string());
    }

    #[inline]
    fn d(&mut self, k: &str, v: f64) {
        self.buf.push_str(",\"");
        self.buf.push_str(k);
        self.buf.push_str("\":");
        push_json_double(&mut self.buf, v);
    }

    #[inline]
    fn end(&mut self) {
        self.buf.push_str("}\n");
        if self.buf.len() > (1 << 16) {
            let _ = self.out.write_all(self.buf.as_bytes());
            self.buf.clear();
        }
    }

    pub fn header(&mut self, run: &str, seed: i64, max_sim_time: i64, max_meas: i64) {
        self.start("header", 0);
        self.s("format", "palladio-trace/1");
        self.s("run", run);
        self.i("seed", seed);
        self.i("max_sim_time", max_sim_time);
        self.i("max_measurements", max_meas);
        self.end();
    }

    pub fn spawn(&mut self, now: i64, p: u64, kind: &str, name: &str, parent: u64) {
        self.start("spawn", now);
        self.i("p", p as i64);
        self.s("kind", kind);
        self.s("name", name);
        self.i("parent", parent as i64);
        self.end();
    }

    pub fn pend(&mut self, now: i64, p: u64) {
        self.start("pend", now);
        self.i("p", p as i64);
        self.end();
    }

    /// begin/end of a model element (`ac` for SEFF actions).
    pub fn element(&mut self, now: i64, begin: bool, p: u64, ty: &str, id: &str, ac: Option<&str>) {
        self.start(if begin { "begin" } else { "end" }, now);
        self.i("p", p as i64);
        self.s("type", ty);
        self.s("id", id);
        if let Some(ac) = ac {
            self.s("ac", ac);
        }
        self.end();
    }

    pub fn system_op(&mut self, now: i64, begin: bool, p: u64, role: &str, sig: &str) {
        self.start(if begin { "begin" } else { "end" }, now);
        self.i("p", p as i64);
        self.s("type", "SystemOperation");
        self.s("role", role);
        self.s("sig", sig);
        self.end();
    }

    pub fn assembly_op(&mut self, now: i64, begin: bool, p: u64, ac: &str, role: &str, sig: &str) {
        self.start(if begin { "begin" } else { "end" }, now);
        self.i("p", p as i64);
        self.s("type", "AssemblyOperation");
        self.s("ac", ac);
        self.s("role", role);
        self.s("sig", sig);
        self.end();
    }

    pub fn hold(&mut self, now: i64, p: u64, d: f64) {
        self.start("hold", now);
        self.i("p", p as i64);
        self.d("d", d);
        self.end();
    }

    pub fn branch(&mut self, now: i64, p: u64, id: &str, idx: i64, sel: Option<&str>) {
        self.start("branch", now);
        self.i("p", p as i64);
        self.s("id", id);
        self.i("idx", idx);
        if let Some(sel) = sel {
            self.s("sel", sel);
        }
        self.end();
    }

    /// `loop` / `infra` count events.
    pub fn count(&mut self, now: i64, ev: &str, p: u64, id: &str, n: i64) {
        self.start(ev, now);
        self.i("p", p as i64);
        self.s("id", id);
        self.i("n", n);
        self.end();
    }

    pub fn fork(&mut self, now: i64, p: u64, id: &str, asy: usize, syn: usize) {
        self.start("fork", now);
        self.i("p", p as i64);
        self.s("id", id);
        self.i("async", asy as i64);
        self.i("sync", syn as i64);
        self.end();
    }

    pub fn join(&mut self, now: i64, p: u64, id: &str) {
        self.start("join", now);
        self.i("p", p as i64);
        self.s("id", id);
        self.end();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn demand(
        &mut self,
        now: i64,
        p: u64,
        res: &str,
        rc: &str,
        spec: &str,
        sched: &str,
        d: f64,
        st: f64,
    ) {
        self.start("demand", now);
        self.i("p", p as i64);
        self.s("res", res);
        self.s("rc", rc);
        self.s("spec", spec);
        self.s("sched", sched);
        self.d("d", d);
        self.d("st", st);
        self.end();
    }

    pub fn demand_done(&mut self, now: i64, p: u64, res: &str, rc: &str) {
        self.start("demand_done", now);
        self.i("p", p as i64);
        self.s("res", res);
        self.s("rc", rc);
        self.end();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn passive(
        &mut self,
        now: i64,
        ev: &str,
        p: u64,
        res: &str,
        ac: &str,
        avail: i64,
        queue: Option<usize>,
    ) {
        self.start(ev, now);
        self.i("p", p as i64);
        self.s("res", res);
        self.s("ac", ac);
        self.i("n", 1);
        self.i("avail", avail);
        if let Some(q) = queue {
            self.i("queue", q as i64);
        }
        self.end();
    }

    pub fn meas(&mut self, now: i64, mp: &str, metric: &str, a: f64, b: f64) {
        self.start("meas", now);
        self.s("mp", mp);
        self.s("metric", metric);
        self.buf.push_str(",\"v\":[");
        push_json_double(&mut self.buf, a);
        self.buf.push(',');
        push_json_double(&mut self.buf, b);
        self.buf.push(']');
        self.end();
    }

    pub fn stop(&mut self, now: i64) {
        self.start("stop", now);
        self.end();
    }

    pub fn finish_line(&mut self, now: i64, uniforms: u64, measurements: u64) {
        self.start("finish", now);
        self.i("uniforms", uniforms as i64);
        self.i("measurements", measurements as i64);
        self.end();
    }
}
