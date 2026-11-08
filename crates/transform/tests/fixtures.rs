//! The macros of the compiler's fact fixtures, as the transform finds them.

use std::path::PathBuf;
use std::sync::LazyLock;

use crucible_compiler::Target;
use crucible_transform::{Crucible, Dependency};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/fixtures")
}

static CRUCIBLE: LazyLock<Crucible> = LazyLock::new(|| {
    let tsconfig = fixtures().join("tsconfig.json");
    let (_, crucible) = crucible_transform::analyze(&tsconfig, Target::Package).unwrap_or_else(|diagnostics| {
        let listed: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
        panic!("the fixtures check without diagnostics (run pnpm build first):\n{}", listed.join("\n"))
    });
    crucible
});

/// The start of `part` inside the only occurrence of `context` in `file`.
fn start(file: &str, context: &str, part: &str) -> u32 {
    let text = std::fs::read_to_string(fixtures().join("src").join(file)).expect("the fixture exists");
    assert_eq!(text.matches(context).count(), 1, "`{context}` occurs once in {file}");
    (text.find(context).unwrap() + context.find(part).unwrap()) as u32
}

fn inject(file: &str, context: &str, part: &str) -> Option<Dependency> {
    let at = start(file, context, part);
    CRUCIBLE.injects().into_iter().find(|(path, start, _)| path.ends_with(file) && *start == at).map(|(_, _, d)| d)
}

fn call(file: &str, context: &str, part: &str) -> Option<&'static str> {
    let at = start(file, context, part);
    CRUCIBLE.calls().into_iter().find(|(path, start, _)| path.ends_with(file) && *start == at).map(|(_, _, m)| m)
}

#[test]
fn inject_carries_its_target_type_id() {
    let store = Some(Dependency::One("crucible-workspace/crucible:Store".into()));
    let call_site = inject("crucible.ts", "inject<Store>()", "inject<Store>()");
    assert_eq!(call_site, store, "the id is relative to the common source directory");
    assert_eq!(call("crucible.ts", "inject<Store>()", "inject<Store>()"), Some("inject"), "inject is the macro");
    let aliased = inject("crucible.ts", "alias<Store>()", "alias<Store>()");
    assert_eq!(aliased, store, "an alias of inject resolves through its signature");
    assert_eq!(call("crucible.ts", "alias<Store>()", "alias<Store>()"), Some("inject"), "the alias is the macro");
    let untyped = inject("crucible.ts", "inject()", "inject()");
    assert_eq!(untyped, Some(Dependency::Invalid), "inject without a type argument is rejected");
}

#[test]
fn crucible_macros_come_from_their_entries() {
    assert_eq!(
        call("crucible.ts", "emitAttribute({}", "emitAttribute"),
        None,
        "the emit module's attribute is not the macro"
    );
    assert_eq!(call("crucible.ts", "boot({ load: [] })", "boot"), Some("boot"), "the server entry's boot is the macro");
}
