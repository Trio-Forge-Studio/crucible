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

fn value(receiver: Receiver, falsy: Falsy) -> Fact {
    Fact { receiver: Some(receiver), falsy: Some(falsy), ..Fact::default() }
}

fn callee(callee: Callee) -> Fact {
    Fact { callee: Some(callee), ..Fact::default() }
}

#[test]
fn array_push_is_an_array_macro_call() {
    let push = fact("arrays.ts", "numbers.push(1)", "numbers.push");
    assert_eq!(push, callee(Callee::Macro(Macro::ArrayPush)), "push resolves to Array.push");
    let receiver = fact("arrays.ts", "numbers.push(1)", "numbers");
    assert_eq!(receiver, value(Receiver::Array, Falsy::NONE), "the receiver is a non-nil array");
    let size = fact("arrays.ts", "numbers.size()", "numbers.size");
    assert_eq!(size, callee(Callee::Macro(Macro::ArrayLikeSize)), "size resolves to ArrayLike.size");
}

#[test]
fn plus_on_strings_is_concatenation() {
    let greeting = fact("strings.ts", "greeting + name", "greeting");
    assert_eq!(greeting, value(Receiver::String, Falsy::NONE), "the literal \"hello\" is never empty or nil");
    let name = fact("strings.ts", "greeting + name", "name");
    assert_eq!(name, value(Receiver::String, Falsy::EMPTY_STRING), "a string may be empty but never nil");
    for operand in ["1", "2"] {
        let fact = fact("strings.ts", "1 + 2", operand);
        assert_eq!(fact, value(Receiver::Other, Falsy::NONE), "`{operand}` is a number, so `+` adds");
    }
}

#[test]
fn type_is_matches_the_macro_by_symbol_identity() {
    let global = fact("macros.ts", "typeIs(value, \"number\")", "typeIs");
    assert_eq!(global, callee(Callee::Macro(Macro::TypeIs)), "the global typeIs is the macro");
    let local = fact("shadow.ts", "typeIs(1, \"number\")", "typeIs");
    assert_eq!(local, callee(Callee::Function), "a local function named typeIs is not");
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
    assert_eq!(tuple, value(Receiver::LuaTuple, Falsy::NONE), "the returned value is a LuaTuple");
    let macro_call = fact("tuples.ts", "return $tuple(1, \"one\")", "$tuple");
    assert_eq!(macro_call, callee(Callee::Macro(Macro::Tuple)), "$tuple is the tuple macro");
    let call = fact("tuples.ts", "= pair();", "pair()");
    assert_eq!(call, value(Receiver::LuaTuple, Falsy::NONE), "a LuaTuple call result destructures into values");
    let pattern = fact("tuples.ts", "const [first, second]", "[first, second]");
    assert_eq!(pattern, value(Receiver::LuaTuple, Falsy::NONE), "the pattern destructures a LuaTuple");
    let iterable = fact("tuples.ts", "pairs(new Map<string, number>())", "pairs(new Map<string, number>())");
    assert_eq!(iterable, value(Receiver::LuaTupleIterFn, Falsy::NONE), "each step of pairs is a LuaTuple");
    let binding = fact("tuples.ts", "const [key, value] of", "[key, value]");
    assert_eq!(binding, value(Receiver::LuaTuple, Falsy::NONE), "the binding pattern destructures one step");
    let plain = fact("tuples.ts", "of step)", "step");
    assert_eq!(plain, value(Receiver::IterFn, Falsy::NONE), "an IterableFunction<number> yields one value");
    let map = fact("tuples.ts", "new Map<string, number>()", "Map");
    assert_eq!(map, Fact { callee: Some(Callee::Macro(Macro::NewMap)), ..Fact::default() }, "new Map is a macro");
}

#[test]
fn every_array_pattern_destructures_by_its_type() {
    let parameter = fact("patterns.ts", "([name, count]) =>", "[name, count]");
    assert_eq!(parameter, value(Receiver::Array, Falsy::NONE), "a callback parameter pattern");
    let inner = fact("patterns.ts", "const [[digit, letter], flag]", "[digit, letter]");
    assert_eq!(inner, value(Receiver::Array, Falsy::NONE), "a pattern nested in an array pattern");
    let outer = fact("patterns.ts", "const [[digit, letter], flag]", "[[digit, letter], flag]");
    assert_eq!(outer, value(Receiver::Array, Falsy::NONE), "the outer pattern");
    let property = fact("patterns.ts", "pair: [two, why],", "[two, why]");
    assert_eq!(property, value(Receiver::Array, Falsy::NONE), "a pattern nested in an object pattern");
    let assigned = fact("patterns.ts", "[left, right] = [3, \"w\"]", "[left, right]");
    assert_eq!(assigned, value(Receiver::Array, Falsy::NONE), "an assignment pattern takes the assigned type");
    let inline = lookup("patterns.ts", span("patterns.ts", "for ([key, score] of", "[key, score]"));
    assert_eq!(inline, None, "a map loop binds its pattern inline without asking its type");
}

#[test]
fn arrow_expression_bodies_are_returned_values() {
    let body = fact("patterns.ts", "() => tuple", "tuple");
    assert_eq!(body, value(Receiver::LuaTuple, Falsy::NONE), "the arrow returns a LuaTuple");
    let count = fact("patterns.ts", "=> name.size() + count)", "name.size() + count");
    assert_eq!(count, value(Receiver::Other, Falsy::ZERO | Falsy::NAN), "the callback returns a number");
}

#[test]
fn tagged_templates_are_calls() {
    let function = fact("templates.ts", "tag`a${1}b`", "tag");
    assert_eq!(function, callee(Callee::Function), "an arrow function tag is called with `.`");
    let method = fact("templates.ts", "tagger.mark`x`", "tagger.mark");
    assert_eq!(method, callee(Callee::Method), "an object method tag takes self");
}

#[test]
fn spreads_know_whether_they_are_objects() {
    assert_eq!(fact("jsx.tsx", "{...props}", "props").falsy, Some(Falsy::NIL), "a JSX spread that may be nil");
    assert_eq!(fact("jsx.tsx", "{...style}", "style").falsy, Some(Falsy::NONE), "a JSX spread of an object");
}

#[test]
fn roblox_api_calls_know_their_argument_calls() {
    let roblox = fact("roblox.ts", "print(nothing())", "print");
    assert_eq!(roblox, Fact { callee: Some(Callee::Function), roblox_api: Some(true), ..Fact::default() });
    let local = fact("roblox.ts", "log(nothing())", "log");
    assert_eq!(local, Fact { callee: Some(Callee::Function), roblox_api: Some(false), ..Fact::default() });
    let plain = fact("roblox.ts", "print(\"plain\")", "print");
    assert_eq!(plain, callee(Callee::Function), "a call without a call argument does not ask");
    let argument = fact("roblox.ts", "print(nothing())", "nothing()");
    assert_eq!(argument, value(Receiver::Other, Falsy::NIL), "the argument call may return nothing");
}

#[test]
fn numeric_loop_bounds_know_integers() {
    let integer = |context, part| fact("loops.ts", context, part).integer;
    assert_eq!(integer("limit - 1", "limit"), Some(true), "a bound nested in a difference");
    assert_eq!(integer("let i = count", "count"), Some(false), "a number is not definitely an integer");
    assert_eq!(integer("i < sizes", "sizes"), Some(true), "a union of integer literals");
    assert_eq!(lookup("loops.ts", span("loops.ts", "limit - 1", "1")), None, "a literal is an integer by syntax");
}

#[test]
fn truthiness_checks_know_the_javascript_falsy_values() {
    let cases = [
        ("if (count)", "count", Falsy::ZERO | Falsy::NAN),
        ("if (text)", "text", Falsy::NIL | Falsy::EMPTY_STRING),
        ("if (part)", "part", Falsy::NONE),
        ("flag ?? true", "flag ?? true", Falsy::FALSE),
    ];
    for (context, part, expected) in cases {
        assert_eq!(fact("truthiness.ts", context, part).falsy, Some(expected), "falsy values of `{part}`");
    }
}

#[test]
fn methods_take_self_and_functions_do_not() {
    let call = fact("methods.ts", "counter.increment();", "counter.increment");
    assert_eq!(call, callee(Callee::Method), "an object method is called with `:`");
    let declaration =
        fact("methods.ts", "increment() {\n\t\tthis.value += 1;\n\t}", "increment() {\n\t\tthis.value += 1;\n\t}");
    assert_eq!(declaration, callee(Callee::Method), "an object method takes self");
    let arrow = fact("methods.ts", "(message: string) => print(message)", "(message: string) => print(message)");
    assert_eq!(arrow, callee(Callee::Function), "an arrow function takes no self");
    let log = fact("methods.ts", "log(\"done\")", "log");
    assert_eq!(log, callee(Callee::Function), "an arrow function is called with `.`");
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
fn re_exported_const_enums_keep_their_import() {
    let import = |name| fact("reexports.ts", "import { Speed, Unused }", name).import;
    assert_eq!(import("Speed"), Some(Import::Keep), "a re-exported const enum import stays");
    assert_eq!(import("Unused"), Some(Import::Elide), "a type-only re-export is not a value use");
    let export = fact("reexports.ts", "export { Speed }", "Speed").import;
    assert_eq!(export, Some(Import::Keep), "the re-export stays");
}

#[test]
fn inject_carries_its_target_type_id() {
    let store = Box::new(Crucible::Inject(Dependency::One("crucible-workspace/crucible:Store".into())));
    let call = fact("crucible.ts", "inject<Store>()", "inject<Store>()");
    assert_eq!(call.crucible, Some(store.clone()), "the id is relative to the common source directory");
    let macro_call = fact("crucible.ts", "inject<Store>()", "inject");
    assert_eq!(macro_call, callee(Callee::Macro(Macro::Inject)), "inject is Crucible's macro");
    let aliased = fact("crucible.ts", "alias<Store>()", "alias<Store>()");
    assert_eq!(aliased.crucible, Some(store), "an alias of inject resolves through its signature");
    let alias = fact("crucible.ts", "alias<Store>()", "alias");
    assert_eq!(alias, callee(Callee::Macro(Macro::Inject)), "the alias is the inject macro");
    let untyped = fact("crucible.ts", "inject()", "inject()");
    let invalid = Box::new(Crucible::Inject(Dependency::Invalid));
    assert_eq!(untyped.crucible, Some(invalid), "inject without a type argument is rejected");
}

#[test]
fn crucible_macros_come_from_their_entries() {
    let emit = fact("crucible.ts", "emitAttribute({}", "emitAttribute");
    assert_eq!(emit, callee(Callee::Function), "the emit module's attribute is not the macro");
    let boot = fact("crucible.ts", "boot({ load: [] })", "boot");
    assert_eq!(boot, callee(Callee::Macro(Macro::Boot)), "the server entry's boot is the macro");
}

#[test]
fn unanswerable_sites_are_reported() {
    let unanswered: Vec<_> =
        FACTS.unanswered.iter().map(|u| (u.path.rsplit('/').next().unwrap(), u.span, u.site, &u.reason)).collect();
    let gap = |context, part| span("gaps.ts", context, part);
    assert_eq!(
        unanswered,
        [
            ("gaps.ts", gap("mixed[0]", "mixed"), Site::ElementReceiver, &Reason::Mixed),
            ("gaps.ts", gap("for ([left, right]", "[left, right]"), Site::Destructure, &Reason::AssignmentPattern),
            ("gaps.ts", gap("[[left, right]]", "[left, right]"), Site::Destructure, &Reason::AssignmentPattern),
            ("gaps.ts", gap("push(...more)", "...more"), Site::Spread, &Reason::MacroSpread),
        ],
        "indexing an array-or-record union, assignment patterns whose type tsrs hides, and a macro spread"
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
