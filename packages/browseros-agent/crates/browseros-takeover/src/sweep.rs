//! The sweep: one pass of sdk-privacy's classifier over every frame of a tab, which puts each
//! value it finds in a password, card or one-time-code field into the known secrets list
//! (design D13, sdk-privacy D4).
//!
//! It is the final catch for values that reached a field without an `input` event the control
//! journal saw: a value a script set, a field in a frame the journal was not armed on, a closed
//! shadow root made during control. The takeover flow sweeps its tab at give-back while the
//! agent is still locked out, a control display that took input sweeps every tab when it closes
//! (decision 31), and a host can sweep any tab (`POST /v1/tabs/{tabId}/sweep`).
//!
//! ```text
//!  seal_tab ─▶ per sealed frame: __clawSeal.sensitiveValues() in the sealed world
//!           ─▶ FieldValues::from_sweep ─▶ union ─▶ registrable (4+ characters) ─▶ SecretSet
//! ```
//!
//! The values never leave this function: it returns counts, and it logs no value and no length.
//! A value shorter than 4 characters is not registered, since it would mask ordinary text; the
//! sealed world still masks it in place.

use crate::sealed_frames::{SealedFrame, seal_tab, sealed_world, within_budget};
use crate::{BrowserTabs, PartError};
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::{CoreError, TabId};
use browseros_trace::{Registration, SecretSet, SecretValue};
use serde_json::Value;

/// What one sweep did, in counts. The values themselves are never handed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swept {
    /// Distinct values the secret set masks because of this sweep, new or refreshed.
    pub registered: usize,
    /// Frames whose fields were read.
    pub frames: usize,
}

/// Reads every frame of `tab` and registers what its sensitive fields hold with `secrets`, at
/// the set's default TTL. A frame that cannot be read is logged and skipped.
///
/// # Errors
/// `TabNotFound` when no open tab has this id; `Browser` when the page cannot be reached, or
/// when not one of its frames could be read.
pub async fn sweep(
    tabs: &BrowserTabs,
    secrets: &SecretSet,
    tab: TabId,
) -> Result<Swept, PartError> {
    let mut values = FieldValues::default();
    let mut frames = 0;
    for frame in seal_tab(tabs, tab).await? {
        match sensitive_values(&frame).await {
            Ok(found) => {
                values = values.union(found);
                frames += 1;
            }
            Err(error) => tracing::warn!(
                tab = tab.0,
                frame = frame.frame.as_str(),
                %error,
                "the sweep skipped a frame it could not read"
            ),
        }
    }
    if frames == 0 {
        return Err(PartError::Browser(CoreError::Message(
            "no frame of the tab could be read".into(),
        )));
    }
    let registered = values
        .registrable()
        .filter(|value| {
            secrets.register(SecretValue::new((*value).to_owned()), None) != Registration::TooShort
        })
        .count();
    Ok(Swept { registered, frames })
}

/// The raw values of the frame's sensitive fields, as the seal reports them to Neo alone.
async fn sensitive_values(frame: &SealedFrame) -> Result<FieldValues, CoreError> {
    let reading = sealed_world(&frame.session, &frame.frame).eval("__clawSeal.sensitiveValues()");
    let values: Value = within_budget("the sweep's read", reading).await?;
    Ok(FieldValues::from_sweep(&values))
}
