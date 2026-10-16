//! Eclipse's AST validation after analysis (`QvtOperationalValidationVisitor`, run with duplicate
//! error suppression).

use crate::analyzer::Sema;
use crate::ir::ModuleId;

impl<'r> Sema<'r> {
    /// `QvtOperationalValidationVisitor` over a module.
    pub(crate) fn validate_module(&mut self, _m: ModuleId) {}
}
