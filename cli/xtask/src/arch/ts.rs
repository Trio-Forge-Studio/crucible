//! Rule 5: the runtime root never reaches the server, client, emit or testing entries.
//!
//! Each file the root reaches is split into tokens first, so comments, strings, template literals and regular
//! expressions can never merge two statements or hide one. The token stream is then read for `import` and `export`
//! declarations, `import(..)` calls and `require(..)` calls with a string literal. A statement that imports only types
//! is skipped, because the compiler erases it. Any other import counts, even one whose names the compiler would erase as
//! unused, so the check over-reports rather than miss a value import. Every `import(..)` counts, even one in a type,
//! because `typeof import("x")` and `import("x").Name` are expressions as well as types and the tokens alone cannot
//! tell which. Name such a type with `import type` instead. A declaration the reader does not recognize, a computed
//! `import(..)` and an unterminated string, comment, template or regular expression fail the check instead of being
//! skipped.
//!
//! Its limits: a `require` of a non-literal, such as the boot glob's `require(module)`, loads game modules at run time
//! and is not an edge of this graph. A `/` right after `)` is read as division, as in most JavaScript tokenizers, so a
//! regular expression at the start of an `if` body would be misread. The runtime has no JSX files, and JSX text would
//! fail the tokenizer.

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use crate::run::{normalize, slashed};

const SOURCE: &str = "packages/crucible/src";
const ROOT: &str = "packages/crucible/src/index.ts";
const PACKAGE: &str = "@trioforge/crucible";
/// The side entries the root must never load.
const FORBIDDEN: &[&str] = &["server", "client", "emit", "testing"];
/// Keywords after which a `/` starts a regular expression rather than a division.
const BEFORE_REGEX: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
];

/// Each import edge from a file the root reaches into a forbidden entry, as repository-relative paths.
pub fn root_reaches(repo: &Path) -> Result<Vec<(String, String)>, String> {
    let repo = &normalize(repo);
    let source = repo.join(SOURCE);
    let forbidden = |path: &Path| {
        path.strip_prefix(&source)
            .ok()
            .and_then(|relative| relative.components().next())
            .is_some_and(|first| FORBIDDEN.iter().any(|entry| first.as_os_str() == *entry))
    };
    let shown = |path: &Path| slashed(path.strip_prefix(repo).unwrap_or(path));
    let mut found = Vec::new();
    let mut seen = BTreeSet::from([repo.join(ROOT)]);
    let mut queue = VecDeque::from([repo.join(ROOT)]);
    while let Some(file) = queue.pop_front() {
        for statement in read(&file)?.statements {
            let Some(specifier) = statement.specifier.filter(|_| !statement.types_only) else { continue };
            let Some(target) = resolve(&source, &file, &specifier)? else { continue };
            if forbidden(&target) {
                found.push((shown(&file), shown(&target)));
            } else if seen.insert(target.clone()) {
                queue.push_back(target);
            }
        }
    }
    Ok(found)
}

/// The value names a runtime source file exports, following `export *`.
pub fn value_exports(repo: &Path, file: &Path) -> Result<BTreeSet<String>, String> {
    let parsed = read(file)?;
    let mut names: BTreeSet<String> = parsed.declared.into_iter().collect();
    for statement in parsed.statements {
        if !statement.export || statement.types_only {
            continue;
        }
        if statement.star
            && let Some(specifier) = &statement.specifier
            && let Some(target) = resolve(&repo.join(SOURCE), file, specifier)?
        {
            names.extend(value_exports(repo, &target)?);
        }
        names.extend(statement.values);
    }
    Ok(names)
}

#[derive(Debug, Default, PartialEq)]
struct Statement {
    export: bool,
    /// `export * from`.
    star: bool,
    /// `None` for a local `export { .. }`.
    specifier: Option<String>,
    types_only: bool,
    /// The value names an `export { .. }` or `export * as` exports, after any `as`.
    values: Vec<String>,
}

#[derive(Debug, Default)]
struct Parsed {
    statements: Vec<Statement>,
    /// The names of exported declarations, such as `export function a` and `export const b`.
    declared: Vec<String>,
}

fn read(file: &Path) -> Result<Parsed, String> {
    let text = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", file.display()))
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Str(String),
    Punct(char),
    /// A number, template literal or regular expression.
    Other,
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut lexer = Lexer { chars: text.chars().collect(), at: 0, tokens: Vec::new() };
    lexer.run(false)?;
    Ok(lexer.tokens)
}

struct Lexer {
    chars: Vec<char>,
    at: usize,
    tokens: Vec<Token>,
}

impl Lexer {
    fn peek(&self, offset: usize) -> char {
        self.chars.get(self.at + offset).copied().unwrap_or('\0')
    }

    fn line(&self) -> usize {
        self.chars[..self.at.min(self.chars.len())].iter().filter(|c| **c == '\n').count() + 1
    }

    /// With `in_template`, stops after the `}` that closes a `${` expression.
    fn run(&mut self, in_template: bool) -> Result<(), String> {
        let mut depth = 0usize;
        while self.at < self.chars.len() {
            let c = self.peek(0);
            match c {
                c if c.is_whitespace() => self.at += 1,
                '/' if self.peek(1) == '/' => {
                    while self.at < self.chars.len() && self.peek(0) != '\n' {
                        self.at += 1;
                    }
                }
                '/' if self.peek(1) == '*' => {
                    let start = self.line();
                    self.at += 2;
                    while !(self.peek(0) == '*' && self.peek(1) == '/') {
                        if self.at >= self.chars.len() {
                            return Err(format!("line {start}: unterminated comment"));
                        }
                        self.at += 1;
                    }
                    self.at += 2;
                }
                '/' if self.regex_allowed() => self.regex()?,
                '"' | '\'' => {
                    let text = self.string(c)?;
                    self.tokens.push(Token::Str(text));
                }
                '`' => self.template()?,
                c if c.is_ascii_digit() => {
                    while self.peek(0).is_alphanumeric() || self.peek(0) == '.' || self.peek(0) == '_' {
                        self.at += 1;
                    }
                    self.tokens.push(Token::Other);
                }
                c if c.is_alphabetic() || c == '_' || c == '$' => {
                    let start = self.at;
                    while self.peek(0).is_alphanumeric() || self.peek(0) == '_' || self.peek(0) == '$' {
                        self.at += 1;
                    }
                    self.tokens.push(Token::Word(self.chars[start..self.at].iter().collect()));
                }
                '{' => {
                    depth += 1;
                    self.at += 1;
                    self.tokens.push(Token::Punct(c));
                }
                '}' if in_template && depth == 0 => {
                    self.at += 1;
                    return Ok(());
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    self.at += 1;
                    self.tokens.push(Token::Punct(c));
                }
                _ => {
                    self.at += 1;
                    self.tokens.push(Token::Punct(c));
                }
            }
        }
        if in_template { Err("unterminated template literal".into()) } else { Ok(()) }
    }

    fn regex_allowed(&self) -> bool {
        match self.tokens.last() {
            None => true,
            Some(Token::Punct(c)) => !matches!(c, ')' | ']' | '}'),
            Some(Token::Word(word)) => BEFORE_REGEX.contains(&word.as_str()),
            Some(Token::Str(_) | Token::Other) => false,
        }
    }

    fn regex(&mut self) -> Result<(), String> {
        let line = self.line();
        self.at += 1;
        let mut class = false;
        loop {
            match self.peek(0) {
                '\0' | '\n' => return Err(format!("line {line}: unterminated regular expression")),
                '\\' => self.at += 1,
                '[' => class = true,
                ']' => class = false,
                '/' if !class => break,
                _ => {}
            }
            self.at += 1;
        }
        self.at += 1;
        while self.peek(0).is_alphanumeric() {
            self.at += 1;
        }
        self.tokens.push(Token::Other);
        Ok(())
    }

    fn string(&mut self, quote: char) -> Result<String, String> {
        let line = self.line();
        self.at += 1;
        let mut text = String::new();
        loop {
            let c = self.peek(0);
            if self.at >= self.chars.len() || c == '\n' {
                return Err(format!("line {line}: unterminated string"));
            }
            self.at += 1;
            match c {
                c if c == quote => return Ok(text),
                '\\' => {
                    let escaped = self.peek(0);
                    self.at += 1;
                    match escaped {
                        'n' => text.push('\n'),
                        't' => text.push('\t'),
                        'r' => text.push('\r'),
                        'b' => text.push('\u{8}'),
                        'f' => text.push('\u{c}'),
                        'v' => text.push('\u{b}'),
                        '0' => text.push('\0'),
                        '\n' => {}
                        'x' => text.push(self.code_point(2)?),
                        'u' if self.peek(0) == '{' => {
                            self.at += 1;
                            let start = self.at;
                            while self.peek(0) != '}' && self.at < self.chars.len() {
                                self.at += 1;
                            }
                            let digits: String = self.chars[start..self.at].iter().collect();
                            self.at += 1;
                            text.push(hex(&digits).ok_or_else(|| format!("line {line}: bad escape"))?);
                        }
                        'u' => text.push(self.code_point(4)?),
                        other => text.push(other),
                    }
                }
                c => text.push(c),
            }
        }
    }

    fn code_point(&mut self, digits: usize) -> Result<char, String> {
        let text: String = self.chars[self.at..(self.at + digits).min(self.chars.len())].iter().collect();
        self.at += digits;
        hex(&text).ok_or_else(|| format!("line {}: bad escape", self.line()))
    }

    fn template(&mut self) -> Result<(), String> {
        let line = self.line();
        self.at += 1;
        self.tokens.push(Token::Other);
        loop {
            match self.peek(0) {
                '\0' if self.at >= self.chars.len() => {
                    return Err(format!("line {line}: unterminated template literal"));
                }
                '\\' => self.at += 2,
                '`' => {
                    self.at += 1;
                    return Ok(());
                }
                '$' if self.peek(1) == '{' => {
                    self.at += 2;
                    self.run(true)?;
                    self.tokens.push(Token::Other);
                }
                _ => self.at += 1,
            }
        }
    }
}

fn hex(digits: &str) -> Option<char> {
    u32::from_str_radix(digits, 16).ok().and_then(char::from_u32)
}

fn parse(text: &str) -> Result<Parsed, String> {
    let tokens = tokenize(text)?;
    let word = |at: usize, expected: &str| matches!(tokens.get(at), Some(Token::Word(w)) if w == expected);
    let punct = |at: usize, expected: char| tokens.get(at) == Some(&Token::Punct(expected));
    let string = |at: usize| match tokens.get(at) {
        Some(Token::Str(text)) => Some(text.clone()),
        _ => None,
    };
    let mut parsed = Parsed::default();
    for at in 0..tokens.len() {
        let Token::Word(keyword) = &tokens[at] else { continue };
        if at > 0 && punct(at - 1, '.') {
            continue;
        }
        match keyword.as_str() {
            "import" if punct(at + 1, '(') => {
                let Some(specifier) = string(at + 2).filter(|_| punct(at + 3, ')')) else {
                    return Err("a computed `import(..)`, which this check cannot follow".into());
                };
                parsed.statements.push(Statement { specifier: Some(specifier), ..Statement::default() });
            }
            "import" if punct(at + 1, '.') || punct(at + 1, ':') => {}
            "import" => parsed.statements.push(import(&tokens, at + 1)?),
            // `import a = require("x")` is read as an import already.
            "require" if at >= 3 && punct(at - 1, '=') && (word(at - 3, "import") || word(at - 3, "type")) => {}
            "require" if punct(at + 1, '(') && punct(at + 3, ')') => {
                if let Some(specifier) = string(at + 2) {
                    parsed.statements.push(Statement { specifier: Some(specifier), ..Statement::default() });
                }
            }
            "export" if !punct(at + 1, ':') => export(&tokens, at + 1, &mut parsed)?,
            _ => {}
        }
    }
    Ok(parsed)
}

fn unexpected(tokens: &[Token], at: usize, what: &str) -> String {
    let near: Vec<String> = tokens[at.min(tokens.len())..(at + 6).min(tokens.len())]
        .iter()
        .map(|token| match token {
            Token::Word(word) => word.clone(),
            Token::Str(text) => format!("{text:?}"),
            Token::Punct(c) => c.to_string(),
            Token::Other => "…".into(),
        })
        .collect();
    format!("cannot read the {what} at `{}`", near.join(" "))
}

/// An `import` declaration whose first token after `import` is at `at`.
fn import(tokens: &[Token], mut at: usize) -> Result<Statement, String> {
    let word = |at: usize, expected: &str| matches!(tokens.get(at), Some(Token::Word(w)) if w == expected);
    let punct = |at: usize, expected: char| tokens.get(at) == Some(&Token::Punct(expected));
    let string = |at: usize| match tokens.get(at) {
        Some(Token::Str(text)) => Some(text.clone()),
        _ => None,
    };
    if let Some(specifier) = string(at) {
        return Ok(Statement { specifier: Some(specifier), ..Statement::default() });
    }
    // `import type from "x"`, `import type, { a } from "x"` and `import type = require("x")` bind a value named `type`.
    let explicit_type = word(at, "type")
        && !(word(at + 1, "from") && string(at + 2).is_some())
        && !punct(at + 1, ',')
        && !punct(at + 1, '=');
    if explicit_type {
        at += 1;
    }
    let mut outside = false;
    let mut named = Vec::new();
    loop {
        match tokens.get(at) {
            Some(Token::Word(w)) if w == "from" && string(at + 1).is_some() => {
                let all_types = !named.is_empty() && named.iter().all(|(is_type, _)| *is_type);
                return Ok(Statement {
                    specifier: string(at + 1),
                    types_only: explicit_type || (all_types && !outside),
                    ..Statement::default()
                });
            }
            Some(Token::Word(_)) if punct(at + 1, '=') => {
                if word(at + 2, "require") && punct(at + 3, '(') && punct(at + 5, ')') {
                    return Ok(Statement {
                        specifier: string(at + 4),
                        types_only: explicit_type,
                        ..Statement::default()
                    });
                }
                // `import A = B.C` aliases a namespace and loads no module.
                return Ok(Statement::default());
            }
            Some(Token::Punct('*')) if word(at + 1, "as") && matches!(tokens.get(at + 2), Some(Token::Word(_))) => {
                outside = true;
                at += 3;
            }
            Some(Token::Punct('{')) => {
                let (entries, end) = entries(tokens, at)?;
                named.extend(entries);
                at = end;
            }
            Some(Token::Punct(',')) => at += 1,
            Some(Token::Word(_)) => {
                outside = true;
                at += 1;
            }
            _ => return Err(unexpected(tokens, at, "import")),
        }
    }
}

/// The entries of the `{ .. }` that opens at `open`, each as whether it is `type` and the name it binds or exports,
/// and the index after the `}`.
fn entries(tokens: &[Token], open: usize) -> Result<(Vec<(bool, String)>, usize), String> {
    let close = (open..tokens.len())
        .find(|at| tokens[*at] == Token::Punct('}'))
        .ok_or_else(|| unexpected(tokens, open, "braces"))?;
    let mut entries = Vec::new();
    for entry in tokens[open + 1..close].split(|token| *token == Token::Punct(',')) {
        let names: Vec<&str> = entry
            .iter()
            .map(|token| match token {
                Token::Word(word) => Ok(word.as_str()),
                Token::Str(text) => Ok(text.as_str()),
                _ => Err(unexpected(tokens, open, "braces")),
            })
            .collect::<Result<_, _>>()?;
        let is_type = match names.as_slice() {
            [] => continue,
            ["type", "as", _] | ["type"] => false,
            ["type", ..] => true,
            _ => false,
        };
        entries.push((is_type, (*names.last().unwrap_or(&"")).to_owned()));
    }
    Ok((entries, close + 1))
}

/// An `export` whose first token after `export` is at `at`.
fn export(tokens: &[Token], mut at: usize, parsed: &mut Parsed) -> Result<(), String> {
    let word = |at: usize, expected: &str| matches!(tokens.get(at), Some(Token::Word(w)) if w == expected);
    let punct = |at: usize, expected: char| tokens.get(at) == Some(&Token::Punct(expected));
    let string = |at: usize| match tokens.get(at) {
        Some(Token::Str(text)) => Some(text.clone()),
        _ => None,
    };
    let name = |at: usize| match tokens.get(at) {
        Some(Token::Word(word)) => Some(word.clone()),
        _ => None,
    };
    let explicit_type = word(at, "type") && (punct(at + 1, '{') || punct(at + 1, '*'));
    if explicit_type {
        at += 1;
    }
    if punct(at, '*') {
        let (star, values, from) = if word(at + 1, "as") {
            let alias = name(at + 2).or_else(|| string(at + 2)).ok_or_else(|| unexpected(tokens, at, "export"))?;
            (false, vec![alias], at + 3)
        } else {
            (true, Vec::new(), at + 1)
        };
        let specifier = string(from + 1).filter(|_| word(from, "from"));
        if specifier.is_none() {
            return Err(unexpected(tokens, at, "export"));
        }
        let values = if explicit_type { Vec::new() } else { values };
        parsed.statements.push(Statement { export: true, star, specifier, types_only: explicit_type, values });
        return Ok(());
    }
    if punct(at, '{') {
        let (entries, end) = entries(tokens, at)?;
        let specifier = string(end + 1).filter(|_| word(end, "from"));
        let values = if explicit_type {
            Vec::new()
        } else {
            entries.iter().filter(|(is_type, _)| !is_type).map(|(_, name)| name.clone()).collect()
        };
        let all_types = !entries.is_empty() && entries.iter().all(|(is_type, _)| *is_type);
        parsed.statements.push(Statement {
            export: true,
            star: false,
            specifier,
            types_only: explicit_type || all_types,
            values,
        });
        return Ok(());
    }
    while word(at, "declare") || word(at, "async") || word(at, "abstract") {
        at += 1;
    }
    let declared = match tokens.get(at) {
        Some(Token::Word(keyword)) => match keyword.as_str() {
            "function" | "class" | "enum" | "namespace" | "module" => {
                let at = if punct(at + 1, '*') { at + 2 } else { at + 1 };
                name(at).into_iter().collect()
            }
            "const" if word(at + 1, "enum") => name(at + 2).into_iter().collect(),
            "const" | "let" | "var" => declarators(tokens, at + 1),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    parsed.declared.extend(declared);
    Ok(())
}

/// The names `const a = .., { b, c: d } = ..;` binds, starting after the keyword.
fn declarators(tokens: &[Token], mut at: usize) -> Vec<String> {
    let mut names = Vec::new();
    loop {
        match tokens.get(at) {
            Some(Token::Word(name)) => {
                names.push(name.clone());
                at += 1;
            }
            Some(Token::Punct(open @ ('{' | '['))) => {
                let close = if *open == '{' { '}' } else { ']' };
                let mut depth = 0usize;
                let start = at;
                while let Some(token) = tokens.get(at) {
                    match token {
                        Token::Punct(c) if c == open => depth += 1,
                        Token::Punct(c) if *c == close => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    at += 1;
                }
                for pattern in start + 1..at {
                    if let Token::Word(name) = &tokens[pattern]
                        && matches!(tokens.get(pattern + 1), Some(Token::Punct(',' | '}' | ']' | '=')))
                    {
                        names.push(name.clone());
                    }
                }
                at += 1;
            }
            _ => return names,
        }
        // Skip the type and the initializer to the comma that starts the next declarator.
        let mut depth = 0usize;
        loop {
            match tokens.get(at) {
                None | Some(Token::Punct(';')) if depth == 0 => return names,
                None => return names,
                Some(Token::Punct('(' | '[' | '{')) => depth += 1,
                Some(Token::Punct(')' | ']' | '}')) => depth = depth.saturating_sub(1),
                Some(Token::Punct(',')) if depth == 0 => {
                    let next_binds = match tokens.get(at + 1) {
                        Some(Token::Word(_)) => {
                            matches!(tokens.get(at + 2), Some(Token::Punct(':' | '=' | ',' | ';')) | None)
                        }
                        Some(Token::Punct('{' | '[')) => true,
                        _ => false,
                    };
                    if next_binds {
                        at += 1;
                        break;
                    }
                }
                _ => {}
            }
            at += 1;
        }
    }
}

/// The source file a specifier names, or `None` for another package.
fn resolve(source: &Path, importer: &Path, specifier: &str) -> Result<Option<PathBuf>, String> {
    let base = if specifier.starts_with('.') {
        importer.parent().unwrap_or(importer).join(specifier)
    } else if specifier == PACKAGE {
        source.to_path_buf()
    } else if let Some(sub) = specifier.strip_prefix(PACKAGE).and_then(|rest| rest.strip_prefix('/')) {
        source.join(sub)
    } else {
        return Ok(None);
    };
    let with = |suffix: &str| {
        let mut path = base.clone().into_os_string();
        path.push(suffix);
        PathBuf::from(path)
    };
    let file = [with(".ts"), with(".tsx"), with(".d.ts"), base.join("index.ts"), base.join("index.tsx")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("{}: cannot resolve the import of {specifier}", importer.display()))?;
    Ok(Some(normalize(&file)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(text: &str) -> Vec<Statement> {
        parse(text).unwrap().statements
    }

    fn one(text: &str) -> Statement {
        let mut found = all(text);
        assert_eq!(found.len(), 1, "{text}");
        found.remove(0)
    }

    #[test]
    fn type_only_statements_are_told_apart_from_value_imports() {
        assert!(one("import type { A } from \"./a\";").types_only);
        assert!(one("import { type A, type B } from \"./a\";").types_only);
        assert!(one("export type { A } from \"./a\";").types_only);
        assert!(one("export type * from \"./a\";").types_only);
        assert!(one("import type * as ns from \"./a\";").types_only);
        assert!(!one("import { type A, b } from \"./a\";").types_only);
        assert!(!one("import D, { type A } from \"./a\";").types_only);
        assert!(!one("import * as ns from \"./a\";").types_only);
        assert!(!one("import \"./a\";").types_only);
        assert!(!one("import {} from \"./a\";").types_only);
        assert!(!one("import type from \"./a\";").types_only);
        assert!(!one("import { type } from \"./a\";").types_only);
        assert!(!one("import x = require(\"./a\");").types_only);
        assert!(!one("export * from \"./a\";").types_only);
        assert!(!one("export * as ns from \"./a\";").types_only);
        let multi = one("import {\n\ta, // a \"quoted\" comment\n\ttype B,\n} from \"../server\";");
        assert_eq!((multi.specifier.as_deref(), multi.types_only), (Some("../server"), false));
    }

    #[test]
    fn a_comment_after_a_statement_never_merges_it_with_the_next() {
        let found =
            all("import \"./server\"; // Install server runtime\nimport type { Subsystem } from \"./subsystem\";\n");
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].specifier.as_deref(), found[0].types_only), (Some("./server"), false));
        assert_eq!((found[1].specifier.as_deref(), found[1].types_only), (Some("./subsystem"), true));
    }

    #[test]
    fn comments_strings_and_templates_hide_nothing_and_add_nothing() {
        let text = "// import \"./a\";\n/* import \"./b\"; */\nconst s = \"import './c'\";\nconst t = `import \"./d\" ${import(\"./e\")}`;\nconst r = /import \"\\/f\"/;\nimport \"./g\";\n";
        let specifiers: Vec<_> = all(text).into_iter().filter_map(|statement| statement.specifier).collect();
        assert_eq!(specifiers, ["./e", "./g"]);
        assert!(parse("const s = \"unterminated;\nimport \"./a\";").is_err());
        assert!(parse("import(path);").is_err());
    }

    #[test]
    fn every_import_call_counts() {
        assert!(!one("const reviewImport = typeof import(\"./server\");").types_only);
        assert!(!one("const reviewImport = import(\"./server\").toString();").types_only);
        assert!(!one("import(\"./server\").then((m) => m);").types_only);
        assert!(!one("const m = await import(\"./server\");").types_only);
        assert!(!one("let m: Map<string, import(\"../emit\").EmittedAttribute>;").types_only);
        assert!(!one("type S = typeof import(\"./server\");").types_only);
    }

    #[test]
    fn exports_give_their_exported_names() {
        let parsed = parse(
            "export { a, b as c, type D } from \"./x\";\nexport { e as f };\nexport function g() {}\nexport const h: Map<A, B> = new Map<A, B>(), i = 1;\nexport const { j, k: l } = m;\nexport class N {}\nexport declare const o: number;\nexport type P = 1;\nexport interface Q {}\n",
        )
        .unwrap();
        assert_eq!(parsed.statements[0].values, ["a", "c"]);
        assert_eq!(parsed.statements[1].values, ["f"]);
        assert_eq!(parsed.statements[1].specifier, None);
        assert_eq!(parsed.declared, ["g", "h", "i", "j", "l", "N", "o"]);
    }

    fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let repo = std::env::temp_dir().join(format!("xtask-arch-ts-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&repo);
        for (path, text) in files {
            let path = repo.join(SOURCE).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        repo
    }

    #[test]
    fn a_root_that_reaches_a_side_entry_is_reported() {
        let repo = tree(
            "reach",
            &[
                ("index.ts", "export { a } from \"./a\";\nimport type { S } from \"./server\";\n"),
                ("a.ts", "import { s } from \"./server\";\nexport const a = s;\n"),
                ("server/index.ts", "export const s = 1;\n"),
            ],
        );
        let found = root_reaches(&repo.join("packages/.."));
        fs::remove_dir_all(&repo).unwrap();
        assert_eq!(
            found.unwrap(),
            [("packages/crucible/src/a.ts".to_owned(), "packages/crucible/src/server/index.ts".to_owned())]
        );
    }

    #[test]
    fn the_formatted_server_import_probe_is_reported() {
        let repo = tree(
            "probe",
            &[
                (
                    "index.ts",
                    "export { Subsystem } from \"./subsystem\";\nimport \"./server\"; // Install server runtime\nimport type { Subsystem } from \"./subsystem\";\n",
                ),
                ("subsystem.ts", "export class Subsystem {}\n"),
                ("server/index.ts", "export const s = 1;\n"),
            ],
        );
        let found = root_reaches(&repo);
        fs::remove_dir_all(&repo).unwrap();
        assert_eq!(
            found.unwrap(),
            [("packages/crucible/src/index.ts".to_owned(), "packages/crucible/src/server/index.ts".to_owned())]
        );
    }

    #[test]
    fn dynamic_imports_in_values_are_reported() {
        for (name, line) in [
            ("typeof", "const reviewImport = typeof import(\"./server\");\n"),
            ("member", "const reviewImport = import(\"./server\").toString();\n"),
        ] {
            let repo = tree(name, &[("index.ts", line), ("server/index.ts", "export const s = 1;\n")]);
            let found = root_reaches(&repo);
            fs::remove_dir_all(&repo).unwrap();
            assert_eq!(
                found.unwrap(),
                [("packages/crucible/src/index.ts".to_owned(), "packages/crucible/src/server/index.ts".to_owned())],
                "{line}"
            );
        }
    }
}
