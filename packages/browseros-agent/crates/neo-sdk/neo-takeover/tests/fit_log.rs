//! What the fit logs, in its own binary so tracing callsite caches cannot race other tests. A
//! failed navigation records core's original error so the host can diagnose both a browser-bar
//! navigation failure and a popup that could not be pulled into the fitted tab.

#[allow(
    dead_code,
    reason = "the fit tests' fake Chromium, of which this binary uses a part"
)]
#[path = "fits/browser.rs"]
mod browser;

use browser::FakeBrowser;
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, TabId};
use neo_takeover::testing::FakeXvnc;
use neo_takeover::{BarAction, BarOutcome, BrowserTabs, Fits, ScreenSize, Viewer};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const HOME: ScreenSize = ScreenSize {
    width: 1600,
    height: 1000,
};
const PHONE: ScreenSize = ScreenSize {
    width: 390,
    height: 844,
};
const QUERY_VALUE: &str = "typed-value";

/// Everything logged on the test's thread, which runs the fits loop too.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Log {
    fn text(&self) -> Result<String, BoxError> {
        let bytes = self.0.lock().map_err(|_| "the log lock was poisoned")?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl std::io::Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("the log lock was poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Log {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Waits up to 5 s for `done`.
async fn until(done: impl Fn() -> Result<bool, BoxError>) -> Result<(), BoxError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !done()? {
        if tokio::time::Instant::now() >= deadline {
            return Err("timed out waiting".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

#[tokio::test]
async fn failed_navigations_log_their_original_addresses() -> Result<(), BoxError> {
    let log = Log::default();
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(log.clone())
            .finish(),
    );
    let xvnc = Arc::new(FakeXvnc::start(HOME).await?);
    let browser = FakeBrowser::new(Arc::clone(&xvnc), &[3]);
    let fits = Fits::new(
        BrowserTabs::new(
            BrowserSession::new(browser.clone(), BrowserSessionHooks::default()),
            Arc::new(Viewports::default()),
        ),
        xvnc.xvnc(HOME),
    );
    let address = format!("https://down.example/?code={QUERY_VALUE}");
    browser.unreachable(&address);
    let fit = fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;

    let typed = BarAction::Open(address.clone());
    assert_eq!(fits.navigate(fit.id, typed).await?, BarOutcome::Failed);
    // After the fit's first owner view read, a page the tab opens is pulled into it.
    until(|| Ok(fits.view(fit.id).is_some_and(|fit| fit.bar.is_some()))).await?;
    browser.open_popup(3, 6, &address, "link");
    let failed = "Navigation to https://down.example/";
    until(|| Ok(log.text()?.matches(failed).count() >= 2)).await?;

    let text = log.text()?;
    for line in [
        "the owner's browser bar did not move the tab",
        "a page the fitted tab opened could not be pulled in",
    ] {
        assert!(text.contains(line), "the fit logged \"{line}\": {text}");
    }
    assert_eq!(
        text.matches(&format!("{address} failed")).count(),
        2,
        "both failures name the original address: {text}"
    );
    Ok(())
}
