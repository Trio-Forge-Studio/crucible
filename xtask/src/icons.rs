use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use resvg::{tiny_skia, usvg};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CELL: u32 = 64;
const GUTTER: u32 = 2;
const ICON_SIZE: u32 = CELL - 2 * GUTTER;
const COLUMNS: u32 = 16;
const SHEET_SIZE: u32 = COLUMNS * CELL;
const PER_SHEET: usize = (COLUMNS * COLUMNS) as usize;
const LOCK_PATH: &str = "xtask/icons.lock";
const GENERATED_PATH: &str = "packages/crucible/src/ui/lucide.ts";
const UPLOAD: &str = "cargo xtask icons upload --group <groupId>";

#[derive(Deserialize)]
struct LucidePackage {
    version: String,
}

struct Atlas {
    version: String,
    names: Vec<String>,
    sheets: Vec<Sheet>,
}

struct Sheet {
    png: Vec<u8>,
    sha256: String,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
struct Lock {
    lucide: String,
    cell: u32,
    #[serde(default)]
    sheets: Vec<LockedSheet>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
struct LockedSheet {
    index: usize,
    sha256: String,
    asset: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadReceipt {
    asset_id: String,
}

enum Action<'a> {
    Render,
    Upload(&'a str),
    Generate,
    Check,
}

pub fn main(args: &[String]) -> Result<(), String> {
    let action = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["render"] => Action::Render,
        ["upload", "--group", group] if positive_id(group) => Action::Upload(group),
        [] | ["generate"] => Action::Generate,
        ["check"] => Action::Check,
        _ => return Err("usage: cargo xtask icons [render|upload --group <groupId>|generate|check]".into()),
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let atlas = render(&root.join("packages/crucible/node_modules/lucide-static"))?;
    if matches!(action, Action::Render) {
        for (index, sheet) in atlas.sheets.iter().enumerate() {
            write_png(&root, index, sheet)?;
            println!("sheet {index} {} {} bytes", sheet.sha256, sheet.png.len());
        }
        println!("Lucide {}: {} icons", atlas.version, atlas.names.len());
        return Ok(());
    }

    let _upload_guard = if matches!(action, Action::Upload(_)) { Some(lock_upload(&root)?) } else { None };
    let lock_path = root.join(LOCK_PATH);
    let previous = match fs::read_to_string(&lock_path) {
        Ok(text) => Some(toml::from_str(&text).map_err(|e| format!("{LOCK_PATH}: {e}"))?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{LOCK_PATH}: {e}")),
    };
    let mut lock = reconcile(&atlas, previous.as_ref())?;
    if let Action::Upload(group) = action {
        return upload(&root, &atlas, &mut lock, previous.as_ref(), group);
    }

    let missing: Vec<_> = atlas
        .sheets
        .iter()
        .enumerate()
        .filter(|(index, _)| !lock.sheets.iter().any(|sheet| sheet.index == *index))
        .map(|(index, _)| index.to_string())
        .collect();
    if !missing.is_empty() {
        return Err(format!("Missing asset IDs in {LOCK_PATH} for sheets {}. Run `{UPLOAD}`.", missing.join(", ")));
    }
    if previous.as_ref() != Some(&lock) {
        return Err(format!("{LOCK_PATH} is out of date. Run `{UPLOAD}` to update it."));
    }

    let generated = typescript(&atlas, &lock);
    let path = root.join(GENERATED_PATH);
    if matches!(action, Action::Check) {
        if fs::read(&path).ok().as_deref() != Some(generated.as_bytes()) {
            return Err(format!("{GENERATED_PATH} is stale. Run `cargo xtask icons generate`."));
        }
    } else {
        fs::write(&path, generated).map_err(|e| format!("{GENERATED_PATH}: {e}"))?;
    }
    Ok(())
}

fn render(package: &Path) -> Result<Atlas, String> {
    let manifest = package.join("package.json");
    let bytes = fs::read(&manifest).map_err(|e| format!("{}: {e}. Run `pnpm install`.", manifest.display()))?;
    let LucidePackage { version } =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let dir = package.join("icons");
    // `icons/` also holds a copy of each icon under its deprecated alias names; `icon-nodes.json` lists only the
    // canonical names that lucide.dev shows.
    let nodes = package.join("icon-nodes.json");
    let bytes = fs::read(&nodes).map_err(|e| format!("{}: {e}", nodes.display()))?;
    let canonical: std::collections::BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", nodes.display()))?;
    let names: Vec<String> = canonical.into_keys().collect();
    if names.is_empty() {
        return Err(format!("{} lists no icons", nodes.display()));
    }

    let options = usvg::Options::default();
    let mut icon = tiny_skia::Pixmap::new(ICON_SIZE, ICON_SIZE).expect("fixed icon size");
    let mut sheets = Vec::new();
    for batch in names.chunks(PER_SHEET) {
        let mut sheet = tiny_skia::Pixmap::new(SHEET_SIZE, SHEET_SIZE).expect("fixed sheet size");
        for (slot, name) in batch.iter().enumerate() {
            let path = dir.join(format!("{name}.svg"));
            let svg = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let tree = usvg::Tree::from_str(&svg.replace("currentColor", "#ffffff"), &options)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            let transform = tiny_skia::Transform::from_scale(
                ICON_SIZE as f32 / tree.size().width(),
                ICON_SIZE as f32 / tree.size().height(),
            );
            icon.fill(tiny_skia::Color::TRANSPARENT);
            resvg::render(&tree, transform, &mut icon.as_mut());
            let slot = slot as u32;
            sheet.draw_pixmap(
                ((slot % COLUMNS) * CELL + GUTTER) as i32,
                ((slot / COLUMNS) * CELL + GUTTER) as i32,
                icon.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            );
        }
        let png = sheet.encode_png().map_err(|e| format!("Encode sheet {}: {e}", sheets.len()))?;
        let sha256 = format!("{:x}", Sha256::digest(&png));
        sheets.push(Sheet { png, sha256 });
    }
    Ok(Atlas { version, names, sheets })
}

fn write_png(root: &Path, index: usize, sheet: &Sheet) -> Result<PathBuf, String> {
    let dir = root.join("target/icons");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("sheet-{index}.png"));
    fs::write(&path, &sheet.png).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn positive_id(id: &str) -> bool {
    id.starts_with(['1', '2', '3', '4', '5', '6', '7', '8', '9']) && id.bytes().all(|b| b.is_ascii_digit())
}

fn lock_upload(root: &Path) -> Result<fs::File, String> {
    let dir = root.join("target/icons");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("upload.lock");
    let guard = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    guard.try_lock().map_err(|e| format!("Could not lock {}: {e}", path.display()))?;
    Ok(guard)
}

fn reconcile(atlas: &Atlas, previous: Option<&Lock>) -> Result<Lock, String> {
    let old = previous.map_or(&[][..], |lock| lock.sheets.as_slice());
    let mut sheets = Vec::new();
    for (index, sheet) in atlas.sheets.iter().enumerate() {
        if let Some(entry) = old.iter().find(|entry| entry.sha256 == sheet.sha256) {
            if !positive_id(&entry.asset) {
                return Err(format!("{LOCK_PATH}: invalid asset ID for sheet {index}: {:?}", entry.asset));
            }
            sheets.push(LockedSheet { index, sha256: sheet.sha256.clone(), asset: entry.asset.clone() });
        }
    }
    Ok(Lock { lucide: atlas.version.clone(), cell: CELL, sheets })
}

fn save_lock(root: &Path, lock: &Lock) -> Result<(), String> {
    let path = root.join(LOCK_PATH);
    let text = toml::to_string_pretty(lock).map_err(|e| format!("{LOCK_PATH}: {e}"))?;
    let save = || -> io::Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(path.parent().expect("lock has a parent directory"))?;
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(&path)?;
        Ok(())
    };
    save().map_err(|e| format!("{LOCK_PATH}: {e}"))
}

fn upload(root: &Path, atlas: &Atlas, lock: &mut Lock, previous: Option<&Lock>, group: &str) -> Result<(), String> {
    if previous != Some(lock) {
        save_lock(root, lock)?;
    }
    for (index, sheet) in atlas.sheets.iter().enumerate() {
        if let Some(entry) = lock.sheets.iter().find(|entry| entry.sha256 == sheet.sha256) {
            if entry.index != index && !lock.sheets.iter().any(|entry| entry.index == index) {
                lock.sheets.push(LockedSheet { index, sha256: sheet.sha256.clone(), asset: entry.asset.clone() });
                lock.sheets.sort_unstable_by_key(|entry| entry.index);
                save_lock(root, lock)?;
            }
            continue;
        }
        let path = write_png(root, index, sheet)?;
        let name = format!("Lucide {} sheet {index}", atlas.version);
        let result = Command::new(env!("CARGO"))
            .current_dir(root)
            .args(["run", "-q", "-p", "crucible", "--", "oc", "asset", "upload"])
            .arg(path)
            .args([
                "--group",
                group,
                "--type",
                "image",
                "--open-use",
                "--name",
                &name,
                "--description",
                "Lucide icons for @trioforge/crucible/ui (ISC).",
            ])
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| format!("Upload sheet {index}: {e}"))?;
        let document = String::from_utf8_lossy(&result.stdout);
        if !result.status.success() {
            return Err(format!("Upload sheet {index} failed ({}):\n{document}", result.status));
        }
        let receipt: UploadReceipt = serde_json::from_slice(&result.stdout)
            .map_err(|e| format!("Upload sheet {index} returned an invalid receipt: {e}\n{document}"))?;
        if !positive_id(&receipt.asset_id) {
            return Err(format!("Upload sheet {index} returned an invalid assetId:\n{document}"));
        }
        lock.sheets.push(LockedSheet { index, sha256: sheet.sha256.clone(), asset: receipt.asset_id });
        lock.sheets.sort_unstable_by_key(|entry| entry.index);
        save_lock(root, lock)
            .map_err(|e| format!("Upload sheet {index} succeeded, but could not save its receipt: {e}\n{document}"))?;
        print!("Sheet {index}: {document}");
    }
    Ok(())
}

fn typescript(atlas: &Atlas, lock: &Lock) -> String {
    let mut text = format!(
        "// Generated by cargo xtask icons.\nexport const lucideVersion = \"{}\";\nexport const lucideNames = [\n",
        atlas.version
    );
    for name in &atlas.names {
        writeln!(text, "\t{name:?},").expect("writing to a string succeeds");
    }
    text.push_str("] as const;\nexport const lucideSheets = [\n");
    for sheet in &lock.sheets {
        writeln!(text, "\t\"rbxassetid://{}\",", sheet.asset).expect("writing to a string succeeds");
    }
    text.push_str("] as const;\n");
    text
}
