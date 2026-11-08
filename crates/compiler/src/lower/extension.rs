//! The lowering half of the extension seam.

use super::Lower;
use crate::extension::{HelperModule, SiteId};
use crate::tree::Node;
use crucible_luau::{Expr, Name};

/// What an extension reads and lowers while the compiler lowers one source file.
pub struct LowerCx<'l, 'a> {
    l: &'l mut Lower<'a>,
    lowered_arguments: bool,
}

impl Lower<'_> {
    /// The first extension's lowering of a call.
    pub(super) fn extension_call(&mut self, call: Node) -> Option<Expr> {
        let extensions = self.extensions;
        extensions.iter().find_map(|e| {
            let mut cx = LowerCx { l: self, lowered_arguments: false };
            let lowered = e.lower_call(&mut cx, call);
            // Lowering the arguments may already have emitted statements, so the compiler cannot lower them again.
            assert!(
                lowered.is_some() || !cx.lowered_arguments,
                "extension `{}` lowered the arguments of a call and then left the call to the compiler",
                e.name()
            );
            lowered
        })
    }

    /// Each extension's statements after a class declaration, in the order of the extensions.
    pub(super) fn extension_class_tail(&mut self, class: Node, local: &Name) {
        let extensions = self.extensions;
        for e in extensions {
            let tail = e.lower_class_tail(&mut LowerCx { l: self, lowered_arguments: false }, class, local);
            self.current_block.extend(tail);
        }
    }

    pub(super) fn replacement(&mut self, n: Node) -> Option<Expr> {
        self.replacements.iter_mut().rev().find_map(|scope| {
            let at = scope.iter().position(|(node, _)| *node == n)?;
            Some(scope.swap_remove(at).1)
        })
    }
}

impl LowerCx<'_, '_> {
    /// The id of a node of the file being lowered.
    pub fn site(&self, n: Node) -> SiteId {
        SiteId { file: self.l.file_id, span: n.span() }
    }

    /// A call's arguments in evaluation order, with a last argument that may return several values cut to one. Each
    /// replacement lowers in place of its node, wherever that node sits in the arguments, until this returns. A call
    /// lowered inside the arguments keeps the replacements of the calls around it, and its own replacement of the same
    /// node wins. A `lower_call` that calls this must lower the call itself: returning `None` afterwards panics.
    pub fn arguments(&mut self, call: Node, replacements: Vec<(Node, Expr)>) -> Vec<Expr> {
        self.lowered_arguments = true;
        self.l.replacements.push(replacements);
        let arguments = self.l.macro_arguments(call);
        self.l.replacements.pop();
        arguments
    }

    /// `name` in a helper module the extension declared, which the file then imports.
    pub fn helper(&mut self, module: &HelperModule, name: &'static str) -> Expr {
        self.l.helper(module, name)
    }
}
