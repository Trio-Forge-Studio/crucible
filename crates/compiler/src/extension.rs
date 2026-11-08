pub use crate::declarations::Side;
pub use crate::extract::Extractor;
pub use crate::extract::crucible::{Api, Code, is_global, is_reference, unalias};
pub use crate::extract::types::{literal_bool, literal_number, literal_string};
pub use crate::facts::{ClassKind, Facts, Literal, ModulePath};
pub use crate::lower::LowerCx;
pub use crate::project::{Game, RbxPath, normalize};
use crate::tree;
use crucible_luau::{Expr, Name};
use std::any::Any;
use std::collections::HashSet;
use std::fmt::Debug;
use std::rc::Rc;
use tsrs_ast::{Kind, Node, SourceFile, Symbol};
use tsrs_checker::TypeFlags;
use tsrs_core::P;

pub trait Payload: Any + Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn equals(&self, other: &dyn Payload) -> bool;
}
impl<T: Any + Debug + PartialEq + Send + Sync> Payload for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn equals(&self, other: &dyn Payload) -> bool {
        other.as_any().downcast_ref::<T>() == Some(self)
    }
}
#[derive(Clone, Debug)]
pub struct Value(std::sync::Arc<dyn Payload>);
impl Value {
    pub fn new<T: Any + Debug + PartialEq + Send + Sync>(value: T) -> Self {
        Self(std::sync::Arc::new(value))
    }
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.0.as_any().downcast_ref()
    }
}
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.0.equals(other.0.as_ref())
    }
}

pub type ExtractHook = Rc<dyn Fn(&mut Extractor, P<Node>, P<SourceFile>)>;
pub type LowerHook = fn(&mut crate::lower::LowerCx<'_, '_>, tree::Node) -> Expr;
pub type TailHook = fn(&mut crate::lower::LowerCx<'_, '_>, tree::Node, Name);

pub struct CallHook {
    pub symbols: HashSet<P<Symbol>>,
    pub extract: ExtractHook,
    pub lower: LowerHook,
}
pub struct ClassHook {
    pub bases: HashSet<P<Symbol>>,
    pub extract: ExtractHook,
    pub lower: TailHook,
}
pub struct TypeHook {
    pub symbols: HashSet<P<Symbol>>,
    pub extract: ExtractHook,
}
pub struct FactHook {
    pub kind: &'static str,
    pub lower: Option<LowerHook>,
}
pub struct HelperModule {
    pub module: &'static str,
    pub local: &'static str,
    pub path: Option<crate::facts::ModulePath>,
}
#[derive(Default)]
pub struct Hooks {
    pub helpers: Vec<HelperModule>,
    call_index: std::collections::HashMap<P<Symbol>, usize>,
    pub calls: Vec<CallHook>,
    pub classes: Vec<ClassHook>,
    pub types: Vec<TypeHook>,
    pub facts: Vec<FactHook>,
}
impl Hooks {
    pub(crate) fn index(&mut self) {
        for (index, hook) in self.calls.iter().enumerate() {
            for &symbol in &hook.symbols {
                self.call_index.entry(symbol).or_insert(index);
            }
        }
    }
}
pub trait Extension {
    fn hooks(&mut self, cx: &mut Extractor) -> Hooks;
    fn finish_program(&mut self, cx: &mut Extractor);
}
pub struct Empty;
impl Extension for Empty {
    fn hooks(&mut self, _: &mut Extractor) -> Hooks {
        Hooks::default()
    }
    fn finish_program(&mut self, _: &mut Extractor) {}
}
pub type Factory = fn() -> Box<dyn Extension>;
pub fn empty() -> Box<dyn Extension> {
    Box::new(Empty)
}

pub(crate) fn dispatch_file(cx: &mut Extractor, file: P<SourceFile>) {
    if cx.crucible.owns(std::path::Path::new(file.file_name())) || cx.crucible.invalid_metadata {
        return;
    }
    let hooks = cx.hooks();
    if hooks.calls.is_empty() && hooks.classes.is_empty() && hooks.types.is_empty() {
        return;
    }
    fn walk(cx: &mut Extractor, hooks: &Hooks, file: P<SourceFile>, n: P<Node>) {
        match n.kind() {
            Kind::ClassDeclaration | Kind::ClassExpression => {
                if let Some(instance) = cx.instance_type(n) {
                    let mut types = vec![instance];
                    let mut seen = HashSet::new();
                    let mut symbols = HashSet::new();
                    while let Some(t) = types.pop() {
                        if !seen.insert(t) {
                            continue;
                        }
                        if let Some(s) = t.symbol() {
                            symbols.insert(s);
                        }
                        if t.flags().intersects(TypeFlags::Intersection) {
                            types.extend(t.types());
                        } else {
                            types.extend(cx.resolved_bases(t));
                        }
                    }
                    if let Some(index) = hooks.classes.iter().position(|h| !h.bases.is_disjoint(&symbols)) {
                        cx.table.store(cx.span(n), crate::facts::Site::Crucible, |f| &mut f.class_hook, index);
                        (hooks.classes[index].extract)(cx, n, file);
                    }
                }
            }
            Kind::TypeReference => {
                let t = cx.checker.get_type_from_type_node_exported(n);
                if let Some(s) = t.symbol() {
                    for hook in &hooks.types {
                        if hook.symbols.contains(&s) {
                            (hook.extract)(cx, n, file);
                        }
                    }
                }
            }
            _ => {}
        }
        n.for_each_child(&mut |child| {
            walk(cx, hooks, file, child);
            false
        });
    }
    walk(cx, &hooks, file, file.as_node());
}

impl Extractor {
    pub fn hooks(&self) -> Rc<Hooks> {
        self.hooks.clone()
    }
    pub fn put<T: Any + Debug + PartialEq + Send + Sync>(&mut self, at: P<Node>, kind: &'static str, value: T) {
        assert!(self.hooks.facts.iter().any(|f| f.kind == kind), "unregistered extension fact kind");
        let span = self.span(at);
        self.table.facts.entry(span).or_default().extension.insert(kind, Value::new(value));
    }
    pub fn at(&mut self, at: P<Node>, run: impl FnOnce(&mut Self)) {
        let file = tsrs_ast::get_source_file_of_node(at).unwrap();
        self.path = file.file_name().replace('\\', "/");
        self.text = file.text();
        let index = self.facts.files.iter().position(|f| f.path == self.path).unwrap();
        self.table.facts = std::mem::take(&mut self.facts.files[index].facts).into_iter().collect();
        run(self);
        let mut facts: Vec<_> = std::mem::take(&mut self.table.facts).into_iter().collect();
        facts.sort_unstable_by_key(|(span, _)| *span);
        self.facts.files[index].facts = facts;
    }
    pub fn named_call(&mut self, n: P<Node>, name: &str) -> bool {
        if n.kind() != Kind::CallExpression {
            return false;
        }
        if resolve_call(self, n).0.is_none() {
            return false;
        }
        let symbol = self.checker.get_symbol_at_location_exported(n.as_call_expression().expression);
        if self.api(symbol) == Some(name) {
            return true;
        }
        let declaration = self.checker.get_resolved_signature_exported(n).declaration().and_then(|d| d.name());
        let symbol = declaration.and_then(|d| self.checker.get_symbol_at_location_exported(d));
        self.api(symbol) == Some(name)
    }
}

impl Extractor {
    pub fn helper_path(&self) -> Option<crate::facts::ModulePath> {
        self.facts.place.crucible_emit.clone()
    }
}

pub(crate) fn resolve_call(cx: &mut Extractor, n: P<Node>) -> (Option<usize>, Option<crate::macros::Macro>) {
    let table = cx.extension_enabled && !cx.hooks.call_index.is_empty();
    let legacy = cx.crucible.has_macros();
    if !table && !legacy {
        return (None, None);
    }
    let expression = n.as_call_expression().expression;
    let symbol = cx.checker.get_symbol_at_location_exported(expression);
    let symbol = symbol.map(|s| unalias(&mut cx.checker, s));
    let hook = if table { symbol.and_then(|s| cx.hooks.call_index.get(&s).copied()) } else { None };
    let mac = if legacy { cx.crucible.macro_of(&mut cx.checker, symbol) } else { None };
    if hook.is_some() || mac.is_some() {
        return (hook, mac);
    }
    let name = cx.checker.get_resolved_signature_exported(n).declaration().and_then(|d| d.name());
    let symbol = name.and_then(|n| cx.checker.get_symbol_at_location_exported(n)).map(|s| unalias(&mut cx.checker, s));
    let hook = if table { symbol.and_then(|s| cx.hooks.call_index.get(&s).copied()) } else { None };
    let mac = if legacy { cx.crucible.macro_of(&mut cx.checker, symbol) } else { None };
    (hook, mac)
}
