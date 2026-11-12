// The run envelope exactly as neo-server renders it reaches the script as the prelude reads it.
// The examples are neo-server's tests/data/envelope-examples.json, which that crate's
// tests/envelope_generated.rs pins byte for byte against the Rust that renders a real run's init
// and replies; the prelude's types for them are generated from the same Rust
// (src/prelude/generated/envelope.ts).
import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import type { Envelope, RunInit } from '../src/prelude/generated/envelope'
import { json, loadPrelude } from './support/prelude-runtime'

const pinned = JSON.parse(
  readFileSync(
    new URL(
      '../../neo-server/tests/data/envelope-examples.json',
      import.meta.url,
    ),
    'utf8',
  ),
) as Record<string, string>

function example(name: string): unknown {
  const text = pinned[name]
  if (text === undefined) throw new Error(`no pinned example ${name}`)
  return JSON.parse(text)
}

const reply = (name: string) => example(name) as Envelope
const init = (name: string) => example(name) as RunInit

/** The run page's frame tree, which a run reads before the agent's first statement. */
const frames = {
  frames: [
    { id: 'main', parentId: null, name: '', url: 'https://facts.example/' },
  ],
}

/** The fields errors.ts copies from the envelope onto the error the script catches. */
const FIELDS = `({ name: e.name, message: e.message, code: e.code, owner: e.owner,
  delivery: e.delivery, retryable: e.retryable, operationId: e.operationId, timeout: e.timeout })`

describe('the pinned run envelope', () => {
  it("tests that a run's init names its page, every tab and their facts", async () => {
    const runtime = loadPrelude({
      seed: init('init.page'),
      answer: ({ name }) => (name === 'page.frames' ? frames : null),
    })
    expect(
      json(
        await runtime.run(`return {
          url: page.url(),
          viewport: page.viewportSize(),
          workers: page.workers().map((worker) => worker.url()),
          pages: context.pages().map((open) => open.url()),
        }`),
      ),
    ).toEqual({
      url: 'https://facts.example/',
      viewport: { width: 800, height: 600 },
      workers: ['https://facts.example/worker.js'],
      // Tab 4 is closed, with a null url the reader leaves out.
      pages: ['https://facts.example/'],
    })
  })

  it("tests that a reply's facts give the run's lazy page its tab and its state", async () => {
    const runtime = loadPrelude({
      seed: init('init.lazy'),
      answer: ({ name }) => (name === 'page.title' ? reply('reply.ok') : null),
    })
    expect(
      json(
        await runtime.run(`return {
          title: await page.title(),
          url: page.url(),
          viewport: page.viewportSize(),
          pages: context.pages().length,
        }`),
      ),
    ).toEqual({
      title: 'Facts',
      url: 'https://facts.example/',
      viewport: { width: 800, height: 600 },
      // Tab 4's close is one the observation approved (`closes`): the page closes when its
      // close event is delivered, in event order, not on the fact.
      pages: 2,
    })
    expect(runtime.sent('page.title')[0]?.input.page).toEqual({
      $neo: 'page',
      tab: null,
    })
  })

  it('tests that a bare answer is the value alone and changes no page', async () => {
    const runtime = loadPrelude({
      seed: init('init.page'),
      answer: ({ name }) =>
        name === 'page.frames'
          ? frames
          : name === 'page.title'
            ? reply('reply.okBare')
            : null,
    })
    expect(
      json(
        await runtime.run(
          'return { title: await page.title(), url: page.url(), pages: context.pages().length }',
        ),
      ),
    ).toEqual({ title: null, url: 'https://facts.example/', pages: 1 })
  })

  it("tests that a failed matcher's envelope reaches the script with every field", async () => {
    const runtime = loadPrelude({
      seed: init('init.page'),
      answer: ({ name }) =>
        name === 'page.frames'
          ? frames
          : name === 'expect.locator'
            ? reply('reply.err')
            : null,
    })
    expect(
      json(
        await runtime.run(`
          try {
            await expect(page.locator('h1')).toHaveText('Done', { timeout: 1 })
          } catch (e) {
            return { ...${FIELDS}, received: e.matcherResult.received }
          }`),
      ),
    ).toEqual({
      name: 'Error',
      message: 'expect(locator).toHaveText(expected) failed',
      code: 'assertion_failed',
      owner: 'website',
      delivery: 'done',
      retryable: true,
      operationId: '0196a0c4-1f2e-7a3b-8c4d-5e6f7a8b9c0d.4',
      timeout: true,
      received: 'Draft',
    })
  })

  it("tests that a refused op's bare envelope reaches the script with every field", async () => {
    const runtime = loadPrelude({
      seed: init('init.page'),
      answer: ({ name }) =>
        name === 'page.frames'
          ? frames
          : name === 'locator.click'
            ? reply('reply.errBare')
            : null,
    })
    expect(
      json(
        await runtime.run(`
          try { await page.click('#go') } catch (e) { return ${FIELDS} }`),
      ),
    ).toEqual({
      name: 'Error',
      message: 'page.click: selector: expected string',
      code: 'invalid_input',
      owner: 'script',
      delivery: 'not_sent',
      retryable: false,
      operationId: '0196a0c4-1f2e-7a3b-8c4d-5e6f7a8b9c0d.4',
      timeout: false,
    })
  })
})
