use super::*;

fn catalog() -> Catalog {
    theme::parse(Path::new("theme.toml"), "spacing = 4\n[color]\nwhite = '#ffffff'\n[color.slate]\n800 = '#1e293b'\n[text]\nlg = 18.5\n[radius]\nDEFAULT = 4\nmd = 6\n").unwrap()
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
    let values = ["size-4", "w-full", "p-4", "px-2"].into_iter().flat_map(|s| c.lookup(s).unwrap());
    let s = Style::new(values, &BTreeSet::new());
    assert_eq!(s.series[&Target::Size(Axis::X)].len(), 2);
    let fixed = s.fixed();
    assert_eq!(fixed[&Target::Size(Axis::X)], Value::Size(Axis::X, Offset::Scale(1.0)));
    assert_eq!(fixed[&Target::Size(Axis::Y)], Value::Size(Axis::Y, Offset::Pixels(16)));
    assert_eq!(fixed[&Target::Padding(Side::Left)], Value::Padding(Side::Left, 8));
    assert!(Style::new(c.lookup("size-4").unwrap(), &BTreeSet::from(["Size".into()])).fixed().is_empty());
    assert_eq!(
        Style::new(c.lookup("border-white").unwrap(), &BTreeSet::new()).fixed()[&Target::StrokeThickness],
        Value::StrokeThickness(1.0)
    );
    assert_eq!(
        Style::new(c.lookup("gap-2").unwrap(), &BTreeSet::new()).fixed()[&Target::ListDirection],
        Value::ListDirection(Direction::Horizontal)
    );
}
#[test]
fn invalid_tokens_explain_their_failure() {
    let c = catalog();
    for (token, message) in [
        ("hover:flex", "variants"),
        ("bg-white/101", "0 to 100"),
        ("opacity-0.5", "whole number"),
        ("bg-slate-801", "Did you mean `bg-slate-800`"),
        ("w-1/0", "not a class"),
    ] {
        assert!(c.lookup(token).unwrap_err().contains(message), "{token}");
    }
    let c = theme::parse(Path::new("theme.toml"), "spacing = 5").unwrap();
    assert!(c.lookup("p-0.5").unwrap_err().contains("whole-pixel"));
    let c = theme::parse(Path::new("theme.toml"), "spacing = 50").unwrap();
    assert_eq!(c.lookup("pt-1.1").unwrap(), vec![Value::Padding(Side::Top, 55)]);
    let c = theme::parse(Path::new("theme.toml"), "spacing = 100").unwrap();
    assert_eq!(c.lookup("pt-0.07").unwrap(), vec![Value::Padding(Side::Top, 7)]);
    assert!(c.lookup("pt-0.075").unwrap_err().contains("7.5 px"));
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
        ("[breakpoint]\nsm = 640", 1, 2),
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
}
