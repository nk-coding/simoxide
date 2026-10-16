//! Units, modules and module elements (`QVTOParser.gi`).

use super::Parser;
use crate::ast::*;
use crate::diag::Code;
use crate::generated::{TokenKind as K, groups};
use crate::span::Span;

/// Where an element appears: at unit level or inside a `transformation { … }` body.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Unit,
    Module,
}

/// Tokens that start a unit or module element (recovery points).
const ELEMENT_START: &[K] = &[
    K::KwImport,
    K::KwTransformation,
    K::KwLibrary,
    K::KwModeltype,
    K::KwIntermediate,
    K::KwException,
    K::KwConfiguration,
    K::KwProperty,
    K::KwHelper,
    K::KwQuery,
    K::KwConstructor,
    K::KwMain,
    K::KwMapping,
    K::KwTag,
    K::KwRename,
    K::KwBlackbox,
    K::KwAbstract,
    K::KwStatic,
];

impl<'s> Parser<'s> {
    /// `topLevel ::= unit_elementList`
    pub(super) fn unit(&mut self, end: u32) -> Unit {
        let elements = self.elements(Level::Unit);
        self.validate_top_level(&elements);
        Unit {
            elements,
            span: Span::new(0, end),
        }
    }

    fn elements(&mut self, level: Level) -> Vec<Element> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                K::Eof => break,
                K::Rbrace if level == Level::Module => break,
                _ => {}
            }
            let before = self.pos;
            match self.element(level) {
                Some(e) => out.push(e),
                None => {
                    // `unit_element ::= qvtErrorToken`: skip to the next element.
                    let start = self.span();
                    if self.pos == before {
                        self.unexpected();
                        self.bump();
                    }
                    self.skip_to_element(level);
                    out.push(Element::Error(start.to(Span::at(self.prev_end()))));
                }
            }
        }
        out
    }

    fn skip_to_element(&mut self, level: Level) {
        loop {
            match self.peek() {
                K::Eof => return,
                K::Rbrace if level == Level::Module => return,
                k if ELEMENT_START.contains(&k) => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn element(&mut self, level: Level) -> Option<Element> {
        let start = self.span();
        let mut qualifiers = Vec::new();
        while self.at_any(groups::QUALIFIER) {
            qualifiers.push(self.bump().kind);
        }
        let k = self.peek();
        let el = match k {
            K::KwImport if level == Level::Unit && qualifiers.is_empty() => {
                Element::Import(self.import())
            }
            K::KwTransformation | K::KwLibrary if level == Level::Unit => {
                if k == K::KwLibrary && !qualifiers.is_empty() {
                    return None;
                }
                Element::Module(self.module(start, qualifiers))
            }
            K::KwModeltype if level == Level::Unit && qualifiers.is_empty() => {
                Element::ModelType(self.modeltype())
            }
            K::KwIntermediate if qualifiers.is_empty() && self.nth(1) == K::KwClass => {
                Element::Class(self.class_def())
            }
            K::KwException if qualifiers.is_empty() => Element::Class(self.class_def()),
            K::KwIntermediate | K::KwConfiguration | K::KwProperty if qualifiers.is_empty() => {
                Element::Property(self.property())
            }
            K::KwHelper | K::KwQuery => Element::Helper(self.helper(start, qualifiers)),
            K::KwConstructor => Element::Constructor(self.constructor(start, qualifiers)),
            K::KwMain if qualifiers.is_empty() => Element::Entry(self.entry()),
            K::KwMapping => Element::Mapping(self.mapping(start, qualifiers)),
            K::KwTag if qualifiers.is_empty() => {
                let t = self.tag();
                self.expect(K::Semicolon);
                Element::Tag(t)
            }
            K::KwRename if level == Level::Unit && qualifiers.is_empty() => {
                Element::Rename(self.rename())
            }
            _ => {
                if !qualifiers.is_empty() {
                    self.unexpected();
                }
                return None;
            }
        };
        Some(el)
    }

    /// `_import ::= import unit ';' | import library unit ';'`
    fn import(&mut self) -> Import {
        let start = self.bump().span;
        let library = self.eat(K::KwLibrary).is_some();
        let unit = self.qualified_name();
        self.expect(K::Semicolon);
        Import {
            library,
            unit,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `transformation_h`/`library_h` followed by `;` or `{ module_elementList } [;]`.
    fn module(&mut self, start: Span, qualifiers: Vec<K>) -> Module {
        let kind = self.bump().kind;
        let name = self.qualified_name();
        let mut params = None;
        if self.at(K::Lparen) {
            params = Some(self.simple_signature());
        } else if kind == K::KwTransformation && !matches!(self.peek(), K::Semicolon | K::Lbrace) {
            // `transformation_h ::= qualifierList transformation qualifiedNameCS
            // transformation_signature …`; only `transformation X;` may omit the signature.
            self.expected("(", false);
        }
        let mut usages = Vec::new();
        let mut refines = None;
        loop {
            match self.peek() {
                K::KwAccess | K::KwExtends => usages.push(self.module_usage()),
                K::KwRefines
                    if kind == K::KwTransformation && usages.is_empty() && refines.is_none() =>
                {
                    self.bump();
                    refines = Some(self.module_ref());
                }
                _ => break,
            }
        }
        let header_span = start.to(Span::at(self.prev_end()));
        let body = if self.at(K::Lbrace) {
            self.bump();
            let els = self.elements(Level::Module);
            self.expect(K::Rbrace);
            self.eat(K::Semicolon);
            Some(els)
        } else {
            self.expect(K::Semicolon);
            None
        };
        Module {
            kind,
            qualifiers,
            name,
            params,
            usages,
            refines,
            body,
            header_span,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `access_usage ::= access module_kindOpt moduleref_list` (and `extends`)
    fn module_usage(&mut self) -> ModuleUsage {
        let t = self.bump();
        let module_kind = if self.at_any(groups::MODULE_KIND) {
            Some(self.bump().kind)
        } else {
            None
        };
        let mut refs = vec![self.module_ref()];
        while self.eat(K::Comma).is_some() {
            refs.push(self.module_ref());
        }
        ModuleUsage {
            kind: t.kind,
            module_kind,
            refs,
            span: t.span.to(Span::at(self.prev_end())),
        }
    }

    /// `moduleref ::= pathNameCS simple_signatureOpt`
    fn module_ref(&mut self) -> ModuleRef {
        let start = self.span();
        let path = match self.path_name() {
            Some(p) => p,
            None => {
                self.expected("IDENTIFIER", false);
                PathName {
                    segments: Vec::new(),
                    span: Span::at(self.prev_end()),
                }
            }
        };
        let params = if self.at(K::Lparen) {
            Some(self.simple_signature())
        } else {
            None
        };
        ModuleRef {
            path,
            params,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `simple_signature ::= '(' param_listOpt ')'`
    fn simple_signature(&mut self) -> Vec<Param> {
        self.expect(K::Lparen);
        let mut params = Vec::new();
        if !self.at(K::Rparen) {
            params = self.param_list();
        }
        if self.expect(K::Rparen).is_none() {
            self.skip_to(&[K::Rparen, K::Semicolon, K::Lbrace]);
            self.eat(K::Rparen);
        }
        params
    }

    /// `param_list ::= param (',' param)*`
    fn param_list(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        loop {
            match self.param() {
                Some(p) => params.push(p),
                None => {
                    self.expected("param", false);
                    break;
                }
            }
            if self.eat(K::Comma).is_none() {
                break;
            }
        }
        params
    }

    /// `param ::= param_directionOpt IDENTIFIER ':' typespec | param_directionOpt typespec`
    fn param(&mut self) -> Option<Param> {
        let start = self.span();
        let direction = if self.at_any(groups::PARAM_DIRECTION) {
            Some(self.bump().kind)
        } else {
            None
        };
        let mut name = None;
        if self.at(K::Identifier) && self.nth(1) == K::Colon {
            name = self.ident();
            self.bump();
        }
        let ty = match self.type_spec() {
            Some(t) => t,
            None if direction.is_some() || name.is_some() => self.expect_type_spec(),
            None => return None,
        };
        Some(Param {
            direction,
            name,
            ty,
            span: start.to(Span::at(self.prev_end())),
        })
    }

    /// `complete_signature ::= simple_signature colon_param_listOpt`
    fn complete_signature(&mut self) -> (Vec<Param>, Vec<Param>) {
        let params = self.simple_signature();
        let results = if self.eat(K::Colon).is_some() {
            self.param_list()
        } else {
            Vec::new()
        };
        (params, results)
    }

    /// `_modeltype ::= modeltype IDENTIFIER compliance_kindOpt uses packageref_list
    /// modeltype_whereOpt ';'`
    fn modeltype(&mut self) -> ModelType {
        let start = self.bump().span;
        let name = self.expect_ident();
        let compliance = self.string_lit();
        self.expect(K::KwUses);
        let mut packages = vec![self.package_ref()];
        while self.eat(K::Comma).is_some() {
            packages.push(self.package_ref());
        }
        let where_ = if self.eat(K::KwWhere).is_some() {
            Some(self.block())
        } else {
            None
        };
        self.expect(K::Semicolon);
        ModelType {
            name,
            compliance,
            packages,
            where_,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `packageref ::= pathNameCS | pathNameCS '(' uri ')' | uri`
    fn package_ref(&mut self) -> PackageRef {
        let start = self.span();
        if let Some(uri) = self.string_lit() {
            return PackageRef {
                path: None,
                span: uri.span,
                uri: Some(uri),
            };
        }
        let path = self.path_name();
        if path.is_none() {
            self.expected("packageref", false);
        }
        let mut uri = None;
        if self.eat(K::Lparen).is_some() {
            uri = Some(self.expect_string_lit());
            self.expect(K::Rparen);
        }
        PackageRef {
            path,
            uri,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `classifierDefCS ::= intermediate class qvtIdentifierCS classifierExtensionOpt '{'
    /// classifierFeatureListOpt '}' semicolonOpt | exception …`
    fn class_def(&mut self) -> ClassDef {
        let start = self.span();
        let exception = self.bump().kind == K::KwException;
        if !exception {
            self.expect(K::KwClass);
        }
        let name = match self.simple_name() {
            Some(n) => n,
            None => {
                self.expected("IDENTIFIER", false);
                Name {
                    text: String::new(),
                    kind: NameKind::Ident,
                    span: Span::at(self.prev_end()),
                }
            }
        };
        let mut extends = Vec::new();
        if self.eat(K::KwExtends).is_some() {
            extends.push(self.expect_type());
            while self.eat(K::Comma).is_some() {
                extends.push(self.expect_type());
            }
        }
        let mut features = Vec::new();
        if self.expect(K::Lbrace).is_some() {
            // classifierFeatureList ::= classifierFeatureCS (';' classifierFeatureCS)* [';']
            while !self.at(K::Rbrace) && !self.at(K::Eof) {
                let before = self.pos;
                let start_pos = self.pos;
                match self.class_feature() {
                    Some(f) => features.push(f),
                    None => {
                        if self.pos == before {
                            self.unexpected();
                        }
                        self.skip_to(&[K::Semicolon, K::Rbrace]);
                    }
                }
                if self.eat(K::Semicolon).is_none() && !self.at(K::Rbrace) {
                    self.expected(";", true);
                    self.skip_to(&[K::Semicolon, K::Rbrace]);
                    self.eat(K::Semicolon);
                }
                if self.pos == start_pos {
                    // No progress (e.g. a stray closing bracket): drop the token.
                    self.bump();
                }
            }
            self.expect(K::Rbrace);
            self.eat(K::Semicolon);
        }
        ClassDef {
            exception,
            name,
            extends,
            features,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `classifierFeatureCS ::= stereotype_qualifier_list feature_key_list qvtIdentifierCS ':'
    /// typeCS multiplicityOpt ordered_prop opposite_propertyOpt init_partOpt | _tag`
    fn class_feature(&mut self) -> Option<ClassFeature> {
        if self.at(K::KwTag) {
            return Some(ClassFeature::Tag(self.tag()));
        }
        let start = self.span();
        let mut stereotypes = Vec::new();
        if self.eat(K::StereotypeQualifierOpen).is_some() {
            loop {
                match self.simple_name() {
                    Some(n) => stereotypes.push(n),
                    None => self.expected("IDENTIFIER", false),
                }
                if self.eat(K::Comma).is_none() {
                    break;
                }
            }
            self.expect(K::StereotypeQualifierClose);
        }
        let mut keys = Vec::new();
        while self.at_any(groups::FEATURE_KEY) {
            keys.push(self.bump().kind);
        }
        let name = self.simple_name()?;
        self.expect(K::Colon);
        let ty = self.expect_type();
        let multiplicity = if self.at(K::Lbracket) {
            Some(self.multiplicity())
        } else {
            None
        };
        let ordered = self.eat(K::KwOrdered).is_some();
        let opposite = if self.eat(K::KwOpposites).is_some() {
            let navigable = self.eat(K::TildeSign).is_some();
            let name = match self.simple_name() {
                Some(n) => n,
                None => {
                    self.expected("IDENTIFIER", false);
                    Name {
                        text: String::new(),
                        kind: NameKind::Ident,
                        span: Span::at(self.prev_end()),
                    }
                }
            };
            let multiplicity = if self.at(K::Lbracket) {
                Some(self.multiplicity())
            } else {
                None
            };
            Some(Opposite {
                navigable,
                name,
                multiplicity,
            })
        } else {
            None
        };
        let init = if self.eat(K::Equal).is_some() {
            Some(self.expr())
        } else {
            None
        };
        Some(ClassFeature::Property(ClassProperty {
            stereotypes,
            keys,
            name,
            ty,
            multiplicity,
            ordered,
            opposite,
            init,
            span: start.to(Span::at(self.prev_end())),
        }))
    }

    /// `multiplicityOpt ::= '[' multiplicity_range ']'`; ranges `n`, `*`, `n..m`, `n...m`, `n..*`
    fn multiplicity(&mut self) -> Multiplicity {
        let start = self.bump().span;
        let bound = |p: &mut Self, allow_int: bool| -> Option<MultBound> {
            if allow_int && p.at(K::IntegerLiteral) {
                let t = p.bump();
                Some(MultBound::Int(p.int_value(t)))
            } else if p.eat(K::Multiply).is_some() {
                Some(MultBound::Many)
            } else {
                p.expected("IntegerLiteralExpCS", false);
                None
            }
        };
        let lower = bound(self, true).unwrap_or(MultBound::Int(0));
        let mut upper = None;
        if matches!(lower, MultBound::Int(_))
            && (self.at(K::Dotdot) || self.at(K::MultiplicityRange))
        {
            let t = self.bump();
            let next = self.tok();
            if t.kind == K::Dotdot && next.kind == K::Dot && next.span.start == t.span.end {
                // `1...*` lexes as INTEGER DOTDOT DOT; LPG repairs it by merging.
                self.bump();
                let span = t.span.to(next.span);
                self.error(Code::Expected, span, "merge tokens \"...\" to form \"...\"");
            }
            upper = bound(self, true);
        }
        self.expect(K::Rbracket);
        Multiplicity {
            lower,
            upper,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `_property ::= configuration property qvtIdentifierCS ':' typeCS ';' | property … |
    /// intermediate property scoped_identifier ':' typeCS ['=' OclExpressionCS] ';'`
    fn property(&mut self) -> Property {
        let start = self.span();
        let kind = match self.bump().kind {
            K::KwConfiguration => PropertyKind::Configuration,
            K::KwIntermediate => PropertyKind::Intermediate,
            _ => PropertyKind::Plain,
        };
        if kind != PropertyKind::Plain {
            self.expect(K::KwProperty);
        }
        let name = if kind == PropertyKind::Intermediate {
            self.expect_scoped_name()
        } else {
            match self.simple_name() {
                Some(n) => ScopedName {
                    context_type: None,
                    span: n.span,
                    names: vec![n],
                },
                None => {
                    self.expected("IDENTIFIER", false);
                    ScopedName {
                        context_type: None,
                        names: Vec::new(),
                        span: Span::at(self.prev_end()),
                    }
                }
            }
        };
        let mut ty = None;
        if self.eat(K::Colon).is_some() {
            ty = Some(self.expect_type());
        } else if kind != PropertyKind::Plain {
            self.expected(":", false);
        }
        let mut init = None;
        if kind != PropertyKind::Configuration && self.eat(K::Equal).is_some() {
            init = Some(self.expr());
        } else if kind == PropertyKind::Plain && ty.is_none() {
            self.expected("=", false);
        }
        if self.expect(K::Semicolon).is_none() {
            self.skip_to(&[K::Semicolon]);
            self.eat(K::Semicolon);
        }
        Property {
            kind,
            name,
            ty,
            init,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `helper_header ::= qualifierList helper_kind scoped_identifier complete_signature`
    /// followed by `;`, `= OclExpressionCS ;` or `expression_block [;]`.
    fn helper(&mut self, start: Span, qualifiers: Vec<K>) -> Helper {
        let kind = self.bump().kind;
        let name = self.expect_scoped_name();
        let (params, results) = self.complete_signature();
        let decl_span = start.to(Span::at(self.prev_end()));
        if name.names.len() == 1
            && name.context_type.is_none()
            && name.names[0].kind == NameKind::Keyword(K::KwMain)
        {
            self.warning(
                Code::EntryAsHelper,
                decl_span,
                "It's disallowed to define entry operation as a query or helper.",
            );
        }
        let body = match self.peek() {
            K::Lbrace => {
                let b = self.block();
                self.eat(K::Semicolon);
                OpBody::Block(b)
            }
            K::Equal => {
                self.bump();
                let e = self.expr();
                self.expect(K::Semicolon);
                OpBody::Expr(e)
            }
            _ => {
                self.expect(K::Semicolon);
                OpBody::None
            }
        };
        Helper {
            qualifiers,
            kind,
            name,
            params,
            results,
            body,
            decl_span,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `constructor_header ::= qualifierList constructor scoped_identifier simple_signature`
    fn constructor(&mut self, start: Span, qualifiers: Vec<K>) -> Constructor {
        self.bump();
        let name = self.expect_scoped_name();
        let params = self.simple_signature();
        let decl_span = start.to(Span::at(self.prev_end()));
        let body = self.op_block_or_semi();
        Constructor {
            qualifiers,
            name,
            params,
            body,
            decl_span,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `entry_header ::= main simple_signature`
    fn entry(&mut self) -> Entry {
        let start = self.bump().span;
        let params = self.simple_signature();
        let decl_span = start.to(Span::at(self.prev_end()));
        let body = self.op_block_or_semi();
        Entry {
            params,
            body,
            decl_span,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    fn op_block_or_semi(&mut self) -> Option<Block> {
        if self.at(K::Lbrace) {
            let b = self.block();
            self.eat(K::Semicolon);
            Some(b)
        } else {
            self.expect(K::Semicolon);
            None
        }
    }

    /// `mapping_header ::= qualifierList mapping param_directionOpt scoped_identifier
    /// complete_signature mapping_extraList`, then `_whenOpt _whereOpt` and `;` or
    /// `'{' mapping_body '}' semicolonOpt`.
    fn mapping(&mut self, start: Span, qualifiers: Vec<K>) -> Mapping {
        self.bump();
        let direction = if self.at_any(groups::PARAM_DIRECTION) {
            Some(self.bump().kind)
        } else {
            None
        };
        let name = self.expect_scoped_name();
        let (params, results) = self.complete_signature();
        let mut extensions = Vec::new();
        while self.at_any(groups::MAPPING_EXTENSION_KEY) {
            let t = self.bump();
            let mut names = vec![self.expect_scoped_name()];
            while self.eat(K::Comma).is_some() {
                names.push(self.expect_scoped_name());
            }
            extensions.push(MappingExtension {
                kind: t.kind,
                names,
                span: t.span.to(Span::at(self.prev_end())),
            });
        }
        let decl_span = start.to(Span::at(self.prev_end()));
        if !matches!(
            self.peek(),
            K::KwWhen | K::KwWhere | K::Lbrace | K::Semicolon
        ) {
            // `scoped_identifier_list qvtErrorToken`: skip what does not belong to the header.
            self.unexpected();
            self.skip_to(&[K::KwWhen, K::KwWhere, K::Lbrace, K::Semicolon]);
        }
        let when = if self.eat(K::KwWhen).is_some() {
            Some(self.block())
        } else {
            None
        };
        let where_ = if self.eat(K::KwWhere).is_some() {
            Some(self.block())
        } else {
            None
        };
        let body = if self.at(K::Lbrace) {
            let b = self.mapping_body();
            self.eat(K::Semicolon);
            Some(b)
        } else {
            self.expect(K::Semicolon);
            None
        };
        Mapping {
            qualifiers,
            direction,
            name,
            params,
            results,
            extensions,
            when,
            where_,
            body,
            decl_span,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `mapping_body ::= init_sectionOpt population_sectionOpt end_sectionOpt`
    fn mapping_body(&mut self) -> MappingBody {
        let start = self.bump().span;
        let init = if self.eat(K::KwInit).is_some() {
            Some(self.block())
        } else {
            None
        };
        let population = if self.eat(K::KwPopulation).is_some() {
            Some(Population {
                explicit: true,
                body: self.block(),
            })
        } else if !self.at(K::KwEnd) && !self.at(K::Rbrace) {
            // population_section ::= expression_list
            let s = self.span();
            let stmts = self.stmt_list(&[K::KwEnd]);
            Some(Population {
                explicit: false,
                body: Block {
                    stmts,
                    span: s.to(Span::at(self.prev_end())),
                },
            })
        } else {
            None
        };
        let end = if self.eat(K::KwEnd).is_some() {
            Some(self.block())
        } else {
            None
        };
        if self.expect(K::Rbrace).is_none() {
            self.skip_to(&[K::Rbrace]);
            self.eat(K::Rbrace);
        }
        MappingBody {
            init,
            population,
            end,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `_tag ::= tag StringLiteralExpCS scoped_identifier tag_valueOpt`
    fn tag(&mut self) -> Tag {
        let start = self.bump().span;
        let id = self.expect_string_lit();
        let element = self.expect_scoped_name();
        let value = if self.eat(K::Equal).is_some() {
            Some(self.expr())
        } else {
            None
        };
        Tag {
            id,
            element,
            value,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `renaming ::= rename typeCS '.' qvtIdentifierCS '=' StringLiteralExpCS ';'`
    fn rename(&mut self) -> Rename {
        let start = self.bump().span;
        let ty = self.expect_type();
        self.expect(K::Dot);
        let property = match self.simple_name() {
            Some(n) => n,
            None => {
                self.expected("IDENTIFIER", false);
                Name {
                    text: String::new(),
                    kind: NameKind::Ident,
                    span: Span::at(self.prev_end()),
                }
            }
        };
        self.expect(K::Equal);
        let new_name = self.expect_string_lit();
        self.expect(K::Semicolon);
        Rename {
            ty,
            property,
            new_name,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `AbstractQVTParser.validateTopLevelElementOrder` and the module checks of
    /// `setupTopLevel`.
    fn validate_top_level(&mut self, elements: &[Element]) {
        let mut other_than_import = false;
        let mut module_declared = false;
        let mut modules = 0;
        let mut imports_and_modeltypes = 0;
        let mut first: Option<Span> = None;
        let mut last: Option<Span> = None;
        for e in elements {
            if matches!(e, Element::Error(_)) {
                continue;
            }
            if first.is_none() {
                first = Some(e.span());
            }
            last = Some(e.span());
            match e {
                Element::Module(_) => {
                    module_declared = true;
                    modules += 1;
                    continue;
                }
                Element::Import(i) => {
                    imports_and_modeltypes += 1;
                    if other_than_import {
                        self.warning(
                            Code::ImportNotFirst,
                            i.span,
                            "Import statement must be the first element in compilation unit. Using this location is deprecated.",
                        );
                    }
                }
                _ => other_than_import = true,
            }
            if let Element::ModelType(_) = e {
                imports_and_modeltypes += 1;
            }
            if !module_declared {
                let span = match e {
                    Element::Property(p) => Some(p.name.names.last().map_or(p.span, |n| n.span)),
                    Element::Helper(h) => Some(h.decl_span),
                    Element::Constructor(c) => Some(c.decl_span),
                    Element::Entry(x) => Some(x.decl_span),
                    Element::Mapping(m) => Some(m.decl_span),
                    Element::Class(c) => Some(c.name.span),
                    _ => None,
                };
                if let Some(span) = span {
                    self.warning(
                        Code::ModuleDeclarationMustPrecede,
                        span,
                        "Module declaration must precede this element",
                    );
                }
            }
        }
        let counted = elements
            .iter()
            .filter(|e| !matches!(e, Element::Error(_)))
            .count();
        if modules == 0 {
            if let Some(f) = first {
                self.error(
                    Code::NoModule,
                    f.to(last.unwrap()),
                    "No module declared in the compilation unit",
                );
            }
        } else if modules > 1 && counted != modules + imports_and_modeltypes {
            self.error(
                Code::MultipleModulesExtraElements,
                first.unwrap().to(last.unwrap()),
                "Unit with multiple modules may contain only imports and modeltypes besides the modules",
            );
        }
    }
}
