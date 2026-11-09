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
    game.main("import { boot } from \"@trioforge/crucible/server\";\nimport { useAtom } from \"@rbxts/vide-charm\";\nimport { atom } from \"@rbxts/charm\";\nexport const value = useAtom(atom(0));\nprint(boot);\n");
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
