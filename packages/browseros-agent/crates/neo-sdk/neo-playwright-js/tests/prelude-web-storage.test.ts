// page.localStorage and page.sessionStorage: upstream's WebStorage as one op per call.
import { describe, expect, it } from 'bun:test'
import { isRefusal, json, loadPrelude, SEED } from './support/prelude-runtime'

describe('WebStorage', () => {
  it('tests that each method sends its own op, without a timeout, and returns upstream values', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'webStorage.items'
          ? [{ name: 'alpha', value: '1' }]
          : call.name === 'webStorage.getItem' && call.input.name === 'alpha'
            ? '1'
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
        input.name,
        input.value,
      ]),
    ).toEqual([
      [
        'webStorage.items',
        { api: 'webStorage.items', timeoutMs: 0 },
        'local',
        undefined,
        undefined,
      ],
      [
        'webStorage.getItem',
        { api: 'webStorage.getItem', timeoutMs: 0 },
        'local',
        'alpha',
        undefined,
      ],
      [
        'webStorage.getItem',
        { api: 'webStorage.getItem', timeoutMs: 0 },
        'session',
        'absent',
        undefined,
      ],
      [
        'webStorage.setItem',
        { api: 'webStorage.setItem', timeoutMs: 0 },
        'local',
        'k',
        'v',
      ],
      [
        'webStorage.removeItem',
        { api: 'webStorage.removeItem', timeoutMs: 0 },
        'session',
        'k',
        undefined,
      ],
      [
        'webStorage.clear',
        { api: 'webStorage.clear', timeoutMs: 0 },
        'local',
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
        (call) => call.name.startsWith('webStorage.') && !isRefusal(call),
      ),
    ).toEqual([])
  })
})
