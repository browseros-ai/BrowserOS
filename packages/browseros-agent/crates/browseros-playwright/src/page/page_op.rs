//! The page ops, their registration metadata, and the small ones' handlers.

use browseros_core::{CoreError, ProtocolSession};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_output::to_output;
use crate::op_scope::{OpScope, Sent};
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::page::document_watch::decode;
use crate::page::navigation::{GotoInput, NavigateInput, Navigation};
use crate::page::{
    PageHandle, document_op, evaluate, frame_tree, navigation, pdf, screenshot, wait_for_url,
};
use crate::probe::DocumentPart;
use crate::target::PageRef;
use crate::{Playwright, PlaywrightError};

listed_enum! {
    /// Page and frame leaves. A frame's form of a method (`frame.title`, `frame.evaluate`,
    /// `frame.waitForLoadState`) is the page op with a frame in its input.
    pub(crate) enum PageOp {
        Goto,
        Reload,
        GoBack,
        GoForward,
        WaitForUrl,
        WaitForLoadState,
        Title,
        Content,
        SetContent,
        Close,
        Opener,
        Frames,
        Evaluate,
        EvaluateOnElement,
        EvaluateOnAll,
        WaitForFunction,
        Screenshot,
        Pdf,
    }
}

impl PageOp {
    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Goto => OpSpec::write(
                "page.goto",
                PAGE,
                "navigate and wait for the load state; the response or null",
            ),
            Self::Reload => {
                OpSpec::write("page.reload", PAGE, "reload and wait for the load state")
            }
            Self::GoBack => OpSpec::write("page.goBack", PAGE, "go back in the tab's history"),
            Self::GoForward => {
                OpSpec::write("page.goForward", PAGE, "go forward in the tab's history")
            }
            Self::WaitForUrl => OpSpec::read(
                "page.waitForURL",
                PAGE,
                "wait until the main frame's URL matches a string, glob or RegExp",
            ),
            Self::WaitForLoadState => OpSpec::read(
                "page.waitForLoadState",
                PAGE,
                "wait for a load state of the page or one of its frames",
            ),
            Self::Title => OpSpec::read("page.title", PAGE, "the document's title"),
            Self::Content => OpSpec::read(
                "page.content",
                PAGE,
                "the document's HTML, read in the sealed world so field values are masked",
            ),
            Self::SetContent => OpSpec::write(
                "page.setContent",
                PAGE,
                "replace the document's HTML and wait for the load state",
            ),
            Self::Close => OpSpec::write("page.close", PAGE, "close the tab"),
            Self::Opener => OpSpec::read("page.opener", PAGE, "the page that opened this one"),
            Self::Frames => OpSpec::read(
                "page.frames",
                PAGE,
                "the frame tree, for the prelude's synchronous frames() and mainFrame()",
            ),
            Self::Evaluate => OpSpec::write(
                "page.evaluate",
                PAGE,
                "run a function in the main world, behind the sensitive-value sweep",
            ),
            Self::EvaluateOnElement => OpSpec::write(
                "locator.evaluate",
                TARGET,
                "run a function on one element in the main world; also $eval",
            ),
            Self::EvaluateOnAll => OpSpec::write(
                "locator.evaluateAll",
                TARGET,
                "run a function on every match in the main world; also $$eval",
            ),
            Self::WaitForFunction => OpSpec::write(
                "page.waitForFunction",
                PAGE,
                "poll a main-world function from Rust until it returns truthy",
            ),
            Self::Screenshot => OpSpec::read(
                "page.screenshot",
                PAGE,
                "capture the page or one element once the host allows it",
            ),
            Self::Pdf => OpSpec::read(
                "page.pdf",
                PAGE,
                "print the page to PDF once the host allows it",
            ),
        }
    }
}

/// Runs one page op. The ops that run in the page's sealed world (title, content, setContent,
/// waitForURL) or its main world (evaluate, waitForFunction) are built with those worlds.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: PageOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    match op {
        PageOp::Goto => {
            let (page, navigation, wait_until) = args.decode::<GotoInput>(scope)?.into_parts();
            let page = page.checked(pw.browser(), scope).await?;
            navigation::navigate(pw, scope, &page, navigation, wait_until).await
        }
        PageOp::Reload => renavigate(pw, scope, args, Navigation::Reload).await,
        PageOp::GoBack => renavigate(pw, scope, args, Navigation::History(-1)).await,
        PageOp::GoForward => renavigate(pw, scope, args, Navigation::History(1)).await,
        PageOp::WaitForLoadState => {
            navigation::wait_for_load_state(pw, scope, args.decode(scope)?).await
        }
        PageOp::Close => close(pw, scope, args.decode(scope)?).await,
        PageOp::Opener => opener(pw, scope, args.decode(scope)?).await,
        PageOp::Frames => frame_tree::frames(pw, scope, args.decode(scope)?).await,
        PageOp::Screenshot => screenshot::capture(pw, scope, args.decode(scope)?).await,
        PageOp::Pdf => pdf::print(pw, scope, args.decode(scope)?).await,
        PageOp::Evaluate => evaluate::evaluate(pw, scope, args.decode(scope)?).await,
        PageOp::WaitForFunction => {
            evaluate::wait_for_function(pw, scope, args.decode(scope)?).await
        }
        PageOp::Title => {
            document_op::read(pw, scope, args.decode(scope)?, DocumentPart::Title).await
        }
        PageOp::Content => {
            document_op::read(pw, scope, args.decode(scope)?, DocumentPart::Content).await
        }
        PageOp::SetContent => document_op::set_content(pw, scope, args.decode(scope)?).await,
        PageOp::WaitForUrl => wait_for_url::wait_for_url(pw, scope, args.decode(scope)?).await,
        PageOp::EvaluateOnElement | PageOp::EvaluateOnAll => Err(scope.not_built()),
    }
}

/// `reload`, `goBack` and `goForward`, which take only `waitUntil`.
async fn renavigate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    args: OpArgs,
    navigation: Navigation,
) -> Result<Value, PlaywrightError> {
    let (page, wait_until) = args.decode::<NavigateInput>(scope)?.into_parts();
    let page = page.checked(pw.browser(), scope).await?;
    navigation::navigate(pw, scope, &page, navigation, wait_until).await
}

/// `page.close({ runBeforeUnload })`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CloseInput {
    page: PageRef,
    /// Run the page's `beforeunload` handlers first. Their dialog follows the dialog rules
    /// (a listener's answer, or accepted after the grace), and the close returns at once,
    /// as upstream does.
    #[serde(default)]
    run_before_unload: bool,
}

async fn close(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: CloseInput,
) -> Result<Value, PlaywrightError> {
    let page = input.page.checked(pw.browser(), scope).await?;
    let admitted = scope.admit(&page).await?;
    let core = |error| scope.core_error(&error);
    let sent = scope.send_effect()?;
    if input.run_before_unload {
        let session = pw
            .browser()
            .protocol_session(admitted.session.session_id.clone());
        close_with_unload(&session, &sent).await.map_err(core)?;
    } else {
        // Through core, so its tab registry forgets the page and the run's view still decides.
        scope.view().close_page(page).await.map_err(core)?;
    }
    Ok(json!({}))
}

/// `Page.close`: the page runs its `beforeunload` handlers and closes unless one cancels.
async fn close_with_unload(session: &ProtocolSession, _sent: &Sent<'_>) -> Result<(), CoreError> {
    session.send_value("Page.close", json!({})).await?;
    Ok(())
}

/// `page.opener()`: `{ page }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenerInput {
    page: PageRef,
}

/// The tab that opened this one while it is still open, as upstream: `{ opener: page | null }`.
async fn opener(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: OpenerInput,
) -> Result<Value, PlaywrightError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TargetInfoReply {
        target_info: TargetFields,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TargetFields {
        #[serde(default)]
        opener_id: Option<String>,
    }
    let admitted = scope
        .admit(&input.page.checked(pw.browser(), scope).await?)
        .await?;
    let core = |error| scope.core_error(&error);
    let info: TargetInfoReply = pw
        .browser()
        .cdp(
            "Target.getTargetInfo",
            json!({ "targetId": admitted.session.target_id.as_str() }),
            None,
        )
        .await
        .and_then(decode)
        .map_err(core)?;
    let opener = match info.target_info.opener_id {
        Some(opener) => scope
            .view()
            .list_pages()
            .await
            .map_err(core)?
            .pages
            .into_iter()
            .find(|page| page.target_id.as_str() == opener)
            .map(|page| PageHandle::new(page.page_id, page.tab_id)),
        None => None,
    };
    to_output(scope, &OpenerResult { opener })
}

/// `page.opener()`'s result.
#[derive(Debug, Serialize)]
struct OpenerResult {
    opener: Option<PageHandle>,
}
