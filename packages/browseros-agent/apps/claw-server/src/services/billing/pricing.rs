//! The plans on sale, cached in memory. A copy past its freshness is still served when the
//! billing gateway fails, so the price panel survives a short outage.

use super::client::Pricing;
use std::time::{Duration, Instant};

/// Short, because the gateway caches the catalogue itself and stops offering a free trial as soon
/// as trials are paused; a long copy here would keep promising it.
pub const FRESH_FOR: Duration = Duration::from_secs(60);

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

    /// Whether the cached copy offers a free trial on this price, however old the copy is.
    #[must_use]
    pub fn offers_trial(&self, plan: &str, interval: &str, interval_count: i32) -> bool {
        self.any().is_some_and(|pricing| {
            pricing
                .plans
                .iter()
                .filter(|priced| priced.plan == plan)
                .flat_map(|priced| &priced.prices)
                .any(|price| {
                    price.interval == interval
                        && price.interval_count == interval_count
                        && price.trial_days.unwrap_or(0) > 0
                })
        })
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
    use super::super::client::{Price, PricedPlan};
    use super::*;

    #[test]
    fn a_copy_goes_stale_but_stays_as_a_fallback_until_dropped() {
        let start = Instant::now();
        let mut cache = PricingCache::default();
        assert!(cache.fresh(start).is_none());
        cache.store(Pricing { plans: Vec::new() }, start);
        assert!(cache.fresh(start + Duration::from_secs(30)).is_some());
        let later = start + FRESH_FOR + Duration::from_secs(1);
        assert!(cache.fresh(later).is_none());
        assert!(cache.any().is_some());
        cache.drop_copy();
        assert!(cache.any().is_none());
    }

    fn price(interval: &str, trial_days: Option<i32>) -> Price {
        Price {
            interval: interval.into(),
            interval_count: 1,
            amount: 2000,
            currency: "usd".into(),
            trial_days,
        }
    }

    #[test]
    fn only_a_price_with_trial_days_offers_a_trial() {
        let mut cache = PricingCache::default();
        assert!(!cache.offers_trial("ultrafast", "month", 1));
        cache.store(
            Pricing {
                plans: vec![PricedPlan {
                    plan: "ultrafast".into(),
                    name: "ultrafast".into(),
                    description: None,
                    features: Vec::new(),
                    prices: vec![price("month", Some(7)), price("year", Some(0))],
                }],
            },
            Instant::now(),
        );
        assert!(cache.offers_trial("ultrafast", "month", 1));
        assert!(!cache.offers_trial("ultrafast", "year", 1));
        assert!(!cache.offers_trial("ultrafast", "month", 3));
        assert!(!cache.offers_trial("gold", "month", 1));
    }
}
