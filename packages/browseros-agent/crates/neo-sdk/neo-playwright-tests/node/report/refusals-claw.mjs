/** Audited v1 refusals: prelude hints and the production door's refusal codes. */
const hints = [
  {
    hint: 'Neo does not share its browser',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo drives its own BrowserOS; there is no browser type to launch',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo owns the browser and its profile; close a tab with page.close()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'raw CDP sessions are not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo has one browser profile; open a tab with context.newPage()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo records its own trace of every run',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: "cookies and storage stay with the owner's signed-in profile",
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: "permissions are the owner's profile settings",
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'network interception is not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'network settings are not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: "geolocation is the owner's profile setting",
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'clock emulation is not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: "credentials stay with the owner's profile",
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'the debugger is not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'API requests are not available in Neo; fetch from the page with page.evaluate()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'locator handlers are not available in Neo; check for the overlay with locator.isVisible() before acting',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'adding tags is not available in Neo; change the page with page.evaluate()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo keeps agent tabs in the background; act on the page directly',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'there is no inspector in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo keeps no console history; listen with page.on("console")',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo keeps no error history; listen with page.on("pageerror")',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo keeps no console history; hosted BrowserOS emits no console events',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo keeps no error history; hosted BrowserOS emits no pageerror events',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo keeps no console history; listen with page.on("console") before the page logs',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'media emulation is not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo returns values as JSON, without handles; use evaluate()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo keeps no error history; listen with page.on("pageerror") before the page fails',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'locator handlers are not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'garbage collection is not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo does not record network requests',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'the hosted viewport is fixed',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo has no touch input; use click()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'use page.waitForURL() or page.waitForLoadState()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo does not report network events; wait for what the page shows instead',
    sources: [
      'neo-playwright-js/src/prelude/unsupported.ts',
      'neo-playwright-js/src/prelude/events.ts',
    ],
  },
  {
    hint: 'coverage is not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'read storage with page.evaluate(() => localStorage.getItem(key))',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo records its own screencast of every tab',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'read storage with page.evaluate(() => sessionStorage.getItem(key))',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo has no touch input; use page.mouse',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'WebMCP is not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'adding tags is not available in Neo; change the page with frame.evaluate()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: "locate the iframe from its parent instead: page.locator('iframe')",
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'use frame.waitForURL() or frame.waitForLoadState()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'dropping data is not available in Neo; use locator.dragTo()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'not available in Neo; use the locator as built',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'poll with expect(locator).toPass() or page.waitForFunction()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'use page.frameLocator() or locator.contentFrame() to enter a frame',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo returns values as JSON; read them with elementHandle.evaluate(el => el.prop)',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'use page.frames() to find a frame',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'element handles cannot root a selector in Neo; use locator.locator(selector) instead',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: "Neo keeps the run's downloads; it cannot cancel one",
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo gives a script the saved file, not its bytes: await download.path() and upload it with setInputFiles; the host reads the bytes from the run',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: "Neo keeps the run's downloads; it cannot delete one",
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo passes console text only; use message.text()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo does not read response bodies; read the page with page.content() or page.evaluate()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo does not report requests',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo does not report TLS details',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo does not report server addresses',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'custom matchers are not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: "expect's test state is not available in Neo",
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'soft assertions are not available in Neo; catch the error to continue',
    sources: [
      'neo-playwright-js/src/prelude/unsupported.ts',
      'neo-playwright-js/src/prelude/expect.ts',
    ],
  },
  {
    hint: 'use expect(response.ok()).toBe(true)',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'visual comparisons are not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'compare the text of await locator.ariaSnapshot() instead, such as with toContain()',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'snapshot files are not available in Neo',
    sources: ['neo-playwright-js/src/prelude/unsupported.ts'],
  },
  {
    hint: 'Neo does not report WebSockets',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report workers',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report service workers',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report extension background pages',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report file choosers; call locator.setInputFiles() on the input',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report frame events; read page.frames(), which Neo refreshes after navigations and waits',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report frame events; use page.waitForURL()',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report load events; use page.waitForLoadState()',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report load events; use page.waitForLoadState("domcontentloaded")',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report crashes; a crashed page fails its next call',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'listen for close on the page itself: page.on("close")',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'listen for pageerror on the page itself: page.on("pageerror")',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'the context reports no weberror events; pageerror listeners are silent in hosted BrowserOS',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'Neo does not report closed dialogs',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
  },
  {
    hint: 'this runtime has no timers; wait on the page instead',
    sources: ['neo-playwright-js/src/prelude/bridge.ts'],
  },
  {
    hint: 'an op cannot be aborted from the script in Neo; pass timeout instead',
    sources: ['neo-playwright-js/src/prelude/evaluation.ts'],
  },
  {
    hint: 'Neo passes element handles as evaluate arguments, not pages, frames, locators or JSHandles; pass the value itself, or an element handle',
    sources: ['neo-playwright-js/src/prelude/evaluation.ts'],
  },
  {
    hint: 'Neo snapshots carry no boxes; use locator.boundingBox()',
    sources: ['neo-playwright-js/src/prelude/frame.ts'],
  },
  {
    hint: "snapshot a child frame through its iframe: page.locator('iframe').contentFrame().locator('body').ariaSnapshot()",
    sources: ['neo-playwright-js/src/prelude/frame.ts'],
  },
  {
    hint: 'Neo cannot mask elements in a screenshot; omit mask',
    sources: ['neo-playwright-js/src/prelude/frame.ts'],
  },
  {
    hint: 'Neo cannot add a stylesheet to a screenshot; omit style',
    sources: ['neo-playwright-js/src/prelude/frame.ts'],
  },
  {
    hint: 'Neo cannot capture a transparent background; omit omitBackground',
    sources: ['neo-playwright-js/src/prelude/frame.ts'],
  },
  {
    hint: 'Neo does not stop animations for a screenshot; omit animations',
    sources: ['neo-playwright-js/src/prelude/frame.ts'],
  },
  {
    hint: 'Neo captures at the device scale; omit scale or pass "device"',
    sources: ['neo-playwright-js/src/prelude/frame.ts'],
  },
  {
    hint: "Neo's waitForFunction handle holds the value, not the page's object; read it with jsonValue()",
    sources: ['neo-playwright-js/src/prelude/frame.ts'],
  },
  {
    hint: 'Neo returns the PDF instead of writing a file; omit path',
    sources: ['neo-playwright-js/src/prelude/page.ts'],
  },
  {
    hint: "frameLocator() needs a selector in Neo, such as frameLocator('iframe') or locator('#pay').contentFrame()",
    sources: ['neo-playwright-js/src/prelude/selectors.ts'],
  },
]

const escapePattern = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
const api = String.raw`(?:browser|browserContext|page|frame|locator|elementHandle|jsHandle|download|consoleMessage|response|expect|keyboard|mouse)\.[^:\n]+`
const prefix = `^(?:\\w*Error: )?${api}: `

export const clawRefusals = hints.map(({ hint, sources }) => ({
  wording: `<api>: ${hint}`,
  sources,
  pattern: new RegExp(`${prefix + escapePattern(hint)}$`),
}))

clawRefusals.push(
  {
    wording: '<api>: event belongs to another emitter',
    sources: ['neo-playwright-js/src/prelude/events.ts'],
    pattern: new RegExp(
      prefix +
        String.raw`(?:"[^"\n]+" is a browser context event: context\.on\("[^"\n]+"\)|Neo reports "[^"\n]+" on pages: page\.on\("[^"\n]+"\)|Neo reports only Playwright's events, not "[^"\n]+")$`,
    ),
  },
  {
    wording: '<api>: neo / unsupported or script / method_unavailable',
    sources: [
      'neo-server/src/dispatch/dispatcher.rs',
      'neo-server/src/dispatch/op_ctx.rs',
      'neo-server/src/builtins/playwright_ops.rs',
      'neo-server/src/builtins/op_marks.rs',
      'neo-playwright/src/op_env.rs',
      'neo-server/src/runtime/js/bootstrap.js',
    ],
    pattern:
      /^(?:\w*Error: )?(?:[\w.]+: )?(?:neo\s*\/\s*unsupported|script\s*\/\s*method_unavailable)(?:\b|:)/,
  },
  {
    wording: '<api>: unknown or held method',
    sources: [
      'neo-server/src/dispatch/dispatcher.rs',
      'neo-server/src/builtins/op_marks.rs',
    ],
    pattern: new RegExp(
      prefix +
        '`[^`]+` (?:is not a Neo method$|is held for now because it .+ Nothing was sent\\. Meanwhile, .+\\.$)',
    ),
  },
)

// Rust returns these hints as the error message; the owner/code need not be in its text.
const rustHints = [
  {
    wording:
      '<api>: printing to PDF is not available in this browser ({message})',
    sources: ['neo-playwright/src/page/pdf.rs'],
    expression: 'printing to PDF is not available in this browser \\(.+\\)',
  },
  {
    wording:
      '<api>: waitUntil "networkidle" cannot be observed after setContent in Neo; use "load"',
    sources: ['neo-playwright/src/page/document_op.rs'],
    expression:
      'waitUntil "networkidle" cannot be observed after setContent in Neo; use "load"',
  },
  {
    wording:
      "<api>: a frame of the script's site did not answer Neo's privacy sweep within {}ms, {consequence}; try again once the frame responds",
    sources: ['neo-playwright/src/page/evaluate_guard.rs'],
    expression:
      "a frame of the script's site did not answer Neo's privacy sweep within .+ms, .+; try again once the frame responds",
  },
  {
    wording:
      "<api>: Neo lost its connection to a frame of the script's site and could not reach it again for its privacy sweep, {consequence}; try again",
    sources: ['neo-playwright/src/page/evaluate_guard.rs'],
    expression:
      "Neo lost its connection to a frame of the script's site and could not reach it again for its privacy sweep, .+; try again",
  },
  {
    wording:
      "<api>: Neo does not open javascript: URLs, whose script runs in the page's main world outside Neo's privacy sweep; run the script with page.evaluate",
    sources: ['neo-playwright/src/page/navigation.rs'],
    expression:
      "Neo does not open javascript: URLs, whose script runs in the page's main world outside Neo's privacy sweep; run the script with page\\.evaluate",
  },
  {
    wording:
      "<api>: Neo navigates only a page's main frame; navigate the page, or set the iframe's src with page.evaluate",
    sources: ['neo-playwright/src/page/navigation.rs'],
    expression:
      "Neo navigates only a page's main frame; navigate the page, or set the iframe's src with page\\.evaluate",
  },
  {
    wording:
      '<api>: this frame belongs to {preview}; Neo enters the frames of <iframe> and <frame> elements only',
    sources: ['neo-playwright/src/resolve/frame_start.rs'],
    expression:
      'this frame belongs to .+; Neo enters the frames of <iframe> and <frame> elements only',
  },
  {
    wording:
      "<api>: a sensitive field of the frame's site changed while Neo read the page, and again when it read once more, so the result is withheld; try again once the field stops changing",
    sources: ['neo-playwright/src/resolve/resolver.rs'],
    expression:
      "a sensitive field of the frame's site changed while Neo read the page, and again when it read once more, so the result is withheld; try again once the field stops changing",
  },
  {
    wording:
      "<api>: a frame did not answer Neo's privacy scan within {}ms, so Neo cannot see every shadow root of its page and refuses the op; try again once the frame responds",
    sources: ['neo-playwright/src/resolve/resolver.rs'],
    expression:
      "a frame did not answer Neo's privacy scan within .+ms, so Neo cannot see every shadow root of its page and refuses the op; try again once the frame responds",
  },
  {
    wording:
      '<api>: the element is inside a rotated, skewed or flipped iframe, which has no box in page coordinates in Neo',
    sources: ['neo-playwright/src/read/viewport_box.rs'],
    expression:
      'the element is inside a rotated, skewed or flipped iframe, which has no box in page coordinates in Neo',
  },
  {
    wording:
      "<api>: Neo selects options by value, label or index, not by element handle; pass the option's value or label",
    sources: ['neo-playwright/src/action/action_op.rs'],
    expression:
      "Neo selects options by value, label or index, not by element handle; pass the option's value or label",
  },
  {
    wording:
      '<api>: an iframe on the way to the element has a CSS transform, which Neo cannot place input through yet',
    sources: ['neo-playwright/src/action/pointer_action.rs'],
    expression:
      'an iframe on the way to the element has a CSS transform, which Neo cannot place input through yet',
  },
  {
    wording:
      '<api>: the popup ({}) opened as a separate window, which Neo cannot drive; Neo drives popups that open as tabs',
    sources: ['neo-playwright/src/events/event_reader.rs'],
    expression:
      'the popup \\(.+\\) opened as a separate window, which Neo cannot drive; Neo drives popups that open as tabs',
  },
  {
    wording:
      "<api>: context.on('{event}') is not available in Neo; listen on the page: page.on('{event}')",
    sources: ['neo-playwright/src/events/event_op.rs'],
    expression:
      "context\\.on\\('.+'\\) is not available in Neo; listen on the page: page\\.on\\('.+'\\)",
  },
  {
    wording:
      "<api>: toHaveJSProperty is not supported: Neo matches in a sealed world that does not see the page's own JavaScript objects; use expect.poll(() => locator.evaluate((element) => element.name)).toEqual(value) instead",
    sources: ['neo-playwright/src/expect/expectation.rs'],
    expression:
      "toHaveJSProperty is not supported: Neo matches in a sealed world that does not see the page's own JavaScript objects; use expect\\.poll\\(\\(\\) => locator\\.evaluate\\(\\(element\\) => element\\.name\\)\\)\\.toEqual\\(value\\) instead",
  },
  {
    wording:
      '<api>: Neo refused this call: linked tabs are not checked by the host',
    sources: ['neo-playwright/src/op_env.rs'],
    expression:
      'Neo refused this call: the page is linked to \\d+ other tab\\(s\\) through window\\.opener .+',
  },
  {
    wording:
      '<api>: a script in this page can act on an unlisted tab through window.opener',
    sources: ['neo-server/src/dispatch/op_ctx.rs'],
    expression:
      'a script in this page can act on tab \\d+ through window\\.opener .+So Neo cannot check its rules there',
  },
]

clawRefusals.push(
  ...rustHints.map(({ wording, sources, expression }) => ({
    wording,
    sources,
    pattern: new RegExp(prefix + expression + '$'),
  })),
)
