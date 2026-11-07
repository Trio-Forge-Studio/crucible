mod support;
use support::{Game, has};

fn game() -> Game {
    let game = Game::ui();
    game.write("theme.toml", include_str!("classes/theme.toml"));
    game
}

#[test]
fn accepted_class_forms_store_one_style_fact() {
    let g = game();
    g.write("src/client/view.tsx", include_str!("classes/forms.tsx"));
    let facts = g.facts();
    assert!(facts.diagnostics.is_empty(), "{}", support::lines(&facts.diagnostics));
    let count = facts
        .files
        .iter()
        .flat_map(|f| f.facts.iter().map(|(_, fact)| fact))
        .filter(|f| matches!(f.prop, Some(crucible_compiler::Prop::Class(_))))
        .count();
    assert_eq!(count, 8);
}

#[test]
fn bad_inputs_report_codes_and_raw_columns_without_deduping_tokens() {
    let g = game();
    g.write("src/client/bad.tsx", include_str!("classes/bad.tsx"));
    assert_eq!(
        g.found_columns(),
        [
            "CRU028@src/client/bad.tsx:2:25",
            "CRU028@src/client/bad.tsx:2:29",
            "CRU028@src/client/bad.tsx:3:29",
            "CRU028@src/client/bad.tsx:4:31",
            "CRU028@src/client/bad.tsx:5:29",
            "CRU028@src/client/bad.tsx:6:20",
            "CRU028@src/client/bad.tsx:7:20",
            "CRU028@src/client/bad.tsx:8:20",
            "CRU031@src/client/bad.tsx:9:20",
            "CRU031@src/client/bad.tsx:10:20",
            "CRU031@src/client/bad.tsx:12:4",
            "CRU029@src/client/bad.tsx:16:36",
            "CRU029@src/client/bad.tsx:17:36",
            "CRU029@src/client/bad.tsx:18:55",
            "CRU029@src/client/bad.tsx:20:34",
        ]
    );
}

#[test]
fn broken_theme_reports_only_cru030() {
    let g = game();
    g.write("theme.toml", "[text]\nleft = 18\n");
    g.write("src/client/view.tsx", "export const view = <frame className=\"unknown also-bad\" />;");
    assert_eq!(g.found_columns(), ["CRU030@theme.toml:2:1"]);
}

#[test]
fn a_classed_element_emits_only_assignments_and_configured_modifiers() {
    let g = game();
    g.write("src/client/panel.tsx", include_str!("classes/panel.tsx"));
    let source = g.luau("client/panel.luau");
    let expected =
        include_str!("classes/panel.luau.snap").lines().map(|line| format!("\t{line}")).collect::<Vec<_>>().join("\n");
    has(&source, &expected);
    for kind in ["UICorner", "UIPadding", "UIStroke", "UIListLayout"] {
        assert_eq!(source.matches(&format!("Instance.new(\"{kind}\")")).count(), 1);
    }
    assert_eq!(source.matches(".Size =").count(), 1);
    assert!(!source.contains("__ui"));
    assert!(!source.contains("className"));
}

#[test]
fn explicit_props_own_each_property_in_either_attribute_order() {
    let g = game();
    g.write("src/client/props.tsx", include_str!("classes/props.tsx"));
    let source = g.luau("client/props.luau");
    assert_eq!(source.matches("__ui.bind(").count(), 4);
    assert_eq!(source.matches("Color3.fromRGB").count(), 2);
    assert_eq!(source.matches("UDim2.new").count(), 2);
    assert_eq!(source.matches(".BackgroundColor3 =").count(), 2);
    assert_eq!(source.matches(".Size =").count(), 2);
    assert_eq!(source.matches(".BackgroundTransparency = 0").count(), 4);
}

#[test]
fn an_optional_prop_overrides_the_class_only_when_it_is_not_nil() {
    let g = game();
    g.write(
        "src/client/fallback.tsx",
        "export const fallback = (color?: Color3) => <frame BackgroundColor3={color} className=\"bg-brand\" />;
",
    );
    has(
        &g.luau("client/fallback.luau"),
        "local _0 = Instance.new(\"Frame\")
	_0.BackgroundColor3 = Color3.fromRGB(37, 99, 235)
	_0.BackgroundTransparency = 0
	local _1 = color
	if _1 ~= nil then
		_0.BackgroundColor3 = _1
	end",
    );
}

#[test]
fn one_size_axis_preserves_the_instances_other_axis() {
    let g = game();
    g.write("src/client/axes.tsx", "export const width = <frame className=\"w-full\" />;\nexport const height = <frame className=\"h-4\" />;\nexport const opacity = <canvasgroup className=\"opacity-90\" />;");
    let source = g.luau("client/axes.luau");
    has(&source, "UDim2.new(UDim.new(1, 0), _0.Size.Y)");
    has(&source, "UDim2.new(_1.Size.X, UDim.new(0, 16))");
    has(&source, ".GroupTransparency = 0.1");
}

#[test]
fn no_theme_keeps_keywords_and_reports_missing_theme_values() {
    let g = Game::ui();
    g.write("src/client/view.tsx", "export const view = <frame className=\"flex border w-full rounded-full\" />;");
    let source = g.luau("client/view.luau");
    has(&source, "Enum.FillDirection.Horizontal");
    has(&source, "UDim.new(1, 0)");
    g.write("src/client/view.tsx", "export const view = <frame className=\"bg-brand\" />;");
    assert_eq!(g.found_columns(), ["CRU028@src/client/view.tsx:1:39"]);
    has(&g.facts().diagnostics[0].message, "No theme.toml");
}

#[test]
fn readonly_and_singleton_types_do_not_authorize_effects_or_mutable_reads() {
    let g = game();
    g.write("src/client/rejected.tsx", include_str!("classes/rejected.tsx"));
    let found = g.found_columns();
    assert_eq!(found, include_str!("classes/rejected.snap").lines().map(str::to_owned).collect::<Vec<_>>());
}

#[test]
fn indirect_unknown_tokens_both_report_at_the_use() {
    let g = game();
    g.write("src/client/view.tsx", "const classes = 'bad nope';\nexport const view = <frame className={classes} />;\n");
    assert_eq!(g.found_columns(), ["CRU028@src/client/view.tsx:2:39", "CRU028@src/client/view.tsx:2:39"]);
}

#[test]
fn theme_is_local_to_the_invoked_config_even_without_a_rojo_project() {
    let g = game();
    g.remove("default.project.json");
    g.write("src/client/view.tsx", "export const view = <frame className=\"bg-brand\" />;");
    g.config(|c| {
        c["compilerOptions"]["rootDir"] = "src/client".into();
        c["compilerOptions"]["jsx"] = "react-jsx".into();
        c["compilerOptions"]["jsxImportSource"] = "@trioforge/crucible/ui".into();
    });
    has(&g.luau("view.luau"), "Color3.fromRGB(37, 99, 235)");
    g.write("src/client/theme.toml", "[color]\nbrand = '#000000'\n");
    has(&g.luau("view.luau"), "Color3.fromRGB(37, 99, 235)");
    g.write("theme.toml", "[color]\nbrand = '#ff0000'\n");
    has(&g.luau("view.luau"), "Color3.fromRGB(255, 0, 0)");
    g.write("nested/tsconfig.json", "{\"extends\": \"../tsconfig.json\", \"include\": [\"../src\"]}");
    let facts = g.analyze_at("nested/tsconfig.json").unwrap();
    assert_eq!(facts.diagnostics.len(), 1);
    assert_eq!(facts.diagnostics[0].code, 90028);
    has(&facts.diagnostics[0].message, "No theme.toml");
}

#[test]
fn explicit_props_do_not_hide_invalid_class_targets() {
    let g = game();
    g.write(
        "src/client/view.tsx",
        "export const view = <frame className=\"text-white\" BackgroundTransparency={1} />;",
    );
    assert_eq!(g.found_columns(), ["CRU031@src/client/view.tsx:1:39"]);
}

#[test]
fn wrong_kind_values_stop_at_the_typescript_boundary() {
    let g = game();
    g.write("src/client/types.tsx", "import type { Readable } from '@trioforge/crucible';\ndeclare const readable: Readable<string>;\nexport const bare = <frame className />;\nexport const read = <frame className={readable} />;\nexport const gui = <screengui className=\"border\" />;\n");
    assert_eq!(g.type_errors(), ["src/client/types.tsx:3", "src/client/types.tsx:4", "src/client/types.tsx:5"]);
}

#[test]
fn each_modifier_kind_conflicts_with_an_authored_direct_child() {
    for (token, tag) in
        [("rounded-md", "uicorner"), ("p-2", "uipadding"), ("border", "uistroke"), ("gap-2", "uilistlayout")]
    {
        let g = game();
        let source = format!("export const view = <frame className=\"{token}\"><{tag} /></frame>;");
        let column = source.find(&format!("<{tag}")).unwrap() + 2;
        g.write("src/client/view.tsx", &source);
        assert_eq!(g.found_columns(), [format!("CRU031@src/client/view.tsx:1:{column}")]);
    }
}

#[test]
fn a_constant_export_does_not_allow_a_call_in_its_receiver() {
    let g = game();
    g.write("src/client/names.ts", "export const card = 'flex';");
    g.write("src/client/view.tsx", "import * as names from './names';\ndeclare function getNames(): typeof names;\nexport const view = <frame className={getNames().card} />;");
    assert_eq!(g.found_columns(), ["CRU029@src/client/view.tsx:3:39"]);
}

const BRAND: &str = "Color3.fromRGB(37, 99, 235)";

#[test]
fn each_variant_binds_its_property_to_the_state_it_reads() {
    let bind = |instance: &str, property: &str, value: &str| {
        format!("__ui.bind({instance}, \"{property}\", function()\n\treturn {value}\nend)")
    };
    let rows: Vec<(&str, String)> = vec![
        (
            "<frame className=\"hover:bg-brand\" />",
            format!(
                "local _1 = __ui.interaction(_0)\nlocal _2 = _0.BackgroundColor3\n{}",
                bind("_0", "BackgroundColor3", &format!("if _1.hover() then {BRAND} else _2"))
            ),
        ),
        (
            "<frame className=\"active:bg-brand\" />",
            bind("_0", "BackgroundColor3", &format!("if _1.press() then {BRAND} else _2")),
        ),
        (
            "<textbox className=\"focus:bg-brand\" />",
            bind("_0", "BackgroundColor3", &format!("if _1.focus() then {BRAND} else _2")),
        ),
        (
            "<frame className=\"bg-white md:bg-brand\" />",
            format!(
                "local _0 = Instance.new(\"Frame\")\n_0.BackgroundTransparency = 0\n{}",
                bind(
                    "_0",
                    "BackgroundColor3",
                    &format!("if __ui.environment.viewport().X >= 768 then {BRAND} else Color3.fromRGB(255, 255, 255)")
                )
            ),
        ),
        (
            "<textlabel className=\"portrait:text-lg\" />",
            format!(
                "local _1 = _0.TextSize\n{}",
                bind("_0", "TextSize", "if __ui.environment.orientation() == \"portrait\" then 18 else _1")
            ),
        ),
        (
            "<textlabel className=\"landscape:text-lg\" />",
            bind("_0", "TextSize", "if __ui.environment.orientation() == \"landscape\" then 18 else _1"),
        ),
        (
            "<frame className=\"touch:bg-brand\" />",
            bind("_0", "BackgroundColor3", &format!("if __ui.environment.input() == \"touch\" then {BRAND} else _1")),
        ),
        (
            "<frame className=\"mouse:bg-brand\" />",
            bind("_0", "BackgroundColor3", &format!("if __ui.environment.input() == \"mouse\" then {BRAND} else _1")),
        ),
        (
            "<frame className=\"gamepad:bg-brand\" />",
            bind("_0", "BackgroundColor3", &format!("if __ui.environment.input() == \"gamepad\" then {BRAND} else _1")),
        ),
        (
            "<frame className=\"dark:bg-brand\" />",
            bind("_0", "BackgroundColor3", &format!("if __ui.environment.dark() then {BRAND} else _1")),
        ),
        (
            "<frame className=\"group\"><frame className=\"group-hover:bg-brand\" /></frame>",
            format!(
                "local _1 = __ui.interaction(_0)\nlocal _2 = Instance.new(\"Frame\")\nlocal _3 = _2.BackgroundColor3\n{}",
                bind("_2", "BackgroundColor3", &format!("if _1.hover() then {BRAND} else _3"))
            ),
        ),
        (
            "<frame className=\"group/card\"><frame className=\"group\"><frame className=\"group-active/card:bg-brand\" /></frame></frame>",
            format!(
                "local _3 = __ui.interaction(_2)\nlocal _4 = Instance.new(\"Frame\")\nlocal _5 = _4.BackgroundColor3\n{}",
                bind("_4", "BackgroundColor3", &format!("if _1.press() then {BRAND} else _5"))
            ),
        ),
        (
            "<frame className=\"md:hover:bg-brand\" />",
            bind(
                "_0",
                "BackgroundColor3",
                &format!("if _1.hover() and __ui.environment.viewport().X >= 768 then {BRAND} else _2"),
            ),
        ),
        (
            "<frame className=\"w-full h-4 hover:w-1/2\" />",
            bind("_0", "Size", "UDim2.new(if _1.hover() then UDim.new(0.5, 0) else UDim.new(1, 0), UDim.new(0, 16))"),
        ),
        (
            "<frame className=\"md:w-1/2\" />",
            format!(
                "local _1 = _0.Size\n{}",
                bind(
                    "_0",
                    "Size",
                    "UDim2.new(if __ui.environment.viewport().X >= 768 then UDim.new(0.5, 0) else _1.X, _1.Y)"
                )
            ),
        ),
        (
            "<frame className=\"hover:rounded-md\" />",
            format!(
                "local _2 = Instance.new(\"UICorner\")\nlocal _3 = _2.CornerRadius\n{}\n{}",
                bind("_2", "CornerRadius", "if _1.hover() then UDim.new(0, 6) else _3"),
                bind("_2", "Parent", "if _1.hover() then _0 else nil")
            ),
        ),
        (
            "<frame className=\"border-white focus:border-2\" />",
            format!(
                "_2.Color = Color3.fromRGB(255, 255, 255)\n_2.Transparency = 0\n{}\n_2.Parent = _0",
                bind("_2", "Thickness", "if _1.focus() then 2 else 1")
            ),
        ),
    ];
    let g = game();
    for (i, (element, _)) in rows.iter().enumerate() {
        g.write(&format!("src/client/v{i}.tsx"), &format!("export const view = {element};\n"));
    }
    let modules = g.compile();
    for (i, (element, expected)) in rows.iter().enumerate() {
        let path = format!("client/v{i}.luau");
        let source = &modules.iter().find(|m| m.path == path).unwrap_or_else(|| panic!("{path} is emitted")).source;
        assert!(source.contains(expected.as_str()), "{element}\n--- expected\n{expected}\n--- in\n{source}");
    }
}

#[test]
fn bad_variants_report_cru028_at_the_token() {
    let g = game();
    g.write("src/client/bad.tsx", include_str!("classes/variants-bad.tsx"));
    let facts = g.facts();
    let found: Vec<String> = facts
        .diagnostics
        .iter()
        .map(|d| format!("{}@{}:{} {}", support::code(d), d.line, d.column, d.message))
        .collect();
    let expected = [
        "CRU028@1:37 CRU028: `group-hover:flex` reads a group, but no element around it in this JSX has class `group` (byte 0)",
        "CRU028@3:20 CRU028: `hovr:` is not a variant. Did you mean `hover:`? (class `hovr:flex`, byte 0)",
        "CRU028@4:20 CRU028: `xl:` is not a variant. Did you mean `lg:`? (class `xl:flex`, byte 0)",
        "CRU028@5:20 CRU028: `hover:` repeats `hover:` (class `hover:hover:flex`, byte 0)",
        "CRU028@6:20 CRU028: `portrait:` and `landscape:` can never hold together (class `portrait:landscape:flex`, byte 0)",
        "CRU028@7:20 CRU028: `touch:` and `gamepad:` can never hold together (class `touch:gamepad:flex`, byte 0)",
        "CRU028@8:20 CRU028: `group-hover:flex` reads a group, but no element around it in this JSX has class `group` (byte 0)",
        "CRU028@10:21 CRU028: `group-hover/card:flex` reads a group, but no element around it in this JSX has class `group/card` (byte 0)",
        "CRU028@12:20 CRU028: `group-hover:flex` reads a group, but no element around it in this JSX has class `group` (byte 0)",
        "CRU028@18:20 CRU028: `hover:bg-brand` never applies: `bg-white` comes later and sets BackgroundColor3",
        "CRU028@19:20 CRU028: `lg:bg-brand` never applies: `md:bg-white` comes later and sets BackgroundColor3",
        "CRU028@20:20 CRU028: `group` takes no variants (class `hover:group`, byte 0)",
    ];
    assert_eq!(found, expected);
}

#[test]
fn a_modifier_conflict_counts_variant_tokens() {
    let g = game();
    g.write("src/client/view.tsx", "export const view = <frame className=\"hover:rounded\"><uicorner /></frame>;");
    assert_eq!(g.found_columns(), ["CRU031@src/client/view.tsx:1:55"]);
}

#[test]
fn each_bad_breakpoint_reports_cru030_at_its_key_or_value() {
    for (entry, column) in [
        ("md = '768'", 6),
        ("md = 767.5", 6),
        ("md = 0", 6),
        ("md = -768", 6),
        ("hover = 768", 1),
        ("group2 = 768", 1),
        ("Md = 768", 1),
    ] {
        let g = game();
        g.write("theme.toml", &format!("spacing = 4\n\n[breakpoint]\n{entry}\n"));
        g.write("src/client/view.tsx", "export const view = <frame className=\"md:flex\" />;");
        assert_eq!(g.found_columns(), [format!("CRU030@theme.toml:4:{column}")], "{entry}");
    }
}

#[test]
fn the_guide_examples_compile_against_the_guide_theme() {
    let guide = include_str!("../../../../docs/ui.md");
    let block = |lang: &str| -> Vec<&str> {
        guide.split(&format!("```{lang}\n")).skip(1).map(|b| b.split("```").next().unwrap()).collect()
    };
    let g = Game::ui();
    g.write("theme.toml", block("toml").first().expect("the guide has a starter theme"));
    let sources = block("tsx");
    assert!(sources.iter().any(|s| s.contains("group-hover:") && s.contains("md:")), "{sources:?}");
    for (i, source) in sources.into_iter().enumerate() {
        g.write(&format!("src/client/guide{i}.tsx"), source);
    }
    let facts = g.facts();
    assert!(facts.diagnostics.is_empty(), "{}", support::lines(&facts.diagnostics));
    g.compile();
}
