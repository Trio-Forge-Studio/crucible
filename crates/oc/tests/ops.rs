use crucible_oc::Safety;
use crucible_oc::auth::Secret;
use crucible_oc::ops::{BadgeDetails, Details, Image, Op, Retry, badges, passes, products};
use crucible_oc::outcome::Failure;
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
    Image { file_name: "coin-pack.png".into(), content_type: "image/png".into(), bytes: b"\x89PNG".to_vec() }
}

fn safety<T, S: Retry>(op: &Op<T, S>) -> Safety {
    op.safety()
}

fn product_json(price: u64) -> serde_json::Value {
    json!({"productId": 3301234567u64, "name": "Coin pack", "description": "100 coins.", "isForSale": true,
           "iconImageAssetId": 0, "priceInformation": {"defaultPriceInRobux": price, "enabledFeatures": []},
           "universeId": 1, "isImmutable": false, "isManagedPricingEnabled": false,
           "createdTimestamp": "2026-01-01T00:00:00Z", "updatedTimestamp": "2026-01-01T00:00:00Z"})
}

fn pass_json() -> Value {
    json!({"gamePassId": 1786543210u64, "name": "VIP", "description": "Double coins.", "isForSale": false,
           "iconAssetId": 81234, "priceInformation": {"defaultPriceInRobux": 250, "enabledFeatures": []},
           "isManagedPricingEnabled": false, "createdTimestamp": "2026-01-01T00:00:00Z",
           "updatedTimestamp": "2026-01-01T00:00:00Z"})
}

fn badge_json() -> Value {
    json!({"id": 2124000000000000u64, "name": "First delivery", "description": "Deliver.", "enabled": true,
           "iconImageId": 9, "displayName": "First delivery", "created": "2026-01-01T00:00:00Z"})
}

fn without(mut value: Value, path: &[&str]) -> Value {
    let (last, parents) = path.split_last().unwrap();
    parents.iter().fold(&mut value, |value, key| &mut value[*key]).as_object_mut().unwrap().remove(*last);
    value
}

#[test]
fn each_operation_carries_its_retry_rule_and_scopes() {
    let details = Details::default();
    assert_eq!(safety(&products::list(id(1), None)), Safety::Read);
    assert_eq!(safety(&products::get(id(1), id(2))), Safety::Read);
    assert_eq!(safety(&products::create(id(1), &details)), Safety::Unsafe);
    assert_eq!(safety(&products::update(id(1), id(2), &details)), Safety::Idempotent);
    assert_eq!(safety(&passes::list(id(1), None)), Safety::Read);
    assert_eq!(safety(&passes::get(id(1), id(2))), Safety::Read);
    assert_eq!(safety(&passes::create(id(1), &details)), Safety::Unsafe);
    assert_eq!(safety(&passes::update(id(1), id(2), &details)), Safety::Idempotent);
    assert_eq!(safety(&badges::create(id(1), "B", "", true, icon())), Safety::Unsafe);
    assert_eq!(safety(&badges::update(id(2), &BadgeDetails::default())), Safety::Idempotent);
    assert_eq!(safety(&badges::overwrite_icon(id(2), icon())), Safety::Idempotent);
    assert_eq!(safety(&badges::list_public(id(1), None)), Safety::Read);
    let scopes = |scopes: &[crucible_oc::Scope]| scopes.iter().map(|scope| scope.name.clone()).collect::<Vec<_>>();
    assert_eq!(scopes(products::create(id(1), &details).scopes()), ["developer-product:write"]);
    assert_eq!(scopes(passes::list(id(1), None).scopes()), ["game-pass:read"]);
    assert_eq!(scopes(badges::overwrite_icon(id(2), icon()).scopes()), ["legacy-badge:manage"]);
    assert!(badges::list_public(id(1), None).scopes().is_empty());
}

#[test]
fn a_product_create_sends_multipart_and_parses_the_product() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let mock = server.mock(|when, then| {
        when.method(Method::POST)
            .path("/developer-products/v2/universes/10772177432/developer-products")
            .header("x-api-key", "test-key")
            .body_includes("name=\"name\"\r\n\r\nCoin pack\r\n")
            .body_includes("name=\"price\"\r\n\r\n49\r\n")
            .body_includes("name=\"isForSale\"\r\n\r\ntrue\r\n")
            .body_includes("name=\"imageFile\"; filename=\"coin-pack.png\"\r\nContent-Type: image/png");
        then.status(200).json_body(product_json(49));
    });
    let details = Details {
        name: Some("Coin pack".into()),
        description: Some("100 coins.".into()),
        price: Some(49),
        for_sale: Some(true),
        icon: Some(icon()),
    };
    let product = transport(&server, &key).run(&products::create(id(10772177432), &details)).unwrap();
    mock.assert();
    assert_eq!((product.id.get(), product.price, product.for_sale, product.icon), (3301234567, 49, true, None));
}

#[test]
fn a_create_whose_answer_is_lost_is_sent_once_and_reported_unknown() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let mock = server.mock(|when, then| {
        when.method(Method::POST).path("/game-passes/v1/universes/1/game-passes");
        then.status(503);
    });
    let failure = transport(&server, &key).run(&passes::create(id(1), &Details::default())).unwrap_err();
    mock.assert_calls(1);
    assert_eq!((failure.code, failure.error["kind"].as_str()), (5, Some("outcome_unknown")));
}

struct Create {
    name: &'static str,
    id_field: &'static str,
    path: &'static str,
    reply: Value,
    compared: &'static [&'static str],
    send: fn(&Transport<'_>) -> Failure,
}

#[test]
fn a_create_that_got_a_success_status_is_never_reported_as_retry() {
    let creates = [
        Create {
            name: "developer-products.create",
            id_field: "productId",
            path: "/developer-products/v2/universes/1/developer-products",
            reply: product_json(49),
            compared: &["priceInformation", "defaultPriceInRobux"],
            send: |transport| transport.run(&products::create(id(1), &Details::default())).unwrap_err(),
        },
        Create {
            name: "game-passes.create",
            id_field: "gamePassId",
            path: "/game-passes/v1/universes/1/game-passes",
            reply: pass_json(),
            compared: &["priceInformation", "defaultPriceInRobux"],
            send: |transport| transport.run(&passes::create(id(1), &Details::default())).unwrap_err(),
        },
        Create {
            name: "badges.create",
            id_field: "id",
            path: "/legacy-badges/v1/universes/1/badges",
            reply: badge_json(),
            compared: &["enabled"],
            send: |transport| transport.run(&badges::create(id(1), "B", "", true, icon())).unwrap_err(),
        },
    ];
    for Create { name, id_field, path, reply: full, compared, send: create } in creates {
        let landed = full[id_field].as_u64().unwrap();
        let mut string_id = full.clone();
        string_id[id_field] = json!(landed.to_string());
        let compared_field = compared.join(".");
        for (case, status, body, field, kept) in [
            ("an empty 200", 200, None, id_field, None),
            ("a 204", 204, None, id_field, None),
            (
                "a reply missing a compared field",
                200,
                Some(without(full.clone(), compared)),
                &compared_field,
                Some(landed),
            ),
            ("an ID sent as a string", 200, Some(string_id), id_field, Some(landed)),
        ] {
            let server = MockServer::start();
            let key = Secret::from_string("test-key".into());
            let mock = server.mock(|when, then| {
                when.method(Method::POST).path(path);
                let then = then.status(status);
                if let Some(body) = &body {
                    then.json_body(body.clone());
                }
            });
            let failure = create(&transport(&server, &key));
            mock.assert_calls(1);
            let error = &failure.error;
            assert_eq!((failure.code, error["kind"].as_str()), (5, Some("outcome_unknown")), "{name}, {case}: {error}");
            assert_eq!((error["operation"].as_str(), error["field"].as_str()), (Some(name), Some(field)), "{case}");
            assert_eq!(error["id"].as_u64(), kept, "{name}, {case}: the ID a caller adopts by");
            assert!(error["message"].as_str().unwrap().contains(field), "{name}, {case}: {error}");
        }
    }
}

#[test]
fn an_update_that_may_have_landed_is_unknown() {
    let details = BadgeDetails { enabled: Some(false), ..BadgeDetails::default() };
    for (status, code) in [(503, 5), (500, 5), (429, 7)] {
        let server = MockServer::start();
        let key = Secret::from_string("test-key".into());
        let mock = server.mock(|when, then| {
            when.method(Method::PATCH).path("/legacy-badges/v1/badges/7").json_body(json!({"enabled": false}));
            then.status(status);
        });
        let failure = transport(&server, &key).run(&badges::update(id(7), &details)).unwrap_err();
        mock.assert_calls(3);
        assert_eq!(failure.code, code, "HTTP {status}: only a reply that proves the update was refused is unavailable");
        assert_eq!(failure.error["operation"], "badges.update");
    }
}

#[test]
fn an_update_answered_with_success_succeeds_whatever_the_body() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let icon_mock = server.mock(|when, then| {
        when.method(Method::POST).path("/legacy-publish/v1/badges/7/icon");
        then.status(200).body("<html>not JSON</html>");
    });
    let product_mock = server.mock(|when, then| {
        when.method(Method::PATCH).path("/developer-products/v2/universes/1/developer-products/2");
        then.status(200).body("{\"unexpected\": ");
    });
    let transport = transport(&server, &key);
    transport.run(&badges::overwrite_icon(id(7), icon())).unwrap();
    transport.run(&products::update(id(1), id(2), &Details { price: Some(5), ..Details::default() })).unwrap();
    icon_mock.assert_calls(1);
    product_mock.assert_calls(1);
}

#[test]
fn lists_follow_their_page_tokens() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let first = server.mock(|when, then| {
        when.method(Method::GET)
            .path("/developer-products/v2/universes/1/developer-products/creator")
            .query_param_missing("pageToken");
        then.status(200).json_body(json!({"developerProducts": [product_json(49)], "nextPageToken": "two"}));
    });
    let second = server.mock(|when, then| {
        when.method(Method::GET)
            .path("/developer-products/v2/universes/1/developer-products/creator")
            .query_param("pageToken", "two");
        then.status(200).json_body(json!({"developerProducts": [], "nextPageToken": ""}));
    });
    let transport = transport(&server, &key);
    let page = transport.run(&products::list(id(1), None)).unwrap();
    assert_eq!((page.items.len(), page.next.as_deref()), (1, Some("two")));
    let page = transport.run(&products::list(id(1), page.next.as_deref())).unwrap();
    assert_eq!((page.items.len(), page.next), (0, None));
    first.assert();
    second.assert();
}

#[test]
fn a_badge_create_never_offers_to_pay() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let mock = server.mock(|when, then| {
        when.method(Method::POST)
            .path("/legacy-badges/v1/universes/1/badges")
            .body_includes("name=\"expectedCost\"\r\n\r\n0\r\n")
            .body_includes("name=\"isActive\"\r\n\r\nfalse\r\n");
        then.status(200).json_body(json!({"id": 2124000000000000u64, "name": "First delivery", "description": "", "enabled": false, "iconImageId": 9}));
    });
    let badge = transport(&server, &key).run(&badges::create(id(1), "First delivery", "", false, icon())).unwrap();
    mock.assert();
    assert_eq!((badge.id.get(), badge.enabled, badge.icon), (2124000000000000, false, Some(9)));
}

#[test]
fn the_public_badge_list_sends_no_key() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let mock = server.mock(|when, then| {
        when.method(Method::GET)
            .path("/v1/universes/1/badges")
            .query_param("limit", "100")
            .query_param("sortOrder", "Asc")
            .header_missing("x-api-key");
        then.status(200).json_body(json!({"previousPageCursor": null, "nextPageCursor": null,
            "data": [{"id": 5, "name": "First delivery", "description": "Deliver.", "enabled": true, "iconImageId": 0}]}));
    });
    let page = transport(&server, &key).run(&badges::list_public(id(1), None)).unwrap();
    mock.assert();
    assert_eq!((page.items[0].name.as_str(), page.items[0].icon, page.next), ("First delivery", None, None));
}

#[test]
fn a_malformed_read_is_an_error_that_names_the_operation_and_the_field() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    server.mock(|when, then| {
        when.method(Method::GET).path("/game-passes/v1/universes/1/game-passes/2/creator");
        then.status(200).json_body(json!({"gamePassId": 2}));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/developer-products/v2/universes/1/developer-products/creator");
        let priceless = without(product_json(49), &["priceInformation", "defaultPriceInRobux"]);
        then.status(200).json_body(json!({"developerProducts": [product_json(49), priceless], "nextPageToken": ""}));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/developer-products/v2/universes/1/developer-products/3/creator");
        then.status(200).json_body(without(product_json(49), &["description"]));
    });
    let transport = transport(&server, &key);
    for (failure, name, field) in [
        (transport.run(&passes::get(id(1), id(2))).unwrap_err(), "game-passes.get", "name"),
        (
            transport.run(&products::list(id(1), None)).unwrap_err(),
            "developer-products.list",
            "developerProducts[1].priceInformation.defaultPriceInRobux",
        ),
        (transport.run(&products::get(id(1), id(3))).unwrap_err(), "developer-products.get", "description"),
    ] {
        let message = format!("{name}: Roblox's reply has no valid `{field}`");
        assert_eq!((failure.code, failure.error["message"].as_str()), (7, Some(message.as_str())));
        assert_eq!((failure.error["operation"].as_str(), failure.error["field"].as_str()), (Some(name), Some(field)));
    }
}
