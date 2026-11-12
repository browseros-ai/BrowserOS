//! Payment card operations over BrowserOS's `Payments` CDP domain. Admission,
//! document identity, cancellation and deadlines live in `native_operation`;
//! this adapter only shapes requests and names its failures.
use super::native_operation::NativeDomain;
use super::*;
use crate::payment_cards::{
    PaymentCardAddOutcome, PaymentCardFailure, PaymentCardFillOutcome, PaymentCardInventory,
    PaymentCardSource, SuppliedCard,
};

const PAYMENTS: NativeDomain = NativeDomain {
    feature: NativeFeature::PaymentCards,
    name: "Payments",
    error_prefix: "payments",
};

impl BrowserView {
    pub async fn list_payment_cards(
        &self,
        page: Option<PageId>,
    ) -> Result<PaymentCardInventory, PaymentCardFailure> {
        self.native_request(&PAYMENTS, page, "listCards", json!({}), false)
            .await
            .map_err(|code| PaymentCardFailure::new(&code))
    }
    pub async fn add_payment_card(
        &self,
        page: Option<PageId>,
        card: SuppliedCard,
    ) -> Result<PaymentCardAddOutcome, PaymentCardFailure> {
        self.native_request(&PAYMENTS, page, "addCard", json!({"card": card}), true)
            .await
            .map_err(|code| PaymentCardFailure::new(&code))
    }
    pub async fn fill_payment_card(
        &self,
        page: PageId,
        source: PaymentCardSource,
    ) -> Result<PaymentCardFillOutcome, PaymentCardFailure> {
        let params = match source {
            PaymentCardSource::Saved { card_id } => json!({"cardId": card_id}),
            PaymentCardSource::Supplied(card) => json!({"card": card}),
        };
        self.native_request(&PAYMENTS, Some(page), "fillCard", params, false)
            .await
            .map_err(|code| PaymentCardFailure::new(&code))
    }
}
