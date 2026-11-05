//! The browser-context ops and their registration metadata.

use browseros_core::pages::NewPageOptions;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_output::to_output;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, OpTarget};
use crate::page::PageHandle;
use crate::{Playwright, PlaywrightError};

listed_enum! {
    /// The one browser context a run sees: its tabs, and opening a new one.
    pub(crate) enum ContextOp {
        Pages,
        NewPage,
    }
}

impl ContextOp {
    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Pages => OpSpec::read(
                "context.pages",
                OpTarget::None,
                "every tab this run's view admits",
            ),
            Self::NewPage => {
                OpSpec::write("context.newPage", OpTarget::None, "open a background tab")
            }
        }
    }
}

/// Neither op takes arguments beyond the call header.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// `context.pages()`: every tab the run's view admits, the owner's and other sessions' tabs
/// included (D8, the owner's decision: no ownership filter).
#[derive(Debug, Serialize)]
struct Pages {
    pages: Vec<PageEntry>,
}

/// One tab, with what the prelude caches for its synchronous `url()`.
#[derive(Debug, Serialize)]
struct PageEntry {
    page: PageHandle,
    url: String,
}

/// Runs one context op.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ContextOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    let NoArgs {} = args.decode(scope)?;
    let core = |error| scope.core_error(&error);
    match op {
        ContextOp::Pages => {
            let listing = scope.view().list_pages().await.map_err(core)?;
            let pages = listing
                .pages
                .into_iter()
                .map(|page| PageEntry {
                    page: PageHandle::new(page.page_id, page.tab_id),
                    url: page.url,
                })
                .collect();
            to_output(scope, &Pages { pages })
        }
        ContextOp::NewPage => {
            // The list of tabs open at the instance's start must be read before this tab
            // exists, or a newPage right after `Playwright::new` lands in that list and its
            // refs print from the random base (`f250e3`) instead of bare (`e3`). Measured by
            // conformance Runner N, whose first op in a new instance is this one.
            pw.pages().note_open_tabs(pw.browser()).await;
            let _sent = scope.send_effect()?;
            // Core opens it in the background and keeps focus emulation on, as for every tab it
            // opens for the agent.
            let opened = scope
                .view()
                .new_page(
                    "about:blank",
                    NewPageOptions {
                        background: Some(true),
                        ..NewPageOptions::default()
                    },
                )
                .await
                .map_err(core)?;
            to_output(
                scope,
                &PageEntry {
                    page: PageHandle::new(opened.page_id, opened.tab_id),
                    url: opened.url,
                },
            )
        }
    }
}
