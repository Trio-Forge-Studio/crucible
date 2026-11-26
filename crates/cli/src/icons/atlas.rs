use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::sync::LazyLock;

use crucible_transform::{ICON_SIZE, IconAtlas, IconRect};
use flate2::read::GzDecoder;
use resvg::{tiny_skia, usvg};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CELL: u32 = 64;
const GUTTER: u32 = (CELL - ICON_SIZE) / 2;
const COLUMNS: u32 = 16;
const PER_SHEET: usize = (COLUMNS * COLUMNS) as usize;
pub const FILE: &str = "crucible-icons.lock";

static LUCIDE: LazyLock<(String, BTreeMap<String, String>)> = LazyLock::new(|| {
    let mut bytes = Vec::new();
    GzDecoder::new(include_bytes!("../../lucide.json.gz").as_slice())
        .read_to_end(&mut bytes)
        .expect("embedded Lucide gzip is valid");
    serde_json::from_slice(&bytes).expect("embedded Lucide SVGs are valid JSON")
});

fn offset(index: usize) -> (u32, u32) {
    let slot = (index % PER_SHEET) as u32;
    ((slot % COLUMNS) * CELL + GUTTER, (slot / COLUMNS) * CELL + GUTTER)
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    lucide: String,
    stroke: f64,
    cell: u32,
    names: Vec<String>,
    pub sheets: Vec<LockedSheet>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedSheet {
    pub sha256: String,
    pub asset: String,
}

impl Lock {
    pub fn new(stroke: f64, names: BTreeSet<String>, sheets: Vec<LockedSheet>) -> Self {
        Self { lucide: LUCIDE.0.clone(), stroke, cell: CELL, names: names.into_iter().collect(), sheets }
    }

    pub fn read(root: &Path) -> Result<Option<Self>, String> {
        let text = match fs::read_to_string(root.join(FILE)) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Cannot read {FILE}: {error}.")),
        };
        let lock: Self = toml::from_str(&text).map_err(|error| format!("Invalid {FILE}: {error}"))?;
        if lock.cell != CELL
            || lock.names.windows(2).any(|pair| pair[0] >= pair[1])
            || lock.sheets.len() != lock.names.len().div_ceil(PER_SHEET)
            || lock.sheets.iter().any(|sheet| {
                sheet.sha256.len() != 64
                    || !sheet.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    || crate::flows::positive_id(&sheet.asset).is_err()
            })
        {
            return Err(format!("Invalid cell, names, or sheets in {FILE}."));
        }
        Ok(Some(lock))
    }

    pub fn write(&self, root: &Path) -> io::Result<()> {
        let text = toml::to_string_pretty(self).expect("an atlas lock serializes");
        crucible_iac::provision::write_atomic(&root.join(FILE), text.as_bytes())
    }
}

pub fn for_build(root: &Path, stroke: f64) -> Result<Option<IconAtlas>, String> {
    let Some(lock) = Lock::read(root)? else { return Ok(None) };
    if lock.stroke != stroke || lock.lucide != LUCIDE.0 {
        return Err(format!(
            "{FILE} was baked with Lucide {} at stroke {}, but this build uses Lucide {} at stroke {stroke}.",
            lock.lucide, lock.stroke, LUCIDE.0
        ));
    }
    let atlas = lock
        .names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let (x, y) = offset(index);
            let asset = lock.sheets[index / PER_SHEET].asset.clone();
            (name, IconRect { asset, x, y })
        })
        .collect();
    Ok(Some(atlas))
}

pub struct Sheet {
    pub png: Vec<u8>,
    pub sha256: String,
}

pub fn render(names: &BTreeSet<String>, stroke: f64) -> Result<Vec<Sheet>, String> {
    let names: Vec<_> = names.iter().collect();
    let options = usvg::Options::default();
    let mut icon = tiny_skia::Pixmap::new(ICON_SIZE, ICON_SIZE).expect("fixed icon size");
    let mut sheets = Vec::new();
    for batch in names.chunks(PER_SHEET) {
        let rows = (batch.len() as u32).div_ceil(COLUMNS);
        let mut sheet = tiny_skia::Pixmap::new(COLUMNS * CELL, rows * CELL).expect("bounded sheet size");
        for (index, name) in batch.iter().enumerate() {
            let svg = LUCIDE.1.get(*name).ok_or_else(|| format!("Lucide {} has no icon {name:?}.", LUCIDE.0))?;
            let root = svg.find("<svg").expect("embedded SVG has a root");
            let end = root + svg[root..].find('>').expect("embedded SVG root has an end");
            let opening = svg[root..end].replace("stroke-width=\"2\"", &format!("stroke-width=\"{stroke}\""));
            let svg = format!("{opening}{}", &svg[end..]).replace("currentColor", "#ffffff");
            let tree = usvg::Tree::from_str(&svg, &options).map_err(|error| format!("Render {name}: {error}"))?;
            let transform = tiny_skia::Transform::from_scale(
                ICON_SIZE as f32 / tree.size().width(),
                ICON_SIZE as f32 / tree.size().height(),
            );
            icon.fill(tiny_skia::Color::TRANSPARENT);
            resvg::render(&tree, transform, &mut icon.as_mut());
            let (x, y) = offset(index);
            sheet.draw_pixmap(
                x as i32,
                y as i32,
                icon.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            );
        }
        let png = sheet.encode_png().map_err(|error| format!("Encode sheet {}: {error}", sheets.len() + 1))?;
        let sha256 = format!("{:x}", Sha256::digest(&png));
        sheets.push(Sheet { png, sha256 });
    }
    Ok(sheets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_only_needed_rows_and_changes_pixels_with_stroke() {
        let names = ["coins".to_owned(), "house".to_owned(), "settings".to_owned()].into();
        let thin = render(&names, 1.25).unwrap();
        let regular = render(&names, 2.0).unwrap();
        assert_eq!(thin.len(), 1);
        assert_eq!(thin[0].sha256, render(&names, 1.25).unwrap()[0].sha256);
        assert_ne!(thin[0].sha256, regular[0].sha256);
        let image = tiny_skia::Pixmap::decode_png(&thin[0].png).unwrap();
        assert_eq!((image.width(), image.height()), (1024, 64));
        assert!(image.pixels().iter().any(|p| p.alpha() != 0));
        assert!(image.pixels().iter().all(|p| p.red() == p.alpha() && p.green() == p.alpha() && p.blue() == p.alpha()));
    }

    #[test]
    fn splits_at_256_and_derives_rectangles_in_the_lock_across_sheets() {
        let names: BTreeSet<_> = LUCIDE.1.keys().take(257).cloned().collect();
        let sheets = render(&names, 2.0).unwrap();
        assert_eq!(sheets.len(), 2);
        assert_eq!(tiny_skia::Pixmap::decode_png(&sheets[0].png).unwrap().height(), 1024);
        assert_eq!(tiny_skia::Pixmap::decode_png(&sheets[1].png).unwrap().height(), 64);
        let locked = sheets
            .into_iter()
            .enumerate()
            .map(|(index, sheet)| LockedSheet { sha256: sheet.sha256, asset: (index + 1).to_string() })
            .collect();
        let root = tempfile::tempdir().unwrap();
        Lock::new(2.0, names.clone(), locked).write(root.path()).unwrap();
        let atlas = for_build(root.path(), 2.0).unwrap().unwrap();
        let rect = |i| &atlas[names.iter().nth(i).unwrap()];
        assert_eq!(rect(16), &IconRect { asset: "1".into(), x: 2, y: 66 });
        assert_eq!(rect(255), &IconRect { asset: "1".into(), x: 962, y: 962 });
        assert_eq!(rect(256), &IconRect { asset: "2".into(), x: 2, y: 2 });
    }
}
