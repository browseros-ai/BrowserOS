import type { RuledCaseSpec } from '../model'

/** SDK rulings preserve original identities while asserting current public behavior. */
// biome-ignore format: Keep the complete pinned rule records together.
export const interactionsCases = [
  {
    "id": "act-009",
    "file": "cases-act.ts",
    "name": "act: press Shift+a types an uppercase A",
    "proves": "Pressing literal Shift+a on the Name ref types lowercase a and resolves without a value.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/act-009.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Pressing literal Shift+a on the Name ref types lowercase a and resolves without a value.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/input/keyboard.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "140-150",
          "claim": "Literal a keeps its lowercase text under Shift, while KeyA can use a shifted description."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "188-188",
          "claim": "The SDK documents Playwright 1.64.1 compatibility with explicitly listed Neo differences."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/VERSION",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-2",
          "claim": "The vendored engine is pinned to Playwright 1.64.1 at cfe490d941f992655948eee8fd4fd94e1c4234bf."
        }
      ]
    },
    "postconditions": [
      "#name.value is exactly 'a' and contains no uppercase A."
    ]
  },
  {
    "id": "act-010",
    "file": "cases-act.ts",
    "name": "act: press resolves the Cmd+c alias",
    "proves": "Cmd+c rejects with code invalid_input, owner script, and delivery not_sent, leaving the Name field empty.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/act-010.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Cmd+c rejects with code invalid_input, owner script, and delivery not_sent, leaving the Name field empty.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/input/keyboard.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "171-178",
          "claim": "The keyboard aliases include Meta but omit Cmd."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/input/keyboard.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "435-447",
          "claim": "Only ControlOrMeta receives platform translation before key lookup."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/key_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "178-184",
          "claim": "Unknown keys fail as invalid_input before send_effect is called."
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
        }
      ]
    },
    "postconditions": [
      "#name.value remains its initial empty string.",
      "No key burst is sent, and no browser clipboard or host alias is consulted."
    ]
  },
  {
    "id": "act-014",
    "file": "cases-act.ts",
    "name": "act: focus by ref rejects an element that cannot receive focus",
    "proves": "Focusing the connected disabled button resolves without a value and retains focus on Name.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/act-014.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Focusing the connected disabled button resolves without a value and retains focus on Name.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/element_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-3",
          "claim": "Focus waits only for attachment."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/element_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "69-124",
          "claim": "Element edits hold the tab's input lock through the sent edit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "57-58",
          "claim": "The focus probe calls injected.focusNode."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "963-988",
          "claim": "The connected-element focus path calls native focus and returns done."
        }
      ]
    },
    "postconditions": [
      "The target remains connected and disabled.",
      "Native focus does not activate the disabled button.",
      "For a deterministic retained-focus assertion, establish #name as active first; after disabled focus #name remains active."
    ]
  },
  {
    "id": "fill-003",
    "file": "cases-fill.ts",
    "name": "act: fill leaves what a rejecting field keeps to the page",
    "proves": "Fill resolves without a value when the rejecting field cancels beforeinput, and explicit reads still show original.",
    "steps": [
      {
        "kind": "script",
        "script": "scripts/g1c/rejecting.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/fill-return-void.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Fill resolves without a value when the rejecting field cancels beforeinput, and explicit reads still show original.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "Rejecting field starts with value original, and Other field starts with untouched."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The Rejecting field's beforeinput handler prevents the edit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
        }
      ]
    },
    "postconditions": [
      "#reject remains original; #other remains untouched.",
      "The original pinned ref inputValue is original.",
      "An explicitly requested AI snapshot line for Rejecting field reports original."
    ]
  },
  {
    "id": "fill-004",
    "file": "cases-fill.ts",
    "name": "act: fill leaves what a reverting field keeps to the page",
    "proves": "Fill resolves without a value while the field's microtask restores original before explicit readback.",
    "steps": [
      {
        "kind": "script",
        "script": "scripts/g1c/reverting.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/fill-return-void.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Fill resolves without a value while the field's microtask restores original before explicit readback.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "Reverting field starts with value original, and Other field starts with untouched."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The Reverting field's input handler restores original in a microtask."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
        }
      ]
    },
    "postconditions": [
      "The page's input-handler microtask restores #revert to original before readback; #other remains untouched.",
      "The original pinned ref inputValue and explicit AI snapshot field line both report original."
    ]
  },
  {
    "id": "fill-005",
    "file": "cases-fill.ts",
    "name": "act: fill leaves a value the page reformats to the page",
    "proves": "Fill resolves without a value while the phone input handler reformats 5551234567 as (555) 123-4567.",
    "steps": [
      {
        "kind": "script",
        "script": "scripts/g1c/reformat.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/fill-return-void.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Fill resolves without a value while the phone input handler reformats 5551234567 as (555) 123-4567.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "The fixture defines Phone field as a tel input."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The phone input handler strips non-digits and formats a ten-digit phone number."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
        }
      ]
    },
    "postconditions": [
      "#phone.value is (555) 123-4567.",
      "The same pinned ref inputValue and explicit AI snapshot line show the formatted value."
    ]
  },
  {
    "id": "fill-008",
    "file": "cases-fill.ts",
    "name": "act: fill rejects covered field without false success",
    "proves": "Fill accepts the covered field without force, resolves without a value, and leaves its cover in place.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/covered.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Fill accepts the covered field without force, resolves without a value, and leaves its cover in place.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "Covered field is followed by a cover element, and Other field starts with untouched."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The fixture records each input event's target ID and isTrusted flag."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
        }
      ]
    },
    "postconditions": [
      "No fill failure occurred.",
      "#covered.value is replacement and #other.value is untouched.",
      "The original cover stays in place. No bypass click or force option is added."
    ]
  },
  {
    "id": "fill-011",
    "file": "cases-fill.ts",
    "name": "act: fill reports a field the page removed as gone, not empty",
    "proves": "An empty fill resolves without a value after the page removes the field. Reading the original ref reports element_detached.",
    "steps": [
      {
        "kind": "script",
        "script": "scripts/g1c/removed.txt"
      },
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/fill-return-void.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "An empty fill resolves without a value after the page removes the field. Reading the original ref reports element_detached.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "Detached field starts with value original."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The Detached field's input handler removes its target."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
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
        }
      ]
    },
    "postconditions": [
      "#detach no longer exists.",
      "inputValue on the original pinned ref rejects as element_detached.",
      "Current snapshot has no Detached field line."
    ]
  },
  {
    "id": "fill-021",
    "file": "cases-fill.ts",
    "name": "run: fill rejects covered field without false success",
    "proves": "Fill accepts the covered field without force, resolves without a value, and leaves its cover in place.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/covered.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Fill accepts the covered field without force, resolves without a value, and leaves its cover in place.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "Covered field is followed by a cover element, and Other field starts with untouched."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The fixture records each input event's target ID and isTrusted flag."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
        }
      ]
    },
    "postconditions": [
      "No fill failure occurred.",
      "#covered.value is replacement and #other.value is untouched.",
      "The original cover stays in place. No bypass click or force option is added."
    ]
  },
  {
    "id": "fill-034",
    "file": "cases-fill.ts",
    "name": "run-legacy: fill rejects covered field without false success",
    "proves": "Fill accepts the covered field without force, resolves without a value, and leaves its cover in place.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/covered.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Fill accepts the covered field without force, resolves without a value, and leaves its cover in place.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "Covered field is followed by a cover element, and Other field starts with untouched."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The fixture records each input event's target ID and isTrusted flag."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
        }
      ]
    },
    "postconditions": [
      "No fill failure occurred.",
      "#covered.value is replacement and #other.value is untouched.",
      "The original cover stays in place. No bypass click or force option is added."
    ]
  },
  {
    "id": "fill-readback-001",
    "file": "cases-fill-readback.ts",
    "name": "act: a multi-field fill carries on past a field the page reformats",
    "proves": "Both serial fills resolve without values. Explicit readback shows the formatted phone value and Ada in the second field.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/readback-multiple.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Both serial fills resolve without values. Explicit readback shows the formatted phone value and Ada in the second field.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "The fixture defines Phone field as a tel input."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The phone input handler strips non-digits and formats a ten-digit phone number."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "129-140",
          "claim": "The delta snapshot option applies only to whole-page v1 text."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1443-1466",
          "claim": "AI snapshots render v1 text with an optional comparison against the stored baseline."
        }
      ]
    },
    "postconditions": [
      "Phone is (555) 123-4567 and Text field is Ada.",
      "An explicitly requested AI snapshot contains both final values under their field names.",
      "The second fill executes despite formatting of the first."
    ]
  },
  {
    "id": "fill-readback-002",
    "file": "cases-fill-readback.ts",
    "name": "act: the fields readback stays when the diff is off",
    "proves": "Fill resolves without a value, and an explicit snapshot with delta disabled shows the formatted verification code 440 212.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/readback-no-diff.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Fill resolves without a value, and an explicit snapshot with delta disabled shows the formatted verification code 440 212.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "Verification code has numeric inputmode."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The code input handler inserts a space after the first three digits."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "129-140",
          "claim": "The delta snapshot option applies only to whole-page v1 text."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1443-1466",
          "claim": "AI snapshots render v1 text with an optional comparison against the stored baseline."
        }
      ]
    },
    "postconditions": [
      "#code.value is 440 212.",
      "The full explicit snapshot field line reports 440 212.",
      "No desktop diff block is present."
    ]
  },
  {
    "id": "fill-readback-003",
    "file": "cases-fill-readback.ts",
    "name": "act: a fill that stops partway still shows every field",
    "proves": "A read-only fill fails before delivery and stops the serial sequence after Text field. Explicit readback still shows all three fields.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/readback-partial.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "The read-only fill throws not_actionable with owner website and delivery not_sent. The caller skips the third fill and explicitly reads all fields.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "74-87",
          "claim": "The fill actionability row lists visible, enabled, and editable states."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/form_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "408-448",
          "claim": "Text fills hold the input lock through the probe edit and text insertion."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "Read only starts with original, and Other field starts with untouched."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The fixture records each input event's target ID and isTrusted flag."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "129-140",
          "claim": "The delta snapshot option applies only to whole-page v1 text."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1443-1466",
          "claim": "AI snapshots render v1 text with an optional comparison against the stored baseline."
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
        }
      ]
    },
    "postconditions": [
      "Attempted labels are exactly ['Text field','Read only'].",
      "Final values are {text:'typed first',readonly:'original',other:'untouched'}.",
      "After catching the refusal, explicitly read all three corresponding snapshot field values.",
      "Normal run metadata is completed, with unknown empty."
    ]
  },
  {
    "id": "fill-readback-004",
    "file": "cases-fill-readback.ts",
    "name": "act: after a fill navigates, its reused ref reads as gone, not as the new page",
    "proves": "The navigating fill resolves without a value. Its original ref reports element_detached, while a distinct fresh ref reads the new document's empty field.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/readback-navigation.txt"
      }
    ],
    "timeoutMs": 30000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "The navigating fill resolves without a value. Its original ref reports element_detached, while a distinct fresh ref reads the new document's empty field.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "281-289",
          "claim": "Locator.fill delegates to the frame with strict: true and returns Promise<void>."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "935-954",
          "claim": "The frame awaits locator.fill and returns no readback value."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "20-33",
          "claim": "The fixture defines the Navigating field input."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "35-61",
          "claim": "The Navigating field's input handler loads /fill.html?navigated."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "129-140",
          "claim": "The delta snapshot option applies only to whole-page v1 text."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1443-1466",
          "claim": "AI snapshots render v1 text with an optional comparison against the stored baseline."
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
        }
      ]
    },
    "postconditions": [
      "The new document contains an empty Navigating field.",
      "The new ref differs from the original and reads empty.",
      "The original ref throws element_detached, and the new snapshot contains no original ref.",
      "Explicit snapshot readback describes the fresh empty field."
    ]
  },
  {
    "id": "sdk-input-010",
    "file": "cases-sdk-input.ts",
    "name": "act: check succeeds when the page re-renders the checkbox it clicked",
    "proves": "Check throws element_detached with owner website and delivery done after the page replaces the original checkbox with a checked node.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-check-rerender.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Check throws element_detached with owner website and delivery done after the page replaces the original checkbox with a checked node.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "325-375",
          "claim": "Check verifies the same pin after clicking and reports element_detached if that verification finds no node."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "432-459",
          "claim": "Checkable reads use ProbeTarget::Pin and map detached replies to None."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/docs/src/api/class-locator.md#L370-L388",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "370-388",
          "claim": "Upstream check verifies the final checked state and throws if the element detaches during the action."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-input.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "27-37",
          "claim": "The change handler replaces the checkbox with a checked node."
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
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "188-188",
          "claim": "The SDK documents Playwright 1.64.1 compatibility with explicitly listed Neo differences."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/VERSION",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-2",
          "claim": "The vendored engine is pinned to Playwright 1.64.1 at cfe490d941f992655948eee8fd4fd94e1c4234bf."
        }
      ]
    },
    "postconditions": [
      "The replacement .rerendered checkbox is checked true after the delivered click.",
      "The original pinned node was detached; no second check is sent.",
      "Normal run effects are done and unknown is empty after catching the error."
    ]
  },
  {
    "id": "sdk-input-011",
    "file": "cases-sdk-input.ts",
    "name": "act: check succeeds when the checkbox it clicked navigates the page",
    "proves": "Check throws element_detached with owner website and delivery done after the checkbox's change handler navigates to /links.html.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-check-navigate.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Check throws element_detached with owner website and delivery done after the checkbox's change handler navigates to /links.html.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "325-375",
          "claim": "Check verifies the same pin after clicking and reports element_detached if that verification finds no node."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "432-459",
          "claim": "Checkable reads use ProbeTarget::Pin and map detached replies to None."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/docs/src/api/class-locator.md#L370-L388",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "370-388",
          "claim": "Upstream check verifies the final checked state and throws if the element detaches during the action."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-input.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "27-37",
          "claim": "The facet checkbox's change handler navigates to /links.html."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
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
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "188-188",
          "claim": "The SDK documents Playwright 1.64.1 compatibility with explicitly listed Neo differences."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/VERSION",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-2",
          "claim": "The vendored engine is pinned to Playwright 1.64.1 at cfe490d941f992655948eee8fd4fd94e1c4234bf."
        }
      ]
    },
    "postconditions": [
      "The click's change handler navigates to pathname /links.html.",
      "The original checkbox document is gone, so same-pin final verification throws.",
      "No repeat click is delivered; normal run effects are done and unknown is empty."
    ]
  },
  {
    "id": "sdk-input-019",
    "file": "cases-sdk-input.ts",
    "name": "act: fill accepts datetime-local 2026-10-29T14:30:00, which the browser stores as 2026-10-29T14:30",
    "proves": "The exact datetime string 2026-10-29T14:30:00 is stored as 2026-10-29T14:30 before fill rejects, with no input or change events.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-fill-normalized-datetime.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Native fill throws Malformed value with code invalid_input, owner script, and delivery done after the browser changes the field.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-8",
          "claim": "Rust marks the write sent before dispatching the pinned element edit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "39-47",
          "claim": "The fill probe calls injected.fill and passes its result through keptFocus."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "899-918",
          "claim": "Native-value fills reject unequal stored values before dispatching input and change events."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-49",
          "claim": "The engine bundle builds upstream modules unmodified, with inspector and devtools helpers replaced by stubs."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/tests/page/page-fill.spec.ts#L54-L67",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "54-67",
          "claim": "The pinned upstream range tests expect Malformed value for invalid, above-bound, and fractional values."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "9-23",
          "claim": "Datetime field is a datetime-local input, and the fixture records input and change events."
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
        }
      ]
    },
    "postconditions": [
      "The browser-stored value is 2026-10-29T14:30 after rejection.",
      "window.nativeEvents is exactly empty.",
      "Normal run effects are done and unknown is empty."
    ]
  },
  {
    "id": "sdk-input-020",
    "file": "cases-sdk-input.ts",
    "name": "act: fill accepts range 75.0, which the browser stores as 75",
    "proves": "The exact range string 75.0 is stored as 75 before fill rejects, with no input or change events.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-fill-normalized-range.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Native fill throws Malformed value with code invalid_input, owner script, and delivery done after the browser changes the field.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-8",
          "claim": "Rust marks the write sent before dispatching the pinned element edit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "39-47",
          "claim": "The fill probe calls injected.fill and passes its result through keptFocus."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "899-918",
          "claim": "Native-value fills reject unequal stored values before dispatching input and change events."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-49",
          "claim": "The engine bundle builds upstream modules unmodified, with inspector and devtools helpers replaced by stubs."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/tests/page/page-fill.spec.ts#L54-L67",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "54-67",
          "claim": "The pinned upstream range tests expect Malformed value for invalid, above-bound, and fractional values."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "9-23",
          "claim": "Range field spans 0 to 100, and the fixture records input and change events."
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
        }
      ]
    },
    "postconditions": [
      "The browser-stored range value is 75 after rejection.",
      "window.nativeEvents is exactly empty.",
      "Normal run effects are done and unknown is empty."
    ]
  },
  {
    "id": "sdk-input-021",
    "file": "cases-sdk-input.ts",
    "name": "act: fill clamps a range value outside its bounds and reads back the bound",
    "proves": "Filling the range field with 150 stores its upper bound 100 before rejecting, with no input or change events.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-fill-clamped-range.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Native fill throws Malformed value with code invalid_input, owner script, and delivery done after the browser changes the field.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-8",
          "claim": "Rust marks the write sent before dispatching the pinned element edit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "39-47",
          "claim": "The fill probe calls injected.fill and passes its result through keptFocus."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "899-918",
          "claim": "Native-value fills reject unequal stored values before dispatching input and change events."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-49",
          "claim": "The engine bundle builds upstream modules unmodified, with inspector and devtools helpers replaced by stubs."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/tests/page/page-fill.spec.ts#L54-L67",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "54-67",
          "claim": "The pinned upstream range tests expect Malformed value for invalid, above-bound, and fractional values."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "9-23",
          "claim": "Range field spans 0 to 100, and the fixture records input and change events."
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
        }
      ]
    },
    "postconditions": [
      "The browser-stored range value is 100 after rejection.",
      "window.nativeEvents is exactly empty.",
      "Normal run effects are done and unknown is empty."
    ]
  },
  {
    "id": "sdk-input-033",
    "file": "cases-sdk-input.ts",
    "name": "run: check succeeds when the page re-renders the checkbox it clicked",
    "proves": "Check throws element_detached with owner website and delivery done after the page replaces the original checkbox with a checked node.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-check-rerender.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Check throws element_detached with owner website and delivery done after the page replaces the original checkbox with a checked node.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "325-375",
          "claim": "Check verifies the same pin after clicking and reports element_detached if that verification finds no node."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "432-459",
          "claim": "Checkable reads use ProbeTarget::Pin and map detached replies to None."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/docs/src/api/class-locator.md#L370-L388",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "370-388",
          "claim": "Upstream check verifies the final checked state and throws if the element detaches during the action."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-input.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "27-37",
          "claim": "The change handler replaces the checkbox with a checked node."
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
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "188-188",
          "claim": "The SDK documents Playwright 1.64.1 compatibility with explicitly listed Neo differences."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/VERSION",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-2",
          "claim": "The vendored engine is pinned to Playwright 1.64.1 at cfe490d941f992655948eee8fd4fd94e1c4234bf."
        }
      ]
    },
    "postconditions": [
      "The replacement .rerendered checkbox is checked true after the delivered click.",
      "The original pinned node was detached; no second check is sent.",
      "Normal run effects are done and unknown is empty after catching the error."
    ]
  },
  {
    "id": "sdk-input-034",
    "file": "cases-sdk-input.ts",
    "name": "run: check succeeds when the checkbox it clicked navigates the page",
    "proves": "Check throws element_detached with owner website and delivery done after the checkbox's change handler navigates to /links.html.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-check-navigate.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Check throws element_detached with owner website and delivery done after the checkbox's change handler navigates to /links.html.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "325-375",
          "claim": "Check verifies the same pin after clicking and reports element_detached if that verification finds no node."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/action/pointer_action.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "432-459",
          "claim": "Checkable reads use ProbeTarget::Pin and map detached replies to None."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/docs/src/api/class-locator.md#L370-L388",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "370-388",
          "claim": "Upstream check verifies the final checked state and throws if the element detaches during the action."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-input.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "27-37",
          "claim": "The facet checkbox's change handler navigates to /links.html."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/agent.d.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "38-43",
          "claim": "Locator refs bind to one node and become stale after removal or navigation."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/src/world/frame_worlds.rs",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "100-125",
          "claim": "Each new document gets a unique sequence, so an old ref cannot resolve in its replacement."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "462-467",
          "claim": "Locator.ref requests a strict node-bound ref, including nodes without size."
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
          "path": "packages/browseros-agent/crates/neo-sdk/neo-server/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "188-188",
          "claim": "The SDK documents Playwright 1.64.1 compatibility with explicitly listed Neo differences."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/VERSION",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-2",
          "claim": "The vendored engine is pinned to Playwright 1.64.1 at cfe490d941f992655948eee8fd4fd94e1c4234bf."
        }
      ]
    },
    "postconditions": [
      "The click's change handler navigates to pathname /links.html.",
      "The original checkbox document is gone, so same-pin final verification throws.",
      "No repeat click is delivered; normal run effects are done and unknown is empty."
    ]
  },
  {
    "id": "sdk-input-042",
    "file": "cases-sdk-input.ts",
    "name": "run: fill accepts datetime-local 2026-10-29T14:30:00, which the browser stores as 2026-10-29T14:30",
    "proves": "The exact datetime string 2026-10-29T14:30:00 is stored as 2026-10-29T14:30 before fill rejects, with no input or change events.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-fill-normalized-datetime.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Native fill throws Malformed value with code invalid_input, owner script, and delivery done after the browser changes the field.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-8",
          "claim": "Rust marks the write sent before dispatching the pinned element edit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "39-47",
          "claim": "The fill probe calls injected.fill and passes its result through keptFocus."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "899-918",
          "claim": "Native-value fills reject unequal stored values before dispatching input and change events."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-49",
          "claim": "The engine bundle builds upstream modules unmodified, with inspector and devtools helpers replaced by stubs."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/tests/page/page-fill.spec.ts#L54-L67",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "54-67",
          "claim": "The pinned upstream range tests expect Malformed value for invalid, above-bound, and fractional values."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "9-23",
          "claim": "Datetime field is a datetime-local input, and the fixture records input and change events."
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
        }
      ]
    },
    "postconditions": [
      "The browser-stored value is 2026-10-29T14:30 after rejection.",
      "window.nativeEvents is exactly empty.",
      "Normal run effects are done and unknown is empty."
    ]
  },
  {
    "id": "sdk-input-043",
    "file": "cases-sdk-input.ts",
    "name": "run: fill accepts range 75.0, which the browser stores as 75",
    "proves": "The exact range string 75.0 is stored as 75 before fill rejects, with no input or change events.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-fill-normalized-range.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Native fill throws Malformed value with code invalid_input, owner script, and delivery done after the browser changes the field.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-8",
          "claim": "Rust marks the write sent before dispatching the pinned element edit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "39-47",
          "claim": "The fill probe calls injected.fill and passes its result through keptFocus."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "899-918",
          "claim": "Native-value fills reject unequal stored values before dispatching input and change events."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-49",
          "claim": "The engine bundle builds upstream modules unmodified, with inspector and devtools helpers replaced by stubs."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/tests/page/page-fill.spec.ts#L54-L67",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "54-67",
          "claim": "The pinned upstream range tests expect Malformed value for invalid, above-bound, and fractional values."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "9-23",
          "claim": "Range field spans 0 to 100, and the fixture records input and change events."
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
        }
      ]
    },
    "postconditions": [
      "The browser-stored range value is 75 after rejection.",
      "window.nativeEvents is exactly empty.",
      "Normal run effects are done and unknown is empty."
    ]
  },
  {
    "id": "sdk-input-044",
    "file": "cases-sdk-input.ts",
    "name": "run: fill clamps a range value outside its bounds and reads back the bound",
    "proves": "Filling the range field with 150 stores its upper bound 100 before rejecting, with no input or change events.",
    "steps": [
      {
        "kind": "script",
        "script": "ruled/bodies/interactions/input-fill-clamped-range.txt"
      }
    ],
    "timeoutMs": 20000,
    "expectation": {
      "kind": "pass"
    },
    "execution": "ordinary",
    "ruling": {
      "sdkBehavior": "Native fill throws Malformed value with code invalid_input, owner script, and delivery done after the browser changes the field.",
      "citations": [
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "1-8",
          "claim": "Rust marks the write sent before dispatching the pinned element edit."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright-js/src/probe/edit.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "39-47",
          "claim": "The fill probe calls injected.fill and passes its result through keptFocus."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "899-918",
          "claim": "Native-value fills reject unequal stored values before dispatching input and change events."
        },
        {
          "path": "packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/README.md",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "45-49",
          "claim": "The engine bundle builds upstream modules unmodified, with inspector and devtools helpers replaced by stubs."
        },
        {
          "path": "https://github.com/microsoft/playwright/blob/cfe490d941f992655948eee8fd4fd94e1c4234bf/tests/page/page-fill.spec.ts#L54-L67",
          "commit": "cfe490d941f992655948eee8fd4fd94e1c4234bf",
          "lines": "54-67",
          "claim": "The pinned upstream range tests expect Malformed value for invalid, above-bound, and fractional values."
        },
        {
          "path": "packages/browseros-agent/contracts/claw-mcp/fixtures/pages/sdk-fill.html",
          "commit": "b3cdccffba5e067807c41ab567325b608ce931dd",
          "lines": "9-23",
          "claim": "Range field spans 0 to 100, and the fixture records input and change events."
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
        }
      ]
    },
    "postconditions": [
      "The browser-stored range value is 100 after rejection.",
      "window.nativeEvents is exactly empty.",
      "Normal run effects are done and unknown is empty."
    ]
  }
] satisfies readonly RuledCaseSpec[]
