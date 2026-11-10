# neo-playwright-tests

Test infrastructure only. Keep upstream modules pinned and complete, with explicit fixture seams. Missing Neo APIs are unsupported; never supply a CDP helper to the test body. One case uses one native run and two sequential private headless browser processes. Raw CDP is for lifecycle and read-only evidence only.

The `playwright` feature enables the expected-red upstream test target and private worker. Default Cargo tests verify the harness without browsers. Follow ../NEO_STYLE.md. Use private Cargo directories. Run Bun and Node tests for js/*.test.mjs. Preserve exact return values, error messages, source hashes, case identities and incomplete evidence. Do not turn expected upstream failure or missing environment into a compatibility pass.
