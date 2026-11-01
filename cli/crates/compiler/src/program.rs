//! The tsrs program boundary: parses a tsconfig, builds the program, and runs the full check with one checker.

use std::path::Path;
use std::sync::Arc;

use tsrs_ast::Diagnostic as TsDiagnostic;
use tsrs_compiler::{CheckerHandle, Context, Program, ProgramOptions, new_cached_fs_compiler_host, new_program};
use tsrs_core::{CompilerOptions, P, tspath};
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{FS, bundled::embed as bundled, osvfs};

use crate::Diagnostic;

/// Every fact query goes to the checker that ran the full check, so it reads the caches that check filled.
pub(crate) struct Checked {
    pub program: &'static Program,
    pub checker: CheckerHandle,
    pub host: ConfigHost,
}

impl Checked {
    pub fn files(&self) -> Vec<crate::tree::File> {
        self.program
            .source_files()
            .iter()
            .filter(|f| !f.is_declaration_file() && !f.is_js())
            .map(|&f| crate::tree::File::new(f))
            .collect()
    }

    pub fn root_dir(&self) -> Option<String> {
        let root = &self.program.options().root_dir;
        (!root.is_empty()).then(|| root.replace('\\', "/"))
    }
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

pub(crate) fn check(tsconfig: &Path) -> Result<Checked, Vec<Diagnostic>> {
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    let cwd = std::env::current_dir().map_err(|e| vec![plain(&format!("cannot read the current directory: {e}"))])?;
    let cwd = tspath::normalize_path(&cwd.to_string_lossy());
    let host = ConfigHost { fs: Arc::clone(&fs), cwd: cwd.clone() };
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
    let compiler_host = new_cached_fs_compiler_host(&cwd, fs, &bundled::lib_path(), None, None, None);
    let program = new_program(ProgramOptions::new(P::new(config), compiler_host));
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
