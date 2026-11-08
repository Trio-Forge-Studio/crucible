mod support;

use std::collections::{BTreeSet, HashMap};

use crucible_compiler::{Crucible, Declaration, Facts, Field, KeyLiteral, Literal, MapKey, Schema, Tail, Target};
use serde_json::json;
use support::{Game, has, lines, project, repo};

const IMPORTS: &str = "import { toServer, toClient, askServer, receive, ClientReady, Subsystem, CrucibleBehaviour, u8, u16, u32, f32, f64, ToServer, ToClient, AskServer, ReceiveTarget } from \"@trioforge/crucible\";\n";

fn declaration(t: &str) -> String {
    declare(t, "toServer", "X.Test")
}

fn declare(t: &str, kind: &str, name: &str) -> String {
    format!("export const Message = {kind}<{t}>(\"{name}\");")
}

fn fresh() -> Game {
    let game = Game::new();
    let mut tree = project();
    tree["tree"]["ReplicatedStorage"]["Util"] = json!({ "$path": "out/util" });
    game.json("default.project.json", &tree);
    game
}

fn fixture(path: &str) -> String {
    let file = repo().join("crates/transform/tests/transform-fixtures").join(path);
    std::fs::read_to_string(&file).unwrap_or_else(|_| panic!("{} exists", file.display())).replace('\r', "")
}

/// A game with the state and data declarations the JavaScript data tests install over the package's, which add
/// `profileStore`, `profileMessage` and `ServerOnly`.
fn data_game() -> Game {
    let game = fresh();
    let out = "node_modules/@trioforge/crucible/out";
    for (file, target, entry, line) in [
        ("state-data/state.d.ts", "state.d.ts", "index.d.ts", "export * from \"./state\";"),
        ("state-data/data.d.ts", "data.d.ts", "index.d.ts", "export * from \"./data\";"),
        ("state-data/server-data.d.ts", "server/data.d.ts", "server/index.d.ts", "export * from \"./data\";"),
        ("state-data/emit.d.ts", "emit/data.d.ts", "emit/index.d.ts", "export * from \"./data\";"),
    ] {
        // TypeScript 7 reports the fixture's `ReservedKeyCheck` as circular where it recurses into Roblox types, and
        // CRU026 checks the same names.
        let text =
            fixture(file).replace("options: ProfileOptions<T> & ReservedKeyCheck<T>,", "options: ProfileOptions<T>,");
        game.write(&format!("{out}/{target}"), &text);
        let index = game.dir.join(out).join(entry);
        let text = std::fs::read_to_string(&index).unwrap();
        if !text.contains(line) {
            std::fs::write(&index, format!("{text}\n{line}\n")).unwrap();
        }
    }
    game.write(&format!("{out}/emit/schema.d.ts"), &fixture("network/schema.d.ts"));
    game
}

struct Report {
    file: String,
    code: i32,
    message: String,
}

fn check(game: &Game) -> (Vec<Report>, Vec<String>) {
    let mut type_errors = Vec::new();
    loop {
        match game.analyze() {
            Ok(facts) => {
                let reports = facts
                    .diagnostics
                    .iter()
                    .filter(|d| d.code >= 90000)
                    .map(|d| Report { file: game.relative(d), code: d.code, message: d.message.clone() })
                    .collect();
                return (reports, type_errors);
            }
            Err(diagnostics) => {
                let files: BTreeSet<String> = diagnostics.iter().map(|d| game.relative(d)).collect();
                assert!(
                    files.iter().all(|f| f.starts_with("src/")),
                    "only sources are rejected:\n{}",
                    lines(&diagnostics)
                );
                type_errors.extend(diagnostics.iter().map(|d| format!("{}:{}", game.relative(d), d.line)));
                for file in files {
                    game.remove(&file);
                }
            }
        }
    }
}

/// A top-level statement's first line. The case files indent or close every other line.
fn starts_statement(line: &str) -> bool {
    line.starts_with(|c: char| c.is_ascii_alphabetic())
}

fn check_statements(game: &Game) -> (Vec<Report>, Vec<(String, usize, usize)>) {
    let mut blanked: Vec<(String, usize, usize)> = Vec::new();
    loop {
        let diagnostics = match game.analyze() {
            Ok(facts) => {
                let reports = facts
                    .diagnostics
                    .iter()
                    .filter(|d| d.code >= 90000)
                    .map(|d| Report { file: game.relative(d), code: d.code, message: d.message.clone() })
                    .collect();
                return (reports, blanked);
            }
            Err(diagnostics) => diagnostics,
        };
        for d in &diagnostics {
            let file = game.relative(d);
            assert!(file.starts_with("src/"), "only sources are rejected:\n{}", lines(&diagnostics));
            let text = std::fs::read_to_string(game.dir.join(&file)).unwrap();
            let mut rows: Vec<String> = text.split('\n').map(str::to_owned).collect();
            let at = d.line as usize - 1;
            let start = (0..=at).rev().find(|&i| starts_statement(&rows[i])).unwrap_or(at);
            let end = (at + 1..rows.len()).find(|&i| starts_statement(&rows[i])).unwrap_or(rows.len());
            assert!(rows[start..end].iter().any(|r| !r.trim().is_empty()), "a type error outside any statement:\n{d}");
            for row in &mut rows[start..end] {
                row.clear();
            }
            game.write(&file, &rows.join("\n"));
            blanked.push((file, start + 1, end));
        }
    }
}

fn line_naming(text: &str, name: &str) -> Option<usize> {
    let quoted = format!("\"{name}\"");
    text.lines().position(|l| l.contains(&quoted)).map(|i| i + 1)
}

fn reports_of(game: &Game) -> Vec<Report> {
    let facts = game.analyze().unwrap_or_else(|d| panic!("the game type-checks:\n{}", lines(&d)));
    facts
        .diagnostics
        .iter()
        .filter(|d| d.code >= 90000)
        .map(|d| Report { file: game.relative(d), code: d.code, message: d.message.clone() })
        .collect()
}

fn messages(reports: &[Report], file: &str) -> Vec<String> {
    reports.iter().filter(|r| r.file == file).map(|r| r.message.clone()).collect()
}

fn files_with(reports: &[Report], code: i32) -> Vec<String> {
    let files: BTreeSet<String> = reports.iter().filter(|r| r.code == code).map(|r| r.file.clone()).collect();
    files.into_iter().collect()
}

fn none(reports: &[Report]) {
    let all: Vec<String> = reports.iter().map(|r| format!("{} {}", r.file, r.message)).collect();
    assert_eq!(all, Vec::<String>::new());
}

/// Luau indexes the `nil` builder as `n["nil"]`, because `nil` is a keyword.
fn canon(code: &str) -> String {
    canon_tokens(code).replace("__crucible.n{\"nil\"}", "__crucible.n.nil")
}

fn canon_tokens(code: &str) -> String {
    let mut out = String::new();
    let mut chars = code.chars().peekable();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        out.push_str(if word == "undefined" { "nil" } else { word });
        word.clear();
    };
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        flush(&mut word, &mut out);
        match c {
            '"' => {
                out.push(c);
                while let Some(c) = chars.next() {
                    out.push(c);
                    if c == '\\' {
                        out.extend(chars.next());
                    } else if c == '"' {
                        break;
                    }
                }
            }
            '[' => out.push('{'),
            ']' => out.push('}'),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    flush(&mut word, &mut out);
    out
}

fn arguments(code: &str, open: usize) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    let mut chars = code[open..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                current.push(c);
                while let Some(c) = chars.next() {
                    current.push(c);
                    if c == '\\' {
                        current.extend(chars.next());
                    } else if c == '"' {
                        break;
                    }
                }
                continue;
            }
            '(' | '{' | '[' => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            ')' | '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    if !current.trim().is_empty() {
                        args.push(current.trim().to_owned());
                    }
                    return args;
                }
            }
            ',' if depth == 1 => {
                args.push(current.trim().to_owned());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    args
}

fn calls(luau: &str, names: &[&str]) -> HashMap<String, Vec<String>> {
    let mut found = HashMap::new();
    for name in names {
        let head = format!("__crucible.{name}(");
        for (at, _) in luau.match_indices(&head) {
            let args = arguments(luau, at + head.len() - 1);
            if let Some(first) = args.first().and_then(|a| a.strip_prefix('"')).and_then(|a| a.strip_suffix('"')) {
                found.insert(first.to_owned(), args);
            }
        }
    }
    found
}

const MESSAGES: [&str; 3] = ["toServer", "toClient", "askServer"];
const DATA: [&str; 7] =
    ["replicated", "replicatedPerPlayer", "profileStore", "dataStore", "profileMessage", "config", "playerConfig"];

mod n {
    pub fn kind(name: &str) -> String {
        format!("__crucible.n.{name}")
    }
    pub fn s() -> String {
        kind("string")
    }
    pub fn f() -> String {
        kind("f64")
    }
    pub fn b() -> String {
        kind("bool")
    }
    pub fn nil() -> String {
        kind("nil")
    }
    pub fn l(value: &str) -> String {
        format!("{}({value})", kind("literal"))
    }
    pub fn ls(value: &str) -> String {
        l(&format!("\"{value}\""))
    }
    pub fn u(branches: &[String]) -> String {
        format!("{}([{}])", kind("union"), branches.join(", "))
    }
    pub fn m(key: String, value: String) -> String {
        format!("{}({key}, {value})", kind("map"))
    }
    pub fn o(fields: &[(&str, String)]) -> String {
        let fields: Vec<String> = fields.iter().map(|(name, node)| format!("[\"{name}\", {node}]")).collect();
        format!("{}([{}])", kind("object"), fields.join(", "))
    }
    pub fn v(key: &str, value: &str, body: String) -> String {
        format!("{}(\"{key}\", \"{value}\", {body})", kind("variant"))
    }
}

fn same(luau: &str, js: &str, label: &str) {
    assert_eq!(canon(luau), canon(js), "{label}");
}

fn declarations(facts: &Facts, game: &Game, file: &str) -> HashMap<String, Declaration> {
    let source = std::fs::read_to_string(game.dir.join(file)).unwrap();
    let table = facts.files.iter().find(|f| f.path.ends_with(file)).unwrap_or_else(|| panic!("{file} has facts"));
    let mut found = HashMap::new();
    for (span, fact) in &table.facts {
        if let Some(Crucible::Declaration(d)) = fact.crucible.as_deref() {
            let call = &source[span.start as usize..span.end as usize];
            found.insert(call_name(call), d.clone());
        }
    }
    found
}

/// The string a declaration call's first argument holds, past type arguments that may hold `=>`.
fn call_name(call: &str) -> String {
    let bytes = call.as_bytes();
    let mut i = call.find(['<', '(']).unwrap_or(0);
    if bytes[i] == b'<' {
        let mut depth = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'=' if bytes.get(i + 1) == Some(&b'>') => i += 1,
                b'<' => depth += 1,
                b'>' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    let rest = &call[i..];
    let start = rest.find('"').expect("the call names its declaration") + 1;
    let end = rest[start..].find('"').unwrap() + start;
    rest[start..end].to_owned()
}

fn json_literal(value: &Literal) -> String {
    match value {
        Literal::Boolean(b) => b.to_string(),
        Literal::Number(n) => serde_json::to_string(n).unwrap().trim_end_matches(".0").to_owned(),
        Literal::String(s) => serde_json::to_string(s.as_ref()).unwrap(),
    }
}

/// The probes' printer, which `state-data-harness.cjs` shares.
fn describe(node: &Schema, marked: &[Vec<String>], prefix: &[String]) -> String {
    match node {
        Schema::Literal(value) => json_literal(value),
        Schema::Object(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|Field { name, value, .. }| {
                    let mut path = prefix.to_vec();
                    path.push(name.to_string());
                    let mark = if marked.contains(&path) { " (serverOnly)" } else { "" };
                    format!("{name}{mark}: {}", describe(value, marked, &path))
                })
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        Schema::Array(element) => format!("{}[]", describe(element, &[], &[])),
        Schema::Tuple(elements) => {
            format!("[{}]", elements.iter().map(|e| describe(e, &[], &[])).collect::<Vec<_>>().join(", "))
        }
        Schema::Map(key, value) => {
            let key = match key {
                MapKey::String => "string".to_owned(),
                MapKey::U8 => "u8".to_owned(),
                MapKey::U16 => "u16".to_owned(),
                MapKey::U32 => "u32".to_owned(),
                MapKey::Literals(keys) => keys
                    .iter()
                    .map(|k| match k {
                        KeyLiteral::Number(n) => json_literal(&Literal::Number(*n)),
                        KeyLiteral::String(s) => json_literal(&Literal::String(s.clone())),
                    })
                    .collect::<Vec<_>>()
                    .join(" | "),
            };
            format!("{{ [{key}]: {} }}", describe(value, &[], &[]))
        }
        Schema::Union(branches) => branches.iter().map(|b| describe(b, &[], &[])).collect::<Vec<_>>().join(" | "),
        Schema::Variant { key, value, body } => {
            format!("{{ {key}: {}, ...{} }}", json_literal(value), describe(body, &[], &[]))
        }
        Schema::Nil => "undefined".to_owned(),
        Schema::F64 => "number".to_owned(),
        Schema::Bool => "boolean".to_owned(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn count(node: &Schema) -> usize {
    1 + match node {
        Schema::Object(fields) => fields.iter().map(|f| count(&f.value)).sum(),
        Schema::Array(element) => count(element),
        Schema::Map(_, value) => count(value),
        Schema::Variant { body, .. } => count(body),
        Schema::Tuple(nodes) | Schema::Union(nodes) => nodes.iter().map(count).sum(),
        _ => 0,
    }
}

fn shape(node: &Schema) -> String {
    format!("{} nodes: {}", count(node), describe(node, &[], &[]))
}

#[derive(Debug, Default)]
struct Verdict {
    name: String,
    lowers: Option<bool>,
    details: Vec<String>,
}

impl Verdict {
    fn diagnostics(&self) -> Vec<(String, String)> {
        self.details
            .iter()
            .filter_map(|d| {
                let rest = d.strip_prefix("CRU")?;
                let (code, rest) = rest.split_once(' ')?;
                let (_, rest) = rest.split_once(' ')?;
                let (at, _) = rest.split_once(": ")?;
                Some((format!("CRU{code}"), at.to_owned()))
            })
            .collect()
    }

    fn shape(&self) -> Option<&str> {
        self.details
            .iter()
            .find(|d| d.contains(" nodes: ") && d.starts_with(|c: char| c.is_ascii_digit()))
            .map(|d| d.as_str())
    }

    fn line(&self, prefix: &str) -> Option<&str> {
        self.details.iter().find(|d| d.starts_with(prefix)).map(|d| d.as_str())
    }
}

fn verdicts(file: &str) -> Vec<Verdict> {
    let text = fixture(&format!("state-data/{file}"));
    let mut entries: Vec<Verdict> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let verdict = |line: &str| {
            if line.ends_with(": lowers") {
                Some(true)
            } else if line.ends_with(": rejected") {
                Some(false)
            } else {
                None
            }
        };
        if (line.starts_with("ok ") || line.starts_with("BAD")) && line.contains(':') {
            let mut words = line.split_whitespace();
            let (_, location, name) = (words.next(), words.next(), words.next());
            let located = location
                .and_then(|l| l.rsplit_once(':'))
                .is_some_and(|(_, line)| !line.is_empty() && line.chars().all(|c| c.is_ascii_digit()));
            if located && let Some(name) = name {
                entries.push(Verdict { name: name.to_owned(), lowers: verdict(line), details: Vec::new() });
                open = true;
                continue;
            }
        }
        match entries.last_mut() {
            Some(last) if open && last.lowers.is_none() => last.lowers = verdict(line),
            Some(last) if open && line.starts_with("      ") => last.details.push(line.trim().to_owned()),
            _ => open = false,
        }
    }
    entries
}

/// Paths differ only in how a map is spelled: the probe writes `[key]`, the transform `.key` and `.value`.
fn normal_path(at: &str) -> String {
    let at = at.replace("[key]", "");
    let mut out = String::new();
    let mut rest = at.as_str();
    while let Some(i) = rest.find(['.']) {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let word = after.find(['.', '[']).map_or(after, |end| &after[..end]);
        if word == "key" || word == "value" {
            rest = &after[word.len()..];
        } else {
            out.push('.');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn reported(reports: &[Report], name: &str) -> Vec<(String, String)> {
    reports
        .iter()
        .filter_map(|r| {
            let (code, rest) = r.message.split_once(": ")?;
            let (declared, rest) = rest.split_once(' ')?;
            let (at, _) = rest.split_once(": ")?;
            (declared == name).then(|| (code.to_owned(), at.to_owned()))
        })
        .collect()
}

fn expect_rejections(reports: &[Report], entries: &[(String, Vec<(String, String)>)]) {
    for (name, expected) in entries {
        let mine = reported(reports, name);
        assert!(!mine.is_empty(), "{name} is rejected");
        for (code, at) in &mine {
            assert!(
                expected.iter().any(|(c, e)| c == code && normal_path(at).starts_with(&normal_path(e))),
                "{name} {code} at {at} matches the probe's {expected:?}"
            );
        }
    }
}

#[test]
fn descriptors_lower_with_precise_builders_and_keep_request_response_order() {
    let game = fresh();
    game.write(
        "src/shared/messages.ts",
        &format!(
            "{IMPORTS}
export const A = toServer<{{ zebra: u8; alpha: string; flag: boolean }}>(\"X.Object\");
export const B = askServer<string, u16>(\"X.Ask\", {{ timeout: 37 }});
export const C = askServer<undefined, unknown>(\"X.Default\");
export const D = toServer<Map<\"z\" | \"a\", number>>(\"X.Map\");
export const E = toServer<[string, u32]>(\"X.Tuple\");
export const F = toClient<{{ maybe: BasePart | undefined; player: Player }}>(\"X.Stream\");
export const G = toServer<{{ Kind: \"Leave\" }} | {{ Kind: \"Join\"; UserId: number }}>(\"X.Variant\");
export const H = toServer<\"z\" | \"a\" | 7 | 2 | false>(\"X.Literals\");
export const Kinds = toServer<number | string | Vector3>(\"X.Kinds\");
export const Choice = toServer<{{z:\"Z1\";a:\"A1\";left:number}}|{{z:\"Z2\";a:\"A2\";right:string}}>(\"X.Choice\");
export const Tagged = toServer<{{Kind:\"A\";Tag:\"A\";x:number}}|{{Kind:\"A\";Tag:\"B\";y:string}}>(\"X.Tagged\");
"
        ),
    );
    none(&reports_of(&game));
    let luau = game.luau("shared/messages.luau");
    let d = calls(&luau, &MESSAGES);
    let first = |name: &str| d[name][1].clone();
    same(
        &first("X.Object"),
        "__crucible.n.object([[\"alpha\", __crucible.n.string], [\"flag\", __crucible.n.bool], [\"zebra\", __crucible.n.u8]])",
        "X.Object",
    );
    assert_eq!(d["X.Ask"][1..], ["__crucible.n.string", "__crucible.n.u16", "37"]);
    let default: Vec<String> = d["X.Default"][1..].iter().map(|a| canon(a)).collect();
    assert_eq!(default, ["__crucible.n.nil", "__crucible.n.dynamic", "nil"]);
    same(
        &first("X.Map"),
        "__crucible.n.map(__crucible.n.union([__crucible.n.literal(\"a\"), __crucible.n.literal(\"z\")]), __crucible.n.f64)",
        "X.Map",
    );
    same(&first("X.Tuple"), "__crucible.n.tuple([__crucible.n.string, __crucible.n.u32])", "X.Tuple");
    same(
        &first("X.Kinds"),
        "__crucible.n.union([__crucible.n.vector3, __crucible.n.f64, __crucible.n.string])",
        "X.Kinds",
    );
    assert!(canon(&first("X.Choice")).contains(&canon("n.variant(\"a\", \"A1\"")), "{}", first("X.Choice"));
    same(
        &first("X.Tagged"),
        "__crucible.n.union([__crucible.n.variant(\"Tag\", \"A\", __crucible.n.object([[\"Kind\", __crucible.n.literal(\"A\")], [\"x\", __crucible.n.f64]])), __crucible.n.variant(\"Tag\", \"B\", __crucible.n.object([[\"Kind\", __crucible.n.literal(\"A\")], [\"y\", __crucible.n.string]]))])",
        "X.Tagged",
    );
    same(
        &first("X.Literals"),
        "__crucible.n.union([__crucible.n.literal(false), __crucible.n.literal(2), __crucible.n.literal(7), __crucible.n.literal(\"a\"), __crucible.n.literal(\"z\")])",
        "X.Literals",
    );
    assert!(
        canon(&first("X.Stream")).contains(&canon("union([__crucible.n.nil, __crucible.n.instance([\"BasePart\"])])")),
        "{}",
        first("X.Stream")
    );
    same(
        &first("X.Variant"),
        "__crucible.n.union([__crucible.n.variant(\"Kind\", \"Join\", __crucible.n.object([[\"UserId\", __crucible.n.f64]])), __crucible.n.variant(\"Kind\", \"Leave\", __crucible.n.object([]))])",
        "X.Variant",
    );
    has(&luau, "askServer(\"X.Ask\", __crucible.n.string, __crucible.n.u16, 37)");
}

fn negatives() -> Vec<(String, &'static str, String)> {
    let mut cases: Vec<(String, &'static str, String)> = vec![
        ("local const".into(), "CRU017", "const X = toServer<string>(\"X.Test\");".into()),
        ("let".into(), "CRU017", "export let X = toServer<string>(\"X.Test\");".into()),
        ("nested".into(), "CRU017", "export function make() { return toServer<string>(\"X.Test\"); }".into()),
        ("missing type".into(), "CRU017", "export const X = toServer(\"X.Test\");".into()),
        (
            "options variable".into(),
            "CRU017",
            "const o = { timeout: 3 }; export const X = askServer<string, number>(\"X.Test\", o);".into(),
        ),
        ("options empty".into(), "CRU017", "export const X = askServer<string, number>(\"X.Test\", {});".into()),
        (
            "timeout zero".into(),
            "CRU017",
            "export const X = askServer<string, number>(\"X.Test\", {timeout: 0});".into(),
        ),
        (
            "timeout ceiling".into(),
            "CRU017",
            "export const X = askServer<string, number>(\"X.Test\", {timeout: 121});".into(),
        ),
        (
            "timeout expression".into(),
            "CRU017",
            "export const X = askServer<string, number>(\"X.Test\", {timeout: 1 + 2});".into(),
        ),
        (
            "extra options".into(),
            "CRU017",
            "export const X = askServer<string, number>(\"X.Test\", {timeout: 2, extra: 1});".into(),
        ),
        (
            "computed option".into(),
            "CRU017",
            "export const X = askServer<string, number>(\"X.Test\", {[\"timeout\"]: 2});".into(),
        ),
        ("name variable".into(), "CRU018", "const n = \"X.Test\"; export const X = toServer<string>(n);".into()),
        ("name template".into(), "CRU018", "export const X = toServer<string>(`X.Test`);".into()),
        ("name concatenation".into(), "CRU018", "export const X = toServer<string>(\"X.\" + \"Test\");".into()),
    ];
    let long = format!("X.{}", "a".repeat(63));
    for name in ["", "1.Bad", "A..B", "A-b", "Crucible", "Crucible.X", long.as_str()] {
        cases.push((format!("invalid name {name}"), "CRU018", format!("export const X = toServer<string>({name:?});")));
    }
    for t in [
        "void",
        "any",
        "never",
        "symbol",
        "bigint",
        "{}",
        "{}[]",
        "Record<never, string>",
        "() => void",
        "{ f(): void }",
        "DateTime",
        "(string | undefined)[]",
        "[string, ...number[]]",
        "[string | undefined]",
        "Map<string, number | undefined>",
        "Map<f32, string>",
        "Map<f64, string>",
        "Map<number, string>",
        "{\"-1\": string}",
        "{2.5: string}",
        "{a: number} | {b: string}",
        "u8 | number",
        "string[] | {n: number}",
        "string[] | number[]",
        "string & {x: number}",
    ] {
        cases.push((format!("unsupported {t}"), "CRU019", declaration(t)));
    }
    let rest: Vec<(&str, &'static str, String)> = vec![
        ("numeric property path", "CRU019: X.Test payload.1: numeric property names", declaration("{1: string}")),
        (
            "nested property path",
            "CRU019: X.Test payload.inner.bad: any cannot be checked",
            declaration("{inner: {bad: any}}"),
        ),
        ("undefined field", "CRU019: X.Test payload.a: ", declaration("{a: undefined; b: string}")),
        ("optional undefined field", "CRU019: X.Test payload.a: ", declaration("{a?: undefined; b: string}")),
        ("optional discriminant", "CRU019", declaration("{Kind?:\"A\";a:number}|{Kind:\"B\";b:string}")),
        ("shared discriminant literal", "CRU019", declaration("{Kind:\"A\";a:number}|{Kind:\"A\";b:string}")),
        ("array beside variants", "CRU019", declaration("string[]|{Kind:\"A\"}|{Kind:\"B\"}")),
        ("nonfinite key literal", "CRU019", declaration("Map<1e400, string>")),
        (
            "width part with another property",
            "CRU019",
            format!(
                "type Brand = u8 extends number & infer B ? B : never; type Flat<T> = {{ [K in keyof T]: T[K] }}; {}",
                declaration("number & Flat<Brand & { extra: string }>")
            ),
        ),
        (
            "extra declaration argument",
            "CRU017: Unexpected declaration arguments.",
            "export const X = toServer<string>(\"X.Test\", {} as never);".into(),
        ),
        ("optional tuple element", "CRU019: X.Test payload: tuples must be fixed length", declaration("[string?]")),
        ("empty interface", "CRU019", format!("interface Empty {{}} {}", declaration("Empty"))),
        (
            "recursive interface",
            "CRU019",
            format!("interface Recursive {{ next?: Recursive }} {}", declaration("Recursive")),
        ),
        (
            "numeric enum properties",
            "CRU019",
            format!("const enum Slot {{ A, B }} {}", declaration("Record<Slot,string>")),
        ),
        ("numeric discriminant", "CRU019", declaration("{1:\"A\";value:number}|{1:\"B\"}")),
        ("conflicting widths", "CRU019", declaration("u8 & u16")),
        ("nonfinite literal", "CRU019", declaration("1e400")),
        ("null", "CRU019", declaration("null")),
        ("invalid key kind", "CRU019", declaration("Map<boolean,string>")),
        ("mixed record", "CRU019", declaration("{[key:string]:number;named:number}")),
        ("construct signature", "CRU019", declaration("new()=>{value:string}")),
        (
            "interface method",
            "CRU019",
            format!("interface MethodPayload {{ run():void }} {}", declaration("MethodPayload")),
        ),
        (
            "class method",
            "CRU019",
            format!("class MethodPayload {{ value=1; run() {{}} }} {}", declaration("MethodPayload")),
        ),
        (
            "bracket macro escape",
            "CRU023",
            "import * as C from \"@trioforge/crucible\"; export const f=C[\"toServer\"];".into(),
        ),
        (
            "symbol discriminant",
            "CRU019",
            format!(
                "declare const key: unique symbol; type Tagged = {{readonly [key]:\"A\"}}|{{readonly [key]:\"B\"}};{}",
                declaration("Tagged")
            ),
        ),
        ("template type", "CRU019", format!("type T = `a${{string}}`; {}", declaration("T"))),
        ("generic payload", "CRU019", "function f<T>() { return toServer<T>(\"X.Test\"); }".into()),
        ("required client instance", "CRU020", declare("BasePart", "toClient", "X.Test")),
        (
            "client instance path",
            "CRU020: X.Test payload.part: a client may not",
            declare("{part: BasePart}", "toClient", "X.Test"),
        ),
        ("client array", "CRU020", declare("BasePart[]", "toClient", "X.Test")),
        ("client tuple", "CRU020", declare("[BasePart]", "toClient", "X.Test")),
        ("client map", "CRU020", declare("Map<string, BasePart>", "toClient", "X.Test")),
        ("response streaming", "CRU020", "export const X = askServer<undefined, Model>(\"X.Test\");".into()),
        ("macro value", "CRU023", "export const f = toServer;".into()),
        ("macro parameter", "CRU023", "function use(f: unknown) {} use(receive);".into()),
        (
            "namespace escape",
            "CRU023",
            "import * as C from \"@trioforge/crucible\"; export const f = C.toClient;".into(),
        ),
    ];
    cases.extend(rest.into_iter().map(|(name, code, source)| (name.to_owned(), code, source)));
    cases
}

#[test]
fn the_diagnostic_catalogue_rejects_every_unsupported_declaration() {
    let cases = negatives();
    let game = fresh();
    for (i, (_, _, source)) in cases.iter().enumerate() {
        game.write(&format!("src/shared/n{i}.ts"), &format!("{IMPORTS}{source}"));
    }
    for side in ["server", "client", "neutral"] {
        game.write(
            &format!("src/{side}/wrong.ts"),
            &format!("{IMPORTS}{}", declare("string", "toServer", &format!("X.{side}"))),
        );
    }
    let (reports, type_errors) = check(&game);
    for (i, (name, code, _)) in cases.iter().enumerate() {
        let file = format!("src/shared/n{i}.ts");
        if ["extra options", "extra declaration argument"].contains(&name.as_str()) {
            assert!(type_errors.contains(&format!("{file}:2")), "{name} is a type error: {type_errors:?}");
            continue;
        }
        let found = messages(&reports, &file);
        assert!(found.iter().any(|m| m.starts_with(code)), "{name} reports {code}: {found:?}");
    }
    assert_eq!(type_errors.len(), 2, "{type_errors:?}");
    for side in ["server", "client", "neutral"] {
        let file = format!("src/{side}/wrong.ts");
        assert!(reports.iter().any(|r| r.file == file && r.code == 90017), "{side} placement");
    }
    let exact = fresh();
    exact.config(|config| config["compilerOptions"]["exactOptionalPropertyTypes"] = json!(true));
    exact.write(
        "src/shared/optional.ts",
        &format!("{IMPORTS}{}", declaration("{Kind?:\"A\";a:number}|{Kind:\"B\";b:string}")),
    );
    let found = messages(&reports_of(&exact), "src/shared/optional.ts");
    assert!(
        found
            .iter()
            .any(|m| m.starts_with("CRU019: X.Test payload: union members ") && m.contains(" have no discriminant")),
        "{found:?}"
    );
}

const RECEIVE_MESSAGES: &str = "export const Event = toServer<string>(\"X.Event\"); export const Out = toClient<string>(\"X.Out\"); export const Ask = askServer<string, number>(\"X.Ask\"); export const AskAny = askServer<string, unknown>(\"X.AskAny\");";

fn receiver_header() -> String {
    format!("{IMPORTS}import {{Event, Out, Ask, AskAny}} from \"../shared/messages\";\n")
}

#[test]
fn receives_lower_owner_field_target_and_callback_and_keep_their_placement_sides_and_returns() {
    let game = fresh();
    game.write("src/shared/messages.ts", &format!("{IMPORTS}{RECEIVE_MESSAGES}"));
    game.write(
        "src/server/good.ts",
        &format!(
            "{}
declare const cond: boolean;
function factory(): (p: Player, v: string) => void {{ return () => {{}}; }}
function stop(): never {{ throw \"stop\"; }}
let mutable: (p: Player, v: string) => void = () => 5;
export abstract class Base extends Subsystem {{ readonly input = receive(Event, (p, s) => {{ print(p.Name, s); }}); readonly plain: unknown = 1; }}
export class Good extends Base {{
 override readonly plain = receive(Event, () => {{}});
 readonly stops = receive(Event, () => stop());
 readonly loose = receive(Event, mutable);
 readonly ready = receive(ClientReady, p => cond ? () => print(p.Name) : undefined);
 readonly readyBlock = receive(ClientReady, p => {{ if(cond) return () => print(p.Name); }});
 readonly request = receive(Ask, (p, s) => s.size());
 readonly made = receive(Event, factory());
}}",
            receiver_header()
        ),
    );
    game.write(
        "src/client/good.ts",
        &format!(
            "{}export class Good extends Subsystem {{ readonly out = receive(Out, s => {{ print(s); }}); }}",
            receiver_header()
        ),
    );
    none(&reports_of(&game));
    let luau = game.luau("server/good.luau");
    has(&luau, "self.input = __crucible.receive(self, \"input\", Event, function(p, s)");
    has(&luau, "self.plain = __crucible.receive(self, \"plain\", Event, function() end)");
    has(&luau, "self.made = __crucible.receive(self, \"made\", Event, (factory()))");
    has(&game.luau("client/good.luau"), "self.out = __crucible.receive(self, \"out\", Out, function(s)");

    let bad: [(&str, &str); 23] = [
        ("CRU021", "const x = receive(Event, () => {});"),
        (
            "CRU021",
            "class X extends CrucibleBehaviour { readonly tag = \"x\"; readonly r = receive(Event, () => {}); }",
        ),
        ("CRU021", "class X extends Subsystem { static readonly r = receive(Event, () => {}); }"),
        ("CRU021", "class X extends Subsystem { r = receive(Event, () => {}); }"),
        ("CRU021", "class X extends Subsystem { method() { receive(Event, () => {}); } }"),
        (
            "CRU021",
            "declare const target: ToServer<string> | ToClient<string>; class X extends Subsystem { readonly r = receive(target, () => {}); }",
        ),
        (
            "CRU021",
            "declare const target: ReceiveTarget; class X extends Subsystem { readonly r = receive(target, () => {}); }",
        ),
        ("CRU021", "declare const target: any; class X extends Subsystem { readonly r = receive(target, () => {}); }"),
        (
            "CRU021",
            "abstract class Base extends Subsystem { readonly r = receive(Event, () => {}); } class X extends Base { readonly r = receive(Event, () => {}); }",
        ),
        ("CRU017", "class X extends Subsystem { readonly r = receive(toServer<string>(\"X.Inline\"), () => {}); }"),
        ("CRU021", "class X extends Subsystem { readonly r = receive(Event, () => { receive(Event, () => {}); }); }"),
        ("CRU022", "class X extends Subsystem { readonly r = receive(Out, () => {}); }"),
        ("CRU012", "class X extends Subsystem { readonly r = receive(Event, async () => {}); }"),
        (
            "CRU012",
            "const cb: (p: Player, s: string) => void = async () => {}; class X extends Subsystem { readonly r = receive(Event, cb); }",
        ),
        ("CRU012", "class X extends Subsystem { readonly r = receive(Event, p => new Set<Player>().add(p)); }"),
        ("CRU012", "function cb() { return 5; } class X extends Subsystem { readonly r = receive(Event, cb); }"),
        (
            "CRU012",
            "declare const cond: boolean; class X extends Subsystem { readonly r = receive(Event, p => cond ? new Set<Player>().add(p) : undefined); }",
        ),
        (
            "CRU012",
            "declare const cond: boolean; const cb: (p: Player) => void = p => cond ? (async () => {})() : undefined; class X extends Subsystem { readonly r = receive(ClientReady, cb); }",
        ),
        (
            "CRU012",
            "declare const cond: boolean; const cb: (p: Player) => void = p => cond ? 5 : undefined; class X extends Subsystem { readonly r = receive(ClientReady, cb); }",
        ),
        (
            "CRU012",
            "const helper = { cb() { return 7; } }; class X extends Subsystem { readonly r = receive(Event, helper.cb); }",
        ),
        ("CRU012", "class X extends Subsystem { readonly r = receive(Ask, async () => 5); }"),
        ("CRU012", "class X extends Subsystem { readonly r = receive(Event, () => () => {}); }"),
        (
            "CRU012",
            "function later(): Promise<number> { return undefined as never; } class X extends Subsystem { readonly r = receive(AskAny, () => later()); }",
        ),
    ];
    let game = fresh();
    game.write("src/shared/messages.ts", &format!("{IMPORTS}{RECEIVE_MESSAGES}"));
    for (i, (_, source)) in bad.iter().enumerate() {
        game.write(&format!("src/server/b{i}.ts"), &format!("{}{source}", receiver_header()));
    }
    for (side, token) in [("client", "ClientReady"), ("shared", "Event"), ("neutral", "Event")] {
        game.write(
            &format!("src/{side}/bad.ts"),
            &format!("{}class X extends Subsystem {{ readonly r = receive({token}, () => {{}}); }}", receiver_header()),
        );
    }
    let (reports, type_errors) = check(&game);
    assert_eq!(
        type_errors,
        ["src/server/b1.ts:3", "src/server/b20.ts:3"],
        "a behaviour needs its tag, and an async Ask callback returns a Promise its type rejects"
    );
    for (i, (code, _)) in bad.iter().enumerate() {
        let file = format!("src/server/b{i}.ts");
        if [1, 20].contains(&i) {
            continue;
        }
        let found = messages(&reports, &file);
        assert!(found.iter().any(|m| m.starts_with(code)), "receiver {i} reports {code}: {found:?}");
    }
    for side in ["client", "shared", "neutral"] {
        let found = messages(&reports, &format!("src/{side}/bad.ts"));
        assert!(found.iter().any(|m| m.starts_with("CRU022")), "{side}: {found:?}");
    }
}

#[test]
fn duplicate_names_and_transitive_global_or_merged_payloads_fail_at_the_declaring_file() {
    let game = fresh();
    game.write("src/shared/a.ts", &format!("{IMPORTS}{}", declaration("string")));
    game.write("src/shared/b.ts", &format!("{IMPORTS}{}", declare("string, number", "askServer", "X.Test")));
    game.write("src/global.d.ts", "interface ScriptPayload { value: string }\n");
    game.write("src/alias.d.ts", "type GlobalString = string; type GlobalTag = \"A\";");
    game.write(
        "src/shared/alias.ts",
        &format!("{IMPORTS}{}", declare("{ name: GlobalString }", "toServer", "X.Alias")),
    );
    game.write(
        "src/shared/global.ts",
        &format!("{IMPORTS}{}", declare("{ nested: ScriptPayload }", "toServer", "X.Global")),
    );
    game.write(
        "src/shared/type.ts",
        "export interface Payload { count: number } export interface Tagged {Kind:GlobalTag;value:number}",
    );
    game.write(
        "src/shared/global-variant.ts",
        &format!(
            "{IMPORTS}import {{Tagged}} from \"./type\";{}",
            declare("Tagged|{Kind:\"B\"}", "toServer", "X.GlobalVariant")
        ),
    );
    game.write(
        "src/shared/merged.ts",
        &format!("{IMPORTS}import {{ Payload }} from \"./type\"; {}", declare("Payload", "toServer", "X.Merge")),
    );
    game.write(
        "src/augment.ts",
        "import \"./shared/type\"; declare module \"./shared/type\" { interface Payload { extra: string } }",
    );
    game.write("src/global-augment.ts", "export {}; declare global { interface GlobalPayload { flag: boolean } }");
    game.write(
        "src/shared/declared-global.ts",
        &format!("{IMPORTS}{}", declare("GlobalPayload", "toServer", "X.DeclaredGlobal")),
    );
    game.write("src/shared/ns.ts", "export namespace Doors { export interface State { open: boolean } }");
    game.write(
        "src/shared/ns-message.ts",
        &format!("{IMPORTS}import {{ Doors }} from \"./ns\"; {}", declare("Doors.State", "toServer", "X.Namespaced")),
    );
    game.write("src/shorthand.ts", "export {}; declare module \"untyped-lib\";");
    game.write(
        "src/ns-augment.ts",
        "import \"./shared/ns\"; declare module \"./shared/ns\" { namespace Doors { interface State { extra: string } } }",
    );
    let (reports, type_errors) = check(&game);
    assert_eq!(type_errors, ["src/shorthand.ts:1"], "a shorthand module declaration in a module is an augmentation");
    let mut duplicates: Vec<(String, String)> =
        reports.iter().filter(|r| r.code == 90018).map(|r| (r.file.clone(), r.message.clone())).collect();
    duplicates.sort();
    assert_eq!(
        duplicates,
        [
            ("src/shared/a.ts".to_owned(), "CRU018: X.Test is already declared at src/shared/b.ts:2.".to_owned()),
            ("src/shared/b.ts".to_owned(), "CRU018: X.Test is already declared at src/shared/a.ts:2.".to_owned()),
        ]
    );
    assert_eq!(
        files_with(&reports, 90010),
        [
            "src/augment.ts",
            "src/global-augment.ts",
            "src/ns-augment.ts",
            "src/shared/alias.ts",
            "src/shared/declared-global.ts",
            "src/shared/global-variant.ts",
            "src/shared/global.ts",
            "src/shared/merged.ts",
            "src/shared/ns-message.ts",
        ]
    );
}

fn written(files: &[(&str, String)]) -> Game {
    let game = fresh();
    for (path, text) in files {
        game.write(path, text);
    }
    game
}

#[test]
fn augmentations_that_add_nothing_a_payload_reaches_compile() {
    let game = written(&[
        ("src/shared/model.ts", "export interface Other { x: number }".into()),
        ("src/shared/messages.ts", format!("{IMPORTS}{}", declare("{ door: Model }", "toServer", "Doors.Toggle"))),
        (
            "src/util/workspace.ts",
            "export {}; declare global { interface Workspace { Lobby: Model } const Lobby: Model; }".into(),
        ),
        (
            "src/util/other.ts",
            "import \"../shared/model\"; declare module \"../shared/model\" { interface Other { door: Model } }".into(),
        ),
    ]);
    none(&reports_of(&game));
    game.compile();
}

fn augmentation(module: &str, body: &str) -> String {
    format!("import \"{module}\"; declare module \"{module}\" {{ {body} }}")
}

fn added_base(module: &str, name: &str) -> String {
    format!(
        "import {{ Added }} from \"{module}\"; declare module \"{module}\" {{ interface {name} extends Added {{}} }}"
    )
}

fn augmented(game: &Game) -> Vec<String> {
    let files: BTreeSet<String> = reports_of(game)
        .into_iter()
        .filter(|r| r.code == 90010 && r.file.starts_with("src/util/"))
        .map(|r| r.file.trim_start_matches("src/util/").trim_end_matches(".ts").to_owned())
        .collect();
    files.into_iter().collect()
}

#[test]
fn augmentations_of_a_base_interface_a_reached_payload_extends_fail_at_the_augmentation() {
    let game = written(&[
        (
            "src/shared/base.ts",
            "export interface Base { base: string } export interface Root { root: string } export interface Gen<T> { value: T } export class KBase { k = 1; }".into(),
        ),
        (
            "src/shared/model.ts",
            "import { Base, Root, Gen, KBase } from \"./base\"; export interface Child extends Base { child: string } export interface Middle extends Root { middle: string } export interface Top extends Middle { top: string } export interface Generic extends Gen<string> { own: number } export interface Unreached extends Base { own: string } export class Klass extends KBase { own = 2; }".into(),
        ),
        (
            "src/shared/messages.ts",
            format!(
                "{IMPORTS}import {{ Child, Top, Generic, Klass }} from \"./model\"; export const Four = toServer<Klass>(\"X.Four\"); export const One = toServer<Child>(\"X.One\"); export const Two = toServer<Top>(\"X.Two\"); export const Three = toServer<Generic>(\"X.Three\");"
            ),
        ),
        ("src/util/base.ts", augmentation("../shared/base", "interface Base { extra: string }")),
        ("src/util/root.ts", augmentation("../shared/base", "interface Root { extra: string }")),
        ("src/util/gen.ts", augmentation("../shared/base", "interface Gen<T> { extra: T }")),
        ("src/util/kbase.ts", augmentation("../shared/base", "interface KBase { extra: string }")),
        ("src/util/middle.ts", augmentation("../shared/model", "interface Middle { extra: string }")),
        ("src/util/unreached.ts", augmentation("../shared/model", "interface Unreached { extra: string }")),
    ]);
    assert_eq!(augmented(&game), ["base", "gen", "kbase", "middle", "root"]);
}

/// TypeScript's own `Omit`. The compiler types constrain `Omit`'s keys to the type's keys, which these fixtures break.
const WITHOUT: &str = "export type Without<T, K extends string | number | symbol> = Pick<T, Exclude<keyof T, K>>;";

const QUERIED: [&str; 7] = ["KQuery", "KMix", "KAlias", "KInline", "KLeft", "KRight", "KOther"];

fn queried_game(classes: &str, util: impl Fn(&str) -> String, alias: &str) -> Game {
    let base = [
        classes.to_owned(),
        "export function mix<T extends new () => object>(base: T): T { return base; }".to_owned(),
        "export function both<A extends new () => object, B extends new () => object>(a: A, b: B): new () => InstanceType<A> & InstanceType<B> { return a as never; }".to_owned(),
        WITHOUT.to_owned(),
    ];
    let model = [
        format!("import {{ {}, mix, both, Without }} from \"./base\";", QUERIED.join(", ")),
        "export interface ViaQuery extends InstanceType<typeof KQuery> { own: string }".to_owned(),
        "export class ViaMix extends mix(KMix) { own = 1; }".to_owned(),
        "export class ViaBoth extends both(KLeft, KRight) { own = 1; }".to_owned(),
        alias.to_owned(),
        "export interface Unreached extends InstanceType<typeof KOther> { own: string }".to_owned(),
    ];
    let messages = [
        "import { KInline, Without } from \"./base\";",
        "import { ViaQuery, ViaMix, ViaBoth, ViaAlias } from \"./model\";",
        "export const One = toServer<ViaQuery>(\"X.One\");",
        "export const Two = toServer<ViaMix>(\"X.Two\");",
        "export const Three = toServer<ViaBoth>(\"X.Three\");",
        "export const Four = toServer<ViaAlias>(\"X.Four\");",
        "export const Five = toServer<Without<InstanceType<typeof KInline>, \"k\">>(\"X.Five\");",
    ];
    let game = written(&[
        ("src/shared/base.ts", base.join("\n")),
        ("src/shared/model.ts", model.join("\n")),
        ("src/shared/messages.ts", format!("{IMPORTS}{}", messages.join("\n"))),
    ]);
    for name in QUERIED {
        game.write(&format!("src/util/{}.ts", name.to_lowercase()), &util(name));
    }
    game
}

#[test]
fn augmentations_of_a_base_reached_through_a_typeof_query_or_a_call_expression_fail_at_the_augmentation() {
    let classes: Vec<String> = QUERIED.iter().map(|n| format!("export class {n} {{ k = 1; }}")).collect();
    let game = queried_game(
        &classes.join("\n"),
        |name| augmentation("../shared/base", &format!("interface {name} {{ extra: string }}")),
        "export type ViaAlias = Without<InstanceType<typeof KAlias>, \"k\">;",
    );
    assert_eq!(augmented(&game), ["kalias", "kinline", "kleft", "kmix", "kquery", "kright"]);
}

#[test]
fn augmentations_that_add_a_base_to_a_class_reached_through_a_typeof_query_or_a_call_expression_fail() {
    let mut classes = vec!["export interface Added { added: string }".to_owned()];
    classes.extend(QUERIED.iter().map(|n| format!("export class {n} {{}}")));
    let game = queried_game(
        &classes.join("\n"),
        |name| added_base("../shared/base", name),
        "export interface ViaAlias extends Without<InstanceType<typeof KAlias>, \"k\"> { own: string }",
    );
    assert_eq!(augmented(&game), ["kalias", "kinline", "kleft", "kmix", "kquery", "kright"]);
}

#[test]
fn augmentations_of_a_class_a_payload_reaches_only_through_a_function_return_or_a_flattened_type_fail() {
    let classes = [
        "KReturn",
        "KConst",
        "KDefault",
        "KPayload",
        "KMapped",
        "KMixin",
        "KField",
        "KRecord",
        "KRecordReturn",
        "KUnreached",
    ];
    let mut base: Vec<String> = classes.iter().map(|n| format!("export class {n} {{ k = 1; keep = 1; }}")).collect();
    base.extend([
        "export const make = () => new KReturn();".to_owned(),
        "export const ConstAlias = KConst;".to_owned(),
        "export const makePayload = () => new KPayload();".to_owned(),
        "export const makeMapped = () => new KMapped();".to_owned(),
        "export const makeField = () => new KField();".to_owned(),
        "export const makeRecord = () => new KRecordReturn();".to_owned(),
        "export const makeUnreached = () => new KUnreached();".to_owned(),
        "export function mixin<T extends new (...args: never[]) => object>(base: T): new () => Without<InstanceType<T>, 'k'> { return base as never; }".to_owned(),
        WITHOUT.to_owned(),
    ]);
    let model = [
        format!(
            "import {{ {}, make, ConstAlias, makePayload, makeMapped, makeField, makeRecord, makeUnreached, mixin, Without }} from \"./base\";",
            classes.join(", ")
        ),
        "export interface ViaReturn extends Without<ReturnType<typeof make>, 'k'> { own: string }".to_owned(),
        "export interface ViaConst extends Without<InstanceType<typeof ConstAlias>, 'k'> { own: string }".to_owned(),
        "export interface Holder<T = KDefault> { v: Without<T, 'k'> }".to_owned(),
        "export interface ViaDefault extends Holder { own: string }".to_owned(),
        "export type ViaPayload = Without<ReturnType<typeof makePayload>, 'k'>;".to_owned(),
        "export type Flat<T> = { [P in keyof T]: T[P] };".to_owned(),
        "export type ViaMapped = Flat<ReturnType<typeof makeMapped>>;".to_owned(),
        "export class ViaMixin extends mixin(KMixin) { own = 1; }".to_owned(),
        "export interface ViaField { inner: Without<ReturnType<typeof makeField>, 'k'> }".to_owned(),
        "export type ViaRecord = Record<keyof InstanceType<typeof KRecord>, string>;".to_owned(),
        "export type ViaRecordReturn = Record<keyof ReturnType<typeof makeRecord>, string>;".to_owned(),
        "export interface ViaUnreached extends Without<ReturnType<typeof makeUnreached>, 'k'> { own: string }"
            .to_owned(),
    ];
    let messages = [
        "import { ViaReturn, ViaConst, ViaDefault, ViaPayload, ViaMapped, ViaMixin, ViaField, ViaRecord, ViaRecordReturn } from \"./model\";",
        "export const A = toServer<ViaReturn>(\"X.A\");",
        "export const B = toServer<ViaConst>(\"X.B\");",
        "export const C = toServer<ViaDefault>(\"X.C\");",
        "export const D = toServer<ViaPayload>(\"X.D\");",
        "export const E = toServer<ViaMapped>(\"X.E\");",
        "export const F = toServer<ViaMixin>(\"X.F\");",
        "export const G = toServer<ViaField>(\"X.G\");",
        "export const H = toServer<ViaRecord>(\"X.H\");",
        "export const I = toServer<ViaRecordReturn>(\"X.I\");",
    ];
    let game = written(&[
        ("src/shared/base.ts", base.join("\n")),
        ("src/shared/model.ts", model.join("\n")),
        ("src/shared/messages.ts", format!("{IMPORTS}{}", messages.join("\n"))),
    ]);
    for name in classes {
        game.write(
            &format!("src/util/{}.ts", name.to_lowercase()),
            &augmentation("../shared/base", &format!("interface {name} {{ extra: string }}")),
        );
    }
    assert_eq!(
        augmented(&game),
        ["kconst", "kdefault", "kfield", "kmapped", "kmixin", "kpayload", "krecord", "krecordreturn", "kreturn"]
    );
}

#[test]
fn augmentations_of_any_base_of_a_multi_base_or_merged_interface_fail_at_the_augmentation() {
    let model = "import { A, B, C, D } from \"./base\"; export interface Multi extends A, B { own: string } export interface Merged extends C { first: string } export interface Merged extends D { second: string }";
    let messages = format!(
        "{IMPORTS}import {{ Multi, Merged }} from \"./model\"; export const One = toServer<Multi>(\"X.One\"); export const Two = toServer<Merged>(\"X.Two\");"
    );
    for (base, util) in [
        (
            "export interface A { a: string } export interface B { b: string } export interface C { c: string } export interface D { d: string }",
            &(|name: &str| augmentation("../shared/base", &format!("interface {name} {{ extra: string }}")))
                as &dyn Fn(&str) -> String,
        ),
        (
            "export interface Added { added: string } export interface A {} export interface B {} export interface C {} export interface D {}",
            &|name: &str| added_base("../shared/base", name),
        ),
    ] {
        let game = written(&[
            ("src/shared/base.ts", base.to_owned()),
            ("src/shared/model.ts", model.to_owned()),
            ("src/shared/messages.ts", messages.clone()),
        ]);
        for name in ["A", "B", "C", "D"] {
            game.write(&format!("src/util/{}.ts", name.to_lowercase()), &util(name));
        }
        assert_eq!(augmented(&game), ["a", "b", "c", "d"]);
    }
}

#[test]
fn augmentations_that_add_a_base_to_an_interface_a_payload_reaches_by_heritage_fail_at_the_augmentation() {
    let game = written(&[
        (
            "src/shared/base.ts",
            "export interface Added { added: string } export interface Base {} export interface Root {} export class KBase {}".into(),
        ),
        (
            "src/shared/model.ts",
            "import { Base, Root, KBase } from \"./base\"; export interface Child extends Base { child: string } export interface Middle extends Root { middle: string } export interface Top extends Middle { top: string } export class Klass extends KBase { own = 2; }".into(),
        ),
        (
            "src/shared/messages.ts",
            format!(
                "{IMPORTS}import {{ Child, Top, Klass }} from \"./model\"; export const One = toServer<Child>(\"X.One\"); export const Two = toServer<Top>(\"X.Two\"); export const Three = toServer<Klass>(\"X.Three\");"
            ),
        ),
        ("src/util/base.ts", added_base("../shared/base", "Base")),
        ("src/util/root.ts", added_base("../shared/base", "Root")),
        ("src/util/kbase.ts", added_base("../shared/base", "KBase")),
    ]);
    assert_eq!(augmented(&game), ["base", "kbase", "root"]);
}

#[test]
fn augmentations_of_a_type_in_a_heritage_cycle_terminate_at_the_cycle_type_error() {
    let game = written(&[
        (
            "src/shared/model.ts",
            "export interface A extends B { a: string } export interface B extends A { b: string } export class P extends Q { p = 1; } export class Q extends P { q = 1; }".into(),
        ),
        (
            "src/shared/messages.ts",
            format!(
                "{IMPORTS}import {{ A, P }} from \"./model\"; export const One = toServer<A>(\"X.One\"); export const Two = toServer<P>(\"X.Two\");"
            ),
        ),
        ("src/util/b.ts", augmentation("../shared/model", "interface B { extra: string }")),
        ("src/util/q.ts", augmentation("../shared/model", "interface Q { extra: string }")),
    ]);
    assert!(game.type_errors().contains(&"src/shared/model.ts:1".to_owned()), "{:?}", game.type_errors());
}

fn payload_game(util: &str) -> Game {
    written(&[
        (
            "src/shared/model.ts",
            "export interface Payload { value: string } export interface Other { value: string }".into(),
        ),
        (
            "src/shared/messages.ts",
            format!("{IMPORTS}import {{ Payload }} from \"./model\"; export const One = toServer<Payload>(\"X.One\");"),
        ),
        ("src/util/value.ts", augmentation("../shared/model", util)),
    ])
}

#[test]
fn augmentations_declaring_values_beside_a_reached_name_fail_like_any_other_declaration() {
    for (form, body, rejected) in [
        ("list", "const a: number, Payload: number;", true),
        ("exported", "export const Payload: number;", true),
        ("exported list", "export const a: number, Payload: number;", true),
        ("unrelated list", "const a: number, b: number;", false),
        ("function", "function Payload(): void;", true),
        ("const", "const Payload: number;", true),
        ("let", "let Payload: number;", true),
        ("var", "var Payload: number;", true),
        ("unreached", "const Other: number;", false),
    ] {
        let expected: Vec<&str> = if rejected { vec!["value"] } else { Vec::new() };
        assert_eq!(augmented(&payload_game(body)), expected, "{form}");
    }
}

#[test]
fn augmentations_of_a_same_named_symbol_or_an_only_implemented_interface_build_cleanly() {
    let game = written(&[
        ("src/shared/model.ts", "export interface Payload { value: string }".into()),
        ("src/shared/other.ts", "export interface Payload { other: number }".into()),
        (
            "src/shared/messages.ts",
            format!("{IMPORTS}import {{ Payload }} from \"./model\"; export const One = toServer<Payload>(\"X.One\");"),
        ),
        ("src/util/other.ts", augmentation("../shared/other", "interface Payload { extra: string }")),
    ]);
    assert_eq!(augmented(&game), Vec::<String>::new());
    game.compile();
    let game = written(&[
        ("src/shared/base.ts", "export interface Contract { name: string }".into()),
        (
            "src/shared/model.ts",
            "import { Contract } from \"./base\"; export class Impl implements Contract { name = \"\"; own = 1; }"
                .into(),
        ),
        (
            "src/shared/messages.ts",
            format!("{IMPORTS}import {{ Impl }} from \"./model\"; export const One = toServer<Impl>(\"X.One\");"),
        ),
        ("src/util/contract.ts", augmentation("../shared/base", "interface Contract { extra?: string }")),
    ]);
    assert_eq!(augmented(&game), Vec::<String>::new());
    game.compile();
}

#[test]
fn augmentations_behind_a_deep_heritage_chain_still_fail_at_the_augmentation() {
    const DEPTH: usize = 1000;
    let mut model: Vec<String> =
        (0..DEPTH).map(|i| format!("export interface L{i} extends L{} {{ f{i}: number }}", i + 1)).collect();
    model.push(format!("export interface L{DEPTH} {{ leaf: string }}"));
    let game = written(&[
        ("src/shared/model.ts", model.join("\n")),
        (
            "src/shared/messages.ts",
            format!("{IMPORTS}import {{ L0 }} from \"./model\"; export const One = toServer<L0>(\"X.One\");"),
        ),
        ("src/util/leaf.ts", augmentation("../shared/model", &format!("interface L{DEPTH} {{ extra: string }}"))),
    ]);
    assert_eq!(augmented(&game), ["leaf"]);
}

#[test]
fn augmentations_reach_a_payload_through_a_dotted_namespace_declaration() {
    let game = written(&[
        (
            "src/shared/model.ts",
            "export namespace A { export namespace B { export interface Payload { x: number } } export const marker = 1 }".into(),
        ),
        (
            "src/shared/messages.ts",
            format!("{IMPORTS}import {{ A }} from \"./model\"; export const One = toServer<A.B.Payload>(\"X.One\");"),
        ),
        ("src/util/dotted.ts", augmentation("../shared/model", "namespace A.B { interface Payload { extra: string } }")),
        ("src/util/unrelated.ts", augmentation("../shared/model", "namespace A.B { const unrelated: number }")),
    ]);
    assert_eq!(augmented(&game), ["dotted"]);
}

#[test]
fn the_augmentation_scan_reports_only_the_augmentation_and_no_diagnostic_of_other_declarations() {
    let game = written(&[
        ("src/global.d.ts", "interface ScriptPayload { value: string }\n".into()),
        ("src/shared/model.ts", "export interface Payload { value: string }".into()),
        (
            "src/shared/messages.ts",
            format!("{IMPORTS}import {{ Payload }} from \"./model\"; export const One = toServer<Payload>(\"X.One\");"),
        ),
        ("src/shared/bad.ts", format!("{IMPORTS}{}", declare("{ a: any; b: ScriptPayload }", "toServer", "X.Bad"))),
        ("src/augment-only.ts", augmentation("./shared/model", "interface Payload { extra: string }")),
    ]);
    let reports = reports_of(&game);
    let codes = |file: &str| -> Vec<i32> { reports.iter().filter(|r| r.file == file).map(|r| r.code).collect() };
    assert_eq!(codes("src/augment-only.ts"), [90010]);
    assert_eq!(codes("src/shared/messages.ts"), [90010], "the payload the augmentation merges into");
    let files: BTreeSet<&str> = reports.iter().map(|r| r.file.as_str()).collect();
    assert_eq!(files, BTreeSet::from(["src/augment-only.ts", "src/shared/bad.ts", "src/shared/messages.ts"]));
    let mut bad: Vec<i32> = reports.iter().filter(|r| r.file == "src/shared/bad.ts").map(|r| r.code).collect();
    bad.sort();
    assert_eq!(bad, [90010, 90019], "each of the bad declaration's rules once");
}

#[test]
fn side_entry_references_follow_barrels_namespace_aliases_type_imports_and_dynamic_imports() {
    let game = written(&[
        ("src/util/named.ts", "export { Network } from \"@trioforge/crucible/client\";".into()),
        ("src/util/star.ts", "export * from \"@trioforge/crucible/client\";".into()),
        ("src/server/named.ts", "import { Network } from \"../util/named\"; export const N = Network;".into()),
        ("src/server/ns.ts", "import * as util from \"../util/star\"; export const N = util.Network;".into()),
        (
            "src/shared/type.ts",
            "import type { Network } from \"@trioforge/crucible/client\"; export type T = typeof Network;".into(),
        ),
        ("src/server/dynamic.ts", "export const N = import(\"@trioforge/crucible/client\");".into()),
        ("src/server/import-type.ts", "export type N = typeof import(\"@trioforge/crucible/client\");".into()),
        ("src/client/good.ts", "import { Network as N } from \"@trioforge/crucible/client\"; export const M = N;".into()),
        (
            "src/server/good.ts",
            "import { Network, boot } from \"@trioforge/crucible/server\"; export const N = Network; export const B = boot;".into(),
        ),
        (
            "src/server/facade.ts",
            "import { Network } from \"@trioforge/crucible/server\"; export function facade() { return Network; }".into(),
        ),
        (
            "src/client/facade.ts",
            "import { facade } from \"../server/facade\"; export const send = facade().send;".into(),
        ),
    ]);
    let (reports, type_errors) = check(&game);
    assert_eq!(type_errors, Vec::<String>::new());
    let flagged = files_with(&reports, 90024);
    for file in [
        "src/util/named.ts",
        "src/util/star.ts",
        "src/server/named.ts",
        "src/server/ns.ts",
        "src/shared/type.ts",
        "src/server/dynamic.ts",
        "src/server/import-type.ts",
        "src/client/facade.ts",
    ] {
        assert!(
            flagged.contains(&file.to_owned()) || type_errors.iter().any(|e| e.starts_with(file)),
            "{file}: {flagged:?} {type_errors:?}"
        );
    }
    assert!(!reports.iter().any(|r| r.file.ends_with("good.ts")), "{flagged:?}");
}

#[test]
fn schema_boundaries_accept_the_limits_and_reject_overflow() {
    let branches = |n: usize| (0..n).map(|i| format!("\"L{i:03}\"")).collect::<Vec<_>>().join("|");
    let nested = |n: usize| format!("{}string{}", "{value:".repeat(n), "}".repeat(n));
    let broad = |n: usize| format!("{{{}}}", (0..n).map(|i| format!("f{i}:string")).collect::<Vec<_>>().join(";"));
    let game = fresh();
    game.write(
        "src/shared/valid.ts",
        &format!(
            "{IMPORTS}
export const Names=toServer<string>(\"{}\");
export const MaxTimeout=askServer<string,number>(\"X.Maximum\",{{timeout:120}});
export const Union=toServer<{}>(\"X.Union\");
export const Depth=toServer<{}>(\"X.Depth\");
export const Nodes=toServer<{}>(\"X.Nodes\");
export const Shapes=toServer<{{a:Map<string,u8>;b:ReadonlyMap<u16,string>;c:Map<u32,number>;d:Readonly<Record<string,number>>;e:ReadonlyArray<number>;f:readonly [string,number];g:Vector3;h:Color3;i:CFrame;j:BasePart|Model;k:false|3|\"z\";l:{{b:number}};m:{{b:number}}}}>(\"X.Shapes\");
export const Alias=toServer<Partial<Record<\"z\"|\"a\",number>>>(\"X.Alias\");
",
            "A".repeat(64),
            branches(256),
            nested(31),
            broad(4095)
        ),
    );
    none(&reports_of(&game));
    let luau = game.luau("shared/valid.luau");
    let output = calls(&luau, &MESSAGES);
    let shapes = canon(&output["X.Shapes"][1]);
    assert!(shapes.contains(&canon("n.instance([\"BasePart\", \"Model\"])")), "{shapes}");
    assert!(
        shapes.contains(&canon(
            "[\"g\", __crucible.n.vector3], [\"h\", __crucible.n.color3], [\"i\", __crucible.n.cframe]"
        )),
        "{shapes}"
    );
    assert!(
        shapes.contains(&canon(
            "n.union([__crucible.n.literal(false), __crucible.n.literal(3), __crucible.n.literal(\"z\")])"
        )),
        "{shapes}"
    );
    same(
        &output["X.Alias"][1],
        "__crucible.n.object([[\"a\", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])], [\"z\", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])]])",
        "X.Alias",
    );
    let game = written(&[
        ("src/shared/union.ts", format!("{IMPORTS}{}", declare(&branches(257), "toServer", "X.Union"))),
        ("src/shared/depth.ts", format!("{IMPORTS}{}", declare(&nested(32), "toServer", "X.Depth"))),
        ("src/shared/nodes.ts", format!("{IMPORTS}{}", declare(&broad(4096), "toServer", "X.Nodes"))),
        (
            "src/shared/fake-width.ts",
            format!(
                "{IMPORTS}declare const width: unique symbol; type Fake=number & {{[width]?:\"u8\"}};{}",
                declare("Fake", "toServer", "X.Fake")
            ),
        ),
    ]);
    let reports = reports_of(&game);
    for file in ["union.ts", "depth.ts", "nodes.ts", "fake-width.ts"] {
        assert!(reports.iter().any(|r| r.file.ends_with(file) && r.code == 90019), "{file}");
    }
}

#[test]
fn callback_parameter_types_survive_imported_payload_aliases() {
    let game = written(&[
        ("src/shared/model.ts", "export interface Payload { value: string }".into()),
        ("src/shared/messages.ts", format!("{IMPORTS}import {{Payload}} from \"./model\";{}", declaration("Payload"))),
        (
            "src/server/service.ts",
            format!(
                "{IMPORTS}import {{Message}} from \"../shared/messages\"; export class Service extends Subsystem {{ readonly r=receive(Message,(p,payload)=>{{print(p.Name,payload.value);}}); }}"
            ),
        ),
    ]);
    none(&reports_of(&game));
    game.compile();
}

#[test]
fn declaration_and_receiver_aliases_and_namespaces_keep_macro_symbol_identity() {
    let game = written(&[
        ("src/shared/macros.ts", "export {toServer as message, receive as claim} from \"@trioforge/crucible\";".into()),
        (
            "src/shared/messages.ts",
            "import {message} from \"./macros\"; import * as C from \"@trioforge/crucible\"; export const Named=message<string>(\"X.Named\"); export const Namespace=C.toServer<number>(\"X.Namespace\"); export const Bracket=C[\"toServer\"]<boolean>(\"X.Bracket\"); export const Paren=(C.toServer)<undefined>(\"X.Paren\");".into(),
        ),
        (
            "src/server/service.ts",
            "import {Subsystem as Base} from \"@trioforge/crucible\"; import {claim} from \"../shared/macros\"; import {Named} from \"../shared/messages\"; export class Service extends Base { readonly r=claim<typeof Named>(Named,(p,value)=>{print(p.Name,value);}); }".into(),
        ),
        (
            "src/client/caller.ts",
            "import {Network as N} from \"@trioforge/crucible/client\"; import {Named,Namespace} from \"../shared/messages\"; N.send(Named,\"literal\"); N.send(Namespace,19);".into(),
        ),
    ]);
    none(&reports_of(&game));
    let messages = game.luau("shared/messages.luau");
    has(&messages, "local Named = __crucible.toServer(\"X.Named\", __crucible.n.string)");
    has(&messages, "local Namespace = __crucible.toServer(\"X.Namespace\", __crucible.n.f64)");
    has(&messages, "local Bracket = __crucible.toServer(\"X.Bracket\", __crucible.n.bool)");
    has(&messages, "local Paren = __crucible.toServer(\"X.Paren\", __crucible.n[\"nil\"])");
    has(&game.luau("server/service.luau"), "self.r = __crucible.receive(self, \"r\", Named, function(p, value)");
}

#[test]
fn width_identity_survives_a_package_linked_from_outside_node_modules() {
    let game = fresh();
    game.write("src/shared/messages.ts", &format!("{IMPORTS}{}", declaration("{a:u8;b:u16;c:u32;d:f32;e:f64}")));
    let installed = game.dir.join("node_modules/@trioforge/crucible");
    let vendor = game.dir.join("vendor/runtime");
    std::fs::create_dir_all(vendor.parent().unwrap()).unwrap();
    std::fs::rename(&installed, &vendor).unwrap();
    #[cfg(windows)]
    let linked = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(installed.to_string_lossy().replace('/', "\\"))
        .arg(vendor.to_string_lossy().replace('/', "\\"))
        .output()
        .is_ok_and(|o| o.status.success());
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&vendor, &installed).is_ok();
    assert!(linked, "the package is linked");
    none(&reports_of(&game));
    let luau = game.luau("shared/messages.luau");
    same(
        &calls(&luau, &MESSAGES)["X.Test"][1],
        "__crucible.n.object([[\"a\", __crucible.n.u8], [\"b\", __crucible.n.u16], [\"c\", __crucible.n.u32], [\"d\", __crucible.n.f32], [\"e\", __crucible.n.f64]])",
        "X.Test",
    );
}

const STATE_IMPORTS: &str =
    "import { replicated, replicatedPerPlayer, toServer, u8, Replicated } from \"@trioforge/crucible\";\n";

fn state(t: &str, name: &str, macro_: &str, initial: &str) -> String {
    format!("export const Value = {macro_}<{t}>(\"{name}\", {initial});")
}

#[test]
fn replicated_calls_keep_their_initial_value_and_append_the_state_schema() {
    let game = data_game();
    game.write("src/shared/fd.ts", &fixture("state-data/fd.ts"));
    game.write(
        "src/shared/state.ts",
        &format!(
            "{STATE_IMPORTS}import type {{ OwnerData, PlayerData }} from \"./fd\";
type JobPhaseName = \"Waiting\" | \"Countdown\" | \"Intro\" | \"Running\" | \"Settling\" | \"Resetting\";
export const JobPhase = replicated<JobPhaseName | undefined>(\"Job.Phase\", undefined);
export const OwnerProfile = replicatedPerPlayer<OwnerData | undefined>(\"Data.Owner\", undefined);
export const Rewards = replicated<{{ coins: number }}>(\"Job.Rewards\", {{ coins: 3 }});
declare const data: PlayerData;
export const wider = {{ ...data, played: {{ savedSeconds: 0, sessionStartedAt: 0 }} }};
export const leak: (player: Player) => OwnerData | undefined = () => wider;
"
        ),
    );
    let facts = game.facts();
    assert_eq!(facts.diagnostics, []);
    let luau = game.luau("shared/state.luau");
    let found = calls(&luau, &DATA);
    let phases: Vec<String> =
        ["Countdown", "Intro", "Resetting", "Running", "Settling", "Waiting"].iter().map(|p| n::ls(p)).collect();
    let mut union = vec![n::nil()];
    union.extend(phases);
    assert_eq!(found["Job.Phase"][1], "nil");
    same(&found["Job.Phase"][2], &n::u(&union), "Job.Phase");
    same(&found["Job.Rewards"][2], &n::o(&[("coins", n::f())]), "Job.Rewards");
    assert_eq!(found["Data.Owner"][1], "nil");
    let owner = &declarations(&facts, &game, "src/shared/state.ts")["Data.Owner"].schemas[0];
    let reactive = verdicts("reactive-lower.out");
    let expected = reactive.iter().find(|v| v.name == "Data.Owner").and_then(Verdict::shape).unwrap();
    assert_eq!(shape(owner), expected);
    for hidden in ["purchaseIds", "redeemedCodes", "analytics"] {
        assert!(!found["Data.Owner"][2].contains(hidden), "{hidden}");
    }
    has(&luau, "__crucible.replicated(\"Job.Phase\", nil, __crucible.n.union(");
    has(&luau, "__crucible.replicatedPerPlayer(\"Data.Owner\", nil, __crucible.n.union(");
}

#[test]
fn replicated_state_lowers_the_reactive_probe_shapes_as_its_verdicts_record() {
    let reactive = verdicts("reactive-lower.out");
    let game = data_game();
    game.write("src/shared/shapes.ts", &fixture("state-data/shapes-positive.ts"));
    none(&reports_of(&game));
    let luau = game.luau("shared/shapes.luau");
    let found = calls(&luau, &["replicated"]);
    for i in 1..=17 {
        let verdict = reactive.iter().find(|v| v.name == format!("P.{i}")).unwrap();
        assert_eq!(verdict.lowers, Some(true));
        assert!(found.contains_key(&format!("P.P{i}")), "P.{i} lowers");
    }
    let value = |i: usize| found[&format!("P.P{i}")][2].clone();
    same(&value(13), &n::m(n::s(), n::f()), "P.13");
    same(&value(14), &n::m(n::s(), n::f()), "P.14");
    same(&value(15), &n::o(&[("a", n::f()), ("b", n::u(&[n::nil(), n::s()]))]), "P.15");
    same(&value(16), &n::o(&[("Game", n::f()), ("Music", n::f())]), "P.16");
    same(&value(17), &n::m(n::u(&[n::ls("Jobs"), n::ls("Money")]), n::f()), "P.17");
    same(&value(10), &n::o(&[("c", n::kind("color3")), ("cf", n::kind("cframe")), ("tag", n::s())]), "P.10");

    let game = data_game();
    game.write("src/shared/shapes.ts", &fixture("state-data/shapes-negative.ts"));
    game.write(
        "src/shared/merge-a.ts",
        &format!(
            "{STATE_IMPORTS}declare global {{ interface Merged {{ readonly a: number }} }} declare const merged: Merged; export const MergedDecl = replicated<Merged>(\"N.N30\", merged);"
        ),
    );
    game.write("src/shared/merge-b.ts", "declare global { interface Merged { readonly b: string } } export {};");
    let reports = reports_of(&game);
    let negatives: Vec<&Verdict> = reactive.iter().filter(|v| v.name.starts_with("N.")).collect();
    assert_eq!(negatives.len(), 30);
    assert!(negatives.iter().all(|v| v.lowers == Some(false)));
    let entries: Vec<(String, Vec<(String, String)>)> = negatives
        .iter()
        .map(|v| {
            let expected =
                if v.name == "N.30" { vec![("CRU010".to_owned(), "value".to_owned())] } else { v.diagnostics() };
            (v.name.replacen('.', ".N", 1), expected)
        })
        .collect();
    expect_rejections(&reports, &entries);
    let shapes = messages(&reports, "src/shared/shapes.ts");
    assert!(
        shapes.contains(
            &"CRU019: N.N17 value.by: replicated state cannot hold Instances; replicate a stable id".to_owned()
        )
    );
    assert!(shapes.contains(
        &"CRU019: N.N19 value: unknown cannot be stored or replicated state; give the value a declared type".to_owned()
    ));
}

#[test]
fn replicated_depth_stops_at_sync_value_depth_where_a_payload_stops_at_max_depth() {
    let nested = |n: usize| format!("{}string{}", "{v:".repeat(n), "}".repeat(n));
    let game = data_game();
    game.write(
        "src/shared/depth.ts",
        &format!(
            "{STATE_IMPORTS}export const Deep = replicated<{}>(\"X.Deep\", undefined as never);\nexport const Payload = toServer<{}>(\"X.Payload\");",
            nested(27),
            nested(31)
        ),
    );
    none(&reports_of(&game));
    let game = data_game();
    game.write(
        "src/shared/depth.ts",
        &format!("{STATE_IMPORTS}{}", state(&nested(28), "X.Deep", "replicated", "undefined as never")),
    );
    let found = messages(&reports_of(&game), "src/shared/depth.ts");
    assert!(
        found
            .iter()
            .any(|m| m.starts_with("CRU019: X.Deep value.v") && m.ends_with(": schema exceeds the depth limit of 28")),
        "{found:?}"
    );
}

#[test]
fn replicated_declarations_take_the_declaration_name_and_escape_rules() {
    let long = format!("X.{}", "a".repeat(63));
    let mut cases: Vec<(String, &str, String)> = vec![
        ("local const".into(), "CRU017", "const X = replicated<string>(\"X.State\", \"\");".into()),
        ("let".into(), "CRU017", "export let X = replicated<string>(\"X.State\", \"\");".into()),
        ("nested".into(), "CRU017", "export function make() { return replicated<string>(\"X.State\", \"\"); }".into()),
        (
            "not exported".into(),
            "CRU017",
            "const X = replicatedPerPlayer<string>(\"X.State\", \"\"); export { X };".into(),
        ),
        ("missing type".into(), "CRU017", "export const X = replicated(\"X.State\", undefined as never);".into()),
        ("two types".into(), "CRU017", "export const X = replicated<string, number>(\"X.State\", \"\");".into()),
        (
            "extra argument".into(),
            "CRU017",
            "export const X = replicated<string>(\"X.State\", \"\", 1 as never);".into(),
        ),
        (
            "name variable".into(),
            "CRU018",
            "const n = \"X.State\"; export const X = replicated<string>(n, \"\");".into(),
        ),
    ];
    for name in ["", "1.Bad", "A..B", "A-b", "Crucible", "Crucible.State", long.as_str()] {
        cases.push((
            format!("invalid name {name:?}"),
            "CRU018",
            format!("export const X = replicated<string>({name:?}, \"\");"),
        ));
    }
    let rest: Vec<(&str, &str, String)> = vec![
        ("macro value", "CRU023", "export const f = replicated;".into()),
        ("macro parameter", "CRU023", "function use(f: unknown) {} use(replicatedPerPlayer);".into()),
        (
            "namespace value",
            "CRU023",
            "import * as C from \"@trioforge/crucible\"; export const f = C.replicated;".into(),
        ),
        (
            "bracket value",
            "CRU023",
            "import * as C from \"@trioforge/crucible\"; export const f = C[\"replicatedPerPlayer\"];".into(),
        ),
        (
            "server state",
            "CRU019: X.State value",
            state("ServerOnlyLike", "X.State", "replicated", "undefined as never"),
        ),
        (
            "state width key",
            "CRU019: X.State value.key: state maps are keyed by string",
            state("ReadonlyMap<u8, string>", "X.State", "replicated", "undefined as never"),
        ),
        (
            "state number literal key",
            "CRU019: X.State value.key: state maps are keyed by string",
            state("ReadonlyMap<1 | 2, string>", "X.State", "replicated", "undefined as never"),
        ),
    ];
    cases.extend(rest.into_iter().map(|(n, c, s)| (n.to_owned(), c, s)));
    let game = data_game();
    let header = format!(
        "{STATE_IMPORTS}import type {{ ServerOnly }} from \"@trioforge/crucible\"; type ServerOnlyLike = {{ x: ServerOnly<number> }};\n"
    );
    for (i, (_, _, source)) in cases.iter().enumerate() {
        game.write(&format!("src/shared/n{i}.ts"), &format!("{header}{source}"));
    }
    for side in ["server", "client", "neutral"] {
        game.write(
            &format!("src/{side}/wrong.ts"),
            &format!("{STATE_IMPORTS}{}", state("string", &format!("X.{side}"), "replicated", "\"\"")),
        );
    }
    game.write("src/shared/a.ts", &format!("{STATE_IMPORTS}{}", state("string", "X.Twice", "replicated", "\"\"")));
    game.write(
        "src/shared/b.ts",
        &format!("{STATE_IMPORTS}{}", state("number", "X.Twice", "replicatedPerPlayer", "0")),
    );
    game.write("src/shared/message.ts", &format!("{STATE_IMPORTS}export const Twice = toServer<string>(\"X.Twice\");"));
    game.write(
        "src/shared/limit.ts",
        &format!("{STATE_IMPORTS}{}", state("string", &format!("X.{}", "a".repeat(62)), "replicated", "\"\"")),
    );
    let (reports, type_errors) = check(&game);
    let typed = ["two types", "extra argument"];
    for (i, (name, code, _)) in cases.iter().enumerate() {
        let file = format!("src/shared/n{i}.ts");
        if typed.contains(&name.as_str()) {
            assert!(type_errors.contains(&format!("{file}:3")), "{name} is a type error: {type_errors:?}");
            continue;
        }
        let found = messages(&reports, &file);
        assert!(found.iter().any(|m| m.starts_with(code)), "{name} reports {code}: {found:?}");
    }
    assert_eq!(type_errors.len(), typed.len(), "{type_errors:?}");
    for side in ["server", "client", "neutral"] {
        assert_eq!(
            messages(&reports, &format!("src/{side}/wrong.ts")),
            ["CRU017: Declare replicated state as an exported top-level const in a shared module."]
        );
    }
    assert_eq!(messages(&reports, "src/shared/a.ts"), ["CRU018: X.Twice is already declared at src/shared/b.ts:2."]);
    assert_eq!(messages(&reports, "src/shared/b.ts"), ["CRU018: X.Twice is already declared at src/shared/a.ts:2."]);
    assert_eq!(messages(&reports, "src/shared/message.ts"), Vec::<String>::new());
    assert_eq!(messages(&reports, "src/shared/limit.ts"), Vec::<String>::new());
}

#[test]
fn replicated_declarations_report_cru010_for_global_merged_and_augmented_types_they_reach() {
    let game = data_game();
    game.write("src/global.d.ts", "interface ScriptState { value: string }\n");
    game.write(
        "src/shared/global.ts",
        &format!("{STATE_IMPORTS}{}", state("{ nested: ScriptState }", "X.Global", "replicated", "undefined as never")),
    );
    game.write("src/shared/type.ts", "export interface Score { points: number }");
    game.write(
        "src/shared/merged.ts",
        &format!(
            "{STATE_IMPORTS}import {{ Score }} from \"./type\"; {}",
            state("Score", "X.Merged", "replicated", "undefined as never")
        ),
    );
    game.write(
        "src/augment.ts",
        "import \"./shared/type\"; declare module \"./shared/type\" { interface Score { extra: string } }",
    );
    game.write("src/shared/clean.ts", "export interface Clean { points: number }");
    game.write(
        "src/shared/untouched.ts",
        &format!(
            "{STATE_IMPORTS}import {{ Clean }} from \"./clean\"; {}",
            state("Clean", "X.Clean", "replicated", "undefined as never")
        ),
    );
    assert_eq!(
        files_with(&reports_of(&game), 90010),
        ["src/augment.ts", "src/shared/global.ts", "src/shared/merged.ts"]
    );
}

#[test]
fn a_replicated_initial_value_that_breaks_the_declared_type_is_a_type_error() {
    for macro_ in ["replicated", "replicatedPerPlayer"] {
        let game = data_game();
        game.write(
            "src/shared/state.ts",
            &format!("{STATE_IMPORTS}export const X = {macro_}<{{ a: number }}>(\"X.State\", {{ a: \"s\" }});"),
        );
        assert!(game.type_errors().iter().any(|e| e.starts_with("src/shared/state.ts:2")), "{macro_}");
    }
    let game = data_game();
    game.write(
        "src/shared/state.ts",
        &format!("{STATE_IMPORTS}export const X = replicatedPerPlayer<{{ a: number }}>(\"X.State\", {{ a: 1 }});"),
    );
    game.write("src/client/read.ts", "import { X } from \"../shared/state\"; export const a: number = X().a;");
    game.compile();
}

const SHARED: &str =
    "import { profileStore, serverOnly, replicatedPaths, toServer, replicated } from \"@trioforge/crucible\";\n";
const SERVER: &str =
    "import { dataStore, profileMessage, config, playerConfig } from \"@trioforge/crucible/server\";\n";

fn case(name: &str) -> String {
    fixture(&format!("state-data/cases/{name}"))
}

fn section(text: &str, title: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| !l.starts_with(title))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .map(|l| l.trim().to_owned())
        .collect()
}

fn segment(key: &str) -> String {
    if ["[]", "*"].contains(&key) || key.starts_with('\\') { format!("\\{key}") } else { key.to_owned() }
}

fn field_paths(node: &Schema, prefix: &[String], out: &mut Vec<Vec<String>>) {
    let add = |path: Vec<String>, out: &mut Vec<Vec<String>>| {
        if !out.contains(&path) {
            out.push(path);
        }
    };
    let with = |s: String| {
        let mut path = prefix.to_vec();
        path.push(s);
        path
    };
    match node {
        Schema::Object(fields) => {
            for field in fields {
                add(with(segment(&field.name)), out);
                field_paths(&field.value, &with(segment(&field.name)), out);
            }
        }
        Schema::Array(element) => field_paths(element, &with("[]".to_owned()), out),
        Schema::Map(_, value) => field_paths(value, &with("*".to_owned()), out),
        Schema::Union(branches) => branches.iter().for_each(|b| field_paths(b, prefix, out)),
        Schema::Variant { key, body, .. } => {
            add(with(segment(key)), out);
            field_paths(body, prefix, out);
        }
        _ => {}
    }
}

fn paths_of(node: &Schema) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    field_paths(node, &[], &mut paths);
    paths.sort_by(|a, b| {
        a.iter()
            .zip(b)
            .map(|(x, y)| x.as_bytes().cmp(y.as_bytes()))
            .find(|o| o.is_ne())
            .unwrap_or(a.len().cmp(&b.len()))
    });
    paths
}

fn show_path(path: &[impl AsRef<str>]) -> String {
    let parts: Vec<String> = path.iter().map(|s| serde_json::to_string(s.as_ref()).unwrap()).collect();
    format!("{{ {} }}", parts.join(", "))
}

fn server_only(declaration: &Declaration) -> Vec<Vec<String>> {
    match &declaration.tail {
        Tail::ServerOnly(paths) => paths.iter().map(|p| p.iter().map(|s| s.to_string()).collect()).collect(),
        _ => Vec::new(),
    }
}

fn local(name: &str) -> String {
    let name = match name.strip_prefix("N.") {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => format!("N.N{rest}"),
        _ => name.to_owned(),
    };
    name.replace("N.N26.ViewNameLong.Abcdefghijklmnopqrstuv", "N.N26.ViewNameLong.Abcdefghijklmnopqrstu")
}

#[test]
fn data_and_handle_cases_lower_or_reject_as_the_data_probe_records() {
    let probe = verdicts("data-lower.out");
    let text = fixture("state-data/data-lower.out");
    let game = data_game();
    game.write("src/shared/fd.ts", &fixture("state-data/fd.ts"));
    game.write("src/shared/data.ts", &case("data-positive-shared.ts"));
    game.write("src/server/data.ts", &case("data-positive-server.ts"));
    game.write("src/shared/handles.ts", &case("handles-positive.ts"));
    let facts = game.facts();
    assert_eq!(facts.diagnostics, []);
    let mut found = HashMap::new();
    for file in ["src/shared/data.ts", "src/server/data.ts", "src/shared/handles.ts"] {
        found.extend(declarations(&facts, &game, file));
    }
    let lowers: Vec<&Verdict> = probe.iter().filter(|v| v.lowers == Some(true)).collect();
    assert_eq!(lowers.len(), 25);
    for entry in lowers {
        let declaration = found.get(&entry.name).unwrap_or_else(|| panic!("{} lowers", entry.name));
        if let Some(expected) = entry.shape() {
            assert_eq!(shape(&declaration.schemas[0]), expected, "{}", entry.name);
        }
        if let Some(expected) = entry.line("persist ") {
            let [persist, replica] = &declaration.schemas[..] else { panic!("{} has a replica", entry.name) };
            let Schema::Object(fields) = replica else { panic!("{}'s replica is an object", entry.name) };
            let marks = server_only(declaration);
            let shown: Vec<String> = marks.iter().map(|p| show_path(p)).collect();
            let handles = format!(
                "persist {} nodes; serverOnly: [{}]; replica {} nodes; {} view declarations",
                count(persist),
                shown.join(", "),
                count(replica),
                1 + fields.len()
            );
            assert_eq!(handles, expected, "{}", entry.name);
            if let Some(inline) = entry.line("{") {
                assert_eq!(describe(persist, &marks, &[]), inline, "{}", entry.name);
            }
        }
    }
    let view = |name: &str| found[name].schemas[1].clone();
    assert_eq!(found["Probe.FdClientType"].schemas[0], view("FD.Players"));
    assert_eq!(found["Probe.HiddenClientType"].schemas[0], view("Game.Hidden"));
    let Schema::Object(fields) = view("FD.Players") else { panic!("FD.Players' view is an object") };
    let mut names = vec!["Crucible.Live.FD.Players".to_owned()];
    names.extend(fields.iter().map(|f| format!("Crucible.Data.FD.Players.{}", f.name)));
    let names: Vec<String> = names.iter().map(|n| format!("{n} ({})", n.len())).collect();
    assert_eq!(names, section(&text, "FD.Players view declarations (18):"));
    let players: Vec<String> = paths_of(&view("FD.Players")).iter().map(|p| show_path(p)).collect();
    assert_eq!(players, section(&text, "FD.Players replicated paths"));
    let added = |name: &str| -> Vec<Vec<String>> {
        let base = paths_of(&view("FD.Players"));
        paths_of(&view(name)).into_iter().filter(|p| !base.contains(p)).collect()
    };
    assert_eq!(added("FD.Players.Plus"), [vec!["mood".to_owned()]]);
    assert_eq!(added("FD.Players.Secret"), Vec::<Vec<String>>::new());
    assert_eq!(view("Game.AllHidden"), Schema::Object(Vec::new()));
    assert_eq!(paths_of(&view("Game.Hidden")), [vec!["coins"], vec!["mixed"], vec!["mixed", "shown"]]);
    let ab = vec!["n".to_owned(), "a.b".to_owned()];
    assert!(paths_of(&view("Game.Collide.Plus")).contains(&ab));
    assert!(!paths_of(&view("Game.Collide")).contains(&ab));
    let markers: Vec<String> = paths_of(&view("Game.Markers")).iter().map(|p| show_path(p)).collect();
    let expected = text.lines().find(|l| l.starts_with("ok  markers:")).unwrap();
    assert_eq!(
        format!("markers: a map value is \"*\", a literal \"*\" key is \"\\*\": {}", markers.join(" ")),
        expected[4..]
    );
    let luau = game.luau("shared/handles.luau");
    let emitted = &calls(&luau, &DATA)["FD.Players"];
    let line = text.lines().find(|l| l.starts_with("export const PlayerData = ")).unwrap();
    let recorded = arguments(line, line.find('(').unwrap());
    let canonical = |args: &[String]| -> Vec<String> { args.iter().map(|a| canon(a)).collect() };
    assert_eq!(canonical(&emitted[2..]), canonical(&recorded[2..]), "the emitted call equals the probe's");
    assert_eq!(emitted[0], recorded[0]);

    let game = data_game();
    game.write("src/shared/fd.ts", &fixture("state-data/fd.ts"));
    game.write("src/shared/data.ts", &case("data-negative-shared.ts"));
    game.write("src/server/data.ts", &case("data-negative-server.ts"));
    game.write("src/shared/handles.ts", &case("handles-negative.ts"));
    let sources = [
        ("src/shared/data.ts", case("data-negative-shared.ts")),
        ("src/server/data.ts", case("data-negative-server.ts")),
        ("src/shared/handles.ts", case("handles-negative.ts")),
    ];
    let (reports, blanked) = check_statements(&game);
    let rejected: Vec<&Verdict> = probe.iter().filter(|v| v.lowers == Some(false)).collect();
    assert_eq!(rejected.len(), 26);
    let mut typed = Vec::new();
    for entry in rejected {
        let name = local(&entry.name);
        let declared = sources.iter().find_map(|(file, text)| line_naming(text, &name).map(|line| (*file, line)));
        let (file, line) = declared.unwrap_or_else(|| panic!("{name} is declared"));
        if blanked.iter().any(|(f, start, end)| f == file && (*start..=*end).contains(&line)) {
            typed.push(entry.name.clone());
            continue;
        }
        let expected = entry.diagnostics();
        let mine = reported(&reports, &local(&entry.name));
        assert!(!mine.is_empty(), "{} is rejected", entry.name);
        for (code, at) in &mine {
            assert!(
                expected.iter().any(|(c, e)| c == code && normal_path(at).starts_with(&normal_path(e))),
                "{} {code} at {at} matches the probe's {expected:?}",
                entry.name
            );
        }
        for (code, at) in expected.iter().filter(|(c, _)| c != "CRU019" || expected.len() == 1) {
            assert!(
                mine.iter().any(|(c, m)| c == code && normal_path(m) == normal_path(at)),
                "{} reports {code} at {at}: {mine:?}",
                entry.name
            );
        }
    }
    assert_eq!(typed, ["N.7.ScalarRoot"], "dataStore<string> breaks dataStore's object constraint");
}

#[test]
fn data_declarations_take_the_declaration_rules_three_name_kinds_and_their_lowering_checks() {
    let long = |n: usize| "a".repeat(n);
    let cases: Vec<(String, &str, &str, String)> = vec![
        ("profileStore local const".into(), "src/shared", "CRU017", "const S = profileStore(\"X.Store\", { template: { a: 0 } });".into()),
        ("profileStore let".into(), "src/shared", "CRU017", "export let S = profileStore(\"X.Store\", { template: { a: 0 } });".into()),
        (
            "profileStore two types".into(),
            "src/shared",
            "CRU017",
            "export const S = profileStore<{ a: number }, 1>(\"X.Store\", { template: { a: 0 } });".into(),
        ),
        (
            "profileStore extra argument".into(),
            "src/shared",
            "CRU017",
            "export const S = profileStore(\"X.Store\", { template: { a: 0 } }, 1 as never);".into(),
        ),
        (
            "profileStore name too long".into(),
            "src/shared",
            "CRU018: Store names are 1 to 50 bytes",
            format!("export const S = profileStore(\"X.{}\", {{ template: {{ a: serverOnly(0) }} }});", long(49)),
        ),
        (
            "profileStore reserved name".into(),
            "src/shared",
            "CRU018",
            "export const S = profileStore(\"Crucible.Store\", { template: { a: 0 } });".into(),
        ),
        (
            "profileStore scalar root".into(),
            "src/shared",
            "CRU019: X.Store value: a store's value is a record",
            "export const S = profileStore<{ a: number } | number>(\"X.Store\", { template: 1 as never });".into(),
        ),
        ("profileStore macro value".into(), "src/shared", "CRU023", "export const f = profileStore;".into()),
        (
            "profileStore namespace value".into(),
            "src/shared",
            "CRU023",
            "import * as C from \"@trioforge/crucible\"; export const f = C[\"profileStore\"];".into(),
        ),
        (
            "profileStore variant reserved".into(),
            "src/shared",
            "CRU026: X.Store value.phase.update",
            "export const S = profileStore<{ phase: { kind: \"a\"; update: number } | { kind: \"b\" } }>(\"X.Store\", { template: { phase: { kind: \"b\" } } });".into(),
        ),
        (
            "profileStore map value reserved".into(),
            "src/shared",
            "CRU026: X.Store value.m.value.insert",
            "export const S = profileStore(\"X.Store\", { template: { m: {} as Record<string, { insert: number }> } });".into(),
        ),
        (
            "profileStore serverOnly in a union".into(),
            "src/shared",
            "CRU019: X.Store value.u",
            "import type { ServerOnly } from \"@trioforge/crucible\"; export const S = profileStore<{ u: ServerOnly<number> | string }>(\"X.Store\", { template: { u: \"\" } });".into(),
        ),
        ("dataStore missing type".into(), "src/server", "CRU017", "export const D = dataStore(\"X.Data\", { defaults: { a: 0 } });".into()),
        (
            "dataStore extra argument".into(),
            "src/server",
            "CRU017",
            "export const D = dataStore<{ a: number }>(\"X.Data\", { defaults: { a: 0 } }, 1 as never);".into(),
        ),
        (
            "dataStore name too long".into(),
            "src/server",
            "CRU018: Store names are 1 to 50 bytes",
            format!("export const D = dataStore<{{ a: number }}>(\"X.{}\", {{ defaults: {{ a: 0 }} }});", long(49)),
        ),
        (
            "dataStore nested".into(),
            "src/server",
            "CRU017",
            "export function make() { return dataStore<{ a: number }>(\"X.Data\", { defaults: { a: 0 } }); }".into(),
        ),
        ("dataStore macro value".into(), "src/server", "CRU023", "export const f = dataStore;".into()),
        ("profileMessage missing type".into(), "src/server", "CRU017", "export const P = profileMessage(\"X.Message\");".into()),
        (
            "profileMessage extra argument".into(),
            "src/server",
            "CRU017",
            "export const P = profileMessage<string>(\"X.Message\", 1 as never);".into(),
        ),
        (
            "profileMessage name too long".into(),
            "src/server",
            "CRU018: Profile message names are 1 to 64 bytes",
            format!("export const P = profileMessage<string>(\"X.{}\");", long(63)),
        ),
        (
            "profileMessage Vector3".into(),
            "src/server",
            "CRU019: X.Message value.at: Vector3 cannot be saved",
            "export const P = profileMessage<{ at: Vector3 }>(\"X.Message\");".into(),
        ),
        ("profileMessage macro value".into(), "src/server", "CRU023", "export const f = profileMessage;".into()),
        ("config missing type".into(), "src/server", "CRU017", "export const C = config(\"X.Config\", 1 as never);".into()),
        (
            "config name too long".into(),
            "src/server",
            "CRU018: Config names are 1 to 64 bytes",
            format!("export const C = config<number>(\"X.{}\", 0);", long(63)),
        ),
        (
            "config serverOnly".into(),
            "src/server",
            "CRU019: X.Config value.a: serverOnly marks a profileStore template field",
            "import type { ServerOnly } from \"@trioforge/crucible\"; export const C = config<{ a: ServerOnly<number> }>(\"X.Config\", undefined as never);".into(),
        ),
        (
            "config Color3".into(),
            "src/server",
            "CRU019: X.Config value: Color3 cannot be saved",
            "export const C = config<Color3>(\"X.Config\", undefined as never);".into(),
        ),
        ("playerConfig let".into(), "src/server", "CRU017", "export let C = playerConfig<number>(\"X.Config\", 0);".into()),
        (
            "playerConfig Instance".into(),
            "src/server",
            "CRU019: X.Config value: Instances cannot be saved",
            "export const C = playerConfig<Part>(\"X.Config\", undefined as never);".into(),
        ),
        ("playerConfig macro value".into(), "src/server", "CRU023", "export const f = playerConfig;".into()),
        ("config macro value".into(), "src/server", "CRU023", "export const f = config;".into()),
    ];
    let game = data_game();
    for (i, (_, dir, _, source)) in cases.iter().enumerate() {
        let header = if *dir == "src/shared" { SHARED } else { SERVER };
        game.write(&format!("{dir}/n{i}.ts"), &format!("{header}{source}"));
    }
    let views = |name: &str, max: usize| format!("{}: 0", "f".repeat(max - format!("Crucible.Data.{name}.").len()));
    for (path, text) in [
        (
            "src/server/profile.ts",
            format!("{SHARED}export const S = profileStore(\"X.Side\", {{ template: {{ a: 0 }} }});"),
        ),
        (
            "src/client/profile.ts",
            format!("{SHARED}export const S = profileStore(\"X.ClientSide\", {{ template: {{ a: 0 }} }});"),
        ),
        (
            "src/shared/store.ts",
            format!("{SHARED}export const S = profileStore(\"X.Twice\", {{ template: {{ a: 0 }} }});"),
        ),
        (
            "src/server/store.ts",
            format!("{SERVER}export const D = dataStore<{{ a: number }}>(\"X.Twice\", {{ defaults: {{ a: 0 }} }});"),
        ),
        (
            "src/server/messages.ts",
            format!(
                "{SERVER}export const A = profileMessage<string>(\"X.Msg\"); export const B = profileMessage<number>(\"X.Msg\");"
            ),
        ),
        (
            "src/server/configs.ts",
            format!(
                "{SERVER}export const A = config<number>(\"X.Cfg\", 0); export const B = playerConfig<number>(\"X.Cfg\", 0);"
            ),
        ),
        (
            "src/shared/namespaces.ts",
            format!(
                "{SHARED}export const S = profileStore(\"Game.Same\", {{ template: {{ a: 0 }} }}); export const R = replicated<number>(\"Game.Same\", 0); export const T = toServer<number>(\"Game.Same\");"
            ),
        ),
        (
            "src/server/namespaces.ts",
            format!(
                "{SERVER}export const P = profileMessage<number>(\"Game.Same\"); export const C = config<number>(\"Game.Same\", 0);"
            ),
        ),
        (
            "src/shared/limits.ts",
            format!(
                "{SHARED}export const S = profileStore(\"X.{}\", {{ template: {{ a: serverOnly(0) }} }}); export const V = profileStore(\"Game.Views\", {{ template: {{ {} }} }});",
                long(48),
                views("Game.Views", 64)
            ),
        ),
        (
            "src/server/limits.ts",
            format!(
                "{SERVER}export const P = profileMessage<string>(\"X.{}\"); export const C = config<number>(\"Y.{}\", 0); export const D = dataStore<{{ a: number }}>(\"Z.{}\", {{ defaults: {{ a: 0 }} }});",
                long(62),
                long(62),
                long(48)
            ),
        ),
        (
            "src/shared/view.ts",
            format!(
                "{SHARED}export const V = profileStore(\"Game.Viewz\", {{ template: {{ {} }} }});",
                views("Game.Viewz", 65)
            ),
        ),
        (
            "src/shared/longview.ts",
            format!("{SHARED}export const V = profileStore(\"A{}\", {{ template: {{ a: 0 }} }});", "1".repeat(49)),
        ),
        (
            "src/shared/wrong.ts",
            format!(
                "{SERVER}export const D = dataStore<{{ a: number }}>(\"X.SharedData\", {{ defaults: {{ a: 0 }} }});"
            ),
        ),
    ] {
        game.write(path, &text);
    }
    let (reports, type_errors) = check(&game);
    let typed = [
        "profileStore two types",
        "profileStore extra argument",
        "profileStore scalar root",
        "dataStore extra argument",
        "profileMessage extra argument",
    ];
    for (i, (name, dir, code, _)) in cases.iter().enumerate() {
        let file = format!("{dir}/n{i}.ts");
        if typed.contains(&name.as_str()) {
            assert!(
                type_errors.iter().any(|e| e.starts_with(&format!("{file}:"))),
                "{name} is a type error: {type_errors:?}"
            );
            continue;
        }
        let found = messages(&reports, &file);
        assert!(found.iter().any(|m| m.starts_with(code)), "{name} reports {code}: {found:?}");
    }
    assert_eq!(type_errors.len(), typed.len(), "{type_errors:?}");
    for side in ["server", "client"] {
        assert_eq!(
            messages(&reports, &format!("src/{side}/profile.ts")),
            ["CRU017: Declare profile stores as an exported top-level const in a shared module."]
        );
    }
    assert_eq!(
        messages(&reports, "src/shared/store.ts"),
        ["CRU018: X.Twice is already declared at src/server/store.ts:2."]
    );
    assert_eq!(
        messages(&reports, "src/server/store.ts"),
        ["CRU018: X.Twice is already declared at src/shared/store.ts:2."]
    );
    let twice = |name: &str, file: &str| vec![format!("CRU018: {name} is already declared at {file}:2."); 2];
    assert_eq!(messages(&reports, "src/server/messages.ts"), twice("X.Msg", "src/server/messages.ts"));
    assert_eq!(messages(&reports, "src/server/configs.ts"), twice("X.Cfg", "src/server/configs.ts"));
    for file in ["src/shared/namespaces.ts", "src/server/namespaces.ts", "src/shared/limits.ts", "src/server/limits.ts"]
    {
        assert_eq!(messages(&reports, file), Vec::<String>::new(), "{file}");
    }
    let field = "f".repeat(40);
    assert_eq!(
        messages(&reports, "src/shared/view.ts"),
        [format!(
            "CRU018: Game.Viewz value.{field}: its view name Crucible.Data.Game.Viewz.{field} is 65 bytes, over 64; this store allows field names of at most 39 bytes; rename the field, shorten the store name, or mark it serverOnly"
        )]
    );
    let store = format!("A{}", "1".repeat(49));
    assert_eq!(
        messages(&reports, "src/shared/longview.ts"),
        [format!(
            "CRU018: {store} value.a: its view name Crucible.Data.{store}.a is 66 bytes, over 64; this store allows field names of at most -1 bytes; rename the field, shorten the store name, or mark it serverOnly"
        )]
    );
    let wrong = messages(&reports, "src/shared/wrong.ts");
    assert!(
        wrong.contains(&"CRU017: Declare data stores as an exported top-level const in a server module.".to_owned())
    );
    assert!(wrong.iter().any(|m| m.starts_with("CRU024")), "{wrong:?}");
}

#[test]
fn cru027_checks_profile_store_calls_by_the_callee_type_in_every_side_through_barrels() {
    let store =
        format!("{SHARED}export const PlayerData = profileStore(\"FD.Players\", {{ template: {{ coins: 0 }} }});\n");
    let using = |from: &str| format!("import {{ PlayerData }} from \"{from}\";\ndeclare const player: Player;\n");
    let server = "is server-only; the client reads PlayerData().";
    let client = "reads the local player's profile; on the server call";
    let game = data_game();
    game.write("src/shared/template.ts", &store);
    game.write("src/shared/barrel.ts", "export { PlayerData } from \"./template\";");
    game.write(
        "src/server/good.ts",
        &format!(
            "{}PlayerData(player).coins.set(1); PlayerData.loaded(player); PlayerData.handoff(player); PlayerData.players();",
            using("../shared/template")
        ),
    );
    game.write(
        "src/client/good.ts",
        &format!("{}PlayerData().coins(); PlayerData.loaded();", using("../shared/template")),
    );
    game.write(
        "src/shared/good.ts",
        &format!(
            "{}import {{ replicatedPaths }} from \"@trioforge/crucible\"; export const paths = replicatedPaths(PlayerData); export const store = PlayerData;",
            using("./template")
        ),
    );
    let triggers = [
        ("client", "../shared/template", "PlayerData(player);", format!("CRU027: PlayerData(player) {server}")),
        ("client", "../shared/template", "PlayerData.handoff(player);", format!("CRU027: PlayerData.handoff {server}")),
        ("client", "../shared/template", "PlayerData.loaded(player);", format!("CRU027: PlayerData.loaded {server}")),
        ("client", "../shared/template", "PlayerData[\"save\"](player);", format!("CRU027: PlayerData.save {server}")),
        ("server", "../shared/template", "PlayerData();", format!("CRU027: PlayerData() {client} PlayerData(player).")),
        (
            "server",
            "../shared/template",
            "PlayerData.loaded();",
            format!("CRU027: PlayerData.loaded() {client} PlayerData.loaded(player)."),
        ),
        ("shared", "./template", "PlayerData(player);", format!("CRU027: PlayerData(player) {server}")),
        ("shared", "./template", "PlayerData.players();", format!("CRU027: PlayerData.players {server}")),
        ("shared", "./template", "PlayerData();", format!("CRU027: PlayerData() {client} PlayerData(player).")),
        ("client", "../shared/barrel", "PlayerData(player);", format!("CRU027: PlayerData(player) {server}")),
        ("client", "../shared/barrel", "PlayerData.inspect(1);", format!("CRU027: PlayerData.inspect {server}")),
        ("server", "../shared/barrel", "PlayerData();", format!("CRU027: PlayerData() {client} PlayerData(player).")),
        ("shared", "./barrel", "PlayerData(player);", format!("CRU027: PlayerData(player) {server}")),
        ("shared", "./barrel", "PlayerData();", format!("CRU027: PlayerData() {client} PlayerData(player).")),
        (
            "neutral",
            "../shared/template",
            "PlayerData();",
            format!("CRU027: PlayerData() {client} PlayerData(player)."),
        ),
        (
            "client",
            "../shared/template",
            "const alias = PlayerData; (alias)(player);",
            "CRU027: alias(player) is server-only; the client reads alias().".to_owned(),
        ),
    ];
    for (i, (side, from, code, _)) in triggers.iter().enumerate() {
        game.write(&format!("src/{side}/t{i}.ts"), &format!("{}{code}", using(from)));
    }
    let reports = reports_of(&game);
    for file in ["src/server/good.ts", "src/client/good.ts", "src/shared/good.ts", "src/shared/barrel.ts"] {
        assert_eq!(messages(&reports, file), Vec::<String>::new(), "{file}");
    }
    for (i, (side, _, code, message)) in triggers.iter().enumerate() {
        assert_eq!(messages(&reports, &format!("src/{side}/t{i}.ts")), std::slice::from_ref(message), "{code}");
    }
}

#[test]
fn cru025_rejects_a_process_receipt_assignment_in_a_program_that_holds_a_grant_claim() {
    let store =
        format!("{SHARED}export const PlayerData = profileStore(\"FD.Players\", {{ template: {{ coins: 0 }} }});\n");
    let claim = "import { Subsystem } from \"@trioforge/crucible\"; import { PlayerData } from \"../shared/template\";\nexport class Shop extends Subsystem { readonly skins = PlayerData.grant(1, (player, data) => data); }";
    let receipt =
        |code: &str| format!("declare const handler: (receipt: ReceiptInfo) => Enum.ProductPurchaseDecision;\n{code}");
    let triggers = [
        "game.GetService(\"MarketplaceService\").ProcessReceipt = handler;",
        "const market = game.GetService(\"MarketplaceService\"); market.ProcessReceipt = handler;",
        "const market = game.GetService(\"MarketplaceService\"); market[\"ProcessReceipt\"] = handler;",
        "const market = game.GetService(\"MarketplaceService\"); (market).ProcessReceipt = handler;",
        "const market = game.GetService(\"MarketplaceService\"); (market.ProcessReceipt) = handler;",
        "export function install(market: MarketplaceService) { market.ProcessReceipt = handler; }",
    ];
    let game = data_game();
    game.write("src/shared/template.ts", &store);
    game.write("src/server/shop.ts", claim);
    for (i, code) in triggers.iter().enumerate() {
        game.write(&format!("src/server/r{i}.ts"), &receipt(code));
    }
    game.write(
        "src/server/other.ts",
        &receipt("const other = { ProcessReceipt: handler }; other.ProcessReceipt = handler;"),
    );
    let reports = reports_of(&game);
    for (i, code) in triggers.iter().enumerate() {
        assert_eq!(
            messages(&reports, &format!("src/server/r{i}.ts")),
            [
                "CRU025: Crucible's data module owns ProcessReceipt; register grants with a profile store's grant(productId, ...)."
            ],
            "{code}"
        );
    }
    assert_eq!(messages(&reports, "src/server/other.ts"), Vec::<String>::new());
    assert_eq!(messages(&reports, "src/server/shop.ts"), Vec::<String>::new());
    let game = data_game();
    game.write("src/shared/template.ts", &store);
    game.write(
        "src/server/shop.ts",
        "import { PlayerData } from \"../shared/template\"; declare const player: Player; PlayerData(player).coins.set(1);",
    );
    game.write("src/server/r.ts", &receipt(triggers[0]));
    none(&reports_of(&game));
    game.compile();
}

#[test]
fn configs_lower_to_their_emit_calls_and_a_client_or_shared_read_is_cru024() {
    let declarations = format!(
        "{SERVER}export interface GlobalEvent {{ readonly name: string; readonly multiplier: number }}\nexport const NO_EVENT: GlobalEvent = {{ name: \"\", multiplier: 1 }};\nexport const GlobalEventConfig = config<GlobalEvent>(\"GlobalEvent\", NO_EVENT);\nexport const Discount = playerConfig<number>(\"Shop.WelcomeDiscount\", 0);\n"
    );
    let read = |from: &str| {
        format!("import {{ GlobalEventConfig, Discount }} from \"{from}\";\ndeclare const player: Player;\n")
    };
    let game = data_game();
    game.write("src/server/configs.ts", &declarations);
    game.write(
        "src/server/use.ts",
        &format!("{}export const event = GlobalEventConfig(); export const d = Discount(player);", read("./configs")),
    );
    game.write("src/util/barrel.ts", "export { GlobalEventConfig, Discount } from \"../server/configs\";");
    game.write(
        "src/client/global.ts",
        &format!("{}export const event = GlobalEventConfig().name;", read("../server/configs")),
    );
    game.write("src/client/player.ts", &format!("{}export const d = Discount(player);", read("../server/configs")));
    game.write(
        "src/shared/global.ts",
        &format!("{}export const event = GlobalEventConfig();", read("../server/configs")),
    );
    game.write("src/shared/player.ts", &format!("{}export const d = Discount(player);", read("../server/configs")));
    game.write("src/client/barrel.ts", &format!("{}export const event = GlobalEventConfig();", read("../util/barrel")));
    let reports = reports_of(&game);
    assert_eq!(messages(&reports, "src/server/configs.ts"), Vec::<String>::new());
    assert_eq!(messages(&reports, "src/server/use.ts"), Vec::<String>::new());
    for file in ["client/global.ts", "client/player.ts", "shared/global.ts", "shared/player.ts", "client/barrel.ts"] {
        let found = messages(&reports, &format!("src/{file}"));
        assert!(found.iter().any(|m| m.starts_with("CRU024")), "{file}: {found:?}");
    }
    let game = data_game();
    game.write("src/server/configs.ts", &declarations);
    game.write(
        "src/server/use.ts",
        &format!("{}export const event = GlobalEventConfig(); export const d = Discount(player);", read("./configs")),
    );
    none(&reports_of(&game));
    let luau = game.luau("server/configs.luau");
    let found = calls(&luau, &DATA);
    assert_eq!(found["GlobalEvent"][1], "NO_EVENT");
    same(&found["GlobalEvent"][2], &n::o(&[("multiplier", n::f()), ("name", n::s())]), "GlobalEvent");
    assert_eq!(found["Shop.WelcomeDiscount"][1..], ["0", "__crucible.n.f64"]);
}

#[test]
fn a_template_defaults_or_fallback_that_breaks_its_type_is_a_type_error() {
    for (file, text) in [
        (
            "src/shared/store.ts",
            format!(
                "{SHARED}export const S = profileStore<{{ a: number }}>(\"X.Store\", {{ template: {{ a: \"s\" }} }});"
            ),
        ),
        (
            "src/server/store.ts",
            format!("{SERVER}export const D = dataStore<{{ a: number }}>(\"X.Data\", {{ defaults: {{ a: \"s\" }} }});"),
        ),
        ("src/server/config.ts", format!("{SERVER}export const C = config<number>(\"X.Config\", \"s\");")),
        (
            "src/server/player.ts",
            format!("{SERVER}export const C = playerConfig<{{ a: number }}>(\"X.Config\", {{ a: true }});"),
        ),
    ] {
        let game = data_game();
        game.write(file, &text);
        assert!(game.type_errors().iter().any(|e| e.starts_with(&format!("{file}:2"))), "{file}");
    }
    let game = data_game();
    game.write(
        "src/shared/store.ts",
        &format!("{SHARED}export const S = profileStore<{{ a: number }}>(\"X.Store\", {{ template: {{ a: 1 }} }});\nexport const Inferred = profileStore(\"X.Inferred\", {{ template: {{ a: 1, secret: serverOnly(\"\") }} }});"),
    );
    game.write(
        "src/server/use.ts",
        "import { S, Inferred } from \"../shared/store\"; declare const player: Player; export const a: number = S(player).a(); export const secret: string = Inferred(player).secret();",
    );
    none(&reports_of(&game));
    game.compile();
}

const HIDE: &str = "import type { ServerOnly } from \"@trioforge/crucible\";\n";

fn store(name: &str, t: &str) -> String {
    format!("export const S{name} = profileStore<{t}>(\"X.{name}\", {{ template: undefined as never }});\n")
}

fn tail(found: &HashMap<String, Vec<String>>, name: &str) -> Vec<String> {
    found[name][2..].iter().map(|a| canon(a)).collect()
}

fn expected(parts: &[String]) -> Vec<String> {
    parts.iter().map(|p| canon(p)).collect()
}

#[test]
fn persist_rejections_name_the_annotation_to_write_and_server_only_marks_follow_optional_fields() {
    let game = data_game();
    game.write(
        "src/shared/good.ts",
        &format!(
            "{SHARED}{HIDE}export const Opt = profileStore<{{ a: number; b?: ServerOnly<number> }}>(\"X.Opt\", {{ template: {{ a: 0 }} }});\nexport const Reserved = replicated<{{ set: number; update: string }}>(\"X.Reserved\", {{ set: 1, update: \"\" }});\nexport const Tagged = replicated<{{ by: string }} & ({{ kind: \"a\"; x: number }} | {{ kind: \"b\" }})>(\"X.Tagged\", {{ by: \"\", kind: \"b\" }});"
        ),
    );
    game.write(
        "src/server/good.ts",
        &format!("{SERVER}export const Records = dataStore<{{ insert: number; remove: string }}>(\"X.Records\", {{ defaults: {{ insert: 0, remove: \"\" }} }});"),
    );
    none(&reports_of(&game));
    let found = calls(&game.luau("shared/good.luau"), &DATA);
    assert_eq!(
        tail(&found, "X.Opt"),
        expected(&[
            n::o(&[("a", n::f()), ("b", n::u(&[n::nil(), n::f()]))]),
            n::o(&[("a", n::f())]),
            "[[\"b\"]]".into()
        ])
    );
    same(&found["X.Reserved"][2], &n::o(&[("set", n::f()), ("update", n::s())]), "X.Reserved");
    same(
        &found["X.Tagged"][2],
        &n::u(&[n::v("kind", "a", n::o(&[("by", n::s()), ("x", n::f())])), n::v("kind", "b", n::o(&[("by", n::s())]))]),
        "X.Tagged",
    );
    let records = calls(&game.luau("server/good.luau"), &DATA);
    same(&records["X.Records"][2], &n::o(&[("insert", n::f()), ("remove", n::s())]), "X.Records");
    let named = |name: &str, template: &str| {
        format!("export const S{name} = profileStore(\"X.{name}\", {{ template: {template} }});\n")
    };
    let game = data_game();
    game.write(
        "src/shared/bad.ts",
        &format!(
            "{SHARED}{HIDE}{}{}{}{}{}{}export const SState = replicated<{{ x: ServerOnly<number> }}>(\"X.State\", undefined as never);",
            named("Empty", "{ quests: [] }"),
            named("Record", "{ upgrades: {} }"),
            named("Undefined", "{ note: undefined }"),
            named("Reserved", "{ coins: 0, update: 0 } as { coins: number; update: number }"),
            named("Element", "{ jobs: [] as ServerOnly<number>[] }"),
            named("Twice", "{ a: serverOnly({ b: serverOnly(0) }) }"),
        ),
    );
    game.write(
        "src/server/bad.ts",
        &format!(
            "{HIDE}{SERVER}export const D = dataStore<{{ key: ServerOnly<string> }}>(\"X.Data\", {{ defaults: undefined as never }});\nexport const U = dataStore<unknown>(\"X.Unknown\", {{ defaults: undefined as never }});"
        ),
    );
    let (reports, blanked) = check_statements(&game);
    assert_eq!(
        blanked,
        [("src/server/bad.ts".to_owned(), 4, 4)],
        "dataStore<unknown> breaks dataStore's object constraint"
    );
    assert_eq!(
        messages(&reports, "src/shared/bad.ts"),
        [
            "CRU019: X.Empty value.quests[]: an empty array has no element type; annotate it: [] as string[]",
            "CRU019: X.Record value.upgrades: an empty object type accepts anything; annotate an empty template table: {} as Record<string, number>",
            "CRU019: X.Undefined value.note: a position that is always undefined; annotate it: undefined as string | undefined",
            "CRU026: X.Reserved value.update: `update` is a handle method name (set, update, insert, remove); rename the field",
            "CRU019: X.Element value.jobs[]: serverOnly marks a field on an object path from the template root, never an array element, a map value or a union member",
            "CRU019: X.Twice value.a.b: this field is already inside a serverOnly field",
            "CRU019: X.State value.x: serverOnly marks a profileStore template field; it means nothing here",
        ]
    );
    assert_eq!(
        messages(&reports, "src/server/bad.ts"),
        ["CRU019: X.Data value.key: serverOnly marks a profileStore template field; it means nothing here"]
    );
}

const ROOT_MARK: &str =
    "serverOnly marks a template field, never the store's whole value; wrap the value in an object and mark the field";
const FIELD_MARK: &str = "serverOnly marks a field on an object path from the template root, never an array element, a map value or a union member";

#[test]
fn rule_129_rejects_a_server_only_mark_on_the_store_s_whole_value_in_every_spelling() {
    let roots = [
        ("Type", store("Type", "ServerOnly<{ secret: number }>")),
        ("Call", "export const SCall = profileStore(\"X.Call\", { template: serverOnly({ secret: 1 }) });\n".to_owned()),
        (
            "CallTyped",
            "export const SCallTyped = profileStore<ServerOnly<{ secret: number }>>(\"X.CallTyped\", { template: serverOnly({ secret: 1 }) });\n".to_owned(),
        ),
        (
            "Const",
            "const tmpl = serverOnly({ secret: 1 });\nexport const SConst = profileStore(\"X.Const\", { template: tmpl });\n".to_owned(),
        ),
        ("Alias", format!("type Hidden = ServerOnly<{{ secret: number }}>;\ntype Root = Hidden;\n{}", store("Alias", "Root"))),
        ("Generic", format!("type Mark<T> = ServerOnly<T>;\n{}", store("Generic", "Mark<{ secret: number }>"))),
        ("EmptyIntersection", store("EmptyIntersection", "ServerOnly<{ secret: number }> & {}")),
        ("Scalar", store("Scalar", "ServerOnly<number>")),
        ("RecordInside", store("RecordInside", "ServerOnly<Record<string, number>>")),
        ("MapInside", store("MapInside", "ServerOnly<Map<string, number>>")),
        ("Optional", store("Optional", "ServerOnly<{ secret: number }> | undefined")),
    ];
    let game = data_game();
    let sources: Vec<&str> = roots.iter().map(|(_, s)| s.as_str()).collect();
    let source = format!("{SHARED}{HIDE}{}", sources.concat());
    game.write("src/shared/root.ts", &source);
    let (reports, blanked) = check_statements(&game);
    let optional = line_naming(&source, "X.Optional").unwrap();
    let starts: Vec<(&str, usize)> = blanked.iter().map(|(file, start, _)| (file.as_str(), *start)).collect();
    assert_eq!(starts, [("src/shared/root.ts", optional)], "an optional value breaks profileStore's object constraint");
    let found = messages(&reports, "src/shared/root.ts");
    for (name, _) in roots.iter().filter(|(name, _)| *name != "Optional") {
        assert!(found.contains(&format!("CRU019: X.{name} value: {ROOT_MARK}")), "{name}: {found:?}");
    }
    assert_eq!(
        found.iter().filter(|m| m.contains(ROOT_MARK)).count(),
        roots.len() - 1,
        "one root report per declaration"
    );
}

#[test]
fn rule_129_keeps_every_server_only_value_that_survives_a_type_spelling_out_of_the_replica() {
    let rejected = [
        ("RootIntersection", store("RootIntersection", "ServerOnly<{ secret: number }> & { a: number }"), "value"),
        ("RootIntersections", store("RootIntersections", "ServerOnly<{ s: number }> & ServerOnly<{ t: number }>"), "value"),
        ("RootExtends", format!("interface Mine extends ServerOnly<{{ s: number }}> {{ b: number }}\n{}", store("RootExtends", "Mine")), "value"),
        ("RootPartial", store("RootPartial", "Partial<ServerOnly<{ s: number }>>"), "value"),
        (
            "RootSpread",
            "export const SRootSpread = profileStore(\"X.RootSpread\", { template: { ...serverOnly({ b: 1 }) } });\n".to_owned(),
            "value",
        ),
        ("FieldIntersection", store("FieldIntersection", "{ a: number; b: ServerOnly<{ x: number }> & { y: number } }"), "value.b"),
        (
            "FieldIntersections",
            store("FieldIntersections", "{ a: number; b: ServerOnly<{ x: number }> & ServerOnly<{ y: number }> }"),
            "value.b",
        ),
        (
            "FieldExtends",
            format!("interface Hides extends ServerOnly<{{ s: number }}> {{ b: number }}\n{}", store("FieldExtends", "{ a: number; m: Hides }")),
            "value.m",
        ),
        ("FieldOmit", store("FieldOmit", "{ a: number; o: Omit<ServerOnly<{ s: number }>, never> }"), "value.o"),
        (
            "FieldSpread",
            "export const SFieldSpread = profileStore(\"X.FieldSpread\", { template: { a: 1, ...serverOnly({ b: 1 }) } });\n".to_owned(),
            "value",
        ),
    ];
    let game = data_game();
    let sources: Vec<&str> = rejected.iter().map(|(_, s, _)| s.as_str()).collect();
    game.write("src/shared/survive.ts", &format!("{SHARED}{HIDE}{}", sources.concat()));
    let found = messages(&reports_of(&game), "src/shared/survive.ts");
    for (name, _, at) in &rejected {
        let message = format!("CRU019: X.{name} {at}: symbol property names cannot cross the network");
        assert!(found.contains(&message), "{name} rejects the brand's symbol key at {at}: {found:?}");
    }
}

#[test]
fn rule_129_rejects_a_server_only_mark_inside_an_array_map_tuple_union_or_optional_object() {
    let positions: [(&str, &str, &str, &str, &str); 12] = [
        ("Element", "{ a: number; b: ServerOnly<number>[] }", "value.b[]", FIELD_MARK, ""),
        ("ElementObject", "{ a: number; b: { s: ServerOnly<number> }[] }", "value.b[].s", FIELD_MARK, ""),
        ("MapValue", "{ a: number; b: Map<string, ServerOnly<number>> }", "value.b.value", FIELD_MARK, ""),
        (
            "MapObjectValue",
            "{ a: number; b: Map<string, { s: ServerOnly<number> }> }",
            "value.b.value.s",
            FIELD_MARK,
            "",
        ),
        ("RecordValue", "{ a: number; b: Record<string, ServerOnly<number>> }", "value.b.value", FIELD_MARK, ""),
        ("Tuple", "{ a: number; b: [ServerOnly<number>, string] }", "value.b[0]", FIELD_MARK, ""),
        ("UnionLiteral", "{ a: number; u: ServerOnly<number> | \"z\" }", "value.u", FIELD_MARK, ""),
        (
            "UnionVariant",
            "{ a: number; u: { k: \"p\"; s: ServerOnly<number> } | { k: \"q\" } }",
            "value.u.s",
            FIELD_MARK,
            "",
        ),
        ("OptionalParent", "{ a: number; p?: { s: ServerOnly<number> } }", "value.p.s", FIELD_MARK, ""),
        (
            "UnionObjects",
            "{ a: number; u: ServerOnly<{ x: number }> | { y: number } }",
            "value.u",
            "union members ServerOnly<{ x: number; }>, { y: number; } have no discriminant; give each a distinct literal property",
            "",
        ),
        (
            "UnionMarks",
            "{ a: number; u: ServerOnly<number> | ServerOnly<string> | undefined }",
            "value.u",
            "union members ServerOnly<string>, ServerOnly<number> have no discriminant; give each a distinct literal property",
            "",
        ),
        (
            "NestedAliasElement",
            "{ a: number; l: Hidden[] }",
            "value.l[]",
            FIELD_MARK,
            "type Hidden = ServerOnly<{ v: number }>;\n",
        ),
    ];
    let aliases: String = positions.iter().map(|p| p.4).collect();
    let stores: String = positions.iter().map(|(name, t, ..)| store(name, t)).collect();
    let game = data_game();
    game.write("src/shared/positions.ts", &format!("{SHARED}{HIDE}{aliases}{stores}"));
    let expected: Vec<String> =
        positions.iter().map(|(name, _, at, message, _)| format!("CRU019: X.{name} {at}: {message}")).collect();
    assert_eq!(messages(&reports_of(&game), "src/shared/positions.ts"), expected);
}

#[test]
fn rule_129_marks_a_field_at_any_object_depth_through_aliases_optionals_and_partials_and_prunes_what_empties() {
    let game = data_game();
    game.write(
        "src/shared/marks.ts",
        &format!(
            "{SHARED}{HIDE}type Hidden = ServerOnly<{{ v: number }}>;\n{}{}{}",
            store("Marks", "{ a: number; c: { w: Hidden; z: string }; h: Hidden; n: Partial<{ b: ServerOnly<number> }>; o?: ServerOnly<number> }"),
            store("Only", "{ only: ServerOnly<number> }"),
            store("Deep", "{ w: { only: ServerOnly<number> } }"),
        ),
    );
    none(&reports_of(&game));
    let found = calls(&game.luau("shared/marks.luau"), &DATA);
    assert_eq!(
        tail(&found, "X.Marks"),
        expected(&[
            n::o(&[
                ("a", n::f()),
                ("c", n::o(&[("w", n::o(&[("v", n::f())])), ("z", n::s())])),
                ("h", n::o(&[("v", n::f())])),
                ("n", n::o(&[("b", n::u(&[n::nil(), n::f()]))])),
                ("o", n::u(&[n::nil(), n::f()])),
            ]),
            n::o(&[("a", n::f()), ("c", n::o(&[("z", n::s())]))]),
            "[[\"c\", \"w\"], [\"h\"], [\"n\", \"b\"], [\"o\"]]".into(),
        ])
    );
    assert_eq!(tail(&found, "X.Only"), expected(&[n::o(&[("only", n::f())]), n::o(&[]), "[[\"only\"]]".into()]));
    assert_eq!(
        tail(&found, "X.Deep"),
        expected(&[n::o(&[("w", n::o(&[("only", n::f())]))]), n::o(&[]), "[[\"w\", \"only\"]]".into()])
    );
}

#[test]
fn a_profile_store_root_is_an_object_with_declared_fields_and_a_data_store_keeps_its_record_root() {
    let message = "a profile store's value is an object with declared fields; wrap the record in a field";
    let game = data_game();
    game.write(
        "src/shared/records.ts",
        &format!(
            "{SHARED}{}{}{}{}export const SInferred = profileStore(\"X.Inferred\", {{ template: {{}} as Record<string, number> }});\n{}",
            store("Record", "Record<string, number>"),
            store("Map", "Map<string, number>"),
            store("Index", "{ [key: string]: number }"),
            store("Union", "Map<\"a\" | \"b\", number>"),
            store("Wrapped", "{ upgrades: Record<string, number> }"),
        ),
    );
    game.write(
        "src/server/records.ts",
        &format!(
            "{SERVER}export const DRecord = dataStore<Record<string, number>>(\"X.DataRecord\", {{ defaults: {{}} }});\nexport const DMap = dataStore<Map<string, number>>(\"X.DataMap\", {{ defaults: new Map() }});\n"
        ),
    );
    let reports = reports_of(&game);
    let rejected: Vec<String> = ["Record", "Map", "Index", "Union", "Inferred"]
        .iter()
        .map(|n| format!("CRU019: X.{n} value: {message}"))
        .collect();
    assert_eq!(messages(&reports, "src/shared/records.ts"), rejected);
    assert_eq!(messages(&reports, "src/server/records.ts"), Vec::<String>::new());
    let facts = game.facts();
    let server = declarations(&facts, &game, "src/server/records.ts");
    let record = Schema::Map(MapKey::String, Box::new(Schema::F64));
    assert_eq!(server["X.DataRecord"].schemas, std::slice::from_ref(&record));
    assert_eq!(server["X.DataMap"].schemas, std::slice::from_ref(&record));
    let wrapped = &declarations(&facts, &game, "src/shared/records.ts")["X.Wrapped"];
    let object = Schema::Object(vec![Field { name: "upgrades".into(), value: record, server_only: false }]);
    assert_eq!(wrapped.schemas, [object.clone(), object]);
    assert_eq!(wrapped.tail, Tail::ServerOnly(Vec::new()));
}

#[test]
fn a_profile_store_field_name_may_hold_digits_and_underscores() {
    let game = data_game();
    game.write(
        "src/shared/digits.ts",
        &format!("{SHARED}{}", store("Digits", "{ slot2: number; item_3b: string; Tier1: boolean }")),
    );
    none(&reports_of(&game));
    let view = n::o(&[("Tier1", n::b()), ("item_3b", n::s()), ("slot2", n::f())]);
    let found = calls(&game.luau("shared/digits.luau"), &DATA);
    assert_eq!(tail(&found, "X.Digits"), expected(&[view.clone(), view, "[]".into()]));
}

#[test]
fn persist_depth_stops_at_persist_depth() {
    let nested = |n: usize| format!("{}string{}", "{v:".repeat(n), "}".repeat(n));
    let game = data_game();
    game.write(
        "src/shared/deep.ts",
        &format!(
            "{SHARED}export const Deep = profileStore<{}>(\"X.Deep\", {{ template: undefined as never }});",
            nested(27)
        ),
    );
    none(&reports_of(&game));
    let game = data_game();
    game.write(
        "src/server/deep.ts",
        &format!("{SERVER}export const Deep = config<{}>(\"X.Deep\", undefined as never);", nested(28)),
    );
    let found = messages(&reports_of(&game), "src/server/deep.ts");
    assert!(
        found
            .iter()
            .any(|m| m.starts_with("CRU019: X.Deep value.v") && m.ends_with(": schema exceeds the depth limit of 28")),
        "{found:?}"
    );
}

fn type_codes(game: &Game) -> Vec<(String, i32)> {
    match game.analyze() {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics.iter().map(|d| (format!("{}:{}", game.relative(d), d.line), d.code)).collect(),
    }
}

#[test]
fn the_network_type_probes_keep_literal_inference_invariance_and_target_callback_errors() {
    let net = fixture("network/net.d.ts");
    let extras = &net[net.find("// @trioforge/crucible/server").expect("net.d.ts declares the facades")..];
    for (name, errors) in [("pos", 0), ("neg", 19), ("v1lit", 6)] {
        let game = fresh();
        let network = game.dir.join("node_modules/@trioforge/crucible/out/network.d.ts");
        let declared = std::fs::read_to_string(&network).unwrap();
        std::fs::write(&network, format!("{declared}\n{extras}")).unwrap();
        let index = game.dir.join("node_modules/@trioforge/crucible/out/index.d.ts");
        let root = std::fs::read_to_string(&index).unwrap();
        std::fs::write(&index, format!("{root}\nexport * from \"./network\";\n")).unwrap();
        let probe = fixture(&format!("network/{name}.ts.txt")).replace("\"./net\"", "\"@trioforge/crucible\"");
        game.write("src/shared/probe.ts", &probe);
        let found = type_codes(&game);
        assert_eq!(found.len(), errors, "{name}: {found:?}");
        if name == "neg" {
            for line in [12, 13, 14, 15, 16, 17, 18, 20, 24, 25, 26, 27, 28] {
                let at = format!("src/shared/probe.ts:{line}");
                assert!(found.iter().any(|(l, _)| *l == at), "negative probe line {line}: {found:?}");
            }
            assert!(
                !found.iter().any(|(l, _)| l == "src/shared/probe.ts:23"),
                "TypeScript 7 reads a block body without a return as returning undefined, which the response admits"
            );
        }
    }
}

#[test]
fn message_tokens_reject_both_variance_directions_receiver_calls_and_client_ready_sends() {
    let game = fresh();
    game.write(
        "src/client/tokens.ts",
        "
import {ToServer,Receiver,ClientReady} from \"@trioforge/crucible\";
import {Network} from \"@trioforge/crucible/client\";
declare const narrow: ToServer<{x:number}>;
declare const wide: ToServer<{x:number|string}>;
declare const receiver: Receiver;
export const covariant: typeof wide = narrow;
export const contravariant: typeof narrow = wide;
receiver();
Network.send(ClientReady);
",
    );
    let mut codes: Vec<i32> = type_codes(&game).into_iter().map(|(_, code)| code).collect();
    codes.sort();
    assert_eq!(codes, [2322, 2322, 2349, 2554]);
}

#[test]
fn an_unknown_response_accepts_an_empty_callback_and_callbacks_that_break_their_target_are_type_errors() {
    let messages = "import { askServer, toServer } from \"@trioforge/crucible\"; export const Query = askServer<undefined, unknown>(\"X.Query\"); export const Typed = askServer<undefined, string | undefined>(\"X.Typed\"); export const Event = toServer<string>(\"X.Event\"); export const Ask = askServer<string, number>(\"X.Ask\");";
    let receiver = |field: &str| {
        format!(
            "import {{ receive, Subsystem, ClientReady, ToServer }} from \"@trioforge/crucible\"; import {{ Query, Typed, Event, Ask }} from \"../shared/messages\"; export class Q extends Subsystem {{ readonly r = {field}; }}"
        )
    };
    let game = written(&[
        ("src/shared/messages.ts", messages.to_owned()),
        ("src/server/query.ts", receiver("receive(Query, () => {})")),
    ]);
    none(&reports_of(&game));
    game.compile();
    let game = written(&[
        ("src/shared/messages.ts", messages.to_owned()),
        ("src/server/typed.ts", receiver("receive(Typed, () => {})")),
    ]);
    assert_eq!(type_codes(&game), [], "TypeScript 7 reads a block body without a return as returning undefined");
    for (field, code) in [
        ("receive(Event, (p: Player, s: number) => {})", 2345),
        ("receive(ClientReady, (p: string) => {})", 2345),
        ("receive(Ask, (p, s) => s)", 2322),
        ("receive<ToServer<number>>(Event, (p, n) => {})", 2345),
    ] {
        let game = written(&[("src/shared/messages.ts", messages.to_owned()), ("src/server/case.ts", receiver(field))]);
        assert_eq!(type_codes(&game), [("src/server/case.ts:1".to_owned(), code)], "{field}");
    }
}

#[test]
fn explicit_receive_type_arguments_check_a_named_callback() {
    let game = written(&[
        (
            "src/shared/messages.ts",
            "import { toServer } from \"@trioforge/crucible\"; export const Event = toServer<string>(\"X.Event\");".into(),
        ),
        (
            "src/server/named.ts",
            "import { receive, Subsystem, ToServer } from \"@trioforge/crucible\"; import { Event } from \"../shared/messages\"; function named(player: Player, payload: string) {} export class Q extends Subsystem { readonly r = receive<ToServer<number>>(Event, named); }".into(),
        ),
    ]);
    assert_eq!(type_codes(&game), [("src/server/named.ts:1".to_owned(), 2345)]);
}

#[test]
fn every_declaration_kind_and_receive_lower_to_their_exact_emit_call() {
    let game = data_game();
    game.write(
        "src/shared/kinds.ts",
        &format!(
            "{SHARED}import {{ toClient, askServer, replicatedPerPlayer }} from \"@trioforge/crucible\";
export const A = toServer<{{ a: string }}>(\"K.ToServer\");
export const B = toClient<u8>(\"K.ToClient\");
export const C = askServer<string, number>(\"K.Ask\", {{ timeout: 5 }});
export const D = replicated<number>(\"K.Replicated\", 0);
export const E = replicatedPerPlayer<boolean>(\"K.PerPlayer\", false);
export const F = profileStore(\"K.Profile\", {{ template: {{ coins: 0, secret: serverOnly(\"\") }} }});
"
        )
        .replace("{ toClient,", "{ toClient, type u8,"),
    );
    game.write(
        "src/server/kinds.ts",
        &format!(
            "{SERVER}import {{ Subsystem, receive }} from \"@trioforge/crucible\";
import {{ A }} from \"../shared/kinds\";
export const G = dataStore<{{ coins: number }}>(\"K.Data\", {{ defaults: {{ coins: 0 }} }});
export const H = profileMessage<string>(\"K.Message\");
export const I = config<number>(\"K.Config\", 1);
export const J = playerConfig<string>(\"K.PlayerConfig\", \"x\");
export class Receiver extends Subsystem {{ readonly ping = receive(A, (player, value) => {{}}); }}
"
        ),
    );
    none(&reports_of(&game));
    let shared = game.luau("shared/kinds.luau");
    for line in [
        "local A = __crucible.toServer(\"K.ToServer\", __crucible.n.object({ { \"a\", __crucible.n.string } }))",
        "local B = __crucible.toClient(\"K.ToClient\", __crucible.n.u8)",
        "local C = __crucible.askServer(\"K.Ask\", __crucible.n.string, __crucible.n.f64, 5)",
        "local D = __crucible.replicated(\"K.Replicated\", 0, __crucible.n.f64)",
        "local E = __crucible.replicatedPerPlayer(\"K.PerPlayer\", false, __crucible.n.bool)",
    ] {
        has(&shared, line);
    }
    let profile = &calls(&shared, &DATA)["K.Profile"];
    let persist = n::o(&[("coins", n::f()), ("secret", n::s())]);
    assert_eq!(
        profile[2..].iter().map(|a| canon(a)).collect::<Vec<_>>(),
        expected(&[persist, n::o(&[("coins", n::f())]), "[[\"secret\"]]".into()])
    );
    let server = game.luau("server/kinds.luau");
    for line in [
        "local H = __crucible.profileMessage(\"K.Message\", __crucible.n.string)",
        "local I = __crucible.config(\"K.Config\", 1, __crucible.n.f64)",
        "local J = __crucible.playerConfig(\"K.PlayerConfig\", \"x\", __crucible.n.string)",
        "self.ping = __crucible.receive(self, \"ping\", A, function(player, value) end)",
    ] {
        has(&server, line);
    }
    same(&calls(&server, &DATA)["K.Data"][2], &n::o(&[("coins", n::f())]), "K.Data");
}

#[test]
#[ignore = "runs the emitted Luau in Lune, which must be on PATH"]
fn compiled_declarations_load_against_the_compiler_built_runtime() {
    let scratch = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("declarations");
    let _ = std::fs::remove_dir_all(&scratch);
    let runtime = crucible_transform::compile(&repo().join("packages/crucible/tsconfig.json"), Target::Package)
        .unwrap_or_else(|f| panic!("the runtime compiles:\n{f}"))
        .modules;
    crucible_compiler::write(&scratch.join("runtime"), &runtime).expect("the runtime is written");
    let game = fresh();
    game.write(
        "src/shared/messages.ts",
        "import { replicated, toServer } from \"@trioforge/crucible\";\nexport const Ping = toServer<{ a: string }>(\"X.Ping\");\nexport const Score = replicated<number>(\"Score\", 0);\n",
    );
    game.write(
        "src/server/data.ts",
        "import { config, dataStore } from \"@trioforge/crucible/server\";\nexport const Limit = config<number>(\"Limit\", 1);\nexport const Saves = dataStore<{ coins: number }>(\"Saves\", { defaults: { coins: 0 } });\n",
    );
    crucible_compiler::write(&scratch.join("game"), &game.compile()).expect("the game is written");
    let result = std::process::Command::new("lune")
        .current_dir(repo())
        .arg("run")
        .arg(repo().join("crates/transform/tests/lowering/declarations.luau"))
        .arg(repo().join("build/sample-game.rbxl"))
        .arg(scratch.join("runtime"))
        .arg(scratch.join("game"))
        .output()
        .expect("lune runs");
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(result.status.success(), "{stdout}{}", String::from_utf8_lossy(&result.stderr));
    assert!(stdout.contains("declarations load"), "{stdout}");
}

#[test]
fn the_fd_catalogue_lowers_all_fifty_payloads_and_the_typed_wire_tree() {
    let game = fresh();
    game.write("src/shared/fd.ts", &fixture("network/fd.ts"));
    game.write("src/shared/messages.ts", &fixture("network/fd-messages.ts"));
    let luau = game.luau("shared/messages.luau");
    let actual = calls(&luau, &["toServer", "toClient", "askServer"]);
    let expected: HashMap<String, Vec<String>> = serde_json::from_str(&fixture("network/fd-expected.json")).unwrap();
    assert_eq!(expected.len(), 51);
    assert_eq!(actual.len(), 52);
    for (name, descriptors) in expected {
        let found = &actual[&name][1..];
        for (actual, expected) in found.iter().zip(&descriptors) {
            same(actual, expected, &name);
        }
        assert_eq!(found.len(), descriptors.len(), "{name}");
    }
    assert!(actual["Admin.FullRecord"][1].contains("DailyClaims"));
    assert!(actual["Admin.FullRecord"][1].contains("JobsCompleted"));
    assert!(actual["Admin.FullRecord"][1].contains("PlayTime"));
}
