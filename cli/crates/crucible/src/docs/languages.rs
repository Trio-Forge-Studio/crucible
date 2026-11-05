/// The syntax token a fence's language highlights with, or `None` for a fence drawn as plain text.
/// `build.rs` bundles a syntax for each token the docs use, so this table is the only list of aliases.
pub fn syntax_token(language: &str) -> Option<&str> {
    match language {
        "" | "text" | "txt" | "plain" | "mermaid" => None,
        "luau" => Some("lua"),
        "console" | "shell" | "sh" => Some("bash"),
        other => Some(other),
    }
}
