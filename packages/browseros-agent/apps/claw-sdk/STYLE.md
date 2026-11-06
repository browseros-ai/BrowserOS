# claw-sdk style

claw-sdk follows the [Google Rust style guide](https://google.github.io/styleguide/rustguide.html)
plus the project rules below. Tools check most of it, and `check.sh` runs them all:

| Tool | Checks |
|---|---|
| `Cargo.toml` `[lints]` | panics on input, docs on exported items, imports, `#[expect]` with a reason, clippy pedantic |
| `clippy.toml` | no tokio lock across an await, no direct core `PageJs` script, no blocking sleep, path depth |
| `check.sh` | Google's rustfmt options, forbidden dependencies, the CI toolchain |
| `tests/style_boundaries.rs` | P1, P2 (entry points, the door's dependencies), P4 (derives), G7, D2 (presence), lint-table drift |

This file is what review checks. Each rule has an id for review comments ("P3").

## Project rules

### P1. Only `runtime/` imports the JS engine

The engine stays swappable and could move out of process. Everything else sees JSON strings.

```rust
// Bad, in dispatch/: rquickjs::Value<'_> as a parameter
// Good: the bridge decodes to serde_json::Value (`Call::input`) before the dispatcher sees it
pub(crate) fn submit(self: &Arc<Self>, run: &Arc<RunCtx>, call: Call) -> PendingReply
```

### P2. One front door

Every SDK action is a `Method` registered in the `Registry`, and only the dispatcher runs it. A
script's call enters through `Dispatcher::submit` from the bridge, as do the idle-tab janitor's
closes. A method that needs another action calls `OpCtx::invoke`, which runs it through the same
door as a child op, with its own record, rules and delivery. A method never
calls another method's `call`, and the types enforce it, because only the door mints the
`OpCtx<M>` that `call` takes. Nor does it drive the browser outside its own op. `mcp/`, `http/`
and `runtime/` never name a browser crate.

The door depends on nothing behind it. `dispatch/` runs methods only as `Method` and rules only as
`Policy`, and never names `builtins/` or `host/`. Those register into the `Registry` and depend on
the door, never the other way round. A door that named a built-in could reach it around its own
registry.

```rust
// Bad: a second path around policy, delivery and the trace (and a type error: `Title::call`
// takes an `&OpCtx<Title>`, which only the door can mint)
let title = Title.call(op, TitleInput { page }).await?;
// Good
let title = op.invoke("page.title", json!({ "page": page })).await?;

// Bad, in dispatch/: the door names a built-in
use crate::builtins::solve_captcha::SolveCaptcha;
// Good, in builtins/: the built-in names the door and registers into it
use crate::dispatch::{Method, MethodSpec, RegistryBuilder};
```

### P3. No code returns a form field's value to the agent

Page reads go through browseros-playwright's sealed world, which masks password, card and code
fields at the source. A direct core `PageJs` script is a clippy error unless it carries a reason
its result cannot hold a field value.

```rust
// Bad: `input.value` can be a password
PageJs::isolated(session).eval::<String>("document.activeElement.value").await
// Good
#[expect(clippy::disallowed_methods, reason = "`document.title` is never a form-control value")]
```

### P4. Secrets never print, serialize or log

A type that holds a secret or a raw field value (`Zeroizing`, `SecretValue`, `FieldValues`) has no
`Serialize` or `Display`, and its `Debug` prints a fixed mask. Never log CDP params or results of
calls that carry values (`Input.insertText`, `Runtime.evaluate`), nor VNC client bytes.

```rust
// Bad
#[derive(Debug, Serialize)] pub struct SecretValue(Zeroizing<String>);
// Good
pub struct SecretValue(Zeroizing<String>);
impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { formatter.write_str("[secret]") }
}
```

### P5. Every failure names its owner where the cause is known

`ErrorOwner::{Neo, Script, Host, Website}`. Set it at the point that knows who failed; keep the
source error (`map_err_ignore` is on). `Script` is the agent's own code: a bad selector, a strict
mode violation, an unknown method. A host reply may set `Script`, `Website` or `Host`, never `Neo`.

```rust
// Bad: a page that never loaded becomes Neo's fault
.map_err(|_| OperationError::new(ErrorOwner::Neo, codes::NAVIGATION_FAILED, "goto failed"))
// Good
.map_err(|error| OperationError::website(codes::NAVIGATION_FAILED, error))
```

### P6. Delivery is reported, never guessed

Call `op.recheck()` and then `op.mark_sent()` immediately before the first side-effecting write,
and nowhere else. Report `Answer::Ambiguous` when the write may have happened. The dispatcher
derives `not_sent` / `done` / `unknown`; a leaf never sets it.

```rust
// Bad: marked before waiting for actionability, so a takeover in between still counts as sent
op.mark_sent(); wait_actionable(&target).await?; click(&target).await
// Good
wait_actionable(&target).await?; op.recheck()?; op.mark_sent(); click(&target).await
```

### P7. Browser awaits are raced with cancel and the deadline

A leaf awaits browser work through `op.race(..)`, so cancellation and the run budget reach it.
Nothing calls back into the agent's JS runtime; events reach JS by pulling.

```rust
// Bad: survives a cancelled run until CDP answers
let node = session.query(selector).await?;
// Good
let node = op.race(session.query(selector)).await?;
```

### P8. Our vocabulary enums are matched exhaustively

No `_ =>` arm on `ErrorOwner`, `Delivery`, `Effect`, `Caller`, `Visibility`, `Answer`, so a new
variant forces a decision at every use. (`_` stays fine on foreign enums such as `serde_json::Value`
or `CoreError`, which is why this is not a lint.)

```rust
// Bad
match delivery { Delivery::Done => record(), _ => {} }
// Good
match delivery { Delivery::Done => record(), Delivery::NotSent | Delivery::Unknown => {} }
```

### P9. JSON input is decoded once, into a typed struct

A leaf deserializes its input at the top, with `deny_unknown_fields`. No `input["url"]`
(`indexing_slicing` flags it), no chains of `.get()` for a known shape.

```rust
// Bad
let url = input["url"].as_str().unwrap_or_default();
// Good
let GotoInput { url, timeout_ms } = serde_json::from_value(input)?;
```

### P10. Tests return `Result` and use `?`

The workspace denies `unwrap` and `expect` in tests too. Tests may `panic!`, `assert!` and index.
Each file under `tests/` opens with a one-line `//!` saying what it covers (`missing_docs` asks
for it).

```rust
#[test]
fn replays_a_repeated_request_id() -> Result<(), Box<dyn Error>> {
    let first = service.run(request.clone()).await?;
    assert_eq!(service.run(request).await?.run_id, first.run_id, "a repeat must not run again");
    Ok(())
}
```

## Google rules that review checks

### G1. Newtypes over aliases when the meaning differs

```rust
type RunId = u64;            // Bad: any u64 is accepted
pub struct RunId(u64);       // Good
```

### G2. Enums over bool parameters whose meaning is not obvious

Clippy catches two bools in one signature; one unclear bool is still a review comment.

```rust
locator.click(true)                      // Bad: true what?
locator.click(Strictness::FirstMatch)    // Good
```

### G3. Named fields over tuples

Newtypes are the exception.

```rust
fn bounds(&self) -> (f64, f64, f64, f64)   // Bad
fn bounds(&self) -> Rect                   // Good: Rect { x, y, width, height }
```

### G4. `Option` for plain absence, `Result` for failures with a cause, panic never on input

```rust
fn page(&self, id: PageId) -> Option<&Page>                 // absence is the whole story
fn navigate(&self, url: &Url) -> Result<(), OperationError> // the caller needs to know why
```

### G5. Borrow instead of cloning to dodge the borrow checker

`.clone()` is a deep copy and must earn its place; a refcount bump is `Arc::clone(&x)` (the lint
makes the two look different). Prefer unique ownership; `Arc` is for state shared across tasks.

```rust
let name = spec.name.clone(); registry.method(&name)   // Bad
registry.method(&spec.name)                             // Good
```

### G6. Imports

Types are imported and used unqualified. Free functions are called through their module. A trait
is imported inside the function that needs it, unless the file uses it throughout. `io::Error`
style is fine where a bare name would be ambiguous.

```rust
use std::mem;
let log = mem::take(&mut self.log);   // Good, not `take(..)` or `std::mem::take(..)`

fn parse(text: &str) -> Result<u8, ParseIntError> {
    use std::str::FromStr;            // Good: scoped to its one use
    u8::from_str(text)
}
```

### G7. Modules

A leaf module is named after its primary item (`operation_guard.rs` holds `OperationGuard`). A
non-leaf module (`dispatch.rs` beside `dispatch/`) holds only `mod` and `pub use`. The test checks
the names; review checks that the item really is the file's main subject.

### G8. Traits only when they are needed

For generics (`Method`), `dyn` seams (`Policy`, the registry's erased methods) or an extension
trait named `…Ext`. Not to group methods.

```rust
trait RunHelpers { fn signal(&self, line: String); }   // Bad: impl RunCtx { fn signal(..) }
```

### G9. Names

`as_` / `to_` / `into_` by cost and ownership; constructors `new`, `new_with_x`, `try_new`;
accessors `foo()`, `foo_mut()`, `set_foo()`, `take_foo()`. No new abbreviations: the project's
`op`, `ctx`, `cdp` and `id` are fine; `mgr`, `cfg_val` and `hndl` are not.

### G10. Explicit types where inference hides them

Annotate the binding after `.parse()`, `.into()`, `.collect()` and long chains, rather than a
turbofish.

```rust
let timeout: Duration = config.timeout.into();
```

### G11. Short iterator chains; a `for` loop when it reads better

If a chain needs a comment to follow, write the loop.

### G12. Assertion messages say why it holds

```rust
assert!(seq > last, "seq > last");                                    // Bad: repeats the code
assert!(seq > last, "op ids are assigned at start, so they sort by start");   // Good
```

### G13. `#[must_use]` on pure functions and lazy types

Clippy checks exported functions; review checks types whose value is the point (a guard, a
builder).

### G14. No `impl Into<T>` parameters

Google: application code should almost never take them. Take the type, or `&str`.

```rust
pub fn note(&self, line: impl Into<String>)   // Bad
pub fn note(&self, line: String)              // Good
```

### G15. Options structs implement `Default`

A bag of options with all-public fields derives `Default`, so callers write
`PointerOptions { force: Force::Skipped, ..PointerOptions::default() }`.

### G16. No `catch_unwind`, no code before `main`, `#[rustfmt::skip]` only for deliberate alignment

## Docs and comments

The owner's rules, from their CLAUDE.md. A new engineer should understand the architecture
without reconstructing it across several files.

### D1. A doc comment says the item's job in the system, not its type

```rust
/// The run id.                                                            // Bad
/// Neo's own id for this run. The host's id is `request_id`, kept apart.  // Good
pub run_id: RunId,
```

### D2. Every non-leaf module opens with `//!`: what the subsystem owns and how data flows

The test checks that the doc exists; review checks that it explains. A flow across several files
gets a short block comment or ASCII diagram there.

```rust
//! The operation dispatcher: the one path every SDK call takes.
//!
//! agent JS ─▶ bridge ─▶ Dispatcher::submit ─▶ rules ─▶ Method::call ─▶ mask ─▶ JSON envelope
//!
//! It owns delivery (from `OperationGuard`) and the trace record; methods report facts.
```

### D3. Comment the seams

Process and thread boundaries, async handoffs, lifecycle and cleanup, cancellation, permissions,
protocol translation, request correlation, ordering and persistence. Say what breaks if the
invariant goes.

```rust
// Bad
// Lock the map.
// Good
// Released before the CDP call: a takeover reads this map while the click waits (D6.10).
```

### D4. No narration

No line-by-line paraphrase, no PR history ("changed to fix…"), no TODO without an issue, no
commented-out code.

### D5. Every `#[expect]` reason says why the exception is safe here

```rust
#[expect(clippy::indexing_slicing, reason = "temporary")]                         // Bad
#[expect(clippy::indexing_slicing, reason = "split_once returned both halves")]   // Good
```
