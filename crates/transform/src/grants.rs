//! CRU039: a place that grants products grants each product `crucible:experience` declares exactly once, and a retired
//! product at most once. A place that grants none leaves ProcessReceipt unset, and Roblox offers its purchases again in
//! a server that handles them.

use std::collections::BTreeMap;
use std::path::Path;

use crucible_compiler::extension::{Kind, Node, ProgramCx, TypeFlags};

use crate::Code;
use crate::api::Api;
use crate::registration::Walk;

/// The module `crucible build` generates from `crucible.toml`.
const EXPERIENCE: &str = "crucible:experience";
const PRODUCTS: &str = "Products";
const HANDLE: &str = "ProductHandle";
/// A product `crucible.toml` retires, which may have a grant for the receipts of earlier purchases, or none.
const RETIRED: &str = "RetiredProductHandle";
const RULE: Code = Code::Rule(39);

#[derive(Default)]
pub(crate) struct Grants {
    /// The program's first grant, where a product with no grant is reported.
    first: Option<Node>,
    /// Whether a grant takes a handle whose key the checker cannot see, such as a helper's parameter.
    unkeyed: bool,
    /// Each product key a grant names, with where each of its grants is.
    by_key: BTreeMap<String, Vec<String>>,
    /// Whether the program assigns `MarketplaceService.ProcessReceipt` itself.
    pub(crate) answers_receipts: bool,
}

fn place(cx: &ProgramCx<'_>, n: Node) -> String {
    let file = n.file();
    let path = file.path();
    let shown = Path::new(&path).strip_prefix(cx.config_dir()).map_or(path.clone(), |p| p.to_string_lossy().into());
    let position = file.position(n.span().start);
    format!("{}:{}:{}", shown.replace('\\', "/"), position.line, position.column)
}

impl Walk<'_, '_> {
    /// Records the products a profile store's `grant` call names through their handles.
    pub(crate) fn grant(&mut self, call: Node) {
        self.state.grants.first.get_or_insert(call);
        let Some(argument) = call.arguments().get(0) else { return };
        let t = self.cx.type_at(argument);
        let members = if t.flags().intersects(TypeFlags::Union) { t.types() } else { vec![t] };
        let mut keys = Vec::new();
        for member in members {
            if !matches!(self.api(member.symbol()), Some(HANDLE | RETIRED)) {
                return;
            }
            match self.cx.type_arguments(member).first().and_then(|key| key.literal_string()) {
                Some(key) => keys.push(key),
                None => {
                    let message = format!(
                        "This grant's product is a handle whose key the compiler cannot see, so Crucible cannot check \
                         that each product has one grant. Pass a handle of {EXPERIENCE}, such as {PRODUCTS}.CoinPack."
                    );
                    self.report(argument, RULE, &message);
                    self.state.grants.unkeyed = true;
                    return;
                }
            }
        }
        let here = place(self.cx, call);
        for key in keys {
            let grants = self.state.grants.by_key.entry(key.to_owned()).or_default();
            let first = grants.first().cloned();
            grants.push(here.clone());
            if let Some(first) = first {
                let message = format!(
                    "Product {key} is granted here and at {first}. A purchase runs one grant, so grant each product \
                     once."
                );
                self.report(call, RULE, &message);
            }
        }
    }
}

impl Grants {
    pub(crate) fn keys(&self) -> Vec<String> {
        self.by_key.keys().cloned().collect()
    }

    /// Reports each product the program declares and never grants at the program's first grant, once it grants any.
    /// A grant whose key the compiler cannot see is already an error, and fixing it may grant any product.
    pub(crate) fn finish(&self, cx: &mut ProgramCx<'_>, api: &Api) {
        let Some(first) = self.first.filter(|_| !self.unkeyed) else { return };
        for (key, handle, declared) in declared_products(cx, api) {
            if self.by_key.contains_key(&key) || handle.retired {
                continue;
            }
            let granted: Vec<_> = self.by_key.values().flatten().map(String::as_str).collect();
            let elsewhere = match granted.as_slice() {
                [] => "This place grants products only by ID".to_owned(),
                _ => format!("This place grants other products at {}", granted.join(", ")),
            };
            let handle = handle.name;
            let (code, message) = RULE.render(&format!(
                "Product {key} has no grant. {declared} declares it as {PRODUCTS}.{handle}. {elsewhere}, so a \
                 purchase of {key} would be refused. Grant it once with a profile store's grant({PRODUCTS}.{handle}, \
                 ...)."
            ));
            cx.report_at(first, code, message);
        }
    }
}

struct Handle {
    name: String,
    retired: bool,
}

/// Each product of `crucible:experience`'s `Products`: its key, its handle and where it is declared.
fn declared_products(cx: &mut ProgramCx<'_>, api: &Api) -> Vec<(String, Handle, String)> {
    let mut found = Vec::new();
    for file in cx.files() {
        if !file.is_declaration() || cx.is_default_library(file) {
            continue;
        }
        for statement in file.root().statements().iter() {
            let named = statement.kind() == Kind::ModuleDeclaration
                && statement.name().is_some_and(|name| name.kind() == Kind::StringLiteral && name.text() == EXPERIENCE);
            let Some(body) = statement.body().filter(|_| named) else { continue };
            for variable in body.statements().iter().filter(|s| s.kind() == Kind::VariableStatement) {
                for declaration in variable.declarations().iter() {
                    let Some(name) = declaration.name().filter(|name| name.text() == PRODUCTS) else { continue };
                    let Some(symbol) = cx.symbol_at(name) else { continue };
                    let products = cx.type_of_symbol(symbol);
                    for property in cx.properties_of_type(products) {
                        let t = cx.type_of_symbol(property);
                        let retired = match api.name(cx, t.symbol()) {
                            Some(HANDLE) => false,
                            Some(RETIRED) => true,
                            _ => continue,
                        };
                        let Some(key) = cx.type_arguments(t).first().and_then(|key| key.literal_string()) else {
                            continue;
                        };
                        let at = property.declarations().first().map_or_else(|| place(cx, name), |d| place(cx, *d));
                        found.push((key.to_owned(), Handle { name: property.name().to_owned(), retired }, at));
                    }
                }
            }
        }
    }
    found
}
