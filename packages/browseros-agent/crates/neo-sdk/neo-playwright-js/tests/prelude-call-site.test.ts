/**
 * Run the real bootstrap and bundle with Node's async stack reconstruction disabled, as in
 * QuickJS. This covers queued subscriptions and compound disposal without requiring a real
 * browser; the native QuickJS tests separately cover sequential, helper and concurrent calls.
 */
import { expect, it } from 'bun:test'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

function runCallSiteProbe(probe: string): unknown {
  const output = execFileSync(
    'node',
    [
      '--no-async-stack-traces',
      '--input-type=module',
      '-e',
      probe,
      fileURLToPath(
        new URL(
          '../../neo-server/src/runtime/js/bootstrap.js',
          import.meta.url,
        ),
      ),
      fileURLToPath(
        new URL('../../neo-playwright/assets/prelude.js', import.meta.url),
      ),
    ],
    { encoding: 'utf8', timeout: 10_000 },
  )
  return JSON.parse(output)
}

it('carries one public disposal site through every nested cleanup await', () => {
  const probe = `
    import { readFileSync } from 'node:fs';
    import { createContext, runInContext } from 'node:vm';
    const calls = [];
    let id = 0;
    const realm = createContext({
      URL, URLSearchParams, atob, btoa, DOMException,
      __neo_log() {},
      __neo_sleep: async () => {},
      __neo_invoke: async (name, input, site) => {
        calls.push({ name, site });
        if (name === 'events.next') return new Promise(() => {});
        const ok = name.endsWith('.exposeBinding') || name.endsWith('.addInitScript')
          ? { id: ++id } : {};
        return JSON.stringify({ ok, pages: [] });
      },
    });
    runInContext('Error.stackTraceLimit = 200', realm);
    runInContext(readFileSync(process.argv[1], 'utf8'), realm, { filename: 'bootstrap.js' });
    runInContext(readFileSync(process.argv[2], 'utf8'), realm, { filename: 'prelude.js' });
    const parameters = runInContext('__neo.parameters()', realm);
    const body = "const registration = await page.addInitScript(({ cb }) => cb(), { cb: () => 1 }, { exposeFunctions: true });\\nawait registration.dispose();";
    const main = runInContext('(async function(' + parameters + ') {\\n{\\n' + body + '\\n}\\n})', realm, { filename: 'run' });
    const result = JSON.parse(await runInContext('__neo.run', realm)(main,
      JSON.stringify({ page: { $neo: 'page', tab: 1007 }, pages: [], observed: null })));
    const sites = calls.filter(call => call.name === 'disposable.dispose').map(call => call.site);
    console.log(JSON.stringify({ result, count: sites.length,
      lines: sites.map(site => Number(/run:(\\d+):/.exec(site)?.[1] ?? 0)),
      allSameSite: Boolean(sites[0]) && sites.every(site => site === sites[0]) }));
  `
  expect(runCallSiteProbe(probe)).toEqual({
    result: { ok: null },
    count: 2,
    lines: [2, 2],
    allSameSite: true,
  })
})

it('keeps script listener sites across queued setup and leaves worker bookkeeping and pulls internal', () => {
  const probe = `
    import { readFileSync } from 'node:fs';
    import { createContext, runInContext } from 'node:vm';
    const calls = [];
    let releaseContext;
    let contextStarted;
    let allListenersSent;
    let queuedContextBlocked = false;
    const contextReady = new Promise(resolve => { contextStarted = resolve; });
    const listenersSent = new Promise(resolve => { allListenersSent = resolve; });
    const realm = createContext({
      URL, URLSearchParams, atob, btoa, DOMException,
      __neo_log() {},
      __neo_sleep: async () => {},
      __neo_invoke: async (name, encoded, site) => {
        const input = JSON.parse(encoded);
        calls.push({ name, input, site });
        if (name === 'events.next') return new Promise(() => {});
        if (name === 'events.listen') {
          const changes = calls.filter(call => call.name === name);
          if (changes.length === 8) allListenersSent();
          if (input.event === 'pageload' && releaseContext === undefined) {
            const ready = new Promise(resolve => { releaseContext = resolve; });
            contextStarted();
            await ready;
          }
        }
        if (name === 'page.title') {
          await contextReady;
          queuedContextBlocked = calls.filter(call =>
            call.name === 'events.listen' && call.input.event === 'pageload').length === 1;
          releaseContext();
        }
        return JSON.stringify({
          ok: name === 'page.title' ? { title: 'ready' } : {},
          pages: name === 'page.close' ? [{ tab: 1007, closed: true, workers: [] }] : [],
          observed: name === 'page.close' ? { seq: 1, events: 0 } : null,
        });
      },
    });
    runInContext('Error.stackTraceLimit = 200', realm);
    runInContext(readFileSync(process.argv[1], 'utf8'), realm, { filename: 'bootstrap.js' });
    runInContext(readFileSync(process.argv[2], 'utf8'), realm, { filename: 'prelude.js' });
    const parameters = runInContext('__neo.parameters()', realm);
    const body = [
      'await Promise.resolve();',
      'const listener = () => {};',
      'page.on("dialog", listener);',
      'context.on("pageload", listener);',
      'context.off("pageload", listener);',
      'context.on("pageload", listener);',
      'await page.title();',
      'page.off("dialog", listener);',
      'context.off("pageload", listener);',
      'await page.close();',
    ].join('\\n');
    const main = runInContext('(async function(' + parameters + ') {\\n{\\n' + body + '\\n}\\n})', realm, { filename: 'run' });
    const result = JSON.parse(await runInContext('__neo.run', realm)(main, JSON.stringify({
      page: { $neo: 'page', tab: 1007 },
      pages: [{ tab: 1007, closed: false, workers: [] }],
      observed: { seq: 0, events: 0 },
    })));
    await listenersSent;
    const listeners = calls.filter(call => call.name === 'events.listen');
    const scriptChanges = event => listeners.filter(call => call.input.event === event).map(call => ({
      enabled: call.input.enabled,
      line: Number(/run:(\\d+):/.exec(call.site)?.[1] ?? 0),
    }));
    console.log(JSON.stringify({ result, queuedContextBlocked,
      pageChanges: scriptChanges('dialog'),
      contextChanges: scriptChanges('pageload'),
      workers: listeners.filter(call => call.input.event === 'workerstate').map(call => ({
        enabled: call.input.enabled,
        site: call.site,
      })),
      pumps: calls.filter(call => call.name === 'events.next').map(call => call.site),
    }));
  `
  expect(runCallSiteProbe(probe)).toEqual({
    result: { ok: null },
    queuedContextBlocked: true,
    pageChanges: [
      { enabled: true, line: 3 },
      { enabled: false, line: 8 },
    ],
    contextChanges: [
      { enabled: true, line: 4 },
      { enabled: false, line: 5 },
      { enabled: true, line: 6 },
      { enabled: false, line: 9 },
    ],
    workers: [
      { enabled: true, site: '' },
      { enabled: false, site: '' },
    ],
    pumps: [''],
  })
})
