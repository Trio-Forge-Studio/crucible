//! Splits Rust source into its string literals and the code around them, with comments and `#[cfg(test)]` items
//! dropped, so the rules see only what the crate ships.

pub struct Scan {
    /// Each string literal's text between its quotes, escapes left as written.
    pub strings: Vec<String>,
    /// The source with comments, strings and character literals removed.
    pub code: String,
    /// The names of `#[cfg(test)] mod name;` declarations, whose files hold only tests.
    pub test_modules: Vec<String>,
}

pub fn scan(source: &str) -> Scan {
    let lexed = lex(source);
    let code = lexed.code;
    let mut skipped = Vec::new();
    let mut test_modules = Vec::new();
    let attribute: Vec<char> = "#[cfg(test)]".chars().collect();
    let mut i = 0;
    while i + attribute.len() <= code.len() {
        if code[i..i + attribute.len()] != attribute[..] {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + attribute.len();
        let mut depth = 0usize;
        let mut header = String::new();
        while j < code.len() {
            match code[j] {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                ';' if depth == 0 => {
                    let words: Vec<&str> = header.split_whitespace().collect();
                    if let [.., "mod", name] = words.as_slice() {
                        test_modules.push((*name).to_owned());
                    }
                    break;
                }
                c if depth == 0 => header.push(c),
                _ => {}
            }
            j += 1;
        }
        skipped.push(start..=j);
        i = j + 1;
    }
    let kept = |at: &usize| !skipped.iter().any(|range| range.contains(at));
    Scan {
        strings: lexed.strings.into_iter().filter(|(at, _)| kept(at)).map(|(_, text)| text).collect(),
        code: code.iter().enumerate().filter(|(at, _)| kept(at)).map(|(_, c)| c).collect(),
        test_modules,
    }
}

struct Lexed {
    /// Each string with the index in `code` of the space that stands for it.
    strings: Vec<(usize, String)>,
    code: Vec<char>,
}

fn lex(source: &str) -> Lexed {
    let chars: Vec<char> = source.chars().collect();
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let mut scan = Lexed { strings: Vec::new(), code: Vec::with_capacity(source.len()) };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let after_identifier = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && at(i + 1) == '*' {
            let mut depth = 0;
            loop {
                match (at(i), at(i + 1)) {
                    ('\0', _) => break,
                    ('/', '*') => (depth, i) = (depth + 1, i + 2),
                    ('*', '/') => {
                        (depth, i) = (depth - 1, i + 2);
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => i += 1,
                }
            }
            scan.code.push(' ');
            continue;
        }
        if !after_identifier {
            // r"..", r#".."#, br"..", cr"..".
            let prefix = match (c, at(i + 1)) {
                ('r', _) => Some(1),
                ('b' | 'c', 'r') => Some(2),
                _ => None,
            };
            if let Some(prefix) = prefix {
                let mut j = i + prefix;
                while at(j) == '#' {
                    j += 1;
                }
                let hashes = j - i - prefix;
                if at(j) == '"' {
                    let start = j + 1;
                    let mut end = start;
                    while end < chars.len() && !(chars[end] == '"' && (1..=hashes).all(|k| at(end + k) == '#')) {
                        end += 1;
                    }
                    scan.strings.push((scan.code.len(), chars[start..end.min(chars.len())].iter().collect()));
                    scan.code.push(' ');
                    i = end + 1 + hashes;
                    continue;
                }
            }
        }
        let quoted = match (c, at(i + 1)) {
            ('"', _) => Some(i + 1),
            ('b' | 'c', '"') if !after_identifier => Some(i + 2),
            _ => None,
        };
        if let Some(start) = quoted {
            let mut end = start;
            while end < chars.len() && chars[end] != '"' {
                end += if chars[end] == '\\' { 2 } else { 1 };
            }
            scan.strings.push((scan.code.len(), chars[start..end.min(chars.len())].iter().collect()));
            scan.code.push(' ');
            i = end + 1;
            continue;
        }
        if c == '\'' {
            // '\n', '\'', '\u{..}' and 'x' are characters. Anything else, such as 'a in a lifetime, is code.
            if at(i + 1) == '\\' {
                let mut end = i + 3;
                while end < chars.len() && chars[end] != '\'' {
                    end += 1;
                }
                i = end + 1;
                scan.code.push(' ');
                continue;
            }
            if at(i + 2) == '\'' {
                i += 3;
                scan.code.push(' ');
                continue;
            }
        }
        scan.code.push(c);
        i += 1;
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::scan;

    #[test]
    fn strings_are_found_and_comments_are_not() {
        let source = r####"
            // "in a line comment"
            /* "in a /* nested */ block comment" */
            let a = "plain \" with an escaped quote";
            let b = r#"raw with "quotes""#;
            let c = b"bytes";
            let d = '"';
            let e = '\'';
            fn f<'a>(x: &'a str) -> char { 'x' }
            let g = br##"raw bytes"##;
            let h = std::env::var("HOME");
        "####;
        let found = scan(source);
        assert_eq!(
            found.strings,
            ["plain \\\" with an escaped quote", "raw with \"quotes\"", "bytes", "raw bytes", "HOME"]
        );
        assert!(found.code.contains("fn f<'a>(x: &'a str)"));
        assert!(found.code.contains("std::env::var("));
        assert!(!found.code.contains("line comment"));
    }

    #[test]
    fn test_items_are_dropped() {
        let source = "fn a() { \"kept\" }\n#[cfg(test)]\nmod tests {\n    fn b() { let x = { \"inner\" }; }\n}\n#[cfg(test)]\nmod more;\nfn c() { \"also kept\" }\n";
        let found = scan(source);
        assert_eq!(found.strings, ["kept", "also kept"]);
        assert_eq!(found.test_modules, ["more"]);
    }
}
