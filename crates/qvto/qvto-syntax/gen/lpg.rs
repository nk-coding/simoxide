//! Generator for `src/generated.rs`: reads Eclipse QVTo's LPG grammar files (`.gi`) and derives
//! the token kinds, the keyword table, the punctuation table, the ASCII character classes, the
//! keyword groups the parser uses and the binary operator precedence levels.
//!
//! Used by `examples/gen_tables.rs` (writes the file) and `tests/generated_tables.rs` (checks that
//! the committed file is up to date). Nothing here is compiled into the library.
//!
//! The grammars are read, not hand-copied: keywords come from the keyword lexers
//! (`*KWLexer.gi`, `%Export` plus the spelling rules), tokens and their spellings from the lexers
//! (`*Lexer.gi`, rules with `makeToken`), character classes from `lpg/LexerBasicMapF.gi`, keyword
//! groups and operator levels from the parser grammars (`EssentialOCL.gi`, `ImperativeOCL.gi`,
//! `QVTOParser.gi`) with their `%Import`/`%DropRules` applied. Every derived table is
//! cross-checked against a second place in the grammars where one exists.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A token of the `.gi` syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    Word(String),
    Quoted(String),
    Action(String),
    Def,
    Arrow,
    Bar,
}

#[derive(Clone, Debug)]
pub struct Section {
    pub kind: String,
    pub toks: Vec<Tok>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alt {
    pub lhs: String,
    pub syms: Vec<Sym>,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sym {
    Name(String),
    Quoted(String),
}

impl Sym {
    fn text(&self) -> &str {
        match self {
            Sym::Name(s) | Sym::Quoted(s) => s,
        }
    }
}

fn tokenize(text: &str) -> Vec<(Option<String>, Tok)> {
    // Returns (directive, tok): a directive marks `%Name` section starts (tok is then unused).
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"--") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/.") {
            let start = i + 2;
            let mut j = start;
            while j + 1 < b.len() && !(b[j] == b'.' && b[j + 1] == b'/') {
                j += 1;
            }
            out.push((None, Tok::Action(text[start..j.min(b.len())].to_string())));
            i = (j + 2).min(b.len());
            continue;
        }
        if c == b'\'' || c == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && b[j] != c {
                j += 1;
            }
            out.push((None, Tok::Quoted(text[start..j.min(b.len())].to_string())));
            i = j + 1;
            continue;
        }
        if b[i..].starts_with(b"::=") {
            out.push((None, Tok::Def));
            i += 3;
            continue;
        }
        if b[i..].starts_with(b"->") {
            out.push((None, Tok::Arrow));
            i += 2;
            continue;
        }
        if c == b'|' {
            out.push((None, Tok::Bar));
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'|' {
            i += 1;
        }
        let w = &text[start..i];
        if w.starts_with("%options") || w.starts_with("%Options") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if let Some(name) = w.strip_prefix('%') {
            if name != "empty" {
                out.push((Some(name.to_string()), Tok::Bar));
                continue;
            }
        }
        out.push((None, Tok::Word(w.to_string())));
    }
    out
}

pub fn parse_sections(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut cur: Option<Section> = None;
    for (dir, tok) in tokenize(text) {
        match dir {
            Some(d) if d == "End" => {
                if let Some(s) = cur.take() {
                    sections.push(s);
                }
            }
            Some(d) => {
                if let Some(s) = cur.take() {
                    sections.push(s);
                }
                cur = Some(Section { kind: d, toks: Vec::new() });
            }
            None => {
                if let Some(s) = cur.as_mut() {
                    s.toks.push(tok);
                }
            }
        }
    }
    if let Some(s) = cur.take() {
        sections.push(s);
    }
    sections
}

pub fn parse_rules(toks: &[Tok]) -> Vec<Alt> {
    let mut alts = Vec::new();
    let mut lhs: Option<String> = None;
    let mut syms: Vec<Sym> = Vec::new();
    let mut action = String::new();
    let mut started = false;
    let mut i = 0;
    let flush = |lhs: &Option<String>, syms: &mut Vec<Sym>, action: &mut String, started: &mut bool, alts: &mut Vec<Alt>| {
        if let (Some(l), true) = (lhs, *started) {
            alts.push(Alt { lhs: l.clone(), syms: std::mem::take(syms), action: std::mem::take(action) });
        }
        syms.clear();
        action.clear();
        *started = false;
    };
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) if matches!(toks.get(i + 1), Some(Tok::Def) | Some(Tok::Arrow)) => {
                flush(&lhs, &mut syms, &mut action, &mut started, &mut alts);
                lhs = Some(w.clone());
                started = true;
                i += 2;
                continue;
            }
            Tok::Bar => {
                flush(&lhs, &mut syms, &mut action, &mut started, &mut alts);
                started = true;
            }
            Tok::Word(w) => {
                if w != "%empty" {
                    syms.push(Sym::Name(w.clone()));
                }
            }
            Tok::Quoted(q) => syms.push(Sym::Quoted(q.clone())),
            Tok::Action(a) => action.push_str(a),
            Tok::Def | Tok::Arrow => {}
        }
        i += 1;
    }
    flush(&lhs, &mut syms, &mut action, &mut started, &mut alts);
    alts
}

/// A grammar with its imports applied.
#[derive(Default, Debug)]
pub struct Grammar {
    pub files: Vec<PathBuf>,
    pub rules: Vec<Alt>,
    pub exports: Vec<String>,
    pub keywords: Vec<String>,
    /// `%Terminals` aliases: name -> spelling.
    pub aliases: Vec<(String, String)>,
}

fn find_import(dir: &Path, name: &str) -> PathBuf {
    for d in [dir.to_path_buf(), dir.join("../lpg"), dir.join("lpg")] {
        let p = d.join(name);
        if p.exists() {
            return p;
        }
    }
    panic!("import {name} not found from {}", dir.display());
}

fn read(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn load(path: &Path) -> Grammar {
    let text = read(path);
    let sections = parse_sections(&text);
    let mut g = Grammar::default();
    let mut drops: Vec<Alt> = Vec::new();
    for s in &sections {
        match s.kind.as_str() {
            "Import" => {
                for t in &s.toks {
                    if let Tok::Word(w) = t {
                        let imp = load(&find_import(path.parent().unwrap(), w));
                        g.files.extend(imp.files);
                        g.rules.extend(imp.rules);
                        g.exports.extend(imp.exports);
                        g.keywords.extend(imp.keywords);
                        g.aliases.extend(imp.aliases);
                    }
                }
            }
            "DropRules" => drops.extend(parse_rules(&s.toks)),
            _ => {}
        }
    }
    g.rules.retain(|a| !drops.iter().any(|d| d.lhs == a.lhs && d.syms == a.syms));
    for s in &sections {
        match s.kind.as_str() {
            "Rules" => g.rules.extend(parse_rules(&s.toks)),
            "Export" => g.exports.extend(words(&s.toks)),
            "KeyWords" => g.keywords.extend(words(&s.toks)),
            "Terminals" => {
                let mut i = 0;
                while i < s.toks.len() {
                    if let (Tok::Word(n), Some(Tok::Def), Some(Tok::Quoted(q))) =
                        (&s.toks[i], s.toks.get(i + 1), s.toks.get(i + 2))
                    {
                        g.aliases.push((n.clone(), q.clone()));
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
            }
            _ => {}
        }
    }
    g.files.push(path.to_path_buf());
    g
}

fn words(toks: &[Tok]) -> Vec<String> {
    toks.iter()
        .filter_map(|t| if let Tok::Word(w) = t { Some(w.clone()) } else { None })
        .collect()
}

fn extract_dollar_name(action: &str, call: &str) -> Option<String> {
    let i = action.find(call)?;
    let rest = &action[i + call.len()..];
    let rest = rest.trim_start().strip_prefix("($_")?;
    let end = rest.find(')')?;
    Some(rest[..end].to_string())
}

/// Everything the generated file contains.
#[derive(Debug, Default)]
pub struct Tables {
    /// Lexer terminals other than keywords, in export order (LPG names).
    pub terminals: Vec<String>,
    /// Punctuation: LPG name -> spelling.
    pub punct: BTreeMap<String, String>,
    /// Keywords in export order.
    pub keywords: Vec<String>,
    /// ASCII class flags.
    pub ascii: Vec<u8>,
    pub non_ascii_letter: bool,
    pub eof_chars: Vec<u32>,
    /// Groups: (const name, grammar nonterminal, members as LPG terminal names or keywords).
    pub groups: Vec<(String, String, Vec<String>)>,
    /// Binary operator levels, loosest first; members as LPG terminal names or keywords.
    pub binary_levels: Vec<(String, Vec<String>)>,
    pub prefix_ops: Vec<String>,
    pub sources: Vec<String>,
}

pub const IDENT_START: u8 = 1;
pub const IDENT_PART: u8 = 2;
pub const DIGIT: u8 = 4;
pub const WHITESPACE: u8 = 8;
pub const CONTROL: u8 = 16;

/// Grammar nonterminals whose alternatives are single tokens and that the parser uses as
/// token sets. The parser refers to these constants instead of listing keywords itself.
pub const GROUPS: &[(&str, &str)] = &[
    ("PRIMITIVE_TYPE", "primitiveTypeCS"),
    ("COLLECTION_TYPE", "CollectionTypeIdentifierCS"),
    ("RESTRICTED_KEYWORD", "restrictedKeywordCS"),
    ("RESERVED_KEYWORD", "reservedKeyword"),
    ("CONCEPTUAL_OPERATION", "conceptualOperationName"),
    ("SIMPLE_NAME", "simpleNameCS"),
    ("BOOLEAN_LITERAL", "BooleanLiteralExpCS"),
    ("FEATURE_KEY", "feature_key"),
    ("QUALIFIER", "qualifier"),
    ("PARAM_DIRECTION", "param_direction"),
    ("HELPER_KIND", "helper_kind"),
    ("MODULE_KIND", "module_kind"),
    ("MAPPING_EXTENSION_KEY", "mapping_extension_key"),
    ("RESOLVE_OP", "resolveOp"),
    ("RESOLVE_IN_OP", "resolveInOp"),
    ("IMPERATIVE_ITERATOR_12", "imperativeIteratorExpCSToken12"),
    ("IMPERATIVE_ITERATOR_3", "imperativeIteratorExpCSToken3"),
    ("FOR_OP", "forOpCode"),
    ("VAR_INIT_OP", "var_init_op"),
    ("OBJECT_IDENTIFIER", "objectIdentifierCS"),
];

pub struct Paths {
    pub cst: PathBuf,
}

impl Paths {
    pub fn new(eclipse_qvto: &Path) -> Paths {
        Paths { cst: eclipse_qvto.join("plugins/org.eclipse.m2m.qvt.oml.cst.parser/cst") }
    }
}

pub fn derive(paths: &Paths) -> Result<Tables, String> {
    let cst = &paths.cst;
    let lexer = load(&cst.join("QVTOLexer.gi"));
    let kw = load(&cst.join("QVTOKWLexer.gi"));
    let parser = load(&cst.join("QVTOParser.gi"));
    let mut t = Tables::default();
    let mut errors = Vec::new();
    let rel = |p: &PathBuf| -> String {
        let mut parts: Vec<String> = Vec::new();
        for c in p.components() {
            match c {
                std::path::Component::ParentDir => {
                    parts.pop();
                }
                std::path::Component::CurDir => {}
                other => parts.push(other.as_os_str().to_string_lossy().into_owned()),
            }
        }
        let s = parts.join("/");
        match s.find("plugins/") {
            Some(i) => s[i..].to_string(),
            None => s,
        }
    };
    let mut srcs: BTreeSet<String> = BTreeSet::new();
    for f in lexer.files.iter().chain(&kw.files).chain(&parser.files) {
        srcs.insert(rel(f));
    }
    let basic = cst.join("../lpg/LexerBasicMapF.gi");
    srcs.insert(rel(&basic));
    t.sources = srcs.into_iter().collect();

    // Keywords: KW lexer exports, checked against the spelling rules and the parser's %KeyWords.
    t.keywords = dedup(&kw.exports);
    for a in &kw.rules {
        if let Some(name) = extract_dollar_name(&a.action, "$setResult") {
            let spelled: String = a.syms.iter().map(|s| s.text()).collect();
            if spelled != name {
                errors.push(format!("keyword rule spells {spelled:?} but sets {name}"));
            }
            if !t.keywords.contains(&name) {
                errors.push(format!("keyword {name} has a rule but is not exported"));
            }
        }
    }
    for k in &t.keywords {
        let has_rule = kw.rules.iter().any(|a| extract_dollar_name(&a.action, "$setResult").as_deref() == Some(k));
        if !has_rule {
            errors.push(format!("keyword {k} exported without a spelling rule"));
        }
    }
    let parser_kws: BTreeSet<_> = parser.keywords.iter().cloned().collect();
    let lexer_kws: BTreeSet<_> = t.keywords.iter().cloned().collect();
    if parser_kws != lexer_kws {
        errors.push(format!(
            "parser %KeyWords and keyword lexer exports differ: only parser {:?}, only lexer {:?}",
            parser_kws.difference(&lexer_kws).collect::<Vec<_>>(),
            lexer_kws.difference(&parser_kws).collect::<Vec<_>>()
        ));
    }

    // Terminals and punctuation from the lexer.
    t.terminals = dedup(&lexer.exports);
    for a in &lexer.rules {
        let Some(name) = extract_dollar_name(&a.action, "makeToken") else { continue };
        if a.syms.is_empty() || !a.syms.iter().all(|s| matches!(s, Sym::Quoted(q) if q.chars().count() == 1)) {
            continue;
        }
        let spelling: String = a.syms.iter().map(|s| s.text()).collect();
        if !t.terminals.contains(&name) {
            errors.push(format!("punctuation {name} is not exported by the lexer"));
        }
        if let Some(old) = t.punct.insert(name.clone(), spelling.clone()) {
            if old != spelling {
                errors.push(format!("punctuation {name} spelled {old:?} and {spelling:?}"));
            }
        }
    }
    for (name, spelling) in &parser.aliases {
        match t.punct.get(name) {
            Some(s) if s == spelling => {}
            Some(s) => errors.push(format!("parser alias {name} ::= {spelling:?}, lexer spells {s:?}")),
            None => errors.push(format!("parser alias {name} ::= {spelling:?} has no lexer rule")),
        }
    }

    // Character classes.
    let basic_text = read(&basic);
    let mut table: Vec<String> = Vec::new();
    for line in basic_text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("$sym_type.$prefix$") {
            if let Some(end) = rest.find("$suffix$") {
                if l.contains("// ") && table.len() < 128 {
                    let comment = &l[l.find("//").unwrap() + 2..].trim();
                    if comment.starts_with(|c: char| c.is_ascii_digit()) {
                        table.push(rest[..end].to_string());
                    }
                }
            }
        }
    }
    if table.len() != 128 {
        errors.push(format!("LexerBasicMapF.gi: found {} ASCII classes, expected 128", table.len()));
    }
    let closure = |start: &str| -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        let mut todo = vec![start.to_string()];
        while let Some(n) = todo.pop() {
            let alts: Vec<&Alt> = lexer.rules.iter().filter(|a| a.lhs == n).collect();
            if alts.is_empty() {
                set.insert(n);
                continue;
            }
            for a in alts {
                if a.syms.len() == 1 {
                    let s = a.syms[0].text().to_string();
                    if !set.contains(&s) {
                        todo.push(s);
                    }
                }
            }
        }
        set
    };
    let letters = closure("Letter");
    let digits = closure("Digit");
    let ws = closure("WSChar");
    // Identifier -> Letter | Identifier Letter | Identifier Digit | Identifier DollarSign
    let mut ident_start = BTreeSet::new();
    let mut ident_part = BTreeSet::new();
    for a in lexer.rules.iter().filter(|a| a.lhs == "Identifier") {
        match a.syms.as_slice() {
            [s] => ident_start.extend(closure(s.text())),
            [Sym::Name(first), s] if first == "Identifier" => ident_part.extend(closure(s.text())),
            _ => errors.push(format!("unexpected Identifier rule {:?}", a.syms)),
        }
    }
    let alias_name_of = |c: char| -> Vec<String> {
        let mut v: Vec<String> = lexer.aliases.iter().filter(|(_, q)| q.chars().eq(std::iter::once(c))).map(|(n, _)| n.clone()).collect();
        v.push(c.to_string());
        v
    };
    for (i, name) in table.iter().enumerate() {
        let names = {
            let mut v = alias_name_of(char::from(i as u8));
            v.push(name.clone());
            v
        };
        let has = |set: &BTreeSet<String>| names.iter().any(|n| set.contains(n));
        let mut f = 0u8;
        if has(&ident_start) {
            f |= IDENT_START;
        }
        if has(&ident_part) || has(&ident_start) {
            f |= IDENT_PART;
        }
        if has(&digits) {
            f |= DIGIT;
        }
        if has(&ws) {
            f |= WHITESPACE;
        }
        if name == "CtlCharNotWS" {
            f |= CONTROL;
        }
        t.ascii.push(f);
    }
    t.non_ascii_letter = letters.contains("AfterASCIINotAcute");
    // getKind(): chars compared with '\uXXXX' that map to EOF.
    if let Some(i) = basic_text.find("getKind(int i)") {
        let body = &basic_text[i..];
        let body = &body[..body.find("orderedExportedSymbols").unwrap_or(body.len())];
        let mut rest = body;
        while let Some(j) = rest.find("c == '\\u") {
            let hex = &rest[j + 8..j + 12];
            if let Ok(v) = u32::from_str_radix(hex, 16) {
                if !t.eof_chars.contains(&v) {
                    t.eof_chars.push(v);
                }
            }
            rest = &rest[j + 12..];
        }
    }
    if t.eof_chars.is_empty() {
        errors.push("LexerBasicMapF.gi: no EOF characters found in getKind".into());
    }

    // Keyword groups.
    let spelled_to_name: BTreeMap<String, String> = t.punct.iter().map(|(n, s)| (s.clone(), n.clone())).collect();
    let term_name = |s: &Sym| -> Option<String> {
        match s {
            Sym::Quoted(q) => spelled_to_name.get(q).cloned(),
            Sym::Name(n) => {
                if t.keywords.contains(n) || t.terminals.contains(n) {
                    Some(n.clone())
                } else {
                    None
                }
            }
        }
    };
    for (cname, nt) in GROUPS {
        let mut members = Vec::new();
        let mut todo = vec![nt.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(n) = todo.pop() {
            if !seen.insert(n.clone()) {
                continue;
            }
            let alts: Vec<&Alt> = parser.rules.iter().filter(|a| a.lhs == n).collect();
            if alts.is_empty() {
                errors.push(format!("group {cname}: no rules for {n}"));
            }
            for a in alts {
                if a.syms.len() != 1 {
                    continue;
                }
                match term_name(&a.syms[0]) {
                    Some(m) => {
                        if !members.contains(&m) {
                            members.push(m)
                        }
                    }
                    None => todo.push(a.syms[0].text().to_string()),
                }
            }
        }
        t.groups.push((cname.to_string(), nt.to_string(), members));
    }

    // Binary operator levels: follow notNameExpressionCS -> impliesNotNameNotLetCS -> ... chain.
    let mut level = "impliesNotNameNotLetCS".to_string();
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 50 {
            errors.push("operator level chain does not terminate".into());
            break;
        }
        let alts: Vec<&Alt> = parser.rules.iter().filter(|a| a.lhs == level).collect();
        let mut ops = Vec::new();
        let mut next = None;
        for a in &alts {
            match a.syms.as_slice() {
                [_, op, _] => match term_name(op) {
                    Some(m) => ops.push(m),
                    None => errors.push(format!("{level}: operator {op:?} is not a terminal")),
                },
                [Sym::Name(n)] => next = Some(n.clone()),
                [op, Sym::Name(_)] if level.starts_with("unary") => match term_name(op) {
                    Some(m) => t.prefix_ops.push(m),
                    None => errors.push(format!("{level}: prefix {op:?} is not a terminal")),
                },
                other => errors.push(format!("{level}: unexpected rule {other:?}")),
            }
        }
        if level.starts_with("unary") {
            break;
        }
        let short = level.trim_end_matches("NotNameNotLetCS").to_string();
        t.binary_levels.push((short, ops));
        match next {
            Some(n) => level = n,
            None => {
                errors.push(format!("{level}: no next level"));
                break;
            }
        }
    }
    if errors.is_empty() { Ok(t) } else { Err(errors.join("\n")) }
}

fn dedup(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

/// LPG terminal name to Rust variant name.
pub fn variant(name: &str, keywords: &[String]) -> String {
    if keywords.iter().any(|k| k == name) {
        let mut c = name.chars();
        let first = c.next().unwrap().to_ascii_uppercase();
        return format!("Kw{first}{}", c.as_str());
    }
    name.split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let lower = p.to_ascii_lowercase();
            let mut c = lower.chars();
            let first = c.next().unwrap().to_ascii_uppercase();
            format!("{first}{}", c.as_str())
        })
        .collect()
}

fn rust_str(s: &str) -> String {
    format!("{s:?}")
}

pub fn render(t: &Tables) -> String {
    let mut o = String::new();
    let kws = &t.keywords;
    let v = |n: &str| variant(n, kws);
    o.push_str("// @generated by `cargo run -p qvto-syntax --example gen_tables` from Eclipse QVTo's LPG\n");
    o.push_str("// grammars. Do not edit; regenerate instead (see the crate README). Sources:\n");
    for s in &t.sources {
        let _ = writeln!(o, "//   {s}");
    }
    o.push_str("\n#![allow(clippy::all)]\n\n");

    // TokenKind
    o.push_str("/// Token kinds: the terminals exported by Eclipse's lexers and keyword lexers.\n");
    o.push_str("#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]\n#[repr(u8)]\npub enum TokenKind {\n");
    o.push_str("    /// End of input (LPG `EOF_TOKEN`).\n    Eof,\n");
    o.push_str("    /// A character or sequence the lexer rejects (LPG `ERROR_TOKEN`).\n    ErrorToken,\n");
    for n in &t.terminals {
        match t.punct.get(n) {
            Some(s) => {
                let _ = writeln!(o, "    /// `{s}` (`{n}`)");
            }
            None => {
                let _ = writeln!(o, "    /// `{n}`");
            }
        }
        let _ = writeln!(o, "    {},", v(n));
    }
    for k in kws {
        let _ = writeln!(o, "    /// keyword `{k}`");
        let _ = writeln!(o, "    {},", v(k));
    }
    o.push_str("}\n\n");
    let first_kw = v(&kws[0]);
    let _ = writeln!(o, "/// Number of keywords.\npub const KEYWORD_COUNT: usize = {};\n", kws.len());
    o.push_str("impl TokenKind {\n");
    o.push_str("    /// All token kinds in declaration order.\n    pub const ALL: &'static [TokenKind] = &[\n        TokenKind::Eof,\n        TokenKind::ErrorToken,\n");
    for n in t.terminals.iter().chain(kws) {
        let _ = writeln!(o, "        TokenKind::{},", v(n));
    }
    o.push_str("    ];\n\n");
    let _ = writeln!(o, "    /// Whether this is a keyword (all keywords are reserved by the lexer).\n    #[inline]\n    pub const fn is_keyword(self) -> bool {{\n        self as u8 >= TokenKind::{first_kw} as u8\n    }}\n");
    o.push_str("    /// The LPG terminal name (`IDENTIFIER`, `ARROW`, `and`, ...).\n    pub const fn lpg_name(self) -> &'static str {\n        match self {\n            TokenKind::Eof => \"EOF_TOKEN\",\n            TokenKind::ErrorToken => \"ERROR_TOKEN\",\n");
    for n in t.terminals.iter().chain(kws) {
        let _ = writeln!(o, "            TokenKind::{} => {},", v(n), rust_str(n));
    }
    o.push_str("        }\n    }\n\n");
    o.push_str("    /// The fixed spelling of a keyword or punctuation token.\n    pub const fn spelling(self) -> Option<&'static str> {\n        match self {\n");
    for n in &t.terminals {
        if let Some(s) = t.punct.get(n) {
            let _ = writeln!(o, "            TokenKind::{} => Some({}),", v(n), rust_str(s));
        }
    }
    for k in kws {
        let _ = writeln!(o, "            TokenKind::{} => Some({}),", v(k), rust_str(k));
    }
    o.push_str("            _ => None,\n        }\n    }\n}\n\n");

    // keyword()
    o.push_str("/// The keyword spelled by `s`, if any (keyword lexer filter, case-sensitive).\n#[inline]\npub fn keyword(s: &[u8]) -> Option<TokenKind> {\n    match s.len() {\n");
    let mut by_len: BTreeMap<usize, Vec<&String>> = BTreeMap::new();
    for k in kws {
        by_len.entry(k.len()).or_default().push(k);
    }
    for (len, ks) in &by_len {
        let _ = writeln!(o, "        {len} => match s {{");
        for k in ks {
            let _ = writeln!(o, "            b{} => Some(TokenKind::{}),", rust_str(k), v(k));
        }
        o.push_str("            _ => None,\n        },\n");
    }
    o.push_str("        _ => None,\n    }\n}\n\n");

    // punctuation()
    o.push_str("/// The longest punctuation token at the start of `s` and its length.\n#[inline]\npub fn punctuation(s: &[u8]) -> Option<(TokenKind, usize)> {\n    let first = *s.first()?;\n    match first {\n");
    let mut by_first: BTreeMap<u8, Vec<(&String, &String)>> = BTreeMap::new();
    for (n, sp) in &t.punct {
        by_first.entry(sp.as_bytes()[0]).or_default().push((n, sp));
    }
    for (first, mut items) in by_first {
        items.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.1.cmp(b.1)));
        let _ = writeln!(o, "        {} => {{", byte_lit(first));
        for (n, sp) in &items {
            if sp.len() == 1 {
                let _ = writeln!(o, "            Some((TokenKind::{}, 1))", v(n));
            } else {
                let _ = writeln!(o, "            if s.starts_with(b{}) {{\n                return Some((TokenKind::{}, {}));\n            }}", rust_str(sp), v(n), sp.len());
            }
        }
        if items.iter().all(|(_, sp)| sp.len() > 1) {
            o.push_str("            None\n");
        }
        o.push_str("        }\n");
    }
    o.push_str("        _ => None,\n    }\n}\n\n");

    // char classes
    o.push_str("/// Character class flags (`LexerBasicMapF.gi` table and the lexer's `Letter`, `Digit`,\n/// `WSChar` and `Identifier` rules).\npub mod class {\n");
    let _ = writeln!(o, "    pub const IDENT_START: u8 = {IDENT_START};\n    pub const IDENT_PART: u8 = {IDENT_PART};\n    pub const DIGIT: u8 = {DIGIT};\n    pub const WHITESPACE: u8 = {WHITESPACE};\n    /// `CtlCharNotWS`: a control character that is not white space.\n    pub const CONTROL: u8 = {CONTROL};");
    o.push_str("    /// Flags of the ASCII characters.\n    pub const ASCII: [u8; 128] = [\n");
    for row in t.ascii.chunks(16) {
        o.push_str("        ");
        o.push_str(&row.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", "));
        o.push_str(",\n");
    }
    o.push_str("    ];\n");
    let _ = writeln!(o, "    /// Whether characters above ASCII (other than the EOF characters) are letters.\n    pub const NON_ASCII_IS_LETTER: bool = {};", t.non_ascii_letter);
    let chars: Vec<String> = t.eof_chars.iter().map(|c| format!("'\\u{{{c:x}}}'")).collect();
    let _ = writeln!(o, "    /// Characters the lexer classifies as end of input (`getKind`).\n    pub const EOF_CHARS: &[char] = &[{}];", chars.join(", "));
    o.push_str("}\n\n");

    // groups
    o.push_str("/// Token sets taken from single-token alternatives of grammar nonterminals.\npub mod groups {\n    use super::TokenKind;\n");
    for (cname, nt, members) in &t.groups {
        let _ = writeln!(o, "    /// `{nt}`");
        let list: Vec<String> = members.iter().map(|m| format!("TokenKind::{}", v(m))).collect();
        let _ = writeln!(o, "    pub const {cname}: &[TokenKind] = &[{}];", list.join(", "));
    }
    o.push_str("}\n\n");

    // operator levels
    o.push_str("/// Binary operator levels from `EssentialOCL.gi` (with QVTo's additions), loosest first.\n/// All are left-associative.\npub const BINARY_LEVELS: &[(&str, &[TokenKind])] = &[\n");
    for (name, ops) in &t.binary_levels {
        let list: Vec<String> = ops.iter().map(|m| format!("TokenKind::{}", v(m))).collect();
        let _ = writeln!(o, "    ({}, &[{}]),", rust_str(name), list.join(", "));
    }
    o.push_str("];\n\n");
    o.push_str("/// The binding level of a binary operator: index into [`BINARY_LEVELS`] plus one.\n#[inline]\npub const fn binary_level(k: TokenKind) -> u8 {\n    match k {\n");
    for (i, (_, ops)) in t.binary_levels.iter().enumerate() {
        let list: Vec<String> = ops.iter().map(|m| format!("TokenKind::{}", v(m))).collect();
        let _ = writeln!(o, "        {} => {},", list.join(" | "), i + 1);
    }
    o.push_str("        _ => 0,\n    }\n}\n\n");
    let list: Vec<String> = t.prefix_ops.iter().map(|m| format!("TokenKind::{}", v(m))).collect();
    let _ = writeln!(o, "/// Prefix operators (`unaryNotNameNotLetCS`).\npub const PREFIX_OPS: &[TokenKind] = &[{}];", list.join(", "));
    o
}

fn byte_lit(b: u8) -> String {
    match b {
        b'\'' => "b'\\''".to_string(),
        b'\\' => "b'\\\\'".to_string(),
        _ => format!("b'{}'", b as char),
    }
}

/// Locates the Eclipse QVTo checkout: `$QVTO_ECLIPSE_DIR`, or `reference/eclipse-qvto` above the
/// crate.
pub fn eclipse_dir(manifest_dir: &Path) -> Option<PathBuf> {
    if let Ok(d) = std::env::var("QVTO_ECLIPSE_DIR") {
        return Some(PathBuf::from(d));
    }
    let p = manifest_dir.join("../../reference/eclipse-qvto");
    p.join("plugins/org.eclipse.m2m.qvt.oml.cst.parser/cst").exists().then_some(p)
}
