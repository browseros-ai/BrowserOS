//! The fit's routes as a host calls them: the contract's `Fit` and `NavigateResult`, the
//! statuses and codes of every refusal, the tab's close, and the screen put home at start.
//!
//! The scripted browser answers the fit's window calls but not the page's readback of a
//! viewport, so a mobile fit here is `partial`: the browser-side behavior of a fit is the
//! takeover crate's own tests' and the real surface's.

use crate::support::{HOME_SCREEN, TestResult, harness, harness_after_a_crash_mid_fit};
use axum::http::StatusCode;
use browseros_core::TabId;
use browseros_takeover::{FitId, ScreenSize};
use serde_json::json;

const PHONE: ScreenSize = ScreenSize {
    width: 390,
    height: 844,
};

#[tokio::test]
async fn a_fit_answers_with_the_contracts_fit_and_restores_once() -> TestResult {
    let harness = harness().await?;
    harness.browser.open_tab(12);
    let fitted = harness
        .post("/v1/tabs/12/fit", r#"{"width":390,"height":844}"#)
        .await?;
    assert_eq!(fitted.status, StatusCode::OK, "{}", fitted.body);
    let id = fitted.body["id"].as_str().ok_or("an id")?.to_string();
    assert!(id.starts_with("ft_"), "{id}");
    assert_eq!(fitted.body["tabId"], 12);
    assert_eq!(
        fitted.body["screen"],
        json!({ "width": 390, "height": 844 })
    );
    assert_eq!(fitted.body["mode"], "mobile");
    assert!(
        matches!(fitted.body["fit"].as_str(), Some("fitted" | "partial")),
        "{}",
        fitted.body
    );
    assert!(fitted.body.get("restoredAt").is_none());
    assert_eq!(harness.xvnc.size(), PHONE);
    assert_eq!(harness.browser.window(), "fullscreen");

    let read = harness.get(&format!("/v1/fits/{id}")).await?;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["id"], id.as_str());

    let refused = harness
        .post(
            &format!("/v1/fits/{id}/navigate"),
            r#"{"action":"open","input":"javascript:alert(1)"}"#,
        )
        .await?;
    assert_eq!(refused.status, StatusCode::OK, "{}", refused.body);
    assert_eq!(refused.body["status"], "refused");
    assert!(refused.body["notice"].is_string());

    let restored = harness.post(&format!("/v1/fits/{id}/restore"), "").await?;
    assert_eq!(restored.status, StatusCode::OK, "{}", restored.body);
    assert!(restored.body["restoredAt"].is_i64(), "{}", restored.body);
    assert_eq!(harness.xvnc.size(), HOME_SCREEN);
    assert_eq!(harness.browser.window(), "maximized");
    let again = harness.post(&format!("/v1/fits/{id}/restore"), "").await?;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body, restored.body, "restoring again changes nothing");

    harness
        .post(&format!("/v1/fits/{id}/navigate"), r#"{"action":"back"}"#)
        .await?
        .assert_error(StatusCode::CONFLICT, "fit_restored");
    Ok(())
}

#[tokio::test]
async fn fit_requests_that_name_nothing_or_do_not_parse_are_refused() -> TestResult {
    let harness = harness().await?;
    harness.browser.open_tab(12);
    harness
        .post("/v1/tabs/99/fit", r#"{"width":390,"height":844}"#)
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    for body in [
        "",
        "{}",
        "nope",
        r#"{"width":99,"height":844}"#,
        r#"{"width":390,"height":4097}"#,
        r#"{"width":-1,"height":844}"#,
    ] {
        harness
            .post("/v1/tabs/12/fit", body)
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    harness
        .post("/v1/tabs/twelve/fit", r#"{"width":390,"height":844}"#)
        .await?
        .assert_error(StatusCode::BAD_REQUEST, "invalid_request");

    let unknown = FitId::new();
    for path in ["/v1/fits/nope".to_string(), format!("/v1/fits/{unknown}")] {
        harness
            .get(&path)
            .await?
            .assert_error(StatusCode::NOT_FOUND, "not_found");
        harness
            .post(&format!("{path}/restore"), "")
            .await?
            .assert_error(StatusCode::NOT_FOUND, "not_found");
    }
    harness
        .post(
            &format!("/v1/fits/{unknown}/navigate"),
            r#"{"action":"back"}"#,
        )
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    let too_long = json!({ "action": "open", "input": "a".repeat(2001) }).to_string();
    for body in [
        r#"{"action":"sideways"}"#,
        r#"{"action":"open"}"#,
        too_long.as_str(),
        "{}",
    ] {
        harness
            .post(&format!("/v1/fits/{unknown}/navigate"), body)
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    assert_eq!(
        harness.xvnc.resizes().len(),
        1,
        "only the screen put home at start"
    );
    Ok(())
}

#[tokio::test]
async fn a_screen_or_a_browser_neo_cannot_reach_is_503() -> TestResult {
    let harness = harness().await?;
    harness.browser.open_tab(12);
    harness.xvnc.refuse_next(3);
    harness
        .post("/v1/tabs/12/fit", r#"{"width":390,"height":844}"#)
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "display_unavailable");
    assert_eq!(harness.xvnc.size(), HOME_SCREEN);

    harness.browser.become_unreachable();
    harness
        .post("/v1/tabs/12/fit", r#"{"width":390,"height":844}"#)
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "browser_unavailable");
    Ok(())
}

#[tokio::test]
async fn a_tab_that_closes_restores_its_fit() -> TestResult {
    let harness = harness().await?;
    harness.browser.open_tab(12);
    let fitted = harness
        .post("/v1/tabs/12/fit", r#"{"width":390,"height":844}"#)
        .await?;
    let id = fitted.body["id"].as_str().ok_or("an id")?.to_string();
    harness.browser.close_tab(12);
    harness.service.tab_closed(TabId(12)).await;
    let read = harness.get(&format!("/v1/fits/{id}")).await?;
    assert!(read.body["restoredAt"].is_i64(), "{}", read.body);
    assert_eq!(harness.xvnc.size(), HOME_SCREEN);
    assert_eq!(harness.browser.window(), "maximized");
    Ok(())
}

#[tokio::test]
async fn the_service_puts_the_screen_and_the_window_home_at_start() -> TestResult {
    let harness = harness_after_a_crash_mid_fit(PHONE).await?;
    assert_eq!(harness.xvnc.size(), HOME_SCREEN);
    // The window leaves fullscreen right after the resize, under the same start task.
    for _ in 0..100 {
        if harness.browser.window() == "maximized" {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Err(format!("the window is still {}", harness.browser.window()).into())
}
