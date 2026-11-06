//! The utility world in the run's main frame: where neo-runtime-injected's glue, Playwright's
//! injected script behind Neo's typed table, answers the page calls a browser call makes.
//!
//! A locator has two halves that must agree on names. neo-sdk builds the selector inside the
//! realm (`getByTestId('x')` is `internal:testid=[data-testid="x"s]`), and the page's injected
//! script matches it against the attribute it was created with. This module owns the names both
//! halves share.

/// The attribute `getByTestId` selects by. One constant for both halves: `realm.rs` hands it to
/// neo-sdk's `createRun`, which sets Playwright's module-level test id attribute, and the page's
/// injected script must be created with the same name. Two copies could drift into a
/// `getByTestId` locator that never matches anything.
pub(crate) const TEST_ID_ATTRIBUTE: &str = "data-testid";
