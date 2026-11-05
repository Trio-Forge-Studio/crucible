use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A game's TypeScript 7 config that reads the repository's installed type packages, which `pnpm install` provides.
pub fn tsconfig() -> Value {
    let types = repo().join("packages/crucible/node_modules");
    json!({
        "compilerOptions": {
            "module": "preserve",
            "moduleResolution": "bundler",
            "noLib": true,
            "strict": true,
            "target": "ESNext",
            "skipLibCheck": true,
            "moduleDetection": "force",
            "typeRoots": [slashes(&types.join("@trioforge")), slashes(&types.join("@rbxts"))],
            "types": ["compiler-types", "types"],
            "rootDir": "src",
            "outDir": "out",
        },
        "include": ["src"],
    })
}
