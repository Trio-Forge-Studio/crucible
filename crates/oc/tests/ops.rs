use crucible_oc::auth::Secret;
use crucible_oc::ops::{BadgeDetails, Details, Image, List, Op, Retry, Sellable, badges, passes, products};
use crucible_oc::outcome::Failure;
use crucible_oc::transport::{Clock, Event, Sink, Timing, Transport};
use crucible_oc::{Safety, Scope};
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

struct Row {
    name: &'static str,
    method: &'static str,
    path: String,
    safety: Safety,
    scopes: Vec<Scope>,
}

fn row<T, S: Retry>(op: Op<T, S>) -> Row {
    Row {
        name: op.name(),
        method: op.method(),
        path: op.path().into(),
        safety: op.safety(),
        scopes: op.scopes().to_vec(),
    }
}

fn list_row<T>(list: List<T>) -> Row {
    Row { name: list.name(), method: list.method(), path: list.path(), safety: list.safety(), scopes: list.scopes() }
}

/// Every operation, with universe 11 and resource 22, and the retry rule it must carry.
fn every_operation() -> Vec<(Row, Safety)> {
    let details = Details::default();
    vec![
        (list_row(products::list(id(11))), Safety::Read),
        (row(products::get(id(11), id(22))), Safety::Read),
        (row(products::create(id(11), &details)), Safety::Unsafe),
        (row(products::update(id(11), id(22), &details)), Safety::Idempotent),
        (list_row(passes::list(id(11))), Safety::Read),
        (row(passes::get(id(11), id(22))), Safety::Read),
        (row(passes::create(id(11), &details)), Safety::Unsafe),
        (row(passes::update(id(11), id(22), &details)), Safety::Idempotent),
        (row(badges::create(id(11), "B", "", true, None)), Safety::Unsafe),
        (row(badges::update(id(22), &BadgeDetails::default())), Safety::Idempotent),
        (row(badges::overwrite_icon(id(22), icon())), Safety::Idempotent),
        (list_row(badges::list_public(id(11))), Safety::Read),
        (row(badges::get_public(id(22))), Safety::Read),
    ]
}

#[test]
fn each_operation_matches_the_registry_and_carries_its_retry_rule() {
    let registry: Value =
        serde_json::from_str(include_str!("../../oc-registry/spec/registry.json")).expect("the registry is JSON");
    let registry = registry["ops"].as_array().unwrap();
    let operations = every_operation();
    assert_eq!(operations.len(), 13);
    for (row, safety) in operations {
        let name = row.name;
        assert_eq!(row.safety, safety, "{name}");
        let Some(entry) = registry.iter().find(|entry| entry["name"] == name) else {
            assert!(name.ends_with("-public"), "{name} is not in the registry");
            assert!(row.scopes.is_empty(), "{name} reads a public endpoint, so it needs no scope");
            continue;
        };
        assert_eq!(entry["method"], row.method, "{name}");
        let path = entry["path"].as_str().unwrap().replace("{universeId}", "11");
        let path = path.split('/').map(|part| if part.starts_with('{') { "22" } else { part }).collect::<Vec<_>>();
        assert_eq!(path.join("/"), row.path, "{name}");
        let scopes: Vec<Scope> = serde_json::from_value(entry["auth"]["apikey"]["scopes"].clone()).unwrap();
        assert_eq!(row.scopes, scopes, "{name}");
        assert_eq!(entry["safety"] == "read", safety == Safety::Read, "{name}: only reads are reads in the registry");
    }
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
            send: |transport| transport.run(&badges::create(id(1), "B", "", true, Some(icon()))).unwrap_err(),
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

const PRODUCTS: &str = "/developer-products/v2/universes/1/developer-products/creator";

fn products_page(ids: &[u64], next: Value) -> Value {
    let items: Vec<Value> = ids
        .iter()
        .map(|id| {
            let mut product = product_json(49);
            product["productId"] = json!(id);
            product
        })
        .collect();
    let mut page = json!({ "developerProducts": items });
    if !next.is_null() || ids.is_empty() {
        page["nextPageToken"] = next;
    }
    page
}

#[test]
fn a_list_follows_its_tokens_to_the_end() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let pages = [
        server.mock(|when, then| {
            when.method(Method::GET).path(PRODUCTS).query_param_missing("pageToken");
            then.status(200).json_body(products_page(&[1, 2], json!("two")));
        }),
        server.mock(|when, then| {
            when.method(Method::GET).path(PRODUCTS).query_param("pageToken", "two");
            then.status(200).json_body(products_page(&[], json!("three")));
        }),
        server.mock(|when, then| {
            when.method(Method::GET).path(PRODUCTS).query_param("pageToken", "three");
            then.status(200).json_body(products_page(&[3], json!("")));
        }),
    ];
    let products = transport(&server, &key).list_all(&products::list(id(1))).unwrap();
    assert_eq!(products.iter().map(|product| product.id.get()).collect::<Vec<_>>(), [1, 2, 3]);
    for page in pages {
        page.assert_calls(1);
    }
}

#[test]
fn a_list_fails_rather_than_return_part_of_itself() {
    let key = Secret::from_string("test-key".into());
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(Method::GET).path(PRODUCTS).query_param_missing("pageToken");
        then.status(200).json_body(products_page(&[1], json!("two")));
    });
    let again = server.mock(|when, then| {
        when.method(Method::GET).path(PRODUCTS).query_param("pageToken", "two");
        then.status(200).json_body(products_page(&[2], json!("two")));
    });
    let failure = transport(&server, &key).list_all(&products::list(id(1))).unwrap_err();
    again.assert_calls(1);
    assert_eq!(failure.code, 7, "a repeated token: {}", failure.error);
    assert_eq!(failure.error["message"], "developer-products.list: Roblox sent the page token \"two\" twice");

    let server = MockServer::start();
    let pages: Vec<_> = [(None, "a"), (Some("a"), "b"), (Some("b"), "")]
        .into_iter()
        .map(|(token, next)| {
            server.mock(|when, then| {
                let when = when.method(Method::GET).path(PRODUCTS);
                match token {
                    Some(token) => when.query_param("pageToken", token),
                    None => when.query_param_missing("pageToken"),
                };
                then.status(200).json_body(products_page(&[1], json!(next)));
            })
        })
        .collect();
    let failure = transport(&server, &key).list_all(&products::list(id(1)).max_pages(2)).unwrap_err();
    assert_eq!(failure.code, 2, "more pages than the cap: {}", failure.error);
    assert_eq!(pages.iter().map(|page| page.calls()).collect::<Vec<_>>(), [1, 1, 0], "the cap stops the reads");

    for next in [None, Some(json!(null)), Some(json!(12))] {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(Method::GET).path(PRODUCTS);
            let mut page = products_page(&[1], json!(null));
            if let Some(next) = &next {
                page["nextPageToken"] = next.clone();
            }
            then.status(200).json_body(page);
        });
        let failure = transport(&server, &key).list_all(&products::list(id(1))).unwrap_err();
        assert_eq!((failure.code, failure.error["field"].as_str()), (7, Some("nextPageToken")), "{next:?}");
    }
}

fn badges_page(first: u64, count: u64) -> Vec<Value> {
    (first..first + count)
        .map(|id| json!({"id": id, "name": format!("Badge {id}"), "description": null, "enabled": true}))
        .collect()
}

#[test]
fn the_public_badge_list_sends_no_key_and_follows_its_cursor() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let first = server.mock(|when, then| {
        when.method(Method::GET)
            .path("/v1/universes/1/badges")
            .query_param("limit", "100")
            .query_param("sortOrder", "Asc")
            .query_param_missing("cursor")
            .header_missing("x-api-key");
        then.status(200)
            .json_body(json!({"previousPageCursor": null, "nextPageCursor": "c2", "data": badges_page(1, 100)}));
    });
    let second = server.mock(|when, then| {
        when.method(Method::GET).path("/v1/universes/1/badges").query_param("cursor", "c2").header_missing("x-api-key");
        then.status(200).json_body(json!({"data": badges_page(101, 1)}));
    });
    let badges = transport(&server, &key).list_all(&badges::list_public(id(1))).unwrap();
    first.assert();
    second.assert();
    assert_eq!(badges.len(), 101);
    assert_eq!((badges[100].id.get(), badges[100].description.as_str(), badges[100].icon), (101, "", None));
}

#[test]
fn a_badge_create_sends_an_icon_only_when_it_has_one() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let mock = server.mock(|when, then| {
        when.method(Method::POST)
            .path("/legacy-badges/v1/universes/1/badges")
            .body_includes("name=\"name\"\r\n\r\nFirst delivery\r\n")
            .body_excludes("name=\"files\"");
        then.status(200).json_body(badge_json());
    });
    let badge = transport(&server, &key).run(&badges::create(id(1), "First delivery", "Deliver.", true, None)).unwrap();
    mock.assert_calls(1);
    assert_eq!(badge.id.get(), 2124000000000000);
}

#[test]
fn a_badge_is_read_by_id_from_the_public_endpoint_without_a_key() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let mock = server.mock(|when, then| {
        when.method(Method::GET).path("/v1/badges/2124000000000000").header_missing("x-api-key");
        then.status(200).json_body(badge_json());
    });
    let badge = transport(&server, &key).run(&badges::get_public(id(2124000000000000))).unwrap();
    mock.assert_calls(1);
    let expected = crucible_oc::ops::Badge {
        id: id(2124000000000000),
        name: "First delivery".into(),
        description: "Deliver.".into(),
        enabled: true,
        icon: Some(9),
    };
    assert_eq!(badge, expected);
}

#[test]
fn the_public_badge_list_ends_only_on_a_short_page() {
    let key = Secret::from_string("test-key".into());
    for cursor in [None, Some(json!(null)), Some(json!(""))] {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(Method::GET).path("/v1/universes/1/badges");
            let mut page = json!({"data": badges_page(1, 100)});
            if let Some(cursor) = &cursor {
                page["nextPageCursor"] = cursor.clone();
            }
            then.status(200).json_body(page);
        });
        let failure = transport(&server, &key).list_all(&badges::list_public(id(1))).unwrap_err();
        let message = "badges.list-public: Roblox's reply ends a full page without `nextPageCursor`";
        assert_eq!((failure.code, failure.error["message"].as_str()), (7, Some(message)), "{cursor:?}");
    }
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
    let badge =
        transport(&server, &key).run(&badges::create(id(1), "First delivery", "", false, Some(icon()))).unwrap();
    mock.assert();
    assert_eq!((badge.id.get(), badge.enabled, badge.icon), (2124000000000000, false, Some(9)));
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
            transport.list_all(&products::list(id(1))).unwrap_err(),
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

#[test]
fn a_game_pass_parses_whole_from_a_get_and_a_list() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let get = server.mock(|when, then| {
        when.method(Method::GET).path("/game-passes/v1/universes/1/game-passes/1786543210/creator");
        then.status(200).json_body(pass_json());
    });
    let list = server.mock(|when, then| {
        when.method(Method::GET)
            .path("/game-passes/v1/universes/1/game-passes/creator")
            .query_param_missing("pageToken");
        then.status(200).json_body(json!({"gamePasses": [pass_json()], "nextPageToken": ""}));
    });
    let expected = Sellable {
        id: id(1786543210),
        name: "VIP".into(),
        description: "Double coins.".into(),
        price: 250,
        for_sale: false,
        icon: Some(81234),
    };
    let transport = transport(&server, &key);
    assert_eq!(transport.run(&passes::get(id(1), id(1786543210))).unwrap(), expected);
    assert_eq!(transport.list_all(&passes::list(id(1))).unwrap(), [expected]);
    get.assert_calls(1);
    list.assert_calls(1);
}

#[test]
fn updates_send_their_parts_to_the_resource_they_name() {
    let server = MockServer::start();
    let key = Secret::from_string("test-key".into());
    let pass = server.mock(|when, then| {
        when.method(Method::PATCH)
            .path("/game-passes/v1/universes/1/game-passes/2")
            .body_includes("name=\"price\"\r\n\r\n300\r\n")
            .body_includes("name=\"isForSale\"\r\n\r\nfalse\r\n")
            .body_excludes("name=\"name\"");
        then.status(204);
    });
    let product = server.mock(|when, then| {
        when.method(Method::PATCH)
            .path("/developer-products/v2/universes/1/developer-products/3")
            .body_includes("name=\"description\"\r\n\r\n200 coins.\r\n")
            .body_includes("name=\"imageFile\"; filename=\"coin-pack.png\"");
        then.status(204);
    });
    let icon_mock = server.mock(|when, then| {
        when.method(Method::POST)
            .path("/legacy-publish/v1/badges/4/icon")
            .body_includes("name=\"Files\"; filename=\"coin-pack.png\"\r\nContent-Type: image/png\r\n\r\n");
        then.status(200).json_body(json!({"targetId": 77}));
    });
    let transport = transport(&server, &key);
    let price = Details { price: Some(300), for_sale: Some(false), ..Details::default() };
    transport.run(&passes::update(id(1), id(2), &price)).unwrap();
    let text = Details { description: Some("200 coins.".into()), icon: Some(icon()), ..Details::default() };
    transport.run(&products::update(id(1), id(3), &text)).unwrap();
    transport.run(&badges::overwrite_icon(id(4), icon())).unwrap();
    pass.assert_calls(1);
    product.assert_calls(1);
    icon_mock.assert_calls(1);
}
