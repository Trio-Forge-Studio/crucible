//! The lowering half of the extension seam.

use super::{Lower, is_paren_or_assertion};
use crate::extension::{Extension, HelperModule, SiteId};
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
        self.first_extension(call, |e, cx, n| e.lower_call(cx, n))
    }

    /// The first extension's lowering of an expression.
    pub(super) fn extension_expression(&mut self, n: Node) -> Option<Expr> {
        self.first_extension(n, |e, cx, n| e.lower_expression(cx, n))
    }

    /// A path that lowers a node by its parts, instead of through `expr`, asks this first.
    pub(super) fn extension_value(&mut self, mut n: Node) -> Option<Expr> {
        loop {
            if let Some(value) = self.extension_expression(n) {
                return Some(value);
            }
            if !is_paren_or_assertion(n) {
                return None;
            }
            n = n.expression();
        }
    }

    fn first_extension(
        &mut self,
        n: Node,
        lower: impl Fn(&dyn Extension, &mut LowerCx<'_, '_>, Node) -> Option<Expr>,
    ) -> Option<Expr> {
        let extensions = self.extensions;
        extensions.iter().find_map(|&e| {
            let mut cx = LowerCx { l: self, lowered_arguments: false };
            let lowered = lower(e, &mut cx, n);
            // Lowering the arguments may already have emitted statements, so the compiler cannot lower them again.
            assert!(
                lowered.is_some() || !cx.lowered_arguments,
                "extension `{}` lowered the parts of a node and then left the node to the compiler",
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
}

impl LowerCx<'_, '_> {
    /// The id of a node of the file being lowered.
    pub fn site(&self, n: Node) -> SiteId {
        SiteId { file: self.l.file_id, span: n.span() }
    }

    /// A call's arguments in evaluation order, with a last argument that may return several values cut to one. A hook
    /// that calls this must lower the node itself: returning `None` afterwards panics.
    pub fn arguments(&mut self, call: Node) -> Vec<Expr> {
        self.lowered_arguments = true;
        self.l.macro_arguments(call)
    }

    /// A call's callee, lowered as the compiler lowers any expression. Like `arguments`, a hook that calls this must
    /// lower the node itself.
    pub fn callee(&mut self, call: Node) -> Expr {
        self.lowered_arguments = true;
        self.l.expr(call.expression())
    }

    /// Some of a call's argument nodes in evaluation order, each lowered to one value, for a lowering that drops the
    /// others. Like `arguments`, a hook that calls this must lower the node itself.
    pub fn some_arguments(&mut self, nodes: &[Node]) -> Vec<Expr> {
        self.lowered_arguments = true;
        self.l.ordered(nodes.iter().copied(), |l, n| l.expr(n))
    }

    /// `name` in a helper module the extension declared, which the file then imports, even if the hook goes on to
    /// return `None`.
    pub fn helper(&mut self, module: &HelperModule, name: &'static str) -> Expr {
        self.l.helper(module, name)
    }

    /// Fails the compile with `<what> is not supported` at `n`, and gives `nil` to lower in its place.
    pub fn unsupported(&mut self, n: Node, what: &str) -> Expr {
        self.l.unsupported(n, what)
    }
}
