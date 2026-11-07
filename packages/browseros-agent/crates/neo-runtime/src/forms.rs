//! Typed page steps for form actions, separate from click's tab seam. Mutating steps own a
//! copy of the input turn and the resolved element until the page answers, even if the call
//! stops waiting. Fill/select also own their delivery ticket: an ambiguous page failure can
//! never make a later attempt repeat input. No keyboard policy or writer lives here.

use std::future::Future;
use std::sync::Arc;

use neo_runtime_spec::injected::{
    ElementState, FormFillOutcome, FormFillOutcomeStatus, FormSelectOutcome,
    FormSelectOutcomeError, InjectedElementStateParams, InjectedElementStateParamsState,
    InjectedFillParams, InjectedFillResult, InjectedFocusElementParams, InjectedInputValueParams,
    InjectedSelectOptionsParams, InjectedSelectOptionsResult,
};
use tokio::sync::oneshot;

pub(crate) use neo_runtime_spec::injected::{
    FormFocusOutcome as FocusOutcome, FormInputValueMatch as InputValueMatch,
    FormSelectOption as SelectOption, InjectedElementStateResult as CheckedState,
};

use crate::click::{PageError, Tab};
use crate::page::{FrameTab, InputPermit, PageSession, ResolvedElement};
use crate::progress::InputTicket;

/// The result of the one fill evaluation. `NeedsInput` retains the original ticket for the
/// insertText/Delete writer. A discarded handoff proves that no keyboard input was sent.
#[derive(Debug)]
pub(crate) enum FillOutcome {
    Done,
    NeedsInput(InputTicket),
    NotConnected,
    Missing(ElementState),
}

/// The result of selecting options. Every refusal proves no option was changed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SelectOutcome {
    Done(Vec<String>),
    NotConnected,
    OptionsNotFound,
    OptionNotEnabled,
    Missing(ElementState),
}

/// One method per forms page evaluation. Returned futures are cancel-safe, as in `click::Tab`.
/// Mutating methods start a detached page task eagerly, retaining input and element ownership
/// until the answer. Callers check Progress before handing off a ticket, as click's writer does.
pub(crate) trait FormTab: Tab {
    fn fill(
        &self,
        element: &Self::Element,
        value: &str,
        force: bool,
        input: Self::Input,
        ticket: InputTicket,
    ) -> impl Future<Output = Result<FillOutcome, PageError>> + Send;

    fn select_options(
        &self,
        element: &Self::Element,
        options: &[SelectOption],
        force: bool,
        input: Self::Input,
        ticket: InputTicket,
    ) -> impl Future<Output = Result<SelectOutcome, PageError>> + Send;

    fn checked_state(
        &self,
        element: &Self::Element,
    ) -> impl Future<Output = Result<CheckedState, PageError>> + Send;

    fn focus(
        &self,
        element: &Self::Element,
        reset_selection_if_not_focused: bool,
        input: Self::Input,
    ) -> impl Future<Output = Result<FocusOutcome, PageError>> + Send;

    fn input_value(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<InputValueMatch>, PageError>> + Send;
}

impl FormTab for FrameTab {
    fn fill(
        &self,
        element: &ResolvedElement,
        value: &str,
        force: bool,
        input: InputPermit,
        ticket: InputTicket,
    ) -> impl Future<Output = Result<FillOutcome, PageError>> + Send {
        let page = Arc::clone(self);
        let element = element.clone();
        let value = value.to_string();
        let (sender, receiver) = oneshot::channel();
        self.spawn(async move {
            let _input = input;
            let result = page
                .world
                .call(
                    None,
                    &InjectedFillParams {
                        element: element.handle().clone(),
                        value,
                        force,
                    },
                )
                .await;
            // Keep the group (not just the objectId) until the page has answered.
            drop(element);
            send_fill_result(sender, finish_fill(result, ticket));
        });
        async move { receiver.await.unwrap_or(Err(PageError::Closed)) }
    }

    fn select_options(
        &self,
        element: &ResolvedElement,
        options: &[SelectOption],
        force: bool,
        input: InputPermit,
        ticket: InputTicket,
    ) -> impl Future<Output = Result<SelectOutcome, PageError>> + Send {
        let page = Arc::clone(self);
        let element = element.clone();
        let options = options.to_vec();
        page_step(self, async move {
            let _input = input;
            let result = page
                .world
                .call(
                    None,
                    &InjectedSelectOptionsParams {
                        element: element.handle().clone(),
                        options,
                        force,
                    },
                )
                .await;
            drop(element);
            finish_select(result, ticket)
        })
    }

    async fn checked_state(&self, element: &ResolvedElement) -> Result<CheckedState, PageError> {
        self.world
            .call(
                None,
                &InjectedElementStateParams {
                    element: element.handle().clone(),
                    state: InjectedElementStateParamsState::Checked,
                },
            )
            .await
    }

    fn focus(
        &self,
        element: &ResolvedElement,
        reset_selection_if_not_focused: bool,
        input: InputPermit,
    ) -> impl Future<Output = Result<FocusOutcome, PageError>> + Send {
        let page = Arc::clone(self);
        let element = element.clone();
        page_step(self, async move {
            let _input = input;
            let result = page
                .world
                .call(
                    None,
                    &InjectedFocusElementParams {
                        element: element.handle().clone(),
                        reset_selection_if_not_focused,
                    },
                )
                .await;
            drop(element);
            result.map(|result| result.outcome)
        })
    }

    async fn input_value(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<InputValueMatch>, PageError> {
        self.world
            .call(
                None,
                &InjectedInputValueParams {
                    selector: selector.to_string(),
                    strict,
                },
            )
            .await
            .map(|result| result.matched)
    }
}

/// Start before returning the future: a begun ticket must always reach its page task. The
/// task marks delivery even if its receiver is dropped, and owns the turn until the answer.
fn page_step<R: Send + 'static>(
    page: &Arc<PageSession>,
    work: impl Future<Output = Result<R, PageError>> + Send + 'static,
) -> impl Future<Output = Result<R, PageError>> + Send {
    let (sender, receiver) = oneshot::channel();
    page.spawn(async move {
        let _waiting = sender.send(work.await);
    });
    async move { receiver.await.unwrap_or(Err(PageError::Closed)) }
}

fn send_fill_result(
    sender: oneshot::Sender<Result<FillOutcome, PageError>>,
    result: Result<FillOutcome, PageError>,
) {
    if let Err(Ok(FillOutcome::NeedsInput(ticket))) = sender.send(result) {
        // The caller stopped before taking the ticket, so no writer could have received it.
        ticket.unsent();
    }
}

fn finish_fill(
    result: Result<InjectedFillResult, PageError>,
    ticket: InputTicket,
) -> Result<FillOutcome, PageError> {
    match result {
        Ok(InjectedFillResult {
            outcome: FormFillOutcome::Status(FormFillOutcomeStatus::Done),
        }) => {
            ticket.acked().delivered();
            Ok(FillOutcome::Done)
        }
        Ok(InjectedFillResult {
            outcome: FormFillOutcome::Status(FormFillOutcomeStatus::Needsinput),
        }) => Ok(FillOutcome::NeedsInput(ticket)),
        Ok(InjectedFillResult {
            outcome: FormFillOutcome::Status(FormFillOutcomeStatus::ErrorNotconnected),
        }) => {
            ticket.unsent();
            Ok(FillOutcome::NotConnected)
        }
        Ok(InjectedFillResult {
            outcome: FormFillOutcome::Missing(missing),
        }) => {
            ticket.unsent();
            Ok(FillOutcome::Missing(missing.missing_state))
        }
        Err(error) => {
            // Playwright's validation exceptions precede mutation; transport and decode errors
            // prove nothing about delivery and leave the dropped ticket Unknown.
            if matches!(error, PageError::Threw(_)) {
                ticket.unsent();
            }
            Err(error)
        }
    }
}

fn finish_select(
    result: Result<InjectedSelectOptionsResult, PageError>,
    ticket: InputTicket,
) -> Result<SelectOutcome, PageError> {
    match result {
        Ok(InjectedSelectOptionsResult {
            values: Some(values),
            outcome: None,
        }) => {
            ticket.acked().delivered();
            Ok(SelectOutcome::Done(values))
        }
        Ok(InjectedSelectOptionsResult {
            values: None,
            outcome: Some(outcome),
        }) => {
            ticket.unsent();
            Ok(match outcome {
                FormSelectOutcome::Error(FormSelectOutcomeError::ErrorNotconnected) => {
                    SelectOutcome::NotConnected
                }
                FormSelectOutcome::Error(FormSelectOutcomeError::ErrorOptionsnotfound) => {
                    SelectOutcome::OptionsNotFound
                }
                FormSelectOutcome::Error(FormSelectOutcomeError::ErrorOptionnotenabled) => {
                    SelectOutcome::OptionNotEnabled
                }
                FormSelectOutcome::Missing(missing) => {
                    SelectOutcome::Missing(missing.missing_state)
                }
            })
        }
        Ok(_) => Err(PageError::Contract(
            "selectOptions must return either values or outcome".to_string(),
        )),
        Err(error) => {
            if matches!(error, PageError::Threw(_)) {
                ticket.unsent();
            }
            Err(error)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use neo_runtime_spec::injected::MissingState;
    use tokio::time;

    use super::*;
    use crate::click::tests::{FakeTab, Step};
    use crate::progress::DeliveryCell;
    use crate::run::Delivery;

    pub(crate) struct FormSteps {
        pub(crate) fill: Step<Result<InjectedFillResult, PageError>>,
        pub(crate) select: Step<Result<InjectedSelectOptionsResult, PageError>>,
        pub(crate) value: Step<Result<Option<InputValueMatch>, PageError>>,
        pub(crate) checked: Step<Result<CheckedState, PageError>>,
        pub(crate) focus: Step<Result<FocusOutcome, PageError>>,
        pub(crate) input_wait: Duration,
    }

    impl FormSteps {
        pub(crate) fn ready() -> Self {
            let trip = Duration::from_millis(1);
            Self {
                fill: Step::always(
                    trip,
                    Ok(InjectedFillResult {
                        outcome: FormFillOutcome::Status(FormFillOutcomeStatus::Done),
                    }),
                ),
                select: Step::always(
                    trip,
                    Ok(InjectedSelectOptionsResult {
                        values: Some(vec!["one".into()]),
                        outcome: None,
                    }),
                ),
                value: Step::always(
                    trip,
                    Ok(Some(InputValueMatch {
                        value: "one".into(),
                        preview: "<input/>".into(),
                    })),
                ),
                checked: Step::always(
                    trip,
                    Ok(CheckedState {
                        matches: false,
                        received: Some("unchecked".into()),
                        is_radio: Some(false),
                    }),
                ),
                focus: Step::always(trip, Ok(FocusOutcome::Done)),
                input_wait: Duration::ZERO,
            }
        }
    }

    impl FormTab for FakeTab {
        fn fill(
            &self,
            _element: &u32,
            value: &str,
            force: bool,
            input: Self::Input,
            ticket: InputTicket,
        ) -> impl Future<Output = Result<FillOutcome, PageError>> + Send {
            self.record("fill");
            self.record(format!("fill {value:?} force={force}"));
            let result = self.forms.fill.next();
            let duration = self.forms.fill.duration;
            let (sender, receiver) = oneshot::channel();
            tokio::spawn(async move {
                time::sleep(duration).await;
                let result = finish_fill(result, ticket);
                drop(input);
                send_fill_result(sender, result);
            });
            async move { receiver.await.expect("fake page task") }
        }
        fn select_options(
            &self,
            _element: &u32,
            options: &[SelectOption],
            force: bool,
            input: Self::Input,
            ticket: InputTicket,
        ) -> impl Future<Output = Result<SelectOutcome, PageError>> + Send {
            self.record("select_options");
            self.record(format!("select {options:?} force={force}"));
            let result = self.forms.select.next();
            let duration = self.forms.select.duration;
            let (sender, receiver) = oneshot::channel();
            tokio::spawn(async move {
                time::sleep(duration).await;
                let result = finish_select(result, ticket);
                drop(input);
                let _waiting = sender.send(result);
            });
            async move { receiver.await.expect("fake page task") }
        }
        async fn checked_state(&self, _element: &u32) -> Result<CheckedState, PageError> {
            self.record("checked_state");
            self.forms.checked.answer().await
        }
        async fn focus(
            &self,
            _element: &u32,
            reset: bool,
            input: Self::Input,
        ) -> Result<FocusOutcome, PageError> {
            self.record(format!("focus {reset}"));
            let result = self.forms.focus.answer().await;
            drop(input);
            result
        }
        async fn input_value(
            &self,
            _selector: &str,
            _strict: bool,
        ) -> Result<Option<InputValueMatch>, PageError> {
            self.record("input_value");
            self.forms.value.answer().await
        }
    }

    #[test]
    fn tests_that_fill_records_done_noop_and_pending_keyboard_delivery() {
        for (value, delivery) in [
            (
                FormFillOutcome::Status(FormFillOutcomeStatus::Done),
                Delivery::Done,
            ),
            (
                FormFillOutcome::Status(FormFillOutcomeStatus::ErrorNotconnected),
                Delivery::NotSent,
            ),
            (
                FormFillOutcome::Missing(Box::new(MissingState {
                    missing_state: ElementState::Editable,
                })),
                Delivery::NotSent,
            ),
            (
                FormFillOutcome::Status(FormFillOutcomeStatus::Needsinput),
                Delivery::Unknown,
            ),
        ] {
            let cell = Arc::new(DeliveryCell::default());
            let result = finish_fill(
                Ok(InjectedFillResult { outcome: value }),
                cell.begin().expect("fresh"),
            );
            assert!(result.is_ok());
            assert_eq!(cell.delivery(), delivery);
            if let Ok(FillOutcome::NeedsInput(ticket)) = result {
                ticket.acked().delivered();
                assert_eq!(cell.delivery(), Delivery::Done);
            }
        }
    }

    #[test]
    fn tests_that_select_records_empty_success_and_each_noop() {
        let cell = Arc::new(DeliveryCell::default());
        assert_eq!(
            finish_select(
                Ok(InjectedSelectOptionsResult {
                    values: Some(vec![]),
                    outcome: None
                }),
                cell.begin().expect("fresh")
            ),
            Ok(SelectOutcome::Done(vec![]))
        );
        assert_eq!(cell.delivery(), Delivery::Done);
        for error in [
            FormSelectOutcomeError::ErrorNotconnected,
            FormSelectOutcomeError::ErrorOptionsnotfound,
            FormSelectOutcomeError::ErrorOptionnotenabled,
        ] {
            let cell = Arc::new(DeliveryCell::default());
            let result = finish_select(
                Ok(InjectedSelectOptionsResult {
                    values: None,
                    outcome: Some(FormSelectOutcome::Error(error)),
                }),
                cell.begin().expect("fresh"),
            );
            assert!(result.is_ok());
            assert_eq!(cell.delivery(), Delivery::NotSent);
        }
    }

    #[test]
    fn tests_that_ambiguous_page_failure_never_permits_another_write() {
        for error in [
            PageError::NotConnected,
            PageError::Closed,
            PageError::Contract("bad result".into()),
            PageError::Threw("bad input".into()),
        ] {
            let cell = Arc::new(DeliveryCell::default());
            assert!(finish_fill(Err(error.clone()), cell.begin().expect("fresh")).is_err());
            assert_eq!(
                cell.delivery(),
                if matches!(error, PageError::Threw(_)) {
                    Delivery::NotSent
                } else {
                    Delivery::Unknown
                }
            );
        }
        let cell = Arc::new(DeliveryCell::default());
        assert!(
            finish_select(
                Ok(InjectedSelectOptionsResult {
                    values: None,
                    outcome: None
                }),
                cell.begin().expect("fresh")
            )
            .is_err()
        );
        assert_eq!(cell.delivery(), Delivery::Unknown);
    }
}
