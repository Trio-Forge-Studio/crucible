//! What the installed runtime exports, which the rules and lowerings recognize by symbol rather than by name.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crucible_abi::PACKAGE;
use crucible_compiler::extension::{Kind, ModifierFlags, Node, ProgramCx, Symbol, Type};

use crate::declarations::{DeclarationKind, Side};
use crate::facts::ClassKind;

pub(crate) const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];

/// A call the transform expands in place of a call to the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Macro {
    Attribute,
    Boot,
    Inject,
    Receive,
    Declaration(DeclarationKind),
}

/// The entries whose exports are macros: the root's own, and the network and data macros any of them exports.
/// `@trioforge/crucible/emit` exports runtime functions of the same names that are not the macros.
const MACRO_ENTRIES: [(&str, &[Macro]); 3] =
    [(".", &[Macro::Attribute, Macro::Inject]), ("./server", &[Macro::Boot]), ("./client", &[Macro::Boot])];

impl Macro {
    /// The network and data macros, which declare something or receive a message.
    pub(crate) fn declarations() -> impl Iterator<Item = Macro> {
        [Macro::Receive].into_iter().chain(DeclarationKind::ALL.map(Macro::Declaration))
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            // These macros have the names of the emit functions they lower to.
            Macro::Attribute => crucible_abi::ATTRIBUTE.name,
            Macro::Receive => crucible_abi::RECEIVE.name,
            Macro::Declaration(kind) => kind.name(),
            Macro::Boot => "boot",
            Macro::Inject => "inject",
        }
    }
}

pub(crate) fn is_declaration_macro(name: &str) -> bool {
    Macro::declarations().any(|m| m.name() == name)
}

fn widths(cx: &mut ProgramCx<'_>, n: Node, out: &mut HashSet<Symbol>) {
    if n.kind() == Kind::VariableDeclaration
        && let Some(name) = n.name().filter(|name| name.kind() == Kind::Identifier && name.text() == "width")
        && let Some(symbol) = cx.symbol_at(name)
    {
        out.insert(symbol);
    }
    n.for_each_child(&mut |child| widths(cx, child, out));
}

/// When TypeScript 5's checker made a class or interface type, and whether the type has a `this` type.
#[derive(Clone, Copy)]
pub(crate) struct Made {
    pub(crate) turn: usize,
    pub(crate) this_type: bool,
}

pub(crate) struct Api {
    exports: HashMap<Symbol, &'static str>,
    macros: HashMap<Symbol, Macro>,
    /// The protected instance methods Crucible's own classes declare, which are its lifecycle hooks.
    pub(crate) hooks: HashSet<&'static str>,
    pub(crate) kinds: HashMap<Type, Option<ClassKind>>,
    /// When TypeScript 5 makes each class and interface type, computed when a union of instance classes needs it.
    pub(crate) made: Option<HashMap<Symbol, Made>>,
    /// The symbols of the runtime's `width` brand keys.
    pub(crate) widths: HashSet<Symbol>,
    /// The declaration files of the server and client entries.
    pub(crate) entries: HashMap<PathBuf, Side>,
    /// What the server and client entries export, with the side that may reference it.
    pub(crate) entry_exports: HashMap<Symbol, Side>,
}

impl Api {
    pub(crate) fn new(cx: &mut ProgramCx<'_>) -> Api {
        let mut api = Api {
            exports: HashMap::new(),
            macros: HashMap::new(),
            hooks: HashSet::new(),
            kinds: HashMap::new(),
            made: None,
            widths: HashSet::new(),
            entries: HashMap::new(),
            entry_exports: HashMap::new(),
        };
        let mut modules = Vec::new();
        for file in cx.files() {
            let path = PathBuf::from(file.path());
            if cx.package_name(&path).as_deref() != Some(PACKAGE) {
                continue;
            }
            widths(cx, file.root(), &mut api.widths);
            let exports = cx.exports_of(file);
            for &symbol in &exports {
                let target = cx.aliased(symbol);
                api.exports.entry(target).or_insert(symbol.name());
            }
            modules.push((path, exports));
        }
        for (path, exports) in modules {
            let Some((_, module)) = cx.module_path(&path) else { continue };
            for (entry, macros) in MACRO_ENTRIES {
                if cx.entry_module(&path, entry).as_deref() != Some(module.as_str()) {
                    continue;
                }
                let side = match entry {
                    "./server" => Some(Side::Server),
                    "./client" => Some(Side::Client),
                    _ => None,
                };
                if let Some(side) = side {
                    api.entries.insert(normalize(&path), side);
                }
                for &symbol in &exports {
                    let target = cx.aliased(symbol);
                    if let Some(side) = side
                        && (side == Side::Client || !api.entry_exports.contains_key(&target))
                    {
                        api.entry_exports.insert(target, side);
                    }
                    let found = macros.iter().copied().chain(Macro::declarations()).find(|m| m.name() == symbol.name());
                    if let Some(m) = found {
                        api.macros.insert(target, m);
                    }
                }
            }
        }
        for (symbol, name) in &api.exports {
            if !OWN_CLASSES.contains(name) {
                continue;
            }
            for declaration in symbol.declarations() {
                for member in declaration.members().iter() {
                    let flags = member.modifier_flags();
                    if member.kind() == Kind::MethodDeclaration
                        && flags.intersects(ModifierFlags::Protected)
                        && !flags.intersects(ModifierFlags::Static)
                        && let Some(name) = member.name()
                    {
                        api.hooks.insert(name.text());
                    }
                }
            }
        }
        api
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.exports.is_empty()
    }

    pub(crate) fn exports_name(&self, name: &str) -> bool {
        self.exports.values().any(|&n| n == name)
    }

    /// The name the runtime exports a symbol under, through any import aliases.
    pub(crate) fn name(&self, cx: &mut ProgramCx<'_>, symbol: Option<Symbol>) -> Option<&'static str> {
        self.exports.get(&cx.aliased(symbol?)).copied()
    }

    fn macro_of(&self, cx: &mut ProgramCx<'_>, symbol: Option<Symbol>) -> Option<Macro> {
        self.macros.get(&cx.aliased(symbol?)).copied()
    }

    /// The macro a call expands.
    pub(crate) fn macro_at(&self, cx: &mut ProgramCx<'_>, call: Node) -> Option<Macro> {
        if self.macros.is_empty() {
            return None;
        }
        let symbol = cx.symbol_at(call.expression());
        if let Some(m) = self.macro_of(cx, symbol) {
            return Some(m);
        }
        let name = cx.signature_at(call).declaration()?.name()?;
        let symbol = cx.symbol_at(name);
        self.macro_of(cx, symbol)
    }
}

/// A path with its `.` and `..` components resolved.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}
