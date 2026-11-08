//! What the transform knows of the runtime package: its exports by symbol, which of them are macros, and the
//! lifecycle hooks its own classes declare.

use std::collections::{HashMap, HashSet};

use crucible_compiler::extension::{ExtractCx, Modifiers, Node, Symbol, SyntaxKind};

use crate::Macro;

pub(crate) const PACKAGE: &str = "@trioforge/crucible";
pub(crate) const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];
/// `@trioforge/crucible/emit` exports a runtime `attribute` that is not the macro.
const MACRO_ENTRIES: [(&str, &[Macro]); 3] =
    [(".", &[Macro::Attribute, Macro::Inject]), ("./server", &[Macro::Boot]), ("./client", &[Macro::Boot])];

#[derive(Default)]
pub(crate) struct Api {
    exports: HashMap<Symbol, &'static str>,
    macros: HashMap<Symbol, Macro>,
    /// The protected instance methods Crucible's own classes declare, which are its lifecycle hooks.
    pub(crate) hooks: HashSet<&'static str>,
}

impl Api {
    pub(crate) fn new(cx: &mut ExtractCx<'_>) -> Api {
        let mut api = Api::default();
        for file in cx.source_files() {
            if cx.package_name(file.path()).as_deref() != Some(PACKAGE) {
                continue;
            }
            let exports = cx.module_exports(file);
            for &symbol in &exports {
                let target = cx.unalias(symbol);
                api.exports.entry(target).or_insert(symbol.name());
            }
            for (entry, macros) in MACRO_ENTRIES {
                if !cx.is_package_entry(file, entry) {
                    continue;
                }
                for &symbol in &exports {
                    if let Some(&m) = macros.iter().find(|m| m.name() == symbol.name()) {
                        let target = cx.unalias(symbol);
                        api.macros.insert(target, m);
                    }
                }
            }
        }
        for (&symbol, name) in &api.exports {
            if !OWN_CLASSES.contains(name) {
                continue;
            }
            for declaration in symbol.declarations() {
                for member in declaration.members() {
                    let flags = member.modifiers();
                    if member.kind() == SyntaxKind::MethodDeclaration
                        && flags.intersects(Modifiers::PROTECTED)
                        && !flags.intersects(Modifiers::STATIC)
                        && let Some(name) = member.name()
                    {
                        api.hooks.insert(name.text());
                    }
                }
            }
        }
        api
    }

    pub(crate) fn macros_empty(&self) -> bool {
        self.macros.is_empty()
    }

    /// The transform's rules apply to the programs that use the package, not to the package's own sources.
    pub(crate) fn owns(&self, cx: &mut ExtractCx<'_>, path: String) -> bool {
        cx.package_name(path).as_deref() == Some(PACKAGE)
    }

    /// The export name of the runtime's symbol, through any alias.
    pub(crate) fn name(&self, cx: &mut ExtractCx<'_>, symbol: Option<Symbol>) -> Option<&'static str> {
        self.exports.get(&cx.unalias(symbol?)).copied()
    }

    /// The macro a call expands, by its callee's symbol or else by its resolved signature's declaration.
    pub(crate) fn macro_at(&self, cx: &mut ExtractCx<'_>, call: Node) -> Option<Macro> {
        let callee = call.expression()?;
        if let Some(symbol) = cx.symbol_at(callee)
            && let Some(&m) = self.macros.get(&cx.unalias(symbol))
        {
            return Some(m);
        }
        let name = cx.resolved_signature_declaration(call)?.name()?;
        let symbol = cx.symbol_at(name)?;
        self.macros.get(&cx.unalias(symbol)).copied()
    }
}
