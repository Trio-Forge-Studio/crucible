//! The `crucible.toml` sections that declare the experience: `[experience]`, `[products.<key>]`, `[passes.<key>]`,
//! `[badges.<key>]`, and each environment's overlays of them. These types follow the file. `crucible-iac` turns them
//! into its model and checks the rules that span tables.

use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// The private server price in Robux. 0 makes private servers free. Roblox's API reference says changing the price
    /// cancels every private server subscription.
    pub private_server_price: Option<u32>,
    pub voice_chat: Option<bool>,
}

/// A developer product or a game pass. One that is not retired needs a name, and one that is neither retired nor
/// untracked a price too.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend(
    "not" = { "required": ["retired", "untrack"], "properties": { "retired": { "const": true }, "untrack": { "const": true } } },
    "if" = { "properties": { "retired": { "const": false }, "untrack": { "const": false } } },
    "then" = { "required": ["name", "price"], "properties": { "name": { "minLength": 1 } } },
    "allOf" = [{
        "if": { "required": ["untrack"], "properties": { "untrack": { "const": true } } },
        "then": { "required": ["name"], "properties": { "name": { "minLength": 1 } } },
    }],
))]
pub struct Priced {
    /// The display name, with no whitespace at either end.
    #[schemars(extend("not" = { "pattern": "^[\\t-\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000\\ufeff]|[\\t-\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000\\ufeff]$" }))]
    pub name: Option<String>,
    pub description: Option<String>,
    /// The price in Robux. A price of 0 is not off sale: set `retired = true` instead.
    #[schemars(range(min = 1))]
    pub price: Option<u64>,
    /// An image path relative to the project root.
    pub icon: Option<PathBuf>,
    /// Take it off sale. Roblox cannot delete it.
    #[serde(default)]
    pub retired: bool,
    /// Remove it from `crucible.lock` and leave it unchanged on Roblox. It keeps its name while declared.
    #[serde(default)]
    pub untrack: bool,
    /// The key it had before a rename, so it keeps its ID.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9-]*$"))]
    pub moved_from: Option<String>,
}

/// A badge. One that is not retired needs a name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend(
    "not" = { "required": ["retired", "untrack"], "properties": { "retired": { "const": true }, "untrack": { "const": true } } },
    "if" = { "properties": { "retired": { "const": false }, "untrack": { "const": false } } },
    "then" = { "required": ["name"], "properties": { "name": { "minLength": 1 } } },
    "allOf" = [{
        "if": { "required": ["untrack"], "properties": { "untrack": { "const": true } } },
        "then": { "required": ["name"], "properties": { "name": { "minLength": 1 } } },
    }],
))]
pub struct Badge {
    /// The display name, with no whitespace at either end.
    #[schemars(extend("not" = { "pattern": "^[\\t-\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000\\ufeff]|[\\t-\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000\\ufeff]$" }))]
    pub name: Option<String>,
    pub description: Option<String>,
    /// An image path relative to the project root.
    pub icon: Option<PathBuf>,
    /// Defaults to true.
    pub enabled: Option<bool>,
    /// Disable it. Roblox cannot delete it.
    #[serde(default)]
    pub retired: bool,
    /// Remove it from `crucible.lock` and leave it unchanged on Roblox. It keeps its name while declared.
    #[serde(default)]
    pub untrack: bool,
    /// The key it had before a rename, so it keeps its ID.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9-]*$"))]
    pub moved_from: Option<String>,
}

/// The fields an environment may override.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PricedOverlay {
    #[schemars(range(min = 1))]
    pub price: Option<u64>,
    /// `false` keeps it off sale in this environment while its handle stays bound. Defaults to true. `retired` wins.
    pub for_sale: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BadgeOverlay {
    pub enabled: Option<bool>,
}

/// A table and the line of its key in `crucible.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located<T> {
    pub line: usize,
    pub value: T,
}

pub type Tables<T> = BTreeMap<String, Located<T>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    pub settings: Option<Located<Settings>>,
    pub products: Tables<Priced>,
    pub passes: Tables<Priced>,
    pub badges: Tables<Badge>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlays {
    pub products: Tables<PricedOverlay>,
    pub passes: Tables<PricedOverlay>,
    pub badges: Tables<BadgeOverlay>,
}
