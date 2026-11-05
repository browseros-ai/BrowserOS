# Client-defined SDK methods

Working design. The requirements are agreed. The HLD contains alternatives, not a selected design.

## Requirements

- A customer can extend the SDK with methods backed by its own code.
- The customer keeps the implementation in its own service. For example, `zuzu.auth.fill` runs Go code inside mirrord.
- That implementation can use Neo's browser operations.

### Example: fill a login using the customer's vault

The agent knows which fields to fill, but does not have the credentials:

```javascript
await zuzu.auth.fill({
  page,
  fields: {
    username: page.getByLabel("Email"),
    password: page.getByLabel("Password"),
  },
});
```

The Go handler chooses the saved login and asks Neo to fill the fields. Credentials stay out of the agent's JavaScript and output. The method returns a status such as `filled` or `needs_owner`.

## HLD

The agent calls the added methods inside `neo.run`; this follows the existing code-mode design.

### Option A: call the customer's running service

- The customer tells Neo which methods its Go service handles.
- When the agent calls `zuzu.auth.fill`, Neo forwards the request to that service.
- Go checks the vault and asks Neo to perform the browser fill.
- Neo performs the fill. Go returns the result to the waiting agent call.

This lets Go coordinate multiple browser steps. It requires Neo to accept requests from Go while the original method call waits.

```text
Agent -> Neo -> Go service
               Go -> Neo -> Chromium
Agent <- Neo <- Go result
```

### Option B: hook-style request and reply

An alternative to explore, borrowing the external-handler pattern from Claude Code hooks:

- The customer supplies a method description and a trusted HTTP endpoint. A local executable is another option if it runs beside Neo.
- When the method is called, Neo sends a JSON request to the handler.
- For login, Go checks the vault and replies privately with the requested fill instructions.
- Neo performs the fill and produces the public result. Go does not call back into Neo.

```text
Agent -> Neo -> Go handler
         Neo <- private fill instructions
         Neo -> Chromium
Agent <- Neo result
```

This is simpler for a request that produces a known browser action. It is less flexible when Go needs to inspect the result of one browser step before choosing the next. That would need more exchanges or Option A.

The private reply may contain credentials. It must never become agent-visible hook output, logs, or trace arguments. Neo reports `filled` only after the browser operation completes; failures remain failures.

### What we borrow from hooks

Claude Code can invoke a command with JSON on stdin or POST JSON to an HTTP handler. The handler returns a defined reply. Event hooks can run only when relevant and may modify inputs or provide feedback.

That is an execution pattern, not automatic creation of new JavaScript methods. We still need to describe the name and signature of `zuzu.auth.fill` and expose it inside `neo.run`. Our private browser-action reply in Option B is a proposed Neo protocol, not a Claude Code feature.

Ordinary before/after hooks can also be useful. A hook around the outer `neo.run` call cannot replace a custom method awaited partway through the script; that requires an invocation point inside the SDK.

### What both options still need

- A description of each method, used to expose its JavaScript function and `.d.ts` declaration in fresh runs.
- Small references to the target tab and fields. A JavaScript `Page` object cannot be passed directly into Go.
- A clear separation between private handler data and the result shown to the agent.

## Misc LLD points

- Decide between a service callback, a hook-style request/reply, or both.
- Define the connection and message format. Local commands require colocated code; HTTP can reach the existing Go service.
- Which host/session may register and invoke a method.
- How page and field references are encoded, checked, and expired.
- How input/output shapes describe special values such as pages and locators.
- For Option B, define the allowed browser instructions and how Neo handles their success or failure.
- Decide which hooks run before/after a whole script and which run around individual operations.
- How nested calls share cancellation, deadlines, and trace IDs.
- What happens when Go disconnects or a browser action's outcome is unknown.

## Design of individual components

- **Method descriptions:** names, signatures, and where each method is handled.
- **JavaScript wrappers and declarations:** expose those methods inside fresh runs.
- **Option A handler connection:** forwards calls to Go and accepts browser requests back.
- **Option B handler runner:** sends one request to a trusted command or HTTP endpoint and reads its reply.
- **Protected browser operations:** perform the requested actions and expose only safe results to the agent.

## References

- Parent design: [Claw SDK working design](claw-sdk-design.md).
- Reference document, section 3 and the login sequence:
  https://docs.shadowfax92.workers.dev/chVZOHAd
- Claude Code hooks reference, including command/HTTP input and output:
  https://code.claude.com/docs/en/hooks
- Claude Code hooks guide. Command hooks do not themselves trigger tool calls; the browser-action reply above is our proposed extension:
  https://code.claude.com/docs/en/hooks-guide
- The user supplied a summary of Daisy Hollman's talk. The hook protocol details above were checked against the current official documentation:
  https://www.youtube.com/watch?v=shZgedW15vg&t=2689
