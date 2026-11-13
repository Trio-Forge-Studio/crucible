use crucible_iac::model::{Fields, Kind};
use crucible_iac::oc::Roblox;
use crucible_iac::plan::Listed;
use crucible_iac::provision::{Patch, Remote};
use crucible_oc::auth::Secret;
use crucible_oc::ops::Image;
use crucible_oc::transport::{Clock, Event, Sink, Timing, Transport};
use httpmock::{Method, MockServer};
use serde_json::{Value, json};
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

struct Quiet;
impl Sink for Quiet {
    fn event(&self, _: Event<'_>) {}
}
struct NoWait;
impl Clock for NoWait {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, _: Duration) {}
}

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn transport<'a>(server: &MockServer, key: &'a Secret) -> Transport<'a> {
    let timing =
        Timing { poll_interval: Duration::ZERO, deadline: Duration::from_secs(60), retry_delay: Some(Duration::ZERO) };
    Transport::new(Some(server.base_url()), Some(key), timing, None, Box::new(Quiet), Box::new(NoWait)).unwrap()
}

fn icon() -> Image {
    Image { file_name: "icon.png".into(), content_type: "image/png".into(), bytes: b"\x89PNG".to_vec() }
}

fn product(id: u64, price: Value) -> Value {
    json!({"productId": id, "name": "Coin pack", "description": "Coins.", "isForSale": true,
           "priceInformation": {"defaultPriceInRobux": price}})
}

fn pass(id: u64) -> Value {
    json!({"gamePassId": id, "name": "VIP", "description": "Double coins.", "isForSale": false,
           "priceInformation": {"defaultPriceInRobux": 250}})
}

fn badge(id: u64) -> Value {
    json!({"id": id, "name": "First delivery", "description": "Deliver.", "enabled": false})
}

fn listed(id: u64, name: &str, description: &str, price: Option<u64>, on: bool) -> Listed {
    Listed { id: self::id(id), name: name.into(), description: description.into(), price, on }
}

#[test]
fn each_kind_is_listed_and_read_through_its_own_endpoint() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(Method::GET).path("/developer-products/v2/universes/1/developer-products/creator");
        then.status(200).json_body(json!({"developerProducts": [product(3, json!(49))], "nextPageToken": null}));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/game-passes/v1/universes/1/game-passes/creator");
        then.status(200).json_body(json!({"gamePasses": [pass(4)], "nextPageToken": null}));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/v1/universes/1/badges");
        then.status(200).json_body(json!({"data": [badge(5)], "nextPageCursor": null}));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/developer-products/v2/universes/1/developer-products/6/creator");
        then.status(200).json_body(product(6, Value::Null));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/game-passes/v1/universes/1/game-passes/7/creator");
        then.status(200).json_body(pass(7));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/v1/badges/8");
        then.status(200).json_body(awarded(badge(8), 1));
    });
    let key = Secret::from_string("test-key".into());
    let transport = transport(&server, &key);
    let mut roblox = Roblox { transport: &transport, universe: id(1) };

    assert_eq!(roblox.list(Kind::Product).unwrap(), [listed(3, "Coin pack", "Coins.", Some(49), true)]);
    assert_eq!(roblox.list(Kind::Pass).unwrap(), [listed(4, "VIP", "Double coins.", Some(250), false)]);
    assert_eq!(roblox.list(Kind::Badge).unwrap(), [listed(5, "First delivery", "Deliver.", None, false)]);
    assert_eq!(roblox.get(Kind::Product, id(6)).unwrap(), Some(listed(6, "Coin pack", "Coins.", None, true)));
    assert_eq!(roblox.get(Kind::Pass, id(7)).unwrap(), Some(listed(7, "VIP", "Double coins.", Some(250), false)));
    assert_eq!(roblox.get(Kind::Badge, id(8)).unwrap(), Some(listed(8, "First delivery", "Deliver.", None, false)));
}

fn awarded(mut badge: Value, universe: u64) -> Value {
    badge["awardingUniverse"] = json!({"id": universe, "name": "A game", "rootPlaceId": 2});
    badge
}

/// Roblox reads a badge by its ID in any universe, so one another universe awards reads as none, and a reply that does
/// not say which universe awards it is a failure.
#[test]
fn a_badge_another_universe_awards_reads_as_none() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(Method::GET).path("/v1/badges/555");
        then.status(200).json_body(awarded(badge(555), 999));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/v1/badges/556");
        then.status(200).json_body(badge(556));
    });
    let key = Secret::from_string("test-key".into());
    let transport = transport(&server, &key);
    let mut roblox = Roblox { transport: &transport, universe: id(1) };
    assert_eq!(roblox.get(Kind::Badge, id(555)).unwrap(), None);
    let failure = roblox.get(Kind::Badge, id(556)).unwrap_err();
    assert_eq!((failure.code, failure.error["field"].as_str()), (7, Some("awardingUniverse.id")));
}

#[test]
fn only_a_404_reads_as_a_resource_roblox_lacks() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(Method::GET).path("/v1/badges/8");
        then.status(404).json_body(json!({"errors": [{"code": 1, "message": "Badge is invalid or does not exist."}]}));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/game-passes/v1/universes/1/game-passes/7/creator");
        then.status(403).json_body(json!({"errorMessage": "Forbidden"}));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/developer-products/v2/universes/1/developer-products/6/creator");
        then.status(400).json_body(json!({"errorMessage": "Bad request"}));
    });
    let key = Secret::from_string("test-key".into());
    let transport = transport(&server, &key);
    let mut roblox = Roblox { transport: &transport, universe: id(1) };

    assert_eq!(roblox.get(Kind::Badge, id(8)).unwrap(), None);
    let forbidden = roblox.get(Kind::Pass, id(7)).unwrap_err();
    assert_eq!((forbidden.code, forbidden.error["status"].as_u64()), (3, Some(403)));
    let rejected = roblox.get(Kind::Product, id(6)).unwrap_err();
    assert_eq!((rejected.code, rejected.error["status"].as_u64()), (4, Some(400)));
}

#[test]
fn a_create_sends_the_declared_fields_on_sale_and_returns_the_new_id() {
    let server = MockServer::start();
    let product_create = server.mock(|when, then| {
        when.method(Method::POST)
            .path("/developer-products/v2/universes/1/developer-products")
            .body_includes("name=\"name\"\r\n\r\nCoin pack\r\n")
            .body_includes("name=\"price\"\r\n\r\n49\r\n")
            .body_includes("name=\"isForSale\"\r\n\r\ntrue\r\n")
            .body_includes("filename=\"icon.png\"");
        then.status(200).json_body(product(30, json!(49)));
    });
    let pass_create = server.mock(|when, then| {
        when.method(Method::POST)
            .path("/game-passes/v1/universes/1/game-passes")
            .body_includes("name=\"name\"\r\n\r\nVIP\r\n")
            .body_includes("name=\"isForSale\"\r\n\r\ntrue\r\n");
        then.status(200).json_body(pass(40));
    });
    let badge_create = server.mock(|when, then| {
        when.method(Method::POST)
            .path("/legacy-badges/v1/universes/1/badges")
            .body_includes("name=\"name\"\r\n\r\nFirst delivery\r\n")
            .body_includes("name=\"isActive\"\r\n\r\nfalse\r\n");
        then.status(200).json_body(badge(50));
    });
    let key = Secret::from_string("test-key".into());
    let transport = transport(&server, &key);
    let mut roblox = Roblox { transport: &transport, universe: id(1) };

    let coins = Fields::Priced { name: "Coin pack".into(), description: "Coins.".into(), price: 49 };
    assert_eq!(roblox.create(Kind::Product, &coins, Some(&icon())).unwrap(), id(30));
    let vip = Fields::Priced { name: "VIP".into(), description: "".into(), price: 250 };
    assert_eq!(roblox.create(Kind::Pass, &vip, None).unwrap(), id(40));
    let first = Fields::Badge { name: "First delivery".into(), description: "".into(), enabled: false };
    assert_eq!(roblox.create(Kind::Badge, &first, None).unwrap(), id(50));
    product_create.assert_calls(1);
    pass_create.assert_calls(1);
    badge_create.assert_calls(1);
}

#[test]
fn an_update_writes_each_kind_through_its_own_endpoint() {
    let server = MockServer::start();
    let product_update = server.mock(|when, then| {
        when.method(Method::PATCH)
            .path("/developer-products/v2/universes/1/developer-products/3")
            .body_includes("name=\"price\"\r\n\r\n99\r\n");
        then.status(204);
    });
    let pass_update = server.mock(|when, then| {
        when.method(Method::PATCH)
            .path("/game-passes/v1/universes/1/game-passes/4")
            .body_includes("name=\"isForSale\"\r\n\r\nfalse\r\n");
        then.status(204);
    });
    let badge_update = server.mock(|when, then| {
        when.method(Method::PATCH).path("/legacy-badges/v1/badges/5").json_body(json!({"name": "Renamed"}));
        then.status(200).json_body(json!({}));
    });
    let badge_icon = server.mock(|when, then| {
        when.method(Method::POST).path("/legacy-publish/v1/badges/5/icon");
        then.status(200).json_body(json!({"targetId": 9}));
    });
    let key = Secret::from_string("test-key".into());
    let transport = transport(&server, &key);
    let mut roblox = Roblox { transport: &transport, universe: id(1) };

    roblox.update(Kind::Product, id(3), &Patch { price: Some(99), ..Patch::default() }).unwrap();
    roblox.update(Kind::Pass, id(4), &Patch { on: Some(false), ..Patch::default() }).unwrap();
    product_update.assert_calls(1);
    pass_update.assert_calls(1);

    roblox.update(Kind::Badge, id(5), &Patch { icon: Some(icon()), ..Patch::default() }).unwrap();
    badge_update.assert_calls(0);
    badge_icon.assert_calls(1);
    roblox.update(Kind::Badge, id(5), &Patch { name: Some("Renamed".into()), ..Patch::default() }).unwrap();
    badge_update.assert_calls(1);
    badge_icon.assert_calls(1);
}
