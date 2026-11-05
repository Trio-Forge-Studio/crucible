#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crucible_compiler::{Diagnostic, Facts, Module};
use serde_json::{Value, json};

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub struct Game {
    pub dir: PathBuf,
}

pub fn copy_declarations(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).expect("the package folder exists; run pnpm build first") {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_declarations(&path, &target);
        } else if path.to_string_lossy().ends_with(".d.ts") || path.file_name().unwrap() == "package.json" {
            std::fs::create_dir_all(to).unwrap();
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

impl Game {
    pub fn new() -> Game {
        Game::under("client")
    }

    /// The game sits in a folder named `ancestor`, which side rules must ignore because it is above the package.
    pub fn under(ancestor: &str) -> Game {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(env!("CARGO_CRATE_NAME")).join(format!("{id}"));
        let _ = std::fs::remove_dir_all(&scratch);
        let dir = scratch.join(ancestor).join("game");
        let installed = dir.join("node_modules/@trioforge/crucible");
        let package = repo().join("packages/crucible");
        for entry in ["out", "server", "client", "emit", "vide", "ui", "util", "maths"] {
            copy_declarations(&package.join(entry), &installed.join(entry));
        }
        copy_declarations(&package.join("node_modules/@rbxts/vide"), &dir.join("node_modules/@rbxts/vide"));
        std::fs::copy(package.join("package.json"), installed.join("package.json")).unwrap();
        std::fs::create_dir_all(dir.join("src/server")).unwrap();
        let game = Game { dir };
        game.json("package.json", &json!({ "name": "fixture-game", "private": true }));
        game.json("default.project.json", &project());
        game.config(|_| {});
        game
    }

    pub fn write(&self, path: &str, text: &str) -> &Game {
        let file = self.dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
        self
    }

    pub fn json(&self, path: &str, value: &Value) -> &Game {
        self.write(path, &serde_json::to_string_pretty(value).unwrap())
    }

    pub fn remove(&self, path: &str) -> &Game {
        std::fs::remove_file(self.dir.join(path)).unwrap();
        self
    }

    pub fn config(&self, edit: impl FnOnce(&mut Value)) -> &Game {
        let roots = repo().join("packages/crucible/node_modules");
        let mut config = json!({
            "extends": slashes(&repo().join("tsconfig.ts7.json")),
            "compilerOptions": {
                "rootDir": "src",
                "outDir": "out",
                "typeRoots": [slashes(&roots.join("@trioforge")), slashes(&roots.join("@rbxts"))],
                "types": ["compiler-types", "types"],
            },
            "include": ["src"],
        });
        edit(&mut config);
        self.json("tsconfig.json", &config)
    }

    pub fn main(&self, text: &str) -> &Game {
        self.write("src/server/main.ts", text)
    }

    pub fn analyze(&self) -> Result<Facts, Vec<Diagnostic>> {
        self.analyze_at("tsconfig.json")
    }

    pub fn analyze_at(&self, tsconfig: &str) -> Result<Facts, Vec<Diagnostic>> {
        crucible_compiler::analyze(&self.dir.join(tsconfig))
    }

    pub fn facts(&self) -> Facts {
        self.analyze().unwrap_or_else(|d| panic!("the game type-checks:\n{}", lines(&d)))
    }

    pub fn relative(&self, diagnostic: &Diagnostic) -> String {
        let dir = format!("{}/", slashes(&self.dir));
        let path = diagnostic.path.as_deref().unwrap_or("");
        path.strip_prefix(&dir).unwrap_or(path).to_owned()
    }

    pub fn found(&self) -> Vec<String> {
        self.found_at("tsconfig.json")
    }

    pub fn found_at(&self, tsconfig: &str) -> Vec<String> {
        let facts = self.analyze_at(tsconfig).unwrap_or_else(|d| panic!("the game type-checks:\n{}", lines(&d)));
        facts
            .diagnostics
            .iter()
            .filter(|d| d.code >= 90000)
            .map(|d| format!("{}@{}:{}", code(d), self.relative(d), d.line))
            .collect()
    }

    pub fn found_in(&self, file: &str) -> Vec<String> {
        let suffix = format!("@{file}:");
        self.found().into_iter().filter(|f| f.contains(&suffix)).collect()
    }

    pub fn type_errors(&self) -> Vec<String> {
        match self.analyze() {
            Ok(_) => Vec::new(),
            Err(diagnostics) => diagnostics.iter().map(|d| format!("{}:{}", self.relative(d), d.line)).collect(),
        }
    }

    pub fn compile(&self) -> Vec<Module> {
        crucible_compiler::compile(&self.dir.join("tsconfig.json"))
            .unwrap_or_else(|f| panic!("the game compiles:\n{f}"))
    }

    pub fn luau(&self, path: &str) -> String {
        let modules = self.compile();
        modules.into_iter().find(|m| m.path == path).unwrap_or_else(|| panic!("{path} is emitted")).source
    }
}

pub fn project() -> Value {
    json!({
        "name": "fixture",
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
            "StarterPlayer": {
                "$className": "StarterPlayer",
                "StarterPlayerScripts": { "$className": "StarterPlayerScripts", "TS": { "$path": "out/client" } },
            },
        },
    })
}

pub fn code(d: &Diagnostic) -> String {
    match d.code {
        90000 => "Crucible".to_owned(),
        code if code > 90000 => format!("CRU{:03}", code - 90000),
        code => format!("TS{code}"),
    }
}

pub fn lines(diagnostics: &[Diagnostic]) -> String {
    diagnostics.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
}

pub fn has(source: &str, part: &str) {
    assert!(source.contains(part), "the output contains `{part}`:\n{source}");
}
