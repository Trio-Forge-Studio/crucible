use std::process::{Command, Output};

fn crucible(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crucible")).args(args).output().unwrap()
}

fn topics() -> Vec<String> {
    let docs = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../docs");
    let mut names: Vec<_> = std::fs::read_dir(docs)
        .unwrap()
        .filter_map(|entry| entry.unwrap().file_name().into_string().unwrap().strip_suffix(".md").map(str::to_owned))
        .collect();
    names.sort();
    names
}

#[test]
fn docs_lists_every_topic_with_its_summary() {
    let out = crucible(&["docs"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let listed: Vec<_> = stdout.lines().map(|line| line.split_whitespace().next().unwrap()).collect();
    assert_eq!(listed, topics());
    let oc = stdout.lines().find(|line| line.starts_with("oc ")).unwrap();
    assert!(
        oc.ends_with("  Call Roblox Open Cloud with `crucible oc`, which prints one JSON document per command."),
        "{oc}"
    );
}

#[test]
fn docs_topic_prints_the_file_verbatim() {
    for topic in topics() {
        let out = crucible(&["docs", &topic]);
        assert_eq!(out.status.code(), Some(0));
        let file = std::fs::read(format!("{}/../../../docs/{topic}.md", env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert_eq!(out.stdout, file, "docs {topic}");
    }
}

#[test]
fn unknown_topic_is_a_usage_error_naming_the_topics() {
    let out = crucible(&["docs", "networking"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    for topic in topics() {
        assert!(stderr.contains(&topic), "stderr names {topic}: {stderr}");
    }
}
