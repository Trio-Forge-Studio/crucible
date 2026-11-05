#[cfg(test)]
mod tests;
mod theme;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(crate) use theme::ThemeError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Axis {
    X,
    Y,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Side {
    Top,
    Right,
    Bottom,
    Left,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Modifier {
    Corner,
    Padding,
    Stroke,
    List,
}
impl Modifier {
    pub(crate) fn class(self) -> &'static str {
        match self {
            Self::Corner => "UICorner",
            Self::Padding => "UIPadding",
            Self::Stroke => "UIStroke",
            Self::List => "UIListLayout",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Offset {
    Pixels(i32),
    Scale(f64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rgb(pub u8, pub u8, pub u8);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Alignment {
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Target {
    BackgroundColor3,
    BackgroundTransparency,
    TextColor3,
    TextTransparency,
    TextSize,
    TextXAlignment,
    GroupTransparency,
    Size(Axis),
    CornerRadius,
    Padding(Side),
    StrokeMode,
    StrokeColor,
    StrokeThickness,
    StrokeTransparency,
    ListDirection,
    ListPadding,
    ListSortOrder,
}
impl Target {
    pub(crate) fn destination(self) -> (Option<Modifier>, &'static str) {
        use Modifier::*;
        match self {
            Self::BackgroundColor3 => (None, "BackgroundColor3"),
            Self::BackgroundTransparency => (None, "BackgroundTransparency"),
            Self::TextColor3 => (None, "TextColor3"),
            Self::TextTransparency => (None, "TextTransparency"),
            Self::TextSize => (None, "TextSize"),
            Self::TextXAlignment => (None, "TextXAlignment"),
            Self::GroupTransparency => (None, "GroupTransparency"),
            Self::Size(_) => (None, "Size"),
            Self::CornerRadius => (Some(Corner), "CornerRadius"),
            Self::Padding(side) => (
                Some(Padding),
                match side {
                    Side::Top => "PaddingTop",
                    Side::Right => "PaddingRight",
                    Side::Bottom => "PaddingBottom",
                    Side::Left => "PaddingLeft",
                },
            ),
            Self::StrokeMode => (Some(Stroke), "ApplyStrokeMode"),
            Self::StrokeColor => (Some(Stroke), "Color"),
            Self::StrokeThickness => (Some(Stroke), "Thickness"),
            Self::StrokeTransparency => (Some(Stroke), "Transparency"),
            Self::ListDirection => (Some(List), "FillDirection"),
            Self::ListPadding => (Some(List), "Padding"),
            Self::ListSortOrder => (Some(List), "SortOrder"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    BackgroundColor3(Rgb),
    BackgroundTransparency(f64),
    TextColor3(Rgb),
    TextTransparency(f64),
    TextSize(f64),
    TextXAlignment(Alignment),
    GroupTransparency(f64),
    Size(Axis, Offset),
    CornerRadius(Offset),
    Padding(Side, i32),
    StrokeMode,
    StrokeColor(Rgb),
    StrokeThickness(f64),
    StrokeTransparency(f64),
    ListDirection(Direction),
    ListPadding(i32),
    ListSortOrder,
}
impl Value {
    pub(crate) fn target(&self) -> Target {
        match self {
            Self::BackgroundColor3(_) => Target::BackgroundColor3,
            Self::BackgroundTransparency(_) => Target::BackgroundTransparency,
            Self::TextColor3(_) => Target::TextColor3,
            Self::TextTransparency(_) => Target::TextTransparency,
            Self::TextSize(_) => Target::TextSize,
            Self::TextXAlignment(_) => Target::TextXAlignment,
            Self::GroupTransparency(_) => Target::GroupTransparency,
            Self::Size(a, _) => Target::Size(*a),
            Self::CornerRadius(_) => Target::CornerRadius,
            Self::Padding(s, _) => Target::Padding(*s),
            Self::StrokeMode => Target::StrokeMode,
            Self::StrokeColor(_) => Target::StrokeColor,
            Self::StrokeThickness(_) => Target::StrokeThickness,
            Self::StrokeTransparency(_) => Target::StrokeTransparency,
            Self::ListDirection(_) => Target::ListDirection,
            Self::ListPadding(_) => Target::ListPadding,
            Self::ListSortOrder => Target::ListSortOrder,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Interaction {
    Hover,
    Press,
    Focus,
}
impl Interaction {
    /// The `Interaction` field of `@trioforge/crucible/ui` that reads this state.
    pub(crate) fn field(self) -> &'static str {
        match self {
            Self::Hover => "hover",
            Self::Press => "press",
            Self::Focus => "focus",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Orientation {
    Portrait,
    Landscape,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum InputMode {
    Mouse,
    Touch,
    Gamepad,
}
/// One variant of a class token. A token's writes apply while all of its conditions hold.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Condition {
    State(Interaction),
    /// The nearest enclosing group with this name, or with a bare `group` class.
    Group(Option<String>, Interaction),
    /// The viewport is at least this many pixels wide.
    Breakpoint(String, u32),
    Orientation(Orientation),
    Input(InputMode),
    Dark,
}
/// Whenever `a` holds, `b` holds.
pub(crate) fn implies(a: &Condition, b: &Condition) -> bool {
    match (a, b) {
        (Condition::Breakpoint(_, a), Condition::Breakpoint(_, b)) => a >= b,
        _ => a == b,
    }
}
fn contradicts(a: &Condition, b: &Condition) -> bool {
    match (a, b) {
        (Condition::Orientation(a), Condition::Orientation(b)) => a != b,
        (Condition::Input(a), Condition::Input(b)) => a != b,
        _ => false,
    }
}
const VARIANTS: [&str; 12] = [
    "hover",
    "active",
    "focus",
    "group-hover",
    "group-active",
    "group-focus",
    "portrait",
    "landscape",
    "touch",
    "mouse",
    "gamepad",
    "dark",
];
/// Whether a `[breakpoint]` name would read as a built-in variant or a group.
pub(super) fn reserved(name: &str) -> bool {
    VARIANTS.contains(&name) || name.starts_with("group")
}
pub(super) fn is_name(text: &str) -> bool {
    text.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && text.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}
/// `group` is `Some(None)` and `group/<name>` is `Some(Some(name))`.
pub(crate) fn group_marker(token: &str) -> Option<Option<String>> {
    match token.strip_prefix("group") {
        Some("") => Some(None),
        Some(rest) => rest.strip_prefix('/').filter(|name| is_name(name)).map(|name| Some(name.to_owned())),
        None => None,
    }
}

/// A resolved class token.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Class {
    /// Typed writes that apply while every condition holds. The conditions are sorted.
    Writes(Vec<Condition>, Vec<Value>),
    /// `group` or `group/<name>`, which writes nothing and lets descendants read this element's interaction.
    Group(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    conditions: Vec<Condition>,
    value: Value,
    token: usize,
}
/// How a target that varies picks its value: the first case whose conditions all hold, else the fallback. A `None`
/// fallback is the value the instance had before its classes applied.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Choice {
    pub(crate) cases: Vec<(Vec<Condition>, Value)>,
    pub(crate) fallback: Option<Value>,
}
/// A conditional rule that never applies, because a later token's rule on the same target holds whenever it does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dead {
    pub(crate) token: usize,
    pub(crate) target: Target,
    pub(crate) by: usize,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Style {
    series: BTreeMap<Target, Vec<Rule>>,
    groups: BTreeSet<Option<String>>,
}
fn defaults(kind: Modifier) -> Vec<Value> {
    match kind {
        Modifier::Stroke => vec![Value::StrokeMode, Value::StrokeThickness(1.0), Value::StrokeColor(Rgb(0, 0, 0))],
        Modifier::List => vec![Value::ListDirection(Direction::Horizontal), Value::ListSortOrder],
        _ => vec![],
    }
}
/// The target's last unconditional rule, and the rules after it, which decide its value while they hold. A rule right
/// after the base that writes the base's value changes nothing, because it is tested last, so it is left out.
fn live(rules: &[Rule]) -> (Option<&Rule>, &[Rule]) {
    match rules.iter().rposition(|r| r.conditions.is_empty()) {
        Some(base) => {
            let same = rules[base + 1..].iter().take_while(|r| r.value == rules[base].value).count();
            (Some(&rules[base]), &rules[base + 1 + same..])
        }
        None => (None, rules),
    }
}
impl Style {
    /// `Dead` names each class by its index in `classes`.
    pub(crate) fn new(classes: impl IntoIterator<Item = Class>, explicit: &BTreeSet<String>) -> Self {
        let mut style = Self::default();
        for (token, class) in classes.into_iter().enumerate() {
            let (conditions, values) = match class {
                Class::Group(name) => {
                    style.groups.insert(name);
                    continue;
                }
                Class::Writes(conditions, values) => (conditions, values),
            };
            for value in values {
                let target = value.target();
                let (owner, property) = target.destination();
                if owner.is_none() && explicit.contains(property) {
                    continue;
                }
                style.series.entry(target).or_default().push(Rule { conditions: conditions.clone(), value, token });
            }
        }
        style
    }
    pub(crate) fn groups(&self) -> &BTreeSet<Option<String>> {
        &self.groups
    }
    /// Whether a rule reads the element's own hover, press or focus.
    pub(crate) fn interacts(&self) -> bool {
        self.series.values().flatten().flat_map(|r| &r.conditions).any(|c| matches!(c, Condition::State(_)))
    }
    /// The value of each target that never varies, and each modifier's defaults for the targets no rule sets.
    pub(crate) fn fixed(&self) -> BTreeMap<Target, Value> {
        let mut values: BTreeMap<_, _> = self
            .series
            .iter()
            .filter_map(|(t, rules)| match live(rules) {
                (Some(base), []) => Some((*t, base.value.clone())),
                _ => None,
            })
            .collect();
        for kind in self.modifiers().into_keys() {
            for value in defaults(kind) {
                if !self.series.contains_key(&value.target()) {
                    values.insert(value.target(), value);
                }
            }
        }
        values
    }
    /// How each target with a conditional rule after its last unconditional one picks its value.
    pub(crate) fn variants(&self) -> BTreeMap<Target, Choice> {
        self.series
            .iter()
            .filter_map(|(t, rules)| {
                let (base, cases) = live(rules);
                if cases.is_empty() {
                    return None;
                }
                let fallback = base.map(|r| r.value.clone()).or_else(|| {
                    t.destination().0.and_then(|kind| defaults(kind).into_iter().find(|v| v.target() == *t))
                });
                let cases = cases.iter().rev().map(|r| (r.conditions.clone(), r.value.clone())).collect();
                Some((*t, Choice { cases, fallback }))
            })
            .collect()
    }
    /// Each modifier kind a rule creates. `None` parents it always. Otherwise it is parented while any of the
    /// condition sets holds.
    pub(crate) fn modifiers(&self) -> BTreeMap<Modifier, Option<Vec<Vec<Condition>>>> {
        let mut kinds: BTreeMap<Modifier, Option<Vec<Vec<Condition>>>> = BTreeMap::new();
        for (target, rules) in &self.series {
            let Some(kind) = target.destination().0 else { continue };
            let entry = kinds.entry(kind).or_insert_with(|| Some(Vec::new()));
            match (live(rules), entry) {
                ((Some(_), _), entry) => *entry = None,
                ((None, cases), Some(when)) => {
                    for rule in cases {
                        if !when.contains(&rule.conditions) {
                            when.push(rule.conditions.clone());
                        }
                    }
                }
                ((None, _), None) => {}
            }
        }
        kinds
    }
    /// Each token with a conditional rule that a later rule on the same target always overrides, once.
    pub(crate) fn dead(&self) -> Vec<Dead> {
        let mut dead = Vec::new();
        for (target, rules) in &self.series {
            for (i, rule) in rules.iter().enumerate() {
                if rule.conditions.is_empty() {
                    continue;
                }
                if let Some(later) = rules[i + 1..]
                    .iter()
                    .find(|later| later.conditions.iter().all(|c| rule.conditions.iter().any(|d| implies(d, c))))
                {
                    dead.push(Dead { token: rule.token, target: *target, by: later.token });
                }
            }
        }
        dead.sort_by_key(|d| d.token);
        dead.dedup_by_key(|d| d.token);
        dead
    }
}

/// A nonnegative decimal kept as its digits and the count of them after the point, so a product of two is exact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Decimal {
    digits: u128,
    scale: u32,
}
impl Decimal {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        if whole.is_empty() && fraction.is_empty() || !whole.bytes().chain(fraction.bytes()).all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let digits = format!("{whole}{fraction}").parse().ok()?;
        Some(Self { digits, scale: u32::try_from(fraction.len()).ok()? })
    }
    /// The product as whole pixels, or `None` when it has a fractional part or does not fit an offset.
    fn whole_product(self, other: Self) -> Option<i32> {
        let product = self.digits.checked_mul(other.digits)?;
        let unit = 10u128.checked_pow(self.scale.checked_add(other.scale)?)?;
        if product % unit != 0 {
            return None;
        }
        i32::try_from(product / unit).ok()
    }
    fn to_f64(self) -> f64 {
        self.digits as f64 / 10f64.powi(self.scale as i32)
    }
}

pub(crate) struct Catalog {
    named: BTreeMap<String, Vec<Value>>,
    spacing: Option<Decimal>,
    breakpoints: BTreeMap<String, u32>,
    has_theme: bool,
}
impl Catalog {
    pub(crate) fn load(dir: &Path) -> Result<Self, ThemeError> {
        theme::load(dir)
    }
    fn empty(has_theme: bool) -> Self {
        let mut named = BTreeMap::new();
        for (name, values) in [
            ("flex", vec![Value::ListDirection(Direction::Horizontal)]),
            ("flex-col", vec![Value::ListDirection(Direction::Vertical)]),
            ("border", vec![Value::StrokeThickness(1.0)]),
            ("rounded-full", vec![Value::CornerRadius(Offset::Scale(1.0))]),
            ("bg-transparent", vec![Value::BackgroundTransparency(1.0)]),
            ("text-left", vec![Value::TextXAlignment(Alignment::Left)]),
            ("text-center", vec![Value::TextXAlignment(Alignment::Center)]),
            ("text-right", vec![Value::TextXAlignment(Alignment::Right)]),
        ] {
            named.insert(name.into(), values);
        }
        for prefix in ["w", "h", "size"] {
            for (suffix, offset) in [("full", Offset::Scale(1.0)), ("px", Offset::Pixels(1))] {
                named.insert(format!("{prefix}-{suffix}"), sizes(prefix, offset));
            }
        }
        Self { named, spacing: None, breakpoints: BTreeMap::new(), has_theme }
    }
    /// Resolves `variant:...:utility`. Variants stack in any order, and the writes apply while all of them hold.
    pub(crate) fn parse(&self, token: &str) -> Result<Class, String> {
        let mut variants: Vec<&str> = token.split(':').collect();
        let utility = variants.pop().unwrap_or_default();
        let mut seen: Vec<(&str, Condition)> = Vec::new();
        for variant in variants {
            let condition = self.variant(variant)?;
            if let Some((other, _)) = seen.iter().find(|(_, c)| *c == condition) {
                return Err(format!("`{variant}:` repeats `{other}:`"));
            }
            if let Some((other, _)) = seen.iter().find(|(_, c)| contradicts(c, &condition)) {
                return Err(format!("`{other}:` and `{variant}:` can never hold together"));
            }
            seen.push((variant, condition));
        }
        if let Some(name) = group_marker(utility) {
            if !seen.is_empty() {
                return Err(format!("`{utility}` takes no variants"));
            }
            return Ok(Class::Group(name));
        }
        let writes = self.lookup(utility)?;
        let mut conditions: Vec<_> = seen.into_iter().map(|(_, c)| c).collect();
        conditions.sort();
        Ok(Class::Writes(conditions, writes))
    }
    fn variant(&self, text: &str) -> Result<Condition, String> {
        let interaction = |text: &str| match text {
            "hover" => Some(Interaction::Hover),
            "active" => Some(Interaction::Press),
            "focus" => Some(Interaction::Focus),
            _ => None,
        };
        if let Some(i) = interaction(text) {
            return Ok(Condition::State(i));
        }
        if let Some(rest) = text.strip_prefix("group-") {
            let (kind, name) = match rest.split_once('/') {
                Some((kind, name)) if is_name(name) => (kind, Some(name.to_owned())),
                Some(_) => ("", None),
                None => (rest, None),
            };
            if let Some(i) = interaction(kind) {
                return Ok(Condition::Group(name, i));
            }
        }
        Ok(match text {
            "portrait" => Condition::Orientation(Orientation::Portrait),
            "landscape" => Condition::Orientation(Orientation::Landscape),
            "touch" => Condition::Input(InputMode::Touch),
            "mouse" => Condition::Input(InputMode::Mouse),
            "gamepad" => Condition::Input(InputMode::Gamepad),
            "dark" => Condition::Dark,
            _ => match self.breakpoints.get(text) {
                Some(px) => Condition::Breakpoint(text.to_owned(), *px),
                None => {
                    let names = VARIANTS.into_iter().chain(self.breakpoints.keys().map(String::as_str));
                    return Err(self.unknown(format!("`{text}:` is not a variant."), text, names, ":"));
                }
            },
        })
    }
    /// `message`, with the nearest of `names` within edit distance 2 and a note when no theme exists.
    fn unknown<'a>(
        &self,
        mut message: String,
        text: &str,
        names: impl Iterator<Item = &'a str>,
        suffix: &str,
    ) -> String {
        if let Some((_, name)) = names
            .filter_map(|name| {
                let d = distance(text, name);
                (d <= 2).then_some((d, name))
            })
            .min()
        {
            message.push_str(&format!(" Did you mean `{name}{suffix}`?"));
        }
        if !self.has_theme {
            message.push_str(" No theme.toml is beside the tsconfig.");
        }
        message
    }
    pub(crate) fn lookup(&self, token: &str) -> Result<Vec<Value>, String> {
        if let Some(values) = self.named.get(token) {
            return Ok(values.clone());
        }
        if let Some((base, alpha)) = token.rsplit_once('/')
            && let Some(values) = self.named.get(base)
            && values
                .iter()
                .any(|v| matches!(v, Value::BackgroundColor3(_) | Value::TextColor3(_) | Value::StrokeColor(_)))
        {
            let alpha = percent(alpha)?;
            return Ok(values
                .iter()
                .map(|v| match v {
                    Value::BackgroundTransparency(_) => Value::BackgroundTransparency(alpha),
                    Value::TextTransparency(_) => Value::TextTransparency(alpha),
                    Value::StrokeTransparency(_) => Value::StrokeTransparency(alpha),
                    _ => v.clone(),
                })
                .collect());
        }
        if let Some((prefix, suffix)) = token.split_once('-') {
            if prefix == "opacity" {
                return Ok(vec![Value::GroupTransparency(percent(suffix)?)]);
            }
            if prefix == "border"
                && let Some(n) = number(suffix)
            {
                return Ok(vec![Value::StrokeThickness(n)]);
            }
            let size = matches!(prefix, "w" | "h" | "size");
            if size
                && let Some((a, b)) = suffix.split_once('/')
                && let (Some(a), Some(b)) = (number(a), number(b))
                && b > 0.0
            {
                return Ok(sizes(prefix, Offset::Scale(a / b)));
            }
            if matches!(prefix, "p" | "px" | "py" | "pt" | "pr" | "pb" | "pl" | "w" | "h" | "size" | "gap")
                && let (Some(n), Some(step)) = (Decimal::parse(suffix), self.spacing)
            {
                let Some(px) = n.whole_product(step) else {
                    return Err(format!("{} px is not a whole-pixel offset in range", n.to_f64() * step.to_f64()));
                };
                return Ok(if size {
                    sizes(prefix, Offset::Pixels(px))
                } else if prefix == "gap" {
                    vec![Value::ListPadding(px)]
                } else {
                    let sides: &[Side] = match prefix {
                        "p" => &[Side::Top, Side::Right, Side::Bottom, Side::Left],
                        "px" => &[Side::Left, Side::Right],
                        "py" => &[Side::Top, Side::Bottom],
                        "pt" => &[Side::Top],
                        "pr" => &[Side::Right],
                        "pb" => &[Side::Bottom],
                        _ => &[Side::Left],
                    };
                    sides.iter().map(|s| Value::Padding(*s, px)).collect()
                });
            }
        }
        Err(self.unknown(format!("`{token}` is not a class."), token, self.named.keys().map(String::as_str), ""))
    }
}
fn sizes(prefix: &str, offset: Offset) -> Vec<Value> {
    match prefix {
        "w" => vec![Value::Size(Axis::X, offset)],
        "h" => vec![Value::Size(Axis::Y, offset)],
        _ => vec![Value::Size(Axis::X, offset), Value::Size(Axis::Y, offset)],
    }
}
fn number(text: &str) -> Option<f64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    text.parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0)
}
fn percent(text: &str) -> Result<f64, String> {
    text.parse::<u8>()
        .ok()
        .filter(|n| *n <= 100 && text.bytes().all(|b| b.is_ascii_digit()))
        .map(|n| f64::from(100 - n) / 100.0)
        .ok_or_else(|| "opacity must be a whole number from 0 to 100".into())
}
fn distance(a: &str, b: &str) -> usize {
    let mut row: Vec<_> = (0..=b.chars().count()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.chars().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j + 1] + 1).min(row[j] + 1).min(diagonal + usize::from(x != y));
            diagonal = old;
        }
    }
    *row.last().unwrap()
}
