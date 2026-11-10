use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use super::{BurstFailure, PageInput, TabInput};
use crate::input::input_devices::{InputDevices, InputStep, approach};
use crate::input::keyboard::{Modifier, Modifiers, Platform};
use crate::input::mouse::{ClickOptions, MouseButton, Point};
use crate::op::Op;
use crate::op_scope::OpScope;
use crate::target::CallHeader;
use crate::test_env::{Recording, TestEnv};

/// Each recorded input event as `method type`.
fn events(recording: &Recording) -> Vec<String> {
    recording
        .sent()
        .into_iter()
        .map(|(method, params)| format!("{method} {}", params["type"].as_str().unwrap_or("")))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_burst_stopped_part_way_releases_what_it_pressed_before_the_next_burst()
-> Result<(), Box<dyn Error>> {
    let recording = Arc::new(Recording::default());
    let session = recording.session("tab");
    let tab = Arc::new(Mutex::new(TabInput::new(Platform::Linux)));
    let env = TestEnv::new();
    let op = Op::from_name("locator.click").ok_or("locator.click is an op")?;
    let header = CallHeader {
        api: None,
        timeout_ms: Some(100),
    };
    let scope = OpScope::new(&env, op, header);
    let sent = scope.send_effect()?;

    let mut input = PageInput::hold(Arc::clone(&tab), session.clone(), 1.0).await;
    let mut planned = input.plan();
    let devices = &mut planned;
    let shift = Modifiers::default().with(Modifier::Shift);
    let (restore, mut steps) = devices.keyboard.ensure_modifiers(shift)?;
    let held = devices.keyboard.modifiers();
    steps.extend(devices.mouse.click(
        Point { x: 5.0, y: 5.0 },
        ClickOptions {
            delay: Duration::from_secs(1),
            ..ClickOptions::default()
        },
        held,
    ));
    let (_, release) = devices.keyboard.ensure_modifiers(restore)?;
    steps.extend(release);
    let stopped = input.send_planned(&sent, &scope, planned, steps).await;
    assert!(
        matches!(stopped, Err(BurstFailure::Expired)),
        "the op's 100 ms ran out inside the click's 1 s delay"
    );
    assert_eq!(
        events(&recording),
        [
            "Input.dispatchKeyEvent rawKeyDown",
            "Input.dispatchMouseEvent mouseMoved",
            "Input.dispatchMouseEvent mousePressed",
        ]
    );
    drop(input);

    let next = PageInput::hold(tab, session, 1.0).await;
    assert_eq!(
        events(&recording)[3..],
        [
            "Input.dispatchMouseEvent mouseReleased",
            "Input.dispatchKeyEvent keyUp",
        ],
        "the button and Shift are let go before the next holder sends anything"
    );
    assert_eq!(
        next.plan().keyboard.modifiers(),
        Modifiers::default(),
        "and the state already said so"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_finished_burst_owes_nothing() -> Result<(), Box<dyn Error>> {
    let recording = Arc::new(Recording::default());
    let session = recording.session("tab");
    let tab = Arc::new(Mutex::new(TabInput::new(Platform::Linux)));
    let env = TestEnv::new();
    let op = Op::from_name("locator.click").ok_or("locator.click is an op")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let sent = scope.send_effect()?;
    let mut input = PageInput::hold(Arc::clone(&tab), session.clone(), 2.0).await;
    let mut planned = input.plan();
    let modifiers = planned.keyboard.modifiers();
    let steps = planned
        .mouse
        .click(Point { x: 5.0, y: 6.0 }, ClickOptions::default(), modifiers);
    input
        .send_planned(&sent, &scope, planned, steps)
        .await
        .map_err(|failure| format!("{failure:?}"))?;
    drop(input);
    drop(PageInput::hold(tab, session, 2.0).await);
    let sent = recording.sent();
    assert_eq!(sent.len(), 3, "move, press, release, and nothing after");
    assert_eq!(
        (sent[1].1["x"].as_f64(), sent[1].1["y"].as_f64()),
        (Some(10.0), Some(12.0)),
        "positions take the pointer scale"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_button_let_go_is_released_once_before_the_next_holder_sends()
-> Result<(), Box<dyn Error>> {
    let recording = Arc::new(Recording::default());
    let session = recording.session("tab");
    let tab = Arc::new(Mutex::new(TabInput::new(Platform::Linux)));
    let env = TestEnv::new();
    let op = Op::from_name("locator.dragTo").ok_or("locator.dragTo is an op")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let sent = scope.send_effect()?;

    // A drag's press, whose release never came: the destination was not found.
    let mut input = PageInput::hold(Arc::clone(&tab), session.clone(), 1.0).await;
    let mut planned = input.plan();
    let modifiers = planned.keyboard.modifiers();
    let mut steps = planned
        .mouse
        .move_to(Point { x: 5.0, y: 5.0 }, 1, modifiers);
    steps.extend(planned.mouse.down(MouseButton::Left, 1, modifiers));
    input
        .send_planned(&sent, &scope, planned, steps)
        .await
        .map_err(|failure| format!("{failure:?}"))?;
    input.let_go(MouseButton::Left);
    input.let_go(MouseButton::Left);
    drop(input);

    let next = PageInput::hold(tab, session, 1.0).await;
    assert_eq!(
        events(&recording),
        [
            "Input.dispatchMouseEvent mouseMoved",
            "Input.dispatchMouseEvent mousePressed",
            "Input.dispatchMouseEvent mouseReleased",
        ],
        "the button went up once, before the next holder sent anything"
    );
    assert!(
        !next.plan().mouse.holds(MouseButton::Left),
        "and the state says it is up"
    );
    Ok(())
}

/// A Shift-click at (5, 5), as a pointer action with `modifiers: ['Shift']` builds it.
fn shift_click(devices: &mut InputDevices) -> Result<Vec<InputStep>, Box<dyn Error>> {
    let shift = Modifiers::default().with(Modifier::Shift);
    let (restore, mut steps) = devices.keyboard.ensure_modifiers(shift)?;
    let held = devices.keyboard.modifiers();
    steps.extend(
        devices
            .mouse
            .click(Point { x: 5.0, y: 5.0 }, ClickOptions::default(), held),
    );
    steps.extend(devices.keyboard.ensure_modifiers(restore)?.1);
    Ok(steps)
}

#[tokio::test]
async fn tests_that_a_burst_held_at_its_approach_repeats_only_the_move_and_owes_only_its_keys()
-> Result<(), Box<dyn Error>> {
    let recording = Arc::new(Recording::default());
    let session = recording.session("tab");
    let tab = Arc::new(Mutex::new(TabInput::new(Platform::Linux)));
    let env = TestEnv::new();
    let op = Op::from_name("locator.click").ok_or("locator.click is an op")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let sent = scope.send_effect()?;
    let failed = |failure: BurstFailure| format!("{failure:?}");

    let mut input = PageInput::hold(Arc::clone(&tab), session.clone(), 1.0).await;
    let steps = shift_click(&mut input.plan())?;
    let to_press = approach(&steps).ok_or("a click has an approach")?;
    input
        .send_first(&sent, &scope, steps, to_press)
        .await
        .map_err(failed)?;
    input.repeat_move(&sent, &scope).await.map_err(failed)?;
    drop(input);
    drop(PageInput::hold(Arc::clone(&tab), session.clone(), 1.0).await);
    assert_eq!(
        events(&recording),
        [
            "Input.dispatchKeyEvent rawKeyDown",
            "Input.dispatchMouseEvent mouseMoved",
            "Input.dispatchMouseEvent mouseMoved",
            "Input.dispatchKeyEvent keyUp",
        ],
        "an attempt given up at its approach sent no button, and let Shift go"
    );

    let mut input = PageInput::hold(tab, session, 1.0).await;
    let mut planned = input.plan();
    let steps = shift_click(&mut planned)?;
    let to_press = approach(&steps).ok_or("a click has an approach")?;
    input
        .send_first(&sent, &scope, steps, to_press)
        .await
        .map_err(failed)?;
    input.repeat_move(&sent, &scope).await.map_err(failed)?;
    input.resume(&sent, &scope, planned).await.map_err(failed)?;
    assert_eq!(
        events(&recording).get(4..),
        Some(
            [
                "Input.dispatchKeyEvent rawKeyDown",
                "Input.dispatchMouseEvent mouseMoved",
                "Input.dispatchMouseEvent mouseMoved",
                "Input.dispatchMouseEvent mousePressed",
                "Input.dispatchMouseEvent mouseReleased",
                "Input.dispatchKeyEvent keyUp",
            ]
            .map(str::to_owned)
            .as_slice()
        ),
        "one that reached its frame sends the rest of its burst after the repeated move"
    );
    Ok(())
}
