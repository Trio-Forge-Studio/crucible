//! Crucible's transform, the compiler extension that knows the framework. It checks that the installed runtime
//! implements the ABI this transform emits, and owns registration and dependency injection. The framework's other rules
//! and lowerings still run inside the compiler and move here one feature at a time.

mod api;
mod facts;
mod lower;
mod registration;

use std::collections::HashMap;
use std::path::Path;

use crucible_abi::{EMIT as EMIT_ENTRY, MANIFEST_FIELD, PACKAGE, UNDECLARED, VERSION};
use crucible_compiler::extension::{
    Extension, ExtractCx, File, HelperModule, LowerCx, Manifest, Node, ProgramCx, SiteId,
};
use crucible_compiler::{Compiled, Failure, Target};
use crucible_luau::{Expr, Name, Stmt};

use facts::Fact;
#[cfg(feature = "test-support")]
pub use facts::{Attribute, Check, Class, ClassKind, Dependency, Literal};

/// TypeScript's number for a Crucible error that is not a rule.
const SETUP: i32 = 90000;

/// The runtime functions compiled code calls.
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

/// `crucible_compiler::analyze` with Crucible's extension, which returns the extension with the facts it kept.
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
    facts: HashMap<SiteId, Fact>,
    /// Each registered class with the path of its file, in the order extraction found them.
    #[cfg(feature = "test-support")]
    classes: Vec<(String, Class)>,
}

#[cfg(feature = "test-support")]
impl Crucible {
    /// The classes registered in the file whose path ends with `file`.
    pub fn classes(&self, file: &str) -> Vec<Class> {
        self.classes.iter().filter(|(path, _)| path.ends_with(file)).map(|(_, class)| class.clone()).collect()
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
    }

    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: File) {
        // The compiler reports metadata it cannot read, and the transform then leaves the program alone.
        if self.api.is_none()
            || cx.metadata_invalid()
            || cx.package_name(Path::new(&file.path())).as_deref() == Some(PACKAGE)
        {
            return;
        }
        registration::Walk { cx, crucible: self }.file(file.root());
    }

    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr> {
        match self.facts.get(&cx.site(call))? {
            Fact::Inject(dependency) => Some(lower::inject(cx, call, dependency)),
            Fact::Attribute => Some(lower::attribute(cx, call)),
            Fact::Class(_) => None,
        }
    }

    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, local: &Name) -> Vec<Stmt> {
        match self.facts.get(&cx.site(class)) {
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
