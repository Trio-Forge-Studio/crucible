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

#[test]
fn explicit_boolean_coercion_and_discarded_steps_are_safe() {
    let game = Game::new();
    game.main(
        r#"
export function probe(value: unknown, array: unknown[], object: Record<string, unknown>) {
    const boolean = !!value;
    let count = 0;
    for (const [key] of pairs(array)) {
        if (!typeIs(key, "number") || key < 1 || key > array.size()) return false;
        count++;
    }
    for (const _ of pairs(object)) count++;
    const components = new CFrame().GetComponents();
    for (const value of new CFrame().GetComponents()) count += value;
    return boolean;
}
"#,
    );
    let found = rules(&game);
    assert_eq!(found.iter().map(|&(l, _, c)| (l, c)).collect::<Vec<_>>(), [(5, 90034), (10, 90033)]);
}

#[test]
fn inspected_keys_still_cannot_index_arrays_and_discard_names_cannot_be_read() {
    let game = Game::new();
    game.main(
        r#"
export function probe(array: unknown[], object: Record<string, unknown>) {
    for (const [key] of pairs(array)) {
        if (typeIs(key, "number")) array[key] = 1;
    }
    for (const _ of pairs(object)) print(_);
}
"#,
    );
    assert_eq!(rules(&game).iter().map(|&(l, _, c)| (l, c)).collect::<Vec<_>>(), [(3, 90034), (6, 90033)]);
}

#[test]
fn suppression_applies_only_to_the_immediately_following_line() {
    let game = Game::new();
    game.main(
        r#"
declare const value: number;
// oxlint-disable-next-line roblox-ts/lua-truthiness

if (value) {}
"#,
    );
    assert_eq!(rules(&game), [(5, 5, 90032)]);
}

#[test]
fn raw_array_keys_report_comparisons_escaped_indices_and_aliases() {
    let game = Game::new();
    game.main(
        r#"export function sawIndexZero(array: string[]) {
	let found = false;
	for (const [key] of pairs(array)) {
		if (typeIs(key, "number") && key === 0) found = true;
	}
	return found;
}
export function lastByKey(array: string[]) {
	let last = -1;
	for (const [key] of pairs(array)) {
		if (typeIs(key, "number") && key > last) last = key;
	}
	return array[last];
}
export function aliasIndex(array: string[]) {
	let out = "";
	for (const [key] of pairs(array)) {
		if (!typeIs(key, "number")) continue;
		const i = key;
		out += array[i];
	}
	return out;
}
"#,
    );
    assert_eq!(rules(&game), [(3, 22, 90034), (10, 22, 90034), (17, 22, 90034)]);
}
