mod support;

use std::path::PathBuf;
use std::sync::LazyLock;

use crucible_compiler::{Failure, Module, Target};
use support::{Game, has, repo};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/jsx").join(name)
}

static VIDE: LazyLock<Vec<Module>> = LazyLock::new(|| {
    crucible_compiler::compile(&fixture("vide/tsconfig.json"), Target::Package)
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
    assert_eq!(found, ["8:21 JSX without `jsx: \"react\"` and a `jsxFactory` is not supported"]);
}

#[test]
fn automatic_jsx_options_fail_at_the_tsconfig_before_checking_ui_imports() {
    for (jsx, source) in [
        ("react-jsx", None),
        ("react-jsxdev", None),
        ("react-jsx", Some("@trioforge/crucible/ui")),
        ("react", Some("@rbxts/vide")),
        ("preserve", Some("another-renderer")),
    ] {
        let game = Game::new();
        game.config(|c| {
            c["compilerOptions"]["jsx"] = jsx.into();
            if let Some(source) = source {
                c["compilerOptions"]["jsxImportSource"] = source.into();
            }
        });
        game.write(
            "src/client/view.tsx",
            "import { mount } from \"@trioforge/crucible/ui\";\nexport const view = <frame />;\n",
        );
        assert_jsx_config_error(&game);
    }
}

#[test]
fn inherited_jsx_options_fail_at_the_invoked_tsconfig() {
    let game = Game::new();
    game.write("jsx-options.json", r#"{"compilerOptions":{"jsx":"react-jsx"}}"#);
    game.config(|c| {
        let base = c["extends"].take();
        c["extends"] = serde_json::json!([base, "./jsx-options.json"]);
    });
    game.main("export {};\n");
    assert_jsx_config_error(&game);
}

fn assert_jsx_config_error(game: &Game) {
    let Err(Failure::Check(errors)) = game.try_compile() else {
        panic!("automatic JSX options fail before source checking");
    };
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(game.relative(error), "tsconfig.json");
    assert_eq!((error.line, error.column, error.code), (1, 1, 0));
    assert_eq!(
        error.message,
        "`jsx: \"react-jsx\"`, `jsx: \"react-jsxdev\"` and `jsxImportSource` are not supported; use `jsx: \"react\"` with `jsxFactory: \"Vide.jsx\"` and remove `jsxImportSource` in the tsconfig."
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
