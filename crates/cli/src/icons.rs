mod atlas;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use crucible_oc::auth;
use crucible_oc::outcome::{Acceptance, Failure};
use crucible_oc::transport::Timing;
use serde::{Deserialize, Serialize};

use crate::build;
use crate::exec::Executor;
use crate::flows::upload;
use crate::output::Output;
use atlas::{Lock, LockedSheet, Sheet};

pub use atlas::{FILE, for_build};

const GUARD: &str = ".crucible/icons-upload.lock";
const JOURNAL: &str = ".crucible/icons-upload.json";

#[derive(clap::Args)]
pub struct Args {
    /// Own any newly uploaded images under this group. Required only to start new uploads.
    #[arg(long, value_name = "GROUP_ID", value_parser = crate::flows::positive_id)]
    group: Option<String>,
}

#[derive(Serialize, Deserialize)]
enum Upload {
    Sending,
    Accepted(String),
    Uploaded(String),
}

pub fn main(args: Args) -> ExitCode {
    let mut output = Output::new(false);
    let result = std::env::current_dir()
        .map_err(|error| io_failure("current directory", error))
        .and_then(|root| run(&root, args, &mut output));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            output.note(&format!("crucible icons: {}", failure.error));
            ExitCode::from(failure.code)
        }
    }
}

fn run(root: &Path, args: Args, output: &mut Output) -> Result<(), Failure> {
    let guard_path = root.join(GUARD);
    fs::create_dir_all(guard_path.parent().expect("guard has a parent")).map_err(|e| io_failure(GUARD, e))?;
    let guard = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(guard_path)
        .map_err(|e| io_failure(GUARD, e))?;
    guard.try_lock().map_err(|e| Failure::unavailable(format!("Cannot lock {GUARD}: {e}")))?;

    let build_failure = |error: build::Error| Failure::new(error.code(), "compile", error.to_string());
    let manifest = build::manifest(root).map_err(build_failure)?;
    let names = build::collect_icons(root, &manifest).map_err(build_failure)?;
    let stroke = manifest.icons.stroke;
    let old = Lock::read(root).map_err(Failure::usage)?;
    if names.is_empty() && old.is_none() {
        return Ok(());
    }
    let sheets = atlas::render(&names, stroke).map_err(Failure::usage)?;
    let mut assets: BTreeMap<_, _> =
        old.iter().flat_map(|lock| &lock.sheets).map(|s| (s.sha256.clone(), s.asset.clone())).collect();
    let journal_path = root.join(JOURNAL);
    let mut journal: BTreeMap<String, Upload> = match fs::read(&journal_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| io_failure(JOURNAL, e))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => return Err(io_failure(JOURNAL, e)),
    };
    for (hash, entry) in &journal {
        if let Upload::Uploaded(asset) = entry {
            crate::flows::positive_id(asset).map_err(|e| io_failure(JOURNAL, e))?;
            assets.entry(hash.clone()).or_insert_with(|| asset.clone());
        }
    }
    let mut missing = Vec::new();
    let mut starts = Vec::new();
    for (index, sheet) in sheets.iter().enumerate() {
        let hash = &sheet.sha256;
        if assets.contains_key(hash) {
            continue;
        }
        match journal.get(hash) {
            Some(Upload::Sending) => {
                return Err(Failure::unknown(format!(
                    "Sheet {} ({hash}) has an unresolved upload in {JOURNAL}. Check the group's Creations.",
                    index + 1
                )));
            }
            Some(Upload::Accepted(_)) => {}
            None => starts.push(format!("sheet {} ({hash})", index + 1)),
            Some(Upload::Uploaded(_)) => unreachable!("journal assets have been loaded"),
        }
        missing.push((index, sheet));
    }
    if !starts.is_empty() && args.group.is_none() {
        return Err(Failure::usage(format!("Set --group <groupId> to upload {}.", starts.join(", "))));
    }

    if !missing.is_empty() {
        let (source, key) = crate::load_key(root)?;
        output.redact_also(&key);
        let timing = Timing {
            deadline: Duration::from_secs_f64(crate::cli::DEFAULT_DEADLINE),
            poll_interval: Duration::from_secs_f64(crate::cli::DEFAULT_POLL_INTERVAL),
            retry_delay: None,
        };
        for (index, sheet) in missing {
            if assets.contains_key(&sheet.sha256) {
                continue;
            }
            let acceptance = RefCell::new(Acceptance::default());
            let ex = Executor::new(
                &crate::REGISTRY,
                std::env::var("CRUCIBLE_OC_ORIGIN").ok(),
                Some(&key),
                timing,
                output,
                &acceptance,
            )?;
            let asset = upload_sheet(&ex, &acceptance, &journal_path, &mut journal, index, sheet, args.group.clone())
                .map_err(|failure| {
                auth::decorate(
                    failure,
                    &crate::flows::op(&crate::REGISTRY, crate::flows::CREATE_ASSET).auth,
                    Some(&source),
                )
            })?;
            assets.insert(sheet.sha256.clone(), asset);
        }
    }

    let sheets = sheets
        .into_iter()
        .map(|sheet| LockedSheet { asset: assets[&sheet.sha256].clone(), sha256: sheet.sha256 })
        .collect::<Vec<_>>();
    let published: BTreeSet<_> = sheets.iter().map(|sheet| sheet.sha256.clone()).collect();
    let lock = Lock::new(stroke, names, sheets);
    if old.as_ref() != Some(&lock) {
        lock.write(root).map_err(|e| io_failure(FILE, e))?;
        println!("Updated {}. Commit it with your game.", FILE);
    }
    let before = journal.len();
    journal.retain(|hash, _| !published.contains(hash));
    if before != journal.len() {
        save_journal(&journal_path, &journal)?;
    }
    Ok(())
}

fn upload_sheet(
    ex: &Executor<'_>,
    acceptance: &RefCell<Acceptance>,
    journal_path: &Path,
    journal: &mut BTreeMap<String, Upload>,
    index: usize,
    sheet: &Sheet,
    group: Option<String>,
) -> Result<String, Failure> {
    let hash = &sheet.sha256;
    let resume = match journal.get(hash) {
        Some(Upload::Accepted(id)) => Some(upload::operation_id(id).map_err(|e| io_failure(JOURNAL, e))?),
        _ => None,
    };
    let mut png = None;
    if resume.is_none() {
        let mut file = tempfile::Builder::new().suffix(".png").tempfile().map_err(|e| io_failure(hash, e))?;
        file.write_all(&sheet.png).map_err(|e| io_failure(hash, e))?;
        png = Some(file);
    }
    let creating = resume.is_none();
    let args = upload::Args {
        file: png.as_ref().map(|file| file.path().to_owned()),
        resume,
        asset_type: Some(upload::AssetType::Image),
        open_use: true,
        name: Some(format!("Lucide icons {} {}", index + 1, hash)),
        description: "Lucide icons (ISC).".into(),
        group,
        user: None,
    };
    let result = upload::run_recording(ex, args, |event| {
        let next = match event {
            upload::UploadEvent::Sending => Upload::Sending,
            upload::UploadEvent::Accepted(id) => Upload::Accepted(id.to_owned()),
        };
        journal.insert(hash.clone(), next);
        save_journal(journal_path, journal)
    });
    let receipt = match acceptance.borrow().finish(result) {
        Ok(receipt) => receipt,
        Err(failure) => {
            if creating && failure.code != 5 && matches!(journal.get(hash), Some(Upload::Sending)) {
                journal.remove(hash);
                save_journal(journal_path, journal)?;
            }
            return Err(failure);
        }
    };
    let asset = receipt["assetId"].as_str().expect("upload validates the asset ID").to_owned();
    journal.insert(hash.clone(), Upload::Uploaded(asset.clone()));
    save_journal(journal_path, journal)?;
    Ok(asset)
}

fn save_journal(path: &Path, journal: &BTreeMap<String, Upload>) -> Result<(), Failure> {
    if journal.is_empty() {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_failure(JOURNAL, error)),
        };
    }
    let bytes = serde_json::to_vec_pretty(journal).expect("an upload journal serializes");
    crucible_iac::provision::write_atomic(path, &bytes).map_err(|e| io_failure(JOURNAL, e))
}

fn io_failure(path: &str, error: impl std::fmt::Display) -> Failure {
    Failure::unavailable(format!("{path}: {error}"))
}
