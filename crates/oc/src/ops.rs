//! The Open Cloud calls provisioning makes for developer products, game passes and badges, as typed requests and
//! replies over `Transport`. An operation's retry rule is its type's `S` parameter, not a registry lookup: a create
//! is `Unsafe`, because Roblox takes no idempotency key and cannot delete a duplicate, so the outcome judge never
//! repeats it after a lost answer. An update writes absolute values, so repeating it leaves the same state.

use crate::outcome::Failure;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sellable {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub price: u64,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
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

/// One request. `T` is the parsed reply and `S` the retry rule.
pub struct Op<T, S> {
    method: &'static str,
    origin: &'static str,
    path: String,
    query: Vec<(&'static str, String)>,
    auth: Auth,
    response: Payload<()>,
    body: Body,
    parse: fn(Value) -> Result<T, Failure>,
    retry: PhantomData<S>,
}

impl<T, S: Retry> Op<T, S> {
    pub fn safety(&self) -> Safety {
        S::SAFETY
    }

    pub fn scopes(&self) -> &[Scope] {
        self.auth.required_scopes().unwrap_or_default()
    }

    pub fn path(&self) -> &str {
        &self.path
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
        let value = match self.send(op)? {
            Reply::Json(value) => value,
            Reply::Empty => Value::Null,
        };
        (op.parse)(value)
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
    method: &'static str,
    origin: &'static str,
    path: String,
    auth: Auth,
    body: Body,
    parse: fn(Value) -> Result<T, Failure>,
) -> Op<T, S> {
    let response = if matches!(method, "PATCH") { Payload::Empty } else { Payload::Json { shape: () } };
    Op { method, origin, path, query: vec![], auth, response, body, parse, retry: PhantomData }
}

fn invalid(what: &str) -> Failure {
    Failure::unavailable(format!("Roblox returned an invalid {what}"))
}

fn nothing(_: Value) -> Result<(), Failure> {
    Ok(())
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

fn page<T>(value: Value, items: &str, next: &str, item: fn(&Value) -> Option<T>) -> Result<Page<T>, Failure> {
    let list = value[items].as_array().ok_or_else(|| invalid("page"))?;
    let items = list.iter().map(item).collect::<Option<Vec<T>>>().ok_or_else(|| invalid("page item"))?;
    let next = value[next].as_str().filter(|token| !token.is_empty()).map(str::to_owned);
    Ok(Page { items, next })
}

fn id(value: &Value) -> Option<Id> {
    value.as_u64().and_then(NonZeroU64::new)
}

pub mod products {
    use super::*;

    const READ: &[(&str, Option<&str>)] = &[("developer-product:read", Some("universes"))];
    const WRITE: &[(&str, Option<&str>)] = &[("developer-product:write", Some("universes"))];

    fn product(value: &Value) -> Option<Sellable> {
        Some(Sellable {
            id: id(&value["productId"])?,
            name: value["name"].as_str()?.into(),
            description: value["description"].as_str().unwrap_or_default().into(),
            price: value["priceInformation"]["defaultPriceInRobux"].as_u64().unwrap_or(0),
            for_sale: value["isForSale"].as_bool()?,
            icon: value["iconImageAssetId"].as_u64().filter(|id| *id != 0),
        })
    }

    pub fn list(universe: Id, token: Option<&str>) -> Op<Page<Sellable>, Read> {
        let mut op = op(
            "GET",
            APIS,
            format!("/developer-products/v2/universes/{universe}/developer-products/creator"),
            key(READ),
            Body::Empty,
            |value| page(value, "developerProducts", "nextPageToken", product),
        );
        op.query.extend(token.map(|token| ("pageToken", token.to_owned())));
        op
    }

    pub fn get(universe: Id, product_id: Id) -> Op<Sellable, Read> {
        op(
            "GET",
            APIS,
            format!("/developer-products/v2/universes/{universe}/developer-products/{product_id}/creator"),
            key(READ),
            Body::Empty,
            |value| product(&value).ok_or_else(|| invalid("developer product")),
        )
    }

    pub fn create(universe: Id, details: &Details) -> Op<Sellable, Unsafe> {
        op(
            "POST",
            APIS,
            format!("/developer-products/v2/universes/{universe}/developer-products"),
            key(WRITE),
            Body::Multipart(parts(details)),
            |value| product(&value).ok_or_else(|| invalid("developer product")),
        )
    }

    pub fn update(universe: Id, product_id: Id, details: &Details) -> Op<(), Idempotent> {
        op(
            "PATCH",
            APIS,
            format!("/developer-products/v2/universes/{universe}/developer-products/{product_id}"),
            key(WRITE),
            Body::Multipart(parts(details)),
            nothing,
        )
    }
}

pub mod passes {
    use super::*;

    const READ: &[(&str, Option<&str>)] = &[("game-pass:read", Some("universes"))];
    const WRITE: &[(&str, Option<&str>)] = &[("game-pass:write", Some("universes"))];

    fn pass(value: &Value) -> Option<Sellable> {
        Some(Sellable {
            id: id(&value["gamePassId"])?,
            name: value["name"].as_str()?.into(),
            description: value["description"].as_str().unwrap_or_default().into(),
            price: value["priceInformation"]["defaultPriceInRobux"].as_u64().unwrap_or(0),
            for_sale: value["isForSale"].as_bool()?,
            icon: value["iconAssetId"].as_u64().filter(|id| *id != 0),
        })
    }

    pub fn list(universe: Id, token: Option<&str>) -> Op<Page<Sellable>, Read> {
        let mut op = op(
            "GET",
            APIS,
            format!("/game-passes/v1/universes/{universe}/game-passes/creator"),
            key(READ),
            Body::Empty,
            |value| page(value, "gamePasses", "nextPageToken", pass),
        );
        op.query.extend(token.map(|token| ("pageToken", token.to_owned())));
        op
    }

    pub fn get(universe: Id, pass_id: Id) -> Op<Sellable, Read> {
        op(
            "GET",
            APIS,
            format!("/game-passes/v1/universes/{universe}/game-passes/{pass_id}/creator"),
            key(READ),
            Body::Empty,
            |value| pass(&value).ok_or_else(|| invalid("game pass")),
        )
    }

    pub fn create(universe: Id, details: &Details) -> Op<Sellable, Unsafe> {
        op(
            "POST",
            APIS,
            format!("/game-passes/v1/universes/{universe}/game-passes"),
            key(WRITE),
            Body::Multipart(parts(details)),
            |value| pass(&value).ok_or_else(|| invalid("game pass")),
        )
    }

    pub fn update(universe: Id, pass_id: Id, details: &Details) -> Op<(), Idempotent> {
        op(
            "PATCH",
            APIS,
            format!("/game-passes/v1/universes/{universe}/game-passes/{pass_id}"),
            key(WRITE),
            Body::Multipart(parts(details)),
            nothing,
        )
    }
}

pub mod badges {
    use super::*;

    fn badge(value: &Value) -> Option<Badge> {
        Some(Badge {
            id: id(&value["id"])?,
            name: value["name"].as_str()?.into(),
            description: value["description"].as_str().unwrap_or_default().into(),
            enabled: value["enabled"].as_bool()?,
            icon: value["iconImageId"].as_u64().filter(|id| *id != 0),
        })
    }

    /// Sends `expectedCost` 0, so Roblox refuses a create that would charge Robux instead of charging it.
    pub fn create(universe: Id, name: &str, description: &str, enabled: bool, icon: Image) -> Op<Badge, Unsafe> {
        let parts = vec![
            ("name", Part::Text(name.into())),
            ("description", Part::Text(description.into())),
            ("isActive", Part::Text(enabled.to_string())),
            ("expectedCost", Part::Text("0".into())),
            ("files", Part::File(icon)),
        ];
        op(
            "POST",
            APIS,
            format!("/legacy-badges/v1/universes/{universe}/badges"),
            key(&[("legacy-universe.badge:manage-and-spend-robux", None)]),
            Body::Multipart(parts),
            |value| badge(&value).ok_or_else(|| invalid("badge")),
        )
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
        op(
            "PATCH",
            APIS,
            format!("/legacy-badges/v1/badges/{badge_id}"),
            key(&scopes),
            Body::Json(Value::Object(body)),
            nothing,
        )
    }

    pub fn overwrite_icon(badge_id: Id, icon: Image) -> Op<(), Idempotent> {
        op(
            "POST",
            APIS,
            format!("/legacy-publish/v1/badges/{badge_id}/icon"),
            key(&[("legacy-badge:manage", None)]),
            Body::Multipart(vec![("Files", Part::File(icon))]),
            nothing,
        )
    }

    /// Badges have no API-key list. This documented public endpoint needs no key, so it sends none.
    pub fn list_public(universe: Id, cursor: Option<&str>) -> Op<Page<Badge>, Read> {
        let mut op = op("GET", BADGES, format!("/v1/universes/{universe}/badges"), Auth::None, Body::Empty, |value| {
            page(value, "data", "nextPageCursor", badge)
        });
        op.query.push(("limit", "100".into()));
        op.query.push(("sortOrder", "Asc".into()));
        op.query.extend(cursor.map(|cursor| ("cursor", cursor.to_owned())));
        op
    }
}
