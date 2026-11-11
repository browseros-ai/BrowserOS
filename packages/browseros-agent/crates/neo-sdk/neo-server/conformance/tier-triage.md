# Retiring the old differential tiers

Every former tier-1 file is accounted for below. The new upstream suite retains expected values and invokes the real product. Neo tab ownership and additional frame regressions use direct assertions in `tests/behavior/`, without an oracle or normalized output.

This is a migration map, not a claim that each new check has already passed. Validation evidence belongs to the build result and Linux reports. A selected upstream id below is an actual runnable row from the pinned inventory.

| Old case | Destination |
|---|---|
| actions-check.js | Upstream `tests/page/page-check.spec.ts › should check the box @smoke` and the remaining assertions in that selected spec |
| actions-click.js | Upstream `tests/page/page-click-react.spec.ts › should timeout when click opens alert` and the remaining assertions in that selected spec |
| actions-hover-dblclick.js | Upstream locator-misc-1.spec.ts › should hover @smoke; locator-click.spec.ts › should double click the button |
| captures.js | captures-and-popups.js plus conformance/screenshots/screenshots.spec.ts |
| console-events.js | Retired: console suppression is no longer a product requirement |
| context-pages.js | ownership-and-refusals.js |
| covered-f9-nested-frame.js | covered-and-unreadable.js |
| dialogs-beforeunload.js | beforeunload.js |
| dialogs.js | dialogs.js |
| element-evaluate.js | navigation-and-scripts.js |
| evaluate-args.js | navigation-and-scripts.js |
| evaluate-errors.js | navigation-and-scripts.js |
| evaluate-navigated.js | frame-lifecycle.js |
| evaluate-values.js | navigation-and-scripts.js |
| expect-matchers.js | Upstream locator-any-frame.spec.ts › should support toHaveText; should support toHaveCount |
| frame-evaluate.js | navigation-and-scripts.js |
| frame-url-moved.js | frame-lifecycle.js |
| frame-wait-moved.js | frame-lifecycle.js |
| frames-live.js | frame-lifecycle.js |
| frames-locator-chain-read.js | nested-frames.js |
| frames.js | frame-lifecycle.js |
| goto-javascript.js | ownership-and-refusals.js |
| goto-responses.js | navigation-and-scripts.js |
| goto-wait-until.js | navigation-and-scripts.js |
| history.js | navigation-and-scripts.js |
| nest-f1-snapshot.js | nested-frames.js (tree, refs, fills, routing and removed-frame identity) |
| nest-f2-refs.js | nested-frames.js (tree, refs, fills, routing and removed-frame identity) |
| nest-f3-click.js | nested-frames.js (tree, refs, fills, routing and removed-frame identity) |
| nest-f3-fill.js | nested-frames.js (tree, refs, fills, routing and removed-frame identity) |
| nest-f4-same-origin-in-cross-site.js | nested-frames.js (tree, refs, fills, routing and removed-frame identity) |
| nest-f5-oopif-in-oopif.js | nested-frames.js (tree, refs, fills, routing and removed-frame identity) |
| nest-f6-unreadable-frame.js | covered-and-unreadable.js |
| nest-f7-navigated-frame.js | nested-frames.js (tree, refs, fills, routing and removed-frame identity) |
| nest-f8-frame-locator-click.js | nested-frames.js (tree, refs, fills, routing and removed-frame identity) |
| popup-evaluate.js | captures-and-popups.js |
| popups.js | ownership-and-refusals.js |
| reads-count.js | Upstream locator-any-frame.spec.ts › should count elements in a single frame |
| reads-text-content.js | Upstream locator-convenience.spec.ts › textContent should work |
| reads-wait.js | Upstream `tests/page/page-wait-for-selector-1.spec.ts › should throw on waitFor` and the remaining assertions in that selected spec |
| refusals.js | ownership-and-refusals.js |
| set-content.js | navigation-and-scripts.js |
| set-input-files-upstream.js | Upstream `tests/page/page-set-input-files.spec.ts › should upload the file` and the remaining assertions in that selected spec |
| set-input-files.js | uploads_browseros.rs::host_owned_paths_and_payloads_upload_and_an_empty_list_clears_the_input (retained real-MCP regression) plus selected upstream upload assertions |
| shadow-roots.js | refs-and-values.js |
| snapshot-ai.js | refs-and-values.js and nested-frames.js |
| snapshot-default.js | refs-and-values.js |
| snapshot-locator.js | refs-and-values.js |
| snapshot-ref-lifetime.js | refs-and-values.js |
| snapshot-sensitive.js | refs-and-values.js (raw field values and surrounding text) |
| strict-mode.js | Upstream locator-any-frame.spec.ts › should fail click upon strict mode violation inside a single frame |
| wait-for-function.js | dialogs.js |
| wait-for-load-state.js | navigation-and-scripts.js |
