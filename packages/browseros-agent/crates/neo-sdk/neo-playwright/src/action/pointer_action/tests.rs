use std::time::Duration;

use super::{MAX_TIMER_MS, arm_expiry};
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
