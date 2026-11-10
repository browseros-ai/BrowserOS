//! Neo: bounded viewport stills on a disposable, non-emulating page session.
//!
//! Host stills do not take the runtime's private screenshot turn. During a script screenshot,
//! a host still may include that screenshot's temporary masks or hidden caret; it does not
//! change the script's own image. Presence and focus emulation are never needed for stills.

use tokio::time;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use neo_capture::{Attached, Link};
use serde_json::{Value, json};

use crate::NeoCapture;
use crate::pages::PageInfo;
use crate::routes::Failure;

/// Neo: modal dialogs can indefinitely block renderer-answered capture commands.
const STILL_TIMEOUT: Duration = Duration::from_secs(3);

/// A still owns a disposable session even when its deadline drops the browser future.
struct StillSession {
    link: Arc<Link>,
    attached: Option<Attached>,
}

impl Drop for StillSession {
    fn drop(&mut self) {
        if let Some(attached) = self.attached.take() {
            let link = Arc::clone(&self.link);
            // Detach is asynchronous and must also happen when timeout drops screenshot().
            tokio::spawn(async move {
                let _ = time::timeout(STILL_TIMEOUT, link.release(&attached)).await;
            });
        }
    }
}

pub(crate) async fn screenshot(
    capture: &NeoCapture,
    page: &str,
    format: &str,
    quality: Option<u8>,
) -> Result<(Bytes, PageInfo), Failure> {
    time::timeout(STILL_TIMEOUT, async {
        let _permit = capture.0.stills.acquire().await.map_err(Failure::browser)?;
        let generation = capture.0.pages.generation().await?;
        let info = generation
            .lookup(page)
            .await?
            .ok_or_else(Failure::missing)?;
        let session = StillSession {
            link: Arc::clone(&generation.link),
            attached: Some(generation.link.attach(page).await?),
        };
        let mut params = json!({"format": format});
        if let Some(quality) = quality {
            params["quality"] = json!(quality);
        }
        let attached = session
            .attached
            .as_ref()
            .ok_or_else(|| Failure::browser("still session disappeared"))?;
        let reply = generation
            .link
            .send("Page.captureScreenshot", params, attached)
            .await?;
        let data = reply
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| Failure::browser("captureScreenshot returned no data"))?;
        let bytes = STANDARD.decode(data).map_err(Failure::browser)?;
        Ok((Bytes::from(bytes), info))
    })
    .await
    .map_err(|_| Failure::timeout())?
}
