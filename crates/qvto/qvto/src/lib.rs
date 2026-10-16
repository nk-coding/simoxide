//! QVTo engine facade: compile a transformation (parse, analyse, compile to bytecode), bind
//! models, run it in compat or fast mode, collect the log and the diagnostics.
//!
//! ```ignore
//! let resolver = qvto::units::FsResolver::folder();
//! let main = resolver.unit_source(Path::new("t.qvto"))?;
//! let compiled = qvto::compile(main, &resolver, resource_set.graph.meta().clone());
//! let program = compiled.program.expect("no errors");
//! let mut vm = qvto::Vm::new(&program, &mut resource_set.graph, qvto::RunOptions::default());
//! let outcome = vm.run_transformation(vec![input_roots, vec![]]);
//! ```

pub mod units;

pub use qvto_bytecode::Program;
pub use qvto_sema::{Compilation, Problem, UnitResolver, UnitResult, UnitSource};
pub use qvto_vm::{RunOptions, RunOutcome, Vm};

use qvto_model::MetaRegistry;

/// The result of compiling a transformation.
pub struct Compiled {
    /// Every unit (the main unit first) with its problems.
    pub units: Vec<UnitResult>,
    /// The program, if the units have no errors and bytecode compilation succeeded.
    pub program: Option<Program>,
    /// Why there is no program although the units have no errors (a limit of the bytecode).
    pub bytecode_error: Option<String>,
}

impl Compiled {
    pub fn has_errors(&self) -> bool {
        self.units.iter().any(|u| u.errors().next().is_some())
    }
}

/// Parses and analyses `main` and everything it imports against the metamodels of `meta`, and
/// compiles the program to bytecode.
pub fn compile(main: UnitSource, resolver: &dyn UnitResolver, meta: MetaRegistry) -> Compiled {
    let c = qvto_sema::compile(main, resolver, meta);
    let has_errors = c.has_errors();
    let Compilation { units, program } = c;
    if has_errors {
        return Compiled {
            units,
            program: None,
            bytecode_error: None,
        };
    }
    match qvto_bytecode::compile(program) {
        Ok(p) => Compiled {
            units,
            program: Some(p),
            bytecode_error: None,
        },
        Err(e) => Compiled {
            units,
            program: None,
            bytecode_error: Some(e.0),
        },
    }
}
