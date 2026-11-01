use std::path::PathBuf;
use std::sync::LazyLock;

use crucible_compiler::{Failure, Module, Written};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(name)
}

static MODULES: LazyLock<Vec<Module>> = LazyLock::new(|| {
    crucible_compiler::compile(&fixture("lowering/tsconfig.json"))
        .unwrap_or_else(|failure| panic!("the lowering fixtures compile (run pnpm build first):\n{failure}"))
});

fn luau(path: &str) -> &'static str {
    let module = MODULES.iter().find(|m| m.path == path).unwrap_or_else(|| panic!("{path} is emitted"));
    &module.source
}

fn has(path: &str, lines: &[&str]) {
    let source = luau(path);
    for line in lines {
        assert!(source.contains(line), "{path} contains `{line}`:\n{source}");
    }
}

#[test]
fn every_source_file_becomes_a_module_and_index_becomes_init() {
    let mut paths: Vec<&str> = MODULES.iter().map(|m| m.path.as_str()).collect();
    paths.sort_unstable();
    let expected = [
        "classes.luau",
        "control.luau",
        "macros.luau",
        "modules.luau",
        "nested/init.luau",
        "operators.luau",
        "patterns.luau",
        "util.luau",
    ];
    assert_eq!(paths, expected);
}

#[test]
fn relative_imports_go_through_ts_import_from_the_module_folder() {
    has(
        "modules.luau",
        &[
            "local TS = _G[script]",
            "local _0 = TS.import(script, script.Parent, \"util\")\nlocal double = _0.double\nlocal greeting = _0.greeting",
            "local quadruple = TS.import(script, script.Parent, \"nested\").quadruple",
            "hello = _1.greeting,",
        ],
    );
    has("nested/init.luau", &["TS.import(script, script.Parent, \"util\")", "local high = 5"]);
    assert!(!luau("modules.luau").contains("Shape"), "a type-only import is elided");
    assert!(
        luau("util.luau").starts_with("-- Compiled with crucible-compiler\nlocal greeting = \"hi\""),
        "a module without imports needs no TS"
    );
}

#[test]
fn a_local_used_before_its_declaration_is_declared_first() {
    has("modules.luau", &["local TS = _G[script]\nlocal late\n", "\nfunction late()\n"]);
    has("modules.luau", &["local Counter = {}\ndo\n\tlocal count = 0\n", "\tCounter.advance = advance\nend"]);
}

#[test]
fn loops_keep_javascript_order_and_continue() {
    has(
        "control.luau",
        &[
            "\t\tlocal i = 0\n\t\tlocal _0 = false\n\t\twhile true do\n\t\t\tif _0 then\n\t\t\t\ti += 1\n\t\t\telse\n\t\t\t\t_0 = true",
            "\t\t\tif not (i < limit) then\n\t\t\t\tbreak\n",
            "\trepeat\n\t\ttable.insert(seen, n)\n\t\tn -= 1\n\tuntil not (n > 0)",
            "for _8, item in list do",
            "for key, value in lookup do",
            "for member in members do",
            ", bound in { bounds() } do",
        ],
    );
}

#[test]
fn switch_lowers_to_a_breakable_block_with_fallthrough() {
    has(
        "control.luau",
        &[
            "\trepeat\n\t\tif n == 0 then\n\t\t\treturn \"zero\"\n\t\tend\n\t\tif n == 1 or n == 2 then",
            "\t\tlocal _4 = false\n\t\tif n == 1 then\n\t\t\ttable.insert(steps, \"one\")\n\t\t\t_4 = true\n\t\tend\n\t\tif _4 or n == 2 then",
        ],
    );
}

#[test]
fn classes_follow_the_roblox_ts_class_shape() {
    has(
        "classes.luau",
        &[
            "\tBase = {}\n\tfunction Base:constructor(name)\n\t\tself.name = name\n\t\tself.tags = {}",
            "\tlocal super = Base\n\tLeaf = setmetatable({}, {\n\t\t__tostring = function()\n\t\t\treturn \"Leaf\"\n\t\tend,\n\t\t__index = super,\n\t})\n\tLeaf.__index = Leaf\n\tfunction Leaf.new(...)",
            "\t\tsuper.constructor(self, name)\n\t\tself.extra = extra\n\t\tself.count = 1\n\t\tself.count += extra",
            "`{super.describe(self)}+{self.count}+{#self.tags}`",
            "\tLeaf[\"end\"] = function(self)",
            "leaf[\"end\"](leaf)",
            "Leaf:make()",
            "\tfunction Leaf:__tostring()\n\t\treturn self:toString()\n\tend\n\tLeaf.label = \"leaf\"",
        ],
    );
    assert!(!luau("classes.luau").contains("Base.new"), "an abstract class has no constructor function");
}

#[test]
fn truthiness_tests_the_falsy_values_the_facts_allow() {
    has(
        "operators.luau",
        &[
            "if n ~= 0 and n == n then",
            "if s ~= \"\" then",
            "if not flag then",
            "\tlocal _3 = n\n\tif not (_3 ~= 0 and _3 == _3) then\n\t\t_3 = fallback\n\tend\n\treturn _3",
            "return value or fallback",
            "\tlocal _5 = value\n\tif _5 == nil then\n\t\t_5 = true\n\tend",
        ],
    );
    has("macros.luau", &["assert(n ~= 0 and n == n, \"zero\")"]);
}

#[test]
fn operators_use_the_operand_facts() {
    has(
        "operators.luau",
        &[
            "local text = label .. tostring(n)\n\ttext ..= \"!\"\n\treturn text .. tostring(n + 1)",
            "\tlocal _6 = i\n\ti = _6 + 1\n\tlocal a = _6\n\ti += 1\n\tlocal b = i",
            "{ bit32.band(a, b), bit32.bor(a, b), bit32.lshift(a, 2) }",
            "\t\t_8 = _8:get()",
            "return if flag then n * 2 else n - 1",
        ],
    );
}

#[test]
fn macros_expand_inline() {
    has(
        "macros.luau",
        &[
            "table.insert(list, 4)",
            "\tlocal _0 = #list\n\tlocal _1 = list[_0]\n\tlist[_0] = nil",
            "_2[_4] = _3(_5, _4 - 1, list)",
            "(table.find(list, 2) or 0) - 1",
            "table.find(list, 3) ~= nil",
            "\tmap.a = 1\n\tmap.b = 2",
            "\tlocal set = {\n\t\tx = true,\n\t\ty = true,\n\t}",
            "{ #text, string.sub(text, 1, 2), #string.split(text, \",\"), type(text) == \"string\", typeof(1) }",
        ],
    );
}

#[test]
fn lua_tuples_bind_and_return_several_values() {
    has(
        "patterns.luau",
        &[
            "\treturn 1, \"one\"",
            "local n, word = pair()",
            "local rest = table.move(list, 4, #list, 1, {})",
            "local function parameters(_0, _1, ...)",
            "\tlocal _2 = pair()\n\ttarget.left = _2",
        ],
    );
}

#[test]
fn out_of_scope_constructs_are_errors_at_their_location() {
    let Err(Failure::Lower(errors)) = crucible_compiler::compile(&fixture("lowering-errors/tsconfig.json")) else {
        panic!("the refused constructs fail the compile");
    };
    let found: Vec<String> = errors
        .iter()
        .map(|e| {
            assert!(e.path.ends_with("/lowering-errors/src/refused.ts"), "{e}");
            format!("{}:{} {}", e.line, e.column, e.message)
        })
        .collect();
    let expected = [
        "1:1 an async function is not supported",
        "3:1 a generator is not supported",
        "5:24 a regular expression literal is not supported",
        "7:1 export let is not supported; export a const",
        "10:9 loose equality is not supported; use === or !==",
        "14:8 `table` cannot be a Luau local name",
        "19:2 try is not supported",
        "25:9 instanceof is not supported",
    ];
    assert_eq!(found, expected);
}

#[test]
fn write_skips_unchanged_modules_and_removes_only_its_own_stale_ones() {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("write");
    let _ = std::fs::remove_dir_all(&out);
    let first = crucible_compiler::write(&out, &MODULES).expect("the fixtures are written");
    assert_eq!(first, Written { written: MODULES.len(), removed: 0 });
    std::fs::write(out.join("foreign.luau"), "return nil\n").unwrap();
    std::fs::write(out.join("nested/init.d.ts"), "export {};\n").unwrap();
    std::fs::create_dir_all(out.join("other")).unwrap();
    std::fs::write(out.join("other/kept.luau"), "-- Compiled with roblox-ts v3.0.0\n").unwrap();

    let without: Vec<Module> =
        MODULES.iter().filter(|m| !matches!(m.path.as_str(), "util.luau" | "nested/init.luau")).cloned().collect();
    let second = crucible_compiler::write(&out, &without).expect("the fixtures are written again");
    assert_eq!(second, Written { written: 0, removed: 2 });
    assert!(!out.join("util.luau").exists(), "a removed source's module is deleted");
    assert!(!out.join("nested/init.luau").exists());
    assert!(out.join("nested/init.d.ts").exists(), "a declaration beside it stays, and so does its folder");
    assert!(out.join("foreign.luau").exists(), "a file without the header stays");
    assert!(out.join("other/kept.luau").exists());

    std::fs::remove_file(out.join("nested/init.d.ts")).unwrap();
    std::fs::write(out.join("nested/init.luau"), luau("nested/init.luau")).unwrap();
    let third = crucible_compiler::write(&out, &without).expect("the fixtures are written a third time");
    assert_eq!(third, Written { written: 0, removed: 1 });
    assert!(!out.join("nested").exists(), "a folder the deletion empties is deleted");
}

#[test]
#[ignore = "runs the emitted Luau in Lune, which must be on PATH"]
fn the_emitted_luau_computes_what_the_typescript_does() {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lowering");
    let _ = std::fs::remove_dir_all(&out);
    crucible_compiler::write(&out, &MODULES).expect("the fixtures are written");
    let result = std::process::Command::new("lune")
        .arg("run")
        .arg(fixture("lowering/run.luau"))
        .arg(&out)
        .output()
        .expect("lune runs");
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(result.status.success(), "{stdout}{}", String::from_utf8_lossy(&result.stderr));
    assert!(stdout.contains("lowering fixtures pass"), "{stdout}");
}
