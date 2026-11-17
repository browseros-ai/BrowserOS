/** Neo: Explicit SDK refusal wordings, kept separate from ordinary Playwright validation errors. */

/** Neo: Message families audited in neo-quickjs-injected/src and neo-runtime/src at e8182e2af. */
export const refusals = [
  {
    wording: 'Neo does not support …',
    sources: [
      'neo-quickjs-injected/src/{browser,page,browser-context,js-handle,neo-commands}.ts',
      'neo-runtime/src/playwright_methods/{browser_methods,content,evaluate,devices}.rs',
      'neo-runtime/src/chromium_bridge/frame_selector.rs',
    ],
    pattern: /(?:^|: )Neo does not support\b/,
  },
  {
    wording: 'Neo does not serve … [on child frames yet]',
    sources: [
      'neo-quickjs-injected/src/browser.ts',
      'neo-runtime/src/method_router.rs',
      'neo-runtime/src/browser_representations/cookies_and_storage.rs',
    ],
    pattern: /(?:^|: )Neo does not serve\b/,
  },
  {
    wording: 'Neo does not produce … events on …',
    sources: ['neo-runtime/src/quickjs_bridge.rs'],
    pattern: /(?:^|: )Neo does not produce .+ events on /,
  },
  {
    wording:
      "neo-runtime: … is not served yet [for a context browser.newContext created, only for the run's own context]",
    sources: [
      'neo-quickjs-injected/src/browser-context.ts',
      'neo-runtime/src/playwright_methods/browser_methods.rs',
    ],
    pattern: /(?:^|: )neo-runtime: \w+ is not served yet\b/,
  },
  {
    wording:
      'Neo reads cookies and localStorage only: storageState({ indexedDB, opfs, credentials }) is not served yet',
    sources: ['neo-runtime/src/browser_representations/cookies_and_storage.rs'],
    pattern: /(?:^|: )Neo reads cookies and localStorage only:/,
  },
  {
    wording:
      'Neo writes cookies and localStorage only: a state with indexedDB, opfs or credentials is not served yet',
    sources: ['neo-runtime/src/browser_representations/cookies_and_storage.rs'],
    pattern: /(?:^|: )Neo writes cookies and localStorage only:/,
  },
  {
    wording:
      "Neo does not clear every cookie of the user's browser, which would sign the user out of every site: pass clearCookies a name, domain or path",
    sources: ['neo-quickjs-injected/src/browser-context.ts'],
    pattern: /(?:^|: )Neo does not clear every cookie of the user's browser,/,
  },
  {
    wording:
      'Neo runs attached to BrowserOS; use the provided context / context.browser()',
    sources: ['neo-quickjs-injected/src/browser-type.ts'],
    pattern: /(?:^|: )Neo runs attached to BrowserOS;/,
  },
  {
    wording:
      'Neo runs attached to BrowserOS and launches no browser; CDP does not report its executable path',
    sources: ['neo-quickjs-injected/src/browser-type.ts'],
    pattern: /(?:^|: )Neo runs attached to BrowserOS and launches no browser;/,
  },
  {
    wording:
      'Neo drives BrowserOS (Chromium) only; firefox and webkit are not available',
    sources: ['neo-quickjs-injected/src/browser-type.ts'],
    pattern: /(?:^|: )Neo drives BrowserOS \(Chromium\) only;/,
  },
  {
    wording:
      'Neo runs inside BrowserOS and serves the browser to no other client',
    sources: ['neo-quickjs-injected/src/browser.ts'],
    pattern:
      /(?:^|: )Neo runs inside BrowserOS and serves the browser to no other client/,
  },
  {
    wording:
      "neo-runtime: Neo's SDK sends files as payloads or localPaths, not streams",
    sources: ['neo-runtime/src/playwright_methods/drop_on.rs'],
    pattern:
      /(?:^|: )neo-runtime: Neo's SDK sends files as payloads or localPaths, not streams/,
  },
  {
    wording: '… this run has no artifact directory, so Neo cannot read/write …',
    sources: [
      'neo-quickjs-injected/src/frame.ts',
      'neo-runtime/src/artifacts.rs',
    ],
    pattern:
      /: this run has no artifact directory, so Neo cannot (?:read|write) /,
  },
  {
    wording:
      "… is outside the run's artifact directory …, the only place Neo …",
    sources: ['neo-runtime/src/artifacts.rs'],
    pattern: / is outside the run's artifact directory .+, the only place Neo /,
  },
  {
    wording: "… leaves / must be relative to the run's artifact directory",
    sources: ['neo-runtime/src/artifacts.rs'],
    pattern: / (?:leaves|must be relative to) the run's artifact directory/,
  },
  {
    wording: '… is … bytes, over the … bytes Neo returns to a script at once',
    sources: ['neo-runtime/src/artifacts.rs'],
    pattern:
      / is \d+ bytes, over the \d+ bytes Neo returns to a script at once/,
  },
  {
    wording:
      'files: the files to drop total … bytes, over the … bytes Neo hands a page at once',
    sources: ['neo-runtime/src/playwright_methods/drop_on.rs'],
    pattern:
      /(?:^|: )files: the files to drop total \d+ bytes, over the \d+ bytes Neo hands a page at once/,
  },
  {
    wording:
      "route.continue: the route's request was fetched already, so a continue delivers that response; Neo cannot change its url, method, headers or post data now",
    sources: ['neo-runtime/src/browser_representations/routes.rs'],
    pattern:
      /(?:^|: )route\.continue: the route's request was fetched already, .*Neo cannot change /,
  },
]
