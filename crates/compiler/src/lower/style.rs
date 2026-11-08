use super::Lower;
use crate::classes::{
    Alignment, Axis, Choice, Condition, Direction, InputMode, Offset, Orientation, Style, Target, Value,
};
use crate::tree::Node;
use crucible_luau::{BinOp, Expr, Function, Stmt};

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
    /// Applies the classes to `instance`. Targets that never vary become assignments and always-parented modifiers.
    /// Each target that varies becomes one binding. Returns the element's `interaction` local when its classes read its
    /// hover, press or focus, or make it a group.
    pub(super) fn class_style(&mut self, node: Node, instance: &Expr, style: &Style) -> Option<Expr> {
        let own = (style.interacts() || !style.groups().is_empty()).then(|| {
            let interaction = self.ui("interaction").call(vec![instance.clone()]);
            self.push_temp(interaction)
        });
        let values = style.fixed();
        let variants = style.variants();
        for (target, value) in &values {
            let (owner, property) = target.destination();
            if owner.is_none() && !matches!(target, Target::Size(_)) {
                self.current_block.push(Stmt::Assign(vec![instance.clone().field(property)], vec![expression(value)]));
            }
        }
        let x = values.get(&Target::Size(Axis::X));
        let y = values.get(&Target::Size(Axis::Y));
        let size_varies =
            variants.contains_key(&Target::Size(Axis::X)) || variants.contains_key(&Target::Size(Axis::Y));
        if !size_varies && (x.is_some() || y.is_some()) {
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
        for (target, choice) in &variants {
            if target.destination().0.is_none() && !matches!(target, Target::Size(_)) {
                self.bind_choice(node, &own, instance, target.destination().1, choice);
            }
        }
        if size_varies {
            let original = [Axis::X, Axis::Y]
                .iter()
                .any(|a| match variants.get(&Target::Size(*a)) {
                    Some(choice) => choice.fallback.is_none(),
                    None => !values.contains_key(&Target::Size(*a)),
                })
                .then(|| self.push_temp(instance.clone().field("Size")));
            let mut axes = Vec::new();
            for (axis, name) in [(Axis::X, "X"), (Axis::Y, "Y")] {
                let kept = || original.clone().map(|o| o.field(name)).unwrap_or(Expr::Nil);
                axes.push(match (variants.get(&Target::Size(axis)), values.get(&Target::Size(axis))) {
                    (Some(choice), _) => {
                        let fallback = choice.fallback.as_ref().map_or_else(kept, expression);
                        self.choose(node, &own, choice, fallback)
                    }
                    (None, Some(value)) => expression(value),
                    (None, None) => kept(),
                });
            }
            let size = constructor("UDim2.new", axes);
            self.bind(instance, "Size", size);
        }
        for (kind, when) in style.modifiers() {
            let child = self.push_temp(constructor("Instance.new", vec![Expr::string(kind.class())]));
            for (target, value) in values.iter().filter(|(target, _)| target.destination().0 == Some(kind)) {
                self.current_block
                    .push(Stmt::Assign(vec![child.clone().field(target.destination().1)], vec![expression(value)]));
            }
            for (target, choice) in variants.iter().filter(|(target, _)| target.destination().0 == Some(kind)) {
                self.bind_choice(node, &own, &child, target.destination().1, choice);
            }
            match when {
                None => self.current_block.push(Stmt::Assign(vec![child.field("Parent")], vec![instance.clone()])),
                Some(sets) => {
                    let mut any: Option<Expr> = None;
                    for set in &sets {
                        let holds = self.conditions(node, &own, set);
                        any = Some(match any {
                            Some(left) => left.binary(BinOp::Or, holds),
                            None => holds,
                        });
                    }
                    let parent = Expr::if_else(any.unwrap_or(Expr::Bool(false)), instance.clone(), Expr::Nil);
                    self.bind(&child, "Parent", parent);
                }
            }
        }
        own
    }

    /// `__ui.bind(instance, property, function() return value end)`.
    fn bind(&mut self, instance: &Expr, property: &'static str, value: Expr) {
        let read = Expr::Function(Box::new(Function {
            params: Vec::new(),
            varargs: false,
            body: vec![Stmt::Return(vec![value])],
        }));
        let call = self.ui("bind").call(vec![instance.clone(), Expr::string(property), read]);
        self.current_block.push(Stmt::Call(call));
    }

    /// Binds one varying property, falling back to the value it had before the binding when no rule is unconditional.
    fn bind_choice(
        &mut self,
        node: Node,
        own: &Option<Expr>,
        instance: &Expr,
        property: &'static str,
        choice: &Choice,
    ) {
        let fallback = match &choice.fallback {
            Some(value) => expression(value),
            None => self.push_temp(instance.clone().field(property)),
        };
        let value = self.choose(node, own, choice, fallback);
        self.bind(instance, property, value);
    }

    fn choose(&mut self, node: Node, own: &Option<Expr>, choice: &Choice, fallback: Expr) -> Expr {
        let mut chosen = fallback;
        for (conditions, value) in choice.cases.iter().rev() {
            let holds = self.conditions(node, own, conditions);
            chosen = Expr::if_else(holds, expression(value), chosen);
        }
        chosen
    }

    fn conditions(&mut self, node: Node, own: &Option<Expr>, conditions: &[Condition]) -> Expr {
        let mut all: Option<Expr> = None;
        for condition in conditions {
            let holds = self.class_condition(node, own, condition);
            all = Some(match all {
                Some(left) => left.binary(BinOp::And, holds),
                None => holds,
            });
        }
        all.unwrap_or(Expr::Bool(true))
    }

    fn class_condition(&mut self, node: Node, own: &Option<Expr>, condition: &Condition) -> Expr {
        let read = |source: Expr, field: &'static str| source.field(field).call(Vec::new());
        match condition {
            Condition::State(i) => read(own.clone().unwrap_or(Expr::Nil), i.field()),
            Condition::Group(name, i) => {
                match self.groups.iter().rev().find(|(names, _)| names.contains(name)).map(|(_, g)| g.clone()) {
                    Some(group) => read(group, i.field()),
                    None => self.unsupported(node, "a group variant outside its group"),
                }
            }
            Condition::Breakpoint(_, px) => {
                let width = read(self.ui("environment"), "viewport").field("X");
                width.binary(BinOp::Ge, Expr::Number(f64::from(*px)))
            }
            Condition::Orientation(o) => read(self.ui("environment"), "orientation").binary(
                BinOp::Eq,
                Expr::string(match o {
                    Orientation::Portrait => "portrait",
                    Orientation::Landscape => "landscape",
                }),
            ),
            Condition::Input(mode) => read(self.ui("environment"), "input").binary(
                BinOp::Eq,
                Expr::string(match mode {
                    InputMode::Mouse => "mouse",
                    InputMode::Touch => "touch",
                    InputMode::Gamepad => "gamepad",
                }),
            ),
            Condition::Dark => read(self.ui("environment"), "dark"),
        }
    }
}
