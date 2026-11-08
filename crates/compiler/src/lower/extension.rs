//! The lowering half of the extension facade.

use super::Lower;
use crate::extension::{HelperModule, Node};
use crate::tree;
use crucible_luau::Expr;

pub struct LowerCx<'l, 'a> {
    l: &'l mut Lower<'a>,
}

impl Lower<'_> {
    fn node(&self, n: Node) -> tree::Node {
        tree::Node::from_raw(n.0, self.file.text())
    }

    /// A reference to `name` in a helper module, which imports the module into this file.
    pub(super) fn helper(&mut self, module: &'static HelperModule, name: &'static str) -> Expr {
        self.declared_helper(module.local, name)
    }

    /// A reference to `name` in the helper module an extension declared under `local`.
    pub(super) fn declared_helper(&mut self, local: &'static str, name: &'static str) -> Expr {
        match self.place.helpers.iter().position(|h| h.local == local) {
            Some(index) if !self.helpers.contains(&index) => self.helpers.push(index),
            Some(_) => {}
            None => {
                let root = self.file.root();
                self.error(root, format!("no extension declares the `{local}` helper module"));
            }
        }
        Expr::global(local).field(name)
    }

    /// The expression an extension lowers in place of `n`, if any.
    pub(super) fn replacement(&mut self, n: tree::Node) -> Option<Expr> {
        let at = self.replacements.iter().position(|(node, _)| node.0 == n.raw())?;
        Some(self.replacements.swap_remove(at).1)
    }

    /// Asks each extension to lower a call.
    pub(super) fn extension_call(&mut self, n: tree::Node) -> Option<Expr> {
        let extensions = self.extensions;
        let call = Node(n.raw());
        extensions.iter().find_map(|e| e.lower_call(&mut LowerCx { l: self }, call))
    }

    /// The statements each extension adds after a class declaration.
    pub(super) fn extension_class_tail(&mut self, class: tree::Node, local: &crucible_luau::Name) {
        let extensions = self.extensions;
        let class = Node(class.raw());
        for e in extensions {
            let tail = e.lower_class_tail(&mut LowerCx { l: self }, class, local);
            self.current_block.extend(tail);
        }
    }
}

impl LowerCx<'_, '_> {
    pub fn helper(&mut self, module: &'static HelperModule, name: &'static str) -> Expr {
        self.l.helper(module, name)
    }

    pub fn expr(&mut self, n: Node) -> Expr {
        let n = self.l.node(n);
        self.l.expr(n)
    }

    /// A call's arguments in evaluation order, with the last kept to one value. Each replacement lowers in place of
    /// its node.
    pub fn arguments(&mut self, call: Node, replacements: Vec<(Node, Expr)>) -> Vec<Expr> {
        let call = self.l.node(call);
        let outer = std::mem::replace(&mut self.l.replacements, replacements);
        let arguments = self.l.macro_arguments(call);
        self.l.replacements = outer;
        arguments
    }

    /// Reports `what` as unsupported at `n`.
    pub fn unsupported(&mut self, n: Node, what: &str) -> Expr {
        let n = self.l.node(n);
        self.l.unsupported(n, what)
    }
}
