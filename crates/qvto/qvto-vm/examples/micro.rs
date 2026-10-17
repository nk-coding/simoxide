//! Micro benchmarks of the VM on hand-written IR (the same programs as
//! `examples/micro.qvto`, for a comparison with the Java runner before the analyser exists).
//!
//! `cargo run --release -p qvto-vm --example micro -- [repeat]`

#[path = "../tests/common/mod.rs"]
mod common;

use common::*;
use qvto_sema::ir::*;
use qvto_types::{CollKind, StdOwner, TypeId};
use qvto_vm::RunOptions;
use std::time::Instant;

const I: TypeId = TypeId::INTEGER;
const B: TypeId = TypeId::BOOLEAN;
const V: TypeId = TypeId::OCL_VOID;

fn program(which: &str) -> Irb {
    let mut t = Irb::new("");
    let m = t.entry;
    match which {
        "loop" => {
            // var s := 0; var i := 0; while (i < 1000000) { s := s + i; i := i + 1; }; log(s.toString())
            let s = t.var(m, "s", I);
            let i = t.var(m, "i", I);
            let body = block(vec![
                assign(
                    s,
                    bin(StdOwner::Integer, "+", local(s, I), local(i, I), I),
                    I,
                ),
                assign(i, bin(StdOwner::Integer, "+", local(i, I), int(1), I), I),
            ]);
            let w = e(
                ExprKind::While(
                    Box::new(bin(StdOwner::Integer, "<", local(i, I), int(1_000_000), B)),
                    Box::new(body),
                ),
                V,
            );
            t.main(vec![
                var_init(s, Some(int(0))),
                var_init(i, Some(int(0))),
                w,
                log(to_string(local(s, I))),
            ]);
        }
        "fib" => {
            // helper fib(n : Integer) : Integer { if n <= 1 then n else fib(n-1) + fib(n-2) endif }
            let f = t.op("fib", OpKind::Helper, None, I);
            let n = t.param(f, "n", I, Direction::In);
            let r1 = call(
                f,
                None,
                vec![bin(StdOwner::Integer, "-", local(n, I), int(1), I)],
                I,
            );
            let r2 = call(
                f,
                None,
                vec![bin(StdOwner::Integer, "-", local(n, I), int(2), I)],
                I,
            );
            t.body(
                f,
                vec![e(
                    ExprKind::If(
                        Box::new(bin(StdOwner::Integer, "<=", local(n, I), int(1), B)),
                        Box::new(local(n, I)),
                        Some(Box::new(bin(StdOwner::Integer, "+", r1, r2, I))),
                    ),
                    I,
                )],
            );
            t.main(vec![log(to_string(call(f, None, vec![int(25)], I)))]);
        }
        "coll" => {
            // Sequence{1..100000}->select(x | x mod 3 = 0)->collect(x | x * 2)->sum()
            let seq = t.coll(CollKind::Sequence, I);
            let x = t.local(m, "x", I, LocalKind::Iterator);
            let y = t.local(m, "y", I, LocalKind::Iterator);
            let src = e(
                ExprKind::Coll(
                    CollKind::Sequence,
                    vec![CollPart::Range(int(1), int(100_000))],
                ),
                seq,
            );
            let sel = lp(
                LoopKind::Select,
                src,
                x,
                Some(bin(
                    StdOwner::Integer,
                    "=",
                    bin(StdOwner::Integer, "mod", local(x, I), int(3), I),
                    int(0),
                    B,
                )),
                None,
                seq,
            );
            let col = lp(
                LoopKind::Collect,
                sel,
                y,
                Some(bin(StdOwner::Integer, "*", local(y, I), int(2), I)),
                None,
                seq,
            );
            let sum = std(sop(StdOwner::Collection, "sum", 0), Some(col), vec![], I);
            t.main(vec![log(to_string(sum))]);
        }
        "set" => {
            // var st : Set(Integer) := Set{}; var i := 0;
            // while (i < 20000) { st := st->including(i); i := i + 1 }; log(st->size().toString())
            let set = t.coll(CollKind::Set, I);
            let st = t.var(m, "st", set);
            let i = t.var(m, "i", I);
            let inc = std(
                sop(StdOwner::Set, "including", 1),
                Some(local(st, set)),
                vec![local(i, I)],
                set,
            );
            let body = block(vec![
                assign(st, inc, set),
                assign(i, bin(StdOwner::Integer, "+", local(i, I), int(1), I), I),
            ]);
            let w = e(
                ExprKind::While(
                    Box::new(bin(StdOwner::Integer, "<", local(i, I), int(20_000), B)),
                    Box::new(body),
                ),
                V,
            );
            let size = std(
                sop(StdOwner::String, "size", 0),
                Some(local(st, set)),
                vec![],
                I,
            );
            t.main(vec![
                var_init(st, Some(coll_lit(CollKind::Set, vec![], set))),
                var_init(i, Some(int(0))),
                w,
                log(to_string(size)),
            ]);
        }
        _ => panic!("unknown benchmark {which}"),
    }
    t
}

fn main() {
    if let Some(w) = std::env::args().nth(2) {
        print!("{}", listing(&w));
        return;
    }
    let repeat: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(10);
    for which in ["loop", "fib", "coll", "set"] {
        let mut times = Vec::new();
        let mut log = String::new();
        for _ in 0..repeat {
            let t = program(which);
            let t0 = Instant::now();
            let (out, l) = t.run_with(RunOptions::default(), vec![]);
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
            assert!(out.error.is_none(), "{out:?}");
            log = l;
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "{which:6} min {:8.3} ms  median {:8.3} ms  log {}",
            times[0],
            times[times.len() / 2],
            log.trim()
        );
    }
}

#[allow(dead_code)]
pub fn listing(which: &str) -> String {
    let t = program(which);
    let p = qvto_bytecode::compile(t.p).unwrap();
    qvto_bytecode::disasm::program(&p)
}
