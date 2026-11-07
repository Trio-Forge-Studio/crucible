//! Reads what a crate ships with `syn`. The scan starts at the source files of the crate's library, binary and build
//! targets and follows `mod` declarations, `#[path]` and `include!`, the way rustc does, so a file reaches the scan
//! exactly when a shipped target compiles it. `#[cfg(test)]` items are dropped, and the modules they declare are never
//! read. Integration tests, examples and benches are other targets, so they are never read either.
//!
//! The rules then read tokens, not text: string literal values, with escapes resolved, including inside macro calls,
//! and paths. Doc comments are not tokens the crate ships, so they are skipped.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit_mut::{self, VisitMut};
use syn::{Attribute, ImplItem, Item, Lit, Stmt, TraitItem};

use crate::run::normalize;

/// The `std::env` functions that read process environment variables. `env!` and `option_env!` read the build's
/// environment, so they are not listed.
const ENVIRONMENT_READS: &[&str] = &["var", "var_os", "vars", "vars_os"];

#[derive(Debug)]
pub struct Source {
    pub path: PathBuf,
    pub strings: Vec<String>,
    /// Rule 3's findings, such as `reads std::env::var` or `renames std::env as e`.
    pub environment: Vec<String>,
}

pub fn sources(roots: &[PathBuf]) -> Result<Vec<Source>, String> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for root in roots {
        let dir = root.parent().unwrap_or(root).to_path_buf();
        load(root, dir, &mut seen, &mut found)?;
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// `dir` is where the file's `mod name;` declarations look for `name.rs` and `name/mod.rs`.
fn load(path: &Path, dir: PathBuf, seen: &mut BTreeSet<PathBuf>, found: &mut Vec<Source>) -> Result<(), String> {
    let path = normalize(path);
    if !seen.insert(path.clone()) {
        return Ok(());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (tokens, modules) = match syn::parse_file(&text) {
        Ok(mut file) => {
            if is_test(&file.attrs) {
                return Ok(());
            }
            let mut walker = Walker::new(&path, dir);
            walker.visit_file_mut(&mut file);
            if let Some(error) = walker.errors.first() {
                return Err(error.clone());
            }
            (file.to_token_stream(), walker.files)
        }
        // `include!` also pastes an expression, which is not a file of items.
        Err(error) => match syn::parse_str::<syn::Expr>(&text) {
            Ok(expr) => (expr.to_token_stream(), Vec::new()),
            Err(_) => return Err(format!("{}: {error}", path.display())),
        },
    };
    let mut scan = Scan::default();
    scan.tokens(tokens);
    found.push(Source { path, strings: scan.strings, environment: scan.environment });
    for (file, dir) in modules {
        load(&file, dir, seen, found)?;
    }
    Ok(())
}

fn is_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg") && attr.meta.require_list().is_ok_and(|list| list.tokens.to_string() == "test")
    })
}

fn path_attr(attrs: &[Attribute]) -> Result<Option<String>, String> {
    for attr in attrs {
        if attr.path().is_ident("cfg_attr") && attr.meta.to_token_stream().to_string().contains("path") {
            return Err("a `#[cfg_attr(.., path = ..)]` module, which this scan cannot follow".into());
        }
        if attr.path().is_ident("path") {
            let value = attr.meta.require_name_value().map_err(|e| e.to_string())?;
            if let syn::Expr::Lit(syn::ExprLit { lit: Lit::Str(path), .. }) = &value.value {
                return Ok(Some(path.value()));
            }
            return Err("a `#[path]` that is not a string literal".into());
        }
    }
    Ok(None)
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

/// Drops `#[cfg(test)]` items and collects the files that `mod` and `include!` bring in.
struct Walker {
    file: PathBuf,
    /// The module folder of each enclosing module, innermost last.
    dirs: Vec<PathBuf>,
    /// Each file to read next, with its module folder.
    files: Vec<(PathBuf, PathBuf)>,
    errors: Vec<String>,
}

impl Walker {
    fn new(file: &Path, dir: PathBuf) -> Self {
        Self { file: file.to_path_buf(), dirs: vec![dir], files: Vec::new(), errors: Vec::new() }
    }

    fn dir(&self) -> &Path {
        self.dirs.last().expect("the file's own module folder")
    }

    fn fail(&mut self, message: String) {
        self.errors.push(format!("{}: {message}", self.file.display()));
    }

    fn declared(&mut self, module: &syn::ItemMod) -> Result<(PathBuf, PathBuf), String> {
        let name = module.ident.to_string();
        let name = name.trim_start_matches("r#");
        if let Some(path) = path_attr(&module.attrs)? {
            // Outside inline modules a `#[path]` is relative to the declaring file, and the loaded file owns its
            // folder like a `mod.rs`.
            let base = if self.dirs.len() == 1 { self.file.parent().unwrap_or(&self.file) } else { self.dir() };
            let file = normalize(&base.join(path));
            let dir = file.parent().unwrap_or(&file).to_path_buf();
            return Ok((file, dir));
        }
        let dir = self.dir().join(name);
        let candidates = [self.dir().join(format!("{name}.rs")), dir.join("mod.rs")];
        match candidates.iter().filter(|file| file.is_file()).collect::<Vec<_>>().as_slice() {
            [file] => Ok(((*file).clone(), dir)),
            [] => Err(format!("cannot find the file of `mod {name};`")),
            _ => Err(format!("`mod {name};` has both {name}.rs and {name}/mod.rs")),
        }
    }
}

impl VisitMut for Walker {
    fn visit_file_mut(&mut self, file: &mut syn::File) {
        file.items.retain(|item| !is_test(item_attrs(item)));
        visit_mut::visit_file_mut(self, file);
    }

    fn visit_item_mod_mut(&mut self, module: &mut syn::ItemMod) {
        let Some((_, items)) = &mut module.content else {
            match self.declared(module) {
                Ok(file) => self.files.push(file),
                Err(message) => self.fail(message),
            }
            return;
        };
        items.retain(|item| !is_test(item_attrs(item)));
        let dir = match path_attr(&module.attrs) {
            Ok(Some(path)) => self.dir().join(path),
            Ok(None) => self.dir().join(module.ident.to_string().trim_start_matches("r#")),
            Err(message) => return self.fail(message),
        };
        self.dirs.push(dir);
        visit_mut::visit_item_mod_mut(self, module);
        self.dirs.pop();
    }

    fn visit_item_impl_mut(&mut self, item: &mut syn::ItemImpl) {
        item.items.retain(|item| {
            let attrs = match item {
                ImplItem::Const(item) => &item.attrs,
                ImplItem::Fn(item) => &item.attrs,
                ImplItem::Type(item) => &item.attrs,
                ImplItem::Macro(item) => &item.attrs,
                _ => return true,
            };
            !is_test(attrs)
        });
        visit_mut::visit_item_impl_mut(self, item);
    }

    fn visit_item_trait_mut(&mut self, item: &mut syn::ItemTrait) {
        item.items.retain(|item| {
            let attrs = match item {
                TraitItem::Const(item) => &item.attrs,
                TraitItem::Fn(item) => &item.attrs,
                TraitItem::Type(item) => &item.attrs,
                TraitItem::Macro(item) => &item.attrs,
                _ => return true,
            };
            !is_test(attrs)
        });
        visit_mut::visit_item_trait_mut(self, item);
    }

    fn visit_block_mut(&mut self, block: &mut syn::Block) {
        block.stmts.retain(|stmt| match stmt {
            Stmt::Local(local) => !is_test(&local.attrs),
            Stmt::Item(item) => !is_test(item_attrs(item)),
            Stmt::Macro(mac) => !is_test(&mac.attrs),
            Stmt::Expr(..) => true,
        });
        visit_mut::visit_block_mut(self, block);
    }

    fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
        let segments: Vec<String> = mac.path.segments.iter().map(|segment| segment.ident.to_string()).collect();
        if matches!(
            segments.iter().map(String::as_str).collect::<Vec<_>>().as_slice(),
            ["include"] | ["std" | "core", "include"]
        ) {
            match syn::parse2::<syn::LitStr>(mac.tokens.clone()) {
                Ok(path) => {
                    let file = normalize(&self.file.parent().unwrap_or(&self.file).join(path.value()));
                    self.files.push((file, self.dir().to_path_buf()));
                }
                // The build script writes `OUT_DIR`, and the scan reads the build script.
                Err(_) if mac.tokens.to_string().contains("OUT_DIR") => {}
                Err(_) => self.fail(format!("cannot follow `include!({})`", mac.tokens)),
            }
        }
        visit_mut::visit_macro_mut(self, mac);
    }
}

#[derive(Default)]
struct Scan {
    strings: Vec<String>,
    environment: Vec<String>,
}

impl Scan {
    fn tokens(&mut self, stream: TokenStream) {
        let tokens: Vec<TokenTree> = stream.into_iter().collect();
        let mut i = 0;
        while i < tokens.len() {
            match &tokens[i] {
                TokenTree::Punct(punct) if punct.as_char() == '#' => {
                    let bang = matches!(tokens.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                    let at = i + 1 + usize::from(bang);
                    match tokens.get(at) {
                        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Bracket && is_doc(group) => {
                            i = at + 1
                        }
                        _ => i += 1,
                    }
                }
                TokenTree::Group(group) => {
                    self.tokens(group.stream());
                    i += 1;
                }
                TokenTree::Literal(literal) => {
                    if let Some(text) = string_value(literal) {
                        self.strings.push(text);
                    }
                    i += 1;
                }
                TokenTree::Ident(_) => i = self.path(&tokens, i),
                TokenTree::Punct(_) => i += 1,
            }
        }
    }

    /// Reads the path that starts at `tokens[start]` and returns the index after it. A path that names an environment
    /// read counts however it is spelled or imported. A `std::env` that is renamed or glob-imported counts too,
    /// because a read through the new name would not name `env`.
    fn path(&mut self, tokens: &[TokenTree], start: usize) -> usize {
        let mut segments = vec![ident(&tokens[start]).unwrap_or_default()];
        let mut i = start + 1;
        while is_path_separator(tokens, i)
            && let Some(name) = tokens.get(i + 2).and_then(ident)
        {
            segments.push(name);
            i += 3;
        }
        for pair in segments.windows(2) {
            if pair[0] == "env" && ENVIRONMENT_READS.contains(&pair[1].as_str()) {
                self.environment.push(format!("reads std::env::{}", pair[1]));
            }
        }
        if segments.last().is_some_and(|last| last == "env") {
            if let (Some(TokenTree::Ident(word)), Some(alias)) = (tokens.get(i), tokens.get(i + 1).and_then(ident))
                && word == "as"
            {
                self.environment.push(format!("renames std::env as {alias}"));
            } else if is_path_separator(tokens, i) {
                match tokens.get(i + 2) {
                    Some(TokenTree::Punct(star)) if star.as_char() == '*' => {
                        self.environment.push("imports std::env::*".into())
                    }
                    Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => self.use_group(group),
                    _ => {}
                }
            }
        }
        i
    }

    fn use_group(&mut self, group: &proc_macro2::Group) {
        let tokens: Vec<TokenTree> = group.stream().into_iter().collect();
        for entry in tokens.split(|token| matches!(token, TokenTree::Punct(p) if p.as_char() == ',')) {
            match entry {
                [TokenTree::Punct(star)] if star.as_char() == '*' => {
                    self.environment.push("imports std::env::*".into())
                }
                [first, ..] if ident(first).is_some_and(|name| ENVIRONMENT_READS.contains(&name.as_str())) => {
                    self.environment.push(format!("reads std::env::{}", ident(first).unwrap_or_default()))
                }
                [first, TokenTree::Ident(word), alias] if ident(first).as_deref() == Some("self") && word == "as" => {
                    self.environment.push(format!("renames std::env as {}", ident(alias).unwrap_or_default()))
                }
                _ => {}
            }
        }
    }
}

fn ident(token: &TokenTree) -> Option<String> {
    match token {
        TokenTree::Ident(ident) => Some(ident.to_string().trim_start_matches("r#").to_owned()),
        _ => None,
    }
}

fn is_path_separator(tokens: &[TokenTree], at: usize) -> bool {
    matches!(
        (tokens.get(at), tokens.get(at + 1)),
        (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b)))
            if a.as_char() == ':' && a.spacing() == Spacing::Joint && b.as_char() == ':'
    )
}

fn is_doc(group: &proc_macro2::Group) -> bool {
    matches!(group.stream().into_iter().next(), Some(TokenTree::Ident(ident)) if ident == "doc")
}

fn string_value(literal: &proc_macro2::Literal) -> Option<String> {
    match Lit::new(literal.clone()) {
        Lit::Str(text) => Some(text.value()),
        Lit::ByteStr(bytes) => Some(String::from_utf8_lossy(&bytes.value()).into_owned()),
        Lit::CStr(text) => Some(text.value().to_string_lossy().into_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(source: &str) -> Source {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("xtask-arch-rust-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("lib.rs"), source).unwrap();
        let found = sources(&[dir.join("lib.rs")]);
        fs::remove_dir_all(&dir).unwrap();
        found.unwrap().remove(0)
    }

    #[test]
    fn strings_are_found_and_comments_are_not() {
        let source = r####"
            //! "in an inner doc comment"
            // "in a line comment"
            /* "in a /* nested */ block comment" */
            /// "in a doc comment"
            fn f<'a>(x: &'a str) -> char {
                let a = "plain \" with an escaped quote";
                let b = r#"raw with "quotes""#;
                let c = b"bytes";
                let d = '"';
                let g = br##"raw bytes"##;
                let h = "\x40trioforge";
                println!("{}", "in a macro");
                'x'
            }
        "####;
        assert_eq!(
            scan(source).strings,
            [
                "plain \" with an escaped quote",
                "raw with \"quotes\"",
                "bytes",
                "raw bytes",
                "@trioforge",
                "{}",
                "in a macro"
            ]
        );
    }

    #[test]
    fn test_items_are_dropped() {
        let source = r#"
            fn a() { "kept"; }
            #[cfg(test)]
            mod tests { fn b() { let x = { "inner" }; } }
            #[cfg(test)]
            mod more;
            struct S;
            impl S {
                #[cfg(test)]
                fn only_in_tests() { "in an impl"; }
                fn shipped() { #[cfg(test)] let y = "in a statement"; }
            }
            #[cfg(any(test, feature = "x"))]
            fn c() { "also kept"; }
        "#;
        assert_eq!(scan(source).strings, ["kept", "x", "also kept"]);
    }

    #[test]
    fn environment_reads_count_however_they_are_spelled() {
        let source = r#"
            use std::env::var as review_var;
            use std::env::{var_os as os, self as e};
            use std::{env::{vars}};
            use std::env as other;
            use std::env::*;
            fn a() {
                let _ = review_var("HOME");
                let _ = std::env::var("HOME");
                let _ = ::std::env::vars_os();
                let _ = env::var("HOME");
                println!("{:?}", std::env::var_os("HOME"));
            }
        "#;
        assert_eq!(
            scan(source).environment,
            [
                "reads std::env::var",
                "reads std::env::var_os",
                "renames std::env as e",
                "reads std::env::vars",
                "renames std::env as other",
                "imports std::env::*",
                "reads std::env::var",
                "reads std::env::vars_os",
                "reads std::env::var",
                "reads std::env::var_os",
            ]
        );
        let quiet = "use std::env;\nfn a() { let _ = (env!(\"CARGO\"), option_env!(\"X\"), env::current_dir(), env::args()); std::process::Command::new(\"x\").env(\"A\", \"B\"); }";
        assert!(scan(quiet).environment.is_empty());
    }

    #[test]
    fn the_scan_reads_exactly_the_files_the_targets_compile() {
        let dir = std::env::temp_dir().join(format!("xtask-arch-modules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let write = |path: &str, text: &str| {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "src/lib.rs",
            "mod a;\n#[cfg(test)]\nmod tests;\n#[path = \"../shared/x.rs\"]\nmod x;\nmod inline { mod deep; }\ninclude!(\"inc.rs\");\nstatic G: &str = include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));\n",
        );
        write("src/a.rs", "mod b;\n");
        write("src/a/b.rs", "fn b() {}\n");
        write("src/tests.rs", "fn t() { std::env::var(\"HOME\"); }\n");
        write("src/inline/deep.rs", "fn d() {}\n");
        write("src/inc.rs", "fn i() {}\n");
        write("src/orphan.rs", "fn o() {}\n");
        write("shared/x.rs", "mod y;\n");
        write("shared/y.rs", "fn y() {}\n");
        write("bad/lib.rs", "include!(make_path!());\n");
        let found = sources(&[dir.join("src/lib.rs")]);
        let bad = sources(&[dir.join("bad/lib.rs")]);
        fs::remove_dir_all(&dir).unwrap();
        let read: Vec<String> =
            found.unwrap().iter().map(|source| crate::run::slashed(source.path.strip_prefix(&dir).unwrap())).collect();
        assert_eq!(
            read,
            ["shared/x.rs", "shared/y.rs", "src/a/b.rs", "src/a.rs", "src/inc.rs", "src/inline/deep.rs", "src/lib.rs"]
        );
        assert!(bad.unwrap_err().contains("cannot follow `include!"));
    }
}
