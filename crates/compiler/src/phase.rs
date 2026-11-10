//! The phases of a compile after the check. Each takes the one before it by value, so the phases run once and in
//! order, and lowering only reads facts that passed validation.

use std::path::PathBuf;

use crate::extension::Extension;
use crate::facts::{Facts, Unanswered};
use crate::lower::{self, Error, ImportBase};
use crate::tree::File;
use crate::write::HEADER;
use crate::{Compiled, Diagnostic, Failure, Module, Target, project};

pub(crate) struct Extracted {
    /// The folder the output layout mirrors.
    root: String,
    out_dir: PathBuf,
    sources: Vec<File>,
    facts: Facts,
}

impl Extracted {
    /// `sources[i]` is the file whose facts are `facts.files[i]`.
    pub(crate) fn new(root: String, out_dir: PathBuf, sources: Vec<File>, facts: Facts) -> Self {
        assert_eq!(sources.len(), facts.files.len(), "each source file has one fact table");
        Self { root, out_dir, sources, facts }
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn into_facts(self) -> Facts {
        self.facts
    }

    /// Fails with the program's rule and place errors, which stop the compile before it lowers anything.
    pub(crate) fn validate(self) -> Result<Validated, Vec<Diagnostic>> {
        if self.facts.diagnostics.is_empty() { Ok(Validated(self)) } else { Err(self.facts.diagnostics) }
    }
}

/// Facts that hold no diagnostic.
pub(crate) struct Validated(Extracted);

impl Validated {
    pub(crate) fn lower(self, extensions: &[&dyn Extension]) -> Result<Lowered, Vec<Error>> {
        let Extracted { root, out_dir, sources, facts } = self.0;
        let mut modules = Vec::with_capacity(sources.len());
        let mut errors = Vec::new();
        for (id, (file, file_facts)) in sources.into_iter().zip(&facts.files).enumerate() {
            let path = &file_facts.path;
            let relative = path.strip_prefix(root.as_str()).unwrap_or(path).trim_start_matches('/');
            let stem = relative.rsplit_once('.').map_or(relative, |(stem, _)| stem);
            let (stem, import_base) = match stem.strip_suffix("index") {
                Some(dir) if dir.is_empty() || dir.ends_with('/') => (format!("{dir}init"), ImportBase::Script),
                _ => (stem.to_owned(), ImportBase::ScriptParent),
            };
            let unanswered: Vec<&Unanswered> = facts.unanswered.iter().filter(|u| u.path == *path).collect();
            match lower::lower(file, file_facts, &unanswered, import_base, &facts.place, id as u32, extensions) {
                Ok(chunk) => {
                    let mut source = crucible_luau::print(&chunk);
                    source.insert_str(0, HEADER);
                    modules.push(Module { path: format!("{stem}.luau"), source });
                }
                Err(e) => errors.extend(e),
            }
        }
        if errors.is_empty() { Ok(Lowered { out_dir, modules }) } else { Err(errors) }
    }
}

pub(crate) struct Lowered {
    out_dir: PathBuf,
    modules: Vec<Module>,
}

impl Lowered {
    /// Adds a game's virtual modules, none of which may take a compiled module's path.
    pub(crate) fn finish(self, target: Target<'_>) -> Result<Compiled, Failure> {
        let Lowered { out_dir, mut modules } = self;
        if let Target::Game { modules: virtual_modules, .. } = target {
            let compiled = modules.len();
            for module in virtual_modules {
                if let Some(other) = modules[..compiled].iter().find(|m| project::same_file(&m.path, module.path)) {
                    let message = format!(
                        "the virtual module `{}` has the output path {}, which collides with the compiled module {}",
                        module.specifier, module.path, other.path
                    );
                    return Err(Failure::Crucible(vec![Diagnostic {
                        path: None,
                        line: 0,
                        column: 0,
                        code: 0,
                        message,
                    }]));
                }
                modules.push(Module { path: module.path.to_owned(), source: format!("{HEADER}{}", module.source) });
            }
        }
        Ok(Compiled { out_dir, modules })
    }
}
