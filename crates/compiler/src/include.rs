//! The files in `roblox-ts/` are copied unchanged from roblox-ts 3.0.0, under its MIT license in `roblox-ts/LICENSE`.
//! `Promise.lua` is evaera's roblox-lua-promise, under its own MIT license in `roblox-ts/Promise.LICENSE`.

use std::io;
use std::path::Path;

use crate::write;

// Keeps rbxtsc's `.lua` name, so a game's first build replaces rbxtsc's copy instead of adding a second
// `RuntimeLib` instance beside it.
pub(crate) const RUNTIME_LIB: &str = "RuntimeLib.lua";

const FILES: [(&str, &[u8]); 2] = [
    (RUNTIME_LIB, include_bytes!("../roblox-ts/RuntimeLib.lua")),
    ("Promise.lua", include_bytes!("../roblox-ts/Promise.lua")),
];

/// Writes the runtime library into `dir` through `write::replace`, creating it, and skips each file whose bytes
/// already match. It deletes the temporary files of its own writes that did not finish, and other files in `dir` stay.
/// Returns how many files it wrote.
pub fn write_include(dir: &Path) -> io::Result<usize> {
    let mut written = 0;
    for (name, bytes) in FILES {
        let path = dir.join(name);
        match std::fs::read(&path) {
            Ok(current) if current == bytes => continue,
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::create_dir_all(dir)?,
            Err(e) => return Err(e),
        }
        write::replace(&path, bytes)?;
        written += 1;
    }
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        let ours =
            name.to_str().and_then(write::partial_write_of).is_some_and(|of| FILES.iter().any(|(f, _)| *f == of));
        if ours {
            std::fs::remove_file(dir.join(name))?;
        }
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
        std::fs::write(include.join("Custom.lua"), "kept").unwrap();
        std::fs::write(include.join("Promise.lua.crucible-write-41-1"), "local Pro").unwrap();
        std::fs::write(include.join("Custom.lua.crucible-write-41-1"), "kept").unwrap();
        assert_eq!(write_include(&include).unwrap(), 1, "only the edited file is written");
        for (name, bytes) in FILES {
            assert_eq!(std::fs::read(include.join(name)).unwrap(), bytes, "{name}");
        }
        let mut names: Vec<String> =
            std::fs::read_dir(&include).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(
            names,
            ["Custom.lua", "Custom.lua.crucible-write-41-1", "Promise.lua", RUNTIME_LIB],
            "a killed write's file goes"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_library_is_roblox_ts_3_0_0() {
        assert_eq!(FILES.map(|(name, bytes)| (name, bytes.len())), [(RUNTIME_LIB, 6018), ("Promise.lua", 60896)]);
        assert!(FILES[0].1.starts_with(b"local Promise = require(script.Parent.Promise)"));
    }
}
