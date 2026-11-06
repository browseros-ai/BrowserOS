//! The saved Amazon.in script runs through one long-lived Rust `Runtime`, then a second
//! script reads the same cart in a fresh JS realm. A separate CDP session verifies the real
//! cart contents, so a script returning its expected JSON alone cannot pass this test.
//!
//! This opt-in test contacts live Amazon.in and adds one pack to an anonymous cart in a
//! temporary browser profile. Product availability, CAPTCHA and site changes can fail it.
//! It never signs in or checks out. From `packages/browseros-agent`, run:
//!
//! ```sh
//! NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" \
//!   cargo test -p neo-runtime --test amazon_cart_live -- --ignored --nocapture
//! ```

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the live scenario uses only part of the shared browser support"
)]
mod browser;

use std::env;
use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use url::Url;

const SHOP: &str = include_str!("../../neo-sdk/tests/scripts/amazon-sensodyne-cart.js");
const PRODUCT: &str = "Sensodyne Toothpaste Fresh Mint | Daily Sensitivity Relief Tooth Paste Pack of 1 - 75gm | Dentist Recommended | Everyday Use Paste, Fresh Breath, Fights Tooth Decay, Maintain Healthy Gums";

/// Reads DOM state independently of the SDK's locators and snapshot renderer. Amazon marks
/// active-cart line items with the ASIN and quantity; saved-for-later items are outside it.
const CART_STATE: &str = r"(() => {
  const cart = document.querySelector('#sc-active-cart');
  return {
    url: location.href,
    title: document.title,
    readyState: document.readyState,
    cartCount: document.querySelector('#nav-cart-count')?.textContent.trim() ?? null,
    items: [...(cart?.querySelectorAll('.sc-list-item[data-asin]') ?? [])].map(item => ({
      asin: item.getAttribute('data-asin'),
      quantity: item.getAttribute('data-quantity'),
      title: item.querySelector('.sc-product-title .a-truncate-full')?.textContent.trim()
        ?? item.querySelector('.sc-product-title')?.textContent.trim() ?? null,
    })),
    pageText: cart ? null : document.body?.innerText.slice(0, 3000),
    html: cart ? null : document.documentElement.outerHTML.slice(0, 1500),
  };
})()";

async fn run(runtime: &Runtime, page: &PageId, code: &str, timeout: Duration) -> RunOutcome {
    runtime
        .run(RunRequest {
            code: code.to_string(),
            page: page.clone(),
            timeout,
            cancel: CancellationToken::new(),
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live Amazon.in cart write; opt in with NEO_E2E_BROWSER and --ignored"]
async fn tests_that_one_runtime_adds_one_pack_and_reads_its_cart_in_a_later_run() {
    let binary = env::var_os("NEO_E2E_BROWSER")
        .expect("set NEO_E2E_BROWSER to the BrowserOS neo binary to run this live test");
    let tab = Tab::open(binary, Fixture::default(), "/amazon-start").await;
    let page = PageId::new(tab.target_id.clone());
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .expect("connect the one Rust runtime controller");

    // A global deliberately set by one script must be gone in the next. The browser and
    // Rust controller stay alive; only the script realm is replaced.
    let before = run(
        &runtime,
        &page,
        "globalThis.__amazonScenario = true; return page.isClosed();",
        Duration::from_secs(10),
    )
    .await;
    let shopping = run(&runtime, &page, SHOP, Duration::from_secs(180)).await;
    let after = run(
        &runtime,
        &page,
        r"const cartCount = await page.locator('#nav-cart-count').textContent({timeout: 15000});
        return {
          cartCount,
          url: page.url(),
          previousGlobal: typeof globalThis.__amazonScenario,
          snapshot: await page.ariaSnapshot(),
        };",
        Duration::from_secs(30),
    )
    .await;
    let observed = tab.evaluate(CART_STATE).await;
    tab.close().await;

    assert_eq!(before.status, RunStatus::Ok, "preflight: {before:#?}");
    assert_eq!(before.value, Some(json!(false)), "{before:#?}");
    assert_eq!(
        shopping.status,
        RunStatus::Ok,
        "amazon-sensodyne-cart.js: {shopping:#?}\nFollow-up: {after:#?}\nBrowser observed: {observed:#}"
    );
    let value = shopping
        .value
        .as_ref()
        .expect("shopping script returns JSON");
    assert_eq!(value["product"], PRODUCT, "{value:#}");
    assert_eq!(value["quantity"], 1, "{value:#}");
    let cart_url = Url::parse(value["cartUrl"].as_str().expect("cartUrl is a string"))
        .expect("cartUrl is an absolute URL");
    assert_eq!(cart_url.host_str(), Some("www.amazon.in"), "{cart_url}");
    assert!(cart_url.path().contains("/cart"), "{cart_url}");

    assert_eq!(after.status, RunStatus::Ok, "follow-up read: {after:#?}");
    let read = after.value.as_ref().expect("follow-up script returns JSON");
    assert_eq!(read["url"], value["cartUrl"], "{read:#}");
    assert_eq!(read["cartCount"], "1", "{read:#}");
    assert_eq!(
        read["previousGlobal"], "undefined",
        "fresh JS realm: {read:#}"
    );
    let snapshot = read["snapshot"].as_str().expect("snapshot is a string");
    assert!(
        snapshot.contains(PRODUCT),
        "product absent from:\n{snapshot}"
    );
    assert!(
        snapshot.contains(&format!(
            "button \"Increase quantity by one, Quantity is 1, {PRODUCT}\""
        )),
        "quantity 1 absent from:\n{snapshot}"
    );
    assert!(
        after
            .pages
            .iter()
            .any(|p| p.id == page.as_str() && !p.closed),
        "the original tab survives into the later run: {after:#?}"
    );

    assert_eq!(observed["url"], value["cartUrl"], "{observed:#}");
    assert_eq!(observed["cartCount"], "1", "{observed:#}");
    let items = observed["items"]
        .as_array()
        .expect("cart items are an array");
    assert_eq!(items.len(), 1, "one active cart item: {observed:#}");
    assert_eq!(items[0]["quantity"], "1", "{observed:#}");
    assert_eq!(items[0]["title"], PRODUCT, "{observed:#}");
    assert!(
        items[0]["asin"]
            .as_str()
            .is_some_and(|asin| !asin.is_empty()),
        "a real cart line item has an ASIN: {observed:#}"
    );
}
