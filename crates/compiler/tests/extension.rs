//! A test extension that uses every hook of the seam. It finds calls to `probe` from the `@rbxts/probe` package,
//! records each one's types from the checker, and lowers it to a call into the package's runtime entry with those
//! types and the program's probe count. It replaces each `HERE` in a probe's arguments with where the probe is, and
//! registers every class with the runtime after its declaration. A second one, `Scopes`, nests the calls it lowers to
//! test how long a replacement lasts.

mod support;

use std::collections::HashMap;
use std::path::Path;

use crucible_compiler::extension::{
    Extension, ExtractCx, File, HelperModule, Kind, LowerCx, Node, ProgramCx, SiteId, Symbol,
};
use crucible_compiler::{Diagnostic, Failure, Target};
use crucible_luau::{Expr, Name, Stmt};
use serde_json::json;
use support::{Game, has, lines};

const PACKAGE: &str = "@rbxts/probe";

const RUNTIME: HelperModule = HelperModule {
    package: PACKAGE,
    entry: "./runtime",
    local: "__probe",
    missing: "probes need a game that maps @rbxts/probe/runtime",
};

#[derive(Default)]
struct Probe {
    begun: bool,
    /// The version in the package's manifest, when the program has a file from the package.
    version: Option<String>,
    /// Each probe call's argument type and return type, as `number -> number`.
    calls: HashMap<SiteId, String>,
    /// Each `HERE` in a probe's arguments, as the file name and line of the probe.
    heres: HashMap<SiteId, String>,
    classes: HashMap<SiteId, &'static str>,
    /// The program's probe calls, counted once every file was extracted.
    total: Option<usize>,
    limit: Option<usize>,
}

impl Probe {
    fn is_probe_export(cx: &mut ExtractCx<'_>, n: Node, name: &str) -> bool {
        if n.kind() != Kind::Identifier {
            return false;
        }
        let Some(symbol) = cx.symbol_at(n) else { return false };
        let symbol: Symbol = cx.aliased(symbol);
        symbol.name() == name && symbol.declarations().into_iter().any(|d| cx.package_of(d).as_deref() == Some(PACKAGE))
    }
}

impl Extension for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn begin_program(&mut self, cx: &mut ProgramCx<'_>) {
        cx.declare_helper(&RUNTIME);
        self.begun = true;
        let manifest = cx.manifest(PACKAGE);
        self.version = manifest.and_then(|m| m.json["version"].as_str().map(str::to_owned));
    }

    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: File) {
        assert!(self.begun, "begin_program runs before the first file");
        assert!(self.total.is_none(), "finish_program runs after the last file");
        let name = file.path().rsplit('/').next().unwrap().to_owned();
        for n in descendants(file.root()) {
            match n.kind() {
                Kind::CallExpression if Probe::is_probe_export(cx, n.expression(), "probe") => {
                    let Some(value) = n.arguments().get(0) else { continue };
                    let value = cx.type_at(value);
                    let value = cx.type_text(value);
                    if value == "any" {
                        cx.report(n, 99001, "probe cannot read an `any` value");
                        continue;
                    }
                    let signature = cx.signature_at(n);
                    let returned = cx.return_type(signature);
                    let returned = cx.type_text(returned);
                    let site = cx.site(n);
                    self.calls.insert(site, format!("{value} -> {returned}"));
                    let line = file.position(n.span().start).line;
                    for argument in n.arguments().iter().flat_map(descendants) {
                        if Probe::is_probe_export(cx, argument, "HERE") {
                            self.heres.insert(cx.site(argument), format!("{name}:{line}"));
                        }
                    }
                }
                Kind::ClassDeclaration => {
                    let symbol = n.name().and_then(|name| cx.symbol_at(name));
                    if let Some(symbol) = symbol {
                        self.classes.insert(cx.site(n), symbol.name());
                    }
                }
                _ => {}
            }
        }
    }

    fn finish_program(&mut self, cx: &mut ProgramCx<'_>) {
        let total = self.calls.len();
        self.total = Some(total);
        if let Some(limit) = self.limit
            && total > limit
        {
            cx.report(0, format!("{total} probes, more than the limit of {limit}"));
        }
    }

    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr> {
        let types = self.calls.get(&cx.site(call))?;
        let replacements = call
            .arguments()
            .iter()
            .flat_map(descendants)
            .filter_map(|n| Some((n, Expr::string(self.heres.get(&cx.site(n))?.clone()))))
            .collect();
        let mut arguments = cx.arguments(call, replacements);
        arguments.push(Expr::string(types.clone()));
        arguments.push(Expr::Number(self.total.expect("finish_program runs before lowering") as f64));
        Some(cx.helper(&RUNTIME, "probe").call(arguments))
    }

    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, local: &Name) -> Vec<Stmt> {
        let Some(&name) = self.classes.get(&cx.site(class)) else { return Vec::new() };
        let register = cx.helper(&RUNTIME, "class").call(vec![Expr::Name(local.clone()), Expr::string(name)]);
        vec![Stmt::Call(register)]
    }
}

/// `n` and every node below it, in source order.
fn descendants(n: Node) -> Vec<Node> {
    let mut out = vec![n];
    let mut i = 0;
    while i < out.len() {
        let mut children = Vec::new();
        out[i].for_each_child(&mut |child| children.push(child));
        out.splice(i + 1..i + 1, children);
        i += 1;
    }
    out
}

fn probe_game() -> Game {
    let game = Game::new();
    game.json(
        "node_modules/@rbxts/probe/package.json",
        &json!({
            "name": PACKAGE,
            "version": "1.2.3",
            "types": "index.d.ts",
            "exports": { ".": { "types": "./index.d.ts" }, "./runtime": { "types": "./runtime.d.ts" } },
        }),
    );
    game.write(
        "node_modules/@rbxts/probe/index.d.ts",
        "export declare function probe<T>(value: T, where?: { readonly at: string }): T;\n\
         export declare const HERE: string;\n",
    );
    game.write("node_modules/@rbxts/probe/runtime.d.ts", "export {};\n");
    game
}

fn compile(game: &Game, extension: &mut dyn Extension, target: Target<'_>) -> Result<Vec<(String, String)>, Failure> {
    let compiled = crucible_compiler::compile_with(&game.dir.join("tsconfig.json"), target, &mut [extension])?;
    Ok(compiled.modules.into_iter().map(|m| (m.path, m.source)).collect())
}

fn in_place(game: &Game) -> (std::path::PathBuf, std::path::PathBuf) {
    (game.project(), game.include())
}

fn module<'a>(modules: &'a [(String, String)], path: &str) -> &'a str {
    &modules.iter().find(|(p, _)| p == path).unwrap_or_else(|| panic!("{path} is emitted")).1
}

#[test]
fn every_hook_runs_and_its_lowering_lands_in_the_module() {
    let game = probe_game();
    game.main(
        "import { HERE, probe } from \"@rbxts/probe\";\n\
         class Crate {}\n\
         let count = 1;\n\
         count += 1;\n\
         probe(count, { at: HERE });\n\
         export const crate = probe(new Crate());\n",
    );
    game.write("src/server/other.ts", "import { probe } from \"@rbxts/probe\";\nprobe(\"label\");\n");
    let (project, include) = in_place(&game);
    let mut probe = Probe::default();
    let modules = compile(&game, &mut probe, Target::Game { project: &project, include: &include })
        .unwrap_or_else(|f| panic!("the game compiles:\n{f}"));
    assert_eq!(probe.name(), "probe");
    assert_eq!(probe.version.as_deref(), Some("1.2.3"));
    assert_eq!(probe.total, Some(3));

    let main = module(&modules, "server/main.luau");
    let runtime = "local __probe = TS.import(script, game:GetService(\"ReplicatedStorage\"), \"rbxts_include\", \
                   \"node_modules\", \"@rbxts\", \"probe\", \"runtime\")";
    has(main, runtime);
    has(main, "__probe.class(Crate, \"Crate\")");
    has(main, "__probe.probe(count, {\n\tat = \"main.ts:5\",\n}, \"number -> number\", 3)");
    has(main, "__probe.probe((Crate.new()), \"Crate -> Crate\", 3)");
    assert!(!main.contains("HERE,"), "every HERE in a probe was replaced:\n{main}");
    let tail = main.find("__probe.class").unwrap();
    assert!(main[..tail].contains("do\n"), "the class tail follows the class's block:\n{main}");

    let other = module(&modules, "server/other.luau");
    has(other, runtime);
    has(other, "__probe.probe(\"label\", \"\\\"label\\\" -> \\\"label\\\"\", 3)");
}

#[test]
fn a_module_without_a_probe_imports_no_helper() {
    let game = probe_game();
    game.main("export const value = 1;\n");
    let (project, include) = in_place(&game);
    let mut probe = Probe::default();
    let modules = compile(&game, &mut probe, Target::Game { project: &project, include: &include })
        .unwrap_or_else(|f| panic!("the game compiles:\n{f}"));
    assert_eq!(probe.version, None, "the program has no file from the package");
    let main = module(&modules, "server/main.luau");
    assert!(!main.contains("__probe"), "{main}");
}

#[test]
fn extension_reports_fail_the_compile_where_they_point() {
    let game = probe_game();
    game.main(
        "import { probe } from \"@rbxts/probe\";\n\
         declare const loose: any;\n\
         probe(loose);\n\
         probe(1);\n\
         probe(2);\n",
    );
    let (project, include) = in_place(&game);
    let mut probe = Probe { limit: Some(1), ..Probe::default() };
    let Err(Failure::Crucible(diagnostics)) =
        compile(&game, &mut probe, Target::Game { project: &project, include: &include })
    else {
        panic!("the extension's reports fail the compile");
    };
    let found: Vec<(Option<String>, u32, i32, &str)> = diagnostics
        .iter()
        .map(|d: &Diagnostic| (d.path.as_ref().map(|_| game.relative(d)), d.line, d.code, &*d.message))
        .collect();
    assert_eq!(
        found,
        [
            (None, 0, 0, "2 probes, more than the limit of 1"),
            (Some("src/server/main.ts".to_owned()), 3, 99001, "probe cannot read an `any` value"),
        ],
        "{}",
        lines(&diagnostics)
    );
}

#[test]
fn a_helper_the_place_cannot_reach_fails_with_its_message() {
    let game = probe_game();
    game.main("import { probe } from \"@rbxts/probe\";\nprobe(1);\n");
    let Err(Failure::Lower(errors)) = compile(&game, &mut Probe::default(), Target::Package) else {
        panic!("a package has no place for the helper");
    };
    let messages: Vec<&str> = errors.iter().map(|e| &*e.message).collect();
    assert_eq!(messages, [RUNTIME.missing]);
    assert!(Path::new(&errors[0].path).ends_with("src/server/main.ts"), "{}", errors[0]);
}

/// Each function `Scopes` lowers, the global its calls lower to, and what replaces a `HERE` while its arguments lower.
/// `outer` and `claim` replace each `HERE` in their own arguments. `reach` replaces every `HERE` of its file, even one
/// outside its call, and `inner` replaces none. `peek` lowers its arguments and then leaves the call to the compiler.
const SCOPED: &[(&str, &str, &str)] = &[
    ("outer", "out", "REPLACED"),
    ("claim", "claimed", "CLAIMED"),
    ("reach", "reached", "REACHED"),
    ("inner", "inside", ""),
    ("peek", "peek", ""),
];

#[derive(Default)]
struct Scopes {
    /// Each call to a scoped function, as the global it lowers to, the replacement and the `HERE`s it replaces.
    calls: HashMap<SiteId, (&'static str, &'static str, Vec<Node>)>,
}

impl Scopes {
    fn named(cx: &mut ExtractCx<'_>, n: Node) -> Option<&'static str> {
        if n.kind() != Kind::Identifier {
            return None;
        }
        cx.symbol_at(n).map(|symbol| symbol.name())
    }
}

impl Extension for Scopes {
    fn name(&self) -> &'static str {
        "scopes"
    }

    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: File) {
        let heres: Vec<Node> =
            descendants(file.root()).into_iter().filter(|&n| Scopes::named(cx, n) == Some("HERE")).collect();
        for n in descendants(file.root()) {
            if n.kind() != Kind::CallExpression {
                continue;
            }
            let Some(callee) = Scopes::named(cx, n.expression()) else { continue };
            let Some(&(function, global, replacement)) = SCOPED.iter().find(|(name, ..)| *name == callee) else {
                continue;
            };
            let replaced = match function {
                "inner" | "peek" => Vec::new(),
                "reach" => heres.clone(),
                _ => {
                    let inside = n.arguments().iter().flat_map(descendants).collect::<Vec<_>>();
                    heres.iter().copied().filter(|h| inside.contains(h)).collect()
                }
            };
            self.calls.insert(cx.site(n), (global, replacement, replaced));
        }
    }

    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr> {
        let (global, replacement, replaced) = self.calls.get(&cx.site(call))?;
        let replacements = replaced.iter().map(|&n| (n, Expr::string(*replacement))).collect();
        let arguments = cx.arguments(call, replacements);
        if *global == "peek" {
            return None;
        }
        Some(Expr::global(global).call(arguments))
    }
}

fn compile_scopes(main: &str) -> String {
    let game = Game::new();
    game.main(&format!(
        "{}declare const HERE: string;\nexport {{}};\n{main}",
        SCOPED
            .iter()
            .map(|(name, ..)| format!("declare function {name}(...values: unknown[]): unknown;\n"))
            .collect::<String>()
    ));
    let (project, include) = in_place(&game);
    let mut scopes = Scopes::default();
    let modules = compile(&game, &mut scopes, Target::Game { project: &project, include: &include })
        .unwrap_or_else(|f| panic!("the game compiles:\n{f}"));
    module(&modules, "server/main.luau").to_owned()
}

#[test]
fn a_call_in_the_arguments_of_another_keeps_its_replacements_and_its_own_win() {
    let main = compile_scopes("outer(inner(HERE));\nouter(claim(HERE), HERE);\n");
    has(&main, "out((inside(\"REPLACED\")))");
    has(&main, "out(claimed(\"CLAIMED\"), \"REPLACED\")");
}

#[test]
fn replacements_end_with_the_call_that_supplies_them() {
    let main = compile_scopes("outer(reach(), inner(HERE));\nreach();\ninner(HERE);\n");
    has(&main, "out(reached(), (inside(\"REPLACED\")))");
    has(&main, "\nreached()\ninside(HERE)\n");
}

#[test]
#[should_panic(expected = "extension `scopes` lowered the arguments of a call and then left the call to the compiler")]
fn declining_a_call_after_lowering_its_arguments_panics() {
    compile_scopes("outer(peek(HERE), HERE);\n");
}
