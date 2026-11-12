/**
 * Run the real bootstrap and bundle with Node's async stack reconstruction disabled, as in
 * QuickJS. This covers successful compound disposal without requiring a real browser; the
 * native QuickJS tests separately cover sequential, helper and concurrent script calls.
 */
import { expect, it } from 'bun:test'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

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
    { encoding: 'utf8' },
  )
  expect(JSON.parse(output)).toEqual({
    result: { ok: null },
    count: 2,
    lines: [2, 2],
    allSameSite: true,
  })
})
