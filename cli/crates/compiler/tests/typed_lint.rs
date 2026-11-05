mod support;
use support::Game;

fn rules(game: &Game) -> Vec<(usize, usize, i32)> {
    game.facts()
        .diagnostics
        .iter()
        .filter(|d| (90032..=90037).contains(&d.code))
        .map(|d| (d.line as usize, d.column as usize, d.code))
        .collect()
}
#[test]
fn fd_typed_rules_report_lua_truthiness_misleading_tuples_and_array_pairs() {
    let game = Game::new();
    game.main(include_str!("fixtures/typed-lint/typed.ts.txt"));
    assert_eq!(
        rules(&game),
        vec![
            (4, 6, 90032),
            (7, 6, 90032),
            (8, 20, 90032),
            (9, 15, 90032),
            (10, 6, 90033),
            (11, 8, 90033),
            (13, 13, 90033),
            (15, 22, 90034),
            (16, 22, 90034)
        ]
    );
}
#[test]
fn fd_crucible_lifetime_rules_follow_abstract_bases() {
    let game = Game::new();
    let source = include_str!("fixtures/typed-lint/classes.ts.txt");
    game.write("src/server/bases.ts", include_str!("fixtures/typed-lint/bases.ts.txt"));
    game.main(source);
    let expected: Vec<_> = source
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            l.split_once("// fires ").map(|(_, rule)| (i + 1, if rule == "frame-hooks" { 90035 } else { 90036 }))
        })
        .collect();
    assert_eq!(rules(&game).iter().map(|&(l, _, c)| (l, c)).collect::<Vec<_>>(), expected);
}
#[test]
fn fd_state_passed_where_plain_function_expected() {
    let game = Game::new();
    let source = include_str!("fixtures/typed-lint/state.ts.txt");
    game.main(source);
    let expected: Vec<_> =
        source.lines().enumerate().filter_map(|(i, l)| l.ends_with("// fires").then_some((i + 1, 90037))).collect();
    assert_eq!(rules(&game).iter().map(|&(l, _, c)| (l, c)).collect::<Vec<_>>(), expected);
}
