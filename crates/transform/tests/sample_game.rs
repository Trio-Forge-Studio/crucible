//! The sample game through Crucible's transform: every site answered, its registrations, and its boot globs.

mod support;

use std::path::{Path, PathBuf};

use crucible_compiler::Target;
use crucible_transform::{Attribute, Check, Class, ClassKind, Dependency};
use support::{Analysis, repo};

fn sample_place() -> (PathBuf, PathBuf) {
    let dir = repo().join("examples/sample-game");
    (dir.join("default.project.json"), dir.join("include"))
}

fn analyze(tsconfig: &str, target: Target<'_>) -> Analysis {
    support::analyze(&repo().join(tsconfig), target).unwrap_or_else(|diagnostics| {
        let listed: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
        panic!(
            "{tsconfig} checks without diagnostics:
{}",
            listed.join(
                "
"
            )
        )
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

fn assert_complete(facts: &Analysis, src: &str) {
    let unanswered: Vec<String> = facts.unanswered.iter().map(|u| format!("{u:?}")).collect();
    assert!(
        unanswered.is_empty(),
        "every site is answered:
{}",
        unanswered.join(
            "
"
        )
    );
    let suffix = |path: &str| path.rsplit_once(src).map(|(_, rest)| format!("{src}{rest}"));
    let mut expected = Vec::new();
    sources(&repo().join(src), &mut expected);
    let mut expected: Vec<Option<String>> = expected.iter().map(|path| suffix(path)).collect();
    let mut tables: Vec<Option<String>> = facts.files.iter().map(|f| suffix(&f.path)).collect();
    expected.sort();
    tables.sort();
    assert_eq!(tables, expected, "every source file has one fact table and nothing else does");
}

fn class(facts: &Analysis, file: &str) -> Class {
    assert!(facts.files.iter().any(|f| f.path.ends_with(file)), "{file} has facts");
    let classes = facts.crucible.classes(file);
    assert_eq!(classes.len(), 1, "{file} registers one class");
    classes.into_iter().next().unwrap()
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
fn the_sample_game_compiles_with_its_registrations_and_boot_globs() {
    let (project, include) = sample_place();
    let target = Target::Game { project: &project, include: &include };
    let modules = crucible_transform::compile(&repo().join("examples/sample-game/tsconfig.json"), target)
        .unwrap_or_else(|failure| panic!("the sample game compiles:\n{failure}"))
        .modules;
    let mut paths: Vec<&str> = modules.iter().map(|m| m.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        [
            "client/main.client.luau",
            "client/systems/hud.luau",
            "client/systems/menu.luau",
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
