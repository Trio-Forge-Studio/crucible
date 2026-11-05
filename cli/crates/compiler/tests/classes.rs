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
