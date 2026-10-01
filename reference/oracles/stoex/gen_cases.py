#!/usr/bin/env python3
"""Generates the StoEx oracle cases (JSON lines on stdout), deterministically.

Sections (id prefix):
  model/   every specification string of every PCM model in the research repos
           (corpus_models.txt, from collect_corpus.py) with three variable bindings each
  hand/    hand-written edge cases (operators, types, literals, PMFs, functions, modes)
  rand/    random well-formed expressions over a fixed set of variables
  fuzz/    random character/token mutations (mostly parse errors)
"""
import json
import os
import random
import re

import argparse

AP = argparse.ArgumentParser()
AP.add_argument("--seed", type=int, default=20250929)
AP.add_argument("--rand", type=int, default=4000, help="number of random expressions")
AP.add_argument("--fuzz", type=int, default=3000, help="number of mutated expressions")
AP.add_argument("--only-generated", action="store_true", help="skip model/hand/edge/mode/dist cases")
AP.add_argument("--varsets", default=None, help="where to write varsets.json (default golden/)")
ARGS = AP.parse_args()

HERE = os.path.dirname(os.path.abspath(__file__))
R = random.Random(ARGS.seed)
cases = []


def add(cid, expr, vars_=(), mode=None, evals=3, uniforms=None, seed=None):
    c = {"id": cid, "expr": expr, "evals": evals,
         "seed": seed if seed is not None else R.randrange(1, 2**40)}
    if vars_ is STD_VARS:
        c["varset"] = "std"
    else:
        c["vars"] = list(vars_)
    if mode:
        c["mode"] = mode
    if uniforms is not None:
        c["uniforms"] = uniforms
    cases.append(c)


def v(id_, t, val):
    return {"id": id_, "t": t, "v": val}


STD_VARS = None  # defined below

# ------------------------------------------------------------------------------------ model/
VAR_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(?:\s*\.\s*[A-Za-z_][A-Za-z0-9_]*)*\s*\.\s*"
                    r"(VALUE|BYTESIZE|NUMBER_OF_ELEMENTS|TYPE|STRUCTURE)\b")
with open(os.path.join(HERE, "corpus_models.txt")) as f:
    model_specs = [json.loads(l) for l in f if l.strip()]
for i, spec in enumerate(model_specs):
    ids = sorted({re.sub(r"\s+", "", m.group(0)) for m in VAR_RE.finditer(spec)})
    for variant in range(3):
        vs = []
        for id_ in ids:
            ch = id_.rsplit(".", 1)[1]
            if ch in ("TYPE", "STRUCTURE"):
                vs.append(v(id_, "string", R.choice(["a", "b", "graphical"])))
            elif ch in ("BYTESIZE", "NUMBER_OF_ELEMENTS") or variant == 0:
                vs.append(v(id_, "int", R.randrange(0, 2000)))
            elif variant == 1:
                vs.append(v(id_, "double", R.uniform(0, 1000)))
            else:
                vs.append(v(id_, "bool", R.random() < 0.5))
        add(f"model/{i}/{variant}", spec, vs)

# ------------------------------------------------------------------------------------- hand/
STD_VARS = [
    v("x.VALUE", "int", 7), v("y.VALUE", "double", 2.5), v("b.VALUE", "bool", True),
    v("s.VALUE", "string", "abc"), v("n.BYTESIZE", "int", 100), v("d.BYTESIZE", "double", 3.5),
    v("z.VALUE", "int", 0), v("neg.VALUE", "int", -3), v("big.VALUE", "int", 2147483647),
    v("min.VALUE", "int", -2147483648), v("nz.VALUE", "double", "0x8000000000000000"),
    v("nan.VALUE", "double", "0x7ff8000000000000"), v("inf.VALUE", "double", "0x7ff0000000000000"),
    v("t.TYPE", "string", "graphical"), v("c.NUMBER_OF_ELEMENTS", "int", 4),
    v("a.b.c.VALUE", "int", 11), v("f.INNER.BYTESIZE", "int", 9),
    v("p.INNER.VALUE", "proxy", "IntPMF[(1;0.5)(2;0.5)] * x.VALUE"),
    v("q.VALUE", "proxy", "y.VALUE + 1"),
]
hand = r"""
1
0
-1
2147483647
2147483648
-2147483648
- 5
--1
- -1
1+2
1-2
1+-1
1--1
2*3
7/2
7/-2
-7/2
7%3
-7%3
7%-3
7.5%2
1/0
1%0
1.0/0
0.0/0
1/0.0
-1/0.0
0/0
-2147483648/-1
-2147483648%-1
2147483647+1
2147483647*2
-(-2147483648)
1.5+2
1+2.5
2^3
2^0.5
2.0^3
(2^3)+1
2^3+1
2^-1
-2^2
2^31
0^0
(-8)^(1/3)
(-8.0)^(1.0/3)
(0-2.0)^0.5
1.5^2
82^10
x.VALUE^2
y.VALUE^2
x.VALUE+1
x.VALUE+1.5
y.VALUE+1
y.VALUE*2
x.VALUE/2
y.VALUE/2
x.VALUE%3
y.VALUE%2
n.BYTESIZE+1
n.BYTESIZE*2.5
n.BYTESIZE/3
d.BYTESIZE+1
d.BYTESIZE*2
d.BYTESIZE
d.BYTESIZE^2
n.BYTESIZE^2
c.NUMBER_OF_ELEMENTS-1
a.b.c.VALUE*2
a . b . c . VALUE
f.INNER.BYTESIZE
p.INNER.VALUE
p.INNER.VALUE+p.INNER.VALUE
q.VALUE
q.VALUE*2
missing.VALUE
missing.BYTESIZE
missing.BYTESIZE+1
missing.NUMBER_OF_ELEMENTS*2
missing.TYPE
b.VALUE
NOT b.VALUE
NOT x.VALUE
NOT true
NOT NOT false
true AND false
true OR false
true XOR true
false OR true AND false
true AND false OR true
b.VALUE AND x.VALUE > 3
x.VALUE AND true
true AND x.VALUE
1 < 2
1 < 2.5
2.5 < 1
1 == 1.0
1.0 == 1
x.VALUE == 7
x.VALUE == 7.0
y.VALUE == 2.5
y.VALUE > x.VALUE
x.VALUE < y.VALUE
s.VALUE == "abc"
s.VALUE <> "abd"
s.VALUE < "abd"
"a" < "B"
"a" == 'a'
t.TYPE == "graphical"
t.TYPE == "x"
true == true
true > false
false >= true
true == 1
"1" == 1
b.VALUE == true
nz.VALUE == 0.0
nz.VALUE < 0.0
0.0 == -0.0
0.0 > -0.0
-0.0 == -0.0
nan.VALUE == nan.VALUE
nan.VALUE > inf.VALUE
nan.VALUE < 1
inf.VALUE > 1e308
1e400
1e400 == inf.VALUE
1E-400
1.e5
1e+5
1.5E3
0.1+0.2
0.1*3
1e308*10
-0.0
- 0.0
nz.VALUE
-nz.VALUE
-z.VALUE
-min.VALUE
-(1<2)
-"a"
-true
1 < 2 ? 3 : 4
1 > 2 ? 3 : 4
x.VALUE > 5 ? 1 : 2.5
(x.VALUE > 5 ? 1 : 2.5) + 1
(1 < 2 ? 3 : 4) * 2
b.VALUE ? "yes" : "no"
1 ? 2 : 3
"true" ? 1 : 2
true ? IntPMF[(1;1.0)] : IntPMF[(2;1.0)]
false ? IntPMF[(1;1.0)] : DoublePMF[(2;1.0)]
(1 < 2) == true
(1 < 2) AND (2 < 3)
NOT 1 < 2
1 < 2 AND 3 > 2 ? 1 : 2
(b.VALUE ? 1 : 2) ? 3 : 4
Trunc(2.7)
Trunc(-2.7)
Trunc(2)
Trunc(2.5e9)
Trunc(-2.5e9)
Trunc(1e300)
Trunc(nan.VALUE)
Trunc("a")
Trunc(1, 2)
Trunc()
Round(2.5)
Round(-2.5)
Round(3.5)
Round(0.49999999999999994)
Round(2)
Round(3e9)
Round(-3e9)
Round(1e20)
Round(nan.VALUE)
Ceil(2.1)
Ceil(-2.1)
Ceil(3e9)
Ceil(-3e9)
Ceil(nan.VALUE)
Ceil(7)
Trunc(2.7) + 1
Round(2.5) * 2
Ceil(2.1) / 2
Log(2, 8)
Log(10, 1000)
Log(2.0, 8)
Log(10, 2)
Log(1, 5)
Log(0, 5)
Log(2, 0)
Log(2, -1)
Log(2.5, 7.25)
Log(2, 8) + 1
Log(2, 8) * 1.0
Log("a", 2)
Sqrt(16)
Sqrt(2)
Sqrt(2.25)
Sqrt(-1)
Sqrt(16) + 1
Sqrt(16) * 2
Sqrt(true)
Min(1, 2)
Max(1, 2)
Min(1.5, 2.5)
Max(1.5, 2.5)
Min(1, 2.5)
Max(2.5, 1)
Min("a", "b")
Max(nan.VALUE, 1.0)
Min(nz.VALUE, 0.0)
Max(nz.VALUE, 0.0)
Min(0.0, nz.VALUE)
Max(0.0, nz.VALUE)
Min(1, 2) + 1
Max(1.5, 2.5) + 1
Min(x.VALUE, 3)
Min(x.VALUE, 3) + 1
Min(n.BYTESIZE, 3) + 1
Min(1)
Max(1, 2, 3)
Min(missing.VALUE, 1)
MinDeviation(10, 1.0, 0.1)
MinDeviation(10, 3.0, 0.1)
MaxDeviation(10, 1.0, 0.1)
MaxDeviation(10, 3.0, 0.1)
MinDeviation(10.5, 1.0, 0.1)
MaxDeviation(10.5, 1.0, 0.1)
MinDeviation("abc", 1.0, 0.1)
MaxDeviation("abc", 1.0, 0.1)
MinDeviation(10, 1, 0.1)
MaxDeviation(10, 1.0)
MaxDeviation(2147483647, 1.0, 0.1)
MinDeviation(10, 1.0, 0.1) + 1
Binom(10, 0.5)
Binom(10, IntPMF[(1;1.0)])
Foo(1)
foo(IntPMF[(1;1.0)])
exp(1)
IntPMF[(1;0.5)(2;0.5)]
IntPMF[(2;0.5)(1;0.5)]
IntPMF[(1;0.3)(2;0.3)(3;0.3)]
IntPMF[(1;0.2)(2;0.2)]
IntPMF[(1;0.5)(2;0.5)(3;0.0)]
IntPMF[(1;0.0)(2;1.0)]
IntPMF[(1;0.0)]
IntPMF[(1;2.0)]
IntPMF[(1;1.5)(2;0.5)]
IntPMF[(1;0.1)(2;1.5)]
IntPMF[(1;0.9999999999)]
IntPMF[(1;0.99999)]
IntPMF[(1;0.999989)]
IntPMF[(1;1e400)]
IntPMF[(-5;0.5)(5;0.5)]
IntPMF[(-2147483648;1)]
IntPMF[(2147483648;1)]
IntPMF[(- 5;1)]
IntPMF[(1;1)(1;0)]
IntPMF[(3;0.25)(1;0.25)(2;0.25)(1;0.25)]
IntPMF[(1;0.5)(2;0.5)] + 1
IntPMF[(1;0.5)(2;0.5)] + 1.5
IntPMF[(1;0.5)(2;0.5)] * IntPMF[(3;0.5)(4;0.5)]
IntPMF[(1;0.5)(2;0.5)] / 2
IntPMF[(1;0.5)(2;0.5)] == 1
IntPMF[(1;0.3)(2;0.3)] + 1
DoublePMF[(1.5;0.5)(2.5;0.5)]
DoublePMF[(1;0.5)(2;0.5)]
DoublePMF[(-1.5;0.5)(-2;0.5)]
DoublePMF[(-0;0.5)(0;0.5)]
DoublePMF[(0;0.5)(-0;0.5)]
DoublePMF[(1.5;0.5)(2.5;0.5)] + 1
DoublePMF[(1.5;0.3)(2.5;0.3)] + 1
EnumPMF[("a";0.5)("b";0.5)]
EnumPMF[("b";0.5)("a";0.5)]
EnumPMF(ordered)[("x";0.2)("y";0.8)]
EnumPMF[("B";0.5)("a";0.5)]
EnumPMF[("a";0.5)("b";0.5)] == "a"
EnumPMF[("a";0.3)("b";0.3)]
EnumPMF[('a';0.5)("b";0.5)]
BoolPMF[(true;0.3)(false;0.7)]
BoolPMF(ordered)[(false;0.1)(true;0.9)]
BoolPMF[(true;0.3)(false;0.7)] AND true
NOT BoolPMF[(true;0.5)(false;0.5)]
BoolPMF[(true;0.3)(false;0.3)]
DoublePDF[(1;0.5)(2;0.5)]
DoublePDF[(2;0.5)(1;0.5)]
DoublePDF[(0;0.5)(1;0.5)]
DoublePDF[(0.5;0.0)(1;0.5)(2;0.5)]
DoublePDF[(1;0.5)(1;0.5)]
DoublePDF[(-1;0.5)(1;0.5)]
DoublePDF[(1;0.3)(2;0.3)]
DoublePDF[(1;0.2)(2;0.2)(3;0.6)]
DoublePDF[(1;0.5)(2;0.0)(3;0.5)]
DoublePDF[(1;1.2)(2;0.3)]
DoublePDF[(10;1.0)]
DoublePDF[(1;0.5)(2;0.5)] + 1
DoublePDF[(1;0.5)(2;0.5)] * 2
IntPMF(ordered)[(1;1)]
IntPMF[]
IntPMF[(1;1)] [x]
DoublePDF(unit="unit")[(1;1)]
"a\nb"
"aA"
"a\u12"
'abc'
'a' == 'b'
"it's"
'it"s'
"x" + "y"
"x" * 2
true + 1
1 + true
x.VALUE + s.VALUE
s.VALUE + 1
/* comment */ 1 + /* c */ 2
1 // trailing
/* only */
1 /* unterminated


1 = 2
1 <> 2
1 >= 1
1 <= 0
2^3^2
1 < 2 < 3
a ? b : c ? d : e
(1))
((1)
f()
x
x.y
x.INNER
VALUE.VALUE
INNER.VALUE
x.VALUE.VALUE
1.5.3
1E
1e
2e+
0.5e05
00.5
01
.5
a$.VALUE
TRUE
not true
1 XOR 2
1 ?2:3
Exp(1)+Exp(2)
"""
for i, line in enumerate(hand.split("\n")[1:-1]):
    add(f"hand/{i}", line, STD_VARS)

# PMF edge uniforms: exact boundaries and the fall-through above the probability sum.
for i, (expr, us) in enumerate([
    ("IntPMF[(1;0.5)(2;0.5)]", ["0x0000000000000000", 0.5, 0.49999999999999994, 0.9999999999999999]),
    ("IntPMF[(1;0.3)(2;0.3)(3;0.3999999999)]", [0.9999999999999999, 0.9999999998, 0.99999999995]),
    ("IntPMF[(1;0.3)(2;0.3)(3;0.3999999999)] + 1", [0.9999999999999999]),
    ("DoublePMF[(1.5;0.3)(2.5;0.6999999999)]", [0.9999999999999999, 0.99999999995]),
    ("DoublePDF[(1;0.3)(2;0.6999999999)]", [0.9999999999999999, 0.0, 0.3]),
    ("DoublePDF[(1;0.5)(2;0.5)]", [0.0, 0.25, 0.5, 0.75, 0.9999999999999999]),
    ("DoublePDF[(0.5;0.0)(1;0.5)(2;0.5)]", [0.0, 0.25, 0.5]),
    ("DoublePDF[(1;0.5)(2;0.0)(3;0.5)]", [0.5, 0.49999999999999994, 0.75]),
    ("BoolPMF[(true;0.3)(false;0.7)]", [0.69999, 0.7, 0.99]),
    ("EnumPMF[(\"z\";0.5)(\"a\";0.5)]", [0.2, 0.7, 0.9999999999999999]),
    ("true OR IntPMF[(1;1.0)] == 1", [0.5]),
    ("false AND IntPMF[(1;1.0)] == 1", [0.5]),
    ("IntPMF[(1;1.0)] == 2 AND IntPMF[(1;1.0)] == 1", [0.5, 0.5]),
    ("true ? IntPMF[(1;1.0)] : DoublePMF[(2;1.0)]", [0.5]),
    ("Min(IntPMF[(1;0.5)(2;0.5)], IntPMF[(1;0.5)(2;0.5)])", [0.1, 0.9, 0.9, 0.1]),
    ("Foo(IntPMF[(1;1.0)])", [0.5]),
    ("Binom(IntPMF[(1;1.0)], 0.5)", [0.5]),
    ("Trunc(IntPMF[(1;1.0)], IntPMF[(1;1.0)])", [0.5, 0.5]),
]):
    add(f"edge/{i}", expr, STD_VARS, evals=len(us) if len(us) < 5 else 5, uniforms=us)

# Variable modes.
for i, expr in enumerate(["missing.VALUE", "missing.BYTESIZE", "missing.NUMBER_OF_ELEMENTS + 1",
                          "missing.BYTESIZE * 2.5", "missing.TYPE", "missing.VALUE + 1",
                          "missing.VALUE == 1", "Min(missing.VALUE, 1)", "NOT missing.VALUE",
                          "missing.VALUE ? 1 : 2", "-missing.VALUE", "missing.BYTESIZE ^ 2",
                          "Trunc(missing.VALUE)", "Exp(missing.VALUE)", "x.VALUE"]):
    for mode in ("DEFAULT", "NULL"):
        add(f"mode/{mode}/{i}", expr, STD_VARS, mode=mode)

# Distribution functions (sampled through simoxide-random).
dists = ["Exp(2.0)", "Exp(1)", "Exp(0)", "Exp(-1)", "Norm(0, 1)", "Norm(10.0, 2.5)", "Norm(0, 0)",
         "Norm(0, -1)", "Norm(1)", "Pois(3)", "Pois(0.5)", "Pois(0)", "Pois(-1)", "UniDouble(1, 2)",
         "UniDouble(2, 1)", "UniDouble(1, 1)", "UniInt(1, 6)", "UniInt(1.0, 6)", "UniInt(6, 1)",
         "Lognorm(0, 1)", "Lognorm(1, 0)", "LognormMoments(10, 2)", "LognormMoments(0, 1)",
         "Gamma(2, 3)", "Gamma(0.5, 1)", "Gamma(0, 1)", "GammaMoments(10, 0.5)", "GammaMoments(10, 0)",
         "Exp(2.0) + 1", "Pois(3) + 1", "UniInt(1, 6) * 2", "Exp(x.VALUE)", "Exp(\"a\")",
         "Norm(y.VALUE, 1) * 2", "Exp(IntPMF[(1;0.5)(2;0.5)])"]
for i, expr in enumerate(dists):
    add(f"dist/{i}", expr, STD_VARS, evals=4)

# ------------------------------------------------------------------------------------- rand/
RVARS = ["x.VALUE", "y.VALUE", "b.VALUE", "s.VALUE", "n.BYTESIZE", "d.BYTESIZE", "c.NUMBER_OF_ELEMENTS",
         "z.VALUE", "big.VALUE", "neg.VALUE", "p.INNER.VALUE", "t.TYPE", "nz.VALUE", "missing.VALUE"]
FUNCS = [("Trunc", 1), ("Round", 1), ("Ceil", 1), ("Sqrt", 1), ("Log", 2), ("Min", 2), ("Max", 2),
         ("MinDeviation", 3), ("MaxDeviation", 3), ("Binom", 2)]


def rnum():
    k = R.random()
    if k < 0.35:
        return str(R.choice([0, 1, 2, 3, 5, 7, 10, 100, 1000, 65536, 2147483647]))
    if k < 0.7:
        return R.choice(["0.0", "0.5", "1.5", "2.0", "2.5", "0.1", "1e3", "3.0E-2", "1e-5", "123.456",
                         "1e300", "1.", "4e9"])
    return str(R.randrange(0, 50))


def rpmf():
    kind = R.choice(["IntPMF", "DoublePMF", "EnumPMF", "BoolPMF", "DoublePDF"])
    n = R.randrange(1, 4)
    probs = [R.choice([0.1, 0.2, 0.25, 0.3, 0.5, 0.0, 1.0]) for _ in range(n)]
    if R.random() < 0.7:
        s = sum(probs)
        probs = [round(p / s, 6) if s > 0 else 1.0 / n for p in probs]
    if kind == "IntPMF":
        vals = [str(R.randrange(-5, 20)) for _ in range(n)]
    elif kind == "DoublePMF":
        vals = [R.choice(["1.5", "2", "-0.5", "3.25", "0.0", "10"]) for _ in range(n)]
    elif kind == "EnumPMF":
        vals = ['"%s"' % R.choice(["a", "b", "c", "A"]) for _ in range(n)]
    elif kind == "BoolPMF":
        vals = [R.choice(["true", "false"]) for _ in range(n)]
    else:
        vals = sorted({round(R.uniform(0.1, 10), 2) for _ in range(n)})
        vals = [str(x) for x in vals]
        probs = probs[:len(vals)]
    pre = kind + ("(ordered)" if kind in ("EnumPMF", "BoolPMF") and R.random() < 0.3 else "")
    return pre + "[" + "".join("(%s;%s)" % (a, repr(p)) for a, p in zip(vals, probs)) + "]"


def atom(d):
    k = R.random()
    if k < 0.3:
        return rnum()
    if k < 0.55:
        return R.choice(RVARS)
    if k < 0.62:
        return R.choice(["true", "false"])
    if k < 0.66:
        return '"' + R.choice(["a", "abc", "graphical"]) + '"'
    if k < 0.76:
        return rpmf()
    if k < 0.88 and d > 0:
        name, ar = R.choice(FUNCS)
        if R.random() < 0.1:
            ar = R.randrange(0, 4)
        return name + "(" + ", ".join(bool_and(d - 1) for _ in range(ar)) + ")"
    if d > 0:
        return "(" + ifelse(d - 1) + ")"
    return rnum()


def unary(d):
    k = R.random()
    if k < 0.1:
        return "-" + unary(d)
    if k < 0.14:
        return "NOT " + unary(d)
    return atom(d)


def power(d):
    s = unary(d)
    if R.random() < 0.08:
        s += " ^ " + unary(d)
    return s


def prod(d):
    s = power(d)
    while R.random() < 0.3:
        s += R.choice([" * ", " / ", " % "]) + power(d)
    return s


def summ(d):
    s = prod(d)
    while R.random() < 0.35:
        s += R.choice([" + ", " - "]) + prod(d)
    return s


def compare(d):
    s = summ(d)
    if R.random() < 0.2:
        s += R.choice([" < ", " > ", " == ", " <> ", " <= ", " >= "]) + summ(d)
    return s


def bool_or(d):
    s = compare(d)
    while R.random() < 0.1:
        s += R.choice([" OR ", " XOR "]) + compare(d)
    return s


def bool_and(d):
    s = bool_or(d)
    while R.random() < 0.08:
        s += " AND " + bool_or(d)
    return s


def ifelse(d):
    s = bool_and(d)
    if R.random() < 0.1:
        s += " ? " + bool_and(d) + " : " + bool_and(d)
    return s


if ARGS.only_generated:
    cases = []
for i in range(ARGS.rand):
    add(f"rand/{i}", ifelse(R.randrange(0, 4)), STD_VARS, evals=2)

# ------------------------------------------------------------------------------------- fuzz/
ALPHABET = list("0123456789.eE+-*/%^()[];,?:<>=!\"' \t\nabxyzVALUEINTPMF_$") + [
    "IntPMF", "DoublePDF", "EnumPMF", "BoolPMF", "DoublePMF", "(ordered)", "AND", "OR", "XOR", "NOT",
    ".VALUE", ".BYTESIZE", ".INNER", "true", "false", "/*", "*/", "//", "\\", "\\n", "\\u", "<>", "==",
    ">=", "<=", "1e", "1.", "Trunc(", "Min(", ")"]
seeds = [c["expr"] for c in cases if c["id"].startswith(("hand/", "rand/"))]
for i in range(ARGS.fuzz):
    s = R.choice(seeds)
    for _ in range(R.randrange(1, 4)):
        op = R.random()
        pos = R.randrange(0, len(s) + 1)
        if op < 0.4:
            s = s[:pos] + R.choice(ALPHABET) + s[pos:]
        elif op < 0.7 and s:
            s = s[:pos] + s[pos + 1:]
        elif s:
            s = s[:pos] + R.choice(ALPHABET) + s[pos + 1:]
    add(f"fuzz/{i}", s, STD_VARS, evals=1)

with open(ARGS.varsets or os.path.join(HERE, "golden", "varsets.json"), "w") as f:
    json.dump({"std": STD_VARS}, f, indent=1)
    f.write("\n")
for c in cases:
    print(json.dumps(c))
