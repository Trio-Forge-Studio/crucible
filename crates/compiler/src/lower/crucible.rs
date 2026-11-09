use super::Lower;
use crate::macros::Macro;
use crate::tree::Node;
use crucible_luau::Expr;

impl Lower<'_> {
    pub(super) fn crucible_macro(&mut self, m: Macro, n: Node) -> Expr {
        match m {
            Macro::Boot => {
                let callee = self.expr(n.expression());
                let args = self.macro_arguments(n);
                callee.call(args)
            }
            _ => self.unsupported(n, &format!("the `{}` macro", m.name())),
        }
    }
}
