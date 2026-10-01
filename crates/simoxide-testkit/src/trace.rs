//! `trace.jsonl` (`palladio-trace/1`): parsed form and a fast writer that emits lines byte-identical to
//! the reference (`reference/src/refsim/trace/Trace.java`). See `docs/guide/formats.md` §2.

use std::io::{self, Write};

use crate::json::{self, LineBuilder, ParseError, Record};

/// Format tag written in the header.
pub const FORMAT: &str = "palladio-trace/1";

/// Simulation time as printed in the trace: DESMO-J integer nanoseconds divided by 1e9.
#[inline]
pub fn t_from_ns(ns: i64) -> f64 {
    ns as f64 / 1e9
}

/// Recovers the integer nanoseconds from a trace time (exact below 2^53 ns).
#[inline]
pub fn ns_from_t(t: f64) -> i64 {
    (t * 1e9).round() as i64
}

/// A parsed trace: raw lines (for exact comparison and reports) plus parsed records.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub lines: Vec<String>,
    pub records: Vec<Record>,
}

impl Trace {
    pub fn parse(text: &str) -> Result<Trace, ParseError> {
        let mut lines = Vec::with_capacity(text.len() / 120);
        let mut records = Vec::with_capacity(text.len() / 120);
        for (n, l) in text.lines().enumerate() {
            if l.is_empty() {
                continue;
            }
            records.push(json::parse_line(l).map_err(|mut e| {
                e.line = n + 1;
                e
            })?);
            lines.push(l.to_string());
        }
        Ok(Trace { lines, records })
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn header(&self) -> Option<&Record> {
        self.records
            .first()
            .filter(|r| r.get_str("ev") == Some("header"))
    }
    pub fn finish(&self) -> Option<&Record> {
        self.records
            .last()
            .filter(|r| r.get_str("ev") == Some("finish"))
    }
    /// Re-serializes all records (equals the input for reference traces).
    pub fn to_text(&self) -> String {
        let mut b = Vec::with_capacity(self.lines.iter().map(|l| l.len() + 1).sum());
        for r in &self.records {
            r.write(&mut b);
            b.push(b'\n');
        }
        String::from_utf8(b).expect("utf8")
    }
}

/// Event kinds of `palladio-trace/1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    Header,
    Spawn,
    Pend,
    Begin,
    End,
    Hold,
    Branch,
    Loop,
    Infra,
    Fork,
    Join,
    Demand,
    DemandDone,
    Acquire,
    Grant,
    Release,
    Meas,
    Stop,
    Finish,
    Other,
}

impl EventKind {
    pub fn of(ev: &str) -> EventKind {
        match ev {
            "header" => EventKind::Header,
            "spawn" => EventKind::Spawn,
            "pend" => EventKind::Pend,
            "begin" => EventKind::Begin,
            "end" => EventKind::End,
            "hold" => EventKind::Hold,
            "branch" => EventKind::Branch,
            "loop" => EventKind::Loop,
            "infra" => EventKind::Infra,
            "fork" => EventKind::Fork,
            "join" => EventKind::Join,
            "demand" => EventKind::Demand,
            "demand_done" => EventKind::DemandDone,
            "acquire" => EventKind::Acquire,
            "grant" => EventKind::Grant,
            "release" => EventKind::Release,
            "meas" => EventKind::Meas,
            "stop" => EventKind::Stop,
            "finish" => EventKind::Finish,
            _ => EventKind::Other,
        }
    }
    pub fn of_record(r: &Record) -> EventKind {
        r.get_str("ev")
            .map(EventKind::of)
            .unwrap_or(EventKind::Other)
    }
}

/// Short description of the model element an event refers to (`InternalAction _id @ac`,
/// `demand CPU@rc`, ...), for reports.
pub fn describe_element(r: &Record) -> Option<String> {
    let ty = r.get_str("type");
    if let Some(ty) = ty {
        let mut s = ty.to_string();
        if let Some(id) = r.get_str("id") {
            s.push(' ');
            s.push_str(id);
        }
        if let (Some(role), Some(sig)) = (r.get_str("role"), r.get_str("sig")) {
            s.push_str(&format!(" role={role} sig={sig}"));
        }
        if let Some(ac) = r.get_str("ac") {
            s.push_str(&format!(" @{ac}"));
        }
        return Some(s);
    }
    if let Some(res) = r.get_str("res") {
        let mut s = format!("res={res}");
        if let Some(rc) = r.get_str("rc") {
            s.push_str(&format!(" rc={rc}"));
        }
        if let Some(ac) = r.get_str("ac") {
            s.push_str(&format!(" @{ac}"));
        }
        return Some(s);
    }
    if let Some(mp) = r.get_str("mp") {
        return Some(format!("{mp} / {}", r.get_str("metric").unwrap_or("?")));
    }
    r.get_str("id").map(|s| s.to_string())
}

/// Open `begin` elements of process `p` just before record index `upto` (outermost first), plus the
/// spawn record of `p` if present.
pub fn process_stack(records: &[Record], upto: usize, p: i64) -> (Option<String>, Vec<String>) {
    let mut stack: Vec<String> = Vec::new();
    let mut spawn = None;
    for r in &records[..upto.min(records.len())] {
        if r.get_i64("p") != Some(p) {
            continue;
        }
        match EventKind::of_record(r) {
            EventKind::Spawn => {
                spawn = Some(format!(
                    "{} '{}' (parent p={})",
                    r.get_str("kind").unwrap_or("?"),
                    r.get_str("name").unwrap_or("?"),
                    r.get_i64("parent").unwrap_or(-1)
                ))
            }
            EventKind::Begin => stack.push(describe_element(r).unwrap_or_default()),
            EventKind::End => {
                stack.pop();
            }
            _ => {}
        }
    }
    (spawn, stack)
}

/// Streaming trace writer: typed methods for every event, canonical formatting, internal buffer.
///
/// ```
/// let mut w = simoxide_testkit::trace::TraceWriter::new(Vec::new());
/// w.header("m", 1, -1, 100);
/// w.hold(0.0, 1, 0.5);
/// let bytes = w.finish().unwrap();
/// assert_eq!(std::str::from_utf8(&bytes).unwrap().lines().nth(1).unwrap(),
///            r#"{"ev":"hold","t":0.0,"p":1,"d":0.5}"#);
/// ```
pub struct TraceWriter<W: Write> {
    out: W,
    buf: Vec<u8>,
    err: Option<io::Error>,
    lines: u64,
}

const FLUSH_AT: usize = 1 << 16;

impl<W: Write> TraceWriter<W> {
    pub fn new(out: W) -> Self {
        TraceWriter {
            out,
            buf: Vec::with_capacity(FLUSH_AT + 1024),
            err: None,
            lines: 0,
        }
    }

    /// Number of lines written so far.
    pub fn lines(&self) -> u64 {
        self.lines
    }

    #[inline]
    fn after(&mut self) {
        self.lines += 1;
        if self.buf.len() >= FLUSH_AT {
            self.flush_buf();
        }
    }

    fn flush_buf(&mut self) {
        if self.err.is_none()
            && let Err(e) = self.out.write_all(&self.buf)
        {
            self.err = Some(e);
        }
        self.buf.clear();
    }

    /// Starts a custom line `{"ev":ev,"t":t` and returns a builder; call `.end()` on it.
    #[inline]
    pub fn event(&mut self, ev: &str, t: f64) -> LineBuilder<'_> {
        if self.buf.len() >= FLUSH_AT {
            self.flush_buf();
        }
        self.lines += 1;
        let mut b = LineBuilder::new(&mut self.buf);
        b.str("ev", ev).num("t", t);
        b
    }

    #[inline]
    fn start(&mut self, ev: &str, t: f64) -> LineBuilder<'_> {
        let mut b = LineBuilder::new(&mut self.buf);
        b.str("ev", ev).num("t", t);
        b
    }

    /// First line (`t` = 0.0).
    pub fn header(&mut self, run: &str, seed: i64, max_sim_time: i64, max_measurements: i64) {
        self.start("header", 0.0)
            .str("format", FORMAT)
            .str("run", run)
            .int("seed", seed)
            .int("max_sim_time", max_sim_time)
            .int("max_measurements", max_measurements)
            .end();
        self.after();
    }
    pub fn spawn(&mut self, t: f64, p: i64, kind: &str, name: &str, parent: i64) {
        self.start("spawn", t)
            .int("p", p)
            .str("kind", kind)
            .str("name", name)
            .int("parent", parent)
            .end();
        self.after();
    }
    pub fn pend(&mut self, t: f64, p: i64) {
        self.start("pend", t).int("p", p).end();
        self.after();
    }
    /// `begin`/`end` of a user or SEFF action (`ac` for SEFF actions).
    pub fn element(&mut self, begin: bool, t: f64, p: i64, ty: &str, id: &str, ac: Option<&str>) {
        let mut b = self.start(if begin { "begin" } else { "end" }, t);
        b.int("p", p).str("type", ty).str("id", id);
        if let Some(ac) = ac {
            b.str("ac", ac);
        }
        b.end();
        self.after();
    }
    /// `begin`/`end` of `SystemOperation`.
    pub fn system_op(&mut self, begin: bool, t: f64, p: i64, role: &str, sig: &str) {
        self.start(if begin { "begin" } else { "end" }, t)
            .int("p", p)
            .str("type", "SystemOperation")
            .str("role", role)
            .str("sig", sig)
            .end();
        self.after();
    }
    /// `begin`/`end` of `AssemblyOperation`.
    pub fn assembly_op(&mut self, begin: bool, t: f64, p: i64, ac: &str, role: &str, sig: &str) {
        self.start(if begin { "begin" } else { "end" }, t)
            .int("p", p)
            .str("type", "AssemblyOperation")
            .str("ac", ac)
            .str("role", role)
            .str("sig", sig)
            .end();
        self.after();
    }
    pub fn hold(&mut self, t: f64, p: i64, d: f64) {
        self.start("hold", t).int("p", p).num("d", d).end();
        self.after();
    }
    /// `sel` is `None` when no transition was chosen (the reference omits null fields).
    pub fn branch(&mut self, t: f64, p: i64, id: &str, idx: i64, sel: Option<&str>) {
        let mut b = self.start("branch", t);
        b.int("p", p).str("id", id).int("idx", idx);
        if let Some(sel) = sel {
            b.str("sel", sel);
        }
        b.end();
        self.after();
    }
    pub fn loop_(&mut self, t: f64, p: i64, id: &str, n: i64) {
        self.start("loop", t)
            .int("p", p)
            .str("id", id)
            .int("n", n)
            .end();
        self.after();
    }
    pub fn infra(&mut self, t: f64, p: i64, id: &str, n: i64) {
        self.start("infra", t)
            .int("p", p)
            .str("id", id)
            .int("n", n)
            .end();
        self.after();
    }
    pub fn fork(&mut self, t: f64, p: i64, id: &str, n_async: i64, n_sync: i64) {
        self.start("fork", t)
            .int("p", p)
            .str("id", id)
            .int("async", n_async)
            .int("sync", n_sync)
            .end();
        self.after();
    }
    pub fn join(&mut self, t: f64, p: i64, id: &str) {
        self.start("join", t).int("p", p).str("id", id).end();
        self.after();
    }
    #[allow(clippy::too_many_arguments)]
    pub fn demand(
        &mut self,
        t: f64,
        p: i64,
        res: &str,
        rc: &str,
        spec: &str,
        sched: &str,
        d: f64,
        st: f64,
    ) {
        self.start("demand", t)
            .int("p", p)
            .str("res", res)
            .str("rc", rc)
            .str("spec", spec)
            .str("sched", sched)
            .num("d", d)
            .num("st", st)
            .end();
        self.after();
    }
    pub fn demand_done(&mut self, t: f64, p: i64, res: &str, rc: &str) {
        self.start("demand_done", t)
            .int("p", p)
            .str("res", res)
            .str("rc", rc)
            .end();
        self.after();
    }
    #[allow(clippy::too_many_arguments)]
    pub fn acquire(&mut self, t: f64, p: i64, res: &str, ac: &str, n: i64, avail: i64, queue: i64) {
        self.start("acquire", t)
            .int("p", p)
            .str("res", res)
            .str("ac", ac)
            .int("n", n)
            .int("avail", avail)
            .int("queue", queue)
            .end();
        self.after();
    }
    pub fn grant(&mut self, t: f64, p: i64, res: &str, ac: &str, n: i64, avail: i64) {
        self.start("grant", t)
            .int("p", p)
            .str("res", res)
            .str("ac", ac)
            .int("n", n)
            .int("avail", avail)
            .end();
        self.after();
    }
    pub fn release(&mut self, t: f64, p: i64, res: &str, ac: &str, n: i64, avail: i64) {
        self.start("release", t)
            .int("p", p)
            .str("res", res)
            .str("ac", ac)
            .int("n", n)
            .int("avail", avail)
            .end();
        self.after();
    }
    /// Measurement tuple `[time, value]` (normalized order, `docs/guide/formats.md` §4).
    pub fn meas(&mut self, t: f64, mp: &str, metric: &str, time: f64, value: f64) {
        self.start("meas", t)
            .str("mp", mp)
            .str("metric", metric)
            .nums("v", &[time, value])
            .end();
        self.after();
    }
    pub fn stop(&mut self, t: f64) {
        self.start("stop", t).end();
        self.after();
    }
    pub fn finish_event(&mut self, t: f64, uniforms: i64, measurements: i64) {
        self.start("finish", t)
            .int("uniforms", uniforms)
            .int("measurements", measurements)
            .end();
        self.after();
    }
    /// Writes a parsed record verbatim.
    pub fn record(&mut self, r: &Record) {
        r.write(&mut self.buf);
        self.buf.push(b'\n');
        self.after();
    }

    /// Flushes and returns the sink (first I/O error, if any).
    pub fn finish(mut self) -> io::Result<W> {
        self.flush_buf();
        if let Some(e) = self.err.take() {
            return Err(e);
        }
        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_matches_reference_lines() {
        let mut w = TraceWriter::new(Vec::new());
        w.header("h11_fork_sync", 10, -1, 50);
        w.spawn(0.0, 2, "OpenWorkloadUser", "OpenUser", 1);
        w.element(true, 0.0, 2, "StartAction", "_a", Some("_ac"));
        w.system_op(true, 0.0, 2, "_r", "_s");
        w.assembly_op(false, 0.01, 2, "_SYSTEM_ASSEMBLY_CONTEXT_", "_r", "_s");
        w.demand(
            0.01,
            3,
            "_cpu",
            "_rc",
            "_prs",
            "PROCESSOR_SHARING",
            0.02832458419797288,
            0.02832458419797288,
        );
        w.meas(
            0.01,
            "ActiveResourceMeasuringPoint[_prs|replicaID=2]",
            "Utilization of Active Resource Tuple",
            0.01,
            0.5,
        );
        w.branch(1.0, 2, "_b", -1, None);
        w.fork(0.01, 2, "_f", 0, 2);
        w.stop(21.826651183);
        w.finish_event(21.826651183, 150, 1155);
        let s = String::from_utf8(w.finish().unwrap()).unwrap();
        let expected = [
            r#"{"ev":"header","t":0.0,"format":"palladio-trace/1","run":"h11_fork_sync","seed":10,"max_sim_time":-1,"max_measurements":50}"#,
            r#"{"ev":"spawn","t":0.0,"p":2,"kind":"OpenWorkloadUser","name":"OpenUser","parent":1}"#,
            r#"{"ev":"begin","t":0.0,"p":2,"type":"StartAction","id":"_a","ac":"_ac"}"#,
            r#"{"ev":"begin","t":0.0,"p":2,"type":"SystemOperation","role":"_r","sig":"_s"}"#,
            r#"{"ev":"end","t":0.01,"p":2,"type":"AssemblyOperation","ac":"_SYSTEM_ASSEMBLY_CONTEXT_","role":"_r","sig":"_s"}"#,
            r#"{"ev":"demand","t":0.01,"p":3,"res":"_cpu","rc":"_rc","spec":"_prs","sched":"PROCESSOR_SHARING","d":0.02832458419797288,"st":0.02832458419797288}"#,
            r#"{"ev":"meas","t":0.01,"mp":"ActiveResourceMeasuringPoint[_prs|replicaID=2]","metric":"Utilization of Active Resource Tuple","v":[0.01,0.5]}"#,
            r#"{"ev":"branch","t":1.0,"p":2,"id":"_b","idx":-1}"#,
            r#"{"ev":"fork","t":0.01,"p":2,"id":"_f","async":0,"sync":2}"#,
            r#"{"ev":"stop","t":21.826651183}"#,
            r#"{"ev":"finish","t":21.826651183,"uniforms":150,"measurements":1155}"#,
        ];
        assert_eq!(s.lines().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn ns_roundtrip() {
        for ns in [0i64, 1, 999_999_999, 21_826_651_183, 1 << 52] {
            assert_eq!(ns_from_t(t_from_ns(ns)), ns);
        }
    }
}
