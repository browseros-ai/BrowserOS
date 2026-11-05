# SDK hooks

Working design. Scope is agreed; the HLD below is a proposal.

## Requirements

- Customers can customize SDK behavior with their own plugins.
- Support exactly two hook types: **PreToolUse** and **PostToolUse**.
- Hooks run automatically outside the agent's generated JavaScript. The agent does not need to remember to call them.
- Customers choose which browser operations their handlers act on.
- Custom callable methods, remote Go handlers, and action-plan replies are out of scope for this design.

## HLD

### A plugin registers handlers

Use Pi's extension model as the reference: the host loads a plugin, passes it an SDK API, and the plugin registers event handlers.

Proposed shape, not a final API:

```javascript
export default function (sdk) {
  sdk.on("PreToolUse", async (event, context) => {
    if (event.method !== "locator.fill") return;
    // context.browser: this run's scoped browser object
    // context.page: the page receiving the fill
    // event.target: the locator receiving the fill
    // event.args: the arguments passed to fill
  });

  sdk.on("PostToolUse", async (event, context) => {
    if (event.method !== "locator.fill") return;
    // Inspect event.result or event.error.
  });
}
```

This borrows Pi's registration model; it does not promise compatibility with existing Pi plugins or Node.js APIs.

### The SDK runs the hooks

```text
JavaScript browser-operation call
   |
Shared dispatcher -> PreToolUse handlers
   |
Rust browser operation, unless blocked
   |
PostToolUse handlers
   |
Result returns to the caller
```

- Plugins are installed by the customer through trusted host setup.
- Each implemented browser operation passes through one shared dispatcher, which invokes both hook stages. Customers do not wrap each Playwright method themselves.
- Before execution, the hook receives the method, target object, arguments, and execution context.
- After execution, the hook also receives the result or error.
- The SDK awaits handlers and applies their decisions in a defined order.
- A handler can return without changing anything. Hook implementation and configuration need not be added to the model's prompt; any returned feedback does consume context.

### Working direction: hook browser operations inside a run

The customer wants to explore hooks around individual browser operations inside `neo.run`. The customer chooses relevant operations using normal JavaScript conditions in its handlers.

Proposed coverage is the operations in our implemented Playwright subset:

- Navigation and page lifecycle: `page.goto`, `page.reload`, `context.newPage`, `page.close`.
- Element and input actions: `locator.click`, `locator.fill`, checking, selecting, keyboard and mouse actions.
- Reads and captures: `page.title`, `locator.count`, text reads, screenshots, and page evaluation.
- Implemented browser/context operations, such as cookies and context lifecycle.

Hook one SDK operation, not every lower-level CDP request it uses.

Locator builders such as `getByRole`, `filter`, `first`, and `nth` remain synchronous. Proposed: they do not fire async hooks. Their eventual browser action does. Other synchronous accessors, such as `page.url()`, need an explicit coverage decision rather than silently becoming asynchronous.

### Proposed event and context

| Object | Fields | Meaning |
|---|---|---|
| Both events | `method`, `target`, `args` | Method name, the receiving Page/Locator/etc., and its arguments at this stage |
| PostToolUse adds | `result` or `error` | The operation's outcome |
| Context | `browser`, `page` | Scoped SDK browser and the page targeted by this operation |

`context.page` is the target page, not the foreground tab. It is optional for operations without a page, such as browser/context creation. A newly created page is available in the operation's result.

PostToolUse receives the arguments actually used after any pre-hook changes.

### Proposed handler returns

| Hook | Return | Effect |
|---|---|---|
| Either | nothing | Continue unchanged |
| PreToolUse | `{ args }` | Replace the operation's arguments |
| PreToolUse | `{ block: true, reason }` | Skip the operation and report why |
| PostToolUse | `{ result }` | Replace a successful result with a value of the same expected type |
| PostToolUse | `{ feedback }` | Add a note to the run's diagnostics without changing the method's return type |

The return shapes are proposals. Feedback should not turn a void-returning Playwright method into an object-returning method.

## Misc LLD points

- Plugin language, loading, and execution runtime inside the Rust sidecar.
- Plugin lifetime versus the fresh agent JavaScript context on each run.
- Finalize the proposed event fields and handler return shapes, including typed arguments for each method.
- Validate arguments after a pre-hook changes them and preserve Playwright result types after a post-hook changes a result.
- Decide how accumulated feedback reaches the caller and whether hooks can change an error outcome.
- Ordering with multiple plugins and behavior when a plugin throws, times out, or is cancelled.
- Whether blocked calls reach PostToolUse; how tool errors are represented.
- What execution context plugins receive, and how nested operations from a hook behave.
- Prevent a hook's browser calls from recursively invoking itself. Decide whether to suppress hooks for hook-origin calls or provide an explicit unhooked browser view.
- Clarify that hooks around `evaluate` or raw CDP see that entry call, not every action the submitted code performs inside Chromium.

## Design of individual components

- **Plugin loader:** loads the customer's configured plugins.
- **Hook registry:** stores PreToolUse and PostToolUse handlers.
- **Hook runner:** invokes handlers in order around tool execution.
- **Tool executor:** runs the actual operation and supplies its outcome.
- **Result handling:** applies post-hook changes before returning the result.

## Reference behavior

Pi extensions export a setup function that receives `ExtensionAPI` and register handlers with `pi.on`. Pi names the two relevant events `tool_call` and `tool_result`.

Its pre-call hook can block execution or change inputs. Its result handlers can return result patches and run in extension load order. These are reference behaviors to consider, not automatically final Neo semantics.

Read through Context7:
https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md

Playwright method behavior, checked through Context7:
https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/client/locator.ts
https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/client/page.ts

Read in BrowserOS neo, including the diagrams and examples:
https://claude.ai/artifact/T9y3X6xonzmJBiwo2xHtBK
