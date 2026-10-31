use std::path::Path;

fn main() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs");
    println!("cargo::rerun-if-changed={}", docs.display());
    let mut files: Vec<_> = std::fs::read_dir(&docs)
        .expect("the repository has a docs folder")
        .map(|entry| entry.expect("the docs folder is readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect();
    files.sort();
    let topics: String = files
        .iter()
        .map(|path| {
            let name = path.file_stem().and_then(|stem| stem.to_str()).expect("topic names are UTF-8");
            let path = path.to_str().expect("docs paths are UTF-8");
            format!("Topic {{ name: {name:?}, markdown: include_str!({path:?}) }},\n")
        })
        .collect();
    let out = Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("topics.rs");
    std::fs::write(out, format!("[\n{topics}]\n")).expect("OUT_DIR is writable");
}
