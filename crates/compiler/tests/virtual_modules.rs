mod support;

use crucible_compiler::{Compiled, Failure, Target, VirtualModule};
use support::{Game, has};

const DECLARATION: &str = "declare module \"virtual:experience\" {\n\texport const coins: number;\n}\n";

fn experience(path: &str) -> VirtualModule<'_> {
    VirtualModule { specifier: "virtual:experience", path, source: "return { coins = 5 }\n" }
}

fn compile(game: &Game, modules: &[VirtualModule<'_>]) -> Result<Compiled, Failure> {
    let (instances, include) = (game.instances(), game.include());
    crucible_compiler::compile(
        &game.dir.join("tsconfig.json"),
        Target::Game { instances: &instances, include: &include, modules },
    )
}

fn game() -> Game {
    let game = Game::new();
    game.write("src/experience.d.ts", DECLARATION);
    game.main("import { coins } from \"virtual:experience\";\nprint(coins);\n");
    game
}

#[test]
fn a_virtual_module_is_emitted_and_imported_through_the_place() {
    let game = game();
    let modules = compile(&game, &[experience("shared/experience.luau")])
        .unwrap_or_else(|failure| panic!("the game compiles:\n{failure}"))
        .modules;
    let source = |path: &str| &modules.iter().find(|m| m.path == path).unwrap_or_else(|| panic!("{path}")).source;
    assert_eq!(source("shared/experience.luau"), "-- Compiled with crucible-compiler\nreturn { coins = 5 }\n");
    has(
        source("server/main.luau"),
        "TS.import(script, game:GetService(\"ReplicatedStorage\"), \"TS\", \"experience\")",
    );
}

#[test]
fn a_virtual_module_needs_a_mapped_path_of_its_own_inside_the_output_folder() {
    let game = game();
    let failure = |modules: &[VirtualModule<'_>]| match compile(&game, modules) {
        Ok(_) => panic!("the compile fails"),
        Err(failure) => failure.to_string(),
    };
    for path in [
        "../experience.luau",
        "/experience.luau",
        "shared/./experience.luau",
        "shared\\experience.luau",
        "shared//experience.luau",
        "C:/experience.luau",
        "shared/experience.lua",
    ] {
        assert_eq!(
            failure(&[experience(path)]),
            format!(
                "error: the virtual module `virtual:experience` needs a path inside the output folder that ends in .luau, \
                 not `{path}`\n"
            ),
        );
    }
    assert_eq!(
        failure(&[experience("server/main.luau")]),
        "error: the virtual module `virtual:experience` has the output path server/main.luau\n",
    );
    assert!(
        failure(&[experience("elsewhere/experience.luau")])
            .ends_with("error: the Rojo project maps no instance for this module\n"),
        "an unmapped virtual module cannot be imported"
    );
}

#[test]
fn without_a_virtual_module_the_specifier_has_no_place() {
    let game = game();
    let failure = compile(&game, &[]).expect_err("the compile fails").to_string();
    assert!(failure.ends_with("error: the Rojo project maps no instance for this module\n"), "{failure}");
}
