//! The plans on sale, cached in memory. A copy past its freshness is still served when the
//! billing gateway fails, so the price panel survives a short outage.

use super::client::Pricing;
use std::time::{Duration, Instant};

pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Default)]
pub struct PricingCache {
    entry: Option<(Instant, Pricing)>,
}

impl PricingCache {
    /// The cached copy if it was fetched within [`FRESH_FOR`].
    #[must_use]
    pub fn fresh(&self, now: Instant) -> Option<&Pricing> {
        self.entry
            .as_ref()
            .filter(|(fetched, _)| now.saturating_duration_since(*fetched) < FRESH_FOR)
            .map(|(_, pricing)| pricing)
    }

    /// The cached copy however old, for when the gateway fails.
    #[must_use]
    pub fn any(&self) -> Option<&Pricing> {
        self.entry.as_ref().map(|(_, pricing)| pricing)
    }

    pub fn store(&mut self, pricing: Pricing, now: Instant) {
        self.entry = Some((now, pricing));
    }

    /// Forgets the copy entirely, for when the gateway says a price on it is gone: serving it
    /// again as a stale fallback would only offer that price once more.
    pub fn drop_copy(&mut self) {
        self.entry = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_goes_stale_but_stays_as_a_fallback_until_dropped() {
        let start = Instant::now();
        let mut cache = PricingCache::default();
        assert!(cache.fresh(start).is_none());
        cache.store(Pricing { plans: Vec::new() }, start);
        assert!(cache.fresh(start + Duration::from_secs(60)).is_some());
        let later = start + FRESH_FOR + Duration::from_secs(1);
        assert!(cache.fresh(later).is_none());
        assert!(cache.any().is_some());
        cache.drop_copy();
        assert!(cache.any().is_none());
    }
}
