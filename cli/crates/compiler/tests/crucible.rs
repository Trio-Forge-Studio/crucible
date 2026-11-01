use std::path::{Path, PathBuf};

use crucible_compiler::{Attribute, Check, Class, ClassKind, Crucible, Dependency, Fact, Facts, Falsy, Receiver, Span};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn analyze(tsconfig: &str) -> Facts {
    crucible_compiler::analyze(&repo().join(tsconfig)).unwrap_or_else(|diagnostics| {
        let listed: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
        panic!("{tsconfig} checks without diagnostics:\n{}", listed.join("\n"))
    })
}

fn sources(dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("the source directory exists") {
        let path = entry.expect("the entry is readable").path();
        let name = path.to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            sources(&path, out);
        } else if (name.ends_with(".ts") || name.ends_with(".tsx")) && !name.ends_with(".d.ts") {
            out.push(name);
        }
    }
}

fn assert_complete(facts: &Facts, src: &str) {
    let unanswered: Vec<String> = facts.unanswered.iter().map(|u| format!("{u:?}")).collect();
    assert!(unanswered.is_empty(), "every site is answered:\n{}", unanswered.join("\n"));
    let suffix = |path: &str| path.rsplit_once(src).map(|(_, rest)| format!("{src}{rest}"));
    let mut expected = Vec::new();
    sources(&repo().join(src), &mut expected);
    let mut expected: Vec<Option<String>> = expected.iter().map(|path| suffix(path)).collect();
    let mut tables: Vec<Option<String>> = facts.files.iter().map(|f| suffix(&f.path)).collect();
    expected.sort();
    tables.sort();
    assert_eq!(tables, expected, "every source file has one fact table and nothing else does");
}

fn fact_at(facts: &Facts, file: &str, context: &str, part: &str) -> Fact {
    let text = std::fs::read_to_string(repo().join(file)).expect("the source exists");
    assert_eq!(text.matches(context).count(), 1, "`{context}` occurs once in {file}");
    let start = (text.find(context).unwrap() + context.find(part).unwrap()) as u32;
    let span = Span { start, end: start + part.len() as u32 };
    let table = facts.files.iter().find(|f| f.path.ends_with(file)).unwrap_or_else(|| panic!("{file} has facts"));
    table.get(span).cloned().unwrap_or_else(|| panic!("`{part}` in {file} has a fact"))
}

fn class(facts: &Facts, file: &str) -> Class {
    let table = facts.files.iter().find(|f| f.path.ends_with(file)).unwrap_or_else(|| panic!("{file} has facts"));
    let classes: Vec<Class> = table
        .facts
        .iter()
        .filter_map(|(_, fact)| match fact.crucible.as_deref() {
            Some(Crucible::Class(class)) => Some(class.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(classes.len(), 1, "{file} registers one class");
    classes.into_iter().next().unwrap()
}

#[test]
fn runtime_checks_and_every_site_is_answered() {
    let facts = analyze("packages/crucible/tsconfig.ts7.json");
    assert_complete(&facts, "packages/crucible/src");
    let array = Fact { receiver: Some(Receiver::Array), falsy: Some(Falsy::NONE), ..Fact::default() };
    for (file, context) in [
        ("packages/crucible/src/internal/network/codec.ts", "node.fields.map(([name, child])"),
        ("packages/crucible/src/internal/network/schema.ts", "fields.map(([name, child])"),
    ] {
        let pattern = fact_at(&facts, file, context, "[name, child]");
        assert_eq!(pattern, array, "the callback's parameter pattern in {file} destructures an array");
    }
}

#[test]
fn sample_game_checks_and_every_site_is_answered() {
    assert!(
        repo().join("packages/crucible/out/index.d.ts").is_file(),
        "packages/crucible/out exists; run pnpm build first"
    );
    let facts = analyze("examples/sample-game/tsconfig.ts7.json");
    assert_complete(&facts, "examples/sample-game/src");

    let id = |module: &str| format!("@trioforge/crucible-sample-game/{module}").into_boxed_str();
    let controller = class(&facts, "/server/systems/controller.ts");
    assert_eq!(controller.kind, ClassKind::Subsystem, "Controller is a subsystem");
    assert_eq!(controller.id, Some(id("server/systems/controller:Controller")), "Controller's type id");
    assert_eq!(controller.deps, [Dependency::One(id("server/systems/store:Store"))], "Controller injects Store");

    let member = class(&facts, "/server/systems/member.ts");
    assert_eq!(member.kind, ClassKind::Behaviour, "Member is a behaviour");
    assert_eq!(member.tag.as_deref(), Some("S2Member"), "Member's tag");
    assert_eq!(member.instance, ["Part".into()], "Member binds to parts");
    assert_eq!(
        member.attributes,
        [
            Attribute { field: "identity".into(), check: Check::String, required: true },
            Attribute { field: "speed".into(), check: Check::Number, required: false },
        ],
        "Member's attributes, with speed defaulted"
    );
}
