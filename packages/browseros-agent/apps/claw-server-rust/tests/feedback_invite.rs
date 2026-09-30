//! In-process coverage of the feedback invitation routes.
//!
//! These drive the real router over real app state, so the checks under test are the ones a
//! cockpit would actually hit: consent, cohort membership and the once-ever rule.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server_rust::{
    AppState, build_router, config::Config, services::feedback_cohort::CohortDocument,
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tower::ServiceExt;

const INVITATION: &str = "/api/v1/feedback/invitation";
const DEFAULT_BOOK_URL: &str = "https://cal.com/team/felafax/browseros";

#[tokio::test]
async fn an_installation_outside_the_cohort_is_never_invited() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    join_cohort(&app, &["someone-else"]).await?;

    let (status, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invitation, json!({ "eligible": false }));
    Ok(())
}

#[tokio::test]
async fn nobody_is_invited_before_a_cohort_is_published() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;

    let (status, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invitation, json!({ "eligible": false }));
    Ok(())
}

#[tokio::test]
async fn a_cohort_member_is_invited_and_reading_the_answer_does_not_spend_it() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invitation["eligible"], true);
    assert_eq!(invitation["bookUrl"], DEFAULT_BOOK_URL);

    // A second page load before anything is recorded still sees it: the invitation is
    // spent by an outcome, not by asking.
    let (_, again) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(again["eligible"], true);
    Ok(())
}

#[tokio::test]
async fn the_published_cohort_can_override_the_booking_link() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    app.state
        .feedback_cohort
        .adopt(
            cohort_document(
                now_ms(),
                &[install_id.as_str()],
                Some("https://cal.test/book"),
            ),
            now_ms(),
        )
        .await;

    let (_, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(invitation["bookUrl"], "https://cal.test/book");
    Ok(())
}

/// The card keeps coming back until it is dismissed. An impression used to spend the
/// invitation, which meant one appearance per installation ever: most of a new tab's
/// openings are incidental, so almost nobody actually read it.
#[tokio::test]
async fn an_impression_does_not_stop_the_card_coming_back() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, recorded) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "shown" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        recorded["eligible"], true,
        "the reply must agree with the next page load"
    );

    let (_, after) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(after["eligible"], true);
    Ok(())
}

#[tokio::test]
async fn booking_stops_the_card_coming_back() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    for outcome in ["shown", "clicked"] {
        request(
            &app.router,
            "POST",
            INVITATION,
            Some(json!({ "outcome": outcome })),
        )
        .await?;
    }

    let (_, after) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(after["eligible"], false);
    Ok(())
}

#[tokio::test]
async fn a_dismissal_ends_it_and_survives_a_restart() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (_, recorded) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "dismissed" })),
    )
    .await?;
    assert_eq!(recorded, json!({ "eligible": false }));

    drop(app);
    let restarted = test_app(dir.path()).await?;
    let same_install = install_id_of(&restarted).await;
    assert_eq!(same_install, install_id);
    join_cohort(&restarted, &[same_install.as_str()]).await?;
    let (_, after_restart) = request(&restarted.router, "GET", INVITATION, None).await?;
    assert_eq!(after_restart, json!({ "eligible": false }));
    Ok(())
}

/// A click landing after a dismissal raises the funnel outcome, because clicking is the
/// stronger claim about what the reader did. It must not un-dismiss the card.
#[tokio::test]
async fn a_click_after_a_dismissal_does_not_bring_the_card_back() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    for outcome in ["shown", "dismissed", "clicked"] {
        request(
            &app.router,
            "POST",
            INVITATION,
            Some(json!({ "outcome": outcome })),
        )
        .await?;
    }

    assert_eq!(
        app.state
            .feedback_invites
            .outcome_of(&install_id)
            .await?
            .as_deref(),
        Some("clicked"),
        "the funnel keeps the stronger claim"
    );
    let (_, after) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(
        after,
        json!({ "eligible": false }),
        "but the reader asked it to stop"
    );
    Ok(())
}

#[tokio::test]
async fn a_dismissal_is_final() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "shown" })),
    )
    .await?;
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "dismissed" })),
    )
    .await?;

    let (_, after) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(after, json!({ "eligible": false }));
    Ok(())
}

/// Consent gates this exactly as it gates the failure reports. An installation that opted
/// out of analytics has not agreed to be contacted either.
#[tokio::test]
async fn an_installation_that_opted_out_is_not_invited() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, _) = request(
        &app.router,
        "PUT",
        "/api/v1/settings/telemetry",
        Some(json!({ "consent": false })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let (_, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(invitation, json!({ "eligible": false }));

    // Nothing is written either, so consenting again later still leaves the invitation
    // available rather than silently spent while the reader was opted out.
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "shown" })),
    )
    .await?;
    request(
        &app.router,
        "PUT",
        "/api/v1/settings/telemetry",
        Some(json!({ "consent": true })),
    )
    .await?;
    let (_, restored) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(restored["eligible"], true);
    Ok(())
}

#[tokio::test]
async fn an_unknown_outcome_is_refused() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, _) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "interested" })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A refused body records nothing, so the invitation is still there.
    let (_, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(invitation["eligible"], true);
    Ok(())
}

/// Anything able to reach the loopback server can POST here, so an outcome must only ever
/// spend an invitation this installation was actually offered. Without the check, one
/// request from an unrelated page would permanently burn it.
#[tokio::test]
async fn an_outcome_cannot_spend_an_invitation_that_was_never_offered() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;

    // No cohort is published, so this installation has been offered nothing.
    let (status, _) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "dismissed" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    // It joins the cohort afterwards and is still invitable: nothing was spent.
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;
    let (_, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(invitation["eligible"], true);
    Ok(())
}

/// The other half of the same rule: once an invitation exists, answering it never consults
/// the cohort again, so a refresh between the card appearing and the reader answering
/// cannot discard what they did.
#[tokio::test]
async fn an_answer_survives_the_cohort_moving_on() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "shown" })),
    )
    .await?;

    // A refresh drops this installation while the card is still on screen.
    join_cohort(&app, &["someone-else"]).await?;
    let (status, _) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "clicked" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        app.state
            .feedback_invites
            .outcome_of(&install_id)
            .await?
            .as_deref(),
        Some("clicked"),
        "the click was recorded even though the cohort had moved on"
    );
    Ok(())
}

/// The whole happy path over HTTP: the card appears, the reader books, then closes the
/// card. Booking is the signal this feature exists to capture, so tidying up afterwards
/// must not erase it.
#[tokio::test]
async fn booking_then_closing_the_card_is_recorded_as_a_click() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    for outcome in ["shown", "clicked", "dismissed"] {
        let (status, _) = request(
            &app.router,
            "POST",
            INVITATION,
            Some(json!({ "outcome": outcome })),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
    }

    assert_eq!(
        app.state
            .feedback_invites
            .outcome_of(&install_id)
            .await?
            .as_deref(),
        Some("clicked"),
        "closing the card after booking must not erase the booking"
    );
    // The invitation is still spent either way.
    let (_, after) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(after, json!({ "eligible": false }));
    Ok(())
}

struct TestApp {
    router: Router,
    state: AppState,
}

#[tokio::test]
async fn every_offered_round_remains_answerable_after_cohort_removal_before_its_first_post()
-> anyhow::Result<()> {
    use claw_server_rust::db::feedback_invite::{INVITATION_COOLDOWN_MS, InviteOutcome};

    for round in 1..=3 {
        for outcome in ["shown", "clicked", "dismissed"] {
            let dir = tempfile::tempdir()?;
            let app = test_app(dir.path()).await?;
            let install_id = install_id_of(&app).await;
            join_cohort(&app, &[install_id.as_str()]).await?;
            for previous in 1..round {
                app.state
                    .feedback_invites
                    .record(
                        &install_id,
                        InviteOutcome::Dismissed,
                        now_ms() - i64::from(4 - previous) * INVITATION_COOLDOWN_MS,
                        previous,
                    )
                    .await?;
            }
            let (_, offered) = request(
                &app.router,
                "GET",
                "/api/v1/feedback/invitation?supportsRounds=true",
                None,
            )
            .await?;
            assert_eq!(offered["round"], round);
            assert_eq!(
                app.state
                    .feedback_invites
                    .recorded_round(&install_id)
                    .await?,
                (round > 1).then_some(round - 1)
            );

            join_cohort(&app, &["someone-else"]).await?;
            let (status, _) = request(
                &app.router,
                "POST",
                INVITATION,
                Some(json!({"outcome": outcome, "round": round})),
            )
            .await?;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                app.state
                    .feedback_invites
                    .recorded_round(&install_id)
                    .await?,
                Some(round)
            );
            assert_eq!(
                app.state
                    .feedback_invites
                    .has_dismissed(&install_id)
                    .await?,
                outcome == "dismissed"
            );
            if outcome == "clicked" {
                assert_eq!(
                    app.state
                        .feedback_invites
                        .outcome_of(&install_id)
                        .await?
                        .as_deref(),
                    Some("clicked")
                );
            }
            join_cohort(&app, &[install_id.as_str()]).await?;
            let (_, after) = request(
                &app.router,
                "GET",
                "/api/v1/feedback/invitation?supportsRounds=true",
                None,
            )
            .await?;
            assert_eq!(after["eligible"], outcome == "shown");
        }
    }
    Ok(())
}

#[tokio::test]
async fn an_unanswered_offer_survives_a_restart_and_cohort_removal() -> anyhow::Result<()> {
    use claw_server_rust::db::feedback_invite::{INVITATION_COOLDOWN_MS, InviteOutcome};

    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;
    app.state
        .feedback_invites
        .record(
            &install_id,
            InviteOutcome::Dismissed,
            now_ms() - INVITATION_COOLDOWN_MS,
            1,
        )
        .await?;
    let (_, offered) = request(
        &app.router,
        "GET",
        "/api/v1/feedback/invitation?supportsRounds=true",
        None,
    )
    .await?;
    assert_eq!(offered["round"], 2);
    drop(app);

    let app = test_app(dir.path()).await?;
    join_cohort(&app, &["someone-else"]).await?;
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "dismissed", "round": 2})),
    )
    .await?;
    assert_eq!(
        app.state
            .feedback_invites
            .recorded_round(&install_id)
            .await?,
        Some(2)
    );
    assert!(
        app.state
            .feedback_invites
            .has_dismissed(&install_id)
            .await?
    );
    Ok(())
}

#[tokio::test]
async fn a_late_legacy_booking_click_stops_later_rounds_even_outside_the_cohort()
-> anyhow::Result<()> {
    use claw_server_rust::db::feedback_invite::{INVITATION_COOLDOWN_MS, InviteOutcome};

    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;
    app.state
        .feedback_invites
        .record(
            &install_id,
            InviteOutcome::Dismissed,
            now_ms() - INVITATION_COOLDOWN_MS,
            1,
        )
        .await?;
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "shown", "round": 2})),
    )
    .await?;
    join_cohort(&app, &["someone-else"]).await?;
    let (_, clicked) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "clicked"})),
    )
    .await?;
    assert_eq!(clicked, json!({"eligible": false}));
    assert_eq!(
        app.state
            .feedback_invites
            .recorded_round(&install_id)
            .await?,
        Some(2)
    );
    assert_eq!(
        app.state
            .feedback_invites
            .outcome_of(&install_id)
            .await?
            .as_deref(),
        Some("clicked")
    );
    join_cohort(&app, &[install_id.as_str()]).await?;
    let (_, after) = request(
        &app.router,
        "GET",
        "/api/v1/feedback/invitation?supportsRounds=true",
        None,
    )
    .await?;
    assert_eq!(after, json!({"eligible": false}));
    Ok(())
}

#[tokio::test]
async fn round_aware_clients_receive_and_report_the_first_round() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, invitation) = request(
        &app.router,
        "GET",
        "/api/v1/feedback/invitation?supportsRounds=true",
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invitation["round"], 1);
    for outcome in ["shown", "clicked"] {
        let (_, reply) = request(
            &app.router,
            "POST",
            INVITATION,
            Some(json!({"outcome": outcome, "round": 1})),
        )
        .await?;
        assert_eq!(reply["eligible"], outcome == "shown");
        if outcome == "shown" {
            assert_eq!(reply["round"], 1);
        } else {
            assert_eq!(reply, json!({"eligible": false}));
        }
    }
    let (_, reply) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "dismissed", "round": 1})),
    )
    .await?;
    assert_eq!(reply, json!({"eligible": false}));
    let (_, early) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "shown", "round": 2})),
    )
    .await?;
    assert_eq!(early, json!({"eligible": false}));
    assert_eq!(
        app.state
            .feedback_invites
            .recorded_round(&install_id)
            .await?,
        Some(1)
    );
    Ok(())
}

#[tokio::test]
async fn only_round_aware_clients_return_after_the_cooldown() -> anyhow::Result<()> {
    use claw_server_rust::db::feedback_invite::{INVITATION_COOLDOWN_MS, InviteOutcome};

    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;
    app.state
        .feedback_invites
        .record(
            &install_id,
            InviteOutcome::Dismissed,
            now_ms() - INVITATION_COOLDOWN_MS,
            1,
        )
        .await?;

    let (_, legacy) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(legacy, json!({"eligible": false}));
    let (_, invitation) = request(
        &app.router,
        "GET",
        "/api/v1/feedback/invitation?supportsRounds=true",
        None,
    )
    .await?;
    assert_eq!(invitation["eligible"], true);
    assert_eq!(invitation["round"], 2);
    let (_, shown) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "shown", "round": 2})),
    )
    .await?;
    assert_eq!(shown["round"], 2);

    for body in [
        json!({"outcome": "dismissed"}),
        json!({"outcome": "dismissed", "round": 1}),
    ] {
        request(&app.router, "POST", INVITATION, Some(body)).await?;
    }
    let (_, after) = request(
        &app.router,
        "GET",
        "/api/v1/feedback/invitation?supportsRounds=true",
        None,
    )
    .await?;
    assert_eq!(after["round"], 2);
    let (_, legacy) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(legacy, json!({"eligible": false}));
    let (_, dismissed) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "dismissed", "round": 2})),
    )
    .await?;
    assert_eq!(dismissed, json!({"eligible": false}));
    Ok(())
}

#[tokio::test]
async fn later_rounds_require_cohort_membership_but_current_answers_survive_its_removal()
-> anyhow::Result<()> {
    use claw_server_rust::db::feedback_invite::{INVITATION_COOLDOWN_MS, InviteOutcome};

    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    app.state
        .feedback_invites
        .record(
            &install_id,
            InviteOutcome::Dismissed,
            now_ms() - INVITATION_COOLDOWN_MS,
            1,
        )
        .await?;
    join_cohort(&app, &["someone-else"]).await?;
    let (_, refused) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "shown", "round": 2})),
    )
    .await?;
    assert_eq!(refused, json!({"eligible": false}));
    assert_eq!(
        app.state
            .feedback_invites
            .recorded_round(&install_id)
            .await?,
        Some(1)
    );

    join_cohort(&app, &[install_id.as_str()]).await?;
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "shown", "round": 2})),
    )
    .await?;
    join_cohort(&app, &["someone-else"]).await?;
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "dismissed", "round": 2})),
    )
    .await?;
    assert_eq!(
        app.state
            .feedback_invites
            .recorded_round(&install_id)
            .await?,
        Some(2)
    );
    assert!(
        app.state
            .feedback_invites
            .has_dismissed(&install_id)
            .await?
    );
    Ok(())
}

#[tokio::test]
async fn the_third_dismissal_stops_round_aware_clients_permanently() -> anyhow::Result<()> {
    use claw_server_rust::db::feedback_invite::{INVITATION_COOLDOWN_MS, InviteOutcome};

    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;
    let start = now_ms() - 4 * INVITATION_COOLDOWN_MS;
    for round in 1..=3 {
        app.state
            .feedback_invites
            .record(
                &install_id,
                InviteOutcome::Dismissed,
                start + i64::from(round) * INVITATION_COOLDOWN_MS,
                round,
            )
            .await?;
    }
    let (_, invitation) = request(
        &app.router,
        "GET",
        "/api/v1/feedback/invitation?supportsRounds=true",
        None,
    )
    .await?;
    assert_eq!(invitation, json!({"eligible": false}));
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({"outcome": "shown", "round": 3})),
    )
    .await?;
    let (_, after) = request(
        &app.router,
        "GET",
        "/api/v1/feedback/invitation?supportsRounds=true",
        None,
    )
    .await?;
    assert_eq!(after, json!({"eligible": false}));
    Ok(())
}

#[tokio::test]
async fn invalid_rounds_and_capabilities_are_rejected() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    for round in [json!(0), json!(4), json!(-1), json!(1.5), json!("2")] {
        let (status, _) = request(
            &app.router,
            "POST",
            INVITATION,
            Some(json!({"outcome": "shown", "round": round})),
        )
        .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, _) = request(
        &app.router,
        "GET",
        "/api/v1/feedback/invitation?supportsRounds=invalid",
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

async fn test_app(root: &Path) -> anyhow::Result<TestApp> {
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: root.join("resources"),
        browserclaw_dir: root.join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
    });
    let state = AppState::new_with_home(config, root.join("home")).await?;
    Ok(TestApp {
        router: build_router(state.clone()),
        state,
    })
}

async fn install_id_of(app: &TestApp) -> String {
    app.state.analytics.get_state().await.distinct_id
}

async fn join_cohort(app: &TestApp, installs: &[&str]) -> anyhow::Result<()> {
    let now = now_ms();
    anyhow::ensure!(
        app.state
            .feedback_cohort
            .adopt(cohort_document(now, installs, None), now)
            .await,
        "fixture cohort was refused"
    );
    Ok(())
}

fn cohort_document(
    generated_at_ms: i64,
    installs: &[&str],
    book_url: Option<&str>,
) -> CohortDocument {
    serde_json::from_value(json!({
        "generated_at_ms": generated_at_ms,
        "installs": installs,
        "copy": { "book_url": book_url },
    }))
    .unwrap_or_else(|error| panic!("fixture: {error}"))
}

fn now_ms() -> i64 {
    claw_server_rust::services::feedback_cohort::now_ms()
}

async fn request(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    let request_body = if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = router.clone().oneshot(builder.body(request_body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}
