mod support;

use crucible_compiler::Target;
use support::Game;

#[test]
fn a_deeply_nested_source_compiles_through_the_library() {
    let game = Game::new();
    game.write("src/shared/sum.ts", &format!("export const sum = 0{};\n", " + 0".repeat(500)));
    let arms: String = (1..=1000).map(|n| format!(" else if (n === {n}) {{\n\t\treturn {n};\n\t}}")).collect();
    game.write(
        "src/shared/pick.ts",
        &format!(
            "export function pick(n: number): number {{\n\tif (n === 0) {{\n\t\treturn 0;\n\t}}{arms}\n\treturn -1;\n}}\n"
        ),
    );
    let config = game.dir.join("tsconfig.json");
    let project = game.project();
    let include = game.include();
    let compiled = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn_scoped(scope, || {
                crucible_compiler::compile(&config, Target::Game { project: &project, include: &include }, Vec::new())
            })
            .unwrap()
            .join()
            .unwrap()
            .unwrap_or_else(|failure| panic!("the deep source compiles:\n{failure}"))
    });
    let source = |path| &compiled.modules.iter().find(|module| module.path == path).unwrap().source;
    assert!(source("shared/sum.luau").contains(&" + 0".repeat(500)));
    assert!(source("shared/pick.luau").contains("return 1000"));
}
