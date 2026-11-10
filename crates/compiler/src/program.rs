//! The tsrs program boundary: parses a tsconfig, builds the program, and runs the full check with one checker.

use std::path::Path;
use std::sync::Arc;

use tsrs_ast::Diagnostic as TsDiagnostic;
use tsrs_compiler::{CheckerHandle, Context, Program, ProgramOptions, new_cached_fs_compiler_host, new_program};
use tsrs_core::{CompilerOptions, JsxEmit, P, tspath};
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{FS, bundled::embed as bundled, osvfs};

use crate::Diagnostic;

/// Every fact query goes to the checker that ran the full check, so it reads the caches that check filled. Only `check`
/// makes one.
pub(crate) struct Checked {
    program: &'static Program,
    checker: CheckerHandle,
    host: ConfigHost,
}

/// Where a checked program's files are. Only `Checked::into_parts` makes one, and `Extracted` takes one, so only a
/// checked program is extracted.
pub(crate) struct Layout {
    root: String,
    out_dir: Option<String>,
    config_dir: String,
}

impl Layout {
    /// The folder the output layout mirrors: `rootDir`, or else the deepest folder that holds every source file and
    /// `rootDirs` entry.
    pub(crate) fn root(&self) -> &str {
        &self.root
    }

    pub(crate) fn out_dir(&self) -> Option<&str> {
        self.out_dir.as_deref()
    }

    /// The folder of the tsconfig, or the current folder.
    pub(crate) fn config_dir(&self) -> &str {
        &self.config_dir
    }
}

impl Checked {
    /// Takes the program apart for extraction, which keeps the checker.
    pub(crate) fn into_parts(self) -> (&'static Program, CheckerHandle, ConfigHost, Layout) {
        let layout = Layout { root: self.source_root(), out_dir: self.out_dir(), config_dir: self.config_dir() };
        (self.program, self.checker, self.host, layout)
    }

    fn source_root(&self) -> String {
        let options = self.program.options();
        if !options.root_dir.is_empty() {
            return options.root_dir.replace('\\', "/");
        }
        let mut dirs: Vec<String> = self
            .program
            .source_files()
            .iter()
            .filter(|f| !f.is_declaration_file() && !f.is_js())
            .map(|f| f.file_name().replace('\\', "/"))
            .map(|path| path.rsplit_once('/').map_or(String::new(), |(dir, _)| dir.to_owned()))
            .collect();
        dirs.extend(options.root_dirs.iter().flatten().map(|d| d.replace('\\', "/").trim_end_matches('/').to_owned()));
        common_dir(&dirs)
    }

    fn out_dir(&self) -> Option<String> {
        let out = &self.program.options().out_dir;
        (!out.is_empty()).then(|| out.replace('\\', "/"))
    }

    fn config_dir(&self) -> String {
        let config = self.program.options().config_file_path.replace('\\', "/");
        match config.rsplit_once('/') {
            Some((dir, _)) => dir.to_owned(),
            None => self.host.cwd.clone(),
        }
    }
}

fn common_dir(dirs: &[String]) -> String {
    let Some(first) = dirs.first() else { return String::new() };
    let mut root = first.clone();
    while !dirs.iter().all(|d| *d == root || d.starts_with(&format!("{root}/"))) {
        match root.rsplit_once('/') {
            Some((parent, _)) => root = parent.to_owned(),
            None => return String::new(),
        }
    }
    root
}

pub(crate) struct ConfigHost {
    fs: Arc<dyn FS>,
    cwd: String,
}

impl ParseConfigHost for ConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }

    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

fn config_host() -> Result<ConfigHost, Vec<Diagnostic>> {
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    let cwd = std::env::current_dir().map_err(|e| vec![plain(&format!("cannot read the current directory: {e}"))])?;
    Ok(ConfigHost { fs, cwd: tspath::normalize_path(&cwd.to_string_lossy()) })
}

pub(crate) fn out_dir(tsconfig: &Path) -> (Option<String>, Vec<Diagnostic>) {
    let host = match config_host() {
        Ok(host) => host,
        Err(errors) => return (None, errors),
    };
    let path = tspath::get_normalized_absolute_path(&tsconfig.to_string_lossy(), &host.cwd);
    let (config, errors) = tsoptions::get_parsed_command_line_of_config_file(&path, None, None, &host, None);
    let errors = errors.iter().chain(config.iter().flat_map(|config| &config.errors)).map(convert).collect();
    let options = config.and_then(|config| config.compiler_options());
    let dir = options.filter(|options| !options.out_dir.is_empty()).map(|options| options.out_dir.replace('\\', "/"));
    (dir, errors)
}

pub(crate) fn check(tsconfig: &Path) -> Result<Checked, Vec<Diagnostic>> {
    let host = config_host()?;
    let (fs, cwd) = (Arc::clone(&host.fs), host.cwd.clone());
    let path = tspath::get_normalized_absolute_path(&tsconfig.to_string_lossy(), &cwd);
    // One checker keeps type and symbol identity across every query, and it is the cheapest in memory and CPU when
    // several builds share a machine.
    let options = CompilerOptions { checkers: Some(1), ..CompilerOptions::default() };
    let (config, errors) = tsoptions::get_parsed_command_line_of_config_file(&path, Some(&options), None, &host, None);
    if !errors.is_empty() {
        return Err(errors.iter().map(convert).collect());
    }
    let Some(config) = config else {
        return Err(vec![plain(&format!("cannot read {path}"))]);
    };
    if let Some(options) = config.compiler_options()
        && (matches!(options.jsx, JsxEmit::REACT_JSX | JsxEmit::REACT_JSX_DEV) || jsx_import_source_set(&config, &host))
    {
        return Err(vec![Diagnostic {
            path: Some(path),
            line: 1,
            column: 1,
            code: 0,
            message: "`jsx: \"react-jsx\"`, `jsx: \"react-jsxdev\"` and `jsxImportSource` are not supported; use `jsx: \"react\"` with `jsxFactory: \"Vide.jsx\"` and remove `jsxImportSource` in the tsconfig.".to_owned(),
        }]);
    }
    let compiler_host = new_cached_fs_compiler_host(&cwd, fs, &bundled::lib_path(), None, None, None);
    let program = new_program(ProgramOptions::new(P::new(config), compiler_host));
    let pragma_errors = jsx_pragma_errors(program);
    if !pragma_errors.is_empty() {
        return Err(pragma_errors);
    }
    let ctx = Context::default();
    let diagnostics = tsrs_compiler::get_diagnostics_of_any_program(
        &ctx,
        program,
        None,
        false,
        &mut |ctx, file| program.get_bind_diagnostics(ctx, file),
        &mut |ctx, file| program.get_semantic_diagnostics(ctx, file),
    );
    if !diagnostics.is_empty() {
        return Err(diagnostics.iter().map(convert).collect());
    }
    Ok(Checked { program, checker: program.get_type_checker(&ctx), host })
}

fn jsx_import_source_set(config: &tsoptions::ParsedCommandLine, host: &ConfigHost) -> bool {
    let has_source = |raw: &tsoptions::CompilerOptionsValue| {
        raw.as_object()
            .and_then(|raw| raw.get("compilerOptions"))
            .and_then(|options| options.as_object())
            .is_some_and(|options| options.contains_key("jsxImportSource"))
    };
    has_source(&config.raw)
        || config.config_file.is_some_and(|file| {
            file.extended_source_files.lock().unwrap().iter().any(|path| {
                let Some(text) = host.fs.read_file(path) else { return false };
                let (raw, _) = tsoptions::parse_config_file_text_to_json(
                    path,
                    tspath::to_path(path, &host.cwd, host.fs.use_case_sensitive_file_names()),
                    &text,
                );
                has_source(&raw)
            })
        })
}

fn jsx_pragma_errors(program: &Program) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    for &file in program.source_files().iter().filter(|file| !file.is_declaration_file() && !file.is_js()) {
        let automatic = !tsrs_ast::get_jsx_implicit_import_base(&program.options(), Some(file)).is_empty();
        let runtime = tsrs_ast::get_pragma_from_source_file(Some(file), "jsxruntime").filter(|pragma| {
            automatic && pragma.args.get("factory").is_some_and(|argument| argument.value == "automatic")
        });
        let source = tsrs_ast::get_pragma_from_source_file(Some(file), "jsximportsource");
        for pragma in runtime.into_iter().chain(source) {
            let pos = pragma.comment_range.pos() as usize;
            let before = &file.text()[..pos];
            errors.push(Diagnostic {
                path: Some(file.file_name().replace('\\', "/")),
                line: (before.matches('\n').count() + 1) as u32,
                column: (pos - before.rfind('\n').map_or(0, |i| i + 1) + 1) as u32,
                code: 0,
                message: "`@jsxRuntime automatic` and `@jsxImportSource` are not supported; use `jsx: \"react\"` with `jsxFactory: \"Vide.jsx\"` in the tsconfig and remove these pragmas.".to_owned(),
            });
        }
    }
    errors
}

fn plain(message: &str) -> Diagnostic {
    Diagnostic { path: None, line: 0, column: 0, code: 0, message: message.to_owned() }
}

fn convert(d: &P<TsDiagnostic>) -> Diagnostic {
    let mut message = Vec::new();
    tsrs_compiler::diagnosticwriter::write_flattened_diagnostic_message(&mut message, *d, "\n");
    let message = String::from_utf8_lossy(&message).into_owned();
    let (path, line, column) = match (d.file(), d.pos()) {
        (Some(file), Some(pos)) => {
            let text = file.text();
            let pos = (tsrs_scanner::skip_trivia(text, pos) as usize).min(text.len());
            let before = &text[..pos];
            let line = before.matches('\n').count() + 1;
            let column = pos - before.rfind('\n').map_or(0, |i| i + 1) + 1;
            (Some(file.file_name().replace('\\', "/")), line as u32, column as u32)
        }
        _ => (None, 0, 0),
    };
    Diagnostic { path, line, column, code: d.code(), message }
}
