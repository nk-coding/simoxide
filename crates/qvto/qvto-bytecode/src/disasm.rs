//! A textual listing of compiled chunks (`qvto disasm`, tests).

use crate::program::{Chunk, Const, Program, Site};
use std::fmt::Write;

/// The listing of one chunk.
pub fn chunk(p: &Program, c: &Chunk) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "chunk {} (regs {}, iters {}, sites {}, handlers {})",
        c.name,
        c.nregs,
        c.n_iters,
        c.sites.len(),
        c.handlers.len()
    );
    for (pc, i) in c.code.iter().enumerate() {
        let line = p
            .line(c.source, c.ips[pc])
            .map(|l| format!("{l:>4}"))
            .unwrap_or_else(|| "   ?".into());
        let _ = writeln!(out, "  {pc:>5} {line}  {}", insn(p, c, i));
    }
    for h in &c.handlers {
        let _ = writeln!(
            out,
            "  try {}..{} -> {}",
            h.start,
            h.end,
            h.catches
                .iter()
                .map(|k| format!("{:?}@{}", k.types, k.target))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    out
}

/// The listing of a whole program.
pub fn program(p: &Program) -> String {
    let mut out = String::new();
    for c in &p.chunks {
        out.push_str(&chunk(p, c));
        out.push('\n');
    }
    out
}

fn insn(p: &Program, c: &Chunk, i: &crate::Insn) -> String {
    use crate::Insn::*;
    let site = |s: u16| -> String { format!("{:?}", c.sites[s as usize]) };
    match *i {
        LoadConst { dst, k } => format!("LoadConst r{dst} = {}", konst(&p.consts[k as usize])),
        CallStd { dst, base, op } => {
            let op = qvto_types::StdOp::all()[op as usize];
            format!("CallStd r{dst} = {op:?}(r{base}..)")
        }
        CallStdX { dst, base, site: s } => format!("CallStdX r{dst} = (r{base}..) {}", site(s)),
        Call { dst, base, op } | CallVirtual { dst, base, op } => {
            let o = &p.ir.operations[op as usize];
            let virt = matches!(i, CallVirtual { .. });
            format!(
                "Call{} r{dst} = {}(r{base}..)",
                if virt { "Virtual" } else { "" },
                o.name
            )
        }
        GetProp { dst, obj, feature } => format!(
            "GetProp r{dst} = r{obj}.{}",
            p.ir.meta
                .feature(qvto_model::FeatureId(feature as u32))
                .name
        ),
        _ => {
            let s = format!("{i:?}");
            match i {
                CallStdX { site: x, .. }
                | CallMapping { site: x, .. }
                | CallMeta { site: x, .. }
                | New { site: x, .. }
                | NewTransformation { site: x, .. }
                | GetPropW { site: x, .. }
                | GetTuple { site: x, .. }
                | GetInterm { site: x, .. }
                | Assign { site: x, .. }
                | Coerce { site: x, .. }
                | SetProp { site: x, .. }
                | SetInterm { site: x, .. }
                | NewTuple { site: x, .. }
                | SortBy { site: x, .. }
                | KindTest { site: x, .. }
                | ObjectInit { site: x, .. }
                | Resolve { site: x, .. }
                | LateResolve { site: x, .. }
                | Log { site: x, .. }
                | AssertFail { site: x, .. }
                | Raise { site: x, .. }
                | Unsupported { site: x } => format!("{s}  ; {}", short(&c.sites[*x as usize])),
                _ => s,
            }
        }
    }
}

fn short(s: &Site) -> String {
    let t = format!("{s:?}");
    if t.len() > 100 {
        format!("{}...", &t[..100])
    } else {
        t
    }
}

fn konst(c: &Const) -> String {
    match c {
        Const::Str(s) => format!("{s:?}"),
        other => format!("{other:?}"),
    }
}
