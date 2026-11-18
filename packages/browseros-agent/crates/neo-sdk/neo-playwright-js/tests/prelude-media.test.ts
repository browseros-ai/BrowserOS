// The prelude preserves omitted-vs-null media fields and refuses invalid preferences before CDP.
import { expect, it } from 'bun:test'
import { isRefusal, json, loadPrelude, SEED } from './support/prelude-runtime'

it('clears only explicit nulls and preserves omissions on the media wire', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => null })
  const result = await runtime.run(`
    await page.emulateMedia({media:'print',colorScheme:'dark',reducedMotion:'reduce',forcedColors:'active',contrast:'more'})
    await page.emulateMedia({media:null,colorScheme:null,reducedMotion:null,forcedColors:null,contrast:null,unknown:'ignored'})
    return await page.emulateMedia({}) === undefined
  `)
  expect(result).toBe(true)
  const inputs = runtime
    .sent('page.emulateMedia')
    .map((call) => json(call.input))
  expect(inputs).toEqual([
    {
      call: { api: 'page.emulateMedia', timeoutMs: 0 },
      page: { $neo: 'page', tab: 1007 },
      media: 'print',
      colorScheme: 'dark',
      reducedMotion: 'reduce',
      forcedColors: 'active',
      contrast: 'more',
    },
    {
      call: { api: 'page.emulateMedia', timeoutMs: 0 },
      page: { $neo: 'page', tab: 1007 },
      media: 'no-override',
      colorScheme: 'no-override',
      reducedMotion: 'no-override',
      forcedColors: 'no-override',
      contrast: 'no-override',
    },
    {
      call: { api: 'page.emulateMedia', timeoutMs: 0 },
      page: { $neo: 'page', tab: 1007 },
    },
  ])
})

it('rejects each invalid media enum before submitting a browser operation', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => null })
  const failures = await runtime.run(`
    const failures = []
    for (const field of ['media','colorScheme','reducedMotion','forcedColors','contrast']) {
      try { await page.emulateMedia({[field]:'bad'}) } catch (error) { failures.push(error.message) }
    }
    return failures
  `)
  expect(json(failures)).toEqual([
    'page.emulateMedia: media: expected one of (screen|print|no-override)',
    'page.emulateMedia: colorScheme: expected one of (dark|light|no-preference|no-override)',
    'page.emulateMedia: reducedMotion: expected one of (reduce|no-preference|no-override)',
    'page.emulateMedia: forcedColors: expected one of (active|none|no-override)',
    'page.emulateMedia: contrast: expected one of (no-preference|more|no-override)',
  ])
  expect(runtime.sent('page.emulateMedia').every(isRefusal)).toBe(true)
})
