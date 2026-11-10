//! The files in `roblox-ts/` are copied unchanged from roblox-ts 3.0.0, under its MIT license in `roblox-ts/LICENSE`.
//! `Promise.lua` is evaera's roblox-lua-promise, under its own MIT license in `roblox-ts/Promise.LICENSE`.

use std::io;
use std::path::Path;

use crate::write::{self, WriteError, at};

// Keeps rbxtsc's `.lua` name, so a game's first build replaces rbxtsc's copy instead of adding a second
// `RuntimeLib` instance beside it.
pub(crate) const RUNTIME_LIB: &str = "RuntimeLib.lua";

const FILES: [(&str, &[u8]); 2] = [
    (RUNTIME_LIB, include_bytes!("../roblox-ts/RuntimeLib.lua")),
    ("Promise.lua", include_bytes!("../roblox-ts/Promise.lua")),
];

/// Writes the runtime library into `dir` through `write::replace`, creating it, and skips each file whose bytes
/// already match. Other files in `dir` stay. Returns how many files it wrote.
pub fn write_include(dir: &Path) -> Result<usize, WriteError> {
    let mut written = 0;
    for (name, bytes) in FILES {
        let path = dir.join(name);
        match std::fs::read(&path) {
            Ok(current) if current == bytes => continue,
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::create_dir_all(dir).map_err(at(dir))?,
            Err(e) => return Err(at(&path)(e)),
        }
        write::replace(&path, bytes).map_err(at(&path))?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_library_once_and_restores_an_edited_file() {
        let dir = std::env::temp_dir().join(format!("crucible-include-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let include = dir.join("places/job/include");
        assert_eq!(write_include(&include).unwrap(), 2, "creates the folder and both files");
        assert_eq!(write_include(&include).unwrap(), 0, "matching bytes are skipped");
        std::fs::write(include.join(RUNTIME_LIB), "edited").unwrap();
        std::fs::write(write::temporary_path(&include.join(RUNTIME_LIB), "tmp"), "local Pro").unwrap();
        let mine =
            ["Custom.lua", "Promise.lua.crucible-write-41-1", "RuntimeLib.lua.crucible-write-0-0", ".crucible-1.tmp"];
        for name in mine {
            std::fs::write(include.join(name), "kept").unwrap();
        }
        assert_eq!(write_include(&include).unwrap(), 1, "only the edited file is written");
        for (name, bytes) in FILES {
            assert_eq!(std::fs::read(include.join(name)).unwrap(), bytes, "{name}");
        }
        let mut names: Vec<String> =
            std::fs::read_dir(&include).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        let mut expected = [&mine[..], &["Promise.lua", RUNTIME_LIB]].concat();
        expected.sort();
        assert_eq!(names, expected, "a killed write's file is written again, and every other file stays");
        for name in mine {
            assert_eq!(std::fs::read(include.join(name)).unwrap(), b"kept", "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_where_a_library_file_goes_fails_the_write_naming_it_and_stays() {
        let dir = std::env::temp_dir().join(format!("crucible-include-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Promise.lua")).unwrap();
        std::fs::write(dir.join("Promise.lua/notes.txt"), "mine").unwrap();
        let error = write_include(&dir).unwrap_err();
        assert_eq!(error.path, dir.join("Promise.lua"), "{error}");
        assert_eq!(std::fs::read(dir.join("Promise.lua/notes.txt")).unwrap(), b"mine");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_library_is_roblox_ts_3_0_0() {
        assert_eq!(FILES.map(|(name, bytes)| (name, bytes.len())), [(RUNTIME_LIB, 6018), ("Promise.lua", 60896)]);
        assert!(FILES[0].1.starts_with(b"local Promise = require(script.Parent.Promise)"));
    }
}
