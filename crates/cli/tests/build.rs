#[path = "support/game.rs"]
mod game;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

use game::{repo, slashes};
use serde_json::{Value, json};

fn vendored(name: &str) -> Vec<u8> {
    fs::read(repo().join("crates/compiler/roblox-ts").join(name)).unwrap()
}

struct Game {
    dir: tempfile::TempDir,
}

impl Game {
    fn new() -> Self {
        let game = Self { dir: tempfile::tempdir().unwrap() };
        game.json("tsconfig.json", &game::ts7_config());
        game
    }

    fn template_layout() -> Self {
        let game = Self::new();
        game.json(
            "default.project.json",
            &json!({
                "name": "single",
                "tree": {
                    "$className": "DataModel",
                    "ReplicatedStorage": {
                        "$className": "ReplicatedStorage",
                        "rbxts_include": { "$path": "include" },
                        "TS": { "$path": "out/shared" },
                    },
                    "ServerScriptService": { "$className": "ServerScriptService", "TS": { "$path": "out/server" } },
                },
            }),
        );
        game.write("src/shared/greeting.ts", "export const greet = (name: string) => `hello from ${name}`;\n");
        game.write(
            "src/server/main.server.ts",
            "import { greet } from \"../shared/greeting\";\nprint(greet(\"start\"));\n",
        );
        game
    }

    fn fd_layout() -> Self {
        let game = Self::new();
        game.config(|c| {
            let options = c["compilerOptions"].as_object_mut().unwrap();
            options.remove("rootDir");
            options.remove("outDir");
        });
        let mut manifest = String::new();
        for (place, folder) in [("job", "Job"), ("warehouse", "Warehouse")] {
            game.json(
                &format!("tsconfig.{place}.json"),
                &json!({
                    "extends": "./tsconfig.json",
                    "compilerOptions": {
                        "rootDir": "src",
                        "rootDirs": ["src/core", format!("src/{place}")],
                        "outDir": format!("places/{place}/out"),
                    },
                    "include": ["src/core", format!("src/{place}")],
                }),
            );
            game.json(
                &format!("{place}.generated.project.json"),
                &json!({
                    "name": place,
                    "tree": {
                        "$className": "DataModel",
                        "ReplicatedStorage": {
                            "$className": "ReplicatedStorage",
                            "rbxts_include": { "$path": { "optional": format!("places/{place}/include") } },
                            folder: { "$className": "Folder", "core": { "$path": format!("places/{place}/out/core") } },
                        },
                        "ServerScriptService": {
                            "$className": "ServerScriptService",
                            "TS": { "$path": format!("places/{place}/out/{place}") },
                        },
                    },
                }),
            );
            game.json(
                &format!("{place}.project.json"),
                &json!({ "name": place, "tree": { "$className": "DataModel", "Workspace": { "$className": "Workspace" } } }),
            );
            game.write(
                &format!("src/{place}/main.server.ts"),
                &format!("import {{ greet }} from \"../core/greeting\";\nprint(greet(\"{place}\"));\n"),
            );
            manifest += &format!(
                "[places.{place}]\ntsconfig = \"tsconfig.{place}.json\"\nproject = \"{place}.generated.project.json\"\ninclude = \"places/{place}/include\"\n"
            );
        }
        game.write("src/core/greeting.ts", "export const greet = (name: string) => `hello from ${name}`;\n");
        game.write("crucible.toml", &manifest);
        game
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn write(&self, relative: &str, text: &str) -> &Self {
        let file = self.path(relative);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
        self
    }

    fn json(&self, relative: &str, value: &Value) -> &Self {
        self.write(relative, &serde_json::to_string_pretty(value).unwrap())
    }

    fn config(&self, edit: impl FnOnce(&mut Value)) -> &Self {
        let mut config: Value = serde_json::from_str(&self.read("tsconfig.json")).unwrap();
        edit(&mut config);
        self.json("tsconfig.json", &config)
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path(relative)).unwrap_or_else(|_| panic!("{relative} exists"))
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn build(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_crucible"))
            .arg("build")
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .unwrap()
    }

    fn snapshot(&self, folders: &[&str]) -> BTreeMap<String, (Vec<u8>, SystemTime)> {
        let mut files = BTreeMap::new();
        let mut pending: Vec<PathBuf> = folders.iter().map(|folder| self.path(folder)).collect();
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).unwrap_or_else(|_| panic!("{} exists", dir.display())) {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let modified = fs::metadata(&path).unwrap().modified().unwrap();
                    files.insert(
                        slashes(path.strip_prefix(self.dir.path()).unwrap()),
                        (fs::read(&path).unwrap(), modified),
                    );
                }
            }
        }
        files
    }
}

const TWO_PLACES: &str =
    "[places.start]\nproject = \"default.project.json\"\n[places.arena]\nproject = \"arena.project.json\"\n";

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_place(path: &Path) {
    let bytes = fs::read(path).unwrap_or_else(|_| panic!("{} was built", path.display()));
    assert!(bytes.starts_with(b"<roblox!"), "{} is a binary place file", path.display());
}

fn printed(output: &Output) -> Vec<String> {
    text(&output.stdout).lines().map(|line| line.replace('\\', "/")).collect()
}

#[test]
fn without_manifest_builds_start_from_the_root_files() {
    let game = Game::template_layout();
    let out = game.build(&[]);
    let stderr = text(&out.stderr);
    assert_eq!(code(&out), 0, "{stderr}");
    assert_eq!(printed(&out), ["build/start.rbxl"], "stdout holds only place paths");
    assert_place(&game.root().join("build/start.rbxl"));
    assert!(
        stderr.contains("crucible build: wrote 2 of 2 modules to out") && stderr.contains("Built project"),
        "{stderr}"
    );
    assert!(game.read("out/server/main.server.luau").starts_with("-- Compiled with crucible-compiler\n"));
    for name in ["RuntimeLib.lua", "Promise.lua"] {
        assert_eq!(fs::read(game.path("include").join(name)).unwrap(), vendored(name), "{name} is roblox-ts's copy");
    }
    assert!(!game.path("out/transformer.sha256").exists(), "the build keeps no transformer receipt");
}

#[test]
fn each_place_compiles_its_own_tsconfig_against_its_own_project() {
    let game = Game::fd_layout();
    let out = game.build(&[]);
    let stderr = text(&out.stderr);
    assert_eq!(code(&out), 0, "{stderr}");
    assert_eq!(printed(&out), ["build/job.rbxl", "build/warehouse.rbxl"]);
    for (place, other, folder) in [("job", "warehouse", "Job"), ("warehouse", "job", "Warehouse")] {
        assert_place(&game.root().join(format!("build/{place}.rbxl")));
        assert!(stderr.contains(&format!("wrote 2 of 2 modules to places/{place}/out")), "{stderr}");
        let main = game.read(&format!("places/{place}/out/{place}/main.server.luau"));
        assert!(
            main.contains(&format!("\"{folder}\", \"core\", \"greeting\"")),
            "{place} requires core through its project:\n{main}"
        );
        assert!(game.path(&format!("places/{place}/out/core/greeting.luau")).is_file());
        assert!(!game.path(&format!("places/{place}/out/{other}")).exists(), "{place} compiles none of {other}'s code");
        assert_eq!(
            fs::read(game.path(&format!("places/{place}/include/RuntimeLib.lua"))).unwrap(),
            vendored("RuntimeLib.lua")
        );
    }
    assert!(!game.path("out").exists() && !game.path("include").exists(), "nothing is written at the root");

    let before = game.snapshot(&["places"]);
    let again = game.build(&[]);
    assert_eq!(code(&again), 0, "{}", text(&again.stderr));
    assert!(text(&again.stderr).contains("wrote 0 of 2 modules to places/job/out"), "{}", text(&again.stderr));
    assert_eq!(game.snapshot(&["places"]), before, "an unchanged rebuild rewrites no output or include file");

    let xml = game.build(&["--place", "job", "--output", "build/job.rbxlx"]);
    assert_eq!(code(&xml), 0, "{}", text(&xml.stderr));
    let place = game.read("build/job.rbxlx").replace("&quot;", "\"");
    assert!(place.contains("print((greet(\"job\")))"), "the job place file holds job's script");
    for module in ["RuntimeLib", "Promise"] {
        assert!(place.contains(&format!("<string name=\"Name\">{module}</string>")), "the place has {module}");
    }
    assert!(!place.contains("greet(\"warehouse\")"), "the job place file holds none of warehouse's code");
}

#[test]
fn a_type_error_prints_positions_and_writes_nothing() {
    let game = Game::fd_layout();
    assert_eq!(code(&game.build(&[])), 0);
    let before = game.snapshot(&["places", "build"]);
    game.write(
        "src/job/main.server.ts",
        "import { greet } from \"../core/greeting\";\nprint(greet(\"job, edited\"));\n",
    );
    game.write("src/warehouse/main.server.ts", "export {};\nconst count: number = \"many\";\n");
    let out = game.build(&[]);
    let stderr = text(&out.stderr);
    assert_eq!(code(&out), 1, "{stderr}");
    assert!(
        stderr.contains(
            "src/warehouse/main.server.ts(2,7): error TS2322: Type 'string' is not assignable to type 'number'."
        ),
        "the error keeps its root-relative file position: {stderr}"
    );
    assert!(stderr.contains("crucible build: place warehouse: the compile failed, so nothing was written"), "{stderr}");
    assert!(out.stdout.is_empty());
    assert_eq!(game.snapshot(&["places", "build"]), before, "job compiled first, but its output waits for every place");

    let fresh = Game::fd_layout();
    fresh.write("src/core/greeting.ts", "export const greet = (name: string): number => name;\n");
    let out = fresh.build(&[]);
    let stderr = text(&out.stderr);
    assert_eq!(code(&out), 1);
    assert_eq!(stderr.matches("src/core/greeting.ts(1,").count(), 1, "a core error prints once: {stderr}");
    assert!(stderr.contains("place job: the compile failed"), "{stderr}");
    for folder in ["places/job/out", "places/job/include", "places/warehouse/out", "build"] {
        assert!(!fresh.path(folder).exists(), "a failed first build creates no {folder}");
    }
}

#[test]
fn place_filter_compiles_only_that_place_and_output_renames_the_file() {
    let game = Game::fd_layout();
    let out = game.build(&["--place", "warehouse", "--output", "custom/warehouse-test.rbxl"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert_eq!(printed(&out), ["custom/warehouse-test.rbxl"]);
    assert_place(&game.root().join("custom/warehouse-test.rbxl"));
    assert!(!game.path("build").exists() && !game.path("places/job").exists(), "no other place was compiled or built");
}

#[test]
fn places_sharing_a_tsconfig_compile_once_or_refuse_to_mix_their_code() {
    let game = Game::template_layout();
    game.write("crucible.toml", &TWO_PLACES.replace("arena.project.json", "default.project.json"));
    let out = game.build(&[]);
    let stderr = text(&out.stderr);
    assert_eq!(code(&out), 0, "{stderr}");
    assert_eq!(printed(&out), ["build/arena.rbxl", "build/start.rbxl"]);
    assert_eq!(stderr.matches("modules to out").count(), 1, "one write for the shared outDir: {stderr}");

    game.write("crucible.toml", TWO_PLACES);
    let mut arena: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    let shared = arena["tree"]["ReplicatedStorage"]["TS"].take();
    arena["tree"]["ReplicatedStorage"].as_object_mut().unwrap().remove("TS");
    arena["tree"]["ReplicatedStorage"]["Shared"] = shared;
    game.json("arena.project.json", &arena);
    fs::remove_dir_all(game.path("build")).unwrap();
    let before = game.snapshot(&["out"]);
    for args in [&[][..], &["--place", "start"][..], &["--place", "arena"][..]] {
        let out = game.build(args);
        let stderr = text(&out.stderr);
        assert_eq!(code(&out), 1, "{args:?}: {stderr}");
        assert!(
            stderr.contains(
                "places arena and start compile different code into out. Give each a tsconfig with its own outDir"
            ),
            "{args:?}: {stderr}"
        );
        assert_eq!(game.snapshot(&["out"]), before, "{args:?} leaves the shared outDir alone");
        assert!(!game.path("build").exists(), "{args:?}");
    }
}

#[test]
fn separate_tsconfigs_refuse_one_out_dir_in_any_case_even_when_one_place_builds() {
    let game = Game::template_layout();
    game.write("src/server/start-only.ts", "export const only = 1;\n");
    let mut arena = game::ts7_config();
    arena["exclude"] = json!(["src/server/start-only.ts"]);
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    project["tree"]["ReplicatedStorage"]["TS"]["$path"] = "OUT/shared".into();
    project["tree"]["ServerScriptService"]["TS"]["$path"] = "OUT/server".into();
    game.json("arena.project.json", &project);
    game.write(
        "crucible.toml",
        "[places.start]\nproject = \"default.project.json\"\n[places.arena]\nproject = \"arena.project.json\"\ntsconfig = \"tsconfig.arena.json\"\n",
    );
    for out_dir in ["OUT", "out/arena"] {
        arena["compilerOptions"]["outDir"] = out_dir.into();
        game.json("tsconfig.arena.json", &arena);
        for args in [&[][..], &["--place", "start"][..]] {
            let out = game.build(args);
            let stderr = text(&out.stderr);
            assert_eq!(code(&out), 1, "{out_dir} {args:?}: {stderr}");
            let refused = if out_dir == "OUT" {
                "places arena and start compile different code into OUT. Give each a tsconfig with its own outDir"
            } else {
                "places arena and start compile into out/arena and out, one inside the other"
            };
            assert!(stderr.contains(refused), "{out_dir} {args:?}: {stderr}");
            assert!(!game.path("out").exists() && !game.path("OUT").exists(), "nothing compiled or written");
        }
    }

    arena["compilerOptions"]["outDir"] = "out-arena".into();
    game.json("tsconfig.arena.json", &arena);
    project["tree"]["ReplicatedStorage"]["TS"]["$path"] = "out-arena/shared".into();
    project["tree"]["ServerScriptService"]["TS"]["$path"] = "out-arena/server".into();
    game.json("arena.project.json", &project);
    let out = game.build(&["--place", "start"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let out = game.build(&["--place", "arena"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(game.path("out/server/start-only.luau").is_file(), "arena's build leaves start's modules alone");
}

#[test]
fn an_unreadable_tsconfig_of_a_place_left_out_stops_the_build_before_it_writes() {
    let game = Game::template_layout();
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    let shared = project["tree"]["ReplicatedStorage"]["TS"].take();
    project["tree"]["ReplicatedStorage"].as_object_mut().unwrap().remove("TS");
    project["tree"]["ReplicatedStorage"]["Shared"] = shared;
    game.json("beta.project.json", &project);
    game.json("tsconfig.beta.json", &game::ts7_config());
    game.write("crucible.toml", "[places.beta]\nproject = \"beta.project.json\"\ntsconfig = \"tsconfig.beta.json\"\n");
    let out = game.build(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let before = game.snapshot(&["out"]);

    game.write(
        "crucible.toml",
        "[places.start]\nproject = \"default.project.json\"\n[places.beta]\nproject = \"beta.project.json\"\ntsconfig = \"tsconfig.beta.json\"\n",
    );
    let mut no_out_dir = game::ts7_config();
    no_out_dir["compilerOptions"].as_object_mut().unwrap().remove("outDir");
    let extends = |base: &str| json!({ "extends": base }).to_string();
    for (beta, base, named) in [
        (Some("{ invalid json".to_owned()), None, "tsconfig.beta.json"),
        (None, None, "tsconfig.beta.json"),
        (Some(extends("./missing-base.json")), None, "missing-base.json"),
        (Some(extends("./base.json")), Some("{ invalid json"), "base.json"),
        (Some(no_out_dir.to_string()), None, "tsconfig.beta.json"),
    ] {
        let _ = fs::remove_file(game.path("tsconfig.beta.json"));
        let _ = fs::remove_file(game.path("base.json"));
        if let Some(beta) = &beta {
            game.write("tsconfig.beta.json", beta);
        }
        if let Some(base) = base {
            game.write("base.json", base);
        }
        let out = game.build(&["--place", "start"]);
        let stderr = text(&out.stderr);
        assert_eq!(code(&out), 1, "{beta:?}: {stderr}");
        assert!(stderr.contains("place beta") && stderr.contains(named), "{beta:?} names place and file: {stderr}");
        assert_eq!(game.snapshot(&["out"]), before, "{beta:?} leaves beta's output alone");
        assert!(out.stdout.is_empty() && !game.path("build/start.rbxl").exists(), "{beta:?}");
    }
}

#[test]
fn a_deeply_nested_source_compiles() {
    let game = Game::template_layout();
    game.write("src/shared/sum.ts", &format!("export const sum = 0{};\n", " + 0".repeat(500)));
    let arms: String = (1..=1000).map(|n| format!(" else if (n === {n}) {{\n\t\treturn {n};\n\t}}")).collect();
    game.write(
        "src/shared/pick.ts",
        &format!(
            "export function pick(n: number): number {{\n\tif (n === 0) {{\n\t\treturn 0;\n\t}}{arms}\n\treturn -1;\n}}\n"
        ),
    );
    let out = game.build(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(game.read("out/shared/sum.luau").contains(&" + 0".repeat(500)));
    assert!(game.read("out/shared/pick.luau").contains("1000"));
}

#[test]
fn usage_errors_exit_2_before_compiling() {
    let game = Game::template_layout();
    game.write("crucible.toml", TWO_PLACES);
    for (args, message) in [
        (&["--place", "lobby"][..], "unknown place lobby. Places: arena, start"),
        (&["--output", "one.rbxl"][..], "--output needs exactly one place"),
        (&["--place", "start", "--place", "arena", "--output", "one.rbxl"][..], "--output needs exactly one place"),
    ] {
        let out = game.build(args);
        assert_eq!(code(&out), 2, "{args:?}");
        assert!(text(&out.stderr).contains(message), "{args:?}: {}", text(&out.stderr));
        assert!(out.stdout.is_empty());
    }
    for manifest in [
        "[places.start]\nprojet = \"default.project.json\"\n",
        "[places.start]\nproject = \"default.project.json\"\nincludes = \"include\"\n",
    ] {
        game.write("crucible.toml", manifest);
        let out = game.build(&[]);
        assert_eq!(code(&out), 2);
        assert!(text(&out.stderr).contains("crucible.toml") && text(&out.stderr).contains("unknown field"));
    }
    assert!(!game.path("out").exists() && !game.path("include").exists(), "no usage error compiles");
}

#[test]
fn a_missing_tsconfig_or_project_exits_1_naming_it() {
    for (manifest, named) in [
        (
            "[places.start]\nproject = \"default.project.json\"\ntsconfig = \"nope.json\"\n",
            "crucible build: place start: nope.json cannot be read",
        ),
        ("[places.start]\nproject = \"missing.project.json\"\n", "missing.project.json"),
    ] {
        let game = Game::template_layout();
        game.write("crucible.toml", manifest);
        let out = game.build(&[]);
        let stderr = text(&out.stderr);
        assert_eq!(code(&out), 1, "{stderr}");
        assert!(stderr.contains(named) && stderr.contains("crucible build: place start: "), "{stderr}");
        assert!(out.stdout.is_empty() && !game.path("build").exists());
    }
}

#[test]
fn a_project_without_the_include_folder_names_both() {
    let game = Game::template_layout();
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    project["tree"]["ReplicatedStorage"].as_object_mut().unwrap().remove("rbxts_include");
    game.json("default.project.json", &project);
    let out = game.build(&[]);
    let stderr = text(&out.stderr);
    assert_eq!(code(&out), 1, "{stderr}");
    assert!(
        stderr.contains("default.project.json maps no folder for include, where roblox-ts's runtime library goes"),
        "{stderr}"
    );
}

#[test]
fn rojo_failure_exits_1_with_rojo_diagnostics() {
    let game = Game::template_layout();
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    project["tree"]["Workspace"] = json!({ "$className": "Workspace", "Map": { "$path": "missing-map" } });
    game.json("default.project.json", &project);
    let out = game.build(&[]);
    assert_eq!(code(&out), 1);
    let stderr = text(&out.stderr);
    assert!(stderr.contains("File $path: missing-map") && stderr.contains("crucible build: place start:"), "{stderr}");
    assert!(out.stdout.is_empty());
}

#[test]
fn a_failed_rojo_build_keeps_the_last_place_whole() {
    let game = Game::template_layout();
    let out = game.build(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("Built project to start.rbxl"), "Rojo's line names the place file");
    let place = fs::read(game.path("build/start.rbxl")).unwrap();
    fs::write(crucible_compiler::temporary_path(&game.path("build/start.rbxl"), "rbxl"), "half a place").unwrap();
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    project["tree"]["Workspace"] = json!({ "$className": "Workspace", "Map": { "$path": "missing-map" } });
    game.json("default.project.json", &project);
    assert_eq!(code(&game.build(&[])), 1);
    assert_eq!(fs::read(game.path("build/start.rbxl")).unwrap(), place, "the last place stays whole");
    assert_eq!(listed(&game.path("build")), ["start.rbxl"], "the failed build removes its temporary file");
}

fn listed(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    names
}

#[test]
fn a_build_removes_no_file_or_folder_named_like_its_own() {
    let game = Game::template_layout();
    assert_eq!(code(&game.build(&[])), 0);
    let mine = [
        "build/.crucible-build-41-1/start.rbxl",
        "build/.crucible-build-0-0/notes.txt",
        "build/.crucible-build-00001-00001/notes.txt",
        "build/.crucible-0000000000000000.rbxl",
        "include/Promise.lua.crucible-write-41-1",
        "include/RuntimeLib.lua.crucible-write-0-0",
        "include/.crucible-0000000000000000.tmp",
        "out/shared/greeting.luau.crucible-write-41-1",
        "out/shared/.crucible-0000000000000000.tmp",
    ];
    for path in mine {
        game.write(path, "mine");
    }
    game.write("src/shared/greeting.ts", "export const greet = (name: string) => `hi from ${name}`;\n");
    game.write("include/Promise.lua", "edited");
    let out = game.build(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(game.read("out/shared/greeting.luau").contains("hi from"), "the module is written again");
    assert_eq!(fs::read(game.path("include/Promise.lua")).unwrap(), vendored("Promise.lua"));
    for path in mine {
        assert_eq!(game.read(path), "mine", "{path} stays");
    }
}

/// The name of every instance in a binary or XML place or model file, sorted.
fn instances(file: &Path) -> Vec<String> {
    let bytes = fs::read(file).unwrap();
    let mut names: Vec<String> = if bytes.starts_with(b"<roblox!") {
        let dom = rbx_binary::from_reader(bytes.as_slice()).unwrap();
        dom.descendants().map(|instance| instance.name.clone()).collect()
    } else {
        let xml = String::from_utf8(bytes).unwrap();
        xml.split("<string name=\"Name\">")
            .skip(1)
            .map(|rest| rest.split_once("</string>").unwrap().0.to_owned())
            .collect()
    };
    names.sort();
    names
}

#[test]
fn every_place_and_model_format_builds_inside_and_outside_a_folder_its_project_reads() {
    let game = Game::template_layout();
    assert_eq!(code(&game.build(&[])), 0);
    fs::write(crucible_compiler::temporary_path(&game.path("out/shared/greeting.luau"), "tmp"), "return 1").unwrap();
    for extension in ["rbxl", "rbxlx", "rbxm", "rbxmx"] {
        let mut expected = None;
        for output in [format!("build/start.{extension}"), format!("out/shared/inside.{extension}")] {
            let path = game.path(&output);
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let (temporary, lock) =
                (crucible_compiler::temporary_path(&path, extension), crucible_compiler::temporary_path(&path, "tmp"));
            for run in ["first build", "rebuild", "build after a killed one"] {
                if run == "build after a killed one" {
                    fs::write(&temporary, "half a place").unwrap();
                    fs::write(&lock, "").unwrap();
                }
                let out = game.build(&["--output", &output]);
                let stderr = text(&out.stderr);
                assert_eq!(code(&out), 0, "{output}, {run}: {stderr}");
                assert!(stderr.contains(&format!("Built project to {name}\n")), "{output}, {run}: {stderr}");
                assert!(!temporary.exists() && !lock.exists(), "{output}, {run}: the build's files go");
                let names = instances(&path);
                assert!(!names.iter().any(|name| name.starts_with(".crucible")), "{output}, {run}: {names:?}");
                // Rojo reads the last build's model in its project, as it did before temporary files.
                if run == "first build" || !(output.starts_with("out/") && extension.starts_with("rbxm")) {
                    assert_eq!(&names, expected.get_or_insert_with(|| names.clone()), "{output}, {run}");
                }
            }
            fs::remove_file(&path).unwrap();
        }
    }
}

#[test]
fn a_second_build_of_a_place_fails_while_the_first_writes_it_and_the_place_stays_whole() {
    use std::io::BufRead;
    let game = Game::template_layout();
    assert_eq!(code(&game.build(&[])), 0);
    let place = fs::read(game.path("build/start.rbxl")).unwrap();
    let mut first = Command::new(env!("CARGO_BIN_EXE_crucible"))
        .arg("build")
        .current_dir(game.dir.path())
        .env("CRUCIBLE_TEST_HOLD_ROJO", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = std::io::BufReader::new(first.stderr.take().unwrap());
    let mut line = String::new();
    while !line.contains("holding") {
        line.clear();
        assert_ne!(stderr.read_line(&mut line).unwrap(), 0, "the first build exited before Rojo started");
    }

    let second = game.build(&[]);
    let _ = first.kill();
    let _ = first.wait();
    assert_eq!(code(&second), 1, "{}", text(&second.stderr));
    assert!(text(&second.stderr).contains("another build is writing it"), "{}", text(&second.stderr));
    assert_eq!(fs::read(game.path("build/start.rbxl")).unwrap(), place, "the place stays whole");

    let third = game.build(&[]);
    assert_eq!(code(&third), 0, "{}", text(&third.stderr));
    assert_eq!(fs::read(game.path("build/start.rbxl")).unwrap(), place);
    assert_eq!(listed(&game.path("build")), ["start.rbxl"], "the next build writes the killed one's file again");
}

#[test]
fn a_folder_where_the_place_goes_fails_the_build_naming_it_and_stays() {
    let game = Game::template_layout();
    game.write("build/start.rbxl/notes.txt", "mine");
    let out = game.build(&[]);
    assert_eq!(code(&out), 1);
    assert!(
        text(&out.stderr).contains("cannot write") && text(&out.stderr).contains("start.rbxl"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(game.read("build/start.rbxl/notes.txt"), "mine");
    assert_eq!(listed(&game.path("build")), ["start.rbxl"], "the temporary file goes");
}

#[test]
fn a_folder_where_a_module_or_library_file_goes_fails_the_build_naming_it_and_stays() {
    for file in ["out/shared/greeting.luau", "include/Promise.lua"] {
        let game = Game::template_layout();
        game.write(&format!("{file}/notes.txt"), "mine");
        let out = game.build(&[]);
        assert_eq!(code(&out), 1);
        assert!(text(&out.stderr).contains(&format!("cannot write {file}: ")), "{}", text(&out.stderr));
        assert_eq!(game.read(&format!("{file}/notes.txt")), "mine");
    }
}

/// Makes `path` read-only, and is false when this process can still write it, as root can on Unix.
fn read_only(path: &Path) -> bool {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).unwrap();
    let writable = fs::OpenOptions::new().write(true).open(path).is_ok();
    if writable {
        eprintln!("skipped: this process can write read-only files");
    }
    !writable
}

fn writable(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).unwrap();
}

/// Links `link` to `target`, and is false on Windows when this process may not make links.
fn link(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    match made {
        Ok(()) => true,
        // ERROR_PRIVILEGE_NOT_HELD: Windows lets only administrators and Developer Mode make links.
        Err(error) if cfg!(windows) && error.raw_os_error() == Some(1314) => {
            eprintln!("skipped: this process may not make links");
            false
        }
        Err(error) => panic!("cannot link {}: {error}", link.display()),
    }
}

const CHANGED_FILES: [&str; 3] = ["out/shared/greeting.luau", "include/Promise.lua", "build/start.rbxl"];

fn change_a_module_and_a_library_file(game: &Game) {
    game.write("src/shared/greeting.ts", "export const greet = (name: string) => `hi from ${name}`;\n");
    game.write("include/Promise.lua", "edited");
}

#[test]
fn a_read_only_place_module_or_library_file_fails_the_build_naming_it_and_keeps_its_bytes() {
    for file in CHANGED_FILES {
        let game = Game::template_layout();
        assert_eq!(code(&game.build(&[])), 0);
        change_a_module_and_a_library_file(&game);
        let path = game.path(file);
        let bytes = fs::read(&path).unwrap();
        if read_only(&path) {
            let out = game.build(&[]);
            let stderr = text(&out.stderr).replace('\\', "/");
            assert_eq!(code(&out), 1, "{file}: {stderr}");
            assert!(stderr.contains("cannot write") && stderr.contains(&format!("{file}: ")), "{file}: {stderr}");
            assert_eq!(fs::read(&path).unwrap(), bytes, "{file} keeps its bytes");
            let names = listed(path.parent().unwrap());
            assert!(!names.iter().any(|name| name.starts_with(".crucible")), "{file}: {names:?}");
        }
        writable(&path);
    }
}

#[test]
fn a_place_module_or_library_file_that_is_a_link_writes_its_target_and_stays_a_link() {
    let game = Game::template_layout();
    assert_eq!(code(&game.build(&[])), 0);
    let place = fs::read(game.path("build/start.rbxl")).unwrap();
    fs::create_dir(game.path("elsewhere")).unwrap();
    let target = |file: &str| game.path("elsewhere").join(Path::new(file).file_name().unwrap());
    for file in CHANGED_FILES {
        fs::rename(game.path(file), target(file)).unwrap();
        if !link(&target(file), &game.path(file)) {
            return;
        }
    }
    change_a_module_and_a_library_file(&game);
    let out = game.build(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    for file in CHANGED_FILES {
        assert!(fs::symlink_metadata(game.path(file)).unwrap().file_type().is_symlink(), "{file} stays a link");
        let names = listed(game.path(file).parent().unwrap());
        assert!(!names.iter().any(|name| name.starts_with(".crucible")), "{file}: {names:?}");
    }
    assert!(fs::read_to_string(target("out/shared/greeting.luau")).unwrap().contains("hi from"));
    assert_eq!(fs::read(target("include/Promise.lua")).unwrap(), vendored("Promise.lua"));
    assert_place(&target("build/start.rbxl"));
    assert_ne!(fs::read(target("build/start.rbxl")).unwrap(), place, "the linked place holds the new module");
}

#[cfg(windows)]
#[test]
fn files_another_program_holds_without_delete_sharing_are_written_in_place() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    let game = Game::template_layout();
    assert_eq!(code(&game.build(&[])), 0);
    let place = fs::read(game.path("build/start.rbxl")).unwrap();
    let held: Vec<fs::File> = ["build/start.rbxl", "out/shared/greeting.luau", "include/Promise.lua"]
        .iter()
        .map(|path| {
            fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(game.path(path))
                .unwrap()
        })
        .collect();
    game.write("src/shared/greeting.ts", "export const greet = (name: string) => `hi from ${name}`;\n");
    game.write("include/Promise.lua", "edited");
    let out = game.build(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(game.read("out/shared/greeting.luau").contains("hi from"));
    assert_eq!(fs::read(game.path("include/Promise.lua")).unwrap(), vendored("Promise.lua"));
    assert_place(&game.path("build/start.rbxl"));
    assert_ne!(fs::read(game.path("build/start.rbxl")).unwrap(), place, "the place holds the new module");
    for dir in ["build", "include", "out/shared"] {
        let names = listed(&game.path(dir));
        assert!(!names.iter().any(|name| name.starts_with(".crucible")), "{dir}: {names:?}");
    }
    drop(held);
}

#[test]
fn unreadable_manifest_exits_1() {
    let game = Game::template_layout();
    fs::create_dir(game.path("crucible.toml")).unwrap();
    let out = game.build(&[]);
    assert_eq!(code(&out), 1, "an I/O failure is not a usage error");
    assert!(text(&out.stderr).contains("cannot read crucible.toml"), "{}", text(&out.stderr));
    assert!(!game.path("out").exists());
}

#[cfg(debug_assertions)]
#[test]
fn killing_crucible_stops_its_rojo_child() {
    use std::io::BufRead;
    use std::time::Duration;
    let game = Game::template_layout();
    let mut parent = Command::new(env!("CARGO_BIN_EXE_crucible"))
        .arg("build")
        .current_dir(game.dir.path())
        .env("CRUCIBLE_TEST_HOLD_ROJO", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (lines, received) = std::sync::mpsc::channel();
    let stderr = std::io::BufReader::new(parent.stderr.take().unwrap());
    std::thread::spawn(move || {
        for line in stderr.lines() {
            let _ = lines.send(Some(line.unwrap()));
        }
        let _ = lines.send(None);
    });
    let pid = loop {
        match received.recv_timeout(Duration::from_secs(60)).expect("the Rojo child starts") {
            Some(line) => {
                if let Some(pid) = line.strip_prefix("rojo child ").and_then(|rest| rest.strip_suffix(" holding")) {
                    break pid.to_owned();
                }
            }
            None => panic!("crucible build exited before Rojo started"),
        }
    };
    parent.kill().unwrap();
    parent.wait().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let gone = loop {
        match received.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(Some(_)) => continue,
            Ok(None) => break true,
            Err(_) => break false,
        }
    };
    if !gone {
        if cfg!(windows) {
            let _ = Command::new("taskkill").args(["/F", "/PID", &pid]).output();
        } else {
            let _ = Command::new("kill").args(["-9", &pid]).output();
        }
    }
    assert!(gone, "the Rojo child {pid} still holds stderr 10 s after its parent was killed");
}

#[test]
fn version_reports_embedded_rojo() {
    let out = Command::new(env!("CARGO_BIN_EXE_crucible")).arg("--version").output().unwrap();
    assert_eq!(text(&out.stdout).trim(), format!("crucible {} (rojo 7.7.1)", env!("CARGO_PKG_VERSION")));
}

fn reactive_game() -> Game {
    let game = Game::template_layout();
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    project["tree"]["ReplicatedStorage"]["Packages"] = json!({"$path": "node_modules/@rbxts"});
    game.json("default.project.json", &project);
    game
}

fn package(game: &Game, path: &str, name: &str, version: &str) {
    game.json(&format!("{path}/package.json"), &json!({"name": name, "version": version}));
    game.write(&format!("{path}/init.luau"), "return {}\n");
}

#[test]
fn reactive_one_copy_and_minimal_games_build() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
}

#[test]
fn reactive_nested_copy_fails_before_writes() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    package(&game, "node_modules/@rbxts/holder", "@rbxts/holder", "1.0.0");
    package(&game, "node_modules/@rbxts/holder/node_modules/@rbxts/charm", "@rbxts/charm", "0.10.0");
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    let message = text(&output.stderr).replace('\\', "/");
    for expected in [
        "multiple copies",
        "0.11.1",
        "0.10.0",
        "node_modules/@rbxts/charm",
        "holder/node_modules/@rbxts/charm",
        "pnpm.overrides",
    ] {
        assert!(message.contains(expected), "{message}");
    }
    assert!(!game.path("out").exists());
    assert!(!game.path("include").exists());
}

#[test]
fn reactive_ranges_come_from_installed_packages() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.10.0");
    game.json(
        "node_modules/@rbxts/vide-charm/package.json",
        &json!({"name":"@rbxts/vide-charm", "version":"0.4.1", "dependencies":{"@rbxts/charm":"^0.11.1"}}),
    );
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    for expected in ["@rbxts/vide-charm", "^0.11.1", "0.10.0", "does not satisfy"] {
        assert!(text(&output.stderr).contains(expected), "{}", text(&output.stderr));
    }
}

#[test]
fn reactive_second_mapped_folder_fails_but_unmapped_copy_is_ignored() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/vide", "@rbxts/vide", "0.6.1");
    package(&game, "other/@rbxts/vide", "@rbxts/vide", "0.6.0");
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    project["tree"]["ReplicatedStorage"]["Other"] = json!({"$path":"other/@rbxts"});
    game.json("default.project.json", &project);
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    let message = text(&output.stderr).replace('\\', "/");
    for expected in ["@rbxts/vide", "0.6.1", "0.6.0", "other/@rbxts/vide", "node_modules/@rbxts/vide"] {
        assert!(message.contains(expected), "{message}");
    }
}

fn junction(game: &Game, link: &str, target: &str) {
    let (link, target) = (game.path(link), game.path(target));
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    #[cfg(windows)]
    {
        let output = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

fn map(game: &Game, instance: &str, path: Value) {
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    project["tree"]["ReplicatedStorage"][instance] = json!({ "$path": path });
    game.json("default.project.json", &project);
}

// Counts the ModuleScripts named `name` in the place, as Rojo's sourcemap lists them.
fn modules_in_place(game: &Game, name: &str) -> usize {
    fn count(node: &Value, name: &str) -> usize {
        let children = node["children"].as_array().into_iter().flatten().map(|child| count(child, name)).sum::<usize>();
        children + usize::from(node["name"] == name && node["className"] == "ModuleScript")
    }
    // Rojo needs the folders a build writes, and a check that fails stops the build before it writes them.
    let missing: Vec<_> =
        ["include", "out/shared", "out/server"].map(|f| game.path(f)).into_iter().filter(|f| !f.exists()).collect();
    for folder in &missing {
        fs::create_dir_all(folder).unwrap();
    }
    let output = game.path("sourcemap.json");
    let command = librojo::cli::SourcemapCommand {
        project: game.path("default.project.json"),
        output: Some(output.clone()),
        include_non_scripts: false,
        watch: false,
        absolute: false,
    };
    command.run().unwrap();
    for folder in missing {
        fs::remove_dir_all(folder).unwrap();
    }
    let _ = fs::remove_dir(game.path("out"));
    count(&serde_json::from_str(&fs::read_to_string(output).unwrap()).unwrap(), name)
}

// pnpm's layout: the top-level link and the store's sibling link for vide-charm reach one store copy.
fn pnpm_game() -> Game {
    let game = reactive_game();
    let store = "node_modules/.pnpm/@rbxts+charm@0.11.1/node_modules/@rbxts/charm";
    package(&game, store, "@rbxts/charm", "0.11.1");
    let bridge = "node_modules/.pnpm/@rbxts+vide-charm@0.4.1/node_modules/@rbxts/vide-charm";
    game.json(
        &format!("{bridge}/package.json"),
        &json!({"name":"@rbxts/vide-charm", "version":"0.4.1", "dependencies":{"@rbxts/charm":"^0.11.1"}}),
    );
    game.write(&format!("{bridge}/init.luau"), "return {}\n");
    junction(&game, "node_modules/.pnpm/@rbxts+vide-charm@0.4.1/node_modules/@rbxts/charm", store);
    junction(&game, "node_modules/@rbxts/charm", store);
    junction(&game, "node_modules/@rbxts/vide-charm", bridge);
    game
}

#[test]
fn reactive_pnpm_links_to_one_store_copy_pass() {
    let game = pnpm_game();
    // A store neighbour of vide-charm that the place does not map, so neither its range nor its manifest counts.
    let helper = "node_modules/.pnpm/@rbxts+vide-charm@0.4.1/node_modules/helper";
    game.json(
        &format!("{helper}/package.json"),
        &json!({"name":"helper", "version":"1.0.0", "dependencies":{"@rbxts/charm":"0.10.0"}}),
    );
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
    assert_eq!(modules_in_place(&game, "charm"), 1);
}

#[test]
fn reactive_one_store_copy_mapped_twice_fails() {
    let game = pnpm_game();
    map(&game, "Second", json!("node_modules/.pnpm/@rbxts+charm@0.11.1/node_modules/@rbxts"));
    assert_eq!(modules_in_place(&game, "charm"), 2, "Rojo builds the one folder into two module trees");
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    let message = text(&output.stderr).replace('\\', "/");
    assert!(message.contains("multiple copies of @rbxts/charm"), "{message}");
    assert!(message.contains("node_modules/.pnpm/@rbxts+charm@0.11.1/node_modules/@rbxts/charm"), "{message}");
}

#[test]
fn reactive_copy_in_a_nested_project_file_fails() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/vide", "@rbxts/vide", "0.6.1");
    package(&game, "vendor/node_modules/@rbxts/vide", "@rbxts/vide", "0.6.0");
    game.json("vendor.project.json", &json!({"name":"vendor", "tree":{"$path":"vendor/node_modules/@rbxts"}}));
    map(&game, "Extra", json!("vendor.project.json"));
    assert_eq!(modules_in_place(&game, "vide"), 2);
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    let message = text(&output.stderr).replace('\\', "/");
    for expected in ["multiple copies of @rbxts/vide", "0.6.0 at", "vendor/node_modules/@rbxts/vide"] {
        assert!(message.contains(expected), "{message}");
    }
}

#[test]
fn reactive_ranges_of_packages_outside_the_place_do_not_count() {
    // npm's layout for a development tool that depends on another Charm. Nothing maps node_modules/tool-x.
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    game.json(
        "node_modules/tool-x/package.json",
        &json!({"name":"tool-x", "version":"1.0.0", "dependencies":{"@rbxts/charm":"0.10.0"}}),
    );
    package(&game, "node_modules/tool-x/node_modules/@rbxts/charm", "@rbxts/charm", "0.10.0");
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
}

#[test]
fn reactive_ranges_count_only_against_the_copy_their_package_loads() {
    // npm nests Charm 0.11.1 under vide-charm when the game pins 0.10.0. The game's range applies to the top copy only.
    let game = reactive_game();
    game.json(
        "package.json",
        &json!({"name":"probe-game", "version":"0.0.0", "dependencies":{"@rbxts/charm":"0.10.0"}}),
    );
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.10.0");
    game.json(
        "node_modules/@rbxts/vide-charm/package.json",
        &json!({"name":"@rbxts/vide-charm", "version":"0.4.1", "dependencies":{"@rbxts/charm":"^0.11.1"}}),
    );
    package(&game, "node_modules/@rbxts/vide-charm/node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    let message = text(&output.stderr);
    assert!(message.contains("multiple copies of @rbxts/charm"), "{message}");
    assert!(!message.contains("does not satisfy"), "{message}");
}

#[test]
fn reactive_glob_ignore_paths_leave_a_nested_copy_out() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    game.json(
        "node_modules/@rbxts/holder/package.json",
        &json!({"name":"@rbxts/holder", "version":"1.0.0", "dependencies":{"@rbxts/charm":"^0.11.0"}}),
    );
    package(&game, "node_modules/@rbxts/holder/node_modules/@rbxts/charm", "@rbxts/charm", "0.11.0");
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    assert!(text(&output.stderr).contains("multiple copies of @rbxts/charm"));
    let mut project: Value = serde_json::from_str(&game.read("default.project.json")).unwrap();
    project["globIgnorePaths"] = json!(["**/node_modules/@rbxts/*/node_modules/**"]);
    game.json("default.project.json", &project);
    assert_eq!(modules_in_place(&game, "charm"), 1);
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
}

#[test]
fn reactive_manifests_outside_the_place_are_not_read() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    game.write("node_modules/broken/package.json", "{\"name\":\"broken\",}");
    game.write("node_modules/bomtool/package.json", "\u{feff}{\"name\":\"bomtool\",\"version\":\"1.0.0\"}");
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
    game.write("node_modules/@rbxts/broken/package.json", "{\"name\":\"broken\",}");
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    let message = text(&output.stderr).replace('\\', "/");
    assert!(message.contains("node_modules/@rbxts/broken/package.json is not valid JSON"), "{message}");
    assert!(message.contains("trailing comma"), "{message}");
}

#[test]
fn reactive_package_project_excludes_unmapped_nested_copy() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    package(&game, "node_modules/@rbxts/holder", "@rbxts/holder", "1.0.0");
    package(&game, "node_modules/@rbxts/holder/node_modules/@rbxts/charm", "@rbxts/charm", "0.10.0");
    game.json("node_modules/@rbxts/holder/default.project.json", &json!({"name":"holder", "tree":{"$path":"src"}}));
    game.write("node_modules/@rbxts/holder/src/init.luau", "return {}\n");
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
    // Mapped directly, the folder is still read through its project.
    map(&game, "Holder", json!("node_modules/@rbxts/holder"));
    assert_eq!(modules_in_place(&game, "charm"), 1);
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
}

#[test]
fn reactive_charm_sync_copies_split_tracking() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm-sync", "@rbxts/charm-sync", "0.4.1");
    package(&game, "node_modules/@rbxts/holder", "holder", "1.0.0");
    package(&game, "node_modules/@rbxts/holder/node_modules/@rbxts/charm-sync", "@rbxts/charm-sync", "0.4.1");
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    assert!(text(&output.stderr).contains("multiple copies of @rbxts/charm-sync"));
}

#[test]
fn reactive_peer_ranges_accept_npm_unions_and_hyphens() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    game.json("node_modules/@rbxts/consumer/package.json", &json!({"name":"consumer", "version":"1.0.0", "peerDependencies":{"@rbxts/charm":"0.10.0 - 0.11.1 || >=1.0.0"}}));
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output.stderr));
    game.json(
        "node_modules/@rbxts/consumer/package.json",
        &json!({"name":"consumer", "version":"1.0.0", "peerDependencies":{"@rbxts/charm":"^0.12.0"}}),
    );
    let output = game.build(&[]);
    assert_eq!(code(&output), 1);
    assert!(text(&output.stderr).contains("peerDependencies range ^0.12.0"));
}

#[test]
fn reactive_entries_that_name_another_source_build() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    for spec in [
        "npm:@rbxts/charm@0.11.1",
        "npm:@rbxts/charm",
        "npm:@rbxts/charm@latest",
        "file:../vendor/charm",
        "link:../vendor/charm",
        "workspace:*",
        "workspace:^",
        "workspace:~",
        "catalog:",
        "catalog:roblox",
        "git+https://github.com/littensy/charm.git#v0.11.1",
        "github:littensy/charm",
        "littensy/charm",
        "https://example.com/charm-0.11.1.tgz",
        "latest",
        "",
    ] {
        let manifest = json!({"name":"probe-game", "version":"0.0.0", "dependencies":{"@rbxts/charm":spec}});
        game.json("package.json", &manifest);
        let holder = json!({"name":"@rbxts/holder", "version":"1.0.0", "peerDependencies":{"@rbxts/charm":spec}});
        game.json("node_modules/@rbxts/holder/package.json", &holder);
        let output = game.build(&[]);
        assert_eq!(code(&output), 0, "{spec:?}: {}", text(&output.stderr));
    }
}

#[test]
fn reactive_alias_and_workspace_entries_keep_their_range() {
    let game = reactive_game();
    package(&game, "node_modules/@rbxts/charm", "@rbxts/charm", "0.11.1");
    for spec in ["npm:@rbxts/charm@^0.12.0", "workspace:^0.12.0"] {
        game.json(
            "package.json",
            &json!({"name":"probe-game", "version":"0.0.0", "dependencies":{"@rbxts/charm":spec}}),
        );
        let output = game.build(&[]);
        assert_eq!(code(&output), 1, "{spec}");
        assert!(text(&output.stderr).contains(spec), "{}", text(&output.stderr));
    }
    let other = "npm:@rbxts/charm-fork@^0.12.0";
    game.json("package.json", &json!({"name":"probe-game", "version":"0.0.0", "dependencies":{"@rbxts/charm":other}}));
    let output = game.build(&[]);
    assert_eq!(code(&output), 0, "an alias of another package names another source: {}", text(&output.stderr));
}
