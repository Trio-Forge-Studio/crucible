use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crucible_abi::PACKAGE;
use crucible_compiler::extension::{Kind, ModifierFlags, Node, ProgramCx, Symbol, Type};

use crate::declarations::{DeclarationKind, Side};
use crate::facts::ClassKind;

pub(crate) const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Macro {
    Attribute,
    Boot,
    Inject,
    Receive,
    Declaration(DeclarationKind),
}

const MACRO_ENTRIES: [(&str, &[Macro]); 3] =
    [(".", &[Macro::Attribute, Macro::Inject]), ("./server", &[Macro::Boot]), ("./client", &[Macro::Boot])];

impl Macro {
    pub(crate) fn declarations() -> impl Iterator<Item = Macro> {
        [Macro::Receive].into_iter().chain(DeclarationKind::ALL.map(Macro::Declaration))
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
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

fn width_keys(cx: &mut ProgramCx<'_>, n: Node, out: &mut HashSet<Symbol>) {
    if n.kind() == Kind::VariableDeclaration
        && let Some(name) = n.name().filter(|name| name.kind() == Kind::Identifier && name.text() == "width")
        && let Some(symbol) = cx.symbol_at(name)
    {
        out.insert(symbol);
    }
    n.for_each_child(&mut |child| width_keys(cx, child, out));
}

pub(crate) struct Api {
    exports: HashMap<Symbol, &'static str>,
    macros: HashMap<Symbol, Macro>,
    /// The protected instance methods Crucible's own classes declare, which are its lifecycle hooks.
    pub(crate) hooks: HashSet<&'static str>,
    pub(crate) kinds: HashMap<Type, Option<ClassKind>>,
    pub(crate) width_keys: HashSet<Symbol>,
    pub(crate) side_entries: HashMap<PathBuf, Side>,
    pub(crate) entry_exports: HashMap<Symbol, Side>,
}

impl Api {
    pub(crate) fn new(cx: &mut ProgramCx<'_>) -> Api {
        let mut api = Api {
            exports: HashMap::new(),
            macros: HashMap::new(),
            hooks: HashSet::new(),
            kinds: HashMap::new(),
            width_keys: HashSet::new(),
            side_entries: HashMap::new(),
            entry_exports: HashMap::new(),
        };
        let mut modules = Vec::new();
        for file in cx.files() {
            let path = PathBuf::from(file.path());
            if cx.package_name(&path).as_deref() != Some(PACKAGE) {
                continue;
            }
            width_keys(cx, file.root(), &mut api.width_keys);
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
                    api.side_entries.insert(normalize(&path), side);
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

    pub(crate) fn name(&self, cx: &mut ProgramCx<'_>, symbol: Option<Symbol>) -> Option<&'static str> {
        self.exports.get(&cx.aliased(symbol?)).copied()
    }

    fn macro_of(&self, cx: &mut ProgramCx<'_>, symbol: Option<Symbol>) -> Option<Macro> {
        self.macros.get(&cx.aliased(symbol?)).copied()
    }

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
