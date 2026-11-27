/**
 * These public SDK rulings preserve original identities while asserting successful SDK behavior.
 * Sequential steps carry caller-received values into fresh request realms; aliases still execute afresh.
 * The harness owns task cleanup and binds host checks to each case's actual MCP replies.
 */
import type { RuledCaseSpec } from '../model'

const combinedLog = 'L'.repeat(13_000) + 'SDK_CAP_LOG_TAIL'
const combinedValue = 'R'.repeat(20_000) + 'SDK_CAP_VALUE_TAIL'

// biome-ignore format: Keep the complete pinned rule records together.
export const readsCases = [
  {
    "id": "capture-io-008",
    "file": "cases-capture-io.ts",
    "name": "upload: an unexpected chooser is recoverable without a ref",
    "proves": "A native FileChooser retains a specific input handle and is reusable while that input remains live.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/capture-io-008.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "A native FileChooser retains a specific input handle and is reusable while that input remains live.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/event-values.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "22-54",
          "claim": "FileChooser owns a retained ElementHandle; setFiles delegates to that handle and resolves void. No consumed page-global request state exists here."
        }
      ]
    },
    "postconditions": [
      "Register filechooser before clicking #upload-button. chooser.page() is exactly the initiating Page.",
      "First setFiles selects pending-chooser.txt through the native chooser without obtaining a snapshot ref.",
      "Repeated setFiles on the same chooser resolves, with consumedError exactly null. Check the same input still contains pending-chooser.txt after reuse.",
      "A fresh chooser for the same live input followed by setFiles([]) leaves files.length exactly 0."
    ]
  },
  {
    "id": "capture-io-016",
    "file": "cases-capture-io.ts",
    "name": "upload: a removed chooser input fails and releases the pending request",
    "proves": "A FileChooser owns its exact ElementHandle even after the input detaches; a new chooser owns the replacement independently.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/capture-io-016.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "A FileChooser owns its exact ElementHandle even after the input detaches; a new chooser owns the replacement independently.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/event-values.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "22-54",
          "claim": "FileChooser owns a retained ElementHandle; setFiles delegates to that handle and resolves void. No consumed page-global request state exists here."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/filechooser-lifetime.test.mjs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "60-76",
          "claim": "A detached temporary chooser input survives native browser GC and accepts setFiles; its retained element exposes the selected file name."
        }
      ]
    },
    "postconditions": [
      "Acquire chooser before removing #hidden-file and prove its element is detached.",
      "chooser.setFiles for the detached original resolves, with removedError exactly null. Read chooser.element() to prove that old input received replacement-input.txt.",
      "Create a distinct replacement input with id hidden-file and acquire a fresh chooser.",
      "The replacement input receives exactly [replacement-input.txt]; the old chooser is never silently rebound to it.",
      "Clear the detached old chooser after selecting the replacement; the old file list becomes empty while the replacement retains its selected file."
    ]
  },
  {
    "id": "capture-io-026",
    "file": "cases-capture-io.ts",
    "name": "download: file lands on disk and its path is reported",
    "proves": "download.path() reports the browser-configured destination, whose readable bytes and same-run size are authoritative; directory spelling is host configuration.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/capture-io-026.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "download.path() reports the browser-configured destination, whose readable bytes and same-run size are authoritative; directory spelling is host configuration.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/tests/desktop-fixtures/browser-adapter.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "68-96",
          "claim": "The test launcher creates its private downloads directory and writes Chromium download.default_directory before browser startup."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/tests/desktop-fixtures/server.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "34-46",
          "claim": "The source SDK server uses the launcher-owned download folder through CLAW_DOWNLOADS_DIR."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/tests/desktop-fixtures/host-checks.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "300-367",
          "claim": "The host matches each returned path to exactly one completed same-run download and compares recorded size, disk bytes and served bytes."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/server.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "40-48",
          "claim": "The unmodified fixture endpoint serves exactly fixture report contents\\n with suggested filename report.txt."
        }
      ]
    },
    "postconditions": [
      "Subscribe to download before trusted activation of Download report on links.html.",
      "download.failure() is null, download.path() is a nonempty absolute path, suggestedFilename() is report.txt, and the URL ends /files/report.txt.",
      "Use the returned path, not a caller-chosen saveAs path, for all host verification.",
      "The host binds the returned path to a completed download in this same run, compares recorded size, disk and service bytes, and requires exactly fixture report contents\\n (24 bytes)."
    ],
    "hostCheck": {
      "kind": "download-file",
      "pathField": "path",
      "contentIncludes": "fixture report",
      "contentEquals": "fixture report contents\n",
      "requireExists": true
    }
  },
  {
    "id": "frame-refs-012",
    "file": "cases-frame-refs.ts",
    "name": "frame refs: a reloaded frame keeps its old refs inside that same frame",
    "proves": "An old ref cannot be rebound to a replacement document inside the same retained iframe.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/frame-refs-012.txt"
      }
    ],
    "timeoutMs": 90000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "An old ref cannot be rebound to a replacement document inside the same retained iframe.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/aria-ref.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-14",
          "claim": "Refs are node-bound and never re-find by role/name. A removed element or stale document yields element_detached."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/target.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "151-177",
          "claim": "Snapshot and minted refs identify the issuing document plus its local element number; a retired document is element_detached."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/resolve/resolver.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "422-425",
          "claim": "A ref whose element or document is gone reports website/element_detached."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/playwright_error.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "117-137",
          "claim": "no_match is script-owned; element_detached and navigation_failed are website-owned; invalid_input is script-owned."
        }
      ]
    },
    "postconditions": [
      "Exercise both original variants, single-cross with reload command at level 0 and nested-mixed with reload command at level 1.",
      "Wait for initial ready state; retain original ref and old docId; reload target document while retaining frame owner; require new docId differs.",
      "Old-ref click rejects as website/element_detached with delivery not_sent, mentioning the old ref and fresh snapshot guidance. No trusted event reaches any frame.",
      "Take a fresh full snapshot. Require fresh ref differs, then exactly one trusted button click reaches the new docId, expected target level, and expected fixture origin."
    ]
  },
  {
    "id": "isolation-003",
    "file": "cases-isolation.ts",
    "name": "isolation: run scripts the page only through page.evaluateInPage",
    "proves": "The run realm has no page window; page.evaluate explicitly executes in the page main world.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/isolation-explicit-evaluate.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "The run realm has no page window; page.evaluate explicitly executes in the page main world.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/page.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "853-865",
          "claim": "page.evaluate is a supported explicit evaluation API forwarded through the main frame."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "186-188",
          "claim": "run returns exactly the script result plus console/notes, and page.evaluate is a main-world API."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/script-isolation.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "23-26",
          "claim": "The committed isolation fixture exposes pageSecret as main-world-only."
        }
      ]
    },
    "postconditions": [
      "Open unchanged script-isolation.html.",
      "typeof window in the run realm is exactly undefined.",
      "page.evaluate(() => window.pageSecret) returns exactly main-world-only."
    ]
  },
  {
    "id": "navigate-snapshot-004",
    "file": "cases-navigate-snapshot.ts",
    "name": "navigate: file/javascript/data schemes are refused, chrome is not guarded",
    "proves": "SDK goto permits data/javascript, refuses file/browser-specific schemes before sending, and preserves Chromium post-execution ERR_ABORTED for JavaScript URLs.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/navigate-snapshot-004.txt"
      }
    ],
    "timeoutMs": 30000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "SDK goto permits data/javascript, refuses file/browser-specific schemes before sending, and preserves Chromium post-execution ERR_ABORTED for JavaScript URLs.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/builtins/playwright_ops.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "139-142",
          "claim": "goto permits http/https/data/about and additionally JavaScript; browser-specific schemes remain unsupported."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/builtins/playwright_ops.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "359-385",
          "claim": "Navigation scheme validation refuses file and other unsupported schemes before navigation is sent."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/builtins/playwright_ops.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "517-536",
          "claim": "URL refusals are script/invalid_input with delivery not_sent; file refusal recommends page.setContent."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/builtins/playwright_ops.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "111-117",
          "claim": "Chromium executes a javascript URL before reporting net::ERR_ABORTED; data URLs are permitted."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/playwright_error.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "117-137",
          "claim": "no_match is script-owned; element_detached and navigation_failed are website-owned; invalid_input is script-owned."
        }
      ]
    },
    "postconditions": [
      "Begin on links.html. file:///etc/hosts refuses with script/invalid_input and not_sent, with file guidance.",
      "Register dialog listener before goto javascript:alert(1). Require alert type and message 1, dismiss it, and then require website/navigation_failed/done with ERR_ABORTED.",
      "data:text/html,<b>x</b> returns null HTTP response and body text x.",
      "chrome://version refuses with script/invalid_input and not_sent."
    ]
  },
  {
    "id": "navigate-snapshot-014",
    "file": "cases-navigate-snapshot.ts",
    "name": "diff: mutation reports added nodes",
    "proves": "Mutation delta is a line diff of condensed v1 snapshot text; passive listitem roles are flattened into text.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/navigate-snapshot-014-baseline.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/reads/navigate-snapshot-014.txt"
      }
    ],
    "timeoutMs": 40000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Mutation delta is a line diff of condensed v1 snapshot text; passive listitem roles are flattened into text.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-delta.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-23",
          "claim": "Delta compares condensed v1 texts that reached the caller and retains the baseline rendering form."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-delta.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "143-158",
          "claim": "Line-diff output reports added/removed text lines, or a full page if comparison would be too large."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-v1.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "326-348",
          "claim": "Non-kept, non-section, non-heading, nonactionable roles have no row of their own; text and children print at their level."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1453-1468",
          "claim": "A baseline commits only after a successful run returns its snapshot text; a snapshot earlier in the same run is not a baseline."
        }
      ]
    },
    "postconditions": [
      "First successful run opens dynamic.html, waits for 3001 h6 filler elements and delayed content ready, returns the whole snapshot with text=true and maxTokens=4000 plus page/task identity.",
      "Second run rejoins exactly that task and tab. Trigger add-item; require #list li count exactly 4.",
      "Require a real delta header with positive added count and an added text row containing delta item 1.",
      "The v1 delta must not invent a + listitem role row. Retain full delta in the return so text committed by the host reached the caller."
    ]
  },
  {
    "id": "navigate-snapshot-016",
    "file": "cases-navigate-snapshot.ts",
    "name": "diff: navigation reports the url change",
    "proves": "A new main document clears the previous document baseline; its first delta is a whole page with the explicit no-earlier-snapshot notice.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/navigate-snapshot-016-baseline.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/reads/navigate-snapshot-016.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "A new main document clears the previous document baseline; its first delta is a whole page with the explicit no-earlier-snapshot notice.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1453-1468",
          "claim": "A baseline commits only after a successful run returns its snapshot text; a snapshot earlier in the same run is not a baseline."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-delta.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "39-40",
          "claim": "The no-baseline notice is # whole page: no earlier snapshot of this page."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/snapshot-delta.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "71-83",
          "claim": "A new document without a stored baseline returns the whole rendered page under the no-baseline notice."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/snapshot/snapshot_baselines.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "180-189",
          "claim": "A main-document commit changes its generation and clears the document baseline."
        }
      ]
    },
    "postconditions": [
      "First successful run opens links.html and returns its v1 snapshot and task/tab/url identity.",
      "Second run rejoins exactly that task/tab and sets location.href to /form.html; require actual path /form.html.",
      "Delta starts with # whole page: no earlier snapshot of this page and includes fresh Form fixture and Submit content.",
      "The return includes actual path and fresh delta. No desktop URL changed marker is required."
    ]
  },
  {
    "id": "read-eval-016",
    "file": "cases-read-eval.ts",
    "name": "evaluate_in_page: large return spills to a file",
    "proves": "A 20000-character page.evaluate string is returned unchanged inside the SDK 256 KiB value budget; text does not auto-spill.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/read-eval-016.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/reads/read-eval-016-verify.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "A 20000-character page.evaluate string is returned unchanged inside the SDK 256 KiB value budget; text does not auto-spill.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/runtime/fit_result.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-15",
          "claim": "Only branded byte values move to result blobs; text that fits stays untouched and text never spills automatically."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/config/defaults.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "26-50",
          "claim": "Default timeout is 300000 ms, maximum 900000 ms, return JSON limit 256 KiB, console limit 1000 lines and 1 MiB."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-62",
          "claim": "Successful MCP replies carry structuredContent.value, failed replies structuredContent.error, with text emitted separately."
        }
      ]
    },
    "postconditions": [
      "Open unchanged form.html. page.evaluate returns exactly z repeated 20000 times.",
      "In-script verification requires length=20000 and all characters z.",
      "At the caller boundary, require structuredContent.value equals the exact 20000-character string, rather than trusting only in-script length.",
      "Run 1 returns the complete string. A second successful request compares args.prior[0] with z repeated 20000 times, proving exact caller receipt."
    ]
  },
  {
    "id": "read-eval-018",
    "file": "cases-read-eval.ts",
    "name": "run: a selector where a ref belongs is refused before the wire",
    "proves": "SDK page.click accepts CSS selectors. The original .apply selector is valid but absent from form.html and times out as no_match before any input.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/read-eval-018.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "SDK page.click accepts CSS selectors. The original .apply selector is valid but absent from form.html and times out as no_match before any input.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/page.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "348-354",
          "claim": "page.locator takes selectors and getByRef explicitly translates a ref into aria-ref=. These are different APIs."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/page.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "391-393",
          "claim": "page.click accepts a string selector and forwards it to the main-frame click."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "817-833",
          "claim": "The selector string becomes target arguments sent with locator.click, rather than being rejected as a ref-domain violation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1115-1122",
          "claim": "A pointer-action timeout while nothing ever matched yields no_match."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/playwright_error.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "117-137",
          "claim": "no_match is script-owned; element_detached and navigation_failed are website-owned; invalid_input is script-owned."
        }
      ]
    },
    "postconditions": [
      "Open unchanged form.html. Use the original page.click(.apply,{timeout:800}).",
      "Catch exactly script/no_match with delivery not_sent. Require timeout/locator(.apply) message, rather than snapshot-ref guidance.",
      "Return the caught operation error from a successful run. Do not treat an uncaught legacy assertion error as pass."
    ]
  },
  {
    "id": "screenshot-scale-001",
    "file": "cases-screenshot-scale.ts",
    "name": "screenshot: default JPEG uses CSS pixels and device scale uses 2x pixels",
    "proves": "Native screenshot defaults to PNG at device density; at actual browser DPR2 both default and explicit device captures encode 2x CSS dimensions.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/screenshot-scale-001.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "density",
    "ruling": {
      "sdkBehavior": "Native screenshot defaults to PNG at device density; at actual browser DPR2 both default and explicit device captures encode 2x CSS dimensions.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/page/screenshot.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "139-147",
          "claim": "Native screenshot format defaults to PNG."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/page/screenshot_geometry.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "57-64",
          "claim": "Native screenshot density defaults to device scale; explicit scale: css selects CSS pixels."
        }
      ]
    },
    "postconditions": [
      "Use the existing isolated native device-scale-factor=2 browser launch and assert actual window.devicePixelRatio===2.",
      "Capture page.screenshot() and page.screenshot({scale:device}) from sdk-screenshot.html.",
      "For both buffers require PNG signature 89504e470d0a1a0a and IHDR width/height exactly innerWidth*2/innerHeight*2.",
      "Optional supplemental explicit {type:jpeg,scale:css} capture should have JPEG magic and dimensions exactly innerWidth/innerHeight if added to demonstrate the documented desktop equivalent."
    ],
    "hostSetup": {
      "kind": "device-scale",
      "deviceScaleFactor": 2
    }
  },
  {
    "id": "sdk-read-eval-001",
    "file": "cases-sdk-read-eval.ts",
    "name": "run: caps a large return value at 30000 bytes and saves the full result",
    "proves": "SDK uses independent 256 KiB JSON-value and 1 MiB console budgets, has no maxChars argument, and returns these exact payloads inline.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/output-value.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/reads/output-value-verify.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "SDK uses independent 256 KiB JSON-value and 1 MiB console budgets, has no maxChars argument, and returns these exact payloads inline.",
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
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-62",
          "claim": "Successful MCP replies carry structuredContent.value, failed replies structuredContent.error, with text emitted separately."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "86-102",
          "claim": "The first text line is JSON of the value; console lines follow a standalone console: marker and dropped lines are explicit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/runtime/fit_result.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-15",
          "claim": "Only branded byte values move to result blobs; text that fits stays untouched and text never spills automatically."
        }
      ]
    },
    "postconditions": [
      "Run 1 returns exactly R repeated 33000 times followed by SDK_CAP_VALUE_TAIL; no writtenToFile/path or string-spill equivalent is required.",
      "Run 2 receives run 1 value through args.prior[0] and compares the entire string before returning its exact length and tail=true.",
      "Run 1 emits no console output. The value alone crosses the historical desktop 30000-byte threshold.",
      "Unknown maxChars is not forwarded. The historical title remains unchanged; public schema citation documents the API change."
    ]
  },
  {
    "id": "sdk-read-eval-002",
    "file": "cases-sdk-read-eval.ts",
    "name": "run: maxChars 40000 keeps a large return value inline",
    "proves": "SDK uses independent 256 KiB JSON-value and 1 MiB console budgets, has no maxChars argument, and returns these exact payloads inline.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/output-value.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/reads/output-value-verify.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "SDK uses independent 256 KiB JSON-value and 1 MiB console budgets, has no maxChars argument, and returns these exact payloads inline.",
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
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-62",
          "claim": "Successful MCP replies carry structuredContent.value, failed replies structuredContent.error, with text emitted separately."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "86-102",
          "claim": "The first text line is JSON of the value; console lines follow a standalone console: marker and dropped lines are explicit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/runtime/fit_result.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-15",
          "claim": "Only branded byte values move to result blobs; text that fits stays untouched and text never spills automatically."
        }
      ]
    },
    "postconditions": [
      "Run 1 returns exactly R repeated 33000 times followed by SDK_CAP_VALUE_TAIL; no writtenToFile/path or string-spill equivalent is required.",
      "Run 2 receives run 1 value through args.prior[0] and compares the entire string before returning its exact length and tail=true.",
      "Run 1 emits no console output. The value alone crosses the historical desktop 30000-byte threshold.",
      "Unknown maxChars is not forwarded. The historical title remains unchanged; public schema citation documents the API change."
    ]
  },
  {
    "id": "sdk-read-eval-003",
    "file": "cases-sdk-read-eval.ts",
    "name": "run: caps the combined return value and logs at 30000 bytes and saves the full result",
    "proves": "SDK uses independent 256 KiB JSON-value and 1 MiB console budgets, has no maxChars argument, and returns these exact payloads inline.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/output-combined.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/reads/output-combined-verify.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "SDK uses independent 256 KiB JSON-value and 1 MiB console budgets, has no maxChars argument, and returns these exact payloads inline.",
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
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-62",
          "claim": "Successful MCP replies carry structuredContent.value, failed replies structuredContent.error, with text emitted separately."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "86-102",
          "claim": "The first text line is JSON of the value; console lines follow a standalone console: marker and dropped lines are explicit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/runtime/fit_result.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-15",
          "claim": "Only branded byte values move to result blobs; text that fits stays untouched and text never spills automatically."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/tests/desktop-fixtures/host-checks.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "370-419",
          "claim": "Host console checks parse the standalone console section, reject dropped-line notices and compare the exact same-run return value."
        }
      ]
    },
    "postconditions": [
      "Run 1 returns exactly R repeated 20000 times followed by SDK_CAP_VALUE_TAIL; no writtenToFile/path or string-spill equivalent is required.",
      "Run 2 receives run 1 value through args.prior[0] and compares the entire string before returning its exact length and tail=true.",
      "The same first raw MCP reply has an exact standalone console line L repeated 13000 times followed by SDK_CAP_LOG_TAIL. No console drop notice is permitted.",
      "Unknown maxChars is not forwarded. The historical title remains unchanged; public schema citation documents the API change."
    ],
    "hostCheck": {
      "kind": "run-log",
      "text": combinedLog,
      "value": combinedValue
    }
  },
  {
    "id": "sdk-read-eval-004",
    "file": "cases-sdk-read-eval.ts",
    "name": "run: maxChars 40000 keeps the combined return value and logs inline",
    "proves": "SDK uses independent 256 KiB JSON-value and 1 MiB console budgets, has no maxChars argument, and returns these exact payloads inline.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/output-combined.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/reads/output-combined-verify.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "SDK uses independent 256 KiB JSON-value and 1 MiB console budgets, has no maxChars argument, and returns these exact payloads inline.",
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
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-62",
          "claim": "Successful MCP replies carry structuredContent.value, failed replies structuredContent.error, with text emitted separately."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/mcp/run_tool.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "86-102",
          "claim": "The first text line is JSON of the value; console lines follow a standalone console: marker and dropped lines are explicit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/src/runtime/fit_result.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-15",
          "claim": "Only branded byte values move to result blobs; text that fits stays untouched and text never spills automatically."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/tests/desktop-fixtures/host-checks.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "370-419",
          "claim": "Host console checks parse the standalone console section, reject dropped-line notices and compare the exact same-run return value."
        }
      ]
    },
    "postconditions": [
      "Run 1 returns exactly R repeated 20000 times followed by SDK_CAP_VALUE_TAIL; no writtenToFile/path or string-spill equivalent is required.",
      "Run 2 receives run 1 value through args.prior[0] and compares the entire string before returning its exact length and tail=true.",
      "The same first raw MCP reply has an exact standalone console line L repeated 13000 times followed by SDK_CAP_LOG_TAIL. No console drop notice is permitted.",
      "Unknown maxChars is not forwarded. The historical title remains unchanged; public schema citation documents the API change."
    ],
    "hostCheck": {
      "kind": "run-log",
      "text": combinedLog,
      "value": combinedValue
    }
  },
  {
    "id": "snapshot-concurrency-003",
    "file": "cases-snapshot-concurrency.ts",
    "name": "snapshot concurrency: mixed frames keep stitch and ref order",
    "proves": "Mixed-frame snapshots stitch content in DOM order and retain each node ref, while reference namespaces are per document rather than a single global counter.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/snapshot-frame-order.txt"
      }
    ],
    "timeoutMs": 40000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Mixed-frame snapshots stitch content in DOM order and retain each node ref, while reference namespaces are per document rather than a single global counter.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/target.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "151-177",
          "claim": "Snapshot and minted refs identify the issuing document plus its local element number; a retired document is element_detached."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/target.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "209-218",
          "claim": "Main document refs print bare; other documents carry f<seq> prefixes. Numeric suffixes are not a global sibling ordering."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/snapshot/snapshot_op.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-22",
          "claim": "Each frame is read in its own document world, then merged by aria_snapshot::stitch; default mode is upstream and does not traverse iframes."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/snapshot-frame-tree.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "7-23",
          "claim": "The committed fixture reports readiness for parent, frame-a, frame-b and grandchild without any private service."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1443-1451,1470-1484",
          "claim": "The normal SDK no-mode text snapshot uses condensed v1 over the ai tree with refs; explicit mode default is the separate upstream YAML path."
        }
      ]
    },
    "postconditions": [
      "Build the mixed fixture URL from args.base and args.base2, switching the frame hostname between localhost and 127.0.0.1 to retain the original cross-origin condition.",
      "Wait for parent/frame-a/frame-b/grandchild readiness.",
      "Both settled default SDK v1 snapshots contain all seven original action/cursor labels in parent, frame A, grandchild, frame B order.",
      "Every label has a ref and the label-to-ref map is identical across captures. Treat each complete ref as opaque."
    ]
  },
  {
    "id": "snapshot-concurrency-006",
    "file": "cases-snapshot-concurrency.ts",
    "name": "snapshot concurrency: tabs new returns complete frame auto-context",
    "proves": "context.newPage and goto are explicit operations; a complete mixed-frame snapshot is explicitly obtained and returned by the script.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/snapshot-new-page-frames.txt"
      }
    ],
    "timeoutMs": 40000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "context.newPage and goto are explicit operations; a complete mixed-frame snapshot is explicitly obtained and returned by the script.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/context.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "173-175",
          "claim": "context.newPage resolves to a Page and does not return a snapshot context envelope."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "165-174",
          "claim": "newPage opens a separate tab in the current task; task ownership and tab enumeration are explicit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "186-188",
          "claim": "run returns exactly the script result plus console/notes, and page.evaluate is a main-world API."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/snapshot/snapshot_op.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-22",
          "claim": "Each frame is read in its own document world, then merged by aria_snapshot::stitch; default mode is upstream and does not traverse iframes."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/snapshot-frame-tree.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "7-23",
          "claim": "The committed fixture reports readiness for parent, frame-a, frame-b and grandchild without any private service."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1443-1451,1470-1484",
          "claim": "The normal SDK no-mode text snapshot uses condensed v1 over the ai tree with refs; explicit mode default is the separate upstream YAML path."
        }
      ]
    },
    "postconditions": [
      "Create a new Page, navigate to the mixed-origin fixture using only runtime fixture bases.",
      "Explicitly capture the created page. Require all seven original labels, a ref for each, and parent/frame A/grandchild/frame B stitch order.",
      "Return dynamic tabId, snapshot and refs; close the created Page in finally.",
      "Do not require an automatic desktop context envelope from newPage."
    ]
  },
  {
    "id": "tabs-001",
    "file": "cases-tabs.ts",
    "name": "tabs: new page opens with auto-context",
    "proves": "SDK newPage returns a Page; the script returns exactly its selected tab id and explicitly requested snapshot context.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/reads/tabs-new-context.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "SDK newPage returns a Page; the script returns exactly its selected tab id and explicitly requested snapshot context.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/context.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "173-175",
          "claim": "context.newPage resolves to a Page and does not return a snapshot context envelope."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "165-174",
          "claim": "newPage opens a separate tab in the current task; task ownership and tab enumeration are explicit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "186-188",
          "claim": "run returns exactly the script result plus console/notes, and page.evaluate is a main-world API."
        }
      ]
    },
    "postconditions": [
      "Create a new page with context.newPage and goto runtime fixture /links.html.",
      "Explicit snapshot includes Form page.",
      "Return the dynamic created tabId and that snapshot; close the Page in finally.",
      "No implicit Page snapshot text is promised by newPage or task metadata."
    ]
  }
] satisfies readonly RuledCaseSpec[]
