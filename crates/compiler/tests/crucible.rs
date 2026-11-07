use std::path::{Path, PathBuf};

use crucible_compiler::{
    Attribute, Check, Class, ClassKind, Crucible, Dependency, Fact, Facts, Falsy, Receiver, Span, Target,
};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sample_place() -> (PathBuf, PathBuf) {
    let dir = repo().join("examples/sample-game");
    (dir.join("default.project.json"), dir.join("include"))
}

fn analyze(tsconfig: &str, target: Target<'_>) -> Facts {
    crucible_compiler::analyze(&repo().join(tsconfig), target).unwrap_or_else(|diagnostics| {
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
    let facts = analyze("packages/crucible/tsconfig.json", Target::Package);
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
    let (project, include) = sample_place();
    let facts = analyze("examples/sample-game/tsconfig.json", Target::Game { project: &project, include: &include });
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

#[test]
fn the_runtime_lowers_to_one_module_per_source_file() {
    let modules = crucible_compiler::compile(&repo().join("packages/crucible/tsconfig.json"), Target::Package)
        .unwrap_or_else(|failure| panic!("the runtime compiles:\n{failure}"))
        .modules;
    let src = repo().join("packages/crucible/src").to_string_lossy().replace('\\', "/");
    let mut files = Vec::new();
    sources(&repo().join("packages/crucible/src"), &mut files);
    let mut expected: Vec<String> = files
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(&format!("{src}/")).expect("the source is under src");
            let stem = relative.strip_suffix(".ts").expect("the runtime is TypeScript");
            let stem = match stem.strip_suffix("index") {
                Some(dir) if dir.is_empty() || dir.ends_with('/') => format!("{dir}init"),
                _ => stem.to_owned(),
            };
            format!("{stem}.luau")
        })
        .collect();
    let mut paths: Vec<String> = modules.iter().map(|m| m.path.clone()).collect();
    expected.sort();
    paths.sort();
    assert_eq!(paths, expected);
    let vide = &modules.iter().find(|m| m.path == "vide/init.luau").expect("the vide entry is emitted").source;
    assert!(
        vide.contains("local _0 = TS.import(script, TS.getModule(script, \"@rbxts\", \"vide\").src)"),
        "an installed package is found with TS.getModule, as roblox-ts emits it:\n{vide}"
    );
}

#[test]
fn the_sample_game_compiles_with_its_registrations_and_boot_globs() {
    let (project, include) = sample_place();
    let target = Target::Game { project: &project, include: &include };
    let modules = crucible_compiler::compile(&repo().join("examples/sample-game/tsconfig.json"), target)
        .unwrap_or_else(|failure| panic!("the sample game compiles:\n{failure}"))
        .modules;
    let mut paths: Vec<&str> = modules.iter().map(|m| m.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        [
            "client/main.client.luau",
            "client/systems/hud.luau",
            "client/systems/renderer.luau",
            "server/main.server.luau",
            "server/systems/controller.luau",
            "server/systems/event.luau",
            "server/systems/member.luau",
            "server/systems/store.luau",
            "server/systems/wallet.luau",
            "shared/messages.luau",
            "shared/profile.luau",
            "shared/state.luau",
        ]
    );
    let source = |path: &str| &modules.iter().find(|m| m.path == path).unwrap().source;
    let runtime = "local TS = require(game:GetService(\"ReplicatedStorage\"):WaitForChild(\"rbxts_include\"):WaitForChild(\"RuntimeLib\"))";
    for module in &modules {
        assert!(module.source.contains(runtime), "{} requires the runtime library:\n{}", module.path, module.source);
    }
    assert!(
        source("server/main.server.luau").contains("load = { \"crucible-path:ServerScriptService/TS/systems/**\" },")
    );
    assert!(
        source("client/main.client.luau")
            .contains("load = { \"crucible-path:StarterPlayer/StarterPlayerScripts/TS/systems/**\" },")
    );
    let controller = source("server/systems/controller.luau");
    assert!(controller.contains(
        "__crucible.defineClass(Controller, {\n\tkind = \"subsystem\",\n\tid = \"@trioforge/crucible-sample-game/server/systems/controller:Controller\",\n\tdeps = { \"@trioforge/crucible-sample-game/server/systems/store:Store\" },\n\tprovides = {},\n})"
    ));
    let member = source("server/systems/member.luau");
    assert!(member.contains("self.speed = __crucible.attribute(self, \"speed\", 17)"));
    assert!(member.contains("\ttag = \"S2Member\",\n\tinstance = { \"Part\" },\n\tattributes = {"));
}
