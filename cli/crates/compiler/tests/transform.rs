//! Crucible's transform: registration payloads, macro lowering, type ids and each rule at its exact line. A case
//! whose source TypeScript 7 rejects asserts the type error at the line its rule would report, because the compiler
//! stops before the Crucible pass.

mod support;

use std::path::{Path, PathBuf};

use crucible_compiler::{Check, Class, ClassKind, Crucible, Dependency, Facts, Failure, Literal};
use serde_json::{Value, json};
use support::{Game, copy_declarations, has, lines, project, repo, slashes};

const HEADER: &str = "import { Subsystem, CrucibleBehaviour, attribute, type All } from \"@trioforge/crucible\";\nimport { boot } from \"@trioforge/crucible/server\";\n";
const POSITIVE: &str = include_str!("../../../../packages/crucible/transformer-src/tests/fixtures/game.ts");

fn line_count(text: &str) -> usize {
    text.trim_end().split('\n').count()
}

fn classes(facts: &Facts, file: &str) -> Vec<Class> {
    let table = facts.files.iter().find(|f| f.path.ends_with(file)).unwrap_or_else(|| panic!("{file} has facts"));
    table
        .facts
        .iter()
        .filter_map(|(_, fact)| match fact.crucible.as_deref() {
            Some(Crucible::Class(class)) => Some(class.clone()),
            _ => None,
        })
        .collect()
}

fn class<'a>(classes: &'a [Class], name: &str) -> &'a Class {
    let id = format!(":{name}");
    classes
        .iter()
        .find(|c| c.id.as_deref().is_some_and(|i| i.ends_with(&id)))
        .unwrap_or_else(|| panic!("{name} registers"))
}

fn id(name: &str) -> Box<str> {
    format!("fixture-game/server/main:{name}").into()
}

#[test]
fn the_fixture_installs_the_real_package_declarations_byte_for_byte() {
    let game = Game::new();
    let installed = game.dir.join("node_modules/@trioforge/crucible");
    let package = repo().join("packages/crucible");
    let read = |p: &Path| std::fs::read(p).unwrap();
    assert_eq!(read(&installed.join("package.json")), read(&package.join("package.json")));
    for side in ["server", "client", "emit"] {
        assert_eq!(read(&installed.join(side).join("package.json")), read(&package.join(side).join("package.json")));
    }
    let mut declarations = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.to_string_lossy().ends_with(".d.ts") {
                out.push(path);
            }
        }
    }
    walk(&package.join("out"), &mut declarations);
    assert!(!declarations.is_empty());
    for file in declarations {
        let relative = file.strip_prefix(&package).unwrap();
        assert_eq!(read(&installed.join(relative)), read(&file), "{}", relative.display());
    }
}

#[test]
fn the_emit_literal_declaration_accepts_boolean_string_and_number_literals() {
    let game = Game::new();
    game.main("import { t } from \"@trioforge/crucible/emit\";\nt.literal(true);\nt.literal(false);\nt.literal(\"walk\");\nt.literal(-2);\n");
    let facts = game.analyze().unwrap_or_else(|d| panic!("the literals type-check:\n{}", lines(&d)));
    let found: Vec<i32> = facts.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(found, [90004], "importing emit is CRU004, and nothing else is reported");
}

#[test]
fn registration_payloads_inherited_deps_tags_attributes_provides_and_symbol_identity() {
    let game = Game::new();
    game.main(POSITIVE);
    let facts = game.facts();
    assert_eq!(facts.diagnostics, [], "the positive fixture reports nothing");
    let registered = classes(&facts, "src/server/main.ts");
    let store = class(&registered, "Store");
    assert_eq!(store.kind, ClassKind::Subsystem);
    assert_eq!(store.id, Some(id("Store")));
    assert_eq!(store.provides, [id("Base"), id("Child"), id("Parent")]);
    assert_eq!(class(&registered, "Jobs").deps, [Dependency::One(id("Store")), Dependency::All(id("Child"))]);
    let door = class(&registered, "Door");
    assert_eq!(door.kind, ClassKind::Behaviour);
    assert_eq!(door.deps, [Dependency::One(id("Store"))], "Door inherits Openable's constructor");
    assert_eq!(door.provides, [id("Openable")]);
    assert_eq!(door.tag.as_deref(), Some("Door"));
    assert_eq!(class(&registered, "ConstTag").tag.as_deref(), Some("ConstTag"));
    assert_eq!(class(&registered, "ConstTag").instance, ["Instance".into()]);
    let aliased = class(&registered, "AliasedTag");
    assert_eq!(aliased.tag.as_deref(), Some("AliasTag"));
    assert_eq!(aliased.instance, ["Model".into(), "BasePart".into()], "the transformer lists them in this order");
    assert_eq!(class(&registered, "ThroughAlias").tag.as_deref(), Some("Alias"));
    let attribute = |name: &str| door.attributes.iter().find(|a| &*a.field == name).unwrap_or_else(|| panic!("{name}"));
    assert_eq!((attribute("fleetId").check.clone(), attribute("fleetId").required), (Check::String, true));
    assert_eq!((attribute("maxSpeed").check.clone(), attribute("maxSpeed").required), (Check::Number, false));
    assert_eq!(attribute("label").check, Check::String);
    let literal = |s: &str| Check::Literal(Literal::String(s.into()));
    let Check::Union(mut modes) = attribute("mode").check.clone() else { panic!("mode is a union") };
    modes.sort_by_key(|m| format!("{m:?}"));
    assert_eq!(modes, [literal("drive"), literal("walk")], "the union keeps the checker's member order");
    assert_eq!(attribute("negative").check, Check::Literal(Literal::Number(-2.0)));
    assert_eq!(attribute("tint").check, Check::Color3);
    assert_eq!(attribute("offset").check, Check::Vector3);
    assert_eq!(attribute("open").check, Check::Boolean);
    assert_eq!(attribute("maybeOpen").check, Check::Boolean);
    assert!(door.attributes.iter().all(|a| a.required == (&*a.field == "fleetId")));

    let luau = game.luau("server/main.luau");
    let order: Vec<&str> =
        luau.match_indices("__crucible.defineClass(").map(|(i, _)| luau[i + 23..].split(',').next().unwrap()).collect();
    assert_eq!(order, ["Store", "Jobs", "Door", "ConstTag", "AliasedTag", "ThroughAlias"]);
    has(&luau, "self.fleetId = __crucible.attribute(self, \"fleetId\")");
    has(&luau, "self.maxSpeed = __crucible.attribute(self, \"maxSpeed\", 30)");
    has(&luau, "self.label = __crucible.attribute(self, \"label\", nil)");
    has(&luau, "__crucible.resolve(\"fixture-game/server/main:Store\")");
    has(&luau, "__crucible.resolveAll(\"fixture-game/server/main:Child\")");
    has(&luau, "\"crucible-path:ServerScriptService/TS/**/*{Service,Behaviour}\"");
    has(&luau, "fleetId = {\n\t\t\tname = \"FleetId\",\n\t\t\tcheck = __crucible.t.string,\n\t\t\trequired = true,");
    has(
        &luau,
        "check = __crucible.t.optional(__crucible.t.union(__crucible.t.literal(\"drive\"), __crucible.t.literal(\"walk\"))),",
    );
    has(&luau, "check = __crucible.t.optional(__crucible.t.literal(-2)),");
    has(&luau, "deps = { \"fixture-game/server/main:Store\", {\n\t\tall = \"fixture-game/server/main:Child\",\n\t} },");
}

#[test]
fn diagnostic_probes_and_clean_controls_use_exact_source_lines() {
    let game = Game::new();
    let probes = [
        ("CRU001", "export class Bad extends Subsystem { constructor(value: number) { super(); } }"),
        ("CRU002", "export class Bad extends CrucibleBehaviour<string> {}"),
        ("CRU003", "export class Bad<T> extends Subsystem {}"),
        ("CRU006", "boot({load: [\"outside/**/*.ts\"]});"),
        ("CRU007", "export class Bad extends CrucibleBehaviour<\"Bad\"> { field = attribute<never>({} as never); }"),
        ("CRU015", "const bad = attribute(1);"),
    ];
    for (i, (_, text)) in probes.iter().enumerate() {
        game.write(&format!("src/server/probe{i}.ts"), &format!("{HEADER}{text}"));
    }
    game.write(
        "src/server/clean.ts",
        &format!("{HEADER}export class Good extends Subsystem {{}}\nexport class Fine extends CrucibleBehaviour<\"Fine\"> {{ field = attribute(1); }}"),
    );
    let found = game.found();
    for (i, (code, _)) in probes.iter().enumerate() {
        let expected = format!("{code}@src/server/probe{i}.ts:3");
        assert!(found.contains(&expected), "{expected} in {found:?}");
    }
    assert!(!found.iter().any(|f| f.contains("clean.ts")), "{found:?}");

    let global = Game::new();
    global.main(&format!(
        "{HEADER}declare global {{ interface GlobalType {{ readonly value: string; }} }}\nexport class Bad extends Subsystem {{ constructor(value: GlobalType) {{ super(); }} }}"
    ));
    assert!(global.found().contains(&"CRU010@src/server/main.ts:4".to_owned()), "{:?}", global.found());

    let rejected = Game::new();
    rejected.main(&format!("{HEADER}export class Bad extends CrucibleBehaviour<\"Bad\", {{ wrong: true }}> {{}}"));
    assert!(rejected.type_errors().contains(&"src/server/main.ts:3".to_owned()), "CRU008's probe is a type error");
    rejected.main(&format!("{HEADER}boot({{load: [undefined]}});"));
    assert!(rejected.type_errors().contains(&"src/server/main.ts:3".to_owned()), "a malformed load is a type error");
}

#[test]
fn cru001_rejects_every_noninjectable_constructor_shape_and_accepts_abstract_behaviours_in_all() {
    let game = Game::new();
    let header = "import { Subsystem, CrucibleBehaviour, type All } from \"@trioforge/crucible\";\n";
    let declarations = "declare class Dependency {}\nabstract class AbstractDependency {}\ninterface Generic<T> { value: T; }\nclass Concrete extends CrucibleBehaviour<\"Concrete\"> {}\n";
    let parameters = [
        "value: number",
        "value: typeof Dependency",
        "value: typeof AbstractDependency",
        "value: Instance",
        "value: Model | BasePart",
        "value?: Dependency",
        "...value: Dependency[]",
        "value: Generic<string>",
        "value: Concrete",
        "value: All<All<Dependency>>",
    ];
    for (i, parameter) in parameters.iter().enumerate() {
        game.write(
            &format!("src/server/case{i}.ts"),
            &format!(
                "{header}{declarations}export class Bad extends Subsystem {{ constructor({parameter}) {{ super(); }} }}"
            ),
        );
    }
    game.write(
        "src/server/good.ts",
        &format!("{header}abstract class Abstract extends CrucibleBehaviour<\"Abstract\"> {{}}\nexport class Good extends Subsystem {{ constructor(value: Abstract, all: All<Abstract>) {{ super(); }} }}"),
    );
    let found = game.found();
    for (i, parameter) in parameters.iter().enumerate() {
        let file = format!("src/server/case{i}.ts");
        assert!(found.iter().any(|f| f.starts_with("CRU001@") && f.contains(&file)), "{parameter}: {found:?}");
    }
    assert!(!found.iter().any(|f| f.contains("good.ts")), "{found:?}");

    let rejected = Game::new();
    rejected.main(&format!(
        "{header}{declarations}export class Bad extends Subsystem {{ constructor(value: All<number>) {{ super(); }} }}"
    ));
    assert!(rejected.type_errors().contains(&"src/server/main.ts:6".to_owned()), "All<number> is a type error");
}

#[test]
fn cru003_rejects_nested_generic_and_nonpublic_constructors_including_inherited_visibility() {
    let game = Game::new();
    let header = "import { Subsystem, CrucibleBehaviour, attribute } from \"@trioforge/crucible\";\n";
    let cases = [
        "export class Bad<T> extends Subsystem {}",
        "export function scope() { class Bad extends Subsystem {} }",
        "export const X = class extends Subsystem {};",
        "export function scope() { return class Y extends Subsystem {}; }",
        "export class Bad extends Subsystem { protected constructor() { super(); } }",
        "export class Bad extends Subsystem { private constructor() { super(); } }",
        "abstract class Base extends Subsystem { protected constructor() { super(); } }\nexport class Bad extends Base {}",
    ];
    for (i, text) in cases.iter().enumerate() {
        game.write(&format!("src/server/case{i}.ts"), &format!("{header}{text}"));
    }
    game.write(
        "src/server/expression.ts",
        &format!("{header}const Bad = class extends CrucibleBehaviour<\"Bad\"> {{ field = attribute(1); }};"),
    );
    let found = game.found();
    for (i, text) in cases.iter().enumerate() {
        let file = format!("src/server/case{i}.ts");
        assert!(found.iter().any(|f| f.starts_with("CRU003@") && f.contains(&file)), "{text}: {found:?}");
    }
    let expression: Vec<&String> = found.iter().filter(|f| f.contains("expression.ts")).collect();
    assert_eq!(expression, ["CRU003@src/server/expression.ts:2"]);
}

#[test]
fn cru002_rejects_tag_unions_and_widened_aliases_but_permits_open_abstract_tags() {
    let game = Game::new();
    let header = "import { CrucibleBehaviour } from \"@trioforge/crucible\";\n";
    game.write(
        "src/server/union.ts",
        &format!("{header}type Tag = \"a\" | \"b\"; export class Bad extends CrucibleBehaviour<Tag> {{}}"),
    );
    game.write(
        "src/server/widened.ts",
        &format!("{header}const tag: string = \"a\"; export class Bad extends CrucibleBehaviour<typeof tag> {{}}"),
    );
    game.write(
        "src/server/open.ts",
        &format!("{header}export abstract class Open<Tag extends string> extends CrucibleBehaviour<Tag> {{}}"),
    );
    let found = game.found();
    assert!(found.contains(&"CRU002@src/server/union.ts:2".to_owned()), "{found:?}");
    assert!(found.contains(&"CRU002@src/server/widened.ts:2".to_owned()), "{found:?}");
    assert!(!found.iter().any(|f| f.contains("open.ts")), "{found:?}");
}

#[test]
fn cru006_rejects_dynamic_load_shapes_and_maps_string_literal_paths() {
    let header = "import { boot } from \"@trioforge/crucible/server\";\n";
    let game = Game::new();
    let cases = [
        "const glob = \"src/server/**/*.ts\"; boot({load: [glob]});",
        "const load = [\"src/server/**/*.ts\"]; boot({load});",
        "const options = {load: []}; boot(options);",
        "const options = {load: [\"src/server/**/*.ts\"]}; boot({...options});",
    ];
    for (i, text) in cases.iter().enumerate() {
        game.write(&format!("src/server/case{i}.ts"), &format!("{header}{text}"));
    }
    let found = game.found();
    for (i, text) in cases.iter().enumerate() {
        let file = format!("src/server/case{i}.ts");
        assert!(found.iter().any(|f| f.starts_with("CRU006@") && f.contains(&file)), "{text}: {found:?}");
    }
    let overwritten = Game::new();
    overwritten
        .main(&format!("{header}const options = {{load: [\"src/server/**/*.ts\"]}}; boot({{load: [], ...options}});"));
    assert!(
        overwritten.type_errors().contains(&"src/server/main.ts:2".to_owned()),
        "a load the spread overwrites is a type error"
    );

    let literal = Game::new();
    literal.main(&format!("{header}boot({{\"load\": [\"src/server/main.ts\"]}});"));
    has(&literal.luau("server/main.luau"), "\"crucible-path:ServerScriptService/TS/main\"");
    literal.main(&format!(
        "{header}const options = {{fixedRate: 30}}; boot({{...options, load: [\"src/server/**/*.ts\"]}});"
    ));
    has(&literal.luau("server/main.luau"), "crucible-path:ServerScriptService/TS/**/*");
    for wrapper in ["({load: [\"src/server/**/*.ts\"]})", "{load: [\"src/server/**/*.ts\"]} as const"] {
        literal.main(&format!("{header}boot({wrapper});"));
        has(&literal.luau("server/main.luau"), "crucible-path:ServerScriptService/TS/**/*");
    }
}

#[test]
fn cru015_rejects_static_fields_nested_calls_plain_classes_and_subsystem_fields() {
    let game = Game::new();
    let header = "import { attribute, CrucibleBehaviour, Subsystem } from \"@trioforge/crucible\";\n";
    let cases = [
        "class Plain { field = attribute(1); }",
        "class Service extends Subsystem { field = attribute(1); }",
        "class Bad extends CrucibleBehaviour<\"Bad\"> { static field = attribute(1); }",
        "class Bad extends CrucibleBehaviour<\"Bad\"> { field = [attribute(1)]; }",
    ];
    for (i, text) in cases.iter().enumerate() {
        game.write(&format!("src/server/case{i}.ts"), &format!("{header}{text}"));
    }
    let found = game.found();
    for (i, text) in cases.iter().enumerate() {
        let expected = format!("CRU015@src/server/case{i}.ts:2");
        assert!(found.contains(&expected), "{text}: {found:?}");
    }
}

#[test]
fn cru010_rejects_global_provides_attributes_and_primitive_aliases() {
    let header = "import { attribute, CrucibleBehaviour, Subsystem } from \"@trioforge/crucible\";\n";
    let provides = Game::new();
    provides.main(&format!("{header}declare global {{ interface GlobalType {{}} }}\nexport class Bad extends Subsystem implements GlobalType {{}}"));
    assert!(provides.found().contains(&"CRU010@src/server/main.ts:3".to_owned()), "{:?}", provides.found());
    let attribute = Game::new();
    attribute.main(&format!("{header}declare global {{ interface GlobalValue {{ readonly x: number; }} }}\nexport class Bad extends CrucibleBehaviour<\"Bad\"> {{ field: GlobalValue = attribute<GlobalValue>({{x: 1}}); }}"));
    assert!(
        attribute.type_errors().contains(&"src/server/main.ts:3".to_owned()),
        "a non-attribute value is a type error"
    );
    let alias = Game::new();
    alias.main(
        "import { attribute, CrucibleBehaviour } from \"@trioforge/crucible\";\ndeclare global {\n type GlobalSpeed = number;\n}\n\nexport class Van extends CrucibleBehaviour<\"Van\"> {\n readonly speed = attribute<GlobalSpeed>();\n}",
    );
    let found: Vec<String> = alias.found().into_iter().filter(|f| f.starts_with("CRU010")).collect();
    assert_eq!(found, ["CRU010@src/server/main.ts:7"]);
}

#[test]
fn cru010_rejects_non_module_declarations() {
    let game = Game::new();
    game.write("global.d.ts", "interface NonModule { readonly name: string; }");
    game.main("/// <reference path=\"../../global.d.ts\" />\nimport { Subsystem } from \"@trioforge/crucible\";\nexport class Bad extends Subsystem { constructor(value: NonModule) { super(); } }");
    assert!(game.found().iter().any(|f| f.starts_with("CRU010@")), "{:?}", game.found());
}

#[test]
fn ambient_classes_register_nothing_while_concrete_classes_do() {
    let game = Game::new();
    game.main("import { Subsystem } from \"@trioforge/crucible\";\nexport declare class Ghost extends Subsystem {}\ndeclare namespace N { export class Ghost extends Subsystem {} }\ndeclare global { class GlobalGhost extends Subsystem {} }\nexport class Real extends Subsystem {}");
    let luau = game.luau("server/main.luau");
    assert!(!luau.contains("defineClass(Ghost") && !luau.contains("defineClass(GlobalGhost"), "{luau}");
    has(&luau, "__crucible.defineClass(Real, {");
}

#[test]
fn attribute_calls_lower_without_their_type_arguments_and_unions_use_a_union_check() {
    let game = Game::new();
    game.main("import { attribute, CrucibleBehaviour } from \"@trioforge/crucible\";\ntype Speed = number;\nexport class Vehicle extends CrucibleBehaviour<\"Vehicle\"> {\n readonly speed = attribute<Speed>();\n}");
    has(&game.luau("server/main.luau"), "self.speed = __crucible.attribute(self, \"speed\")");
    game.main("import { attribute, CrucibleBehaviour } from \"@trioforge/crucible\"; export class Mixed extends CrucibleBehaviour<\"Mixed\"> { readonly value = attribute<string | number>(); }");
    has(&game.luau("server/main.luau"), "__crucible.t.union(__crucible.t.string, __crucible.t.number)");
}

#[test]
fn api_identity_ignores_local_lookalikes_and_follows_namespace_and_renamed_imports() {
    let game = Game::new();
    game.main("import * as C from \"@trioforge/crucible\";\nimport { Subsystem as RealSubsystem } from \"@trioforge/crucible\";\nclass Subsystem { value = 1; }\nclass Fake extends Subsystem {}\nclass CrucibleBehaviour<T> { value?: T; }\nclass FakeBehaviour extends CrucibleBehaviour<\"Fake\"> {}\nclass Real extends RealSubsystem {}\nclass RealBehaviour extends C.CrucibleBehaviour<\"Real\"> {}\nfunction inject<T>(): T { return undefined as T; }\nconst untouched = inject<Real>();");
    let luau = game.luau("server/main.luau");
    assert_eq!(luau.matches("__crucible.defineClass(").count(), 2, "{luau}");
    has(&luau, "local untouched = inject()");
}

#[test]
fn cru007_and_cru008_reject_engine_type_lookalikes_and_allow_engine_aliases() {
    let header = "import { CrucibleBehaviour, attribute } from \"@trioforge/crucible\";\n";
    let vector = Game::new();
    vector.main(&format!("{header}class Vector3 {{ fake = 1; }}\nclass Bad extends CrucibleBehaviour<\"Bad\"> {{ field = attribute<never>(new Vector3() as never); }}"));
    assert!(vector.found().iter().any(|f| f.starts_with("CRU007@")), "{:?}", vector.found());
    let model = Game::new();
    model.main(&format!(
        "{header}class Model {{ fake = 1; }}\nclass Bad extends CrucibleBehaviour<\"Bad\", Model> {{}}"
    ));
    assert!(model.type_errors().contains(&"src/server/main.ts:3".to_owned()), "a Model lookalike is a type error");
    let aliases = Game::new();
    aliases.main(&format!("{header}type Body = Model; type Tint = Color3;\nclass Good extends CrucibleBehaviour<\"Good\", Body> {{ field = attribute<Tint>(new Color3()); }}"));
    assert_eq!(aliases.found(), Vec::<String>::new());
}

#[test]
fn a_malformed_macro_input_fails_the_compile_with_its_rule_and_no_panic() {
    let game = Game::new();
    game.main("import { boot } from \"@trioforge/crucible/server\";\nconst glob = \"src/server/**/*.ts\";\nboot({load: [glob]});");
    match crucible_compiler::compile(&game.dir.join("tsconfig.json")) {
        Err(Failure::Crucible(diagnostics)) => {
            assert!(diagnostics.iter().any(|d| d.code == 90006 && d.line == 3), "{}", lines(&diagnostics));
            assert!(lines(&diagnostics).contains("CRU006: load entry must be a string literal."));
        }
        other => panic!("CRU006 fails the compile: {other:?}"),
    }
}

#[test]
fn review_regressions_cover_alias_macros_abstract_only_imports_empty_files_and_client_boot() {
    let header = "import { CrucibleBehaviour, attribute, inject, type All } from \"@trioforge/crucible\";\n";
    let game = Game::new();
    game.main(&format!(
        "{header}export abstract class Base extends CrucibleBehaviour<\"Base\"> {{ field = attribute<boolean>(); }}"
    ));
    let luau = game.luau("server/main.luau");
    has(
        &luau,
        "local __crucible = TS.import(script, game:GetService(\"ReplicatedStorage\"), \"rbxts_include\", \"node_modules\", \"@trioforge\", \"crucible\", \"out\", \"emit\")",
    );
    assert!(!luau.contains("defineClass"), "{luau}");
    game.main("export const untouched = 1;");
    assert!(!game.luau("server/main.luau").contains("__crucible"));
    game.main(&format!("{header}const a = attribute; class B extends CrucibleBehaviour<\"B\"> {{ x = a(1); }}"));
    let luau = game.luau("server/main.luau");
    has(&luau, "self.x = __crucible.attribute(self, \"x\", 1)");
    has(&luau, "\t\tx = {\n\t\t\tname = \"X\",");
    game.main(&format!("{header}const a = attribute; const bad = a(1);"));
    assert!(game.found().contains(&"CRU015@src/server/main.ts:2".to_owned()), "{:?}", game.found());
    game.remove("src/server/main.ts");
    game.write(
        "src/client/main.ts",
        "import { boot } from \"@trioforge/crucible/client\"; boot({load: [\"src/server/**/*.ts\"]});",
    );
    has(&game.luau("client/main.luau"), "crucible-path:ServerScriptService/TS/**/*");
}

#[test]
fn type_id_failures_report_a_setup_diagnostic_for_classes_injects_and_provides() {
    let game = Game::new();
    game.json("package.json", &json!({ "private": true }));
    game.main("import { Subsystem, inject } from \"@trioforge/crucible\";\ninterface Dep {}\nclass S extends Subsystem implements Dep {}\nconst one = inject<S>();");
    let found = game.found();
    assert!(found.contains(&"Crucible@src/server/main.ts:3".to_owned()), "{found:?}");
    assert!(found.contains(&"Crucible@src/server/main.ts:4".to_owned()), "{found:?}");
    assert!(found.iter().all(|f| f.starts_with("Crucible@")), "{found:?}");
    game.main("import { Subsystem } from \"@trioforge/crucible\"; interface Dep {} class S extends Subsystem { constructor(dep: Dep) { super(); } }");
    let found = game.found();
    assert!(
        found.iter().any(|f| f.starts_with("Crucible@")) && !found.iter().any(|f| f.starts_with("CRU001")),
        "{found:?}"
    );
    game.write("package.json", "{");
    game.main("export const value = 1;");
    assert!(game.found().iter().any(|f| f.starts_with("Crucible@")), "{:?}", game.found());

    let sub = Game::new();
    sub.json("src/server/sub/package.json", &json!({ "type": "module" }));
    sub.write("src/server/sub/dep.ts", "export interface Dep {}");
    sub.main("import { Subsystem } from \"@trioforge/crucible\";\nimport type { Dep } from \"./sub/dep\";\nclass S extends Subsystem implements Dep {}");
    assert!(sub.found().iter().any(|f| f.starts_with("Crucible@")), "{:?}", sub.found());
}

#[test]
fn root_dirs_globs_map_and_a_missing_rojo_project_reports_cru006() {
    let text = "import { boot } from \"@trioforge/crucible/server\"; boot({load: [\"src/server/**/*.ts\"]});";
    let game = Game::new();
    game.main(text).config(|c| {
        c["compilerOptions"].as_object_mut().unwrap().remove("rootDir");
        c["compilerOptions"]["rootDirs"] = json!(["src"]);
    });
    has(&game.luau("server/main.luau"), "crucible-path:ServerScriptService/TS/**/*");
    game.remove("default.project.json");
    assert!(game.found().iter().any(|f| f.starts_with("CRU006@")), "{:?}", game.found());

    let roots = Game::new();
    let mut project = project();
    project["tree"]["ReplicatedStorage"]["Shared"] = json!({ "$path": "out/shared" });
    roots.json("default.project.json", &project).config(|c| {
        c["compilerOptions"].as_object_mut().unwrap().remove("rootDir");
        c["compilerOptions"]["rootDirs"] = json!(["src/shared", "src/server"]);
    });
    roots.write("src/shared/foo.ts", "export interface Foo { shared: true; }");
    roots.write("src/server/foo.ts", "export interface Foo { server: true; }");
    roots.main("import { Subsystem } from \"@trioforge/crucible\"; import { boot } from \"@trioforge/crucible/server\";\nimport type { Foo as Shared } from \"../shared/foo\"; import type { Foo } from \"./foo\";\nexport class S extends Subsystem implements Shared, Foo { shared = true as const; server = true as const; }\nboot({load: [\"src/server/**/*.ts\"]});");
    let registered = classes(&roots.facts(), "src/server/main.ts");
    let s = class(&registered, "S");
    assert_eq!(s.id, Some(id("S")));
    assert_eq!(s.provides, ["fixture-game/shared/foo:Foo".into(), "fixture-game/server/foo:Foo".into()]);
    let luau = roots.luau("server/main.luau");
    has(&luau, "id = \"fixture-game/server/main:S\"");
    has(&luau, "crucible-path:ServerScriptService/TS/**/*");
}

#[test]
fn diagnostics_deduplicate_repeated_and_inherited_faults_across_files() {
    let game = Game::new();
    game.main("import { Subsystem, CrucibleBehaviour, attribute } from \"@trioforge/crucible\";\ndeclare global { interface Global {} }\nabstract class Base extends Subsystem { constructor(value: Global) { super(); } }\nclass A extends Base {}\nclass B extends Base {}\nclass Bad extends CrucibleBehaviour<\"Bad\"> { field = attribute<any>(); }");
    let found = game.found();
    assert_eq!(found.iter().filter(|f| f.starts_with("CRU010")).count(), 1, "{found:?}");
    assert_eq!(found.iter().filter(|f| f.starts_with("CRU007")).count(), 1, "{found:?}");

    let files = Game::new();
    files.write("src/server/base.ts", "import { Subsystem } from \"@trioforge/crucible\"; export abstract class Base extends Subsystem { constructor(n: number) { super(); } }");
    files.write("src/server/a.ts", "import { Base } from \"./base\"; export class A extends Base {}");
    files.write("src/server/b.ts", "import { Base } from \"./base\"; export class B extends Base {}");
    files.main("export const unrelated = 1;");
    let inherited: Vec<String> = files.found().into_iter().filter(|f| f.starts_with("CRU001")).collect();
    assert_eq!(inherited, ["CRU001@src/server/base.ts:1"]);
}

#[test]
fn package_declaration_ids_ignore_the_consumer_root_and_respect_exports_types_conditions() {
    let expected: Box<str> = "@trioforge/fixture/events/handler:Handler".into();
    for root in ["src", "."] {
        let game = Game::new();
        let installed = "node_modules/@trioforge/fixture";
        game.json(
            &format!("{installed}/package.json"),
            &json!({ "name": "@trioforge/fixture", "version": "0.0.0", "types": "out/index.d.ts" }),
        );
        game.write(&format!("{installed}/out/index.d.ts"), "export type { Handler } from \"./events/handler\";\n");
        game.write(
            &format!("{installed}/out/events/handler.d.ts"),
            "export interface Handler {\n    handle(): void;\n}\n",
        );
        game.config(|c| c["compilerOptions"]["rootDir"] = json!(root));
        game.main("import { Subsystem } from \"@trioforge/crucible\";\nimport type { Handler } from \"@trioforge/fixture\";\nexport class Uses extends Subsystem implements Handler {\n\thandle() {}\n}\n");
        let registered = classes(&game.facts(), "src/server/main.ts");
        assert_eq!(registered[0].provides, std::slice::from_ref(&expected), "rootDir {root}");
        game.json(
            &format!("{installed}/package.json"),
            &json!({
                "name": "@trioforge/fixture",
                "types": "wrong/index.d.ts",
                "exports": { ".": { "import": { "types": "./out/index.d.ts", "default": "./out/index.js" } } },
            }),
        );
        let registered = classes(&game.facts(), "src/server/main.ts");
        assert_eq!(registered[0].provides, std::slice::from_ref(&expected), "exports types condition, rootDir {root}");
    }
    let source = Game::new();
    source.json("src/server/fixture/package.json", &json!({ "name": "@trioforge/fixture" }));
    source
        .json("src/server/fixture/tsconfig.json", &json!({ "compilerOptions": { "rootDir": "src", "outDir": "out" } }));
    source.write("src/server/fixture/src/events/handler.ts", "export interface Handler {\n\thandle(): void;\n}\n");
    source.main("import { Subsystem } from \"@trioforge/crucible\";\nimport type { Handler } from \"./fixture/src/events/handler\";\nexport class Uses extends Subsystem implements Handler {\n\thandle() {}\n}\n");
    let registered = classes(&source.facts(), "src/server/main.ts");
    assert_eq!(registered[0].provides, [expected], "the package's own source has the same id");
}

#[test]
fn the_positive_fixture_compiles_to_luau_and_hidden_collections_resolve() {
    let game = Game::new();
    game.main(POSITIVE);
    let luau = game.luau("server/main.luau");
    assert_eq!(luau.matches("__crucible.defineClass(").count(), 6);
    has(&luau, "kind = \"behaviour\"");
    has(&luau, "tag = \"Door\"");
    has(&luau, "crucible-path:ServerScriptService/TS");
    game.write("src/server/collection.ts", "import type { All } from \"@trioforge/crucible\"; interface Hidden { value: number; } export type Collection = All<Hidden>;");
    game.main("import { inject } from \"@trioforge/crucible\"; import type { Collection } from \"./collection\"; export const result: Collection = inject<Collection>();");
    has(&game.luau("server/main.luau"), "__crucible.resolveAll(\"fixture-game/server/collection:Hidden\")");
}

#[test]
fn exact_line_diagnostics_and_reviewer_bypasses() {
    let prefix = "import { Subsystem, CrucibleBehaviour, inject, type All } from \"@trioforge/crucible\";\n";
    let cases = [
        ("CRU004", "import \"@trioforge/crucible/emit\";"),
        ("CRU004", "export type Emit = typeof import(\"@trioforge/crucible/emit\");"),
        ("CRU005", "export class Concrete extends Subsystem {}\nexport const bad = inject<typeof Concrete>();"),
        ("CRU005", "export class Concrete extends Subsystem {}\nexport const bad = inject<All<typeof Concrete>>();"),
        ("CRU005", "export const bad = inject<{ a: number }>();"),
        (
            "CRU012",
            "interface MyP extends Promise<void> {}\nexport class Bad extends Subsystem { protected override start(): MyP | undefined { return undefined; } }",
        ),
        (
            "CRU014",
            "interface H<T> { value: T; }\nexport abstract class Openable<Tag extends string> extends CrucibleBehaviour<Tag, Model> {}\nexport abstract class B<Tag extends string> extends Openable<Tag> implements H<number> { value = 1; }\nexport class Door extends B<\"Door\"> {}",
        ),
        ("CRU005", "export class Bad extends Subsystem { field = inject<Subsystem>(); }"),
        ("CRU005", "export const bad = inject();"),
        ("CRU009", "export class Bad extends Subsystem { onInit() {} }"),
        ("CRU009", "export class Bad extends Subsystem { Update() {} }"),
        ("CRU009", "export class Bad extends Subsystem { updte() {} }"),
        ("CRU011", "export class Bad extends Subsystem { protected override preRender() {} }"),
        ("CRU011", "export class Bad extends Subsystem { static override renderPriority = 1; }"),
        ("CRU012", "export class Bad extends Subsystem { protected override async start() {} }"),
        (
            "CRU012",
            "export class Bad extends Subsystem { protected override start(): Promise<void> | undefined { return undefined; } }",
        ),
        (
            "CRU012",
            "type Result = Promise<void> | undefined;\nexport class Bad extends Subsystem { protected override start(): Result { return undefined; } }",
        ),
        (
            "CRU013",
            "export class Concrete extends Subsystem {}\nlet variable = Concrete;\nexport class Bad extends variable {}",
        ),
        (
            "CRU013",
            "export class Concrete extends Subsystem {}\nconst alias = Concrete;\nexport class Bad extends alias {}",
        ),
        (
            "CRU013",
            "export class Concrete extends Subsystem {}\ntype Constructor = typeof Concrete;\nconst alias: Constructor = Concrete;\nexport class Bad extends alias {}",
        ),
    ];
    let game = Game::new();
    for (i, (_, text)) in cases.iter().enumerate() {
        game.write(&format!("src/server/case{i}.ts"), &format!("{prefix}{text}"));
    }
    let found = game.found();
    for (i, (code, text)) in cases.iter().enumerate() {
        let expected = format!("{code}@src/server/case{i}.ts:{}", line_count(&format!("{prefix}{text}")));
        assert!(found.contains(&expected), "{expected}: {text}\n{found:?}");
    }
    assert!(!found.iter().any(|f| f.starts_with("Crucible@")), "no setup failure: {found:?}");

    for text in [
        "export const bad = inject<number>();",
        "export function make<T>() { return inject<T>(); }",
        "enum Target { A, B }\nexport const bad = inject<Target>();",
        "import \"@trioforge/crucible/internal/foo\";",
        "import type { Missing } from \"@trioforge/crucible/missing\";",
        "export const bad = inject<Missing>();",
        "export class Bad extends Subsystem { onInit(value: Missing) {} }",
        "export class Bad extends Subsystem { protected override preRender(value: Missing) {} }",
        "export class Bad extends Subsystem { protected override async start(value: Missing) {} }",
        "export class Concrete extends Subsystem {}\nexport class Bad extends Concrete implements Missing {}",
        "interface Generic<T> { value: T; }\nexport const bad = inject<Generic<Missing>>();",
    ] {
        let rejected = Game::new();
        rejected.main(&format!("{prefix}{text}"));
        let expected = format!("src/server/main.ts:{}", line_count(&format!("{prefix}{text}")));
        assert!(rejected.type_errors().contains(&expected), "{text} is a type error at {expected}");
    }

    let definitions = "export interface Handler<T = string> { handle(value: T): void; }\ntype Alias = Handler<string>;\ninterface Mid extends Handler<number> {}\ninterface Transitive extends Mid {}\n";
    let generic = Game::new();
    let mut expected = Vec::new();
    for (t, target) in ["Handler<string>", "Handler", "Alias", "Transitive"].iter().enumerate() {
        for (k, text) in [
            format!("export class Bad extends Subsystem implements {target} {{ handle(value: any) {{}} }}"),
            format!("export let bad: All<{target}>;"),
            format!("export const bad = inject<{target}>();"),
            format!("export const bad = inject<All<{target}>>();"),
        ]
        .iter()
        .enumerate()
        {
            let file = format!("src/server/generic{t}_{k}.ts");
            generic.write(&file, &format!("{prefix}{definitions}{text}"));
            expected.push(format!("CRU014@{file}:6"));
        }
    }
    let found = generic.found();
    for e in expected {
        assert!(found.contains(&e), "{e}: {found:?}");
    }

    let control = Game::new();
    control.main(&format!("{prefix}interface Plain {{ value: string; }}\nexport const one = inject<Plain>();\nexport let all: All<Plain>;\nexport class Clean extends Subsystem {{ protected override update() {{}} helper() {{}} upXXte() {{}} }}\nexport class CleanBehaviour extends CrucibleBehaviour<\"Clean\", Model> {{ Remove() {{}} isEnable() {{}} }}"));
    assert_eq!(control.found(), Vec::<String>::new());
}

#[test]
fn tag_forwarding_written_defaults_and_non_tag_parameters() {
    let prefix = "import { CrucibleBehaviour, inject, type All } from \"@trioforge/crucible\";\nexport abstract class Openable<Tag extends string = string> extends CrucibleBehaviour<Tag, Model> {}\nexport abstract class Forward<Tag extends string = string> extends Openable<Tag> {}\nexport abstract class ForwardAgain<Tag extends string = string> extends Forward<Tag> {}\n";
    let game = Game::new();
    for (i, target) in ["Openable<string>", "Openable", "ForwardAgain<string>", "ForwardAgain"].iter().enumerate() {
        game.write(
            &format!("src/server/clean{i}.ts"),
            &format!("{prefix}export let all: All<{target}>;\nexport const one = inject<{target}>();"),
        );
    }
    game.write("src/server/door.ts", &format!("{prefix}export class Door extends ForwardAgain<\"Door\"> {{}}"));
    let mut expected = Vec::new();
    for (i, target) in ["Openable<\"Door\">", "ForwardAgain<\"Door\">"].iter().enumerate() {
        for (k, text) in [format!("export let all: All<{target}>;"), format!("export const one = inject<{target}>();")]
            .iter()
            .enumerate()
        {
            let file = format!("src/server/tagged{i}_{k}.ts");
            game.write(&file, &format!("{prefix}{text}"));
            expected.push(format!("CRU014@{file}:5"));
        }
    }
    let mixed = format!(
        "{prefix}export abstract class Mixed<Tag extends string, Other = number> extends ForwardAgain<Tag> {{ value!: Other; }}\n"
    );
    for (i, text) in [
        "export class Door extends Mixed<\"Door\"> {}",
        "export let all: All<Mixed<string>>;",
        "export const one = inject<Mixed<string>>();",
    ]
    .iter()
    .enumerate()
    {
        game.write(&format!("src/server/mixed{i}.ts"), &format!("{mixed}{text}"));
    }
    let found = game.found();
    assert!(!found.iter().any(|f| f.contains("clean") || f.contains("door.ts")), "{found:?}");
    for e in expected {
        assert!(found.contains(&e), "{e}: {found:?}");
    }
    for i in 0..3 {
        let file = format!("src/server/mixed{i}.ts");
        assert!(found.iter().any(|f| f.starts_with("CRU014@") && f.contains(&file)), "{file}: {found:?}");
    }
}

fn both_hooks(class: &str) -> String {
    format!(
        "import {{ Subsystem }} from \"@trioforge/crucible\";\nexport class {class} extends Subsystem {{ protected override preRender() {{}} protected override onShutdown() {{}} }}"
    )
}

#[test]
fn folder_sides_ignore_directories_above_the_package() {
    for rootless in [false, true] {
        let game = Game::new();
        game.write("src/unmarked.ts", &both_hooks("Unmarked")).main("");
        if rootless {
            game.config(|c| {
                c["compilerOptions"].as_object_mut().unwrap().remove("rootDir");
            });
        }
        assert_eq!(game.found_in("src/unmarked.ts"), ["CRU011@src/unmarked.ts:2", "CRU016@src/unmarked.ts:2"]);
    }
}

#[test]
fn folder_sides_preserve_side_named_source_roots() {
    for folder in ["server", "client", "shared"] {
        for both in [false, true] {
            let game = Game::new();
            let members = match (both, folder) {
                (true, _) => "protected override preRender() {} protected override onShutdown() {}",
                (false, "server") => "protected override onShutdown() {}",
                (false, "client") => "protected override preRender() {}",
                (false, _) => "protected override update() {}",
            };
            let file = format!("src/{folder}/root-side.ts");
            game.write(&file, &format!("import {{ Subsystem }} from \"@trioforge/crucible\";\nexport class RootSide extends Subsystem {{ {members} }}"));
            game.config(|c| {
                c["compilerOptions"]["rootDir"] = json!(format!("src/{folder}"));
                c["include"] = json!([format!("src/{folder}")]);
            });
            let mut expected = Vec::new();
            if both && folder != "client" {
                expected.push(format!("CRU011@{file}:2"));
            }
            if both && folder != "server" {
                expected.push(format!("CRU016@{file}:2"));
            }
            assert_eq!(game.found_in(&file), expected, "{folder} both={both}");
        }
    }
}

#[test]
fn folder_sides_include_inherited_members_and_the_nearest_segment() {
    let game = Game::new();
    let prefix = "import { Subsystem } from \"@trioforge/crucible\";\n";
    for folder in ["client", "server", "shared", "server/client", "client/shared"] {
        game.write(
            &format!("src/{folder}/side.ts"),
            &format!("{prefix}export abstract class Base extends Subsystem {{ protected override preRender() {{}} static override renderPriority = 1; protected override onShutdown() {{}} }}"),
        );
    }
    game.write(
        "src/client/base.ts",
        &format!("{prefix}export abstract class Foreign extends Subsystem {{ protected override preRender() {{}} }}"),
    );
    game.write(
        "src/server/shutdown.ts",
        &format!("{prefix}export abstract class Shutdown extends Subsystem {{ protected override onShutdown() {{}} }}"),
    );
    game.write(
        "src/client/inherited.ts",
        "import { Shutdown } from \"../server/shutdown\";\nexport class Derived extends Shutdown {}",
    );
    for (folder, member) in [
        ("client", "protected override preRender() {}"),
        ("server", "protected override onShutdown() {}"),
        ("shared", "protected override update() {}"),
    ] {
        game.write(
            &format!("src/{folder}/clean.ts"),
            &format!("{prefix}export class Clean extends Subsystem {{ {member} }}"),
        );
    }
    game.main("import { Foreign } from \"../client/base\";\nexport class Derived extends Foreign {}");
    let found = game.found();
    assert!(!found.iter().any(|f| f.contains("clean.ts")), "{found:?}");
    assert!(found.contains(&"CRU016@src/client/inherited.ts:2".to_owned()), "{found:?}");
    for folder in ["client", "server", "shared", "server/client", "client/shared"] {
        let file = format!("src/{folder}/side.ts");
        let side = folder.rsplit('/').next().unwrap();
        assert_eq!(found.iter().any(|f| f.starts_with("CRU011") && f.contains(&file)), side != "client", "{file}");
        assert_eq!(found.iter().any(|f| f.starts_with("CRU016") && f.contains(&file)), side != "server", "{file}");
    }
    assert!(found.contains(&"CRU011@src/server/main.ts:2".to_owned()), "{found:?}");
}

#[test]
fn every_rule_reports_together_without_a_panic() {
    let game = Game::new();
    game.main("import { Subsystem, inject } from \"@trioforge/crucible\";\nimport \"@trioforge/crucible/emit\";\ninterface Promise<T> { value: T; }\ninterface Generic<T = string> { value: T; }\nexport class Concrete extends Subsystem { protected override preRender() {} onInit() {} field = inject<Subsystem>(); protected override start(): Promise<void> | undefined { return undefined; } }\nconst Alias = Concrete;\nexport class Derived extends Alias implements Generic { value = \"value\"; }");
    game.write("src/client/negative.ts", "import { Subsystem } from \"@trioforge/crucible\"; export class Bad extends Subsystem { protected override onShutdown() {} }");
    let Err(Failure::Crucible(diagnostics)) = crucible_compiler::compile(&game.dir.join("tsconfig.json")) else {
        panic!("the rules fail the compile");
    };
    let text = lines(&diagnostics);
    for code in ["CRU004", "CRU005", "CRU009", "CRU011", "CRU012", "CRU013", "CRU014", "CRU016"] {
        assert!(text.contains(code), "{code}:\n{text}");
    }
}

#[test]
fn constructor_collections_share_generic_validation() {
    let game = Game::new();
    game.main("import { Subsystem, CrucibleBehaviour, type All } from \"@trioforge/crucible\";\nexport abstract class Openable<Tag extends string = string> extends CrucibleBehaviour<Tag> {}\nexport class Clean extends Subsystem { constructor(readonly items: All<Openable>) { super(); } }");
    assert_eq!(game.found(), Vec::<String>::new());
}

#[test]
fn attribute_reads_keep_their_types_and_checks_use_dot_calls() {
    let game = Game::new();
    game.write("src/server/attribute-types.ts", "export type Tint = Color3;");
    game.main("import { attribute, CrucibleBehaviour } from \"@trioforge/crucible\";\nimport type { Tint } from \"./attribute-types\";\nexport class Van extends CrucibleBehaviour<\"Van\", Model> {\n readonly fleetId = attribute<string>();\n readonly maxSpeed = attribute(30);\n readonly label = attribute<string | undefined>(undefined);\n readonly tint: Tint = attribute(new Color3());\n readonly accent = attribute<Tint>(new Color3());\n readonly offset = attribute(new Vector3());\n readonly mode = attribute<\"walk\" | \"drive\">(\"walk\");\n protected override fixedUpdate(step: number) {\n  const root = this.instance.PrimaryPart;\n  if (root === undefined) return;\n  const velocity = root.AssemblyLinearVelocity;\n  if (velocity.Magnitude > this.maxSpeed) root.AssemblyLinearVelocity = velocity.Unit.mul(this.maxSpeed);\n  print(this.fleetId + \"-\", this.fleetId.size(), this.fleetId.sub(1, 2), this.label ?? \"none\", this.tint.R, this.accent.R, this.offset.Magnitude, step);\n }\n}");
    let luau = game.luau("server/main.luau");
    for part in ["__crucible.t.optional(", "__crucible.t.literal(", "__crucible.t.union("] {
        has(&luau, part);
    }
    assert!(!luau.contains("__crucible.t:"), "{luau}");
}

#[test]
fn inferred_attribute_aliases_resolve_outside_the_emitting_file() {
    let game = Game::new();
    game.write("src/server/modes.ts", "export type Mixed = \"a\" | 3; export const DEFAULT_MIXED: Mixed = \"a\";");
    game.main("import { attribute, CrucibleBehaviour } from \"@trioforge/crucible\";\nimport { DEFAULT_MIXED } from \"./modes\";\nexport class Vehicle extends CrucibleBehaviour<\"Vehicle\"> {\n readonly mixed = attribute(DEFAULT_MIXED);\n}");
    has(
        &game.luau("server/main.luau"),
        "check = __crucible.t.optional(__crucible.t.union(__crucible.t.literal(\"a\"), __crucible.t.literal(3))),",
    );
}

#[test]
fn abstract_attributes_compile_across_base_and_concrete_modules() {
    let game = Game::new();
    game.write("src/server/base.ts", "import { attribute, CrucibleBehaviour } from \"@trioforge/crucible\";\nexport abstract class Openable<Tag extends string> extends CrucibleBehaviour<Tag, Model> {\n readonly closedYaw = attribute(0);\n readonly key = attribute<string>();\n}");
    game.main("import { Openable } from \"./base\"; export class Door extends Openable<\"Door\"> {}");
    let modules = game.compile();
    let source = |path: &str| modules.iter().find(|m| m.path == path).unwrap().source.clone();
    let base = source("server/base.luau");
    has(&base, "local __crucible = ");
    has(&base, "__crucible.attribute(self, \"closedYaw\", 0)");
    has(&base, "__crucible.attribute(self, \"key\")");
    assert!(!base.contains("defineClass("), "{base}");
    let door = source("server/main.luau");
    has(&door, "defineClass(Door");
    has(&door, "name = \"ClosedYaw\"");
    has(&door, "name = \"Key\"");
}

struct Mode {
    name: &'static str,
    edit: fn(&mut Value),
}

struct SideCase {
    ancestor: &'static str,
    root_dirs: Option<Value>,
    files: Vec<(&'static str, String)>,
    expected: Vec<&'static str>,
}

fn rootless(config: &mut Value) {
    config["compilerOptions"].as_object_mut().unwrap().remove("rootDir");
}

#[test]
fn package_sides_follow_the_nearest_folder_under_the_package() {
    let shutdown = "import { Subsystem } from \"@trioforge/crucible\";\nexport class S extends Subsystem { protected override onShutdown() {} }";
    let render = "import { Subsystem } from \"@trioforge/crucible\";\nexport class C extends Subsystem { protected override preRender() {} }";
    let modes = [
        Mode { name: "explicit", edit: |_| {} },
        Mode { name: "inferred", edit: rootless },
        Mode {
            name: "rootDirs",
            edit: |c| {
                rootless(c);
                c["compilerOptions"]["rootDirs"] = json!(["src"]);
            },
        },
        Mode {
            name: "rootDirsWide",
            edit: |c| {
                rootless(c);
                c["compilerOptions"]["rootDirs"] = json!(["src", "generated"]);
                c["include"] = json!(["src", "generated"]);
            },
        },
    ];
    for ancestor in ["client", "server", "shared"] {
        for mode in &modes {
            let game = Game::under(ancestor);
            game.config(mode.edit);
            if mode.name == "rootDirsWide" {
                game.write("generated/keep.ts", "export const keep = 1;\n");
            }
            game.write("src/unmarked.ts", &both_hooks("U"));
            game.write("src/server/s.ts", shutdown);
            game.write("src/client/c.ts", render);
            game.write("src/server/x.ts", &render.replace("class C", "class X"));
            game.write("src/client/y.ts", &shutdown.replace("class S", "class Y"));
            assert_eq!(
                game.found(),
                [
                    "CRU016@src/client/y.ts:2",
                    "CRU011@src/server/x.ts:2",
                    "CRU011@src/unmarked.ts:2",
                    "CRU016@src/unmarked.ts:2",
                ],
                "{ancestor}/{}",
                mode.name
            );
        }
    }
    let server_files =
        || vec![("src/server/s.ts", shutdown.to_owned()), ("src/server/x.ts", render.replace("class C", "class X"))];
    let cases = [
        SideCase {
            ancestor: "client",
            root_dirs: None,
            files: server_files(),
            expected: vec!["CRU011@src/server/x.ts:2"],
        },
        SideCase {
            ancestor: "server",
            root_dirs: None,
            files: vec![
                ("src/client/c.ts", render.to_owned()),
                ("src/client/y.ts", shutdown.replace("class S", "class Y")),
            ],
            expected: vec!["CRU016@src/client/y.ts:2"],
        },
        SideCase {
            ancestor: "client",
            root_dirs: Some(json!(["src/server"])),
            files: server_files(),
            expected: vec!["CRU011@src/server/x.ts:2"],
        },
        SideCase {
            ancestor: "server",
            root_dirs: None,
            files: vec![("src/server/client/a.ts", render.to_owned()), ("src/client/server/b.ts", shutdown.to_owned())],
            expected: vec![],
        },
    ];
    for case in cases {
        let game = Game::under(case.ancestor);
        game.config(|c| {
            if let Some(dirs) = case.root_dirs {
                rootless(c);
                c["compilerOptions"]["rootDirs"] = dirs;
            }
        });
        for (file, text) in &case.files {
            game.write(file, text);
        }
        assert_eq!(game.found(), case.expected);
    }
}

#[test]
fn package_sides_follow_layouts_where_the_tsconfig_moves() {
    let combat = |game: &Game| {
        game.write("src/server/combat/main.ts", "import { Subsystem } from \"@trioforge/crucible\";\nexport class S extends Subsystem { protected override onShutdown() {} }");
        game.write("src/server/combat/r.ts", "import { Subsystem } from \"@trioforge/crucible\";\nexport class R extends Subsystem { protected override preRender() {} }");
    };
    let expected = ["CRU011@src/server/combat/r.ts:2"];

    let nested = Game::under("plain");
    combat(&nested);
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(nested.dir.join("tsconfig.json")).unwrap()).unwrap();
    nested.json("configs/tsconfig.base.json", &config);
    nested.json("configs/tsconfig.json", &json!({ "extends": "./tsconfig.base.json", "compilerOptions": { "rootDir": "../src", "outDir": "../out" }, "include": ["../src"] }));
    assert_eq!(nested.found_at("configs/tsconfig.json"), expected, "a tsconfig in another folder");

    let inside = Game::under("plain");
    combat(&inside);
    let mut config: Value =
        serde_json::from_str(&std::fs::read_to_string(inside.dir.join("tsconfig.json")).unwrap()).unwrap();
    config["compilerOptions"]["rootDir"] = json!(".");
    config["compilerOptions"]["outDir"] = json!("../../out/server");
    config["include"] = json!(["."]);
    inside.json("src/server/tsconfig.json", &config);
    assert_eq!(inside.found_at("src/server/tsconfig.json"), expected, "a tsconfig inside the server folder");

    for edit in [
        json!({ "rootDir": "src/server/combat" }),
        json!({ "rootDirs": ["src/server/combat"] }),
        json!({ "rootDir": "src/server" }),
    ] {
        let game = Game::under("plain");
        combat(&game);
        game.config(|c| {
            c["compilerOptions"].as_object_mut().unwrap().remove("rootDir");
            for (key, value) in edit.as_object().unwrap() {
                c["compilerOptions"][key] = value.clone();
            }
        });
        assert_eq!(game.found(), expected, "{edit}");
    }
}

#[test]
fn package_sides_stop_at_a_nested_package() {
    for folder in ["client", "server", "shared"] {
        for root in [".", "src"] {
            let game = Game::under(folder);
            game.write("src/unmarked.ts", &both_hooks("U"));
            if root == "src" {
                game.json("src/package.json", &json!({ "name": "nested-game" }));
            }
            assert_eq!(game.found(), ["CRU011@src/unmarked.ts:2", "CRU016@src/unmarked.ts:2"], "{folder} {root}");
        }
        let game = Game::under("plain");
        let file = format!("src/{folder}/main.ts");
        game.write(
            &file,
            "import { Subsystem } from \"@trioforge/crucible\";\nexport abstract class U extends Subsystem { protected override preRender() {} protected override onShutdown() {} }",
        );
        game.json(
            &format!("src/{folder}/package.json"),
            &if folder == "server" { json!({}) } else { json!({ "name": "nested" }) },
        );
        assert_eq!(game.found(), [format!("CRU011@{file}:2"), format!("CRU016@{file}:2")], "{folder} package root");
    }
}

#[test]
fn package_sides_without_a_package_search_the_whole_path() {
    let base = Game::under("plain");
    let dir = std::env::temp_dir().join(format!("crucible-side-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = dir.join("server/combat/main.ts");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "import { Subsystem } from \"@trioforge/crucible\";\nexport abstract class S extends Subsystem { protected override onShutdown() {} protected override preRender() {} }").unwrap();
    copy_declarations(
        &base.dir.join("node_modules/@trioforge/crucible"),
        &dir.join("node_modules/@trioforge/crucible"),
    );
    let roots = repo().join("packages/crucible/node_modules");
    let config = json!({
        "extends": slashes(&repo().join("tsconfig.ts7.json")),
        "compilerOptions": {
            "rootDir": "server/combat",
            "typeRoots": [slashes(&roots.join("@trioforge")), slashes(&roots.join("@rbxts"))],
            "types": ["compiler-types", "types"],
        },
        "include": ["server"],
    });
    std::fs::write(dir.join("tsconfig.json"), serde_json::to_string(&config).unwrap()).unwrap();
    let facts = crucible_compiler::analyze(&dir.join("tsconfig.json")).unwrap_or_else(|d| panic!("{}", lines(&d)));
    let found: Vec<(i32, u32)> = facts.diagnostics.iter().map(|d| (d.code, d.line)).collect();
    assert_eq!(found, [(90011, 2)]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A layer class inside a mixin can extend the concrete class too, so only `class`'s own line is required.
fn mixin_case(text: &str, class: &str) -> Vec<String> {
    let game = Game::new();
    game.main(text);
    let found: Vec<String> = game.found().into_iter().filter(|f| f.starts_with("CRU013")).collect();
    let line = text.lines().position(|l| l.contains(&format!("class {class}"))).unwrap() + 1;
    assert!(found.contains(&format!("CRU013@src/server/main.ts:{line}")), "{class}: {found:?}");
    assert!(!game.found().iter().any(|f| f.starts_with("Crucible@")));
    found
}

#[test]
fn cru013_rejects_concrete_mixins_directly_nested_through_variables_aliases_and_intersections() {
    let mixin = "import { Subsystem } from \"@trioforge/crucible\";\nfunction Mixin<T extends new (...args: any[]) => object>(Base: T) { return class extends Base {}; }\n";
    mixin_case(
        &format!(
            "{mixin}export class DirectConcrete extends Subsystem {{}}\nexport class DirectMixed extends Mixin(DirectConcrete) {{}}"
        ),
        "DirectMixed",
    );
    mixin_case(
        &format!(
            "{mixin}export class NestedConcrete extends Subsystem {{}}\nfunction Other(Base: typeof NestedConcrete) {{ return class InnerLayer extends Base {{}}; }}\nexport class NestedMixed extends Mixin(Other(NestedConcrete)) {{}}"
        ),
        "NestedMixed",
    );
    mixin_case(
        &format!(
            "{mixin}export class VariableConcrete extends Subsystem {{}}\nlet variable = Mixin(VariableConcrete);\nexport class VariableMixed extends variable {{}}"
        ),
        "VariableMixed",
    );
    mixin_case(
        &format!(
            "{mixin}export class AliasConcrete extends Subsystem {{}}\nconst mixinAlias = Mixin;\nconst baseAlias = mixinAlias(AliasConcrete);\nexport class AliasMixed extends baseAlias {{}}"
        ),
        "AliasMixed",
    );
    let ordered = mixin_case(
        "import { Subsystem } from \"@trioforge/crucible\";\ntype Ctor = new (...args: any[]) => object;\nfunction Extra<T extends Ctor>(B: T): T & (new (...args: any[]) => { extra: number }) { return B as never; }\nexport class OrderConcrete extends Subsystem {}\nexport class OrderPlain {}\nexport class PlainOrdered extends Extra(OrderPlain) {}\nexport class OrderMixed extends Extra(OrderConcrete) {}",
        "OrderMixed",
    );
    assert_eq!(
        ordered,
        ["CRU013@src/server/main.ts:7"],
        "a concrete class in any intersection position, and only there"
    );
}

#[test]
fn cru013_leaves_abstract_and_unregistered_mixins_legal() {
    let game = Game::new();
    game.main("import { Subsystem } from \"@trioforge/crucible\";\nfunction Mixin<T extends abstract new (...args: any[]) => object>(Base: T) { abstract class Layer extends Base {} return Layer; }\nexport abstract class AbstractOrigin extends Subsystem {}\nexport class AbstractMixed extends Mixin(AbstractOrigin) {}\nexport class PlainOrigin {}\nexport class PlainMixed extends Mixin(PlainOrigin) {}\nexport class PlainNested extends Mixin(Mixin(PlainOrigin)) {}\n");
    assert_eq!(game.found(), Vec::<String>::new());
}

#[test]
fn imports_the_place_cannot_hold_are_errors_at_their_specifier() {
    let game = Game::new();
    game.write("src/shared/util.ts", "export const shared = 1;");
    game.write("src/server/secret.ts", "export const secret = 1;");
    game.write("src/client/main.client.ts", "import { secret } from \"../server/secret\";\nprint(secret);");
    game.write("src/server/runner.server.ts", "export const runner = 1;");
    game.write("src/loose.ts", "export const loose = 1;");
    game.main("import { shared } from \"../shared/util\";\nimport { runner } from \"./runner.server\";\nimport { loose } from \"../loose\";\nprint(shared, runner, loose);");
    let diagnostics = game.facts().diagnostics;
    let found: Vec<String> =
        diagnostics.iter().map(|d| format!("{}:{} {}", game.relative(d), d.line, d.message)).collect();
    assert_eq!(
        found,
        [
            "src/client/main.client.ts:1 a module in a server container cannot be imported from outside one",
            "src/server/main.ts:2 a script is not a module and cannot be imported",
            "src/server/main.ts:3 the Rojo project maps no instance for this module",
        ]
    );
    game.main("import { shared } from \"../shared/util\";\nprint(shared);").remove("src/client/main.client.ts");
    has(
        &game.luau("server/main.luau"),
        "local shared = TS.import(script, game:GetService(\"ReplicatedStorage\"), \"TS\", \"util\").shared",
    );
}

#[test]
fn package_metadata_that_cannot_be_read_is_the_only_report_on_every_file() {
    let game = Game::new();
    game.write("src/shared/package.json", "{ not json");
    game.write("src/shared/model.ts", "export interface Model { a: string }");
    game.main("import { Subsystem, inject } from \"@trioforge/crucible\";\nimport type { Model } from \"../shared/model\";\nexport class S extends Subsystem { onStart(): void {} }\nexport type M = Model;\nexport const model = inject<Model>();\nexport const again = inject<Model>();");
    let diagnostics = game.facts().diagnostics;
    let found: Vec<String> =
        diagnostics.iter().map(|d| format!("{} {} {}", game.relative(d), d.code, d.message)).collect();
    let message = "90000 Crucible: Cannot read package metadata. Check the project's package.json files.";
    assert_eq!(found, [format!("src/server/main.ts {message}"), format!("src/shared/model.ts {message}")]);
}

#[test]
fn heritage_chains_hundreds_deep_finish_the_rules_and_the_facts() {
    const INTERFACES: usize = 300;
    const CLASSES: usize = 60;
    let mut model: Vec<String> =
        (0..INTERFACES).map(|i| format!("export interface L{i} extends L{} {{ f{i}: number }}", i + 1)).collect();
    model.push(format!("export interface L{INTERFACES} {{ leaf: string }}"));
    model.push("import { Subsystem } from \"@trioforge/crucible\";".to_owned());
    model.push(format!("export abstract class C{CLASSES} extends Subsystem {{}}"));
    model.extend((0..CLASSES).rev().map(|i| format!("export abstract class C{i} extends C{} {{ c{i} = 1; }}", i + 1)));
    let game = Game::new();
    game.write("src/shared/model.ts", &model.join("\n"));
    game.main("import { inject } from \"@trioforge/crucible\";\nimport { C0, L0 } from \"../shared/model\";\nexport const one = inject<L0>();\nexport const leaf = one.leaf + one.f0;\nexport class Leaf extends C0 {}");
    let luau = game.luau("server/main.luau");
    has(&luau, "__crucible.resolve(\"fixture-game/shared/model:L0\")");
    has(&luau, "__crucible.defineClass(Leaf, {");
}

#[test]
fn network_and_data_macros_lower_to_their_emit_calls() {
    let game = Game::new();
    game.write(
        "src/shared/messages.ts",
        "import { replicated, syncStatus, toServer } from \"@trioforge/crucible\";
export const Ping = toServer<{ a: string }>(\"X.Ping\");
export const Score = replicated<number>(\"Score\", 0);
export const status = () => syncStatus(Score);",
    );
    game.main(
        "import { config, dataStore, operationId } from \"@trioforge/crucible/server\";
export const Limit = config<number>(\"Limit\", 1);
export const Saves = dataStore<{ coins: number }>(\"Saves\", { defaults: { coins: 0 } });
export const plain = [1].map((n) => n);
export const id = operationId();",
    );
    assert_eq!(game.found(), Vec::<String>::new());
    let shared = game.luau("shared/messages.luau");
    has(
        &shared,
        "local Ping = __crucible.toServer(\"X.Ping\", __crucible.n.object({ { \"a\", __crucible.n.string } }))",
    );
    has(&shared, "local Score = __crucible.replicated(\"Score\", 0, __crucible.n.f64)");
    let server = game.luau("server/main.luau");
    has(&server, "local Limit = __crucible.config(\"Limit\", 1, __crucible.n.f64)");
    has(
        &server,
        "local Saves = __crucible.dataStore(\"Saves\", {
	defaults = {
		coins = 0,
	},
}, __crucible.n.object({ { \"coins\", __crucible.n.f64 } }))",
    );
}

#[test]
fn network_and_data_macros_used_as_values_are_cru023_at_the_reference() {
    let game = Game::new();
    game.write(
        "src/shared/values.ts",
        "import * as C from \"@trioforge/crucible\";\nimport { replicated, replicatedPerPlayer as perPlayer, receive } from \"@trioforge/crucible\";\nexport { toServer } from \"@trioforge/crucible\";\nexport const f = replicated;\nfunction use(value: unknown) {}\nuse(perPlayer);\nexport const g = C.toClient;\nexport const h = C[\"askServer\"];\nexport const i = (receive);\n",
    );
    game.main("import * as S from \"@trioforge/crucible/server\";\nexport const c = S.dataStore;\nexport type P = typeof S.playerConfig;\n");
    let found: Vec<String> = game
        .facts()
        .diagnostics
        .iter()
        .map(|d| format!("{}:{}:{} {} {}", game.relative(d), d.line, d.column, d.code, d.message))
        .collect();
    let value = |name: &str| format!("90023 CRU023: {name} is a declaration macro and cannot be used as a value.");
    assert_eq!(
        found,
        [
            format!("src/server/main.ts:2:20 {}", value("dataStore")),
            format!("src/server/main.ts:3:26 {}", value("playerConfig")),
            format!("src/shared/values.ts:4:18 {}", value("replicated")),
            format!("src/shared/values.ts:6:5 {}", value("replicatedPerPlayer")),
            format!("src/shared/values.ts:7:20 {}", value("toClient")),
            format!("src/shared/values.ts:8:18 {}", value("askServer")),
            format!("src/shared/values.ts:9:19 {}", value("receive")),
        ]
    );
}

#[test]
fn a_parenthesized_network_macro_call_lowers_like_a_plain_one() {
    let game = Game::new();
    game.main(
        "import * as S from \"@trioforge/crucible/server\";
export const Limit = (S.config)<number>(\"Limit\", 1);
",
    );
    assert_eq!(game.found(), Vec::<String>::new());
    has(&game.luau("server/main.luau"), "local Limit = __crucible.config(\"Limit\", 1, __crucible.n.f64)");
}

#[test]
fn attribute_inject_and_boot_used_as_values_compile_as_the_transformer_leaves_them() {
    let game = Game::new();
    game.main("import { attribute, inject } from \"@trioforge/crucible\";\nimport { boot } from \"@trioforge/crucible/server\";\nexport const macros = [attribute, inject, boot];\n");
    assert_eq!(game.found(), Vec::<String>::new());
    let luau = game.luau("server/main.luau");
    has(&luau, "local attribute = _0.attribute");
    has(&luau, "local inject = _0.inject");
    has(&luau, ".boot");
    has(&luau, "local macros = { attribute, inject, boot }");
}

#[test]
fn instance_classes_list_in_the_order_typescript_5_first_makes_their_types() {
    let game = Game::new();
    game.write("src/shared/parts.ts", "export type Seated = Seat;\n");
    game.main("import { CrucibleBehaviour } from \"@trioforge/crucible\";\nimport type { Seated } from \"../shared/parts\";\ninterface Gear extends Tool {}\nexport type S = Seated;\nexport class Mixed extends CrucibleBehaviour<\"Mixed\", Part | Model | Folder> {}\nexport class Later extends CrucibleBehaviour<\"Later\", BasePart | Model> {}\nexport class Shared extends CrucibleBehaviour<\"Shared\", Accessory | Seat> {}\nexport class Heard extends CrucibleBehaviour<\"Heard\", Sound | Tool> {}\nexport class Aliased extends CrucibleBehaviour<\"Aliased\", Pair> {}\nexport const fire: Fire | undefined = undefined;\ntype Pair = Sparkles | Fire;\n");
    let facts = game.facts();
    assert_eq!(facts.diagnostics, []);
    let registered = classes(&facts, "src/server/main.ts");
    let instance =
        |name: &str| -> Vec<String> { class(&registered, name).instance.iter().map(|n| n.to_string()).collect() };
    assert_eq!(instance("Mixed"), ["Part", "Model", "Folder"], "each member in the order the union names it");
    assert_eq!(instance("Later"), ["BasePart", "Model"], "making Part made its base interfaces, BasePart among them");
    assert_eq!(instance("Shared"), ["Seat", "Accessory"], "an earlier file made Seat");
    assert_eq!(
        instance("Heard"),
        ["Sound", "Tool"],
        "an interface's base is made with the interface, not where it is named"
    );
    assert_eq!(instance("Aliased"), ["Sparkles", "Fire"], "the alias made its members where the class used it");
}
