//! Crucible's transform, the compiler extension that knows the framework. Today it checks that the installed runtime
//! implements the ABI this transform emits. The framework's rules and lowerings still run inside the compiler and move
//! here one feature at a time.

use std::path::Path;

use crucible_abi::{MANIFEST_FIELD, PACKAGE, UNDECLARED, VERSION};
use crucible_compiler::extension::{Extension, Manifest, ProgramCx};
use crucible_compiler::{Compiled, Failure, Target};

/// TypeScript's number for a Crucible error that is not a rule.
const SETUP: i32 = 90000;

/// `crucible_compiler::compile` with Crucible's extension.
pub fn compile(tsconfig: &Path, target: Target<'_>) -> Result<Compiled, Failure> {
    crucible_compiler::compile_with(tsconfig, target, &mut [&mut Crucible])
}

pub struct Crucible;

impl Extension for Crucible {
    fn name(&self) -> &'static str {
        "crucible"
    }

    fn begin_program(&mut self, cx: &mut ProgramCx<'_>) {
        if let Some(problem) = cx.manifest(PACKAGE).and_then(|manifest| mismatch(&manifest)) {
            cx.report(SETUP, problem);
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
