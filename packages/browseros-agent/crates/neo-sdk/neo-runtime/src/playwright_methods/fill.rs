//! Playwright's fill/selectOption loop B over the shared selector loop A, and inputValue's
//! read-only selector polling. Each mutating page call owns an input turn and delivery ticket;
//! only a page-proven no-op permits another attempt.

use std::sync::{Arc, OnceLock};

use neo_runtime_spec::injected::ElementState;
use neo_runtime_spec::serve::{FrameFillParams, FrameInputValueParams, FrameSelectOptionParams};

use crate::chromium_bridge::keyboard_input::{self, KeyboardTab, Platform};
use crate::chromium_bridge::mouse_and_touch_input::{Devices, Plan};
use crate::playwright_methods::evaluate::HandleEntry;
use crate::playwright_methods::forms::{FillOutcome, FormTab, SelectOption, SelectOutcome};
use crate::playwright_methods::retry::{
    self, ActionResult, ElementAction, LoopAWaits, PreChecks, RetryAction,
};
use crate::playwright_methods::tab::{NotReady, PageError, PressReport, Stop};
use crate::progress::{CallError, DeliveryCell, InputTicket, PlaywrightError, Progress};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FillOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) value: String,
    pub(crate) force: bool,
}

impl FillOptions {
    pub(crate) fn from_params(params: FrameFillParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            value: params.value,
            force: params.force.unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    /// Playwright's `elements`: the guids of the `ElementHandle` values, matched by node
    /// identity before `options` (`dom.ts` `_selectOption`).
    pub(crate) elements: Vec<String>,
    pub(crate) options: Vec<SelectOption>,
    pub(crate) force: bool,
}

impl SelectOptions {
    /// The generated params never carry `elements`: `method_router.rs` takes them out of the wire
    /// first (their channel type has no values) and hands their guids to
    /// [`SelectOptions::with_elements`].
    pub(crate) fn from_params(params: FrameSelectOptionParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            elements: Vec::new(),
            options: params
                .options
                .unwrap_or_default()
                .into_iter()
                .map(|option| SelectOption {
                    value_or_label: option.value_or_label,
                    value: option.value,
                    label: option.label,
                    index: option.index,
                })
                .collect(),
            force: params.force.unwrap_or(false),
        }
    }

    /// These options with `ElementHandle` values, by guid.
    pub(crate) fn with_elements(self, elements: Vec<String>) -> Self {
        Self { elements, ..self }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputValueOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
}

impl InputValueOptions {
    pub(crate) fn from_params(params: FrameInputValueParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }
}

pub(crate) async fn fill<T: FormTab + KeyboardTab>(
    tab: &T,
    options: &FillOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    let delivery = delivery(progress)?;
    retry::retry_with_progress_if_not_connected(
        tab,
        &options.selector,
        options.strict,
        PreChecks::unless_forced(options.force),
        progress,
        &FillAction {
            options,
            delivery,
            platform: OnceLock::new(),
        },
    )
    .await
}

pub(crate) async fn select_option<T: FormTab>(
    tab: &T,
    options: &SelectOptions,
    progress: &Progress,
) -> Result<Vec<String>, CallError> {
    let delivery = delivery(progress)?;
    // Checked before any work, as Playwright's dispatcher checks `elements` channels.
    let elements = options
        .elements
        .iter()
        .enumerate()
        .map(|(index, guid)| progress.handles().select_element(index, guid))
        .collect::<Result<Vec<_>, _>>()?;
    retry::retry_with_progress_if_not_connected(
        tab,
        &options.selector,
        options.strict,
        PreChecks::unless_forced(options.force),
        progress,
        &SelectAction {
            options,
            elements: &elements,
            delivery,
        },
    )
    .await
}

/// `_waitForFunctionOnSelector`: no action pre-check, resolved handle or input turn. The page
/// marks only the first match, follows labels for the value, and previews the original node.
pub(crate) async fn input_value<T: FormTab>(
    tab: &T,
    options: &InputValueOptions,
    progress: &Progress,
) -> Result<String, CallError> {
    progress.name_locator(&options.selector);
    let mut named = false;
    let mut waits = LoopAWaits::new(progress.timeout_ms());
    loop {
        let wait = waits.next_wait();
        if !wait.is_zero() {
            progress.wait(wait).await?;
        }
        let iteration: Result<Option<String>, Stop> = async {
            progress.race(tab.prepare()).await??;
            if !named {
                let facts = progress.race(tab.describe(&options.selector)).await??;
                progress.name_locator(&facts.locator);
                named = true;
                if let Some(message) = facts.invalid {
                    return Err(PlaywrightError::error(message).into());
                }
            }
            let matched = progress
                .race(tab.input_value(&options.selector, options.strict))
                .await??;
            Ok(matched.map(|matched| {
                progress.log(format!("  locator resolved to {}", matched.preview));
                matched.value
            }))
        }
        .await;
        match iteration {
            Ok(Some(value)) => return Ok(value),
            Ok(None) | Err(Stop::Retry) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    }
}

struct FillAction<'a> {
    options: &'a FillOptions,
    delivery: &'a Arc<DeliveryCell>,
    platform: OnceLock<Platform>,
}

impl<T: FormTab + KeyboardTab> ElementAction<T> for FillAction<'_> {
    type Output = ();

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<()>, Stop> {
        let options = self.options;
        progress.log(format!("  fill(\"{}\")", options.value));
        let mut retry = RetryAction::new("fill");
        loop {
            retry
                .before(tab, PreChecks::unless_forced(options.force), progress)
                .await?;
            if !options.force {
                progress.log("  waiting for element to be visible, enabled and editable");
            }
            // An empty value is a press of Delete, planned under the turn from the tab's keyboard.
            let delete = if options.value.is_empty() {
                let platform = if let Some(platform) = self.platform.get() {
                    *platform
                } else {
                    let platform = progress.race(tab.platform()).await??;
                    *self.platform.get_or_init(|| platform)
                };
                Some(platform)
            } else {
                None
            };
            // Take the turn after every retry wait, before the page can focus/select/set a value.
            let input = progress.race(tab.input()).await?;
            progress.check()?;
            let ticket = self.delivery.begin().map_err(|_| input_already_sent())?;
            let outcome = progress
                .race(tab.fill(
                    element,
                    &options.value,
                    options.force,
                    input.clone(),
                    ticket,
                ))
                .await?
                .map_err(mutation_error)?;
            match outcome {
                FillOutcome::Done => return Ok(ActionResult::Done(())),
                FillOutcome::NotConnected => return Ok(ActionResult::NotConnected),
                FillOutcome::Missing(state) => log_missing(progress, state),
                FillOutcome::NeedsInput(ticket) => {
                    write_fill_input(tab, &options.value, delete, input, ticket, progress).await?;
                    return Ok(ActionResult::Done(()));
                }
            }
        }
    }
}

/// The page has focused and selected the target under this turn. The original ticket moves
/// into the detached writer, which retains delivery truth even if this call stops waiting.
/// `delete` (the browser's platform) presses Delete for an empty value; any other value is one
/// `Input.insertText`. Both are planned from the tab's devices under the turn.
async fn write_fill_input<T: KeyboardTab>(
    tab: &T,
    value: &str,
    delete: Option<Platform>,
    input: T::Input,
    ticket: InputTicket,
    progress: &Progress,
) -> Result<(), Stop> {
    let planned = progress
        .check()
        .map_err(Stop::from)
        .and_then(|()| plan_fill_input(value, delete, &input).map_err(Stop::from));
    let plan = match planned {
        Ok(plan) => plan,
        Err(error) => {
            // needsinput proves the page did not fill the value. Until a writer takes the
            // ticket, failed preparation or cancellation proves no input was sent.
            ticket.unsent();
            return Err(error);
        }
    };
    let report = progress.race(tab.write(plan, ticket, input)).await?;
    match report {
        PressReport::Acked(acked) => acked.delivered(),
        PressReport::NothingWritten | PressReport::Failed { closed: true, .. } => {
            return Err(PlaywrightError::target_closed().into());
        }
        PressReport::Failed {
            closed: false,
            message,
        } => {
            return Err(PlaywrightError::error(message).into());
        }
    }
    Ok(())
}

/// Fill's typing as a plan: Playwright's `keyboard.press('Delete')` for an empty value, else
/// `keyboard.insertText(value)`.
fn plan_fill_input<D: Devices>(
    value: &str,
    delete: Option<Platform>,
    input: &D,
) -> Result<Plan, PageError> {
    let mut simulated = input.snapshot()?;
    let mut plan = Plan::default();
    match delete {
        Some(platform) => {
            keyboard_input::plan_press(
                &mut plan,
                &mut simulated,
                "Delete",
                platform,
                &input.owner(),
                None,
            )
            .map_err(|unknown| PageError::Contract(unknown.to_string()))?;
        }
        None => {
            keyboard_input::plan_insert_text(&mut plan, &mut simulated, value);
        }
    }
    Ok(plan)
}

struct SelectAction<'a> {
    options: &'a SelectOptions,
    elements: &'a [Arc<HandleEntry>],
    delivery: &'a Arc<DeliveryCell>,
}

impl<T: FormTab> ElementAction<T> for SelectAction<'_> {
    type Output = Vec<String>;

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<Vec<String>>, Stop> {
        let options = self.options;
        let mut retry = RetryAction::new("select option");
        loop {
            retry
                .before(tab, PreChecks::unless_forced(options.force), progress)
                .await?;
            if !options.force {
                progress.log("  waiting for element to be visible and enabled");
            }
            let input = progress.race(tab.input()).await?;
            progress.check()?;
            let ticket = self.delivery.begin().map_err(|_| input_already_sent())?;
            let outcome = progress
                .race(tab.select_options(
                    element,
                    &options.options,
                    self.elements,
                    options.force,
                    input,
                    ticket,
                ))
                .await?
                .map_err(mutation_error)?;
            match outcome {
                SelectOutcome::Done(values) => {
                    progress.log("  selected specified option(s)");
                    return Ok(ActionResult::Done(values));
                }
                SelectOutcome::NotConnected => return Ok(ActionResult::NotConnected),
                SelectOutcome::Missing(state) => log_missing(progress, state),
                SelectOutcome::OptionsNotFound => progress.log("  did not find some options"),
                SelectOutcome::OptionNotEnabled => {
                    progress.log("  option being selected is not enabled");
                }
                SelectOutcome::Unadoptable => {
                    // Playwright: `server/dom.ts` `kUnableToAdoptErrorMessage`: `Unable to adopt element handle from a different document`
                    return Err(PlaywrightError::error(
                        "Unable to adopt element handle from a different document",
                    )
                    .into());
                }
            }
        }
    }
}

fn delivery(progress: &Progress) -> Result<&Arc<DeliveryCell>, CallError> {
    progress.delivery().ok_or_else(|| {
        PlaywrightError::error("neo-runtime: a form input call has no delivery cell").into()
    })
}

fn input_already_sent() -> Stop {
    PlaywrightError::error("neo-runtime: a form tried to write again after input").into()
}

// A transport failure after the mutating page call cannot prove it did nothing. Unlike a
// read's PageError::NotConnected, it must end the call, with the ticket left Unknown.
fn mutation_error(error: PageError) -> Stop {
    match error {
        PageError::NotConnected => PlaywrightError::error(
            "neo-runtime: form page call failed after input may have been delivered",
        )
        .into(),
        other => other.into(),
    }
}

fn log_missing(progress: &Progress, state: ElementState) {
    if let Some(line) = retry::not_ready_line(&NotReady::Missing(state)) {
        progress.log(line);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use neo_runtime_spec::injected::{
        ElementState, FormFillOutcome, FormFillOutcomeStatus, FormSelectOutcome,
        FormSelectOutcomeError, InjectedFillResult, InjectedSelectOptionsResult, MissingState,
    };
    use tokio::time;
    use tokio_util::sync::CancellationToken;
    use tokio_util::task::TaskTracker;

    use super::*;
    use crate::deadline::Deadline;
    use crate::playwright_methods::forms::InputValueMatch;
    use crate::playwright_methods::tab::Tab;
    use crate::playwright_methods::tab::fake::{FakeTab, Step, Writes};
    use crate::progress::{CallEntry, CallStart, ErrorName, RunScope, Trace};
    use crate::run::{Delivery, RunEnd};

    const TRIP: Duration = Duration::from_millis(1);

    fn fill_options() -> FillOptions {
        FillOptions {
            selector: "input".into(),
            strict: true,
            value: "one".into(),
            force: false,
        }
    }

    fn needs_input(tab: &mut FakeTab) {
        tab.forms.fill = Step::always(
            TRIP,
            Ok(fill_reply(FormFillOutcome::Status(
                FormFillOutcomeStatus::Needsinput,
            ))),
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_stopping_fill_during_platform_lookup_records_no_input() {
        let mut tab = FakeTab::ready();
        needs_input(&mut tab);
        tab.platform.duration = Duration::from_secs(10);
        let mut options = fill_options();
        options.value.clear();
        let progress = Progress::for_test(100.0);
        is_error(
            &fill(&tab, &options, &progress).await,
            ErrorName::TimeoutError,
        );
        assert!(tab.count("platform") > 0);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
        assert!(
            !tab.calls()
                .iter()
                .any(|call| call.starts_with("insertText ") || call.contains("Delete"))
        );
        assert_eq!(tab.count("input"), tab.count("input released"));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_stopping_fill_at_the_writer_check_records_no_input() {
        let tab = FakeTab::ready();
        let (progress, run) = scoped_progress(true, 1000.0, Duration::from_secs(30));
        let input = tab.input().await;
        // This is the ticket returned by a successful needsinput page step, before typing.
        let ticket = progress.delivery().expect("input").begin().expect("fresh");
        run.end(RunEnd::Cancelled);
        let result = write_fill_input(&tab, "one", None, input, ticket, &progress).await;
        assert!(matches!(
            result,
            Err(Stop::End(CallError::RunEnded(RunEnd::Cancelled)))
        ));
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
        assert_eq!(tab.count("insertText one"), 0);
        assert_eq!(tab.count("input"), tab.count("input released"));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_fill_hands_text_to_the_keyboard_writer_and_delivers_once() {
        let mut tab = FakeTab::ready();
        needs_input(&mut tab);
        let progress = Progress::for_test(1000.0);
        assert_eq!(fill(&tab, &fill_options(), &progress).await, Ok(()));
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
        assert_eq!(tab.count("insertText one"), 1, "one text insertion");
        assert_eq!(tab.count("platform"), 0, "insertText needs no key layout");
        assert_eq!(tab.count("input"), tab.count("input released"));
        let calls = tab.calls();
        assert!(
            calls
                .iter()
                .position(|call| call == "insertText one")
                .expect("insertText")
                < calls
                    .iter()
                    .position(|call| call == "input released")
                    .expect("released")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_delete_reads_the_platform_once_before_any_input_turn() {
        let mut tab = FakeTab::ready();
        tab.forms.fill = Step::new(
            TRIP,
            [
                Ok(fill_reply(missing(ElementState::Editable))),
                Ok(fill_reply(FormFillOutcome::Status(
                    FormFillOutcomeStatus::Needsinput,
                ))),
            ],
        );
        let mut options = fill_options();
        options.value.clear();
        assert_eq!(
            fill(&tab, &options, &Progress::for_test(1000.0)).await,
            Ok(())
        );
        let calls = tab.calls();
        assert_eq!(
            tab.count("platform"),
            1,
            "platform is cached across retries"
        );
        assert!(
            calls
                .iter()
                .position(|call| call == "platform")
                .expect("platform")
                < calls
                    .iter()
                    .position(|call| call == "input")
                    .expect("input")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_empty_fill_plans_delete_and_holds_the_turn_through_keyup() {
        let mut tab = FakeTab::ready();
        needs_input(&mut tab);
        let mut options = fill_options();
        options.value.clear();
        let progress = Progress::for_test(1000.0);
        assert_eq!(fill(&tab, &options, &progress).await, Ok(()));
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
        assert!(tab.calls().iter().any(|call| call.contains("Delete")));
        assert!(tab.count("platform") > 0);
        let calls = tab.calls();
        let keyup = calls
            .iter()
            .position(|call| call == "keyUp Delete")
            .expect("Delete released");
        let released = calls
            .iter()
            .position(|call| call == "input released")
            .expect("turn released");
        assert!(keyup < released, "the turn spans key release");
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_failed_or_refused_text_write_ends_without_repeating_input() {
        for (writes, delivery, category) in [
            (
                Writes::Nothing,
                Delivery::NotSent,
                ErrorName::TargetClosedError,
            ),
            (Writes::Fail, Delivery::Unknown, ErrorName::Error),
        ] {
            let mut tab = FakeTab::ready();
            needs_input(&mut tab);
            tab.press = Step::always(TRIP, writes);
            let progress = Progress::for_test(1000.0);
            is_error(&fill(&tab, &fill_options(), &progress).await, category);
            assert_eq!(progress.delivery().expect("input").delivery(), delivery);
            assert_eq!(
                tab.count("insertText one"),
                1,
                "a failed write is never repeated"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_cancelled_insert_text_finishes_and_the_record_reads_the_writers_truth() {
        for (writes, delivery) in [
            (Writes::Ack, Delivery::Done),
            (Writes::Nothing, Delivery::NotSent),
            (Writes::Fail, Delivery::Unknown),
        ] {
            let mut tab = FakeTab::ready();
            needs_input(&mut tab);
            tab.press = Step::always(Duration::from_secs(2), writes);
            let run = RunScope::new(
                Deadline::new(Duration::from_secs(30), CancellationToken::new()),
                TaskTracker::new(),
            );
            let entry = Arc::new(CallEntry::new(CallStart {
                call_id: 1,
                guid: "frame@test".into(),
                method: "fill".into(),
                title: None,
                params: serde_json::Value::Null,
                location: None,
                input: true,
            }));
            let trace = Trace::default();
            trace.push(Arc::clone(&entry));
            let progress = Progress::new(Arc::clone(&entry), 100.0, run, CancellationToken::new());
            let result = fill(&tab, &fill_options(), &progress).await;
            is_error(&result, ErrorName::TimeoutError);
            let Err(CallError::Playwright(error)) = result else {
                panic!("call timeout")
            };
            entry.finish(error.to_result());
            trace.finished(&entry);
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::Unknown
            );
            assert_eq!(
                tab.count("insertText one"),
                1,
                "the insertion command was started"
            );
            assert_eq!(tab.count("input released"), 0, "writer still owns the turn");
            time::sleep(Duration::from_secs(3)).await;
            let (records, dropped) = trace.take(RunEnd::Settled);
            assert!(dropped.is_none());
            assert_eq!(records[0].delivery, Some(delivery));
            assert_eq!(
                tab.count("input released"),
                1,
                "the writer released its turn"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_fill_delivers_a_set_value_answer() {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(1000.0);
        assert_eq!(fill(&tab, &fill_options(), &progress).await, Ok(()));
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
        assert_eq!(tab.count("fill"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_select_returns_the_pages_values() {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(1000.0);
        let options = SelectOptions {
            elements: Vec::new(),
            selector: "select".into(),
            strict: true,
            options: vec![],
            force: false,
        };
        assert_eq!(
            select_option(&tab, &options, &progress).await,
            Ok(vec!["one".into()])
        );
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
        assert_eq!(tab.count("select_options"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_input_value_reads_without_taking_an_input_turn() {
        let tab = FakeTab::ready();
        let (progress, _) = scoped_progress(false, 1000.0, Duration::from_secs(30));
        let options = InputValueOptions {
            selector: "input".into(),
            strict: true,
        };
        assert_eq!(
            input_value(&tab, &options, &progress).await,
            Ok("one".into())
        );
        assert_eq!(tab.calls(), ["describe", "input_value"]);
        assert!(progress.delivery().is_none());
    }

    fn scoped_progress(input: bool, timeout: f64, run_timeout: Duration) -> (Progress, RunScope) {
        let run = RunScope::new(
            Deadline::new(run_timeout, CancellationToken::new()),
            TaskTracker::new(),
        );
        let entry = Arc::new(CallEntry::new(CallStart {
            call_id: 1,
            guid: "frame@test".into(),
            method: "forms".into(),
            title: None,
            params: serde_json::Value::Null,
            location: None,
            input,
        }));
        (
            Progress::new(entry, timeout, run.clone(), CancellationToken::new()),
            run,
        )
    }

    fn fill_reply(outcome: FormFillOutcome) -> InjectedFillResult {
        InjectedFillResult { outcome }
    }

    fn missing(state: ElementState) -> FormFillOutcome {
        FormFillOutcome::Missing(Box::new(MissingState {
            missing_state: state,
        }))
    }

    fn selected() -> InjectedSelectOptionsResult {
        InjectedSelectOptionsResult {
            values: Some(vec!["two".into(), "one".into()]),
            outcome: None,
        }
    }

    fn select_options() -> SelectOptions {
        SelectOptions {
            elements: Vec::new(),
            selector: "select".into(),
            strict: true,
            options: vec![],
            force: false,
        }
    }

    /// An `ElementHandle` value whose guid names no handle of the run (a disposed one) fails
    /// before any page work, with Playwright's dispatcher message for the channel.
    #[tokio::test(start_paused = true)]
    async fn tests_that_an_unknown_element_value_fails_before_any_page_work() {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(1000.0);
        let options = select_options().with_elements(vec!["handle@gone".into()]);
        assert_eq!(
            select_option(&tab, &options, &progress).await,
            Err(CallError::Playwright(PlaywrightError::error(
                "elements[0]: no object with guid handle@gone"
            )))
        );
        assert_eq!(tab.count("select_options"), 0);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
    }

    fn is_error(result: &Result<(), CallError>, name: ErrorName) {
        assert!(
            matches!(result, Err(CallError::Playwright(error)) if error.name == name),
            "{result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_fill_retries_proven_noops_and_releases_each_turn() {
        for outcome in [
            missing(ElementState::Visible),
            missing(ElementState::Enabled),
            missing(ElementState::Editable),
            missing(ElementState::NonInert),
            FormFillOutcome::Status(FormFillOutcomeStatus::ErrorNotconnected),
        ] {
            let mut tab = FakeTab::ready();
            tab.forms.fill = Step::new(
                TRIP,
                [
                    Ok(fill_reply(outcome)),
                    Ok(fill_reply(FormFillOutcome::Status(
                        FormFillOutcomeStatus::Done,
                    ))),
                ],
            );
            let progress = Progress::for_test(1000.0);
            assert_eq!(fill(&tab, &fill_options(), &progress).await, Ok(()));
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::Done
            );
            assert!(tab.count("fill") > 1);
            assert_eq!(tab.count("input"), tab.count("input released"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_disabled_and_readonly_fill_wait_then_timeout_without_delivery() {
        for state in [ElementState::Enabled, ElementState::Editable] {
            let mut tab = FakeTab::ready();
            tab.forms.fill = Step::always(TRIP, Ok(fill_reply(missing(state))));
            let progress = Progress::for_test(500.0);
            let result = fill(&tab, &fill_options(), &progress).await;
            is_error(&result, ErrorName::TimeoutError);
            assert!(tab.count("fill") > 1);
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::NotSent
            );
            assert_eq!(tab.count("input"), tab.count("input released"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_select_retries_each_proven_refusal_and_returns_selected_values() {
        for outcome in [
            FormSelectOutcome::Error(FormSelectOutcomeError::ErrorNotconnected),
            FormSelectOutcome::Error(FormSelectOutcomeError::ErrorOptionsnotfound),
            FormSelectOutcome::Error(FormSelectOutcomeError::ErrorOptionnotenabled),
            FormSelectOutcome::Missing(Box::new(MissingState {
                missing_state: ElementState::Enabled,
            })),
        ] {
            let mut tab = FakeTab::ready();
            tab.forms.select = Step::new(
                TRIP,
                [
                    Ok(InjectedSelectOptionsResult {
                        values: None,
                        outcome: Some(outcome),
                    }),
                    Ok(selected()),
                ],
            );
            let progress = Progress::for_test(1000.0);
            assert_eq!(
                select_option(&tab, &select_options(), &progress).await,
                Ok(vec!["two".into(), "one".into()])
            );
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::Done
            );
            assert!(tab.count("select_options") > 1);
            assert_eq!(tab.count("input"), tab.count("input released"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_missing_and_disabled_options_wait_without_changing_selection() {
        for error in [
            FormSelectOutcomeError::ErrorOptionsnotfound,
            FormSelectOutcomeError::ErrorOptionnotenabled,
        ] {
            let mut tab = FakeTab::ready();
            tab.forms.select = Step::always(
                TRIP,
                Ok(InjectedSelectOptionsResult {
                    values: None,
                    outcome: Some(FormSelectOutcome::Error(error)),
                }),
            );
            let progress = Progress::for_test(500.0);
            let result = select_option(&tab, &select_options(), &progress)
                .await
                .map(|_| ());
            is_error(&result, ErrorName::TimeoutError);
            assert!(tab.count("select_options") > 1);
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::NotSent
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_page_validation_errors_end_without_delivery_or_retry() {
        for message in [
            "Cannot type text into input[type=number]",
            "Input of type \"checkbox\" cannot be filled",
            "Element is not an <input>, <textarea> or [contenteditable] element",
        ] {
            let mut tab = FakeTab::ready();
            tab.forms.fill = Step::always(TRIP, Err(PageError::Threw(message.into())));
            let progress = Progress::for_test(1000.0);
            is_error(
                &fill(&tab, &fill_options(), &progress).await,
                ErrorName::Error,
            );
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::NotSent
            );
            assert_eq!(tab.count("fill"), 1, "validation does not retry");
        }
        let mut tab = FakeTab::ready();
        tab.forms.select = Step::always(
            TRIP,
            Err(PageError::Threw("Element is not a <select> element".into())),
        );
        let progress = Progress::for_test(1000.0);
        is_error(
            &select_option(&tab, &select_options(), &progress)
                .await
                .map(|_| ()),
            ErrorName::Error,
        );
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
        assert_eq!(tab.count("select_options"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_ambiguous_page_errors_end_without_repeating_input() {
        for error in [
            PageError::NotConnected,
            PageError::Contract("bad result".into()),
            PageError::Closed,
        ] {
            let mut tab = FakeTab::ready();
            tab.forms.fill = Step::always(TRIP, Err(error.clone()));
            let progress = Progress::for_test(1000.0);
            assert!(fill(&tab, &fill_options(), &progress).await.is_err());
            assert_eq!(tab.count("fill"), 1);
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::Unknown
            );
            tab.forms.select = Step::always(TRIP, Err(error));
            let progress = Progress::for_test(1000.0);
            assert!(
                select_option(&tab, &select_options(), &progress)
                    .await
                    .is_err()
            );
            assert_eq!(tab.count("select_options"), 1);
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::Unknown
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_cancelled_input_wait_holds_nothing_and_sends_nothing() {
        let mut tab = FakeTab::ready();
        tab.forms.input_wait = Duration::from_secs(10);
        for select in [false, true] {
            let progress = Progress::for_test(100.0);
            let result = if select {
                select_option(&tab, &select_options(), &progress)
                    .await
                    .map(|_| ())
            } else {
                fill(&tab, &fill_options(), &progress).await
            };
            is_error(&result, ErrorName::TimeoutError);
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::NotSent
            );
        }
        assert_eq!(tab.count("input"), 0);
        assert_eq!(tab.count("fill"), 0);
        assert_eq!(tab.count("select_options"), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_cancelled_page_steps_keep_the_turn_and_record_the_answer() {
        for (select, needs_keyboard) in [(false, false), (true, false), (false, true)] {
            let mut tab = FakeTab::ready();
            if needs_keyboard {
                needs_input(&mut tab);
            }
            tab.forms.fill.duration = Duration::from_secs(1);
            tab.forms.select.duration = Duration::from_secs(1);
            let progress = Progress::for_test(100.0);
            let result = if select {
                select_option(&tab, &select_options(), &progress)
                    .await
                    .map(|_| ())
            } else {
                fill(&tab, &fill_options(), &progress).await
            };
            is_error(&result, ErrorName::TimeoutError);
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::Unknown
            );
            assert_eq!(tab.count("input released"), 0);
            time::sleep(Duration::from_secs(2)).await;
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                if needs_keyboard {
                    Delivery::NotSent
                } else {
                    Delivery::Done
                }
            );
            assert_eq!(tab.count("input released"), 1);
            assert_eq!(tab.count("insertText one"), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_the_run_deadline_in_a_retry_wait_leaves_delivery_not_sent() {
        let mut tab = FakeTab::ready();
        tab.forms.fill = Step::always(TRIP, Ok(fill_reply(missing(ElementState::Editable))));
        let (progress, _) = scoped_progress(true, 1000.0, Duration::from_millis(50));
        assert_eq!(
            fill(&tab, &fill_options(), &progress).await,
            Err(CallError::RunEnded(RunEnd::Timeout))
        );
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
        assert_eq!(tab.count("input"), tab.count("input released"));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_force_skips_navigation_checks_and_reaches_the_page_with_force() {
        let tab = FakeTab::ready();
        *tab.pending.lock().expect("pending") = Some("pending".into());
        let mut options = fill_options();
        options.force = true;
        assert_eq!(
            fill(&tab, &options, &Progress::for_test(1000.0)).await,
            Ok(())
        );
        let mut options = select_options();
        options.force = true;
        assert!(
            select_option(&tab, &options, &Progress::for_test(1000.0))
                .await
                .is_ok()
        );
        assert_eq!(tab.count("navigation_settled"), 0);
        assert!(tab.calls().iter().any(|call| call.contains("force=true")));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_input_value_polls_missing_or_replaced_contexts_without_input() {
        let mut tab = FakeTab::ready();
        tab.forms.value = Step::new(
            TRIP,
            [
                Ok(None),
                Err(PageError::NotConnected),
                Ok(Some(InputValueMatch {
                    value: "last".into(),
                    preview: "<input/>".into(),
                })),
            ],
        );
        let (progress, _) = scoped_progress(false, 1000.0, Duration::from_secs(30));
        let options = InputValueOptions {
            selector: "input".into(),
            strict: true,
        };
        assert_eq!(
            input_value(&tab, &options, &progress).await,
            Ok("last".into())
        );
        assert!(tab.count("input_value") > 1);
        assert_eq!(tab.count("resolve"), 0);
        assert_eq!(tab.count("input"), 0);
        assert!(progress.delivery().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_invalid_selectors_strict_matches_and_wrong_value_elements_fail_at_once() {
        let options = InputValueOptions {
            selector: "input".into(),
            strict: true,
        };
        let mut tab = FakeTab::ready();
        let mut facts = tab.describe.next().expect("facts");
        facts.invalid = Some("Invalid selector".into());
        tab.describe = Step::always(TRIP, Ok(facts));
        is_error(
            &input_value(&tab, &options, &Progress::for_test(1000.0))
                .await
                .map(|_| ()),
            ErrorName::Error,
        );
        assert_eq!(tab.count("input_value"), 0);
        for message in [
            "strict mode violation",
            "Node is not an <input>, <textarea> or <select> element",
        ] {
            let mut tab = FakeTab::ready();
            tab.forms.value = Step::always(TRIP, Err(PageError::Threw(message.into())));
            is_error(
                &input_value(&tab, &options, &Progress::for_test(1000.0))
                    .await
                    .map(|_| ()),
                ErrorName::Error,
            );
            assert_eq!(tab.count("input_value"), 1);
        }
    }
}
