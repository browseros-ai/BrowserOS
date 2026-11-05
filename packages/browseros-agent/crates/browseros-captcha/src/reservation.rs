use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{Arc, Mutex},
    time::Instant,
};

use super::types::{CaptchaKind, SolveStep};

#[derive(Debug, Clone)]
pub struct ActiveSolve {
    pub request_id: String,
    pub page: u32,
    pub kind: Option<CaptchaKind>,
    pub step: SolveStep,
    pub started_at: Instant,
}

#[derive(Default)]
pub struct TabReservations {
    active: Mutex<HashMap<u32, ActiveSolve>>,
}

impl TabReservations {
    pub fn try_reserve(
        self: &Arc<Self>,
        page: u32,
        request_id: &str,
    ) -> Result<SolveLease, ActiveSolve> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match active.entry(page) {
            Entry::Occupied(entry) => return Err(entry.get().clone()),
            Entry::Vacant(entry) => {
                entry.insert(ActiveSolve {
                    request_id: request_id.to_owned(),
                    page,
                    kind: None,
                    step: SolveStep::Detecting,
                    started_at: Instant::now(),
                });
            }
        }
        Ok(SolveLease {
            reservations: Arc::clone(self),
            page,
            request_id: request_id.to_owned(),
        })
    }
    pub fn active(&self, page: u32) -> Option<ActiveSolve> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&page)
            .cloned()
    }
    pub fn busy_message(&self, page: u32) -> Option<String> {
        let active = self.active(page)?;
        Some(format!(
            "page {page} is busy: solve_captcha (request {}) is working on {} CAPTCHA ({}, {}s). Wait for it to return, then take a fresh snapshot.",
            active.request_id,
            kind_with_article(active.kind),
            step_label(active.step),
            active.started_at.elapsed().as_secs()
        ))
    }
    pub fn status_line(&self, page: u32) -> Option<String> {
        let active = self.active(page)?;
        Some(format!(
            "CAPTCHA: solve_captcha (request {}) is solving {} CAPTCHA on this page — {}, {}s elapsed. Do not act on this page until it returns.",
            active.request_id,
            kind_with_article(active.kind),
            step_label(active.step),
            active.started_at.elapsed().as_secs()
        ))
    }
}

fn kind_with_article(kind: Option<CaptchaKind>) -> String {
    let kind = kind.unwrap_or(CaptchaKind::Unknown);
    let article = match kind {
        CaptchaKind::Unknown | CaptchaKind::Arkose | CaptchaKind::AwsWaf => "an",
        _ => "a",
    };
    format!("{article} {}", kind.as_str())
}

fn step_label(step: SolveStep) -> String {
    match step {
        SolveStep::Detecting => "detecting".into(),
        SolveStep::AutoPass => "waiting for auto-pass".into(),
        SolveStep::Checkbox => "clicking the checkbox".into(),
        SolveStep::Tiles { round } => format!("image round {round}"),
        SolveStep::Token => "applying a token".into(),
        SolveStep::Verifying => "verifying".into(),
    }
}

pub struct SolveLease {
    reservations: Arc<TabReservations>,
    page: u32,
    request_id: String,
}

impl SolveLease {
    pub fn set_progress(&self, kind: Option<CaptchaKind>, step: SolveStep) {
        let mut active = self
            .reservations
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(active) = active.get_mut(&self.page)
            && active.request_id == self.request_id
        {
            active.kind = kind;
            active.step = step;
        }
    }
    pub fn page(&self) -> u32 {
        self.page
    }
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

impl Drop for SolveLease {
    fn drop(&mut self) {
        let mut active = self
            .reservations
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active
            .get(&self.page)
            .is_some_and(|active| active.request_id == self.request_id)
        {
            active.remove(&self.page);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace_reservation(
        reservations: &Arc<TabReservations>,
    ) -> anyhow::Result<(SolveLease, SolveLease)> {
        let stale = reservations
            .try_reserve(42, "cap_stale")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        reservations
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .remove(&42);
        let current = reservations
            .try_reserve(42, "cap_current")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        Ok((stale, current))
    }

    #[test]
    fn tests_that_a_stale_lease_cannot_release_a_new_reservation() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let (stale, _current) = replace_reservation(&reservations)?;
        drop(stale);
        assert_eq!(
            reservations.active(42).map(|active| active.request_id),
            Some("cap_current".into())
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_stale_lease_cannot_update_a_new_reservation() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let (stale, _current) = replace_reservation(&reservations)?;
        stale.set_progress(Some(CaptchaKind::Turnstile), SolveStep::Token);
        let active = reservations
            .active(42)
            .ok_or_else(|| anyhow::anyhow!("missing"))?;
        assert_eq!(active.kind, None);
        assert_eq!(active.step, SolveStep::Detecting);
        Ok(())
    }

    #[test]
    fn tests_that_elapsed_text_uses_whole_seconds() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let _lease = reservations
            .try_reserve(42, "cap_test")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        reservations
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .get_mut(&42)
            .ok_or_else(|| anyhow::anyhow!("missing"))?
            .started_at = Instant::now() - std::time::Duration::from_millis(12_500);
        assert_eq!(reservations.busy_message(42), Some("page 42 is busy: solve_captcha (request cap_test) is working on an unknown CAPTCHA (detecting, 12s). Wait for it to return, then take a fresh snapshot.".into()));
        assert_eq!(reservations.status_line(42), Some("CAPTCHA: solve_captcha (request cap_test) is solving an unknown CAPTCHA on this page — detecting, 12s elapsed. Do not act on this page until it returns.".into()));
        Ok(())
    }

    #[test]
    fn tests_that_busy_and_status_text_choose_the_article_for_the_kind() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_test")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        for (kind, phrase) in [
            (None, "an unknown"),
            (Some(CaptchaKind::Unknown), "an unknown"),
            (Some(CaptchaKind::RecaptchaV2), "a recaptcha_v2"),
            (Some(CaptchaKind::Turnstile), "a turnstile"),
            (Some(CaptchaKind::Arkose), "an arkose"),
            (Some(CaptchaKind::AwsWaf), "an aws_waf"),
        ] {
            lease.set_progress(kind, SolveStep::Detecting);
            assert_eq!(
                reservations.busy_message(42),
                Some(format!(
                    "page 42 is busy: solve_captcha (request cap_test) is working on {phrase} CAPTCHA (detecting, 0s). Wait for it to return, then take a fresh snapshot."
                ))
            );
            assert_eq!(
                reservations.status_line(42),
                Some(format!(
                    "CAPTCHA: solve_captcha (request cap_test) is solving {phrase} CAPTCHA on this page — detecting, 0s elapsed. Do not act on this page until it returns."
                ))
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_reservations_refuse_duplicates_and_release_on_drop() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_first")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        assert_eq!(lease.page(), 42);
        assert_eq!(lease.request_id(), "cap_first");
        let active = reservations
            .active(42)
            .ok_or_else(|| anyhow::anyhow!("reservation missing"))?;
        assert_eq!(active.request_id, "cap_first");
        assert_eq!(active.kind, None);
        assert_eq!(active.step, SolveStep::Detecting);
        assert!(
            matches!(reservations.try_reserve(42, "cap_second"), Err(active) if active.request_id == "cap_first")
        );
        let other = reservations
            .try_reserve(7, "cap_other")
            .map_err(|_| anyhow::anyhow!("other page busy"))?;
        drop(lease);
        assert!(reservations.active(42).is_none());
        assert!(reservations.busy_message(42).is_none());
        assert!(reservations.status_line(42).is_none());
        assert!(reservations.active(other.page()).is_some());
        assert!(reservations.try_reserve(42, "cap_next").is_ok());
        Ok(())
    }

    #[test]
    fn tests_that_progress_is_visible_in_busy_and_snapshot_text() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_test")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        for (step, label) in [
            (SolveStep::Detecting, "detecting"),
            (SolveStep::AutoPass, "waiting for auto-pass"),
            (SolveStep::Checkbox, "clicking the checkbox"),
            (SolveStep::Tiles { round: 3 }, "image round 3"),
            (SolveStep::Token, "applying a token"),
            (SolveStep::Verifying, "verifying"),
        ] {
            lease.set_progress(Some(CaptchaKind::RecaptchaV2), step);
            assert_eq!(
                reservations.active(42).map(|active| active.step),
                Some(step)
            );
            assert_eq!(
                reservations.busy_message(42),
                Some(format!(
                    "page 42 is busy: solve_captcha (request cap_test) is working on a recaptcha_v2 CAPTCHA ({label}, 0s). Wait for it to return, then take a fresh snapshot."
                ))
            );
            assert_eq!(
                reservations.status_line(42),
                Some(format!(
                    "CAPTCHA: solve_captcha (request cap_test) is solving a recaptcha_v2 CAPTCHA on this page — {label}, 0s elapsed. Do not act on this page until it returns."
                ))
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_simultaneous_reservations_have_one_winner() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let barrier = std::sync::Barrier::new(8);
        let winners = std::thread::scope(|scope| -> anyhow::Result<usize> {
            let handles = (0..8)
                .map(|request| {
                    let reservations = &reservations;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        let lease = reservations.try_reserve(42, &format!("cap_{request}"));
                        barrier.wait();
                        lease.is_ok()
                    })
                })
                .collect::<Vec<_>>();
            handles.into_iter().try_fold(0, |count, handle| {
                handle
                    .join()
                    .map(|won| count + usize::from(won))
                    .map_err(|_| anyhow::anyhow!("reservation thread panicked"))
            })
        })?;
        assert_eq!(winners, 1);
        assert!(reservations.active(42).is_none());
        Ok(())
    }
}
