//! The errors extraction reports at a node.

use tsrs_ast::{Node, Symbol, SymbolFlags};
use tsrs_checker::Checker;
use tsrs_core::P;

use super::Extractor;
use crate::tree::{File, Position};

/// What an error at a node reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Code {
    Rule(u16),
    /// The place cannot hold a module where the program needs it.
    Place,
    /// An extension's, with its TypeScript error number. It writes the whole message.
    Extension(i32),
}

impl Code {
    /// The TypeScript error number and the printed message.
    pub(super) fn render(self, message: &str) -> (i32, String) {
        match self {
            Code::Rule(rule) => (90000 + i32::from(rule), format!("CRU{rule:03}: {message}")),
            Code::Place => (0, message.to_owned()),
            Code::Extension(code) => (code, message.to_owned()),
        }
    }
}

pub(super) fn unalias(c: &mut Checker, s: P<Symbol>) -> P<Symbol> {
    if s.flags().intersects(SymbolFlags::Alias) { c.get_aliased_symbol(s) } else { s }
}

impl Extractor {
    pub(super) fn report(&mut self, at: P<Node>, code: Code, message: &str) {
        let Some(file) = tsrs_ast::get_source_file_of_node(at) else { return };
        let span = crate::tree::span(at, file.text());
        let path = file.file_name().replace('\\', "/");
        if self.reported.insert((path, span.start, code.render("").0)) {
            self.push_diagnostic(at, span, code, message);
        }
    }

    fn push_diagnostic(&mut self, at: P<Node>, span: crate::facts::Span, code: Code, message: &str) {
        let file = tsrs_ast::get_source_file_of_node(at).unwrap();
        let path = file.file_name().replace('\\', "/");
        let start = span.start;
        let Position { line, column } = File::new(file).position(start);
        let (number, message) = code.render(message);
        self.facts.diagnostics.push(crate::Diagnostic { path: Some(path), line, column, code: number, message });
    }
}
