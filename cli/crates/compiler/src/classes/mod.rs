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
#[derive(Clone, Debug, PartialEq)]
enum Variant {
    Base,
}
#[derive(Clone, Debug, PartialEq)]
struct Rule {
    variant: Variant,
    value: Value,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Style {
    series: BTreeMap<Target, Vec<Rule>>,
}
impl Style {
    pub(crate) fn new(values: impl IntoIterator<Item = Value>, explicit: &BTreeSet<String>) -> Self {
        let mut style = Self::default();
        for value in values {
            let target = value.target();
            let (owner, property) = target.destination();
            if owner.is_none() && explicit.contains(property) {
                continue;
            }
            style.series.entry(target).or_default().push(Rule { variant: Variant::Base, value });
        }
        style
    }
    pub(crate) fn fixed(&self) -> BTreeMap<Target, Value> {
        let mut values: BTreeMap<_, _> = self
            .series
            .iter()
            .filter_map(|(t, rules)| {
                rules.iter().rev().find(|r| r.variant == Variant::Base).map(|r| (*t, r.value.clone()))
            })
            .collect();
        let kinds: BTreeSet<_> = values.keys().filter_map(|t| t.destination().0).collect();
        for kind in kinds {
            let defaults = match kind {
                Modifier::Stroke => {
                    vec![Value::StrokeMode, Value::StrokeThickness(1.0), Value::StrokeColor(Rgb(0, 0, 0))]
                }
                Modifier::List => vec![Value::ListDirection(Direction::Horizontal), Value::ListSortOrder],
                _ => vec![],
            };
            for value in defaults {
                values.entry(value.target()).or_insert(value);
            }
        }
        values
    }
}

pub(crate) struct Catalog {
    named: BTreeMap<String, Vec<Value>>,
    spacing: Option<f64>,
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
        Self { named, spacing: None, has_theme }
    }
    pub(crate) fn lookup(&self, token: &str) -> Result<Vec<Value>, String> {
        if token.contains(':') {
            return Err("variants such as `hover:` are not supported yet".into());
        }
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
                && let (Some(n), Some(step)) = (number(suffix), self.spacing)
            {
                let px = n * step;
                if px.fract() != 0.0 || px > i32::MAX as f64 {
                    return Err(format!("{px} px is not a whole-pixel offset in range"));
                }
                let px = px as i32;
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
        let mut message = format!("`{token}` is not a class.");
        if let Some(name) = self
            .named
            .keys()
            .filter_map(|name| {
                let d = distance(token, name);
                (d <= 2).then_some((d, name))
            })
            .min()
        {
            message.push_str(&format!(" Did you mean `{}`?", name.1));
        }
        if !self.has_theme {
            message.push_str(" No theme.toml is beside the tsconfig.");
        }
        Err(message)
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
