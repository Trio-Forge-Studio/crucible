use super::*;

fn catalog() -> Catalog {
    theme::parse(Path::new("theme.toml"), "spacing = 4\n[color]\nwhite = '#ffffff'\n[color.slate]\n800 = '#1e293b'\n[text]\nlg = 18.5\n[radius]\nDEFAULT = 4\nmd = 6\n[breakpoint]\nsm = 640\nmd = 768\nlg = 1024\n").unwrap()
}
fn style(c: &Catalog, tokens: &[&str]) -> Style {
    Style::new(tokens.iter().map(|t| c.parse(t).unwrap()), &BTreeSet::new())
}
#[test]
fn theme_discovery_distinguishes_absent_and_unreadable_files() {
    let dir = std::env::temp_dir().join(format!("crucible-theme-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    assert!(Catalog::load(&dir).unwrap().lookup("bg-white").unwrap_err().contains("No theme.toml"));
    std::fs::create_dir(dir.join("theme.toml")).unwrap();
    let e = Catalog::load(&dir).err().unwrap();
    assert_eq!((e.line, e.column), (1, 1));
    assert!(e.path.ends_with("/theme.toml"));
    std::fs::remove_dir(dir.join("theme.toml")).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
#[test]
fn every_family_has_typed_writes() {
    use Value::*;
    let c = catalog();
    for (token, expected) in [
        ("bg-slate-800", vec![BackgroundColor3(Rgb(30, 41, 59)), BackgroundTransparency(0.0)]),
        ("bg-slate-800/90", vec![BackgroundColor3(Rgb(30, 41, 59)), BackgroundTransparency(0.1)]),
        ("text-white/50", vec![TextColor3(Rgb(255, 255, 255)), TextTransparency(0.5)]),
        ("text-white", vec![TextColor3(Rgb(255, 255, 255)), TextTransparency(0.0)]),
        ("border-white/90", vec![StrokeColor(Rgb(255, 255, 255)), StrokeTransparency(0.1)]),
        ("border-white", vec![StrokeColor(Rgb(255, 255, 255)), StrokeTransparency(0.0)]),
        ("bg-transparent", vec![BackgroundTransparency(1.0)]),
        ("text-lg", vec![TextSize(18.5)]),
        ("text-left", vec![TextXAlignment(Alignment::Left)]),
        ("text-center", vec![TextXAlignment(Alignment::Center)]),
        ("text-right", vec![TextXAlignment(Alignment::Right)]),
        ("rounded", vec![CornerRadius(Offset::Pixels(4))]),
        ("rounded-md", vec![CornerRadius(Offset::Pixels(6))]),
        ("rounded-full", vec![CornerRadius(Offset::Scale(1.0))]),
        ("opacity-90", vec![GroupTransparency(0.1)]),
        ("border", vec![StrokeThickness(1.0)]),
        ("border-2", vec![StrokeThickness(2.0)]),
        ("flex", vec![ListDirection(Direction::Horizontal)]),
        ("flex-col", vec![ListDirection(Direction::Vertical)]),
        ("gap-2", vec![ListPadding(8)]),
        ("p-2", vec![Padding(Side::Top, 8), Padding(Side::Right, 8), Padding(Side::Bottom, 8), Padding(Side::Left, 8)]),
        ("px-2", vec![Padding(Side::Left, 8), Padding(Side::Right, 8)]),
        ("py-2", vec![Padding(Side::Top, 8), Padding(Side::Bottom, 8)]),
        ("pt-0.5", vec![Padding(Side::Top, 2)]),
        ("pr-2", vec![Padding(Side::Right, 8)]),
        ("pb-2", vec![Padding(Side::Bottom, 8)]),
        ("pl-2", vec![Padding(Side::Left, 8)]),
    ] {
        assert_eq!(c.lookup(token).unwrap(), expected, "{token}");
    }
    for (prefix, axes) in [("w", vec![Axis::X]), ("h", vec![Axis::Y]), ("size", vec![Axis::X, Axis::Y])] {
        for (suffix, offset) in [
            ("2", Offset::Pixels(8)),
            ("0.5", Offset::Pixels(2)),
            ("px", Offset::Pixels(1)),
            ("full", Offset::Scale(1.0)),
            ("1/2", Offset::Scale(0.5)),
        ] {
            let expected: Vec<_> = axes.iter().map(|axis| Value::Size(*axis, offset)).collect();
            assert_eq!(c.lookup(&format!("{prefix}-{suffix}")).unwrap(), expected);
        }
    }
}
#[test]
fn ordered_lanes_merge_and_explicit_properties_own_their_lanes() {
    let c = catalog();
    let s = style(&c, &["size-4", "w-full", "p-4", "px-2"]);
    assert_eq!(s.series[&Target::Size(Axis::X)].len(), 2);
    let fixed = s.fixed();
    assert_eq!(fixed[&Target::Size(Axis::X)], Value::Size(Axis::X, Offset::Scale(1.0)));
    assert_eq!(fixed[&Target::Size(Axis::Y)], Value::Size(Axis::Y, Offset::Pixels(16)));
    assert_eq!(fixed[&Target::Padding(Side::Left)], Value::Padding(Side::Left, 8));
    assert!(Style::new([c.parse("size-4").unwrap()], &BTreeSet::from(["Size".into()])).fixed().is_empty());
    assert_eq!(style(&c, &["border-white"]).fixed()[&Target::StrokeThickness], Value::StrokeThickness(1.0));
    assert_eq!(style(&c, &["gap-2"]).fixed()[&Target::ListDirection], Value::ListDirection(Direction::Horizontal));
}
#[test]
fn invalid_tokens_explain_their_failure() {
    let c = catalog();
    for (token, message) in [
        ("bg-white/101", "0 to 100"),
        ("opacity-0.5", "whole number"),
        ("bg-slate-801", "Did you mean `bg-slate-800`"),
        ("w-1/0", "not a class"),
    ] {
        assert!(c.parse(token).unwrap_err().contains(message), "{token}");
    }
    let c = theme::parse(Path::new("theme.toml"), "spacing = 5").unwrap();
    assert!(c.lookup("p-0.5").unwrap_err().contains("whole-pixel"));
    let c = theme::parse(Path::new("theme.toml"), "spacing = 50").unwrap();
    assert_eq!(c.lookup("pt-1.1").unwrap(), vec![Value::Padding(Side::Top, 55)]);
    let c = theme::parse(Path::new("theme.toml"), "spacing = 100").unwrap();
    assert_eq!(c.lookup("pt-0.07").unwrap(), vec![Value::Padding(Side::Top, 7)]);
    assert!(c.lookup("pt-0.075").unwrap_err().contains("7.5 px"));
    for (spacing, token, px) in [
        ("3", "pt-0.333333333", None),
        ("4", "pt-0.25", Some(1)),
        ("0.1", "pt-10", Some(1)),
        ("0.1", "pt-0.5", None),
        ("1", "pt-2147483647", Some(i32::MAX)),
        ("1", "pt-2147483648", None),
        ("1", "pt-2147483647.5", None),
        ("4", "pt-0", Some(0)),
    ] {
        let c = theme::parse(Path::new("theme.toml"), &format!("spacing = {spacing}")).unwrap();
        match px {
            Some(px) => assert_eq!(c.lookup(token).unwrap(), vec![Value::Padding(Side::Top, px)], "{spacing} {token}"),
            None => assert!(c.lookup(token).unwrap_err().contains("whole-pixel"), "{spacing} {token}"),
        }
    }
    let c = Catalog::empty(false);
    assert!(c.lookup("bg-white").unwrap_err().contains("No theme.toml"));
    for token in ["border", "flex", "w-full", "h-px", "size-1/2"] {
        assert!(c.lookup(token).is_ok());
    }
}
#[test]
fn theme_errors_pin_keys_and_values_to_line_and_column() {
    for (text, line, column) in [
        ("spacing = [", 1, 12),
        ("[color]\nwhite = '#ffffff'\nwhite = '#000000'", 3, 1),
        ("[breakpoints]\nsm = 640", 1, 2),
        ("[breakpoint]\nmd = '768'", 2, 6),
        ("[breakpoint]\nmd = 767.5", 2, 6),
        ("[breakpoint]\nmd = 0", 2, 6),
        ("[breakpoint]\nmd = -768", 2, 6),
        ("[breakpoint]\nmd = 100001", 2, 6),
        ("[breakpoint]\nmd = inf", 2, 6),
        ("[breakpoint]\nhover = 768", 2, 1),
        ("[breakpoint]\ndark = 768", 2, 1),
        ("[breakpoint]\ngroupwide = 768", 2, 1),
        ("[breakpoint]\nMd = 768", 2, 1),
        ("[breakpoint]\nDEFAULT = 768", 2, 1),
        ("spacing = 0", 1, 11),
        ("spacing = inf", 1, 11),
        ("color = 4", 1, 9),
        ("[color]\nWhite = '#ffffff'", 2, 1),
        ("[color]\nwhite = '#fffff'", 2, 9),
        ("[color.white]\nBAD = '#ffffff'", 2, 1),
        ("[color.white.deep]\nx = '#ffffff'", 1, 14),
        ("[text]\nsm = 0", 2, 6),
        ("[text]\nsm = 101", 2, 6),
        ("[text]\nsm = '12'", 2, 6),
        ("[text]\nDEFAULT = 12", 2, 1),
        ("[radius]\nsm = -1", 2, 6),
        ("[radius]\nsm = 0.5", 2, 6),
        ("[color]\ntransparent = '#ffffff'", 2, 1),
        ("[text]\nleft = 12", 2, 1),
        ("[radius]\nfull = 4", 2, 1),
        ("[color]\nlabel = '#ffffff'\n[text]\nlabel = 12", 4, 1),
    ] {
        let e = theme::parse(Path::new("theme.toml"), text).err().expect(text);
        assert_eq!((e.line, e.column), (line, column), "{text}: {}", e.message);
    }
}

#[test]
fn class_keywords_and_diagnostics_are_documented() {
    let docs = include_str!("../../../../../docs/ui.md");
    for token in Catalog::empty(false).named.keys() {
        assert!(docs.contains(token), "{token}");
    }
    for prefix in [
        "bg-", "text-", "w-", "h-", "size-", "opacity-", "rounded-", "p-", "px-", "py-", "pt-", "pr-", "pb-", "pl-",
        "border-", "gap-",
    ] {
        assert!(docs.contains(prefix), "{prefix}");
    }
    for code in ["CRU028", "CRU029", "CRU030", "CRU031"] {
        assert!(docs.contains(code));
    }
    for variant in VARIANTS {
        assert!(docs.contains(&format!("`{variant}:`")), "{variant}");
    }
    assert!(docs.contains("[breakpoint]") && docs.contains("`group/<name>`"));
}

#[test]
fn every_variant_parses_to_its_condition_and_stacks_sorted() {
    use Condition::*;
    let c = catalog();
    let bp = |name: &str, px| Breakpoint(name.into(), px);
    for (token, expected) in [
        ("hover:flex", vec![State(Interaction::Hover)]),
        ("active:flex", vec![State(Interaction::Press)]),
        ("focus:flex", vec![State(Interaction::Focus)]),
        ("group-hover:flex", vec![Group(None, Interaction::Hover)]),
        ("group-active:flex", vec![Group(None, Interaction::Press)]),
        ("group-focus:flex", vec![Group(None, Interaction::Focus)]),
        ("group-hover/card:flex", vec![Group(Some("card".into()), Interaction::Hover)]),
        ("sm:flex", vec![bp("sm", 640)]),
        ("md:flex", vec![bp("md", 768)]),
        ("lg:flex", vec![bp("lg", 1024)]),
        ("portrait:flex", vec![Orientation(super::Orientation::Portrait)]),
        ("landscape:flex", vec![Orientation(super::Orientation::Landscape)]),
        ("touch:flex", vec![Input(InputMode::Touch)]),
        ("mouse:flex", vec![Input(InputMode::Mouse)]),
        ("gamepad:flex", vec![Input(InputMode::Gamepad)]),
        ("dark:flex", vec![Dark]),
        ("md:hover:flex", vec![State(Interaction::Hover), bp("md", 768)]),
        ("dark:hover:md:flex", vec![State(Interaction::Hover), bp("md", 768), Dark]),
        ("sm:lg:flex", vec![bp("lg", 1024), bp("sm", 640)]),
    ] {
        assert_eq!(
            c.parse(token).unwrap(),
            Class::Writes(expected, vec![Value::ListDirection(Direction::Horizontal)]),
            "{token}"
        );
    }
    assert_eq!(c.parse("group").unwrap(), Class::Group(None));
    assert_eq!(c.parse("group/card").unwrap(), Class::Group(Some("card".into())));
    assert_eq!(c.parse("bg-white").unwrap(), Class::Writes(vec![], c.lookup("bg-white").unwrap()));
}

#[test]
fn bad_variants_explain_their_failure() {
    let c = catalog();
    for (token, message) in [
        ("hovr:flex", "`hovr:` is not a variant. Did you mean `hover:`?"),
        ("xl:flex", "`xl:` is not a variant"),
        ("mdd:flex", "Did you mean `md:`?"),
        ("hover:hover:flex", "`hover:` repeats `hover:`"),
        ("md:hover:md:flex", "`md:` repeats `md:`"),
        ("portrait:landscape:flex", "`portrait:` and `landscape:` can never hold together"),
        ("touch:mouse:flex", "`touch:` and `mouse:` can never hold together"),
        ("gamepad:md:touch:flex", "`gamepad:` and `touch:` can never hold together"),
        ("group-hover/Card:flex", "not a variant"),
        ("group-hovr:flex", "Did you mean `group-hover:`?"),
        (":flex", "`:` is not a variant"),
        ("hover:group", "`group` takes no variants"),
        ("hover:bg-whit", "Did you mean `bg-white`?"),
        ("group/Card", "not a class"),
    ] {
        let error = c.parse(token).unwrap_err();
        assert!(error.contains(message), "{token}: {error}");
    }
    let bare = Catalog::empty(false);
    assert!(bare.parse("md:flex").unwrap_err().contains("No theme.toml"));
    assert!(bare.parse("hover:flex").is_ok());
}

#[test]
fn a_breakpoint_implies_every_narrower_one_and_nothing_else() {
    use Condition::*;
    let (sm, md, lg) = (Breakpoint("sm".into(), 640), Breakpoint("md".into(), 768), Breakpoint("lg".into(), 1024));
    assert!(implies(&lg, &md) && implies(&lg, &sm) && implies(&md, &md));
    assert!(!implies(&sm, &md) && !implies(&md, &lg));
    assert!(implies(&Breakpoint("tablet".into(), 768), &md));
    assert!(implies(&Dark, &Dark) && !implies(&Dark, &State(Interaction::Hover)));
    assert!(!implies(&Group(None, Interaction::Hover), &Group(Some("card".into()), Interaction::Hover)));
    assert!(!implies(&State(Interaction::Hover), &Group(None, Interaction::Hover)));
}

#[test]
fn a_conditional_rule_that_a_later_rule_always_overrides_is_dead() {
    let c = catalog();
    let dead = |tokens: &[&str]| -> Vec<(usize, Target, usize)> {
        style(&c, tokens).dead().into_iter().map(|d| (d.token, d.target, d.by)).collect()
    };
    assert_eq!(dead(&["hover:bg-white", "bg-slate-800"]), [(0, Target::BackgroundColor3, 1)]);
    assert_eq!(dead(&["md:hover:bg-white", "hover:bg-slate-800"]), [(0, Target::BackgroundColor3, 1)]);
    assert_eq!(dead(&["lg:bg-white", "md:bg-slate-800"]), [(0, Target::BackgroundColor3, 1)]);
    assert_eq!(dead(&["hover:size-4", "w-8"]), [(0, Target::Size(Axis::X), 1)]);
    assert_eq!(dead(&["hover:border-2", "border"]), [(0, Target::StrokeThickness, 1)]);
    for alive in [
        &["bg-white", "hover:bg-slate-800"][..],
        &["md:bg-white", "lg:bg-slate-800"],
        &["hover:bg-white", "md:hover:bg-slate-800"],
        &["hover:bg-white", "focus:bg-slate-800"],
        &["hover:bg-white", "group-hover:bg-slate-800"],
        &["bg-transparent", "bg-white"],
        &["hover:w-4", "h-8"],
        &["group", "hover:bg-white"],
    ] {
        assert_eq!(dead(alive), [], "{alive:?}");
    }
}

#[test]
fn a_varying_target_picks_its_last_holding_rule_over_its_base() {
    use Condition::*;
    let c = catalog();
    let s = style(&c, &["bg-slate-800", "hover:bg-white", "md:hover:bg-white/50", "w-full", "md:w-1/2", "p-2"]);
    let fixed = s.fixed();
    assert!(!fixed.contains_key(&Target::BackgroundColor3) && !fixed.contains_key(&Target::Size(Axis::X)));
    assert_eq!(fixed[&Target::Padding(Side::Top)], Value::Padding(Side::Top, 8));
    let variants = s.variants();
    let color = &variants[&Target::BackgroundColor3];
    assert_eq!(color.fallback, Some(Value::BackgroundColor3(Rgb(30, 41, 59))));
    assert_eq!(
        color.cases,
        [
            (
                vec![State(Interaction::Hover), Breakpoint("md".into(), 768)],
                Value::BackgroundColor3(Rgb(255, 255, 255))
            ),
            (vec![State(Interaction::Hover)], Value::BackgroundColor3(Rgb(255, 255, 255))),
        ]
    );
    assert_eq!(variants[&Target::BackgroundTransparency].cases[0].1, Value::BackgroundTransparency(0.5));
    assert_eq!(variants[&Target::Size(Axis::X)].fallback, Some(Value::Size(Axis::X, Offset::Scale(1.0))));
    assert!(!variants.contains_key(&Target::Size(Axis::Y)));
    assert!(s.interacts());
    assert!(!style(&c, &["md:bg-white", "group-hover:bg-slate-800"]).interacts());
    let same = style(&c, &["bg-white", "hover:bg-slate-800", "focus:bg-white/50"]);
    assert_eq!(same.fixed().get(&Target::BackgroundTransparency), None);
    assert_eq!(same.variants()[&Target::BackgroundTransparency].cases.len(), 1);
    let same = style(&c, &["bg-white", "hover:bg-slate-800"]);
    assert_eq!(same.fixed()[&Target::BackgroundTransparency], Value::BackgroundTransparency(0.0));
}

#[test]
fn a_modifier_that_only_variants_create_is_parented_while_one_holds() {
    use Condition::*;
    let c = catalog();
    let s = style(&c, &["hover:border-white", "focus:border-2", "rounded-md", "dark:p-2"]);
    let modifiers = s.modifiers();
    assert_eq!(modifiers[&Modifier::Corner], None);
    assert_eq!(
        modifiers[&Modifier::Stroke],
        Some(vec![vec![State(Interaction::Hover)], vec![State(Interaction::Focus)]])
    );
    assert_eq!(modifiers[&Modifier::Padding], Some(vec![vec![Dark]]));
    let fixed = s.fixed();
    assert_eq!(fixed[&Target::StrokeMode], Value::StrokeMode);
    assert!(!fixed.contains_key(&Target::StrokeThickness));
    let variants = s.variants();
    assert_eq!(variants[&Target::StrokeThickness].fallback, Some(Value::StrokeThickness(1.0)));
    assert_eq!(variants[&Target::StrokeColor].fallback, Some(Value::StrokeColor(Rgb(0, 0, 0))));
    assert_eq!(variants[&Target::StrokeTransparency].fallback, None);
    assert_eq!(variants[&Target::Padding(Side::Top)].fallback, None);
    let s = style(&c, &["group/card", "group", "flex"]);
    assert_eq!(s.groups(), &BTreeSet::from([None, Some("card".into())]));
    assert_eq!(s.modifiers()[&Modifier::List], None);
}
