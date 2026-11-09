//! A test extension that uses every hook of the seam. It finds calls to `probe` from the `@rbxts/probe` package,
//! records each one's types from the checker, and lowers it to a call into the package's runtime entry with those
//! types and the program's probe count. It replaces each `HERE` in a probe's arguments with where the probe is, and
//! registers every class with the runtime after its declaration. A second one, `Heres`, replaces every `HERE` of a
//! file to test that an expression an extension lowers lowers in place wherever it sits.

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
        let mut arguments = cx.arguments(call);
        arguments.push(Expr::string(types.clone()));
        arguments.push(Expr::Number(self.total.expect("finish_program runs before lowering") as f64));
        Some(cx.helper(&RUNTIME, "probe").call(arguments))
    }

    fn lower_expression(&self, cx: &mut LowerCx<'_, '_>, n: Node) -> Option<Expr> {
        self.heres.get(&cx.site(n)).map(|at| Expr::string(at.clone()))
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

fn in_place(game: &Game) -> (crucible_compiler::InstanceMap, std::path::PathBuf) {
    (game.instances(), game.include())
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
    let (instances, include) = in_place(&game);
    let mut probe = Probe::default();
    let modules = compile(&game, &mut probe, Target::Game { instances: &instances, include: &include, modules: &[] })
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
    let (instances, include) = in_place(&game);
    let mut probe = Probe::default();
    let modules = compile(&game, &mut probe, Target::Game { instances: &instances, include: &include, modules: &[] })
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
    let (instances, include) = in_place(&game);
    let mut probe = Probe { limit: Some(1), ..Probe::default() };
    let Err(Failure::Crucible(diagnostics)) =
        compile(&game, &mut probe, Target::Game { instances: &instances, include: &include, modules: &[] })
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

/// Replaces every `HERE` of a file with `"REPLACED"`, and lowers each call to `outer` to a call to the global `out`.
/// `peek` lowers its arguments and then leaves the call to the compiler.
#[derive(Default)]
struct Heres {
    heres: Vec<SiteId>,
    calls: HashMap<SiteId, &'static str>,
}

impl Heres {
    fn named(cx: &mut ExtractCx<'_>, n: Node) -> Option<&'static str> {
        if n.kind() != Kind::Identifier {
            return None;
        }
        cx.symbol_at(n).map(|symbol| symbol.name())
    }
}

impl Extension for Heres {
    fn name(&self) -> &'static str {
        "heres"
    }

    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: File) {
        for n in descendants(file.root()) {
            match Heres::named(cx, n) {
                Some("HERE") => self.heres.push(cx.site(n)),
                Some(callee @ ("outer" | "peek")) if n.parent().is_some_and(|p| p.kind() == Kind::CallExpression) => {
                    let call = n.parent().unwrap();
                    if call.expression() == n {
                        self.calls.insert(cx.site(call), callee);
                    }
                }
                _ => {}
            }
        }
    }

    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr> {
        let callee = *self.calls.get(&cx.site(call))?;
        let arguments = cx.arguments(call);
        (callee == "outer").then(|| Expr::global("out").call(arguments))
    }

    fn lower_expression(&self, cx: &mut LowerCx<'_, '_>, n: Node) -> Option<Expr> {
        self.heres.contains(&cx.site(n)).then(|| Expr::string("REPLACED"))
    }
}

fn compile_heres(main: &str) -> String {
    let game = Game::new();
    game.main(&format!(
        "declare function outer(...values: unknown[]): unknown;\n\
         declare function peek(...values: unknown[]): unknown;\n\
         declare const maybe: ((...values: unknown[]) => unknown) | undefined;\n\
         declare const HERE: string;\nexport {{}};\n{main}"
    ));
    let (instances, include) = in_place(&game);
    let mut heres = Heres::default();
    let modules = compile(&game, &mut heres, Target::Game { instances: &instances, include: &include, modules: &[] })
        .unwrap_or_else(|f| panic!("the game compiles:\n{f}"));
    module(&modules, "server/main.luau").to_owned()
}

#[test]
fn an_expression_an_extension_lowers_lowers_in_place_on_every_path() {
    let main = compile_heres(
        "outer(HERE);\nprint(HERE);\nmaybe?.(HERE);\nexport const values = [HERE, { at: HERE }];\nouter(print(HERE));\n",
    );
    has(&main, "\nout(\"REPLACED\")\n");
    has(&main, "\nprint(\"REPLACED\")\n");
    has(&main, "_0(\"REPLACED\")");
    has(&main, "{ \"REPLACED\", {\n\tat = \"REPLACED\",\n} }");
    has(&main, "out((print(\"REPLACED\")))");
    assert!(!main.contains("HERE"), "every HERE lowered in place:\n{main}");
}

#[test]
#[should_panic(expected = "extension `heres` lowered the parts of a node and then left the node to the compiler")]
fn declining_a_call_after_lowering_its_arguments_panics() {
    compile_heres("outer(peek(HERE), HERE);\n");
}
