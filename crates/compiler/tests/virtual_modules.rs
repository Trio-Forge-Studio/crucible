mod support;

use std::process::Command;

use crucible_compiler::{Compiled, Failure, Target, VirtualModule};
use serde_json::json;
use support::{Game, has, repo};

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

fn failure(game: &Game, modules: &[VirtualModule<'_>]) -> String {
    match compile(game, modules) {
        Ok(_) => panic!("the compile fails"),
        Err(failure) => failure.to_string(),
    }
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
    let failure = |modules: &[VirtualModule<'_>]| failure(&game, modules);
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
        "error: the virtual module `virtual:experience` has the output path server/main.luau, which collides with the \
         compiled module server/main.luau\n",
    );
    assert!(
        failure(&[experience("elsewhere/experience.luau")])
            .ends_with("error: the Rojo project maps no instance for this module\n"),
        "an unmapped virtual module cannot be imported"
    );
}

#[test]
fn an_output_path_that_differs_from_another_module_s_only_in_case_is_refused() {
    let game = game();
    assert_eq!(
        failure(&game, &[experience("server/MAIN.luau")]),
        "error: the virtual module `virtual:experience` has the output path server/MAIN.luau, which collides with the \
         compiled module server/main.luau\n",
    );
    let other = VirtualModule { specifier: "virtual:other", path: "shared/Experience.luau", source: "return {}\n" };
    for paths in
        [["shared/experience.luau", "shared/Experience.luau"], ["shared/experience.luau", "shared/experience.luau"]]
    {
        let other = VirtualModule { path: paths[1], ..other };
        assert_eq!(
            failure(&game, &[experience(paths[0]), other]),
            format!(
                "error: the virtual module `virtual:other` has the output path {}, which collides with the virtual \
                 module `virtual:experience`\n",
                paths[1]
            ),
        );
    }
}

#[test]
fn a_path_rojo_builds_as_a_script_or_folds_into_its_folder_is_refused() {
    let game = game();
    for path in [
        "shared/experience.server.luau",
        "shared/experience.client.luau",
        "shared/init.luau",
        "shared/init.server.luau",
        "shared/init.client.luau",
        "init.luau",
    ] {
        assert_eq!(
            failure(&game, &[experience(path)]),
            format!(
                "error: the virtual module `virtual:experience` has the path {path}, which Rojo builds as a script or \
                 as its folder, not as a module\n"
            ),
        );
    }
}

#[test]
#[ignore = "builds a place with Rojo and runs it in Lune, which must both be on PATH"]
fn a_place_built_with_a_virtual_module_imports_its_value_at_run_time() {
    let game = Game::new();
    game.write(
        "src/crucible-experience.d.ts",
        "declare module \"crucible:experience\" {\n\texport const coins: number;\n}\n",
    );
    game.main("import { coins } from \"crucible:experience\";\nexport const value = coins;\n");
    let experience = VirtualModule {
        specifier: "crucible:experience",
        path: "shared/experience.luau",
        source: "return { coins = 5 }\n",
    };
    let compiled = compile(&game, &[experience]).unwrap_or_else(|failure| panic!("the game compiles:\n{failure}"));
    crucible_compiler::write(&compiled.out_dir, &compiled.modules).expect("the modules are written");
    crucible_compiler::write_include(&game.include()).expect("the runtime library is written");
    game.json(
        "default.project.json",
        &json!({
            "name": "virtual",
            "tree": {
                "$className": "DataModel",
                "ReplicatedStorage": {
                    "$className": "ReplicatedStorage",
                    "rbxts_include": {
                        "$path": "include",
                        "node_modules": {
                            "$className": "Folder",
                            "@rbxts": { "$path": "node_modules/@rbxts" },
                            "@trioforge": { "$className": "Folder", "crucible": { "$path": "node_modules/@trioforge/crucible" } },
                        },
                    },
                    "TS": { "$path": "out/shared" },
                },
                "ServerScriptService": { "$className": "ServerScriptService", "TS": { "$path": "out/server" } },
            },
        }),
    );
    let place = game.dir.join("game.rbxl");
    let built = Command::new("rojo")
        .arg("build")
        .arg(game.dir.join("default.project.json"))
        .arg("--output")
        .arg(&place)
        .output()
        .expect("rojo runs");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new("lune")
        .arg("run")
        .arg(repo().join("crates/compiler/tests/virtual/run.luau"))
        .arg(&place)
        .output()
        .expect("lune runs");
    let stdout = String::from_utf8_lossy(&ran.stdout);
    assert!(ran.status.success(), "{stdout}{}", String::from_utf8_lossy(&ran.stderr));
    assert!(stdout.contains("main read 5 from crucible:experience"), "{stdout}");
}

#[test]
fn without_a_virtual_module_the_specifier_has_no_place() {
    let game = game();
    let failure = compile(&game, &[]).expect_err("the compile fails").to_string();
    assert!(failure.ends_with("error: the Rojo project maps no instance for this module\n"), "{failure}");
}
