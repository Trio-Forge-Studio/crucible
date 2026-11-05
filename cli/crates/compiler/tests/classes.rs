mod support;
use support::Game;

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
