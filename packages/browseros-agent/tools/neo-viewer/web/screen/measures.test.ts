import { describe, expect, it } from 'bun:test'
import { runSummary } from '../test-support/runs'
import { barScale, neoMs, runTotals, share, tokenBar } from './measures'

describe('the run bars', () => {
  it("measures Neo's time as the round trip less the time the viewer held the call", () => {
    expect(neoMs(runSummary(1, { roundTripMs: 431, proxyMs: 19 }))).toBe(412)
    expect(neoMs(runSummary(2, { roundTripMs: null }))).toBeNull()
    // A clock step can make the hold look longer than the round trip; never a negative bar.
    expect(neoMs(runSummary(3, { roundTripMs: 5, proxyMs: 9 }))).toBe(0)
  })

  it("uses Neo's own record of the run once it is joined", () => {
    const joined = {
      ...runSummary(1, { roundTripMs: 431, proxyMs: 19 }),
      neoDurationMs: 398,
    }
    expect(neoMs(joined)).toBe(398)
  })

  it('totals the calls Neo recorded over the runs whose record is joined', () => {
    const runs = [
      { ...runSummary(1), operationCount: 6 },
      { ...runSummary(2), operationCount: 10 },
      runSummary(3),
    ]
    expect(runTotals(runs).calls).toBe(16)
    expect(runTotals([runSummary(1)]).calls).toBeNull()
  })

  it('draws each bar as a share of the largest value in its column', () => {
    const runs = [
      runSummary(1, { roundTripMs: 412 }),
      runSummary(2, { roundTripMs: 849 }),
      runSummary(3, { roundTripMs: null }),
    ]
    const scale = barScale(runs)
    expect(scale.neoMs).toBe(849)
    expect(share(412, scale.neoMs)).toBeCloseTo(0.4853, 4)
    expect(share(849, scale.neoMs)).toBe(1)
    expect(share(0, scale.neoMs)).toBe(0)
    expect(share(10, 0)).toBe(0)
  })

  it('stacks the code and reply tokens on one scale, the largest run filling the track', () => {
    const small = runSummary(1, {
      code: 'x'.repeat(40),
      reply: `### Result\n"${'y'.repeat(8)}"`,
    })
    const large = runSummary(2, {
      code: 'x'.repeat(120),
      reply: `### Result\n"${'y'.repeat(888)}"`,
    })
    const running = runSummary(3, { code: 'x'.repeat(164), roundTripMs: null })
    const scale = barScale([small, large, running])
    // ≈30 + ≈225 tokens = 255, the widest bar.
    expect(scale.tokens).toBe(255)
    expect(tokenBar(large, scale)).toEqual({ code: 30 / 255, reply: 225 / 255 })
    expect(tokenBar(small, scale)).toEqual({ code: 10 / 255, reply: 5 / 255 })
    // A running run has written its code but not read a reply yet.
    expect(tokenBar(running, scale)).toEqual({ code: 41 / 255, reply: 0 })
  })

  it('totals the rows on screen, a running run adding only what it has so far', () => {
    const totals = runTotals([
      runSummary(1, {
        roundTripMs: 431,
        proxyMs: 19,
        code: 'x'.repeat(88),
        reply: `### Result\n"${'y'.repeat(16)}"`,
      }),
      runSummary(2, {
        roundTripMs: 47,
        proxyMs: 13,
        gapMs: 6_400,
        code: 'x'.repeat(120),
        reply: `### Result\n"${'y'.repeat(888)}"`,
      }),
      runSummary(3, {
        roundTripMs: null,
        gapMs: 12_100,
        code: 'x'.repeat(164),
      }),
    ])
    expect(totals).toEqual({
      runs: 3,
      neoMs: 412 + 34,
      roundTripMs: 431 + 47,
      codeTokens: 22 + 30 + 41,
      replyTokens: 7 + 225,
      gapMs: 6_400 + 12_100,
      calls: null,
    })
  })
})
