//! Focused parser tests: precedence, the C-style/OCL `if` split, iterator detection, shorthands,
//! diagnostics, and robustness on deep nesting.

use qvto_syntax::{dump, parse, Code};

/// Dump of the first statement of `main`.
fn stmt(src: &str) -> String {
    let full = format!("transformation T();\nmain() {{\n{src}\n}}\n");
    let p = parse(&full);
    assert!(!p.has_errors(), "{src}: {:?}", p.diagnostics);
    let qvto_syntax::ast::Element::Entry(e) = &p.unit.elements[1] else { panic!() };
    dump::expr(&e.body.as_ref().unwrap().stmts[0]).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn errors(src: &str) -> Vec<(Code, String)> {
    let full = format!("transformation T();\nmain() {{\n{src}\n}}\n");
    parse(&full).errors().map(|d| (d.code, d.message.clone())).collect()
}

#[test]
fn precedence() {
    assert_eq!(stmt("a or b and c = d + e * -f;"), r#"(or (name "a") (and (name "b") (= (name "c") (+ (name "d") (* (name "e") (unary - (name "f")))))))"#);
    assert_eq!(stmt("a implies b xor c;"), r#"(implies (name "a") (xor (name "b") (name "c")))"#);
    assert_eq!(stmt("a - b - c;"), r#"(- (- (name "a") (name "b")) (name "c"))"#);
    assert_eq!(stmt("a != b <> c;"), r#"(<> (!= (name "a") (name "b")) (name "c"))"#);
    assert_eq!(stmt("not a.b();"), r#"(unary not (call (src. (name "a")) "b"))"#);
    // let extends as far right as possible.
    assert_eq!(stmt("1 + let x = 2 in x * 3 = 6;"), r#"(+ (int 1) (let (var "x" = (int 2)) (= (* (name "x") (int 3)) (int 6))))"#);
    // assignment is lowest and right-associative.
    assert_eq!(stmt("x := y := a + b;"), r#"(:= (name "x") (:= (name "y") (+ (name "a") (name "b"))))"#);
}

#[test]
fn if_forms() {
    assert_eq!(stmt("if (a) then b else c endif;"), r#"(if (paren (name "a")) (name "b") (else (name "c")))"#);
    assert_eq!(stmt("if (a) and b then c endif;"), r#"(if (and (paren (name "a")) (name "b")) (name "c"))"#);
    assert_eq!(stmt("if (a) x := 1 else x := 2;"), r#"(if-c (name "a") (:= (name "x") (int 1)) (else (:= (name "x") (int 2))))"#);
    // dangling else and endif go to the inner if.
    assert_eq!(stmt("if (a) if (b) x else y;"), r#"(if-c (name "a") (if-c (name "b") (name "x") (else (name "y"))))"#);
    // C-style if is a statement, not a value.
    assert_eq!(errors("var x := if (a) 1 else 2 endif;"), vec![(Code::Expected, r#""then" expected after ")""#.to_string())]);
}

#[test]
fn iterators_and_calls() {
    assert_eq!(stmt("s->select(e | e > 1);"), r#"(iterate (name "s") "select" (var "e") (> (name "e") (int 1)))"#);
    assert_eq!(stmt("s->exists(e = 1);"), r#"(call (src-> (name "s")) "exists" (= (name "e") (int 1)))"#);
    assert_eq!(stmt("s->including(a, b);"), r#"(call (src-> (name "s")) "including" (name "a") (name "b"))"#);
    assert_eq!(stmt("s->iterate(e; acc : Integer = 0 | acc + e);").len() > 0, true);
    assert_eq!(stmt("s->name;"), r#"(nav-> (name "s") "name")"#);
    assert_eq!(stmt("s[A]![x | x.b];"), r#"(![] ([] (name "s") - (name "A")) "x" (nav. (name "x") "b"))"#);
    assert_eq!(stmt("a::b::c(1);"), r#"(call "a" :: "b" :: "c" (int 1))"#);
    assert_eq!(stmt("ecore::EString;"), r#"(path "ecore" "EString")"#);
    assert_eq!(stmt("x.+(1);"), r#"(call (src. (name "x")) #+ (int 1))"#);
}

#[test]
fn literals() {
    assert_eq!(stmt("'a' \n 'b';"), r#"(str "ab")"#);
    assert_eq!(errors("'a' 'b';"), vec![(Code::StringsOnSameLine, "Multiline string literals must be located in different lines!".to_string())]);
    assert_eq!(stmt(r"'\4a';"), r#"(str "\u{4}4a")"#);
    assert_eq!(errors("99999999999999999999;")[0].0, Code::IntegerOverflow);
    assert_eq!(stmt("Sequence{1..3, 5};"), r#"(Sequence{} (.. (int 1) (int 3)) (int 5))"#);
    assert_eq!(errors("Sequence(Integer){1};").is_empty(), false);
}

#[test]
fn while_header_checks() {
    assert_eq!(errors("while (1 := 0; true) {};")[0].0, Code::IdentifierExpectedOnLeftSide);
    assert_eq!(errors("while (x += 0; true) {};")[0].0, Code::InvalidAdditiveAssignment);
}

#[test]
fn deep_nesting_does_not_overflow() {
    // Run on a thread with a small stack, as test threads and embedders may have.
    let h = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| {
            for open in ["(", "if c then ", "Set{", "a->select(x | ", "-", "do { "] {
                let src = format!("transformation T(); main() {{ x := {} 1; }}", open.repeat(100_000));
                let p = parse(&src);
                assert!(p.has_errors());
            }
        })
        .unwrap();
    h.join().unwrap();
}

#[test]
fn printer_round_trip_small() {
    let src = "transformation T(in a : MM);\nmain() { var x := -(-1); var y := - -1; if (a) { b := 1 } else c := 2 endif; }";
    let p = parse(src);
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    let printed = qvto_syntax::print::unit(&p.unit);
    let q = parse(&printed);
    assert!(!q.has_errors(), "{printed}\n{:?}", q.diagnostics);
    assert_eq!(dump::unit(&p.unit), dump::unit(&q.unit));
}
