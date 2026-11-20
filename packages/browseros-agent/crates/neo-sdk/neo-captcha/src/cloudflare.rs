//! Observes Cloudflare's own full-page check and hands a persistent wall to a person.

use std::time::Duration;

use serde_json::json;
use tokio::time::{self, Instant};
use url::{Origin, Url};

use crate::{
    SolveCtx, detect,
    page::CaptchaPage,
    page_failure,
    providers::CaptchaBackend,
    terminal,
    types::{
        CaptchaEventName, CaptchaKind, DocumentId, FlowResult, PageError, SolveMethod, SolveReason,
        SolveStatus, SolveStep,
    },
};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(500);

enum PageState {
    Wall,
    Cleared,
    Unconfirmed,
}

async fn page_state(
    page: &impl CaptchaPage,
    original: &DocumentId,
    origin: &Origin,
) -> Result<PageState, PageError> {
    let current = page.document().await?;
    if !Url::parse(&current.url).is_ok_and(|url| url.origin() == *origin) {
        return Ok(PageState::Unconfirmed);
    }
    let observed = async {
        match detect::cloudflare_wall(page).await? {
            Some(true) => return Ok(PageState::Wall),
            Some(false) if current != *original => {}
            _ => return Ok(PageState::Unconfirmed),
        }
        let cookies = page.cookie_names(&current.url).await?;
        Ok(if cookies.iter().any(|name| name == "cf_clearance") {
            PageState::Cleared
        } else {
            PageState::Unconfirmed
        })
    }
    .await;
    // A reload can invalidate a successful read or destroy its execution context mid-read.
    if page.document().await? != current {
        return Ok(PageState::Unconfirmed);
    }
    observed
}

pub(super) async fn observe<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
) -> FlowResult {
    let kind = CaptchaKind::CloudflareChallenge;
    ctx.lease.set_progress(Some(kind), SolveStep::AutoPass);
    ctx.events.emit(
        CaptchaEventName::Solving,
        json!({"type": kind, "method": SolveMethod::AutoPass, "attempt": 1}),
    );

    let Ok(url) = Url::parse(&ctx.document.url) else {
        return page_failure(PageError::Browser("Invalid challenge URL".into()));
    };
    let origin = url.origin();
    let until = Instant::now() + WAIT;
    let mut state = PageState::Unconfirmed;
    while Instant::now() < until {
        state = match time::timeout_at(until, page_state(ctx.page, &ctx.document, &origin)).await {
            Ok(Ok(state)) => state,
            Ok(Err(PageError::Navigated)) | Err(_) => PageState::Unconfirmed,
            Ok(Err(error)) => return page_failure(error),
        };
        if matches!(state, PageState::Cleared) {
            return terminal(
                SolveStatus::Solved,
                SolveReason::AutoPassed,
                "Cloudflare cleared the page. Continue the task.",
            );
        }
        time::sleep_until(until.min(Instant::now() + POLL)).await;
    }
    if matches!(state, PageState::Wall) {
        terminal(
            SolveStatus::NeedsHuman,
            SolveReason::ChallengeNotPassed,
            "Cloudflare's full-page check is still showing.",
        )
    } else {
        terminal(
            SolveStatus::Failed,
            SolveReason::AcceptanceUnconfirmed,
            "Cloudflare clearance could not be confirmed. Take a fresh snapshot.",
        )
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::PageId;
    use serde_json::Value;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        LiveDriver, SOLVE_DEADLINE,
        orchestration_tests::service,
        types::{Attempts, FrameRef, Rect, SolveOutcome},
    };

    #[derive(Clone, Copy)]
    enum Contents {
        Wall,
        Marker,
        Site,
    }

    #[derive(Clone, Copy)]
    struct Screen {
        loader: &'static str,
        url: &'static str,
        contents: Contents,
        clearance: bool,
    }

    const WALL: Screen = Screen {
        loader: "original",
        url: "https://fixture.test/shop",
        contents: Contents::Wall,
        clearance: false,
    };
    const CLEARED: Screen = Screen {
        loader: "replacement",
        contents: Contents::Site,
        clearance: true,
        ..WALL
    };

    struct Page {
        opened: Instant,
        changes: Vec<(u64, Screen)>,
        cookie_delay: Duration,
        error_on_change: bool,
    }

    impl Page {
        fn new(changes: Vec<(u64, Screen)>) -> Self {
            Self {
                opened: Instant::now(),
                changes,
                cookie_delay: Duration::ZERO,
                error_on_change: false,
            }
        }
        fn current(&self) -> Screen {
            self.changes
                .iter()
                .rev()
                .find(|(ms, _)| self.opened.elapsed() >= Duration::from_millis(*ms))
                .map_or(WALL, |(_, screen)| *screen)
        }
        async fn solve(&self) -> anyhow::Result<SolveOutcome> {
            service()
                .solve_with(
                    self,
                    PageId(42),
                    CancellationToken::new(),
                    false,
                    Instant::now() + SOLVE_DEADLINE,
                    &LiveDriver,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))
        }
    }

    impl CaptchaPage for Page {
        async fn eval_main(&self, _: &str) -> Result<Value, PageError> {
            Ok(json!({"complete":true}))
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            Ok(vec![])
        }
        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            unreachable!("no frames")
        }
        async fn scan_dom(&self, _: Option<&FrameRef>, function: &str) -> Result<Value, PageError> {
            let current = self.current();
            if function.contains("captcha:cloudflare-wall") {
                return Ok(json!(matches!(current.contents, Contents::Wall)));
            }
            let widgets = match current.contents {
                Contents::Site => vec![],
                Contents::Wall | Contents::Marker => vec![json!({
                    "kind":"cloudflare_challenge", "state":"active", "marker":matches!(current.contents, Contents::Marker),
                    "invisible":false, "enterprise":false,
                })],
            };
            Ok(
                json!({"widgets":widgets, "frames":[], "expired":false, "complete":true, "top":true, "url":current.url}),
            )
        }
        async fn document(&self) -> Result<DocumentId, PageError> {
            let current = self.current();
            Ok(DocumentId {
                loader_id: current.loader.into(),
                url: current.url.into(),
            })
        }
        async fn cookie_names(&self, url: &str) -> Result<Vec<String>, PageError> {
            let screen = self.current();
            assert_eq!(url, screen.url);
            time::sleep(self.cookie_delay).await;
            if self.error_on_change && self.current().loader != screen.loader {
                return Err(PageError::Browser("document replaced during read".into()));
            }
            Ok(if screen.clearance {
                vec!["cf_clearance".into()]
            } else {
                vec![]
            })
        }
        async fn click_at(&self, _: f64, _: f64) -> Result<(), PageError> {
            unreachable!("observing a page must not click it")
        }
        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            unreachable!("no frames")
        }
        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            unreachable!("observing a page needs no screenshot")
        }
    }

    #[tokio::test(start_paused = true)]
    async fn observes_clearance_after_the_site_replaces_the_wall() -> anyhow::Result<()> {
        for contents in [Contents::Site, Contents::Marker] {
            let outcome = Page::new(vec![(
                1250,
                Screen {
                    contents,
                    ..CLEARED
                },
            )])
            .solve()
            .await?;
            assert_eq!(
                (outcome.status, outcome.reason, outcome.method),
                (
                    SolveStatus::Solved,
                    SolveReason::AutoPassed,
                    SolveMethod::AutoPass
                )
            );
            assert_eq!(outcome.kind, Some(CaptchaKind::CloudflareChallenge));
            assert_eq!(outcome.attempts, Attempts::default());
            assert_eq!(outcome.paid_tasks, 0);
            assert!((1250..10_000).contains(&outcome.duration_ms));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn hands_persistent_and_reloaded_walls_to_a_person_even_with_a_cookie()
    -> anyhow::Result<()> {
        for screen in [
            WALL,
            Screen {
                clearance: true,
                ..WALL
            },
            Screen {
                loader: "fresh-wall",
                clearance: true,
                ..WALL
            },
        ] {
            let outcome = Page::new(vec![(500, screen)]).solve().await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::NeedsHuman, SolveReason::ChallengeNotPassed)
            );
            assert_eq!(outcome.paid_tasks, 0);
            assert_eq!(outcome.duration_ms, 10_000);
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn clearance_requires_a_replacement_document_and_its_own_cookie() -> anyhow::Result<()> {
        for screen in [
            Screen {
                clearance: false,
                ..CLEARED
            },
            Screen {
                loader: WALL.loader,
                ..CLEARED
            },
            Screen {
                url: "https://unrelated.test/",
                ..CLEARED
            },
        ] {
            let outcome = Page::new(vec![(500, screen)]).solve().await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn a_wall_that_returns_during_the_cookie_read_still_needs_a_person() -> anyhow::Result<()>
    {
        let mut page = Page::new(vec![
            (500, CLEARED),
            (
                750,
                Screen {
                    loader: "fresh-wall",
                    ..WALL
                },
            ),
        ]);
        page.cookie_delay = Duration::from_millis(400);
        for error_on_change in [false, true] {
            page.opened = Instant::now();
            page.error_on_change = error_on_change;
            let outcome = page.solve().await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::NeedsHuman, SolveReason::ChallengeNotPassed)
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn a_background_marker_alone_stays_unsupported_without_waiting() -> anyhow::Result<()> {
        let outcome = Page::new(vec![(
            0,
            Screen {
                contents: Contents::Marker,
                ..WALL
            },
        )])
        .solve()
        .await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::Unsupported, SolveReason::UnsupportedType)
        );
        assert_eq!(outcome.method, SolveMethod::None);
        assert_eq!(outcome.duration_ms, 0);
        Ok(())
    }
}
