//! Diagnostics collected while loading, resolving and validating.

use std::fmt;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Level {
    /// Informational (e.g. a tolerant fallback that reproduces what the author obviously meant).
    Info,
    /// Something the reference would silently accept or tolerate differently; worth a look.
    Warning,
    /// EMF would report a load error, or the model is not simulatable as is.
    Error,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub level: Level,
    /// Stable machine-readable kind, e.g. `unresolved-ref`, `unknown-feature`.
    pub kind: &'static str,
    /// `resource:line` or an object description.
    pub location: String,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let l = match self.level {
            Level::Info => "info",
            Level::Warning => "warning",
            Level::Error => "error",
        };
        write!(f, "{l}[{}] {}: {}", self.kind, self.location, self.message)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn push(
        &mut self,
        level: Level,
        kind: &'static str,
        location: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.0.push(Diagnostic {
            level,
            kind,
            location: location.into(),
            message: message.into(),
        });
    }
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter()
    }
    pub fn count(&self, level: Level) -> usize {
        self.0.iter().filter(|d| d.level == level).count()
    }
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Diagnostic> + 'a {
        self.0.iter().filter(move |d| d.kind == kind)
    }
    pub fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d.level == Level::Error)
    }
}
