//! The `run` tool's description: what a model reads before it writes a script.
//!
//! Each constant is a raw string with one sentence per line, served exactly as written, so the
//! source reads as the model reads it.
//!
//! A model reads this on every call, follows it literally and pays for every word, so each
//! sentence is either true of every build and every caller, or says the condition it depends on.
//! Behaviour a script meets rarely stays out: its refusal names what to do instead, and
//! `agent.d.ts` and neo-server's README hold the detail. A host may copy this text into its own
//! instructions, so a change of meaning here is named in the pull request that makes it.

use crate::config::Defaults;
use crate::dispatch::{Listing, Registry};

/// What a script is. True of every build and every caller.
const USAGE: &str = r"Runs JavaScript in the user's browser and returns what the script returns.
`code` is the body of an async function: `await` every SDK call and `return` a JSON value.
Each run starts with fresh JavaScript; tabs stay open between runs.";

/// The habit a host's evaluation (13 Nov) showed models lacked: look, act on a ref, look
/// again. Shown once `page.ariaSnapshot` is registered. Every sentence answers a counted
/// mistake: locators guessed where the snapshot printed a ref (308 actions, 20% failed), actions
/// chained past a click that opened a dialog, snapshots sliced until the dialog was cut off.
///
/// What it says the snapshot prints is the snapshot design's, and its author approved each
/// sentence. The prelude's v1 text (`neo-playwright-js/src/prelude/snapshot-v1.ts`) prints refs
/// only on what can be acted on and on sections, and the page's text while the whole stays under
/// `TEXT_LIMIT_CHARS` (a test reads the number from there). A field with no size or no pointer
/// gets no ref, and `fill` waits out its whole timeout on one with no size and names nothing, so
/// that sentence stays. The cap (`snapshot-cap.ts`) prints a dialog, alertdialog, alert or status
/// block whole past its cut and drops every other cover, so the text says "popup" for what a page
/// shows and names the roles for what survives. A sentence about what the snapshot's first line
/// says stands over the page goes right after this paragraph, once the snapshot prints one.
const LOOP: &str = r#"Work in a loop: look, act, look again.
- Look: `return await page.ariaSnapshot()` lists what you can act on, each with a ref, such as `- button "Add to cart" [ref=e41]`.
  It includes the page's text while the whole stays under about 3,000 tokens, and a line at its top says when it left the text out; `{ text: true }` always includes it.
- Act on a ref from your latest snapshot: `await page.getByRef('e41').click()`.
  A guessed locator (`getByRole`, `getByText`, `locator('main')`) can match nothing and wait out its timeout, or match two and fail; use one only for an element printed without a ref.
  `page.getByRef('e12').ariaSnapshot({ mode: 'full' })` prints section e12 with a ref on every visible element.
  A field printed without a ref has no size or takes no pointer: `page.getByRole('textbox', { name: '…' }).pressSequentially('…')` types into it; `fill` waits out its timeout on one with no size.
- Look again after an action that can open, close or load something (a click, Enter, a submit): end the script by returning a new snapshot, and choose the next action from it.
  Fills of one form can share a script; the click that submits it ends the script.
- Return each snapshot whole, as the string it is.
  When its `# cut:` line says more was left out, snapshot the section it offers (`page.getByRef('e12').ariaSnapshot()`), or reach what you know is there with a locator.
  Pages often list a popup last, where a slice, a line filter or a small `maxTokens` can cut it off; only a block with a dialog or alert role survives the cut."#;

/// The wait times, which a test reads from the prelude's `timeouts.ts` so the two cannot drift
/// (this text once said 5 s while the prelude waited 30). None of the 71 waits in that evaluation
/// that ran the full 30 s would have succeeded after 5 s, so the text advises a short timeout
/// when the target is uncertain. Timeout settings live in the prelude, which every run starts
/// afresh, hence "for the rest of the run".
const WAITS: &str = r"A wait (an action, a read, a snapshot, a navigation) gives up after 30 s and an `expect` after 5 s (`toPass` never), unless given a `timeout`.
Waiting longer does not bring a missing or covered element: when you are not sure an element is there and free, pass `{ timeout: 5000 }`.
`page.setDefaultTimeout(ms)` sets the wait for that page for the rest of the run, navigations included.";

/// A pointer action under a cover: it waits out its timeout, then fails `covered`, and its
/// message names the cover with its ref, its snapshot and its controls, and one line of what to
/// do (`neo-playwright/src/action/cover.rs`). When a covered click comes to answer early, the pull
/// request that builds it rewrites this sentence.
const COVERED_CLICK: &str = r"A click on an element that another element covers waits out its timeout and fails `covered`; its message says what is `On top:`, with its ref and its controls, and what to do.
Answer or close it with one of those controls (in a script: `error.cover.controls`), then click again; if the message says the same layer covered your last click, closing it again will not help.";

/// A checkbox or radio under its own label, which upstream Playwright refuses to click too, and
/// the trusted ways that work today, as the cover rules' author measured them on a real browser:
/// Space on a rendered control (a 1 px clipped one included) is one trusted press that no cover
/// delays; on a control that is not rendered it returns without error and changes nothing, hence
/// the read-back; on a box that is already checked it unchecks it (a host measured this), hence
/// "only when `isChecked()` differs"; a click at a label's centre followed the link inside it, a
/// click at its top-left corner did not. The text names no untrusted way (`dispatchEvent`): a
/// model would read it as leave to press through any cover. Removed by the pull request that
/// makes such a click land through its label; its pinning test fails until then.
const OWN_LABEL_CLICK: &str = r"When a checkbox's or radio's own label covers it (`<label …> intercepts pointer events`), press Space on the control: `await page.getByRef('e5').press('Space')` is one trusted press and waits for no cover.
Space toggles: press it only when `isChecked()` differs from the state you want.
If the control is not rendered (only its label shows), click the label at its top-left corner, `click({ position: { x: 5, y: 5 } })`, and only when the label starts with plain words: a click at the centre of a label that holds a link can follow the link and lose the page.
Read the state back with `isChecked()` either way: Space on a control that is not rendered changes nothing and says nothing.";

/// The error model, which the agent needs before it retries anything. The model reads
/// `retryable` inside its script, so the text says it means "safe to repeat", never "will
/// succeed": a refusal that is permanent still reads `true` because nothing was sent, and a model
/// that took it as a promise would loop. A host setting that limits what a run may do goes right
/// after this paragraph, worded as a condition.
const ERRORS: &str = r#"A failed SDK call throws a `NeoError` (a `TimeoutError` for a timeout) with `code`, `owner` (who must fix it: script, website, host or neo), `delivery` and `retryable`.
`delivery: "unknown"` means a write may have happened: look at the page before repeating it.
`retryable: true` means a repeat is safe, never that it will succeed: read the `message` first."#;

/// A host's per-call restriction (`_meta` `access: "read"`), stated as a condition so this static
/// text stays true for every run. The dispatch door refuses disallowed operations before any
/// browser work, which is why the sentence can promise "nothing sent". The refusal explains the
/// rejected operation, so the sentence lists only what a read-only script may do.
const READ_ONLY: &str = r"A host may start a run read-only: then only reads, `context.newPage()`, a `goto` to an `http` or `https` address, and closing a tab this run opened are allowed; anything else fails `read_only_run` with nothing sent.";

/// Tabs and tasks. One static text, never built from `tabs.scope` or the call: whether a run is
/// bound to its task is the host's setting and the call's `taskId`, so the text says what binding
/// does only as a condition, and stays true under either scope and for a run without a task. A
/// host's `taskId` seeds the run's current task (`dispatch/run_ctx.rs`), and the lazy page takes
/// that task's last-used open tab (`Dispatcher::open_default_page`), so the text names both ways
/// a run gets a task.
const TASKS: &str = r"A task is a set of tabs that later runs can return to.
A run has one when the host gives it one or the script picks one.
Built-in `page` is the task's last-used open tab (else another of its open tabs, else a new tab in it); with no task, `page` is a new tab every run.
`context.newPage()` opens a tab, in the task if there is one.
To keep tabs across runs yourself, create a task and return its id: `const task = await neo.createTask('flights'); return task.id`; start later runs with `await neo.task('<id>')`, and `await task.close()` when done.
`context.pages()` lists the tabs this run may use, with `page.tabId` and `page.taskId`; use it to find a tab or a lost task id.
When the host binds runs to their task, that is only this task's tabs (not the browser's tab count), and any other tab is refused with `page_not_in_task`.";

/// The realm. A test runs it and fails when a parameter the prelude adds, or an `Intl` a QuickJS
/// bump brings, is not said here.
const GLOBALS: &str = r"The function's parameters are Playwright's `page`, `context`, `browser`, `expect` and `playwright`, then `neo`, Node's `Buffer` (what `screenshot` and `pdf` return), `console` (its lines come back with the result), `setTimeout`, `clearTimeout`, `NeoError` and `TimeoutError`.
The engine is QuickJS, which has no `Intl`.";

const SNAPSHOT_METHOD: &str = "page.ariaSnapshot";

/// The description, in the order a model needs it: what a script is, the look-act loop with its
/// waits and covers (once the snapshot method exists), the error model, tasks, the realm and its
/// size limit, then the registry's listed methods: each `Own` line, and each `Grouped` line once
/// (the Playwright API is one line, not 120). Built once at startup; the registry is frozen, so
/// it never changes while Neo runs.
#[must_use]
pub(crate) fn run_description(registry: &Registry) -> String {
    let listed = registry.listed();
    let mut lines = vec![USAGE.to_owned()];
    if listed.iter().any(|spec| spec.name == SNAPSHOT_METHOD) {
        lines.push(LOOP.to_owned());
        lines.push(WAITS.to_owned());
        lines.push(format!("{COVERED_CLICK}\n{OWN_LABEL_CLICK}"));
    }
    lines.push(ERRORS.to_owned());
    lines.push(READ_ONLY.to_owned());
    lines.push(TASKS.to_owned());
    lines.push(format!(
        "{GLOBALS}\nA reply holds at most {} KiB of JSON.",
        Defaults::MAX_RESULT_BYTES / 1024
    ));
    let mut methods: Vec<&str> = Vec::new();
    for spec in &listed {
        match spec.listing {
            Listing::Own(line) => methods.push(line),
            Listing::Grouped(line) if !methods.contains(&line) => methods.push(line),
            Listing::Grouped(_) | Listing::Hidden => {}
        }
    }
    if !methods.is_empty() {
        lines.push("SDK methods:".to_owned());
        lines.extend(methods.into_iter().map(|line| format!("- {line}")));
    }
    lines.join("\n\n")
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::future::{self, Future};
    use std::time::Duration;

    use neo_trace::Effect;
    use serde_json::Value;

    use super::{
        COVERED_CLICK, ERRORS, GLOBALS, LOOP, OWN_LABEL_CLICK, READ_ONLY, TASKS, WAITS,
        run_description,
    };
    use crate::dispatch::{
        ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, PageTarget, RegistryBuilder,
        RegistryError,
    };
    use crate::runtime::{Script, ScriptOutcome, run_script};
    use crate::test_rig::{TestError, TestRig, on_neo_runtime};

    /// A method that is only its spec.
    struct Named(MethodSpec);

    impl Method for Named {
        type Input = Value;
        type Output = Value;

        fn spec(&self) -> MethodSpec {
            self.0
        }

        fn target(_input: &Value) -> Option<&PageTarget> {
            None
        }

        fn call(
            &self,
            _op: &OpCtx<Self>,
            _input: Value,
        ) -> impl Future<Output = Result<Value, LeafError>> + Send {
            future::ready(Ok(Value::Null))
        }
    }

    fn named(name: &'static str, listing: Listing) -> Named {
        Named(MethodSpec {
            listing,
            ..MethodSpec::new(name, Effect::Read)
        })
    }

    /// The description with the snapshot method registered, as a real server lists it.
    fn with_snapshot() -> Result<String, RegistryError> {
        let mut builder = RegistryBuilder::default();
        builder.define(named(
            "page.ariaSnapshot",
            Listing::Grouped("Playwright's Page API"),
        ))?;
        Ok(run_description(&builder.build()))
    }

    /// The digits of `const {name} = …` in a TypeScript source, `_` separators dropped, so a
    /// `;` or a comment after the literal changes nothing.
    fn ts_number(source: &str, name: &str) -> Result<u64, Box<dyn Error>> {
        let (_, value) = source
            .lines()
            .find_map(|line| line.split_once(&format!("const {name} = ")))
            .ok_or(format!("no `const {name}`"))?;
        Ok(value
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '_')
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse()?)
    }

    /// A TypeScript file's words with comment markers and line breaks gone, so a sentence that
    /// wraps across `*` lines reads as one.
    fn prose(source: &str) -> String {
        source
            .lines()
            .map(|line| line.trim().trim_start_matches(['/', '*']).trim())
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `3000` as the text writes it, `3,000`.
    fn grouped(number: u64) -> String {
        let digits = number.to_string();
        let mut out = String::new();
        for (index, digit) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                out.push(',');
            }
            out.push(digit);
        }
        out
    }

    #[test]
    fn grouped_methods_share_one_line_and_hidden_ones_stay_out() -> Result<(), RegistryError> {
        let mut builder = RegistryBuilder::default();
        builder.define(named("page.goto", Listing::Grouped("the page API")))?;
        builder.define(named("page.title", Listing::Grouped("the page API")))?;
        builder.define(named(
            "neo.solveCaptcha",
            Listing::Own("neo.solveCaptcha(page)"),
        ))?;
        builder.define(named("neo.internal", Listing::Hidden))?;
        let description = run_description(&builder.build());
        assert_eq!(
            description.matches("- the page API").count(),
            1,
            "one line for the group"
        );
        assert!(description.contains("- neo.solveCaptcha(page)"));
        assert!(!description.contains("neo.internal"), "hidden stays hidden");
        assert!(
            !description.contains("ariaSnapshot"),
            "no snapshot loop before the method it names exists"
        );
        Ok(())
    }

    /// The loop is the habit the text exists to teach; a host reads its sentences to keep its
    /// own instructions from contradicting them, so each is pinned whole.
    #[test]
    fn the_loop_waits_and_covers_appear_with_the_snapshot_method() -> Result<(), Box<dyn Error>> {
        let description = with_snapshot()?;
        for part in [LOOP, WAITS, COVERED_CLICK, OWN_LABEL_CLICK] {
            assert!(description.contains(part), "{part}");
        }
        for sentence in [
            "Work in a loop: look, act, look again.",
            "Act on a ref from your latest snapshot: `await page.getByRef('e41').click()`.",
            "use one only for an element printed without a ref.",
            "end the script by returning a new snapshot, and choose the next action from it.",
            "Fills of one form can share a script; the click that submits it ends the script.",
            "Return each snapshot whole, as the string it is.",
            "only a block with a dialog or alert role survives the cut.",
            "`fill` waits out its timeout on one with no size.",
            "when you are not sure an element is there and free, pass `{ timeout: 5000 }`.",
            "if the message says the same layer covered your last click, closing it again will not help.",
            "press Space on the control: `await page.getByRef('e5').press('Space')` is one trusted press and waits for no cover.",
            "Space toggles: press it only when `isChecked()` differs from the state you want.",
            "Read the state back with `isChecked()` either way:",
        ] {
            assert!(description.contains(sentence), "{sentence}: {description}");
        }
        assert!(
            !description.contains("ariaSnapshotJSON"),
            "the text never sends the model to the uncapped, unmarked tree: {description}"
        );
        assert!(
            !description.contains("dispatchEvent"),
            "the text names no untrusted press: a model reads it as leave to press through any \
             cover: {description}"
        );
        let snapshot = description.find("Work in a loop").ok_or("no loop")?;
        let errors = description.find(ERRORS).ok_or("no errors")?;
        assert!(
            snapshot < errors,
            "the common path comes before the exceptions"
        );
        Ok(())
    }

    /// The cap keeps these four roles whole past its cut, and only these: what the text says
    /// survives a small `maxTokens` must be what the cap keeps.
    #[test]
    fn the_roles_said_to_survive_a_cut_are_the_caps() {
        const V1: &str = include_str!("../../../neo-playwright-js/src/prelude/snapshot-v1.ts");
        let kept = V1
            .split("export const KEPT = new Set<string>([")
            .nth(1)
            .and_then(|rest| rest.split("])").next())
            .unwrap_or_default();
        for role in ["'dialog'", "'alertdialog'", "'alert'", "'status'"] {
            assert!(kept.contains(role), "the cap keeps {role}: {kept}");
        }
        assert!(LOOP.contains("a dialog or alert role survives the cut"));
    }

    /// The text-length rule's number and the default cap are the prelude's, read rather than
    /// repeated, here and in `agent.d.ts`. The limit is written there as tokens times
    /// `CHARS_PER_TOKEN`; a change of that form fails here first.
    #[test]
    fn the_described_text_limit_is_the_preludes() -> Result<(), Box<dyn Error>> {
        const V1: &str = include_str!("../../../neo-playwright-js/src/prelude/snapshot-v1.ts");
        const AGENT_D_TS: &str = include_str!("../../../agent.d.ts");
        let line = V1
            .lines()
            .find(|line| line.contains("const TEXT_LIMIT_CHARS = "))
            .ok_or("snapshot-v1.ts has no TEXT_LIMIT_CHARS")?;
        assert!(
            line.trim_end().ends_with("* CHARS_PER_TOKEN"),
            "the limit is tokens times CHARS_PER_TOKEN: {line}"
        );
        let tokens = ts_number(V1, "TEXT_LIMIT_CHARS")?;
        let said = format!(
            "while the whole stays under about {} tokens",
            grouped(tokens)
        );
        assert!(LOOP.contains(&said), "the loop says: {said}");
        let handout = prose(AGENT_D_TS);
        let cap = ts_number(V1, "DEFAULT_MAX_TOKENS")?;
        for said in [
            format!(
                "while the whole stays under about {} tokens",
                grouped(tokens)
            ),
            format!("The cap, about {} tokens by default.", grouped(cap)),
        ] {
            assert!(handout.contains(&said), "agent.d.ts says: {said}");
        }
        Ok(())
    }

    /// The description's wait times are the prelude's defaults, read from `timeouts.ts` rather
    /// than repeated, so the two cannot drift apart again (the text once said 5 s while the
    /// prelude waited 30). neo-server's README and `agent.d.ts` say the same. `toPass` is the
    /// one `expect` with no default limit (expect.ts), which the sentence names.
    #[test]
    fn tests_that_the_described_wait_times_are_the_preludes_defaults() -> Result<(), Box<dyn Error>>
    {
        const TIMEOUTS: &str = include_str!("../../../neo-playwright-js/src/prelude/timeouts.ts");
        const README: &str = include_str!("../../README.md");
        const AGENT_D_TS: &str = include_str!("../../../agent.d.ts");
        let wait = ts_number(TIMEOUTS, "DEFAULT_TIMEOUT")? / 1000;
        let expect = ts_number(TIMEOUTS, "DEFAULT_EXPECT_TIMEOUT")? / 1000;
        assert_eq!(
            ts_number(TIMEOUTS, "DEFAULT_NAVIGATION_TIMEOUT")? / 1000,
            wait,
            "the description gives a navigation the wait's time"
        );
        let said = format!("gives up after {wait} s and an `expect` after {expect} s");
        assert!(WAITS.contains(&said), "the run description says: {said}");
        let readme = README.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(readme.contains(&said), "neo-server's README says: {said}");
        let handout = format!("wait up to {wait} s (expect: {expect} s, toPass: no limit)");
        assert!(
            prose(AGENT_D_TS).contains(&handout),
            "agent.d.ts says: {handout}"
        );
        Ok(())
    }

    /// Every error code the text names is one Neo answers, and the cover lines it quotes are the
    /// ones the covered answer writes.
    #[test]
    fn the_codes_and_the_cover_line_the_text_names_are_neos() {
        const POINTER: &str = include_str!("../../../neo-playwright/src/action/pointer_action.rs");
        const COVER: &str = include_str!("../../../neo-playwright/src/action/cover.rs");
        for (code, part) in [
            (ErrorCode::COVERED, COVERED_CLICK),
            (ErrorCode::PAGE_NOT_IN_TASK, TASKS),
            (ErrorCode::READ_ONLY_RUN, READ_ONLY),
        ] {
            assert!(part.contains(&format!("`{}`", code.as_str())), "{part}");
        }
        assert!(POINTER.contains("intercepts pointer events"));
        assert!(OWN_LABEL_CLICK.contains("intercepts pointer events"));
        assert!(COVER.contains("\\n  On top: "));
        assert!(COVERED_CLICK.contains("`On top:`"));
        assert!(COVER.contains("The same layer covered your last "));
        assert!(COVERED_CLICK.contains("the same layer covered your last click"));
    }

    #[test]
    fn the_read_only_sentence_is_conditional_and_follows_errors() {
        const SENTENCE: &str = r"A host may start a run read-only: then only reads, `context.newPage()`, a `goto` to an `http` or `https` address, and closing a tab this run opened are allowed; anything else fails `read_only_run` with nothing sent.";
        let description = run_description(&RegistryBuilder::default().build());
        assert_eq!(
            description.matches(SENTENCE).count(),
            1,
            "the host condition and refusal are stated once"
        );
        assert!(
            description.contains(&format!("{ERRORS}\n\n{SENTENCE}")),
            "read-only access follows the error model it relies on"
        );
    }

    #[test]
    fn the_usage_teaches_task_selection_and_plain_playwright() -> Result<(), TestError> {
        let description = run_description(&RegistryBuilder::default().build());
        for sentence in [
            "returns what the script returns.",
            "Each run starts with fresh JavaScript; tabs stay open between runs.",
            "A run has one when the host gives it one or the script picks one.",
            "Built-in `page` is the task's last-used open tab (else another of its open tabs, else a new tab in it); with no task, `page` is a new tab every run.",
            "`context.newPage()` opens a tab, in the task if there is one.",
            "To keep tabs across runs yourself, create a task and return its id: `const task = await neo.createTask('flights'); return task.id`;",
            "start later runs with `await neo.task('<id>')`, and `await task.close()` when done.",
            "`context.pages()` lists the tabs this run may use, with `page.tabId` and `page.taskId`; use it to find a tab or a lost task id.",
            "When the host binds runs to their task, that is only this task's tabs (not the browser's tab count), and any other tab is refused with `page_not_in_task`.",
        ] {
            assert!(
                description.contains(sentence),
                "the model writes its first script from this text, so it must learn the task \
                 recipe and the built-in page rule here: {sentence}"
            );
        }
        let recipe = description
            .find("To keep tabs across runs")
            .ok_or("missing task recipe")?;
        let first_script = description
            .find("`const task =")
            .ok_or("missing first script")?;
        assert!(
            recipe < first_script,
            "the model learns how to keep tabs before copying its first script"
        );
        assert!(
            !description.contains(concat!("neo.", "tabs")),
            "tab discovery uses Playwright's context"
        );
        Ok(())
    }

    /// The prose is hand-written, so the realm checks it: a parameter a prelude adds, or an
    /// `Intl` a QuickJS bump brings, fails here until the description says so.
    #[test]
    fn the_usage_names_every_parameter_and_the_missing_intl() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let description = run_description(rig.dispatcher.registry());
            let (run, _guard) = rig.run(Duration::from_mins(1), None)?;
            let script = Script {
                code: "return [__neo.parameters(), typeof Intl];".to_owned(),
                page: None,
                pages: Vec::new(),
                observed: None,
            };
            let ScriptOutcome::Returned(value) =
                run_script(&run, &rig.dispatcher, script).await.outcome
            else {
                return Err("the probe script returns".into());
            };
            let (parameters, intl): (String, String) = serde_json::from_value(value)?;
            for name in parameters.split(", ") {
                assert!(
                    GLOBALS.contains(&format!("`{name}`")),
                    "the description names the parameter `{name}`: {description}"
                );
            }
            assert_eq!(intl, "undefined");
            assert!(GLOBALS.contains("no `Intl`"));
            assert!(description.contains(GLOBALS));
            Ok(())
        })
    }
}
