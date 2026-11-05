# SDK hooks

Working design. Scope is agreed; the HLD below is a proposal.

## Requirements

- Customers can customize SDK behavior with their own plugins.
- Support exactly two hook types: **PreToolUse** and **PostToolUse**.
- Hooks run automatically outside the agent's generated JavaScript. The agent does not need to remember to call them.
- Custom callable methods, remote Go handlers, and action-plan replies are out of scope for this design.

## HLD

### A plugin registers handlers

Use Pi's extension model as the reference: the host loads a plugin, passes it an SDK API, and the plugin registers event handlers.

Proposed shape, not a final API:

```javascript
export default function (sdk) {
  sdk.on("PreToolUse", async (event, context) => {
    // Inspect the upcoming call; optionally change or block it.
  });

  sdk.on("PostToolUse", async (event, context) => {
    // Inspect the result; optionally add feedback or change the result.
  });
}
```

This borrows Pi's registration model; it does not promise compatibility with existing Pi plugins or Node.js APIs.

### The SDK runs the hooks

```text
Tool call
   |
PreToolUse handlers
   |
Tool executes, unless blocked
   |
PostToolUse handlers
   |
Result returns to the caller
```

- Plugins are installed by the customer through trusted host setup.
- Before execution, the hook receives the tool name, input, and execution context.
- After execution, the hook receives the tool name, input, result, and error status.
- The SDK awaits handlers and applies their decisions in a defined order.
- A handler can return without changing anything. Hook implementation and configuration need not be added to the model's prompt; any returned feedback does consume context.

### First decision: what counts as a tool?

This is not settled.

- **Whole-run boundary:** hooks see one `neo.run` request and its final result.
- **Browser-operation boundary:** hooks see the individual operations inside the script, such as click or fill.

My recommendation is to start with browser operations, where a customer can inspect meaningful inputs and results without parsing JavaScript source. The user still needs to choose this boundary.

## Misc LLD points

- Plugin language, loading, and execution runtime inside the Rust sidecar.
- Plugin lifetime versus the fresh agent JavaScript context on each run.
- Exact event fields and handler return shapes.
- Whether PreToolUse may change input as well as block it.
- Which result fields PostToolUse may change, and how feedback reaches the caller.
- Ordering with multiple plugins and behavior when a plugin throws, times out, or is cancelled.
- Whether blocked calls reach PostToolUse; how tool errors are represented.
- What execution context plugins receive, and how nested operations from a hook behave.

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

Read in BrowserOS neo, including the diagrams and examples:
https://claude.ai/artifact/T9y3X6xonzmJBiwo2xHtBK
