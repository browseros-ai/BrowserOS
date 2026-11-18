import { expect, test } from 'bun:test'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { UNSUPPORTED } from '../../../neo-playwright-js/src/prelude/unsupported'

test('the judge uses exactly the production refusal table', () => {
  const actual = Object.entries(UNSUPPORTED)
    .flatMap(([name, members]) =>
      Object.keys(members).map(
        (member) => `${name[0].toLowerCase() + name.slice(1)}.${member}`,
      ),
    )
    .sort()
  expect(
    JSON.parse(readFileSync(new URL('../unsupported.json', import.meta.url))),
  ).toEqual(actual)
})
test.skipIf(!process.env.PLAYWRIGHT_SRC)(
  'source pin, registration parity and generated program syntax',
  () => {
    // Catalog registration must use the same Node host as the CLI. Executing the registrar in
    // Bun changes Function#toString and registration of upstream's generated matcher tests.
    const here = resolve(import.meta.dirname, '..')
    const code = `
 import {materialize} from ${JSON.stringify(new URL('./upstream.mjs', import.meta.url).href)};
 import {load} from ${JSON.stringify(new URL('./loader.mjs', import.meta.url).href)};
 import {parity} from ${JSON.stringify(new URL('./parity.mjs', import.meta.url).href)};
 const here=${JSON.stringify(here)};
 const pin=materialize({checkout:process.env.PLAYWRIGHT_SRC,cache:${JSON.stringify(resolve(here, '../../../../target/conformance'))}});
 const catalog=load(pin.dir,pin,here);
 const result=parity(catalog,pin,pin.dir+'/../registration-test');
 const removed=catalog.inventory.tests.find(r=>r.id.endsWith('› should fill textarea @smoke'));
 catalog.inventory.tests=catalog.inventory.tests.filter(r=>r!==removed);
 try {
  parity(catalog,pin,pin.dir+'/../registration-missing-test');
  throw new Error('removed core case passed the registration gate');
 } catch(error) {
  if (!error.message.includes('registration mismatch') || !error.message.includes('should fill textarea @smoke')) throw error;
 }
 catalog.inventory.tests.push(removed);
 const row=catalog.inventory.tests.find(r=>r.spec.endsWith('page-fill.spec.ts'));
 const program=catalog.program(row.spec,row.ordinal,{fixtures:catalog.fixtures,server:{},assetDir:''});
 const AsyncFunction=Object.getPrototypeOf(async()=>{}).constructor;
 new AsyncFunction('context','expect','setTimeout','clearTimeout',program);
 console.log(JSON.stringify({core:catalog.inventory.tests.filter(r=>r.selected&&r.spec.startsWith('tests/page/')).length,...result}));`
    const result = JSON.parse(
      execFileSync(
        process.env.CONFORMANCE_NODE || 'node',
        ['--input-type=module', '-e', code],
        { encoding: 'utf8', timeout: 60000 },
      ),
    )
    expect(result).toEqual({
      core: 633,
      registered: 2552,
      upstream: 2619,
      unmatched: 0,
    })
  },
  60000,
)
test.skipIf(!process.env.PLAYWRIGHT_SRC)(
  'a corrupted cached spec cannot claim the upstream commit',
  () => {
    const code = `
 import {materialize} from ${JSON.stringify(new URL('./upstream.mjs', import.meta.url).href)};
 import {mkdtempSync,writeFileSync,rmSync} from 'node:fs';
 import {tmpdir} from 'node:os';import {join} from 'node:path';
 const cache=mkdtempSync(join(tmpdir(),'neo-pin-'));
 try {
  const pin=materialize({checkout:process.env.PLAYWRIGHT_SRC,cache});
  writeFileSync(join(pin.dir,'tests/page/page-fill.spec.ts'),'throw new Error("tampered");');
  try {materialize({checkout:process.env.PLAYWRIGHT_SRC,cache});process.exitCode=1}
  catch(error){if(!String(error).includes('cached upstream source drift'))throw error}
 } finally {rmSync(cache,{recursive:true,force:true})}`
    expect(() =>
      execFileSync(
        process.env.CONFORMANCE_NODE || 'node',
        ['--input-type=module', '-e', code],
        { timeout: 60000 },
      ),
    ).not.toThrow()
  },
  60000,
)
