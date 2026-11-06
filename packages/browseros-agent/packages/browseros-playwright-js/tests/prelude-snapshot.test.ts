// ariaSnapshot() as scripts see it: Rust returns the stitched, masked JSON, and the prelude
// renders it with the vendored renderer, adding a `# partial` comment under each iframe line
// whose frame could not be read.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { renderAriaSnapshotAsYaml } from '../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

const fixture: {
  result: {
    json: Parameters<typeof renderAriaSnapshotAsYaml>[0]
    partial: unknown[]
  }
  yaml: string[]
} = JSON.parse(
  readFileSync(new URL('./data/snapshot-render.json', import.meta.url), 'utf8'),
)

function snapshotting(result: unknown) {
  return loadPrelude({
    seed: SEED,
    answer: (call) => (call.name.endsWith('.ariaSnapshot') ? result : null),
  })
}

it("tests that a snapshot without partial frames is the vendored renderer's YAML, byte for byte", async () => {
  const whole = { json: fixture.result.json, partial: [] }
  const runtime = snapshotting(whole)
  const yaml = await runtime.run(
    "return await page.ariaSnapshot({ mode: 'ai' })",
  )
  expect(yaml).toBe(renderAriaSnapshotAsYaml(fixture.result.json))
  expect(yaml).toBe(
    fixture.yaml
      .filter((line) => !line.trim().startsWith('# partial'))
      .join('\n'),
  )
})

it('tests that each frame Rust could not read gets its comment under its iframe line', async () => {
  const runtime = snapshotting(fixture.result)
  expect(
    await runtime.run("return await page.ariaSnapshot({ mode: 'ai' })"),
  ).toBe(fixture.yaml.join('\n'))
})

it('tests that a partial frame whose iframe line is missing is still reported, at the end', async () => {
  const runtime = snapshotting({
    json: [{ role: 'button', name: 'Go', ref: 'e2' }],
    partial: [
      { ref: 'e9', frame: 'https://gone.example/', reason: 'target closed' },
    ],
  })
  expect(await runtime.run('return await page.ariaSnapshot()')).toBe(
    '- button "Go" [ref=e2]\n# partial: https://gone.example/ not read (target closed)',
  )
})

it('tests that page and locator snapshots send their mode and depth, and JSON skips the rendering', async () => {
  const runtime = snapshotting(fixture.result)
  const result = json(
    await runtime.run(`
      await page.ariaSnapshot()
      await page.getByRole('main').ariaSnapshot({ mode: 'ai', depth: 3, timeout: 900 })
      return await page.ariaSnapshotJSON({ mode: 'ai' })
    `),
  )
  expect(result).toEqual(fixture.result.json)
  expect(
    runtime.calls.map(
      (call): Record<string, unknown> => ({
        name: call.name,
        ...call.input,
        target: undefined,
      }),
    ),
  ).toEqual([
    {
      name: 'page.ariaSnapshot',
      call: { api: 'page.ariaSnapshot', timeoutMs: 5000 },
      page: { $neo: 'page', id: 7, tab: 1007 },
      mode: 'default',
      target: undefined,
    },
    {
      name: 'locator.ariaSnapshot',
      call: { api: 'locator.ariaSnapshot', timeoutMs: 900 },
      strict: 'strict',
      mode: 'ai',
      depth: 3,
      target: undefined,
    },
    {
      name: 'page.ariaSnapshot',
      call: { api: 'page.ariaSnapshotJSON', timeoutMs: 5000 },
      page: { $neo: 'page', id: 7, tab: 1007 },
      mode: 'ai',
      target: undefined,
    },
  ])
})

it('tests that boxes, which Neo snapshots do not carry, are refused before anything reaches the page', async () => {
  const runtime = snapshotting(fixture.result)
  const error = json(
    await runtime.run(
      'try { await page.ariaSnapshot({ boxes: true }) } catch (e) { return { code: e.code, message: e.message } }',
    ),
  )
  const message =
    'page.ariaSnapshot: Neo snapshots carry no boxes; use locator.boundingBox()'
  expect(error).toEqual({ code: 'method_unavailable', message })
  expect(runtime.calls).toEqual([
    {
      name: 'page.ariaSnapshot',
      input: { $neo: 'refused', code: 'method_unavailable', reason: message },
    },
  ])
})
