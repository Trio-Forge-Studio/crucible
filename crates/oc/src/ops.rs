//! The Open Cloud calls provisioning and deploy make for developer products, game passes, badges, the universe and the
//! key itself, as typed requests and replies over `Transport`. An operation's retry rule is its type's `S` parameter, not a registry lookup: a create
//! is `Unsafe`, because Roblox takes no idempotency key and cannot delete a duplicate, so the outcome judge never
//! repeats it after a lost answer. An update writes absolute values, so repeating it leaves the same state.
//!
//! Once a write may have acted, because an attempt got a 2xx or a 5xx or lost its answer after it was sent, `run` never
//! reports it as unavailable: it returns the parsed reply or exit 5, with what the reply held, so a caller checks
//! Roblox before it writes again.

use crate::auth::Introspection;
use crate::outcome::{Failure, UNKNOWN_HINT};
use crate::transport::{Operation, Reply, Request, Transport};
use crate::{Auth, Payload, Safety, Scope};
use serde_json::{Value, json};
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

pub type Id = NonZeroU64;

const APIS: &str = "https://apis.roblox.com";
const BADGES: &str = "https://badges.roblox.com";

pub trait Retry {
    const SAFETY: Safety;
}
pub struct Read;
pub struct Idempotent;
pub struct Unsafe;
impl Retry for Read {
    const SAFETY: Safety = Safety::Read;
}
impl Retry for Idempotent {
    const SAFETY: Safety = Safety::Idempotent;
}
impl Retry for Unsafe {
    const SAFETY: Safety = Safety::Unsafe;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub file_name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// What a create or an update of a product or a pass sends. `None` leaves a field unchanged on update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: Option<u64>,
    pub for_sale: Option<bool>,
    pub icon: Option<Image>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BadgeDetails {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

/// The experience settings a universe update writes. `None` leaves a setting unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UniverseSettings {
    pub private_server_price: Option<u32>,
    pub voice_chat: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sellable {
    pub id: Id,
    pub name: String,
    pub description: String,
    /// `None` when Roblox holds no price for it, which is not the same as off sale.
    pub price: Option<u64>,
    pub for_sale: bool,
    pub icon: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub icon: Option<u64>,
}

/// A badge read by its ID, which Roblox answers for a badge of any universe, with the universe that awards it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwardedBadge {
    pub badge: Badge,
    pub universe: Id,
}

struct Page<T> {
    items: Vec<T>,
    next: Option<String>,
}

/// Builds the request for the page after `token`, or the first page.
type PageOp<T> = dyn Fn(Option<&str>) -> Op<Page<T>, Read>;

/// The most pages `Transport::list_all` reads before it fails, unless `List::max_pages` sets another cap.
pub const MAX_PAGES: usize = 1000;

/// A whole list, which `Transport::list_all` reads page by page to its end. No single page is public, so no caller can
/// mistake one page for the list.
pub struct List<T> {
    page: Box<PageOp<T>>,
    max_pages: usize,
}

impl<T> List<T> {
    fn new(page: impl Fn(Option<&str>) -> Op<Page<T>, Read> + 'static) -> Self {
        Self { page: Box::new(page), max_pages: MAX_PAGES }
    }

    /// Fails instead of reading more than `pages` pages.
    pub fn max_pages(self, pages: usize) -> Self {
        Self { max_pages: pages, ..self }
    }

    pub fn name(&self) -> &'static str {
        (self.page)(None).name
    }

    pub fn method(&self) -> &'static str {
        (self.page)(None).method
    }

    pub fn path(&self) -> String {
        (self.page)(None).path
    }

    pub fn safety(&self) -> Safety {
        Safety::Read
    }

    pub fn scopes(&self) -> Vec<Scope> {
        (self.page)(None).scopes().to_vec()
    }
}

enum Body {
    Empty,
    Json(Value),
    Multipart(Vec<(&'static str, Part)>),
}

enum Part {
    Text(String),
    File(Image),
}

/// A reply field that is missing, null or of the wrong type, by its path in the reply.
struct Bad {
    field: String,
    problem: &'static str,
}

type Parsed<T> = Result<T, Bad>;

/// One request. `T` is the parsed reply and `S` the retry rule.
pub struct Op<T, S> {
    name: &'static str,
    method: &'static str,
    origin: &'static str,
    path: String,
    query: Vec<(&'static str, String)>,
    auth: Auth,
    response: Payload<()>,
    body: Body,
    parse: fn(&Value) -> Parsed<T>,
    /// The reply field that holds the ID of what a create made, which a failure keeps so a caller can adopt it.
    id_field: Option<&'static str>,
    retry: PhantomData<S>,
}

impl<T, S: Retry> Op<T, S> {
    /// The registry's name for the operation, or a name in its style for an endpoint outside the registry.
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn method(&self) -> &'static str {
        self.method
    }

    pub fn safety(&self) -> Safety {
        S::SAFETY
    }

    pub fn scopes(&self) -> &[Scope] {
        self.auth.required_scopes().unwrap_or_default()
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    fn failed(&self, mut failure: Failure, acted: bool) -> Failure {
        if S::SAFETY != Safety::Read && acted && failure.code != 5 {
            failure.code = 5;
            failure.error["kind"] = json!("outcome_unknown");
            failure.error["hint"] = json!(UNKNOWN_HINT);
        }
        failure.with("operation", json!(self.name))
    }

    fn malformed(&self, bad: Bad, reply: Value) -> Failure {
        let message = format!("{}: Roblox's reply {} `{}`", self.name, bad.problem, bad.field);
        let failure = if S::SAFETY == Safety::Read {
            Failure::unavailable(message)
        } else {
            Failure::unknown(format!("{message}, but its status was a success, so the write may have landed"))
        };
        let id = self.id_field.map(|field| &reply[field]).and_then(|id| {
            id.as_u64().or_else(|| id.as_str().and_then(|id| id.parse().ok())).and_then(NonZeroU64::new)
        });
        let failure = failure.with("operation", json!(self.name)).with("field", json!(bad.field));
        let failure = match id {
            Some(id) => failure.with("id", json!(id)),
            None => failure,
        };
        failure.with("detail", reply)
    }
}

impl<T, S: Retry> Request for Op<T, S> {
    fn operation(&self) -> Operation<'_> {
        Operation {
            origin: self.origin,
            path: &self.path,
            auth: &self.auth,
            safety: S::SAFETY,
            retry: None,
            response: self.response.clone(),
            timeout: Duration::from_secs(60),
        }
    }

    fn build(&self, origin: &str) -> Result<ureq::http::Request<Vec<u8>>, Failure> {
        let mut url = url::Url::parse(&format!("{origin}{}", self.path))
            .map_err(|_| Failure::usage("cannot construct the request URL"))?;
        if !self.query.is_empty() {
            url.query_pairs_mut().extend_pairs(&self.query);
        }
        let request = ureq::http::Request::builder().method(self.method).uri(url.as_str());
        let (request, bytes) = match &self.body {
            Body::Empty => (request, vec![]),
            Body::Json(value) => (
                request.header("content-type", "application/json"),
                serde_json::to_vec(value).expect("a JSON value serializes"),
            ),
            Body::Multipart(parts) => {
                let boundary = "crucible-iac-boundary-7f3a9c1e";
                let mut bytes = vec![];
                for (name, part) in parts {
                    bytes.extend_from_slice(
                        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"").as_bytes(),
                    );
                    match part {
                        Part::Text(text) => {
                            bytes.extend_from_slice(b"\r\n\r\n");
                            bytes.extend_from_slice(text.as_bytes());
                        }
                        Part::File(image) => {
                            let file_name = image.file_name.replace(['"', '\r', '\n'], "_");
                            bytes.extend_from_slice(
                                format!("; filename=\"{file_name}\"\r\nContent-Type: {}\r\n\r\n", image.content_type)
                                    .as_bytes(),
                            );
                            bytes.extend_from_slice(&image.bytes);
                        }
                    }
                    bytes.extend_from_slice(b"\r\n");
                }
                bytes.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
                (request.header("content-type", format!("multipart/form-data; boundary={boundary}")), bytes)
            }
        };
        request.body(bytes).map_err(|_| Failure::usage("cannot construct the HTTP request"))
    }

    fn until(&self) -> Option<Instant> {
        None
    }
}

impl Transport<'_> {
    pub fn run<T, S: Retry>(&self, op: &Op<T, S>) -> Result<T, Failure> {
        let mut acted = false;
        let value = match self.send_noting(op, &mut acted).map_err(|failure| op.failed(failure, acted))? {
            Reply::Json(value) => value,
            Reply::Empty => Value::Null,
        };
        (op.parse)(&value).map_err(|bad| op.malformed(bad, value))
    }

    /// Reads every page of `list`, following its tokens to the end. A token Roblox sends twice, or more pages than the
    /// list's cap, fails rather than returning part of the list, because provisioning reads "not listed" as "create".
    pub fn list_all<T>(&self, list: &List<T>) -> Result<Vec<T>, Failure> {
        let (mut items, mut seen, mut token) = (vec![], std::collections::BTreeSet::new(), None::<String>);
        for _ in 0..list.max_pages {
            let op = (list.page)(token.as_deref());
            let page = self.run(&op)?;
            items.extend(page.items);
            let Some(next) = page.next else { return Ok(items) };
            if !seen.insert(next.clone()) {
                return Err(Failure::unavailable(format!("{}: Roblox sent the page token {next:?} twice", op.name))
                    .with("operation", json!(op.name)));
            }
            token = Some(next);
        }
        let name = list.name();
        Err(Failure::usage(format!(
            "{name}: the list has more than {} pages, so it was not read whole",
            list.max_pages
        ))
        .with("operation", json!(name)))
    }
}

fn key(scopes: &[(&str, Option<&str>)]) -> Auth {
    Auth::ApiKey {
        scopes: Some(
            scopes
                .iter()
                .map(|(name, target)| Scope { name: (*name).into(), target: target.map(Into::into) })
                .collect(),
        ),
    }
}

fn op<T, S>(
    name: &'static str,
    method: &'static str,
    origin: &'static str,
    path: String,
    auth: Auth,
    body: Body,
    parse: fn(&Value) -> Parsed<T>,
) -> Op<T, S> {
    let response = Payload::Json { shape: () };
    Op { name, method, origin, path, query: vec![], auth, response, body, parse, id_field: None, retry: PhantomData }
}

/// A write whose reply nothing reads, so any 2xx is its success, whatever the body holds.
fn write<S>(name: &'static str, method: &'static str, path: String, auth: Auth, body: Body) -> Op<(), S> {
    let mut op = op(name, method, APIS, path, auth, body, |_| Ok(()));
    op.response = Payload::Empty;
    op
}

fn bad(field: &str) -> Bad {
    Bad { field: field.into(), problem: "has no valid" }
}

/// The value at a dotted path, which must be present and not null.
fn at<'a>(value: &'a Value, field: &str) -> Parsed<&'a Value> {
    field
        .split('.')
        .try_fold(value, |value, key| value.get(key).filter(|value| !value.is_null()))
        .ok_or_else(|| bad(field))
}

fn text(value: &Value, field: &str) -> Parsed<String> {
    at(value, field)?.as_str().map(str::to_owned).ok_or_else(|| bad(field))
}

fn flag(value: &Value, field: &str) -> Parsed<bool> {
    at(value, field)?.as_bool().ok_or_else(|| bad(field))
}

fn number(value: &Value, field: &str) -> Parsed<u64> {
    at(value, field)?.as_u64().ok_or_else(|| bad(field))
}

fn id(value: &Value, field: &str) -> Parsed<Id> {
    NonZeroU64::new(number(value, field)?).ok_or_else(|| bad(field))
}

/// The Robux price. The registry requires both fields and lets either be null, which means no price is set.
fn price(value: &Value) -> Parsed<Option<u64>> {
    const FIELD: &str = "priceInformation.defaultPriceInRobux";
    let price = match value.get("priceInformation").ok_or_else(|| bad(FIELD))? {
        Value::Null => return Ok(None),
        information => information.get("defaultPriceInRobux").ok_or_else(|| bad(FIELD))?,
    };
    match price {
        Value::Null => Ok(None),
        price => price.as_u64().map(Some).ok_or_else(|| bad(FIELD)),
    }
}

/// An icon asset ID, where 0 or no field means no icon. Nothing compares icons yet, so this field stays lenient.
fn icon(value: &Value, field: &str) -> Option<u64> {
    value[field].as_u64().filter(|id| *id != 0)
}

fn parts(details: &Details) -> Vec<(&'static str, Part)> {
    let mut parts = vec![];
    if let Some(name) = &details.name {
        parts.push(("name", Part::Text(name.clone())));
    }
    if let Some(description) = &details.description {
        parts.push(("description", Part::Text(description.clone())));
    }
    if let Some(price) = details.price {
        parts.push(("price", Part::Text(price.to_string())));
    }
    if let Some(for_sale) = details.for_sale {
        parts.push(("isForSale", Part::Text(for_sale.to_string())));
    }
    if let Some(icon) = &details.icon {
        parts.push(("imageFile", Part::File(icon.clone())));
    }
    parts
}

/// Reads one page. Every page must hold the `next` field, so a misnamed field fails the list instead of ending it
/// after one page. A null or empty token ends the list.
fn page<T>(value: &Value, items: &str, next: &str, item: fn(&Value) -> Parsed<T>) -> Parsed<Page<T>> {
    let list = at(value, items)?.as_array().ok_or_else(|| bad(items))?;
    let items = list
        .iter()
        .enumerate()
        .map(|(index, value)| {
            item(value).map_err(|bad| Bad { field: format!("{items}[{index}].{}", bad.field), ..bad })
        })
        .collect::<Parsed<Vec<T>>>()?;
    let next = match value.get(next).ok_or_else(|| bad(next))? {
        Value::Null => None,
        token => Some(token.as_str().ok_or_else(|| bad(next))?.to_owned()),
    };
    Ok(Page { items, next: next.filter(|token| !token.is_empty()) })
}

pub mod products {
    use super::*;

    const READ: &[(&str, Option<&str>)] = &[("developer-product:read", Some("universes"))];
    const WRITE: &[(&str, Option<&str>)] = &[("developer-product:write", Some("universes"))];

    fn product(value: &Value) -> Parsed<Sellable> {
        Ok(Sellable {
            id: id(value, "productId")?,
            name: text(value, "name")?,
            description: text(value, "description")?,
            price: price(value)?,
            for_sale: flag(value, "isForSale")?,
            icon: icon(value, "iconImageAssetId"),
        })
    }

    pub fn list(universe: Id) -> List<Sellable> {
        List::new(move |token| {
            let mut op = op(
                "developer-products.list",
                "GET",
                APIS,
                format!("/developer-products/v2/universes/{universe}/developer-products/creator"),
                key(READ),
                Body::Empty,
                |value| page(value, "developerProducts", "nextPageToken", product),
            );
            op.query.extend(token.map(|token| ("pageToken", token.to_owned())));
            op
        })
    }

    pub fn get(universe: Id, product_id: Id) -> Op<Sellable, Read> {
        op(
            "developer-products.get",
            "GET",
            APIS,
            format!("/developer-products/v2/universes/{universe}/developer-products/{product_id}/creator"),
            key(READ),
            Body::Empty,
            product,
        )
    }

    pub fn create(universe: Id, details: &Details) -> Op<Sellable, Unsafe> {
        let mut op = op(
            "developer-products.create",
            "POST",
            APIS,
            format!("/developer-products/v2/universes/{universe}/developer-products"),
            key(WRITE),
            Body::Multipart(parts(details)),
            product,
        );
        op.id_field = Some("productId");
        op
    }

    pub fn update(universe: Id, product_id: Id, details: &Details) -> Op<(), Idempotent> {
        write(
            "developer-products.update",
            "PATCH",
            format!("/developer-products/v2/universes/{universe}/developer-products/{product_id}"),
            key(WRITE),
            Body::Multipart(parts(details)),
        )
    }
}

pub mod passes {
    use super::*;

    const READ: &[(&str, Option<&str>)] = &[("game-pass:read", Some("universes"))];
    const WRITE: &[(&str, Option<&str>)] = &[("game-pass:write", Some("universes"))];

    fn pass(value: &Value) -> Parsed<Sellable> {
        Ok(Sellable {
            id: id(value, "gamePassId")?,
            name: text(value, "name")?,
            description: text(value, "description")?,
            price: price(value)?,
            for_sale: flag(value, "isForSale")?,
            icon: icon(value, "iconAssetId"),
        })
    }

    pub fn list(universe: Id) -> List<Sellable> {
        List::new(move |token| {
            let mut op = op(
                "game-passes.list",
                "GET",
                APIS,
                format!("/game-passes/v1/universes/{universe}/game-passes/creator"),
                key(READ),
                Body::Empty,
                |value| page(value, "gamePasses", "nextPageToken", pass),
            );
            op.query.extend(token.map(|token| ("pageToken", token.to_owned())));
            op
        })
    }

    pub fn get(universe: Id, pass_id: Id) -> Op<Sellable, Read> {
        op(
            "game-passes.get",
            "GET",
            APIS,
            format!("/game-passes/v1/universes/{universe}/game-passes/{pass_id}/creator"),
            key(READ),
            Body::Empty,
            pass,
        )
    }

    pub fn create(universe: Id, details: &Details) -> Op<Sellable, Unsafe> {
        let mut op = op(
            "game-passes.create",
            "POST",
            APIS,
            format!("/game-passes/v1/universes/{universe}/game-passes"),
            key(WRITE),
            Body::Multipart(parts(details)),
            pass,
        );
        op.id_field = Some("gamePassId");
        op
    }

    pub fn update(universe: Id, pass_id: Id, details: &Details) -> Op<(), Idempotent> {
        write(
            "game-passes.update",
            "PATCH",
            format!("/game-passes/v1/universes/{universe}/game-passes/{pass_id}"),
            key(WRITE),
            Body::Multipart(parts(details)),
        )
    }
}

pub mod badges {
    use super::*;

    fn badge(value: &Value) -> Parsed<Badge> {
        Ok(Badge {
            id: id(value, "id")?,
            name: text(value, "name")?,
            // The legacy schema marks no field required. Null reads as no description; a missing field is an error.
            description: match value.get("description") {
                Some(Value::Null) => String::new(),
                _ => text(value, "description")?,
            },
            enabled: flag(value, "enabled")?,
            icon: icon(value, "iconImageId"),
        })
    }

    /// Sends `expectedCost` 0, so Roblox refuses a create that would charge Robux instead of charging it.
    /// The registry makes the icon optional, as `crucible.toml` does.
    pub fn create(
        universe: Id,
        name: &str,
        description: &str,
        enabled: bool,
        icon: Option<Image>,
    ) -> Op<Badge, Unsafe> {
        let mut parts = vec![
            ("name", Part::Text(name.into())),
            ("description", Part::Text(description.into())),
            ("isActive", Part::Text(enabled.to_string())),
            ("expectedCost", Part::Text("0".into())),
        ];
        parts.extend(icon.map(|icon| ("files", Part::File(icon))));
        let mut op = op(
            "badges.create",
            "POST",
            APIS,
            format!("/legacy-badges/v1/universes/{universe}/badges"),
            key(&[("legacy-universe.badge:manage-and-spend-robux", None)]),
            Body::Multipart(parts),
            badge,
        );
        op.id_field = Some("id");
        op
    }

    /// The metadata `PATCH` takes only the name, the description and the enabled state.
    pub fn update(badge_id: Id, details: &BadgeDetails) -> Op<(), Idempotent> {
        let mut body = serde_json::Map::new();
        if let Some(name) = &details.name {
            body.insert("name".into(), json!(name));
        }
        if let Some(description) = &details.description {
            body.insert("description".into(), json!(description));
        }
        if let Some(enabled) = details.enabled {
            body.insert("enabled".into(), json!(enabled));
        }
        let scopes = [("legacy-universe.badge:write", None), ("legacy-universe.badge:manage-and-spend-robux", None)];
        write(
            "badges.update",
            "PATCH",
            format!("/legacy-badges/v1/badges/{badge_id}"),
            key(&scopes),
            Body::Json(Value::Object(body)),
        )
    }

    pub fn overwrite_icon(badge_id: Id, icon: Image) -> Op<(), Idempotent> {
        write(
            "publish.overwrite-badge-icon",
            "POST",
            format!("/legacy-publish/v1/badges/{badge_id}/icon"),
            key(&[("legacy-badge:manage", None)]),
            Body::Multipart(vec![("Files", Part::File(icon))]),
        )
    }

    /// The pinned registry has no badge read by ID, so this reads the documented public endpoint, which needs no key.
    /// It answers for a badge of any universe, so the reply must name the awarding universe.
    pub fn get_public(badge_id: Id) -> Op<AwardedBadge, Read> {
        op("badges.get-public", "GET", BADGES, format!("/v1/badges/{badge_id}"), Auth::None, Body::Empty, |value| {
            Ok(AwardedBadge { badge: badge(value)?, universe: id(value, "awardingUniverse.id")? })
        })
    }

    /// Badges have no API-key list. This documented public endpoint needs no key, so it sends none.
    pub fn list_public(universe: Id) -> List<Badge> {
        const LIMIT: usize = 100;
        List::new(move |cursor| {
            let mut op = op(
                "badges.list-public",
                "GET",
                BADGES,
                format!("/v1/universes/{universe}/badges"),
                Auth::None,
                Body::Empty,
                |value| page(value, "data", "nextPageCursor", badge),
            );
            op.query.push(("limit", LIMIT.to_string()));
            op.query.push(("sortOrder", "Asc".into()));
            op.query.extend(cursor.map(|cursor| ("cursor", cursor.to_owned())));
            op
        })
    }
}

pub mod universes {
    use super::*;

    /// Writes the settings that are set, and names each in `updateMask`, the registry's "list of fields to update", so
    /// Roblox leaves every other field alone. `None` when no setting is set, because an empty mask could read as every
    /// field. The registry marks the operation unsafe, as it does every `PATCH` it has no rule for. The body holds
    /// absolute values under the mask, so a repeat leaves the same state, and it is `Idempotent` here.
    pub fn update(universe: Id, settings: &UniverseSettings) -> Option<Op<(), Idempotent>> {
        let mut body = serde_json::Map::new();
        if let Some(price) = settings.private_server_price {
            body.insert("privateServerPriceRobux".into(), json!(price));
        }
        if let Some(enabled) = settings.voice_chat {
            body.insert("voiceChatEnabled".into(), json!(enabled));
        }
        if body.is_empty() {
            return None;
        }
        let mask = body.keys().cloned().collect::<Vec<_>>().join(",");
        let mut op = write(
            "universes.update",
            "PATCH",
            format!("/cloud/v2/universes/{universe}"),
            key(&[("universe:write", Some("universes"))]),
            Body::Json(Value::Object(body)),
        );
        op.query.push(("updateMask", mask));
        Some(op)
    }
}

pub mod api_keys {
    use super::*;

    /// What the key that `Transport` sends can do. The key goes in the body, so the call needs no scope.
    pub fn introspect() -> Op<Introspection, Read> {
        op(
            "api-keys.introspect",
            "POST",
            APIS,
            "/api-keys/v1/introspect".into(),
            Auth::BodyKey { field: "apiKey".into() },
            Body::Empty,
            |value| {
                text(value, "name")?;
                flag(value, "enabled")?;
                flag(value, "expired")?;
                at(value, "scopes")?.as_array().ok_or_else(|| bad("scopes"))?;
                Introspection::parse(value.clone()).map_err(|_| bad("scopes"))
            },
        )
    }
}
