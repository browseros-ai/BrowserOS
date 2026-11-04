# Client-defined SDK methods

Working design. The requirement is agreed; the HLD below is a proposal for discussion. Transport and other low-level choices remain open.

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

### 1. Registration: tell Neo where calls should go

There are two running programs: Neo's Rust sidecar and the customer's Go service.

The Go service already contains a function, say `FillLogin`. At startup, it registers the public name `zuzu.auth.fill`, the input/output shapes, and that function with its Neo client.

Illustrative setup, not a final API:

```text
client.register("zuzu.auth.fill", inputShape, outputShape, FillLogin)
```

The client keeps `FillLogin` in Go. It sends Neo the method's name, input/output shapes, and a way to route calls back to this service. It does not send the Go function.

```text
Go service                         Neo sidecar
keeps: name -> Go function          keeps: name -> Go service
           -- registration ------->
```

### 2. Give the agent a JavaScript method

Neo creates a small JavaScript function named `zuzu.auth.fill`. This function forwards calls to the registered Go handler. It is installed in each fresh run.

The same registration describes the method in generated `.d.ts` declarations so the agent knows how to call it.

### 3. Send references, not JavaScript objects

The `page` object lives in Neo's JavaScript runtime. Go cannot directly use an object in another program's memory.

Instead, Neo sends a reference identifying the tab. It resolves the field locators and sends references identifying those fields.

For illustration only:

| Value in agent JavaScript | Information sent to Go |
|---|---|
| `page` | Reference to tab 17 |
| `page.getByLabel("Email")` | Reference to the email field in that tab |
| `page.getByLabel("Password")` | Reference to the password field in that tab |

These references let Go ask Neo to act on the same browser objects. The actual reference format and lifetime are still to design.

### 4. Go performs the customer logic; Neo performs browser actions

```text
Agent JavaScript      Neo sidecar (Rust)      mirrord (Go)
      | call method          |                    |
      |--------------------->| send references     |
      |                      |------------------->|
      |                      |   protected fill   |
      |                      |<-------------------|
      |                      |                    |
      |                      +--> Chromium        |
      |                      |   fill completed   |
      |                      |------------------->|
      |                      |   safe result      |
      |<---------------------|<-------------------|
```

Go checks its rules, gets the credentials, and requests a protected fill from Neo. Neo types into Chromium and reports completion. Go returns the safe result to the waiting JavaScript call.

Neo must remain able to handle that fill request while `neo.run` waits for Go.

## Misc LLD points

- Connection and message format for registration, method calls, and browser calls back from Go.
- Which host/session may register and invoke a method.
- How page and field references are encoded, checked, and expired.
- How input/output shapes describe special values such as pages and locators.
- How nested calls share cancellation, deadlines, and trace IDs.
- What happens when Go disconnects or a browser action's outcome is unknown.

## Design of individual components

- **Go client registry:** keeps the customer's handler functions and receives method calls.
- **Neo method registry:** knows which methods exist and where each is implemented.
- **JavaScript wrappers and declarations:** expose those methods inside fresh runs.
- **Protected browser interface:** lets trusted Go handlers request browser actions without returning secrets to agent code.

## References

- Parent design: [Claw SDK working design](claw-sdk-design.md).
- Reference document, section 3 and the login sequence:
  https://docs.shadowfax92.workers.dev/chVZOHAd
