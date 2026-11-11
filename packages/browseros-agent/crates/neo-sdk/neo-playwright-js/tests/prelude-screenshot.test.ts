// Screenshot client ordering and MIME inference, over the real bootstrap byte codec.
import { expect, it } from 'bun:test'
import { isRefusal, loadPrelude, SEED } from './support/prelude-runtime'

const IMAGE = { $neo: 'bytes', base64: 'iVBORw0KGgo=' }

it('retains caller masks in capture when a screenshot is also saved to a path', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => (name === 'page.screenshot' ? IMAGE : null),
  })
  await runtime.run(`await page.screenshot({
    path: 'masked.png', mask: [page.locator('#secret')], maskColor: '#123456'
  })`)
  expect(runtime.sent('page.screenshot')[0]?.input).toMatchObject({
    type: 'png',
    maskColor: '#123456',
    mask: [{ $neo: 'locator' }],
  })
  expect(runtime.sent('page.saveScreenshot')[0]?.input).toEqual({
    call: { api: 'page.screenshot', timeoutMs: 0 },
    path: 'masked.png',
    base64: IMAGE.base64,
  })
})

it('saves element-handle screenshots with their pinned target, explicit type and API alias', async () => {
  const element = { $neo: 'element', page: SEED.page, seq: 1, node: 9 }
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => {
      if (name === 'locator.elementHandle') return element
      return name === 'page.screenshot' ? IMAGE : null
    },
  })
  const result = await runtime.run(`
    const handle = await page.locator('#a').elementHandle();
    const image = await handle.screenshot({ path: 'nested/image.png', type: 'jpeg' });
    return [Buffer.isBuffer(image), image.toString('base64')];
  `)
  expect(JSON.parse(JSON.stringify(result))).toEqual([true, IMAGE.base64])
  const capture = runtime.sent('page.screenshot')[0]
  expect(capture?.input.target).toMatchObject(element)
  expect(capture?.input.type).toBe('jpeg')
  expect(runtime.sent('page.saveScreenshot')[0]?.input).toEqual({
    call: { api: 'elementHandle.screenshot', timeoutMs: 0 },
    path: 'nested/image.png',
    base64: IMAGE.base64,
  })
})

it('infers upstream filename types, preserves explicit types and saves the captured bytes last', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => (name === 'page.screenshot' ? IMAGE : null),
  })
  const result = await runtime.run(`
    const out = [];
    for (const options of [
      { path: 'nested/shot.png' }, { path: 'shot.jpg' }, { path: 'shot.jpe' },
      { path: 'shot.jpeg' }, { path: 'shot.webp' },
      { path: 'shot.txt', type: 'jpeg' }, { path: 'shot.JPG' },
    ]) {
      const image = await page.screenshot(options);
      out.push([Buffer.isBuffer(image), image.toString('base64')]);
    }
    return out;
  `)
  expect(JSON.parse(JSON.stringify(result))).toEqual(
    Array.from({ length: 7 }, () => [true, IMAGE.base64]),
  )
  expect(runtime.calls.map((c) => c.name)).toEqual(
    Array.from({ length: 7 }, () => [
      'page.screenshot',
      'page.saveScreenshot',
    ]).flat(),
  )
  expect(runtime.sent('page.screenshot').map((c) => c.input.type)).toEqual([
    'png',
    'jpeg',
    'jpeg',
    'jpeg',
    'webp',
    'jpeg',
    'jpeg',
  ])
  for (const call of runtime.sent('page.saveScreenshot')) {
    expect(call.input.base64).toBe(IMAGE.base64)
    expect(call.input.call).toEqual({ api: 'page.screenshot', timeoutMs: 0 })
    expect(call.input.page).toBeUndefined()
  }
  expect(runtime.sent('page.saveScreenshot').at(-1)?.input.path).toBe(
    'shot.JPG',
  )
})

it('keeps upstream path errors before capture, and does not save failed or pathless captures', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => (name === 'page.screenshot' ? IMAGE : null),
  })
  const errors = await runtime.run(`
    const errors = [];
    for (const path of ['shot.txt', 'shot']) {
      try { await page.screenshot({ path }); } catch (e) { errors.push(e.message); }
    }
    await page.screenshot(); await page.screenshot({ path: '' });
    return errors;
  `)
  expect(JSON.parse(JSON.stringify(errors))).toEqual([
    'path: unsupported mime type "text/plain"',
    'path: unsupported mime type "null"',
  ])
  expect(
    runtime.sent('page.screenshot').filter((call) => !isRefusal(call)),
  ).toHaveLength(2)
  expect(runtime.sent('page.saveScreenshot')).toHaveLength(0)

  const failed = loadPrelude({
    seed: SEED,
    answer: ({ name }) =>
      name === 'page.screenshot'
        ? {
            err: {
              owner: 'website',
              code: 'target_closed',
              message: 'capture failed',
            },
          }
        : null,
  })
  await expect(
    failed.run("await page.screenshot({ path: 'shot.png' })"),
  ).rejects.toThrow('capture failed')
  expect(failed.sent('page.saveScreenshot')).toHaveLength(0)
})

it('finishes the save before resolving, and retains the calling locator alias on its file op', async () => {
  let release = () => {}
  let started = () => {}
  const saving = new Promise<null>((resolve) => {
    release = () => resolve(null)
  })
  const savingStarted = new Promise<void>((resolve) => {
    started = resolve
  })
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => {
      if (name === 'page.screenshot') return IMAGE
      if (name === 'page.saveScreenshot') {
        started()
        return saving
      }
      return null
    },
  })
  let done = false
  const running = runtime
    .run("return await page.locator('#a').screenshot({ path: 'shot.png' })")
    .then(() => {
      done = true
    })
  await savingStarted
  expect(done).toBe(false)
  expect(runtime.sent('page.saveScreenshot')[0]?.input.call).toEqual({
    api: 'locator.screenshot',
    timeoutMs: 0,
  })
  release()
  await running
  expect(done).toBe(true)
})
