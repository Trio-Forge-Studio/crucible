mod support;
use crucible_compiler::extension::{Extension, ExtractCx, File, Kind};
use crucible_transform::Crucible;
use support::{Game, copy_declarations, game_or_package, repo};

fn rules(game: &Game) -> Vec<(usize, usize, i32)> {
    game.facts()
        .diagnostics
        .iter()
        .filter(|d| (90032..=90038).contains(&d.code))
        .map(|d| (d.line as usize, d.column as usize, d.code))
        .collect()
}

fn marked(source: &str, code: impl Fn(&str) -> i32) -> Vec<(usize, i32)> {
    source
        .lines()
        .enumerate()
        .flat_map(|(i, l)| {
            let rules = l.split_once("// fires ").map_or("", |(_, rules)| rules);
            rules.split_whitespace().map(|rule| (i + 1, code(rule))).collect::<Vec<_>>()
        })
        .collect()
}

fn lines(found: Vec<(usize, usize, i32)>) -> Vec<(usize, i32)> {
    found.into_iter().map(|(l, _, c)| (l, c)).collect()
}

fn with_charm() -> Game {
    let game = Game::new();
    let charm = repo().join("packages/crucible/node_modules/@rbxts/charm");
    copy_declarations(&charm, &game.dir.join("node_modules/@rbxts/charm"));
    game
}

#[test]
fn fd_crucible_lifetime_rules_follow_abstract_bases() {
    let game = Game::new();
    let source = include_str!("transform-fixtures/typed-lint/classes.ts.txt");
    game.write("src/server/bases.ts", include_str!("transform-fixtures/typed-lint/bases.ts.txt"));
    game.main(source);
    let expected = marked(source, |rule| if rule == "frame-hooks" { 90035 } else { 90036 });
    assert_eq!(lines(rules(&game)), expected);
}

#[test]
fn charm_lifetime_calls_in_game_code_report_cru038_at_the_import_and_the_namespace_read() {
    let game = with_charm();
    let source = include_str!("transform-fixtures/typed-lint/charm.ts.txt");
    game.write("src/shared/charm.ts", source);
    game.main("import { boot } from \"@trioforge/crucible/server\";\nimport { lifetimes } from \"../shared/charm\";\nprint(lifetimes, boot);\n");
    let found = game.facts();
    let reports: Vec<_> = found.diagnostics.iter().filter(|d| d.code == 90038).collect();
    assert_eq!(
        reports.iter().map(|d| (d.line as usize, d.column as usize)).collect::<Vec<_>>(),
        [(1, 33), (1, 41), (1, 64), (4, 10), (9, 1), (11, 54)],
        "each named import of effect, effectScope or onCleanup, the re-export, and each namespace read; atom, \
         computed, batch and type-only imports are fine"
    );
    assert_eq!(
        reports[0].message,
        "CRU038: effect from @rbxts/charm belongs to whatever scope is running when it is called, not to the object \
         whose code called it. Use effect from @trioforge/crucible, which ends with its owner."
    );
    assert!(reports[1].message.contains("Use scope from @trioforge/crucible"), "{}", reports[1].message);
    assert!(reports[2].message.starts_with("CRU038: onCleanup from @rbxts/charm"), "{}", reports[2].message);
    assert_eq!(lines(rules(&game)), marked(source, |_| 90038), "the fixture marks each line that fires");
}

#[test]
fn charm_imports_inside_installed_packages_raise_nothing() {
    let game = with_charm();
    let vide_charm = "node_modules/@rbxts/vide-charm";
    game.json(
        &format!("{vide_charm}/package.json"),
        &serde_json::json!({ "name": "@rbxts/vide-charm", "types": "index.d.ts" }),
    );
    game.write(
        &format!("{vide_charm}/index.d.ts"),
        "import { effect, onCleanup } from \"@rbxts/charm\";\nexport declare function useAtom<T>(value: () => T): () => T;\nexport { effect, onCleanup };\n",
    );
    let watch = "node_modules/@rbxts/watch";
    game.json(&format!("{watch}/package.json"), &serde_json::json!({ "name": "@rbxts/watch", "types": "index.ts" }));
    game.write(
        &format!("{watch}/index.ts"),
        "import { effect } from \"@rbxts/charm\";\nexport function watch(read: () => unknown) {\n\treturn effect(() => {\n\t\tread();\n\t});\n}\n",
    );
    game.main("import { boot } from \"@trioforge/crucible/server\";\nimport { useAtom } from \"@rbxts/vide-charm\";\nimport { watch } from \"@rbxts/watch\";\nimport { atom } from \"@rbxts/charm\";\nexport const value = useAtom(atom(0));\nexport const stop = watch(value);\nprint(boot);\n");
    let facts = game.facts();
    assert!(
        facts.files.iter().any(|f| f.path.ends_with("/node_modules/@rbxts/watch/index.ts")),
        "the dependency's TypeScript source is extracted, so the exemption is what keeps it clean"
    );
    assert_eq!(rules(&game), []);
}

/// Reports TS90035 at every `.Connect(..)` call, the number CRU035 renders to.
struct Echo;

impl Extension for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: File) {
        let mut nodes = vec![file.root()];
        while let Some(n) = nodes.pop() {
            if n.kind() == Kind::CallExpression
                && n.expression().kind() == Kind::PropertyAccessExpression
                && n.expression().name().is_some_and(|name| name.text() == "Connect")
            {
                cx.report(n, 90035, "echo: a connection");
            }
            n.for_each_child(&mut |child| nodes.push(child));
        }
    }
}

#[test]
fn another_extension_s_report_where_a_crucible_rule_reports_with_the_same_number_is_kept() {
    let game = Game::new();
    game.main(
        "import { Subsystem } from \"@trioforge/crucible\";\nconst RunService = game.GetService(\"RunService\");\n\
         declare function step(): void;\nexport class Direct extends Subsystem { start() { RunService.Heartbeat.Connect(step); } }\n",
    );
    let (instances, include) = (game.instances(), game.include());
    let facts = crucible_compiler::analyze_with(
        &game.dir.join("tsconfig.json"),
        game_or_package(instances.as_ref(), &include),
        &mut [&mut Crucible::default(), &mut Echo],
    )
    .unwrap_or_else(|d| panic!("the game type-checks: {d:?}"));
    let found: Vec<(u32, u32, i32, &str)> =
        facts.diagnostics.iter().map(|d| (d.line, d.column, d.code, d.message.split(':').next().unwrap())).collect();
    assert_eq!(found, [(4, 51, 90035, "CRU035"), (4, 51, 90035, "echo")]);
}

#[test]
fn charm_lifetime_calls_through_brackets_destructuring_and_barrels_report_cru038_where_game_code_takes_them() {
    let game = with_charm();
    let lifecycle = "node_modules/@rbxts/lifecycle";
    game.json(
        &format!("{lifecycle}/package.json"),
        &serde_json::json!({ "name": "@rbxts/lifecycle", "types": "index.d.ts" }),
    );
    game.write(&format!("{lifecycle}/index.d.ts"), "import { effect } from \"@rbxts/charm\";\nexport { effect };\n");
    game.write(
        "src/server/uses.ts",
        "import { Subsystem } from \"@trioforge/crucible\";\nexport const base = Subsystem;\n",
    );
    game.write("src/shared/bridge.ts", "export * from \"@rbxts/charm\";\n");
    game.write("src/shared/named.ts", "export { effect } from \"@rbxts/charm\";\n");
    game.write("src/server/reads.ts", "import charm from \"@rbxts/charm\";\ncharm.effect(() => {});\n");
    game.write("src/server/required.ts", "import charm = require(\"@rbxts/charm\");\ncharm.effect(() => {});\n");
    game.main(
        "import * as charm from \"@rbxts/charm\";\n\
         import { effect } from \"../shared/bridge\";\n\
         import * as bridge from \"../shared/bridge\";\n\
         import { effect as viaNamed } from \"../shared/named\";\n\
         import { effect as viaPackage } from \"@rbxts/lifecycle\";\n\
         import { atom, computed } from \"../shared/bridge\";\n\
         charm[\"effect\"](() => {});\n\
         const { effectScope } = charm;\n\
         const { onCleanup: later } = bridge;\n\
         effect(() => {});\n\
         bridge.effect(() => {});\n\
         bridge[\"onCleanup\"](() => {});\n\
         viaNamed(() => {});\n\
         viaPackage(() => {});\n\
         const { atom: makeAtom, computed: derive } = charm;\n\
         export type Read = typeof charm.effect;\n\
         export const values = [effectScope, later, makeAtom(0), derive, atom, computed];\n",
    );
    let found = game.facts();
    let reports: Vec<_> = found
        .diagnostics
        .iter()
        .filter(|d| d.code == 90038)
        .map(|d| (game.relative(d), d.line, d.column, d.message.split(' ').nth(1).unwrap_or("")))
        .collect();
    let at = |file: &str, line: u32, column: u32, name| (file.to_owned(), line, column, name);
    assert_eq!(
        reports,
        [
            at("src/server/main.ts", 2, 10, "effect"),
            at("src/server/main.ts", 5, 10, "effect"),
            at("src/server/main.ts", 7, 1, "effect"),
            at("src/server/main.ts", 8, 9, "effectScope"),
            at("src/server/main.ts", 9, 9, "onCleanup"),
            at("src/server/main.ts", 11, 1, "effect"),
            at("src/server/main.ts", 12, 1, "onCleanup"),
            at("src/server/reads.ts", 2, 1, "effect"),
            at("src/server/required.ts", 2, 1, "effect"),
            at("src/shared/named.ts", 1, 10, "effect"),
        ],
        "the barrel's export * and the import of named.ts's re-export are not reported, because the import from the \
         barrel and the re-export are"
    );
}

#[test]
fn cru038_applies_wherever_crucible_is_installed_even_in_a_program_that_never_imports_it() {
    let game = with_charm();
    game.main("import { effect } from \"@rbxts/charm\";\neffect(() => {});\n");
    assert_eq!(lines(rules(&game)), [(1, 90038)]);

    std::fs::remove_dir_all(game.dir.join("node_modules/@trioforge/crucible")).unwrap();
    game.remove("default.project.json");
    assert_eq!(rules(&game), [], "a program without Crucible installed is exempt");
}

#[test]
fn cru038_reads_the_keys_the_checker_knows_and_destructuring_assignments() {
    let game = with_charm();
    game.main(
        "import * as charm from \"@rbxts/charm\";\n\
         charm[(\"effect\")](() => {});\n\
         const k: \"effect\" = \"effect\";\n\
         charm[k](() => {});\n\
         const u = math.random() > 0.5 ? \"effect\" : \"onCleanup\";\n\
         charm[u](() => {});\n\
         const key = \"effect\" as const;\n\
         const { [key]: run } = charm;\n\
         run(() => {});\n\
         let effect: typeof charm.effect;\n\
         ({ effect } = charm);\n\
         effect(() => {});\n\
         let later: typeof charm.onCleanup;\n\
         [{ onCleanup: later }] = [charm];\n\
         const [{ effectScope }] = [charm];\n\
         let made: typeof charm.atom;\n\
         ({ atom: made } = charm);\n\
         const kept = charm[\"computed\"];\n\
         export const values = [later, effectScope, made, kept];\n",
    );
    let found: Vec<_> =
        game.facts().diagnostics.iter().filter(|d| d.code == 90038).map(|d| (d.line, d.column)).collect();
    assert_eq!(found, [(2, 1), (4, 1), (6, 1), (8, 9), (11, 4), (14, 4), (15, 10)]);
}

#[test]
fn cru038_reports_a_value_whose_type_reuses_charm_s_lifetime_functions_and_not_one_with_its_own_function_type() {
    let game = with_charm();
    game.main(
        "import type * as charm from \"@rbxts/charm\";\n\
         const safe: Pick<typeof charm, \"effect\" | \"onCleanup\"> = { effect: () => () => {}, onCleanup: () => {} };\n\
         safe.effect(() => {});\n\
         safe[\"onCleanup\"](() => {});\n\
         const { effect } = safe;\n\
         let later: typeof charm.onCleanup;\n\
         ({ onCleanup: later } = safe);\n\
         function run({ effectScope }: typeof charm) {\n\treturn effectScope;\n}\n\
         type Run = (run: () => void) => () => void;\n\
         const own: { effect: Run } = { effect: () => () => {} };\n\
         own.effect(() => {});\n\
         export const values = [effect, later, run];\n",
    );
    let found: Vec<_> =
        game.facts().diagnostics.iter().filter(|d| d.code == 90038).map(|d| (d.line, d.column)).collect();
    assert_eq!(
        found,
        [(3, 1), (4, 1), (5, 9), (7, 4), (8, 16)],
        "accepted: the checker resolves each of these to Charm's declaration, which a lint cannot tell from Charm's \
         own value, and the import type and the annotations are not reported; a function type of the game's own is \
         fine"
    );
}

#[test]
fn cru038_reports_charm_s_lifetime_calls_however_a_value_carries_charm() {
    const FORMS: [(&str, u32); 16] = [
        ("const [copy] = [charm]; copy.effect(() => {});", 25),
        ("(charm as typeof charm).effect(() => {});", 1),
        ("const {effect} = charm as typeof charm; effect(() => {});", 8),
        (
            "function assign<T>(target: {}, source: T): T { return source; } const copy = assign({}, charm); \
             copy.effect(() => {});",
            97,
        ),
        ("class Holder { c = charm; } new Holder().c.effect(() => {});", 29),
        ("(math.random() > 0.5 ? charm : charm).effect(() => {});", 1),
        ("function identity<T>(x: T): T { return x; } identity(charm).effect(() => {});", 45),
        ("function use<T extends typeof charm>(x:T) { x.effect(() => {}); } use(charm);", 45),
        ("let c = charm; c = charm; c.effect(() => {});", 27),
        ("charm!.effect(() => {});", 1),
        ("(charm satisfies typeof charm).effect(() => {});", 1),
        ("const copy = {...charm}; copy.effect(() => {});", 26),
        ("const c = charm; c.effect(() => {});", 18),
        ("const wrapped = {nested: charm}; wrapped.nested.effect(() => {});", 34),
        ("const run = ({ effect } = charm) => effect(() => print(\"parameter\")); run();", 16),
        (
            "const wrapped = { nested: charm }; const { nested: { effect } } = wrapped; effect(() => print(\"nested\"));",
            54,
        ),
    ];
    let game = with_charm();
    game.write("src/shared/local.ts", "export function effect(run: () => void) {\n\trun();\n}\n");
    for (i, (form, _)) in FORMS.iter().enumerate() {
        game.write(
            &format!("src/server/form{i:02}.ts"),
            &format!("import * as charm from \"@rbxts/charm\";\n{form}\n"),
        );
    }
    game.main(
        "import * as charm from \"@rbxts/charm\";\n\
         import { atom, computed, batch } from \"@rbxts/charm\";\n\
         import * as local from \"../shared/local\";\n\
         local.effect(() => {});\n\
         const count = charm.atom(0);\n\
         const doubled = charm[\"computed\"](() => 2);\n\
         charm.batch(() => {});\n\
         const { atom: make, computed: derive, batch: group } = charm;\n\
         batch(() => atom(0));\n\
         function effect(run: () => void) {\n\trun();\n}\n\
         effect(() => print(doubled()));\n\
         const unrelated = { effect: (run: () => void) => run(), onCleanup: () => {} };\n\
         unrelated.effect(() => {});\n\
         unrelated[\"onCleanup\"]();\n\
         const { effectScope: scope = () => {} } = { effectScope: undefined as (() => void) | undefined };\n\
         const { effect: also } = unrelated;\n\
         export const values = [count, make, derive, group, also, scope, computed];\n",
    );
    let found: Vec<_> = game
        .facts()
        .diagnostics
        .iter()
        .filter(|d| (90032..=90038).contains(&d.code))
        .map(|d| (game.relative(d), d.line, d.column, d.code))
        .collect();
    let expected: Vec<_> = FORMS
        .iter()
        .enumerate()
        .map(|(i, &(_, column))| (format!("src/server/form{i:02}.ts"), 2, column, 90038))
        .collect();
    assert_eq!(
        found, expected,
        "each form reports once, at its read or destructured name; atom, computed, batch, a local effect function and \
         objects of the game's own in main.ts are fine"
    );
}

#[test]
fn cru038_applies_where_crucible_resolves_through_paths() {
    let game = with_charm();
    std::fs::create_dir_all(game.dir.join("vendor")).unwrap();
    std::fs::rename(game.dir.join("node_modules/@trioforge/crucible"), game.dir.join("vendor/crucible")).unwrap();
    game.config(|config| {
        config["compilerOptions"]["paths"] = serde_json::json!({
            "@trioforge/crucible": ["./vendor/crucible/out/index.d.ts"],
            "@trioforge/crucible/*": ["./vendor/crucible/out/*/index.d.ts"],
        });
    });
    game.main(
        "import { Subsystem } from \"@trioforge/crucible\";\n\
         import { effect } from \"@rbxts/charm\";\n\
         export const used = [Subsystem, effect];\n",
    );
    assert_eq!(lines(rules(&game)), [(2, 90038)]);
}
