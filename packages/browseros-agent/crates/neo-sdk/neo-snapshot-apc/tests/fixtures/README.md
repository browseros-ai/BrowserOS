# Recorded AnnotatedPageContent fixtures

Real `Page.getAnnotatedPageContent` (`includeActionableInformation: true`) protobuf payloads recorded from stock Chromium 158 and decoded offline by `tests/fixtures.rs`.

## Recording environment

- Browser binary: `BrowserOS Dev` (`/Users/shadowfax/code/chromium-3/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev`)
- Browser version: Chromium `158.0.8127.18` (stock, unpatched APC)

## Fixture inventory

| Fixture | Edge case covered | Frames | Nodes | Size (bytes) |
| --- | --- | ---: | ---: | ---: |
| `nested-cross-site-frames.pb` | 4-level, 8-frame tree across 3 loopback sites (`127.0.0.1`, `localhost`, `[::1]`) with same-origin and cross-site OOPIF nesting (`Top -> A1 -> B2 -> B3`, `Top -> B1 -> C2 -> A3 -> C4`) | 8 | 71 | 5,435 |
| `shadow-dom.pb` | Light DOM alongside open (`mode: 'open'`) and closed (`mode: 'closed'`) shadow roots with `<slot>` text projections | 1 | 23 | 1,924 |
| `zero-size-checkbox.pb` | CSS `0x0` (`opacity: 0; position: absolute`) checkboxes paired via `<label for>` and wrapping `<label>` (`label_for_dom_node_id` links back to each `0x0` input) | 1 | 13 | 1,136 |
| `covered-button.pb` | Visible `<button>` occluded by a fixed full-viewport backdrop and `<div role="dialog">` with higher `document_scoped_z_order` | 1 | 16 | 1,418 |
| `sensitive-form.pb` | Password (`RedactedHasBeenPassword`, omitted `field_value`), credit-card (`CoarseAutofillFieldType::CreditCard`), and OTP (`CoarseAutofillFieldType::Otp`) form controls with dummy test values | 1 | 24 | 1,989 |
| `hung-subframe.pb` | Healthy cross-site OOPIF (`localhost`) preserved alongside a spinning cross-site OOPIF (`[::1]`) that hits Chromium's 10 s subframe timeout and emits empty `IframeData.data` | 3 | 16 | 1,494 |

## Re-recording

Run from `packages/browseros-agent` (optionally pass one or more fixture names after `record_fixtures` to re-record a subset):

```sh
NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-3/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
  bun run test:browser -- cargo run -p neo-snapshot-apc --example record_fixtures
```
