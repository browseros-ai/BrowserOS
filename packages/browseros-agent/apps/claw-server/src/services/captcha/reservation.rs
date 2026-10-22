use std::{sync::Arc, time::Instant};

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
pub struct TabReservations;

impl TabReservations {
    pub fn try_reserve(
        self: &Arc<Self>,
        page: u32,
        request_id: &str,
    ) -> Result<SolveLease, ActiveSolve> {
        Ok(SolveLease {
            page,
            request_id: request_id.to_owned(),
        })
    }
    pub fn active(&self, _page: u32) -> Option<ActiveSolve> {
        None
    }
    pub fn busy_message(&self, _page: u32) -> Option<String> {
        None
    }
    pub fn status_line(&self, _page: u32) -> Option<String> {
        None
    }
}

pub struct SolveLease {
    page: u32,
    request_id: String,
}

impl SolveLease {
    pub fn set_progress(&self, _kind: Option<CaptchaKind>, _step: SolveStep) {}
    pub fn page(&self) -> u32 {
        self.page
    }
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}
