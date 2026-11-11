// page.localStorage and page.sessionStorage: upstream's WebStorage as one op per call.
import { describe, expect, it } from 'bun:test'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

describe('WebStorage', () => {
  it('tests that each method sends its read or write op, without a timeout, and returns upstream values', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'webStorage.read'
          ? call.input.action === 'items'
            ? [{ name: 'alpha', value: '1' }]
            : call.input.name === 'alpha'
              ? '1'
              : null
          : null,
    })
    const results = json(
      await runtime.run(`
        const items = await page.localStorage.items()
        const alpha = await page.localStorage.getItem('alpha')
        const absent = await page.sessionStorage.getItem('absent')
        await page.localStorage.setItem('k', 'v')
        await page.sessionStorage.removeItem('k')
        await page.localStorage.clear()
        return { items, alpha, absent, same: page.localStorage === page.localStorage }
      `),
    )
    expect(results).toEqual({
      items: [{ name: 'alpha', value: '1' }],
      alpha: '1',
      absent: null,
      same: true,
    })
    expect(
      runtime.calls.map(({ name, input }) => [
        name,
        input.call,
        input.kind,
        input.action,
        input.name,
        input.value,
      ]),
    ).toEqual([
      [
        'webStorage.read',
        { api: 'webStorage.items', timeoutMs: 0 },
        'local',
        'items',
        undefined,
        undefined,
      ],
      [
        'webStorage.read',
        { api: 'webStorage.getItem', timeoutMs: 0 },
        'local',
        'getItem',
        'alpha',
        undefined,
      ],
      [
        'webStorage.read',
        { api: 'webStorage.getItem', timeoutMs: 0 },
        'session',
        'getItem',
        'absent',
        undefined,
      ],
      [
        'webStorage.write',
        { api: 'webStorage.setItem', timeoutMs: 0 },
        'local',
        'setItem',
        'k',
        'v',
      ],
      [
        'webStorage.write',
        { api: 'webStorage.removeItem', timeoutMs: 0 },
        'session',
        'removeItem',
        'k',
        undefined,
      ],
      [
        'webStorage.write',
        { api: 'webStorage.clear', timeoutMs: 0 },
        'local',
        'clear',
        undefined,
        undefined,
      ],
    ])
  })

  it('tests that a non-string name or value is refused before any op, as upstream validates it', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const message = await runtime.run(`
      try { await page.localStorage.setItem('k', 5) } catch (e) { return e.message }
    `)
    expect(message).toBe(
      'webStorage.setItem: value: expected string, got number',
    )
    // The refusal is a door call of its own; no storage op went out.
    expect(
      runtime.calls.filter(
        (call) =>
          call.name === 'webStorage.read' || call.name === 'webStorage.write',
      ),
    ).toEqual([])
  })
})
