//! Random small transformations over the fuzz metamodel: module properties, helpers and
//! contextual queries, mappings with `when`/`where`/`init`/`end`, `inherits`, `disjuncts`,
//! recursive `map`, `xmap`, `resolve`/`resolveone`/`invresolve`/`late resolve`, `object`
//! expressions, constructors, intermediate classes, user exceptions with `raise`/`try`, loops
//! and `switch`. Bodies use the expression generator ([`super::expr`]); the output model (`o`)
//! and the log are compared strictly.

use super::Rng;
use super::expr::{self, Ck, Cls, Gen, Ty};
use std::fmt::Write;

/// Module properties with the names of [`expr::base_scope`], so that generated expressions
/// work in every operation; the object-valued ones are set at the start of `main`.
const PROPS: &str = "\
property i0 : Integer = 7;
property i1 : Integer = -3;
property r0 : Real = 2.5;
property s0 : String = 'Hello World';
property s1 : String = '';
property b0 : Boolean = true;
property root : FZ::Root = null;
property ns : OrderedSet(FZ::Node) = OrderedSet{};
property n0 : FZ::Node = null;
property n1 : FZ::Node = null;
property all : Set(FZ::Node) = Set{};
property l0 : List(Integer) = List{3, 1, 2};
property d0 : Dict(String, Integer) = Dict{'a' = 1, 'b' = 2};
property seq0 : Sequence(Integer) = Sequence{4, 2, 2, 9};
property set0 : Set(String) = Set{'x', 'y', 'z'};
";

const INIT: &str = "\
\troot := m.rootObjects()![FZ::Root];
\tns := root.nodes;
\tn0 := ns->first();
\tn1 := ns->last();
\tall := m.objectsOfType(FZ::Node);
";

struct P<'r> {
    rng: &'r mut Rng,
    out: String,
    /// Mappings `FZ::Node::<name>() : FZ::Node` defined so far (callable on nodes).
    node_maps: Vec<String>,
    leaf_maps: Vec<String>,
    item_maps: Vec<String>,
    helpers: Vec<(String, Ty)>,
    queries: Vec<(String, Ty, Cls)>,
    interm: bool,
    exceptions: Vec<String>,
    label: usize,
}

fn scope_with(extra: &[(&str, Ty)]) -> Vec<(String, Ty)> {
    let mut s = expr::base_scope();
    for (n, t) in extra {
        s.push((n.to_string(), t.clone()));
    }
    s
}

impl P<'_> {
    fn label(&mut self) -> String {
        self.label += 1;
        format!("@e{}@", self.label)
    }

    fn expr(&mut self, scope: Vec<(String, Ty)>, t: &Ty, depth: u32) -> String {
        let mut g = Gen::new(self.rng, scope);
        g.ex(t, depth).render()
    }

    fn stmt(&mut self, scope: Vec<(String, Ty)>, depth: u32) -> String {
        let l = self.label();
        let mut g = Gen::new(self.rng, scope);
        expr::statement(&mut g, &l, depth).render()
    }

    /// Population statements of a node mapping (result `FZ::Node`, context `self`).
    fn node_population(&mut self, self_ty: Ty, rec: Option<&str>) -> String {
        let scope = scope_with(&[("self", self_ty)]);
        let mut o = String::new();
        let n = self.rng.range(1, 6);
        for _ in 0..n {
            let line =
                match self.rng.below(16) {
                    0 | 1 => format!("result.name := {};", self.expr(scope.clone(), &Ty::Str, 3)),
                    2 => format!("result.value := {};", self.expr(scope.clone(), &Ty::Int, 3)),
                    3 => format!("result.big := {};", self.expr(scope.clone(), &Ty::Int, 2)),
                    4 => format!(
                        "result.weight := {};",
                        self.expr(scope.clone(), &Ty::Real, 2)
                    ),
                    5 => format!("result.flag := {};", self.expr(scope.clone(), &Ty::Bool, 2)),
                    6 => format!("result.kind := {};", self.expr(scope.clone(), &Ty::Enum, 2)),
                    7 => format!("result.tags += {};", self.expr(scope.clone(), &Ty::Str, 2)),
                    8 => format!(
                        "result.tags := {};",
                        self.expr(scope.clone(), &Ty::coll(Ck::Sequence, Ty::Str), 2)
                    ),
                    9 => format!("result.nums += {};", self.expr(scope.clone(), &Ty::Int, 2)),
                    10 => format!(
                        "result.nums := {};",
                        self.expr(scope.clone(), &Ty::coll(Ck::Bag, Ty::Int), 2)
                    ),
                    11 => match rec {
                        Some(r) => format!("result.children := self.children->map {r}();"),
                        None => String::new(),
                    },
                    12 => {
                        let k = self.rng.below(4);
                        match k {
                            0 => "result.refs := self.refs->collect(r | r.resolveone(FZ::Node));"
                                .into(),
                            1 => "result.refs := self.refs.late resolve(FZ::Node);".into(),
                            2 => "result.refs += self.refs->any(true).late resolveone(FZ::Node);"
                                .into(),
                            _ => "result.refs := self.refs;".into(),
                        }
                    }
                    13 => {
                        if self.item_maps.is_empty() {
                            String::new()
                        } else {
                            let im = self.rng.pick(&self.item_maps).clone();
                            if self.rng.pct(50) {
                                format!("result.first := self.first.map {im}();")
                            } else {
                                format!("result.items := self.items->map {im}();")
                            }
                        }
                    }
                    14 => self.stmt(scope.clone(), 2),
                    _ => {
                        let c = self.expr(scope.clone(), &Ty::Bool, 2);
                        let a = self.expr(scope.clone(), &Ty::Str, 2);
                        format!("if ({c}) {{ result.name := {a}; }};")
                    }
                };
            if !line.is_empty() {
                let _ = writeln!(o, "\t\t{line}");
            }
        }
        o
    }

    fn helper(&mut self, k: usize) {
        let t = self
            .rng
            .pick(&[
                Ty::Int,
                Ty::Str,
                Ty::Bool,
                Ty::Real,
                Ty::coll(Ck::Sequence, Ty::Int),
            ])
            .clone();
        let name = format!("h{k}");
        let scope = scope_with(&[("px", Ty::Int), ("ps", Ty::Str)]);
        let kind = if self.rng.pct(50) { "helper" } else { "query" };
        let _ = writeln!(
            self.out,
            "{kind} {name}(px : Integer, ps : String) : {} {{",
            t.src()
        );
        for _ in 0..self.rng.below(3) {
            if kind == "helper" {
                let st = self.stmt(scope.clone(), 2);
                let _ = writeln!(self.out, "\t{st}");
            }
        }
        if self.rng.pct(30) && !self.helpers.is_empty() {
            // call an earlier helper (no recursion cycles)
            let (h, ht) = self.rng.pick(&self.helpers).clone();
            let _ = writeln!(
                self.out,
                "\tvar hv : {} := {h}(px + 1, ps + 'h');",
                ht.src()
            );
        }
        let e = self.expr(scope, &t, 3);
        let _ = writeln!(self.out, "\treturn {e};\n}}\n");
        self.helpers.push((name, t));
    }

    fn query(&mut self, k: usize) {
        let t = self.rng.pick(&[Ty::Int, Ty::Str, Ty::Bool]).clone();
        let name = format!("q{k}");
        let ctx = if self.rng.pct(70) {
            Cls::Node
        } else {
            Cls::Leaf
        };
        let scope = scope_with(&[("self", Ty::Obj(ctx))]);
        let e = self.expr(scope, &t, 3);
        let _ = writeln!(
            self.out,
            "query FZ::{}::{name}() : {} {{\n\treturn {e};\n}}\n",
            ctx.name(),
            t.src()
        );
        self.queries.push((name, t, ctx));
    }

    fn item_mapping(&mut self, k: usize) {
        let name = format!("item{k}");
        let scope = scope_with(&[("self", Ty::Obj(Cls::Item))]);
        let mut body = String::new();
        let _ = writeln!(
            body,
            "\t\tresult.name := {};",
            self.expr(scope.clone(), &Ty::Str, 2)
        );
        if self.rng.pct(60) {
            let _ = writeln!(
                body,
                "\t\tresult.count := {};",
                self.expr(scope.clone(), &Ty::Int, 2)
            );
        }
        if self.rng.pct(50) {
            let _ = writeln!(
                body,
                "\t\tresult.ratio := {};",
                self.expr(scope.clone(), &Ty::Real, 2)
            );
        }
        if self.rng.pct(60) {
            let r = *self.rng.pick(&[
                "self.owner.late resolveone(FZ::Node)",
                "self.owner.resolveone(FZ::Node)",
                "self.owner",
            ]);
            let _ = writeln!(body, "\t\tresult.owner := {r};");
        }
        let when = if self.rng.pct(30) {
            format!("\n\twhen {{ {} }}", self.expr(scope.clone(), &Ty::Bool, 2))
        } else {
            String::new()
        };
        let _ = writeln!(
            self.out,
            "mapping FZ::Item::{name}() : FZ::Item{when} {{\n{body}}}\n"
        );
        self.item_maps.push(name);
    }

    fn node_mapping(&mut self, k: usize) {
        let name = format!("node{k}");
        let leaf = self.rng.pct(20);
        let ctx = if leaf { Cls::Leaf } else { Cls::Node };
        let res = if leaf && self.rng.pct(50) {
            "FZ::Leaf"
        } else {
            "FZ::Node"
        };
        let scope = scope_with(&[("self", Ty::Obj(ctx))]);
        let mut head = format!("mapping FZ::{}::{name}() : {res}", ctx.name());
        if !leaf && !self.node_maps.is_empty() && self.rng.pct(20) {
            let base = self.rng.pick(&self.node_maps).clone();
            let _ = write!(head, " inherits FZ::Node::{base}");
        }
        if self.rng.pct(40) {
            let _ = write!(
                head,
                "\n\twhen {{ {} }}",
                self.expr(scope.clone(), &Ty::Bool, 2)
            );
        }
        if self.rng.pct(15) {
            let _ = write!(
                head,
                "\n\twhere {{ {} }}",
                self.expr(scope.clone(), &Ty::Bool, 1)
            );
        }
        let mut body = String::new();
        if self.rng.pct(30) {
            let st = self.stmt(scope.clone(), 2);
            let _ = writeln!(body, "\tinit {{\n\t\t{st}\n\t}}");
        }
        let rec = if leaf { None } else { Some(name.clone()) };
        let pop = self.node_population(Ty::Obj(ctx), rec.as_deref());
        if self.rng.pct(50) {
            let _ = writeln!(body, "\tpopulation {{\n{pop}\t}}");
        } else {
            body.push_str(&pop);
        }
        if self.rng.pct(25) {
            let l = self.label();
            let _ = writeln!(body, "\tend {{\n\t\tlog('{l}', result.name);\n\t}}");
        }
        let _ = writeln!(self.out, "{head} {{\n{body}}}\n");
        if leaf {
            self.leaf_maps.push(name);
        } else {
            self.node_maps.push(name);
        }
    }

    fn disjunct_mapping(&mut self, k: usize) -> Option<String> {
        if self.node_maps.len() < 2 {
            return None;
        }
        let name = format!("disj{k}");
        let a = self.rng.pick(&self.node_maps).clone();
        let b = self.rng.pick(&self.node_maps).clone();
        let _ = writeln!(
            self.out,
            "mapping FZ::Node::{name}() : FZ::Node disjuncts FZ::Node::{a}, FZ::Node::{b};\n"
        );
        Some(name)
    }

    fn main(&mut self, root_map: &str, extra: &[String]) {
        let mut o = String::from("main() {\n");
        o.push_str(INIT);
        let scope = expr::base_scope();
        let _ = writeln!(o, "\tvar r := root.map {root_map}();");
        let n = self.rng.range(3, 10);
        for _ in 0..n {
            let line = match self.rng.below(20) {
                0 | 1 => {
                    let l = self.label();
                    format!(
                        "log('{l}', all->collect(x | x.resolveone(FZ::Node))->collect(name)->asSequence()->sortedBy(x | x));"
                    )
                }
                2 => {
                    let l = self.label();
                    let mm = self.rng.pick(&self.node_maps).clone();
                    format!("log('{l}', n0.map {mm}().name);")
                }
                3 => {
                    let l = self.label();
                    let mm = self.rng.pick(&self.node_maps).clone();
                    format!("log('{l}', n1.resolveIn(FZ::Node::{mm}, FZ::Node)->size());")
                }
                4 => {
                    let l = self.label();
                    format!("log('{l}', r.invresolveone().oclIsUndefined());")
                }
                5 => {
                    let l = self.label();
                    let e = self.expr(scope.clone(), &Ty::Str, 2);
                    format!(
                        "var o{l2} := object FZ::Node {{ name := {e}; value := i0; }}; log('{l}', o{l2}.name);",
                        l2 = self.label
                    )
                }
                6 => {
                    let l = self.label();
                    let e = self.expr(scope.clone(), &Ty::Str, 2);
                    format!("log('{l}', new FZ::Item({e}).name);")
                }
                7 if !self.exceptions.is_empty() => {
                    let ex = self.rng.pick(&self.exceptions).clone();
                    let c = self.expr(scope.clone(), &Ty::Bool, 2);
                    let l = self.label();
                    let handler = *self
                        .rng
                        .pick(&["Exception", ex.as_str(), "AssertionFailed"]);
                    format!(
                        "try {{ if ({c}) {{ raise {ex}; }}; log('{l}', 'no raise'); }} except ({handler}) {{ log('{l}', 'caught'); }};"
                    )
                }
                8 => {
                    let l = self.label();
                    let e = super::quote(self.rng.pick(&["boom", "", "it's"]));
                    format!(
                        "try {{ raise {e}; }} except (e : Exception) {{ log('{l}', e.getArgument()); }};"
                    )
                }
                9 if self.interm => {
                    let l = self.label();
                    let a = self.expr(scope.clone(), &Ty::Int, 2);
                    let s = self.expr(scope.clone(), &Ty::Str, 2);
                    format!(
                        "var ic{n} := new IC(); ic{n}.a := {a}; ic{n}.s := {s}; ic{n}.many += ic{n}.s; ic{n}.node := n0; log('{l}', ic{n}.many->size() + ic{n}.a);",
                        n = self.label
                    )
                }
                10 if !self.queries.is_empty() => {
                    let (q, _, _) = self.rng.pick(&self.queries).clone();
                    let l = self.label();
                    format!(
                        "log('{l}', all->select(oclIsKindOf(FZ::Leaf))->collect(x | x.oclAsType(FZ::Leaf).{q}())->asSequence()->size());"
                    )
                }
                11 if self.queries.iter().any(|q| q.2 == Cls::Node) => {
                    let qs: Vec<_> = self
                        .queries
                        .iter()
                        .filter(|q| q.2 == Cls::Node)
                        .cloned()
                        .collect();
                    let (q, _, _) = self.rng.pick(&qs).clone();
                    let l = self.label();
                    format!("log('{l}', ns->collect(x | x.{q}().repr()));")
                }
                12 if !self.helpers.is_empty() => {
                    let (h, _) = self.rng.pick(&self.helpers).clone();
                    let l = self.label();
                    let a = self.expr(scope.clone(), &Ty::Int, 2);
                    format!("log('{l}', {h}({a}, s0));")
                }
                13 => {
                    let l = self.label();
                    let c = self.expr(scope.clone(), &Ty::Int, 2);
                    format!(
                        "var w{n} := 0; while (w{n} < ({c}).min(5)) {{ w{n} := w{n} + 1; if (w{n} = 3) {{ break; }}; }}; log('{l}', w{n});",
                        n = self.label
                    )
                }
                14 => {
                    let l = self.label();
                    let e = self.expr(scope.clone(), &Ty::Int, 2);
                    format!(
                        "switch {{ case ({e} > 3) log('{l}', 'big'); case ({e} < 0) log('{l}', 'neg'); else log('{l}', 'other'); }};"
                    )
                }
                15 => {
                    let l = self.label();
                    let mm = self.rng.pick(&self.node_maps).clone();
                    format!("log('{l}', ns->xmap {mm}()->size());")
                }
                16 if !extra.is_empty() => {
                    let l = self.label();
                    let d = self.rng.pick(extra).clone();
                    format!(
                        "log('{l}', all->map {d}()->collect(name)->asSequence()->sortedBy(x | x));"
                    )
                }
                17 => {
                    let l = self.label();
                    format!("log('{l}', o.objectsOfType(FZ::Node)->size());")
                }
                _ => self.stmt(scope.clone(), 3),
            };
            let _ = writeln!(o, "\t{line}");
        }
        o.push_str("}\n");
        self.out.push_str(&o);
    }
}

/// A random transformation `name(in m : FZ, out o : FZ)`.
pub fn transformation(rng: &mut Rng, name: &str) -> String {
    let mut p = P {
        rng,
        out: String::new(),
        node_maps: Vec::new(),
        leaf_maps: Vec::new(),
        item_maps: Vec::new(),
        helpers: Vec::new(),
        queries: Vec::new(),
        interm: false,
        exceptions: Vec::new(),
        label: 0,
    };
    let _ = writeln!(
        p.out,
        "modeltype FZ uses fz('{}');\n\ntransformation {name}(in m : FZ, out o : FZ);\n",
        super::FUZZ_NS_URI
    );
    p.out.push_str(PROPS);
    p.out.push('\n');
    if p.rng.pct(40) {
        p.interm = true;
        p.out.push_str("intermediate class IC {\n\ta : Integer;\n\ts : String;\n\tmany : Sequence(String);\n\tnode : FZ::Node;\n}\n\n");
    }
    if p.rng.pct(40) {
        p.out
            .push_str("exception ExA {}\nexception ExB extends ExA {}\n\n");
        p.exceptions = vec!["ExA".into(), "ExB".into()];
    }
    p.out
        .push_str("constructor FZ::Item::Item(n : String) {\n\tname := n;\n}\n\n");
    for k in 0..p.rng.range(0, 3) as usize {
        p.helper(k);
    }
    for k in 0..p.rng.range(0, 2) as usize {
        p.query(k);
    }
    for k in 0..p.rng.range(0, 2) as usize {
        p.item_mapping(k);
    }
    for k in 0..p.rng.range(1, 4) as usize {
        p.node_mapping(k);
    }
    if p.node_maps.is_empty() {
        p.out
            .push_str("mapping FZ::Node::nodeX() : FZ::Node {\n\tname := self.name;\n}\n\n");
        p.node_maps.push("nodeX".into());
    }
    let mut extra = Vec::new();
    if p.rng.pct(30)
        && let Some(d) = p.disjunct_mapping(0)
    {
        extra.push(d);
    }
    // the root mapping
    let nm = p.rng.pick(&p.node_maps).clone();
    let items = if p.item_maps.is_empty() {
        String::new()
    } else {
        format!(
            "\titems := self.items->map {}();\n",
            p.rng.pick(&p.item_maps)
        )
    };
    let _ = writeln!(
        p.out,
        "mapping FZ::Root::rootM() : FZ::Root {{\n\tname := self.name;\n\tnodes := self.nodes->map {nm}();\n{items}}}\n"
    );
    p.main("rootM", &extra);
    p.out
}
