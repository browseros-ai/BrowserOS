use std::time::Duration;

use super::{MAX_TIMER_MS, arm_expiry, own_line, unchanged};
use crate::input::{ClickOptions, InputDevices, Modifiers, Platform, Point};

#[test]
fn tests_that_an_interceptor_outlives_every_event_its_op_can_still_send() {
    let mut devices = InputDevices::new(Platform::Linux);
    let slow = devices.mouse.click(
        Point { x: 5.0, y: 5.0 },
        ClickOptions {
            click_count: 2,
            delay: Duration::from_millis(3500),
            ..ClickOptions::default()
        },
        Modifiers::default(),
    );
    assert_eq!(
        arm_expiry(Some(Duration::from_secs(5)), &slow),
        8000,
        "3 s past the op's timeout, after which the op sends nothing"
    );
    assert_eq!(
        arm_expiry(None, &slow),
        13_500,
        "an op without a timeout: 3 s past its burst's three 3.5 s delays"
    );
    assert_eq!(
        arm_expiry(Some(Duration::from_secs(u64::MAX)), &slow),
        MAX_TIMER_MS,
        "a timeout too long for setTimeout, which would fire at once, is the longest it keeps"
    );
}

/// Covered-click C: the call log's one line per case, in the fixed form a host counts with one
/// match, and a checking action's error that names which of the target's own took the press.
#[test]
fn tests_that_a_press_of_the_targets_own_says_which_in_one_form() {
    use crate::probe::{OwnCase, OwnPress, Point as FramePoint};

    let press = |case, markup: Option<&str>, position| OwnPress {
        case,
        markup: markup.map(str::to_owned),
        position,
        checked: None,
    };
    assert_eq!(
        own_line(&press(
            OwnCase::Label,
            Some("<label for=\"terms\">…</label>"),
            None
        )),
        "  its own label takes the press: <label for=\"terms\">…</label>"
    );
    assert_eq!(
        own_line(&press(
            OwnCase::Wrapper,
            Some("<div class=\"option\">…</div>"),
            None
        )),
        "  its own wrapper takes the press: <div class=\"option\">…</div>"
    );
    assert_eq!(
        own_line(&press(
            OwnCase::Point,
            None,
            Some(FramePoint { x: 12.4, y: 7.6 })
        )),
        "  a clear point of it takes the press: { x: 12, y: 8 }"
    );
    assert_eq!(
        unchanged(None),
        "Clicking the checkbox did not change its state"
    );
    assert_eq!(
        unchanged(Some(OwnCase::Label)),
        "Clicking the checkbox did not change its state: its own label took the press"
    );
    assert_eq!(
        unchanged(Some(OwnCase::Wrapper)),
        "Clicking the checkbox did not change its state: its own wrapper took the press"
    );
}
