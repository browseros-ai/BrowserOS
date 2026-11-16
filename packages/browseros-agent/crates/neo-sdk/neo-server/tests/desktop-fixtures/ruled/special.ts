import type { RuledCaseSpec } from '../model'

/** SDK rulings preserve original identities while asserting current public behavior. */
// biome-ignore format: Keep the complete pinned rule records together.
export const specialCases = [
  {
    "id": "act-017",
    "file": "cases-act.ts",
    "name": "act: select updates a covered styled native select semantically",
    "proves": "The terminal contract correlates one unknown locator.selectOption with the caught error. A pass requires verified cleanup and browser destruction.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/act-017.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "terminal-select",
    "ruling": {
      "sdkBehavior": "Selecting green returns ['green'] and updates the prompt with one change. Missing chartreuse throws TimeoutError with code not_actionable, owner website, and delivery unknown. Same-script readback retains green, one change, and the Green prompt.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "190-225",
          "claim": "Form edits retry until completion, a definite failure, or the operation timeout."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "368-393",
          "claim": "SelectOption marks the write sent before editing and retries when options are missing."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "491-497",
          "claim": "A form timeout uses not_actionable after a match and no_match before a match."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "883-889",
          "claim": "Missing options return before selection changes or input and change events."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/form.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "34-43",
          "claim": "The Color select includes green but no chartreuse option."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/form.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "70-76",
          "claim": "The change handler increments colorChanges and updates the prompt to the selected option's text."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "115-129",
          "claim": "NeoError exposes code, owner, delivery, retryable, and operationId."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/dispatch/operation_guard.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "173-181",
          "claim": "Delivery is not_sent before sending, done after a definite sent answer, and unknown after an ambiguous sent answer."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "182-188",
          "claim": "Run metadata reports script status, write effects, and the list of unconfirmed writes."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/dispatch/run_ctx.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-423",
          "claim": "The run reports effects unknown whenever its ledger contains an unknown write."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/dispatch/run_ctx.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "522-550",
          "claim": "Unknown writes append to the ledger, and later read settlements leave that list unchanged."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "66-83",
          "claim": "Run metadata serializes each unknown operation's name and operationId."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "104-108",
          "claim": "The rendered reply names each unknown write as sent but unconfirmed."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/errors.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-10",
          "claim": "Operation errors retain their envelope fields and object identity when reshaped for scripts."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/errors.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "36-44",
          "claim": "The bridge error field list includes operationId."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/errors.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "68-80",
          "claim": "Error shaping copies defined envelope fields without changing their values."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/errors.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "101-135",
          "claim": "Existing Error objects keep their identity when shapeBridgeError sets the name and timeout field."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/runtime/tests.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "106-127",
          "claim": "The source test asserts that caught operationId is the run ID followed by .1."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_description.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-40",
          "claim": "The run description defines unknown delivery as a write that may have happened."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/tests/desktop-fixtures/mcp.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "13-45",
          "claim": "The ordinary parser rejects unknown effects, unknown writes, and unfinished-operation markers."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/tests/desktop-fixtures/mcp.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "48-66",
          "claim": "parseRunReply validates the run ID and invokes requireCompletedRun before returning a reply."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/tests/desktop-fixtures/harness.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "154-185",
          "claim": "The harness sets poisoned after a run failure and refuses later case reuse."
        }
      ]
    },
    "postconditions": [
      "After green, {value:'green', changes:1, prompt:'Green'}.",
      "After missing-option timeout, the selected value, change count and prompt remain green/1/Green.",
      "Read the page within the same script after catching the missing-option error.",
      "Capture caught.operationId and require the only unknown meta entry to equal {name:'locator.selectOption', operationId:capturedId}.",
      "Require capturedId to belong to this run ID with a positive operation sequence.",
      "Require meta status ok, effects unknown, one unknown entry, no unrelated error or unfinished-operation marker."
    ]
  },
  {
    "id": "read-eval-022",
    "file": "cases-read-eval.ts",
    "name": "run: clamps an over-long timeout",
    "proves": "A requested timeout of 600000 ms is valid and is used unchanged; SDK default is 300000 and maximum 900000.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/read-eval-022.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/read-eval-022-verify.txt"
      }
    ],
    "timeoutMs": 600000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "A requested timeout of 600000 ms is valid and is used unchanged; SDK default is 300000 and maximum 900000.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_args.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "10-24",
          "claim": "The run tool accepts exactly code and optional timeout; unknown fields such as maxChars are refused."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/config/defaults.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "26-50",
          "claim": "Default timeout is 300000 ms, maximum 900000 ms, return JSON limit 256 KiB, console limit 1000 lines and 1 MiB."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/run/run_request.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "21-50",
          "claim": "A nonzero timeout up through 900000 ms becomes RunTimeout unchanged; out-of-range budgets are script/invalid_input."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/run/run_service.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "273-285",
          "claim": "The run starts with a deadline of Instant::now() + timeout.get(), with no desktop clamp."
        }
      ]
    },
    "postconditions": [
      "Issue a short deterministic successful run carrying the original timeout=600000.",
      "Require the exact returned sentinel value, status ok, isError false, and known completion through the existing MCP parser.",
      "Cite RunTimeout and run-service deadline code for the stronger non-clamping claim. An immediate result cannot itself measure the accepted deadline."
    ],
    "driverTimeoutMs": 20000
  }
] satisfies readonly RuledCaseSpec[]
