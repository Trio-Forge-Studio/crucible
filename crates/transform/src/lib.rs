//! Crucible's transform: the framework features that plug into the compiler through its `Extension` trait. Boot,
//! class registration and dependency injection live here. The compiler names none of them.

mod api;
mod boot;
mod lower;
mod registration;

use std::collections::HashMap;
use std::path::Path;

use crucible_compiler::extension::{Extension, ExtractCx, HelperModule, LowerCx, Node, SourceFile};
use crucible_compiler::{Compiled, Failure, Target};
use crucible_luau::{Expr, Name, Stmt};

pub use registration::{Attribute, Check, Class, ClassKind, Dependency, Literal};

/// `@trioforge/crucible/emit`, which registration and the macros call.
const EMIT: HelperModule = HelperModule {
    package: "@trioforge/crucible",
    entry: "./emit",
    local: "__crucible",
    missing: "Crucible's macros and registration need a game that maps @trioforge/crucible/emit",
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Macro {
    Attribute,
    Inject,
    Boot,
}

impl Macro {
    fn name(self) -> &'static str {
        match self {
            Macro::Attribute => "attribute",
            Macro::Inject => "inject",
            Macro::Boot => "boot",
        }
    }
}

/// The transform's state for one program. Facts are keyed by the syntax node they lower, which the compiler hands
/// back unchanged at lowering time.
#[derive(Default)]
pub struct Crucible {
    api: api::Api,
    registration: registration::Cache,
    calls: HashMap<Node, Macro>,
    injects: HashMap<Node, Dependency>,
    /// Each `boot` call's load globs, by the string literal they replace.
    loads: HashMap<Node, Vec<(Node, Box<str>)>>,
    classes: HashMap<Node, Class>,
}

impl Extension for Crucible {
    fn name(&self) -> &'static str {
        "crucible"
    }

    fn begin_program(&mut self, cx: &mut ExtractCx<'_>) {
        self.api = api::Api::new(cx);
        cx.declare_helper(&EMIT);
    }

    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: SourceFile) {
        if self.api.macros_empty() || !cx.package_metadata_valid() {
            return;
        }
        let owned = self.api.owns(cx, file.path());
        self.walk(cx, file.root(), owned);
    }

    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr> {
        let m = *self.calls.get(&call)?;
        Some(self.lower_macro(cx, m, call))
    }

    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, local: &Name) -> Vec<Stmt> {
        match self.classes.get(&class) {
            Some(registered) => vec![lower::register(cx, registered, local.clone())],
            None => Vec::new(),
        }
    }
}

impl Crucible {
    /// One walk of the file. Macro calls are found in every file, and the rules run outside the runtime package.
    fn walk(&mut self, cx: &mut ExtractCx<'_>, n: Node, owned: bool) {
        use crucible_compiler::extension::SyntaxKind as K;
        let kind = n.kind();
        if kind == K::CallExpression
            && let Some(m) = self.api.macro_at(cx, n)
        {
            self.calls.insert(n, m);
            if m == Macro::Inject {
                let target = self.inject_target(cx, n);
                self.injects.insert(n, target);
            }
            if !owned {
                match m {
                    Macro::Inject => self.inject_rules(cx, n),
                    Macro::Attribute => self.attribute_rules(cx, n),
                    Macro::Boot => self.boot(cx, n),
                }
            }
        }
        if !owned {
            match kind {
                K::ClassDeclaration | K::ClassExpression => self.class_rules(cx, n),
                K::TypeReference => self.all_rule(cx, n),
                _ => {}
            }
        }
        let mut children = Vec::new();
        n.for_each_child(&mut |child| children.push(child));
        for child in children {
            self.walk(cx, child, owned);
        }
        if !owned
            && matches!(kind, K::ClassDeclaration | K::ClassExpression)
            && let Some(class) = self.registration(cx, n)
        {
            self.classes.insert(n, class);
        }
    }

    /// The registered classes of the program, for tests.
    #[cfg(feature = "test-support")]
    pub fn classes(&self) -> Vec<(String, u32, Class)> {
        let mut classes: Vec<(String, u32, Class)> = self
            .classes
            .iter()
            .map(|(node, class)| {
                (node.source_file().map(|f| f.path()).unwrap_or_default(), node.start(), class.clone())
            })
            .collect();
        classes.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        classes
    }

    /// The macro calls of the program in source order, by macro name, for tests.
    #[cfg(feature = "test-support")]
    pub fn calls(&self) -> Vec<(String, u32, &'static str)> {
        let mut calls: Vec<(String, u32, &'static str)> = self
            .calls
            .iter()
            .map(|(node, m)| (node.source_file().map(|f| f.path()).unwrap_or_default(), node.start(), m.name()))
            .collect();
        calls.sort();
        calls
    }

    /// The `inject` targets of the program in source order, for tests.
    #[cfg(feature = "test-support")]
    pub fn injects(&self) -> Vec<(String, u32, Dependency)> {
        let mut injects: Vec<(String, u32, Dependency)> = self
            .injects
            .iter()
            .map(|(node, d)| (node.source_file().map(|f| f.path()).unwrap_or_default(), node.start(), d.clone()))
            .collect();
        injects.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        injects
    }
}

/// The extensions a Crucible build registers.
pub fn extensions() -> Vec<Box<dyn Extension>> {
    vec![Box::new(Crucible::default())]
}

/// Compiles `tsconfig` with Crucible's transform.
pub fn compile(tsconfig: &Path, target: Target<'_>) -> Result<Compiled, Failure> {
    crucible_compiler::compile(tsconfig, target, extensions())
}

/// Builds the program for `tsconfig` and returns the compiler's fact table and the transform's facts.
#[cfg(feature = "test-support")]
pub fn analyze(
    tsconfig: &Path,
    target: Target<'_>,
) -> Result<(crucible_compiler::Facts, Crucible), Vec<crucible_compiler::Diagnostic>> {
    let mut crucible = Crucible::default();
    let facts = crucible_compiler::analyze(tsconfig, target, &mut [&mut crucible])?;
    Ok((facts, crucible))
}
