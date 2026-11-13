mod api;
mod boot;
mod data;
mod declarations;
mod facts;
mod grants;
mod lints;
mod lower;
mod network;
mod registration;

use std::collections::HashMap;
use std::path::Path;

use crucible_abi::{EMIT as EMIT_ENTRY, MANIFEST_FIELD, PACKAGE, UNDECLARED, VERSION};
#[cfg(feature = "test-support")]
use crucible_compiler::extension::Span;
use crucible_compiler::extension::{
    Extension, ExtractCx, File, HelperModule, LowerCx, Manifest, Node, ProgramCx, SiteId,
};
use crucible_compiler::{Compiled, Failure, Target};
use crucible_luau::{Expr, Name, Stmt};

#[cfg(feature = "test-support")]
pub use declarations::DeclarationKind;
use facts::Fact;
#[cfg(feature = "test-support")]
pub use facts::{
    Attribute, Check, Class, ClassKind, Declaration, Dependency, Field, KeyLiteral, Literal, MapKey, Schema, Tail,
};

/// TypeScript's number for a Crucible error that is not a rule.
const SETUP: i32 = 90000;

pub(crate) const EMIT: HelperModule = HelperModule {
    package: PACKAGE,
    entry: EMIT_ENTRY,
    local: "__crucible",
    missing: "Crucible's macros and registration need a game that maps @trioforge/crucible/emit",
};

/// What a Crucible diagnostic reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Code {
    Rule(u16),
    /// The project's packages do not let Crucible compute a type id.
    Setup,
}

impl Code {
    /// The TypeScript error number and the printed message.
    pub(crate) fn render(self, message: &str) -> (i32, String) {
        match self {
            Code::Rule(rule) => (SETUP + i32::from(rule), format!("CRU{rule:03}: {message}")),
            Code::Setup => (SETUP, format!("Crucible: {message}")),
        }
    }
}

/// `crucible_compiler::compile` with Crucible's extension.
pub fn compile(tsconfig: &Path, target: Target<'_>) -> Result<Compiled, Failure> {
    crucible_compiler::compile_with(tsconfig, target, &mut [&mut Crucible::default()])
}

/// `compile`, and the key of each product the program grants through its handle. A place that grants nothing passes
/// CRU039, so `crucible build` checks across its places that some place grants each product.
pub fn compile_granting(tsconfig: &Path, target: Target<'_>) -> Result<(Compiled, Vec<String>), Failure> {
    let mut crucible = Crucible::default();
    let compiled = crucible_compiler::compile_with(tsconfig, target, &mut [&mut crucible])?;
    Ok((compiled, crucible.state.grants.keys()))
}

#[cfg(feature = "test-support")]
pub fn analyze(
    tsconfig: &Path,
    target: Target<'_>,
) -> Result<(crucible_compiler::Facts, Crucible), Vec<crucible_compiler::Diagnostic>> {
    let mut crucible = Crucible::default();
    let facts = crucible_compiler::analyze_with(tsconfig, target, &mut [&mut crucible])?;
    Ok((facts, crucible))
}

#[derive(Default)]
pub struct Crucible {
    api: Option<api::Api>,
    /// Whether CRU038 applies, which it does wherever Crucible is installed, imported or not, and wherever the
    /// program reaches Crucible another way, such as through `paths`.
    charm: bool,
    state: State,
}

#[derive(Default)]
pub(crate) struct State {
    facts: HashMap<SiteId, Fact>,
    network: network::Network,
    grants: grants::Grants,
    #[cfg(feature = "test-support")]
    recorded: Vec<(String, Span, Fact)>,
}

#[cfg(feature = "test-support")]
impl Crucible {
    pub fn classes(&self, path_suffix: &str) -> Vec<Class> {
        self.facts_in(path_suffix)
            .filter_map(|(_, fact)| if let Fact::Class(c) = fact { Some(c.clone()) } else { None })
            .collect()
    }

    pub fn declarations(&self, path_suffix: &str) -> Vec<(Span, Declaration)> {
        self.facts_in(path_suffix)
            .filter_map(|(span, fact)| if let Fact::Declaration(d) = fact { Some((span, d.clone())) } else { None })
            .collect()
    }

    fn facts_in(&self, path_suffix: &str) -> impl Iterator<Item = (Span, &Fact)> {
        self.state
            .recorded
            .iter()
            .filter(move |(path, ..)| path.ends_with(path_suffix))
            .map(|(_, span, fact)| (*span, fact))
    }
}

impl Extension for Crucible {
    fn name(&self) -> &'static str {
        "crucible"
    }

    fn begin_program(&mut self, cx: &mut ProgramCx<'_>) {
        if let Some(problem) = cx.manifest(PACKAGE).and_then(|manifest| mismatch(&manifest)) {
            cx.report(SETUP, problem);
        }
        cx.declare_helper(&EMIT);
        let api = api::Api::new(cx);
        self.api = (!api.is_empty()).then_some(api);
        self.charm = cx.installed(PACKAGE) || self.api.is_some();
    }

    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: File) {
        if cx.package_name(Path::new(&file.path())).as_deref() == Some(PACKAGE) {
            return;
        }
        lints::Lints { cx: &mut *cx, charm: self.charm }.file(file);
        if cx.metadata_invalid() {
            let (code, message) =
                Code::Setup.render("Cannot read package metadata. Check the project's package.json files.");
            cx.report(file.root(), code, &message);
            return;
        }
        let Some(api) = self.api.as_mut() else { return };
        registration::Walk { cx, api, state: &mut self.state }.file(file);
    }

    fn finish_program(&mut self, cx: &mut ProgramCx<'_>) {
        if let Some(api) = &self.api {
            self.state.grants.finish(cx, api);
        }
    }

    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr> {
        match self.state.facts.get(&cx.site(call))? {
            Fact::Inject(dependency) => Some(lower::inject(cx, call, dependency)),
            Fact::Attribute => Some(lower::attribute(cx, call)),
            Fact::Declaration(declaration) => Some(lower::declaration(cx, call, declaration)),
            Fact::Receive { field } => Some(lower::receive(cx, call, field)),
            Fact::Boot => Some(lower::boot(cx, call)),
            Fact::Class(_) | Fact::Load(_) => None,
        }
    }

    fn lower_expression(&self, cx: &mut LowerCx<'_, '_>, n: Node) -> Option<Expr> {
        match self.state.facts.get(&cx.site(n))? {
            Fact::Load(glob) => Some(lower::load(glob)),
            _ => None,
        }
    }

    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, local: &Name) -> Vec<Stmt> {
        match self.state.facts.get(&cx.site(class)) {
            Some(Fact::Class(registered)) => vec![lower::register(cx, registered, local)],
            _ => Vec::new(),
        }
    }
}

/// Why the installed runtime cannot run what this transform emits, when its manifest records another ABI version.
fn mismatch(manifest: &Manifest) -> Option<String> {
    let path = manifest.path.to_string_lossy().replace('\\', "/");
    let runtime = match manifest.json.get(MANIFEST_FIELD) {
        None => UNDECLARED,
        Some(value) => match value.as_u64().and_then(|v| u32::try_from(v).ok()) {
            Some(version) => version,
            None => {
                return Some(format!(
                    "Crucible: {path} sets {MANIFEST_FIELD} to {value}, which is not an ABI version. This compiler \
                     emits ABI {VERSION}."
                ));
            }
        },
    };
    (runtime != VERSION).then(|| {
        format!(
            "Crucible: {path} implements ABI {runtime}, and this compiler emits ABI {VERSION}. Install the {PACKAGE} \
             release that matches this compiler."
        )
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn manifest(json: &str) -> Manifest {
        let json = format!("{{ \"name\": \"{PACKAGE}\"{json} }}");
        Manifest { path: PathBuf::from("game/node_modules/x/package.json"), json: json.parse().unwrap() }
    }

    #[test]
    fn a_runtime_that_records_this_version_passes() {
        assert_eq!(mismatch(&manifest(&format!(", \"{MANIFEST_FIELD}\": {VERSION}"))), None);
    }

    #[test]
    fn a_runtime_without_the_field_implements_the_first_version() {
        let found = mismatch(&manifest(""));
        assert_eq!(found.is_some(), UNDECLARED != VERSION, "{found:?}");
    }

    #[test]
    fn another_version_names_both_numbers() {
        let other = VERSION + 1;
        let found = mismatch(&manifest(&format!(", \"{MANIFEST_FIELD}\": {other}"))).unwrap();
        assert_eq!(
            found,
            format!(
                "Crucible: game/node_modules/x/package.json implements ABI {other}, and this compiler emits ABI \
                 {VERSION}. Install the {PACKAGE} release that matches this compiler."
            )
        );
    }

    #[test]
    fn a_field_that_is_not_a_version_is_named() {
        let found = mismatch(&manifest(&format!(", \"{MANIFEST_FIELD}\": \"1\""))).unwrap();
        assert!(found.contains(&format!("sets {MANIFEST_FIELD} to \"1\", which is not an ABI version")), "{found}");
    }
}
