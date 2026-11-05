use super::Lower;
use crate::classes::{Alignment, Axis, Direction, Modifier, Offset, Style, Target, Value};
use crate::luau::{Expr, Stmt};

fn constructor(name: &'static str, args: Vec<Expr>) -> Expr {
    Expr::global(name).call(args)
}
fn offset(offset: Offset) -> (f64, f64) {
    match offset {
        Offset::Pixels(n) => (0.0, f64::from(n)),
        Offset::Scale(n) => (n, 0.0),
    }
}
fn udim(value: Offset) -> Expr {
    let (scale, pixels) = offset(value);
    constructor("UDim.new", vec![Expr::Number(scale), Expr::Number(pixels)])
}
fn enumeration(kind: &'static str, item: &'static str) -> Expr {
    Expr::global("Enum").field(kind).field(item)
}
fn expression(value: &Value) -> Expr {
    match *value {
        Value::BackgroundColor3(rgb) | Value::TextColor3(rgb) | Value::StrokeColor(rgb) => constructor(
            "Color3.fromRGB",
            vec![Expr::Number(f64::from(rgb.0)), Expr::Number(f64::from(rgb.1)), Expr::Number(f64::from(rgb.2))],
        ),
        Value::BackgroundTransparency(n)
        | Value::TextTransparency(n)
        | Value::TextSize(n)
        | Value::GroupTransparency(n)
        | Value::StrokeThickness(n)
        | Value::StrokeTransparency(n) => Expr::Number(n),
        Value::Size(_, o) | Value::CornerRadius(o) => udim(o),
        Value::Padding(_, n) | Value::ListPadding(n) => udim(Offset::Pixels(n)),
        Value::TextXAlignment(a) => enumeration(
            "TextXAlignment",
            match a {
                Alignment::Left => "Left",
                Alignment::Center => "Center",
                Alignment::Right => "Right",
            },
        ),
        Value::StrokeMode => enumeration("ApplyStrokeMode", "Border"),
        Value::ListDirection(d) => enumeration(
            "FillDirection",
            match d {
                Direction::Horizontal => "Horizontal",
                Direction::Vertical => "Vertical",
            },
        ),
        Value::ListSortOrder => enumeration("SortOrder", "LayoutOrder"),
    }
}

impl Lower<'_> {
    pub(super) fn fixed_style(&mut self, instance: &Expr, style: &Style) {
        let values = style.fixed();
        for (target, value) in &values {
            let (owner, property) = target.destination();
            if owner.is_none() && !matches!(target, Target::Size(_)) {
                self.current_block.push(Stmt::Assign(vec![instance.clone().field(property)], vec![expression(value)]));
            }
        }
        let x = values.get(&Target::Size(Axis::X));
        let y = values.get(&Target::Size(Axis::Y));
        if x.is_some() || y.is_some() {
            let axis = |value: Option<&Value>, name| match value {
                Some(Value::Size(_, o)) => udim(*o),
                None => instance.clone().field("Size").field(name),
                _ => unreachable!(),
            };
            let size = match (x, y) {
                (Some(Value::Size(_, x)), Some(Value::Size(_, y))) => {
                    let (xs, xo) = offset(*x);
                    let (ys, yo) = offset(*y);
                    constructor(
                        "UDim2.new",
                        vec![Expr::Number(xs), Expr::Number(xo), Expr::Number(ys), Expr::Number(yo)],
                    )
                }
                _ => constructor("UDim2.new", vec![axis(x, "X"), axis(y, "Y")]),
            };
            self.current_block.push(Stmt::Assign(vec![instance.clone().field("Size")], vec![size]));
        }
        for kind in [Modifier::Corner, Modifier::Padding, Modifier::Stroke, Modifier::List] {
            let writes: Vec<_> = values.iter().filter(|(target, _)| target.destination().0 == Some(kind)).collect();
            if writes.is_empty() {
                continue;
            }
            let child = self.push_temp(constructor("Instance.new", vec![Expr::string(kind.class())]));
            for (target, value) in writes {
                self.current_block
                    .push(Stmt::Assign(vec![child.clone().field(target.destination().1)], vec![expression(value)]));
            }
            self.current_block.push(Stmt::Assign(vec![child.field("Parent")], vec![instance.clone()]));
        }
    }
}
