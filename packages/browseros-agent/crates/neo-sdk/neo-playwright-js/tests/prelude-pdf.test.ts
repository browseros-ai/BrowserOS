/** Numeric PDF lengths belong to the script API; only CSS strings cross the channel. */
import { expect, it } from 'bun:test'
import { isRefusal, loadPrelude, SEED } from './support/prelude-runtime'

it('normalizes both dimensions and all margin sides while preserving explicit CSS units', async () => {
  const runtime = loadPrelude({ seed: SEED })
  await runtime.run(`await page.pdf({
    width: 960, height: 0, margin: { top: 96, right: 48, bottom: 0, left: -1 },
  }); await page.pdf({ width: '10in', height: '25.4cm', margin: { top: '1mm' } })`)
  const [pixels, css] = runtime.sent('page.pdf')
  expect(pixels?.input).toMatchObject({
    width: '960px',
    height: '0px',
    margin: { top: '96px', right: '48px', bottom: '0px', left: '-1px' },
  })
  expect(css?.input).toMatchObject({
    width: '10in',
    height: '25.4cm',
    margin: { top: '1mm' },
  })
})

it('keeps caller margin getters and unknown fields for the existing host validation', async () => {
  const runtime = loadPrelude({ seed: SEED })
  expect(
    await runtime.run(`
    let reads = 0
    await page.pdf({ margin: { get top() { ++reads; return 12 }, extra: true } })
    return reads
  `),
  ).toBe(1)
  expect(runtime.sent('page.pdf')[0]?.input.margin).toEqual({
    top: '12px',
    extra: true,
  })
})

it('continues to refuse host-file PDF paths before sending a PDF command', async () => {
  const runtime = loadPrelude({ seed: SEED })
  expect(
    await runtime.run(
      `try { await page.pdf({path:'/tmp/report.pdf'}) } catch(e) { return e.message }`,
    ),
  ).toBe('page.pdf: Neo returns the PDF instead of writing a file; omit path')
  expect(runtime.sent('page.pdf').filter((call) => !isRefusal(call))).toEqual(
    [],
  )
})
