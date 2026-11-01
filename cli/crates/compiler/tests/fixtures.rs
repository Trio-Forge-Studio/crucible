use std::path::PathBuf;
use std::sync::LazyLock;

use crucible_compiler::{
    Callee, Constant, Crucible, Dependency, Fact, Facts, Falsy, Import, Macro, Reason, Receiver, Site, Span,
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

static FACTS: LazyLock<Facts> = LazyLock::new(|| {
    let tsconfig = fixtures().join("tsconfig.json");
    crucible_compiler::analyze(&tsconfig).unwrap_or_else(|diagnostics| {
        let listed: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
        panic!("the fixtures check without diagnostics (run pnpm build first):\n{}", listed.join("\n"))
    })
});

fn source(file: &str) -> String {
    std::fs::read_to_string(fixtures().join("src").join(file)).expect("the fixture exists")
}

/// The span of `part` inside the only occurrence of `context` in `file`.
fn span(file: &str, context: &str, part: &str) -> Span {
    let text = source(file);
    let at = text.find(context).unwrap_or_else(|| panic!("{file} contains `{context}`"));
    assert_eq!(text.matches(context).count(), 1, "`{context}` occurs once in {file}");
    let offset = context.find(part).unwrap_or_else(|| panic!("`{context}` contains `{part}`"));
    let start = (at + offset) as u32;
    Span { start, end: start + part.len() as u32 }
}

fn lookup(file: &str, span: Span) -> Option<Fact> {
    let facts = FACTS
        .files
        .iter()
        .find(|f| f.path.ends_with(&format!("/src/{file}")))
        .unwrap_or_else(|| panic!("{file} has facts"));
    facts.get(span).cloned()
}

fn fact(file: &str, context: &str, part: &str) -> Fact {
    lookup(file, span(file, context, part)).unwrap_or_else(|| panic!("`{part}` in {file} has a fact"))
}

#[test]
fn array_push_is_an_array_macro_call() {
    let callee = fact("arrays.ts", "numbers.push(1)", "numbers.push");
    assert_eq!(callee.callee, Some(Callee::Macro(Macro::ArrayPush)), "push resolves to Array.push");
    let receiver = fact("arrays.ts", "numbers.push(1)", "numbers");
    assert_eq!(receiver.receiver, Some(Receiver::Array), "the receiver is an array");
    let size = fact("arrays.ts", "numbers.size()", "numbers.size");
    assert_eq!(size.callee, Some(Callee::Macro(Macro::ArrayLikeSize)), "size resolves to ArrayLike.size");
}

#[test]
fn plus_on_strings_is_concatenation() {
    for operand in ["greeting", "name"] {
        let fact = fact("strings.ts", "greeting + name", operand);
        assert_eq!(fact.receiver, Some(Receiver::String), "`{operand}` is a string");
        assert!(!fact.nullable(), "`{operand}` is never nil");
    }
    for operand in ["1", "2"] {
        let fact = fact("strings.ts", "1 + 2", operand);
        assert_eq!(fact.receiver, Some(Receiver::Other), "`{operand}` is a number, so `+` adds");
    }
}

#[test]
fn type_is_matches_the_macro_by_symbol_identity() {
    let global = fact("macros.ts", "typeIs(value, \"number\")", "typeIs");
    assert_eq!(global.callee, Some(Callee::Macro(Macro::TypeIs)), "the global typeIs is the macro");
    let local = fact("shadow.ts", "typeIs(1, \"number\")", "typeIs");
    assert_eq!(local.callee, Some(Callee::Function), "a local function named typeIs is not");
}

#[test]
fn enum_members_and_const_enum_references_carry_constant_values() {
    let cases = [
        (fact("enums.ts", "Green = 4", "Green = 4"), Constant::Number(4.0)),
        (fact("enums.ts", "Blue,", "Blue"), Constant::Number(5.0)),
        (fact("enums.ts", "Warm = \"warm\"", "Warm = \"warm\""), Constant::String("warm".into())),
        (fact("imports.ts", "Mode.Slow", "Mode.Slow"), Constant::Number(1.0)),
    ];
    for (fact, expected) in cases {
        assert_eq!(fact.constant, Some(expected), "the enum value is known at compile time");
    }
    let regular = lookup("enums.ts", span("enums.ts", "green = Color.Green", "Color.Green"));
    assert!(regular.is_none_or(|f| f.constant.is_none()), "a regular enum reference reads the enum table");
}

#[test]
fn lua_tuple_returns_and_pairs_iteration() {
    let tuple = fact("tuples.ts", "return $tuple(1, \"one\")", "$tuple(1, \"one\")");
    assert_eq!(tuple.receiver, Some(Receiver::LuaTuple), "the returned value is a LuaTuple");
    let macro_call = fact("tuples.ts", "return $tuple(1, \"one\")", "$tuple");
    assert_eq!(macro_call.callee, Some(Callee::Macro(Macro::Tuple)), "$tuple is the tuple macro");
    let destructured = fact("tuples.ts", "= pair();", "pair()");
    assert_eq!(destructured.receiver, Some(Receiver::LuaTuple), "a LuaTuple call result destructures into values");
    let iterable = fact("tuples.ts", "pairs(new Map<string, number>())", "pairs(new Map<string, number>())");
    assert_eq!(iterable.receiver, Some(Receiver::IterFn), "pairs returns an iterator function");
    let binding = fact("tuples.ts", "const [key, value] of", "[key, value]");
    assert_eq!(binding.receiver, Some(Receiver::LuaTuple), "each step of pairs yields a LuaTuple");
    let map = fact("tuples.ts", "new Map<string, number>()", "Map");
    assert_eq!(map.callee, Some(Callee::Macro(Macro::NewMap)), "new Map is the Map constructor macro");
}

#[test]
fn truthiness_checks_know_the_javascript_falsy_values() {
    let cases = [
        ("if (count)", "count", Falsy::ZERO | Falsy::NAN),
        ("if (text)", "text", Falsy::NIL | Falsy::EMPTY_STRING),
        ("if (part)", "part", Falsy::NONE),
    ];
    for (context, part, expected) in cases {
        assert_eq!(fact("truthiness.ts", context, part).falsy, Some(expected), "falsy values of `{part}`");
    }
    let coalesce = fact("truthiness.ts", "flag ?? true", "flag ?? true");
    assert!(coalesce.falsy.is_some_and(|f| f.contains(Falsy::FALSE)), "`??` sees that the result may be false");
}

#[test]
fn methods_take_self_and_functions_do_not() {
    let call = fact("methods.ts", "counter.increment();", "counter.increment");
    assert_eq!(call.callee, Some(Callee::Method), "an object method is called with `:`");
    let declaration =
        fact("methods.ts", "increment() {\n\t\tthis.value += 1;\n\t}", "increment() {\n\t\tthis.value += 1;\n\t}");
    assert_eq!(declaration.callee, Some(Callee::Method), "an object method takes self");
    let arrow = fact("methods.ts", "(message: string) => print(message)", "(message: string) => print(message)");
    assert_eq!(arrow.callee, Some(Callee::Function), "an arrow function takes no self");
    let log = fact("methods.ts", "log(\"done\")", "log");
    assert_eq!(log.callee, Some(Callee::Function), "an arrow function is called with `.`");
}

#[test]
fn imports_used_only_as_types_are_elided() {
    let import = |name| fact("imports.ts", "{ Mode, origin, Shape, Unused }", name).import;
    assert_eq!(import("origin"), Some(Import::Keep), "a value import stays");
    assert_eq!(import("Shape"), Some(Import::Elide), "an interface import goes");
    assert_eq!(
        import("Mode"),
        Some(Import::Keep),
        "a const enum read as a value stays, because it compiles as an enum"
    );
    assert_eq!(import("Unused"), Some(Import::Elide), "a const enum used only as a type goes");
}

#[test]
fn inject_carries_its_target_type_id() {
    let call = fact("crucible.ts", "inject<Store>()", "inject<Store>()");
    let Some(crucible) = call.crucible else { panic!("inject has a Crucible fact") };
    let Crucible::Inject(Dependency::One(target)) = *crucible else {
        panic!("inject resolves one target, got {crucible:?}")
    };
    assert!(target.ends_with("/src/crucible:Store"), "the target id names the Store class, got {target}");
    let callee = fact("crucible.ts", "inject<Store>()", "inject");
    assert_eq!(callee.callee, Some(Callee::Macro(Macro::Inject)), "inject is Crucible's macro");
}

#[test]
fn unanswerable_sites_are_reported() {
    let gap = span("gaps.ts", "mixed[0]", "mixed");
    let unanswered: Vec<_> =
        FACTS.unanswered.iter().map(|u| (u.path.rsplit('/').next().unwrap(), u.span, u.site, &u.reason)).collect();
    assert_eq!(
        unanswered,
        [("gaps.ts", gap, Site::ElementReceiver, &Reason::Mixed)],
        "only indexing an array-or-record union is unanswered"
    );
}

#[test]
fn type_errors_stop_the_build() {
    let diagnostics =
        crucible_compiler::analyze(&fixtures().join("broken/tsconfig.json")).expect_err("the check fails");
    assert_eq!(diagnostics.len(), 1, "one error: {diagnostics:?}");
    let error = &diagnostics[0];
    assert_eq!((error.code, error.line, error.column), (2322, 1, 14), "TS2322 at the declaration: {error}");
    assert!(error.path.as_deref().is_some_and(|p| p.ends_with("/broken/error.ts")), "the error has its file: {error}");
    assert_eq!(error.message, "Type 'string' is not assignable to type 'number'.", "the message is formatted");
}
