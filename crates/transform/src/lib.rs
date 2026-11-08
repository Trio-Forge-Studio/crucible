//! Crucible's transform, as an extension of the compiler: class registration, dependency injection, `attribute` and
//! `boot`. Its facts stay here, keyed by the compiler's site ids.

mod extract;
mod lower;

use std::collections::HashMap;
use std::path::Path;

use crucible_compiler::ext::{ExtractCx, Extension, LowerCx, ProgramCx, SiteId};
use crucible_compiler::tree::Node;
use crucible_compiler::tsrs::ast::SourceFile;
use crucible_compiler::tsrs::core::P;
use crucible_compiler::{Compiled, Failure, Span, Target};
use crucible_luau::{Expr, Name, Stmt};

/// The module registration and the macros call, bound to this local in each file that uses it.
const EMIT: &str = "__crucible";

#[derive(Default, Debug)]
pub struct Crucible {
    framework: Option<extract::Framework>,
    facts: HashMap<SiteId, Fact>,
}

impl Crucible {
    /// The fact at a site, for tests.
    pub fn fact(&self, site: SiteId) -> Option<&Fact> {
        self.facts.get(&site)
    }

    /// Every fact, for tests.
    pub fn facts(&self) -> impl Iterator<Item = (&SiteId, &Fact)> {
        self.facts.iter()
    }
}

impl Extension for Crucible {
    fn name(&self) -> &'static str {
        "crucible"
    }

    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: P<SourceFile>) {
        let framework = self.framework.get_or_insert_with(|| extract::Framework::new(cx));
        extract::file(framework, &mut self.facts, cx, file);
    }

    fn finish_program(&mut self, cx: &mut ProgramCx<'_>) {
        let emit = self.framework.as_ref().and_then(|f| f.emit.as_deref()).and_then(|emit| cx.game_module(emit));
        cx.helper_module(EMIT, emit, "Crucible's macros and registration need a game that maps @trioforge/crucible/emit");
    }

    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr> {
        let fact = self.facts.get(&cx.site(call))?;
        lower::call(cx, call, fact)
    }

    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, name: &Name) -> Vec<Stmt> {
        match self.facts.get(&cx.site(class)) {
            Some(Fact::Class(registered)) => vec![lower::register(cx, registered, name.clone())],
            _ => Vec::new(),
        }
    }
}

/// What the transform knows about a site.
#[derive(Clone, Debug, PartialEq)]
pub enum Fact {
    /// A class that extends `Subsystem` or `CrucibleBehaviour`, registered with `defineClass`.
    Class(Class),
    /// An `inject` call. Without a type argument its target is `Invalid`.
    Inject(Dependency),
    /// An `attribute` call.
    Attribute,
    /// A `boot` call, with each load glob's span and the instance path pattern the runtime matches, such as
    /// `crucible-path:ServerScriptService/TS/systems/**`.
    Boot(Vec<(Span, Box<str>)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassKind {
    Subsystem,
    Behaviour,
}

/// The registration data of a concrete Crucible class. A type id is `<package>/<module path>:<name>`, and `None` where the
/// transform reports that it cannot compute one.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    pub kind: ClassKind,
    pub id: Option<Box<str>>,
    /// The constructor's parameters in order.
    pub deps: Vec<Dependency>,
    /// Every base class and implemented interface below Crucible's own classes.
    pub provides: Vec<Box<str>>,
    /// The behaviour's tag literal.
    pub tag: Option<Box<str>>,
    /// The behaviour's `Instances` class names.
    pub instance: Vec<Box<str>>,
    pub attributes: Vec<Attribute>,
}

/// A constructor parameter or `inject` target, by type id.
#[derive(Clone, Debug, PartialEq)]
pub enum Dependency {
    One(Box<str>),
    /// `All<T>`.
    All(Box<str>),
    /// A target CRU001 or CRU005 rejects, or one without a type id. The pass reports it and the compile fails.
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub field: Box<str>,
    pub check: Check,
    /// False when `attribute()` has a default value.
    pub required: bool,
}

/// A runtime type check for an attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    Boolean,
    String,
    Number,
    Vector3,
    Color3,
    Literal(Literal),
    Union(Vec<Check>),
    /// A type CRU007 rejects. The pass reports it and the compile fails.
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Boolean(bool),
    Number(f64),
    String(Box<str>),
}

/// Compiles with Crucible's transform.
pub fn compile(tsconfig: &Path, target: Target<'_>) -> Result<Compiled, Failure> {
    crucible_compiler::compile_with(tsconfig, target, &mut [&mut Crucible::default()])
}

/// The compiler's facts and the transform's.
#[cfg(feature = "test-support")]
#[derive(Debug)]
pub struct Analysis {
    pub facts: crucible_compiler::Facts,
    pub crucible: Crucible,
}

#[cfg(feature = "test-support")]
impl std::ops::Deref for Analysis {
    type Target = crucible_compiler::Facts;

    fn deref(&self) -> &crucible_compiler::Facts {
        &self.facts
    }
}

#[cfg(feature = "test-support")]
impl Analysis {
    fn file_id(&self, path_suffix: &str) -> Option<crucible_compiler::ext::FileId> {
        let index = self.facts.files.iter().position(|f| f.path.ends_with(path_suffix))?;
        Some(crucible_compiler::ext::FileId(index as u32))
    }

    /// The transform's fact at `span` of the file whose path ends with `path_suffix`.
    pub fn fact(&self, path_suffix: &str, span: Span) -> Option<&Fact> {
        let file = self.file_id(path_suffix)?;
        self.crucible.fact(SiteId { file, span })
    }

    /// The registered classes of the file whose path ends with `path_suffix`, in source order.
    pub fn classes(&self, path_suffix: &str) -> Vec<Class> {
        let file = self.file_id(path_suffix).unwrap_or_else(|| panic!("{path_suffix} has facts"));
        let mut found: Vec<(Span, Class)> = self
            .crucible
            .facts()
            .filter(|(site, _)| site.file == file)
            .filter_map(|(site, fact)| match fact {
                Fact::Class(class) => Some((site.span, class.clone())),
                _ => None,
            })
            .collect();
        found.sort_by_key(|(span, _)| *span);
        found.into_iter().map(|(_, class)| class).collect()
    }
}

#[cfg(feature = "test-support")]
pub fn analyze(tsconfig: &Path, target: Target<'_>) -> Result<Analysis, Vec<crucible_compiler::Diagnostic>> {
    let mut crucible = Crucible::default();
    let facts = crucible_compiler::analyze_with(tsconfig, target, &mut [&mut crucible])?;
    Ok(Analysis { facts, crucible })
}
