use std::collections::BTreeMap;

use crucible_compiler::extension::{Kind, Node};
use crucible_luau::{Expr, Field};
use serde::{Deserialize, Serialize};

use crate::Code;
use crate::facts::Fact;
use crate::registration::Walk;

pub const ICON_SIZE: u32 = 60;
pub type IconAtlas = BTreeMap<String, IconRect>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconRect {
    pub asset: String,
    pub x: u32,
    pub y: u32,
}

#[derive(Serialize, Deserialize)]
pub enum IconMode {
    Build(Result<IconAtlas, String>),
    Collect,
}

impl Default for IconMode {
    fn default() -> Self {
        Self::Build(Ok(IconAtlas::new()))
    }
}

impl Walk<'_, '_> {
    pub(crate) fn icon(&mut self, call: Node) {
        let arguments = call.arguments();
        let Some(name) = arguments.get(0).filter(|n| arguments.len() == 1 && n.kind() == Kind::StringLiteral) else {
            self.report(
                arguments.get(0).unwrap_or(call),
                Code::Rule(41),
                "icon() needs a string literal, such as icon(\"house\"). Crucible bakes only the icons it can see.",
            );
            return;
        };
        let name = name.text();
        self.state.icon_names.insert(name.to_owned());
        let problem = match &self.state.icons {
            IconMode::Collect => return,
            IconMode::Build(Err(problem)) => problem.clone(),
            IconMode::Build(Ok(atlas)) => {
                if let Some(rect) = atlas.get(name).cloned() {
                    self.store(call, Fact::Icon(rect));
                    return;
                }
                format!("The icon atlas has no entry for {name:?}.")
            }
        };
        let message = format!("{problem} Run `crucible icons` to update the game's icon atlas.");
        self.report(call, Code::Rule(42), &message);
    }
}

pub(crate) fn lower(rect: &IconRect) -> Expr {
    let vector = |x, y| Expr::global("Vector2.new").call(vec![Expr::Number(f64::from(x)), Expr::Number(f64::from(y))]);
    Expr::Table(vec![
        Field::Keyed(Expr::string("Image"), Expr::string(format!("rbxassetid://{}", rect.asset))),
        Field::Keyed(Expr::string("ImageRectOffset"), vector(rect.x, rect.y)),
        Field::Keyed(Expr::string("ImageRectSize"), vector(ICON_SIZE, ICON_SIZE)),
    ])
}
