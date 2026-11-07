// Neo's documented run-lifetime clock deviation: restore native timing APIs and
// cancel pending fake timers. Native page probes run before and after the run;
// the paused state and pending timer are checked before cleanup as controls.
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  launchBrowser,
  openTab,
  runCleanups,
  runInNeo,
  serveFixture,
  skip,
} from './harness.mjs'

async function fixture(run) {
  const cleanups = []
  try {
    const neoRun = buildNeoRun()
    const origin = await serveFixture(cleanups, {
      pages: {
        '/clock-review': '<title>clock cleanup review</title><p>fixture</p>',
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const { targetId, sessionId } = await openTab(cdp, origin + '/clock-review')
    const folder = mkdtempSync(join(tmpdir(), 'neo-clock-cleanup-review-'))
    cleanups.push(() => rmSync(folder, { recursive: true, force: true }))
    let next = 0
    const execute = async (source) => {
      const path = join(folder, 'run-' + ++next + '.js')
      writeFileSync(path, source)
      const result = await runInNeo(neoRun, browser.port, targetId, path)
      assert.equal(result.error, undefined, JSON.stringify(result))
      return result.value
    }
    const evaluate = async (expression) => {
      const result = await cdp.send(
        'Runtime.evaluate',
        { expression, returnByValue: true, awaitPromise: true },
        sessionId,
      )
      assert.equal(result.exceptionDetails, undefined, JSON.stringify(result))
      return result.result.value
    }
    await run({ execute, evaluate })
  } finally {
    await runCleanups(cleanups)
  }
}

const TIMING_APIS = `async()=>{
  const before=new Event('probe').timeStamp
  const signal=AbortSignal.timeout(10)
  const aborts=await new Promise(resolve=>{
    if(signal.aborted)resolve(true)
    else signal.addEventListener('abort',()=>resolve(true),{once:true})
    setTimeout(()=>resolve(false),1000)
  })
  await new Promise(resolve=>setTimeout(resolve,25))
  return {realDate:Date.now()>1600000000000,clock:typeof globalThis.__pwClock,
    aborts,eventTimeAdvances:new Event('probe').timeStamp>before}
}`

test('run end restores timing APIs in the document and a later run', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await fixture(async ({ execute, evaluate }) => {
    const before = await evaluate('(' + TIMING_APIS + ')()')
    assert.deepEqual(
      before,
      {
        realDate: true,
        clock: 'undefined',
        aborts: true,
        eventTimeAdvances: true,
      },
      'native control',
    )
    const during = await execute(
      `await page.clock.install({time:0});await page.clock.pauseAt(1000000);return await page.evaluate(()=>Date.now())`,
    )
    assert.equal(during, 1000000)
    const after = await evaluate('(' + TIMING_APIS + ')()')
    const later = await execute(
      'return await page.evaluate(' + TIMING_APIS + ')',
    )
    t.diagnostic(JSON.stringify({ before, during, after, later }))
    assert.deepEqual(
      after,
      before,
      'every timing API returns to its native behavior after run end',
    )
    assert.deepEqual(later, before, 'a later run sees native timing APIs')
  })
})

test('a pending fake timer never fires after its resumed clock run ends', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await fixture(async ({ execute, evaluate }) => {
    const during = await execute(`await page.clock.install({time:0})
await page.clock.pauseAt(1000000)
await page.evaluate(()=>{window.oldRunTimer=0;setTimeout(()=>window.oldRunTimer++,3000)})
await page.clock.resume()
return await page.evaluate(()=>window.oldRunTimer)`)
    assert.equal(during, 0, 'the timer was pending when the script ended')
    const returned = await evaluate('window.oldRunTimer')
    const after = await evaluate(
      '(async()=>{await new Promise(resolve=>setTimeout(resolve,3500));return {fired:window.oldRunTimer,clock:typeof globalThis.__pwClock,realDate:Date.now()>1600000000000}})()',
    )
    t.diagnostic(JSON.stringify({ during, returned, after }))
    assert.equal(
      returned,
      0,
      'the pending timer has not fired when the run returns',
    )
    assert.deepEqual(
      after,
      { fired: 0, clock: 'undefined', realDate: true },
      'ending the run cancels its fake timers',
    )
  })
})
