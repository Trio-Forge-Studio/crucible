use crucible_iac::journal::{self, Intent, Journal};
use crucible_iac::lock::{self, Lock};
use crucible_iac::model::Kind;
use crucible_iac::provision::{Files, Store};
use std::num::NonZeroU64;

fn universe() -> NonZeroU64 {
    NonZeroU64::new(7).unwrap()
}

#[test]
fn the_lock_is_written_whole_over_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(lock::FILE), "old").unwrap();
    let mut lock = Lock::empty();
    lock.env_mut("staging", universe());
    Files { root: dir.path().into() }.save_lock(&lock).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join(lock::FILE)).unwrap(), lock.render());
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(left, [lock::FILE], "no temporary file is left");
}

#[test]
fn a_journal_with_intents_is_written_and_an_empty_one_removes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = Files { root: dir.path().into() };
    let path = dir.path().join(journal::path("staging"));
    let mut journal = Journal::new(universe());
    files.save_journal("staging", &journal).unwrap();
    assert!(!path.exists(), "an empty journal never creates the file");

    journal.intents.push(Intent { kind: Kind::Pass, key: "vip".into(), name: "VIP".into(), before: None });
    files.save_journal("staging", &journal).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), journal.render());

    journal.intents.clear();
    files.save_journal("staging", &journal).unwrap();
    assert!(!path.exists());
}

#[test]
fn a_journal_that_cannot_be_removed_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(journal::path("staging"));
    std::fs::create_dir_all(&path).unwrap();
    let error = Files { root: dir.path().into() }.save_journal("staging", &Journal::new(universe())).unwrap_err();
    assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(path.exists());
}

#[test]
fn a_second_write_replaces_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("file.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    crucible_iac::provision::write_atomic(&path, b"one").unwrap();
    crucible_iac::provision::write_atomic(&path, b"two").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"two");
}
