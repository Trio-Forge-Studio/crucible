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
