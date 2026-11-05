use std::path::Path;

/// The reader picks the widest of these that fits the page, so the diagrams cost no Mermaid renderer in the binary.
const DIAGRAM_WIDTHS: [usize; 4] = [40, 60, 80, 100];

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
    let mut diagrams = String::new();
    for path in &files {
        let markdown = std::fs::read_to_string(path).expect("docs files are UTF-8");
        for source in mermaid_fences(&markdown) {
            let renders: Vec<String> = DIAGRAM_WIDTHS
                .iter()
                .map(|&width| match mermaid_text::render_with_width(&source, Some(width)) {
                    Ok(diagram) => format!("{diagram:?}"),
                    Err(error) => panic!("{} has a mermaid block that does not render: {error}", path.display()),
                })
                .collect();
            diagrams += &format!("({source:?}, &[{}]),\n", renders.join(", "));
        }
    }
    let out = Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).to_owned();
    std::fs::write(out.join("topics.rs"), format!("[\n{topics}]\n")).expect("OUT_DIR is writable");
    std::fs::write(out.join("diagrams.rs"), format!("[\n{diagrams}]\n")).expect("OUT_DIR is writable");
}

fn mermaid_fences(markdown: &str) -> Vec<String> {
    let mut fences = Vec::new();
    let mut open: Option<String> = None;
    for line in markdown.lines() {
        match open.as_mut() {
            None if line == "```mermaid" => open = Some(String::new()),
            None => {}
            Some(_) if line == "```" => fences.extend(open.take()),
            Some(source) => {
                source.push_str(line);
                source.push('\n');
            }
        }
    }
    fences
}
