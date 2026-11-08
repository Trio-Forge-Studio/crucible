mod support;

use std::path::PathBuf;
use std::sync::LazyLock;

use crucible_compiler::{Failure, Module, Target};
use support::{Game, has, repo};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/jsx").join(name)
}

static VIDE: LazyLock<Vec<Module>> = LazyLock::new(|| {
    crucible_transform::compile(&fixture("vide/tsconfig.json"), Target::Package)
        .unwrap_or_else(|failure| panic!("the Vide fixtures compile (run pnpm build first):\n{failure}"))
        .modules
});

fn vide(path: &str) -> &'static str {
    &VIDE.iter().find(|m| m.path == path).unwrap_or_else(|| panic!("{path} is emitted")).source
}

#[test]
fn an_element_calls_the_factory_with_its_tag_attributes_and_children() {
    let panel = vide("panel.luau");
    for part in [
        "return Vide.jsx(\"textlabel\", {\n\t\tName = \"Label\",\n\t\tText = props.text,\n\t\tLayoutOrder = props.order,\n\t\tBackgroundTransparency = 1,\n\t}, props.children)",
        "Name = \"Panel\",\n\t\tVisible = true,",
        "Vide.jsx(Label, {\n\t\ttext = props.title,\n\t\torder = 1,\n\t})",
        "Vide.jsx(Show, {\n\t\twhen = props.open,\n\t}, function()",
        "Vide.jsx(Vide.Fragment, nil, Vide.jsx(\"uicorner\", {",
    ] {
        has(panel, part);
    }
}

#[test]
fn a_default_import_of_an_export_equals_module_is_the_whole_module() {
    has(
        vide("panel.luau"),
        "local _0 = TS.import(script, TS.getModule(script, \"@rbxts\", \"vide\").src)\nlocal Vide = _0\n",
    );
    has(vide("defaults.luau"), "local value = TS.import(script, script.Parent, \"value\").default\n");
}

#[test]
fn spreads_copy_in_at_their_place_and_skip_a_nil_source() {
    has(
        vide("panel.luau"),
        "local _1 = {}\n\tfor _2, _3 in close do\n\t\t_1[_2] = _3\n\tend\n\t_1.Activated = props.onClose",
    );
    has(
        vide("shapes.luau"),
        "local _0 = {\n\tName = \"Last\",\n}\nif maybe then\n\tfor _1, _2 in maybe do\n\t\t_0[_1] = _2\n\tend\nend\n_0.Visible = true\nlocal spreadAfter = h(\"frame\", _0)",
    );
    has(vide("shapes.luau"), "local spreadChildren = h(\"frame\", nil, table.unpack(rows))");
}

#[test]
fn values_keep_their_order_when_a_later_one_needs_statements() {
    has(
        vide("panel.luau"),
        "local _4 = {\n\t\tName = \"Panel\",\n\t\tVisible = true,\n\t}\n\tlocal _5 = Vide.jsx(Label, {",
    );
    has(vide("panel.luau"), "return Vide.jsx(\"frame\", _4, _5, Vide.jsx(\"textbutton\", _1),");
    has(vide("shapes.luau"), "local ordered = h(\"frame\", {\n\tLayoutOrder = count(),\n}, count(), count())");
}

#[test]
fn text_is_trimmed_and_joined_by_line_and_a_pragma_picks_the_factory() {
    let shapes = vide("shapes.luau");
    has(
        shapes,
        "local text = h(\"frame\", nil, \"Coins & gems ! left over from the last delivery, which the formatter keeps on several lines because the text is longer than one line\")",
    );
    has(shapes, "local spaced = h(\"frame\", nil, \" a \")");
    has(
        shapes,
        "local entities = h(\"frame\", {
	Name = \"© &x41; &#X41; &bogus;\",
}, \"© © A — … € &bogus; &\")",
    );
    has(shapes, "local empty = h(Group)");
    has(shapes, "local grouped = h(Group, nil, count())");
}

#[test]
fn jsx_needs_the_react_setting_and_a_factory() {
    let game = Game::new();
    game.config(|c| c["compilerOptions"]["jsx"] = "preserve".into());
    game.write("src/client/view.tsx", "declare global {\n\tnamespace JSX {\n\t\tinterface IntrinsicElements {\n\t\t\tframe: {};\n\t\t}\n\t}\n}\nexport const view = <frame />;\n");
    let Err(Failure::Lower(errors)) = game.try_compile() else {
        panic!("JSX without a factory fails the compile");
    };
    let found: Vec<String> = errors.iter().map(|e| format!("{}:{} {}", e.line, e.column, e.message)).collect();
    assert_eq!(
        found,
        [
            "8:21 JSX without `jsx: \"react\"` and a `jsxFactory`, or `jsx: \"react-jsx\"` and `jsxImportSource: \"@trioforge/crucible/ui\"` is not supported"
        ]
    );
}

#[test]
#[ignore = "runs the emitted Luau in Lune, which must be on PATH"]
fn the_vide_lowering_mounts_updates_and_connects_through_vide() {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("jsx-vide");
    let _ = std::fs::remove_dir_all(&out);
    crucible_compiler::write(&out, &VIDE).expect("the fixtures are written");
    let result = std::process::Command::new("lune")
        .current_dir(fixture(""))
        .arg("run")
        .arg("vide.luau")
        .arg(&out)
        .arg(repo().join("packages/crucible/node_modules/@rbxts/vide"))
        .output()
        .expect("lune runs");
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(result.status.success(), "{stdout}{}", String::from_utf8_lossy(&result.stderr));
    assert!(stdout.contains("vide fixtures pass"), "{stdout}");
}

fn ui_game() -> Game {
    let game = Game::ui();
    game.write("theme.toml", include_str!("classes/theme.toml"));
    for entry in std::fs::read_dir(fixture("ui")).expect("the UI fixtures exist") {
        let path = entry.expect("the entry is readable").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        game.write(&format!("src/client/{name}"), &std::fs::read_to_string(&path).unwrap());
    }
    game
}

static UI: LazyLock<(Game, Vec<Module>)> = LazyLock::new(|| {
    let game = ui_game();
    let modules = game.compile();
    (game, modules)
});

fn ui(path: &str) -> &'static str {
    &UI.1.iter().find(|m| m.path == path).unwrap_or_else(|| panic!("{path} is emitted")).source
}

#[test]
fn an_intrinsic_element_sets_fixed_props_binds_readables_and_connects_events() {
    has(
        ui("client/counter.luau"),
        "local _0 = Instance.new(\"TextButton\")\n\t_0.Name = \"Counter\"\n\t__ui.bind(_0, \"Text\", function()\n\t\treturn `Coins {props.coins()}`\n\tend)\n\t_0.TextSize = 18\n\t_0.Activated:Connect(props.onBuy)\n\treturn _0",
    );
    has(ui("client/panel.luau"), "__ui.bind(_3, \"Text\", title)");
    has(ui("client/list.luau"), "_3.Text = item\n\t\t\t\t__ui.bind(_3, \"LayoutOrder\", index)");
}

#[test]
fn compiled_jsx_imports_the_ui_entry_from_the_place() {
    has(
        ui("client/counter.luau"),
        "local __ui = TS.import(script, game:GetService(\"ReplicatedStorage\"), \"rbxts_include\", \"node_modules\", \"@trioforge\", \"crucible\", \"out\", \"ui\")",
    );
}

#[test]
fn children_parent_an_element_and_append_any_other_node() {
    has(
        ui("client/list.luau"),
        "local _2 = Instance.new(\"UIListLayout\")\n\t\t_2.Name = \"Layout\"\n\t\t_2.Parent = _1\n\t\t__ui.append(_1, For({",
    );
    has(ui("client/cards.luau"), "_0.Name = props.title\n\t__ui.append(_0, props.children)");
}

#[test]
fn a_component_takes_a_props_table_with_its_children_and_a_fragment_is_an_array() {
    has(ui("client/cards.luau"), "_2.children = _1\n\t\treturn { Card(_2), Card({\n\t\t\ttitle = \"B\",\n\t\t}) }");
    has(ui("client/panel.luau"), "Show({\n\t\t\twhen = open,\n\t\t\tfallback = function()");
}

#[test]
fn a_value_that_may_be_nil_is_set_bound_or_connected_only_when_it_is_not_nil() {
    has(
        ui("client/optional.luau"),
        "local _1 = props.label\n\tif _1 ~= nil then\n\t\t_0.Text = _1\n\tend\n\tlocal _2 = props.size\n\tif _2 ~= nil then\n\t\t__ui.bind(_0, \"TextSize\", _2)\n\tend\n\tlocal _3 = props.onClick\n\tif _3 ~= nil then\n\t\t_0.Activated:Connect(_3)\n\tend",
    );
    has(ui("client/optional.luau"), "_0.Name = \"Button\"\n\tlocal _1");
}

#[test]
fn an_intrinsic_element_refuses_mixed_props_and_spreads() {
    let game = ui_game();
    game.write(
        "src/client/mixed.tsx",
        "import type { Readable } from \"@trioforge/crucible\";\nexport const mixed = (text: string | Readable<string>) => [\n\t<textlabel Text={text} />,\n\t<textlabel {...{ Text: \"x\" }} />,\n];\n",
    );
    let Err(Failure::Lower(errors)) = game.try_compile() else {
        panic!("the refused props fail the compile");
    };
    let mut found: Vec<String> = errors
        .iter()
        .filter(|e| e.path.ends_with("/mixed.tsx"))
        .map(|e| format!("{}:{} {}", e.line, e.column, e.message))
        .collect();
    found.sort();
    let expected = [
        "3:13 the type facts cannot answer this Jsx site (Mixed)",
        "3:13 the type facts do not say how this attribute lowers",
        "4:17 a spread attribute on an intrinsic element is not supported",
    ];
    assert_eq!(found, expected);
}

#[test]
#[ignore = "runs the emitted Luau in Lune, which must be on PATH"]
fn the_compiled_lowering_mounts_updates_and_unmounts_on_the_compiler_built_runtime() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("jsx-ui");
    let _ = std::fs::remove_dir_all(&root);
    let runtime = crucible_transform::compile(&repo().join("packages/crucible/tsconfig.json"), Target::Package)
        .unwrap_or_else(|f| panic!("the runtime compiles:\n{f}"))
        .modules;
    let installed = root.join("ReplicatedStorage/rbxts_include/node_modules/@trioforge/crucible/out");
    crucible_compiler::write(&installed, &runtime).expect("the runtime is written");
    let client: Vec<Module> =
        UI.1.iter()
            .filter_map(|m| Some(Module { path: m.path.strip_prefix("client/")?.to_owned(), source: m.source.clone() }))
            .collect();
    crucible_compiler::write(&root.join("StarterPlayer/StarterPlayerScripts/TS"), &client).expect("the UI is written");
    let result = std::process::Command::new("lune")
        .current_dir(fixture(""))
        .arg("run")
        .arg("ui.luau")
        .arg(&root)
        .output()
        .expect("lune runs");
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(result.status.success(), "{stdout}{}", String::from_utf8_lossy(&result.stderr));
    assert!(stdout.contains("ui specs pass"), "{stdout}");
}
