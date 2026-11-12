//! Live tests against a staging universe. Every test is ignored, so `cargo test --workspace` and CI never run one.
//!
//! ```sh
//! CRUCIBLE_OC_API_KEY=... CRUCIBLE_STAGING_UNIVERSE=... cargo test -p crucible-oc --test live -- --ignored
//! ```
//!
//! `writes_adopt_then_update_one_product` also needs `CRUCIBLE_OC_LIVE_WRITES=1`. It adopts the product named
//! "Crucible live test" or creates it once, off sale, so repeated runs never add a second one.

use crucible_oc::auth::Secret;
use crucible_oc::ops::{Details, Id, badges, passes, products};
use crucible_oc::transport::{Event, Sink, SystemClock, Timing, Transport};
use std::time::Duration;

const PRODUCT: &str = "Crucible live test";

struct Stderr;
impl Sink for Stderr {
    fn event(&self, event: Event<'_>) {
        if let Event::Note(line) = event {
            eprintln!("{line}");
        }
    }
}

fn staging() -> (Secret, Id) {
    let key = std::env::var("CRUCIBLE_OC_API_KEY").expect("set CRUCIBLE_OC_API_KEY to a staging key");
    let universe =
        std::env::var("CRUCIBLE_STAGING_UNIVERSE").expect("set CRUCIBLE_STAGING_UNIVERSE to its universe ID");
    (Secret::from_string(key), universe.parse().expect("the universe ID is a positive integer"))
}

fn transport(key: &Secret) -> Transport<'_> {
    let timing =
        Timing { poll_interval: Duration::from_secs(2), deadline: Duration::from_secs(300), retry_delay: None };
    Transport::new(None, Some(key), timing, None, Box::new(Stderr), Box::new(SystemClock)).unwrap()
}

#[test]
#[ignore = "needs CRUCIBLE_OC_API_KEY and CRUCIBLE_STAGING_UNIVERSE; read-only"]
fn reads_list_products_passes_and_public_badges() {
    let (key, universe) = staging();
    let transport = transport(&key);
    let products = transport.list_all(&products::list(universe)).unwrap();
    if let Some(first) = products.first() {
        assert_eq!(transport.run(&products::get(universe, first.id)).unwrap().id, first.id);
    }
    let passes = transport.list_all(&passes::list(universe)).unwrap();
    if let Some(first) = passes.first() {
        assert_eq!(transport.run(&passes::get(universe, first.id)).unwrap().id, first.id);
    }
    let badges = transport.list_all(&badges::list_public(universe)).unwrap();
    eprintln!("{} products, {} passes, {} badges", products.len(), passes.len(), badges.len());
}

#[test]
#[ignore = "needs CRUCIBLE_OC_API_KEY, CRUCIBLE_STAGING_UNIVERSE and CRUCIBLE_OC_LIVE_WRITES=1; writes one product"]
fn writes_adopt_then_update_one_product() {
    assert_eq!(std::env::var("CRUCIBLE_OC_LIVE_WRITES").as_deref(), Ok("1"), "set CRUCIBLE_OC_LIVE_WRITES=1 to write");
    let (key, universe) = staging();
    let transport = transport(&key);
    let mut found = transport.list_all(&products::list(universe)).unwrap();
    found.retain(|product| product.name == PRODUCT);
    assert!(found.len() <= 1, "two products are named {PRODUCT:?}; delete neither, and adopt one by hand");
    let product = match found.pop() {
        Some(product) => product,
        None => transport
            .run(&products::create(
                universe,
                &Details {
                    name: Some(PRODUCT.into()),
                    description: Some("Made by crucible-oc's live test.".into()),
                    price: Some(1),
                    for_sale: Some(false),
                    icon: None,
                },
            ))
            .unwrap(),
    };
    for description in ["Updated by crucible-oc's live test.", "Made by crucible-oc's live test."] {
        let details = Details { description: Some(description.into()), for_sale: Some(false), ..Details::default() };
        transport.run(&products::update(universe, product.id, &details)).unwrap();
        let read = transport.run(&products::get(universe, product.id)).unwrap();
        assert_eq!((read.description.as_str(), read.for_sale), (description, false));
    }
}
