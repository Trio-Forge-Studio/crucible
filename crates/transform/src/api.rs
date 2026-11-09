//! What the installed runtime exports, which the rules and lowerings recognize by symbol rather than by name.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crucible_abi::PACKAGE;
use crucible_compiler::extension::{Kind, ModifierFlags, Node, ProgramCx, Symbol, Type};

use crate::facts::ClassKind;

pub(crate) const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];

/// A call the transform expands in place of a call to the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Macro {
    Attribute,
    Inject,
}

impl Macro {
    /// The macros the package's root entry exports.
    const ROOT: [Macro; 2] = [Macro::Attribute, Macro::Inject];

    pub(crate) fn name(self) -> &'static str {
        match self {
            // The root's macro has the name of the emit function it lowers to.
            Macro::Attribute => crucible_abi::ATTRIBUTE.name,
            Macro::Inject => "inject",
        }
    }
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
}

impl Api {
    pub(crate) fn new(cx: &mut ProgramCx<'_>) -> Api {
        let mut api = Api {
            exports: HashMap::new(),
            macros: HashMap::new(),
            hooks: HashSet::new(),
            kinds: HashMap::new(),
            made: None,
        };
        let mut modules = Vec::new();
        for file in cx.files() {
            let path = PathBuf::from(file.path());
            if cx.package_name(&path).as_deref() != Some(PACKAGE) {
                continue;
            }
            let exports = cx.exports_of(file);
            for &symbol in &exports {
                let target = cx.aliased(symbol);
                api.exports.entry(target).or_insert(symbol.name());
            }
            modules.push((path, exports));
        }
        for (path, exports) in modules {
            let Some((_, module)) = cx.module_path(&path) else { continue };
            if cx.entry_module(&path, ".").as_deref() != Some(module.as_str()) {
                continue;
            }
            for symbol in exports {
                if let Some(m) = Macro::ROOT.into_iter().find(|m| m.name() == symbol.name()) {
                    let target = cx.aliased(symbol);
                    api.macros.insert(target, m);
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
