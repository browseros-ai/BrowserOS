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
/// sentence. Neo's default snapshot format prints refs only on what can be acted on and on
/// sections, and the page's text while the whole stays under `neo_snapshot::TEXT_LIMIT_TOKENS`
/// (the tests compare the numbers here with the renderer's constants). A field with no size or no
/// pointer gets no ref, and `fill` waits out its whole timeout on one with no size and names
/// nothing, so that sentence stays. The cap prints a block of a role in `neo_snapshot::KEPT_ROLES`
/// (dialog, alertdialog, alert, status) whole past its cut and drops every other cover, so the
/// text says "popup" for what a page shows and names the roles for what survives.
/// A ref resolves from `page` in any frame: the resolver jumps to the document that printed it
/// (`neo-playwright/src/resolve/resolver.rs`). A model that knows Playwright doubts that for a
/// frame's content, and in a real run guessed a locator beside a printed frame ref, so the text
/// says it. `SNAPSHOT_ON_TOP`, right after this paragraph, says what the snapshot's `# on top:`
/// lines mean.
const LOOP: &str = r#"Work in a loop: look, act, look again.
- Look: `return await page.ariaSnapshot()` lists what you can act on, each with a ref, such as `- button "Add to cart" [ref=e41]`.
  It includes the page's text while the whole stays under about 3,000 tokens, and a line at its top says when it left the text out; `{ text: true }` always includes it.
- Act on a ref from your latest snapshot: `await page.getByRef('e41').click()`.
  A guessed locator (`getByRole`, `getByText`, `locator('main')`) can match nothing and wait out its timeout, or match two and fail; use one only for an element printed without a ref.
  A ref works from `page` wherever the snapshot printed it, inside a frame too: `page.getByRef('f12e12')`.
  `page.getByRef('e12').ariaSnapshot({ mode: 'full' })` prints section e12 with a ref on every visible element.
  A field printed without a ref has no size or takes no pointer: `page.getByRole('textbox', { name: '…' }).pressSequentially('…')` types into it; `fill` waits out its timeout on one with no size.
- Look again after an action that can open, close or load something (a click, Enter, a submit): end the script by returning a new snapshot, and choose the next action from it.
  Fills of one form can share a script; the click that submits it ends the script.
- Return each snapshot whole, as the string it is.
  When its `# cut:` line says more was left out, snapshot the section it offers (`page.getByRef('e12').ariaSnapshot()`), or reach what you know is there with a locator.
  Pages often list a popup last, where a slice, a line filter or a small `maxTokens` can cut it off; only a block with a dialog or alert role survives the cut."#;

/// What the snapshot's `# on top:` lines mean. The snapshot prints them above everything else
/// when a layer takes the pointer from controls (`neo-playwright/src/snapshot/snapshot_op.rs`,
/// worded by `on_top_line` in `cover.rs`), and they ask the agent to act on that layer first, so
/// they earn a sentence on every call; the inert rule's `# not listed:` line explains itself
/// and has none.
const SNAPSHOT_ON_TOP: &str = r"A line `# on top: …` at the snapshot's top names a layer that takes the pointer from the controls under it, with that layer's own controls: deal with it first.";

/// The wait times, which a test reads from the prelude's `timeouts.ts` so the two cannot drift
/// (this text once said 5 s while the prelude waited 30). None of the 71 waits in that evaluation
/// that ran the full 30 s would have succeeded after 5 s, so the text advises a short timeout
/// when the target is uncertain. Timeout settings live in the prelude, which every run starts
/// afresh, hence "for the rest of the run".
const WAITS: &str = r"A wait (an action, a read, a snapshot, a navigation) gives up after 30 s and an `expect` after 5 s (`toPass` never), unless given a `timeout`.
Waiting longer does not bring a missing element: when you are not sure an element is there, pass `{ timeout: 5000 }`.
`page.setDefaultTimeout(ms)` sets the wait for that page for the rest of the run, navigations included.";

/// A pointer action under a cover fails `covered`, and its message names the cover with its ref,
/// its snapshot and its controls, and one line of what to do (`neo-playwright/src/action/cover.rs`).
/// Under Neo's own timeout it stops once the same cover has stayed 5 s (`EARLY_STOP` in that
/// file); a timeout someone chose, a cover that moves, fades or says it is loading, and `force`
/// keep the whole wait. The text names those covers so a script does not lengthen its timeout for
/// them, and names `page.setDefaultTimeout(30_000)`, the prelude's default set by hand, as the way
/// back. A press whose cover is the target's own label, wrapper or drawing, or that finds a clear
/// point of the target, lands by itself and the call log says which, so the text says nothing of
/// it; a label that is all link gets this answer, whose Do line says to press Space.
const COVERED_CLICK: &str = r"A click on an element that another element covers fails `covered`; its message says what is `On top:`, with its ref and its controls, and what to do.
Answer or close it with one of those controls (in a script: `error.cover.controls`), then click again; if the message says the same layer covered your last click, closing it again will not help.
Unless you set a timeout, a click stops waiting after 5 s if the same cover stays on top and is not moving, fading or loading; to wait for a cover that leaves on its own, set one: `page.setDefaultTimeout(30_000)`, or `timeout` on the call.";

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
        lines.push(SNAPSHOT_ON_TOP.to_owned());
        lines.push(WAITS.to_owned());
        lines.push(COVERED_CLICK.to_owned());
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
        COVERED_CLICK, ERRORS, GLOBALS, LOOP, READ_ONLY, SNAPSHOT_ON_TOP, TASKS, WAITS,
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
        for part in [LOOP, SNAPSHOT_ON_TOP, WAITS, COVERED_CLICK] {
            assert!(description.contains(part), "{part}");
        }
        for sentence in [
            "Work in a loop: look, act, look again.",
            "A line `# on top: …` at the snapshot's top names a layer that takes the pointer from the controls under it, with that layer's own controls: deal with it first.",
            "Act on a ref from your latest snapshot: `await page.getByRef('e41').click()`.",
            "use one only for an element printed without a ref.",
            "A ref works from `page` wherever the snapshot printed it, inside a frame too: `page.getByRef('f12e12')`.",
            "end the script by returning a new snapshot, and choose the next action from it.",
            "Fills of one form can share a script; the click that submits it ends the script.",
            "Return each snapshot whole, as the string it is.",
            "only a block with a dialog or alert role survives the cut.",
            "`fill` waits out its timeout on one with no size.",
            "when you are not sure an element is there, pass `{ timeout: 5000 }`.",
            "Unless you set a timeout, a click stops waiting after 5 s if the same cover stays on top and is not moving, fading or loading; to wait for a cover that leaves on its own, set one: `page.setDefaultTimeout(30_000)`, or `timeout` on the call.",
            "if the message says the same layer covered your last click, closing it again will not help.",
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

    /// This text must ship with the snapshot's behaviour, not merely with an unused formatter.
    #[test]
    fn the_on_top_sentence_has_a_snapshot_caller() {
        const SNAPSHOT: &str = include_str!("../../../neo-playwright/src/snapshot/snapshot_op.rs");
        assert!(SNAPSHOT.contains("action::on_top_line(&note)"));
        assert!(SNAPSHOT_ON_TOP.contains("`# on top: …`"));
    }

    /// The cap keeps these four roles whole past its cut, and only these: what the text says
    /// survives a small `maxTokens` must be what the cap keeps. Held to the renderer's own list.
    #[test]
    fn the_roles_said_to_survive_a_cut_are_the_caps() {
        let mut kept = neo_snapshot::KEPT_ROLES.to_vec();
        kept.sort_unstable();
        assert_eq!(kept, ["alert", "alertdialog", "dialog", "status"]);
        assert!(LOOP.contains("a dialog or alert role survives the cut"));
    }

    /// The text-length rule's number and the default cap, said here and in `agent.d.ts`, are the
    /// renderer's constants by value: a cap or limit moved by a single token fails here.
    #[test]
    fn the_described_text_limit_is_the_renderers() {
        const AGENT_D_TS: &str = include_str!("../../../agent.d.ts");
        let tokens = u64::from(neo_snapshot::TEXT_LIMIT_TOKENS);
        let cap = u64::from(neo_snapshot::DEFAULT_MAX_TOKENS);
        let said = format!(
            "while the whole stays under about {} tokens",
            grouped(tokens)
        );
        assert!(LOOP.contains(&said), "the loop says: {said}");
        let handout = prose(AGENT_D_TS);
        for said in [
            format!(
                "while the whole stays under about {} tokens",
                grouped(tokens)
            ),
            format!("The cap, about {} tokens by default.", grouped(cap)),
        ] {
            assert!(handout.contains(&said), "agent.d.ts says: {said}");
        }
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

    /// The covered click's stop is covered-click B's own constant (`EARLY_STOP` in `cover.rs`), and
    /// its way back is the prelude's `DEFAULT_TIMEOUT` set by hand, read rather than repeated, so
    /// neither sentence can drift from the code.
    #[test]
    fn tests_that_the_covered_stop_and_its_way_back_are_the_codes() -> Result<(), Box<dyn Error>> {
        const COVER: &str = include_str!("../../../neo-playwright/src/action/cover.rs");
        const TIMEOUTS: &str = include_str!("../../../neo-playwright-js/src/prelude/timeouts.ts");
        let stop = COVER
            .split_once("const EARLY_STOP: Duration = Duration::from_secs(")
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(seconds, _)| seconds)
            .ok_or("cover.rs gives EARLY_STOP in whole seconds")?;
        let said = format!("a click stops waiting after {stop} s");
        assert!(
            COVERED_CLICK.contains(&said),
            "the run description says: {said}"
        );
        let default = ts_number(TIMEOUTS, "DEFAULT_TIMEOUT")?;
        let way_back = format!(
            "`page.setDefaultTimeout({}_{:03})`",
            default / 1000,
            default % 1000
        );
        assert!(
            COVERED_CLICK.contains(&way_back),
            "the way back is the prelude's default: {way_back}"
        );
        Ok(())
    }

    /// Every error code the text names is one Neo answers, and the cover lines it quotes are the
    /// ones the covered answer writes.
    #[test]
    fn the_codes_and_the_cover_line_the_text_names_are_neos() {
        const COVER: &str = include_str!("../../../neo-playwright/src/action/cover.rs");
        for (code, part) in [
            (ErrorCode::COVERED, COVERED_CLICK),
            (ErrorCode::PAGE_NOT_IN_TASK, TASKS),
            (ErrorCode::READ_ONLY_RUN, READ_ONLY),
        ] {
            assert!(part.contains(&format!("`{}`", code.as_str())), "{part}");
        }
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
