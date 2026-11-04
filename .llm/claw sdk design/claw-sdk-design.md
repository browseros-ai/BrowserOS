# Claw SDK — working design

Updated: 2026-11-04. Branch: `feat/claw-sdk-design`.

These are living notes from our discussion. Agreed points are recorded below; open questions and outside ideas are marked separately.

## Requirements

- **Clear boundary.** Distinguish BrowserOS execution failures from failures in the customer's agent or code.
- **Better browser controls.** Provide capable, reliable browser controls for customer agents.
- **Design for advanced capabilities.** Account for Jev-like APIs and reusable skills with code mode while designing the foundation. Their details and V1 scope remain open.

## HLD

### Agreed direction

- Start a new SDK codebase and a new binary. Implement browser operations in Rust, using the existing tool logic as a reference.
- `claw-app` and `claw-server` continue independently and can change as needed. Keeping their code frozen is not an SDK requirement.
- The Rust browser layer implements navigation, finding elements, clicking, filling, and reading pages. It talks to Chromium through CDP, Chromium's browser-control protocol.
- The customer owns the agent loop and adds one agent-facing tool: `neo.run`. It accepts JavaScript that can combine browser actions, loops, conditions, and data processing.
- The customer calls `neo.run`; an embedded JavaScript runtime invokes Rust browser operations; those operations control BrowserOS Chromium.
- Support a large subset of Playwright, including the supported functions' expected behavior.
- JavaScript is the agent-facing API. Rust is the browser-operation implementation. `.d.ts` files describe the callable API; they do not implement it.
- Expose capabilities outside Playwright through the same code mode. Publish `.d.ts` definitions and teach the agent how to call them.
- Provide REST endpoints for screencasts, screenshots, recordings, tool traces, and audit traces. Customer applications can use them independently of the agent's tool list.
- Design the browser layer so Jev-like operations and reusable skills can build on it.

### Still to decide

- The exact boundary between the Rust library, its binary, and thin Go/JavaScript clients.
- One-shot JavaScript execution, state retained between calls, and how callers resume browser sessions.
- Which CAPTCHA, secure form filling, takeover, and proxy features ship first.
- Hooks and customer-defined extensions were discussed; their V1 scope is not settled.

## Misc LLD points

No detailed API or runtime implementation has been agreed yet.

- Choose the Playwright subset, including locators, waits, frames, pages, and network operations.
- Define session identity, page handles, concurrent calls, timeouts, cancellation, and retry behavior.
- Define the extra BrowserOS methods and generate matching `.d.ts` declarations.
- Work through one concrete Jev operation and one reusable skill before choosing their interfaces.
- Resolve secure filling without exposing credentials to agent-written JavaScript.

## Design of individual components

### neo.run design

#### Agreed

- The customer adds one agent-facing tool, `neo.run`, which accepts JavaScript.
- The script can combine browser actions, loops, conditions, and data processing.
- The runtime exposes a large subset of Playwright plus additional BrowserOS methods described by `.d.ts` files.
- Browser operations are implemented in Rust and control Chromium through CDP.

#### Next to design

- Follow one call from the customer's agent through JavaScript execution and Rust operations to its result.
- Define the request, returned value, captured output, errors, and links to execution traces.
- Choose the JavaScript engine, JavaScript-to-Rust binding, and state retained between calls.

## Inspiration

- **Ego:** complete code blocks can move loops and decisions into the runtime and reduce model/tool round trips. Its one-shot execution results support exploring that model; they do not settle our runtime design.
  https://lite.ego.app/blog/heredoc-vs-repl-ai-agents-2026
- **Aside:** use familiar Playwright methods and compact browser observations. It implements its own browser layer over CDP and uses a REPL. Familiar API design and execution lifetime are separate choices.
  https://aside.com/blog/how-we-built-the-sota-browser-agent-that-outperforms-fable


### Pomerado: research, not agreed SDK design

- **Build once, call again.** An AI agent explores a website, writes a task function, and tests it. Guardian reviews the work before publication. Later calls run saved code without another model request. Source: `pomerado-source/typescript/src/standalone/run-operation.ts:66`.
- **Give each function a contract.** A saved operation declares its name, inputs, outputs, and code. The runtime checks input and output shapes. This does not prove the answer is correct. Sources: `pomerado-source/typescript/authoring/examples/native-page.ts:6`, `pomerado-source/typescript/src/runtime/kernel-operation-run.ts:99`.
- **Browser and HTTP paths.** Saved operations use Playwright; the shared core also supports HTTP implementations. HTTP recording, automatic HTTP versions, and repair are listed as hosted features that the local host lacks. Sources: `pomerado-source/typescript/src/runtime/http-operation.ts:15`, `pomerado-source/typescript/src/hosted-features.ts:15`.
- **Possible SDK fit.** Reusable website functions could sit above our Playwright operations, called inside `neo.run` and described by `.d.ts`. This keeps the one-tool integration. This is a proposal, not an agreed API.
- **Keep skills and functions distinct.** A skill teaches the agent a workflow; a saved function executes it. Turning a skill into a function needs parameters, tests, and a result contract. That process remains open for our SDK.

Website: https://pomerado.ai/

Code inspected: https://github.com/Pomerado/pomerado/tree/194eb6c3ebe882cede2d37cdb9f4ff80cd4b5ed7

Source paths above are relative to `.llm/`. Cloud repair and performance claims were not verified by reading the public core.

## Discussion checks

- A single `run` tool does not identify who caused a failure. Caller code, a website, the runtime, or Chromium can fail within the same call. The error contract is still to design.
- The current Rust implementation already embeds QuickJS, but that is a reference, not a choice for the new SDK.
- The current API's lack of Playwright locators does not limit the new SDK. Supporting them is part of the new work.

## Sources

- User's transcript and follow-up corrections in this conversation.
- `/Users/felarof01/Downloads/IMG_8547.HEIC`: requirements and SDK sketch.
- `/Users/felarof01/Downloads/IMG_8548.HEIC`: code mode, browser operations, API endpoints, Jev, and skill-building sketch.
- Existing code, relative to the repository root:
  - `packages/browseros-agent/crates/browseros-mcp/src/tools/run.rs:201`: unsupported Playwright methods in the current API.
  - `packages/browseros-agent/crates/browseros-mcp/src/tools/run.rs:798`: current embedded QuickJS execution.
  - `packages/browseros-agent/apps/claw-server/src/api/hosted/mod.rs:1`: existing hosted session and recording API.
