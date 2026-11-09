use std::path::{Path, PathBuf};

use crucible_compiler::{Failure, Target};
use serde_json::{Value, json};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn write(dir: &Path, path: &str, text: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

/// A program that imports a runtime whose `package.json` has `fields`.
fn program(name: &str, fields: Value) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(env!("CARGO_CRATE_NAME")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    let roots = repo().join("packages/crucible/node_modules");
    let config = json!({
        "extends": slashes(&repo().join("tsconfig.json")),
        "compilerOptions": {
            "rootDir": "src",
            "outDir": "out",
            "typeRoots": [slashes(&roots.join("@trioforge")), slashes(&roots.join("@rbxts"))],
            "types": ["compiler-types", "types"],
        },
        "include": ["src"],
    });
    write(&dir, "tsconfig.json", &config.to_string());
    write(&dir, "package.json", &json!({ "name": "fixture", "private": true }).to_string());
    let mut manifest = json!({
        "name": crucible_abi::PACKAGE,
        "types": "index.d.ts",
        "exports": { ".": { "types": "./index.d.ts" } },
    });
    manifest.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
    write(&dir, "node_modules/@trioforge/crucible/package.json", &manifest.to_string());
    write(&dir, "node_modules/@trioforge/crucible/index.d.ts", "export declare const label: string;\n");
    write(&dir, "src/main.ts", "import { label } from \"@trioforge/crucible\";\nprint(label);\n");
    dir
}

fn compile(dir: &Path) -> Result<(), Failure> {
    crucible_transform::compile(&dir.join("tsconfig.json"), Target::Package).map(drop)
}

#[test]
fn a_runtime_that_records_this_abi_compiles() {
    let dir = program("matching", json!({ (crucible_abi::MANIFEST_FIELD): crucible_abi::VERSION }));
    compile(&dir).unwrap_or_else(|f| panic!("the program compiles:\n{f}"));
}

#[test]
fn a_runtime_that_records_another_abi_fails_with_both_numbers() {
    let other = crucible_abi::VERSION + 1;
    let dir = program("other", json!({ (crucible_abi::MANIFEST_FIELD): other }));
    let Err(Failure::Crucible(diagnostics)) = compile(&dir) else { panic!("the handshake fails the compile") };
    let [diagnostic] = &diagnostics[..] else { panic!("one diagnostic: {diagnostics:?}") };
    assert_eq!((diagnostic.path.as_deref(), diagnostic.code), (None, 90000));
    let ending = format!(
        "/node_modules/@trioforge/crucible/package.json implements ABI {other}, and this compiler emits ABI {}. Install \
         the {} release that matches this compiler.",
        crucible_abi::VERSION,
        crucible_abi::PACKAGE,
    );
    let message = &diagnostic.message;
    assert!(message.starts_with("Crucible: ") && message.ends_with(&ending), "{message}");
}

#[test]
fn the_compiler_alone_skips_the_handshake() {
    let dir = program("compiler", json!({ (crucible_abi::MANIFEST_FIELD): crucible_abi::VERSION + 1 }));
    crucible_compiler::compile(&dir.join("tsconfig.json"), Target::Package)
        .unwrap_or_else(|f| panic!("the compiler names no ABI:\n{f}"));
}
