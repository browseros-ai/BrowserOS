//! Launch-gated request/reply CDP for browser features Playwright does not expose.
//! Calls still pass through the dispatcher for policy, cancellation, delivery and tracing.

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use neo_trace::Effect;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::dispatch::{
    ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, OperationError, PageScript,
    PageTarget, RegistryBuilder, RegistryError,
};

const LISTED: &str = "neo.cdp(method, params?, { page }?) -> the command's result: sends one \
    Chrome DevTools Protocol command to the browser, or to page's tab session. For what \
    Playwright lacks (tab groups, windows, profiles, history). Prefer page and locators for \
    anything they can do. This call bypasses Neo's page model.";

/// These commands would close Neo's browser, interfere with target bookkeeping, or tunnel
/// commands around this request/reply boundary.
const REFUSED: &[&str] = &[
    "Browser.close",
    "Target.setAutoAttach",
    "Target.attachToTarget",
    "Target.detachFromTarget",
    "Target.setDiscoverTargets",
    "Target.exposeDevToolsProtocol",
    "Target.autoAttachRelated",
    "Target.attachToBrowserTarget",
    "Target.sendMessageToTarget",
];

/// Registers even when disabled, so a cached caller learns that its host must enable it.
pub(crate) fn neo_cdp(builder: &mut RegistryBuilder, enabled: bool) -> Result<(), RegistryError> {
    if enabled {
        builder.define(NeoCdp::<true>)
    } else {
        builder.define(NeoCdp::<false>)
    }
}

/// The registry freezes this permission at startup. A disabled or refused call declares no
/// target, because resolving the script's lazy default page could otherwise open a tab.
struct NeoCdp<const ENABLED: bool>;

/// The raw request, decoded once before policy checks or browser work.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CdpInput {
    method: String,
    #[serde(default)]
    params: Map<String, Value>,
    #[serde(default)]
    page: Option<PageTarget>,
}

impl<const ENABLED: bool> Method for NeoCdp<ENABLED> {
    type Input = CdpInput;
    // The runtime revives SDK handles in operation results. A JSON string keeps arbitrary
    // CDP data, including objects with a `$neo` key, out of that handle codec.
    type Output = String;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: if ENABLED {
                Listing::Own(LISTED)
            } else {
                Listing::Hidden
            },
            // Any raw reply may contain page data, even though every command counts as a
            // write. Recheck read permission before releasing its value or browser error.
            page_script: PageScript::MainWorld,
            // The method name is supplied by the script, so every call may write.
            ..MethodSpec::new("neo.cdp", Effect::Write)
        }
    }

    fn target(input: &CdpInput) -> Option<&PageTarget> {
        if ENABLED && !REFUSED.contains(&input.method.as_str()) {
            input.page.as_ref()
        } else {
            None
        }
    }

    async fn call(&self, op: &OpCtx<Self>, input: CdpInput) -> Result<String, LeafError> {
        if !ENABLED {
            return Err(LeafError::definite(OperationError::host(
                ErrorCode::CDP_UNAVAILABLE,
                "neo.cdp is off; the host must set flags.script_cdp to true in its launch file",
            )));
        }
        if REFUSED.contains(&input.method.as_str()) {
            return Err(LeafError::definite(OperationError::script(
                ErrorCode::UNSUPPORTED,
                format_args!(
                    "neo.cdp refuses {} because it would close Neo's browser or interfere with its target bookkeeping",
                    input.method
                ),
            )));
        }

        let view = op.run().view();
        let params = Value::Object(input.params);
        // Core repeats this immutable capability gate inside `cdp`. Check it here first so
        // a local deployment refusal is `not_sent`, rather than an ambiguous sent write.
        view.native()
            .check_raw(&input.method, &params)
            .map_err(|error| {
                LeafError::definite(OperationError::host(ErrorCode::UNSUPPORTED, error))
            })?;
        let session = if input.page.is_some() {
            Some(op.race(view.page_session(op.core_page()?.clone())).await?)
        } else {
            None
        };
        // Session lookup can wait. Recheck takeover and CAPTCHA rules at the send boundary.
        op.recheck()?;
        op.mark_sent();
        let result = op
            .race_answer(view.cdp(
                &input.method,
                params,
                session.as_ref().map(|session| &session.session_id),
            ))
            .await?
            .map_err(answer_error)?;
        Ok(result.to_string())
    }
}

/// The script owns malformed raw commands; transport failures keep the door's ambiguous
/// classification, and other browser answers retain the SDK's ordinary CDP error handling.
fn answer_error(error: CoreError) -> LeafError {
    match error {
        error @ CoreError::Cdp(CdpError::Protocol {
            code: -32601 | -32602,
            ..
        }) => LeafError::definite(OperationError::script(ErrorCode::CDP_ERROR, error)),
        CoreError::UnknownPage(_) | CoreError::UnknownPageShort(_) => {
            LeafError::definite(OperationError::neo(ErrorCode::PAGE_CLOSED, error))
        }
        CoreError::Cdp(CdpError::Protocol { .. }) => {
            LeafError::definite(OperationError::neo(ErrorCode::CDP_ERROR, error))
        }
        other => LeafError::from(OperationError::neo(ErrorCode::INTERNAL_ERROR, other)),
    }
}
