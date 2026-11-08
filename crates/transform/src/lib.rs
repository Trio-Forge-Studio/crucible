const ATTRIBUTE: &str = "attribute";
const EMIT: &str = "@trioforge/crucible/emit";
mod algorithms;
mod payload;
use crucible_compiler::extension::Extractor;
use crucible_compiler::extension::LowerCx;
use crucible_compiler::extension::{CallHook, ClassHook, Extension, FactHook, HelperModule, Hooks, TypeHook};
use crucible_compiler::tree;
use crucible_luau::{Expr, Name};
pub use payload::{Attribute, Check, Class, Dependency};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use tsrs_ast::{Node, Symbol};
use tsrs_core::P;

#[derive(Default)]
struct State {
    made: Option<HashMap<P<Symbol>, algorithms::Made>>,
    pending: Vec<P<Node>>,
}
struct DiCx<'a> {
    core: &'a mut Extractor,
    state: &'a mut State,
}
impl std::ops::Deref for DiCx<'_> {
    type Target = Extractor;
    fn deref(&self) -> &Extractor {
        self.core
    }
}
impl std::ops::DerefMut for DiCx<'_> {
    fn deref_mut(&mut self) -> &mut Extractor {
        self.core
    }
}

#[derive(Default)]
pub struct Framework {
    state: Rc<RefCell<State>>,
}
pub fn extension() -> Box<dyn Extension> {
    Box::<Framework>::default()
}

impl Extension for Framework {
    fn hooks(&mut self, cx: &mut Extractor) -> Hooks {
        let mut hooks = Hooks::default();
        for (entry, name, lower) in [
            (".", ATTRIBUTE, lower_attribute as crucible_compiler::extension::LowerHook),
            (".", "inject", lower_inject),
            ("./server", "boot", lower_boot),
            ("./client", "boot", lower_boot),
        ] {
            let symbols = cx.crucible.symbols(&mut cx.checker, entry, name);
            let state = self.state.clone();
            hooks.calls.push(CallHook {
                symbols,
                lower,
                extract: Rc::new(move |core, n, _| {
                    let mut state = state.borrow_mut();
                    let mut cx = DiCx { core, state: &mut state };
                    match name {
                        ATTRIBUTE => cx.attribute_rules(n),
                        "inject" => {
                            cx.inject_rules(n);
                            let target = cx.inject_target(n);
                            cx.site();
                            cx.put(n, "inject", target);
                        }
                        "boot" => cx.state.pending.push(n),
                        _ => unreachable!(),
                    }
                }),
            });
        }
        let mut bases = cx.crucible.symbols(&mut cx.checker, ".", "Subsystem");
        bases.extend(cx.crucible.symbols(&mut cx.checker, ".", "CrucibleBehaviour"));
        let state = self.state.clone();
        hooks.classes.push(ClassHook {
            bases,
            lower: lower_class,
            extract: Rc::new(move |core, n, file| {
                let mut state = state.borrow_mut();
                let mut cx = DiCx { core, state: &mut state };
                cx.crucible_class_rules(n, file);
                if let Some(class) = cx.registration(n) {
                    cx.site();
                    cx.put(n, "class", class);
                }
            }),
        });
        let symbols = cx.crucible.symbols(&mut cx.checker, ".", "All");
        let state = self.state.clone();
        hooks.types.push(TypeHook {
            symbols,
            extract: Rc::new(move |core, n, _| {
                let mut state = state.borrow_mut();
                DiCx { core, state: &mut state }.all_rules(n);
            }),
        });
        hooks.facts = vec![
            FactHook { kind: "class", lower: None },
            FactHook { kind: "inject", lower: None },
            FactHook { kind: "load", lower: Some(lower_load) },
        ];
        hooks.helpers.push(HelperModule { module: EMIT, local: "__crucible", path: cx.helper_path() });
        hooks
    }
    fn finish_program(&mut self, cx: &mut Extractor) {
        let mut state = self.state.borrow_mut();
        for call in std::mem::take(&mut state.pending) {
            cx.at(call, |core| DiCx { core, state: &mut state }.boot(call));
        }
    }
}

fn lower_boot(cx: &mut LowerCx<'_, '_>, n: tree::Node) -> Expr {
    let callee = cx.expression(n.expression());
    let args = cx.arguments(n);
    callee.call(args)
}
fn lower_load(cx: &mut LowerCx<'_, '_>, n: tree::Node) -> Expr {
    Expr::String(std::borrow::Cow::Owned(cx.fact::<String>(n, "load").unwrap().clone()))
}
fn lower_inject(cx: &mut LowerCx<'_, '_>, n: tree::Node) -> Expr {
    let target = cx.fact::<Dependency>(n, "inject");
    let (function, id) = match target {
        Some(Dependency::One(id)) => ("resolve", id),
        Some(Dependency::All(id)) => ("resolveAll", id),
        _ => return cx.unsupported(n, "inject without a class, interface or All target"),
    };
    cx.helper(EMIT, function).call(vec![Expr::String(std::borrow::Cow::Owned(id.to_string()))])
}
fn lower_attribute(cx: &mut LowerCx<'_, '_>, n: tree::Node) -> Expr {
    let field = n.parent().filter(|p| {
        p.kind() == tree::Kind::PropertyDeclaration && p.initializer().is_some_and(|i| skip_wrappers(i) == n)
    });
    let Some(name) = field.and_then(|f| f.name()).filter(|name| {
        matches!(
            name.kind(),
            tree::Kind::Identifier | tree::Kind::StringLiteral | tree::Kind::NoSubstitutionTemplateLiteral
        )
    }) else {
        return cx.unsupported(n, "attribute() outside a behaviour field initializer");
    };
    let mut args = vec![Expr::global("self"), Expr::String(std::borrow::Cow::Owned(name.text().to_owned()))];
    args.extend(cx.arguments(n));
    cx.helper(EMIT, ATTRIBUTE).call(args)
}
fn skip_wrappers(mut n: tree::Node) -> tree::Node {
    while matches!(
        n.kind(),
        tree::Kind::ParenthesizedExpression
            | tree::Kind::AsExpression
            | tree::Kind::TypeAssertionExpression
            | tree::Kind::NonNullExpression
            | tree::Kind::SatisfiesExpression
    ) {
        n = n.expression();
    }
    n
}
fn lower_class(cx: &mut LowerCx<'_, '_>, n: tree::Node, name: Name) {
    if let Some(class) = cx.fact::<Class>(n, "class") {
        emit::register(cx, class, name);
    }
}
mod emit;

pub fn compile(
    tsconfig: &std::path::Path,
    target: crucible_compiler::Target<'_>,
) -> Result<crucible_compiler::Compiled, crucible_compiler::Failure> {
    crucible_compiler::compile_with(tsconfig, target, extension)
}
pub fn analyze(
    tsconfig: &std::path::Path,
    target: crucible_compiler::Target<'_>,
) -> Result<crucible_compiler::extension::Facts, Vec<crucible_compiler::Diagnostic>> {
    crucible_compiler::analyze_with(tsconfig, target, extension)
}
