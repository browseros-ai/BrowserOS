import { describe, expect, it } from 'bun:test'
import { generateFunName } from '../../../src/lib/mcp-session/fun-names'

describe('generateFunName', () => {
  it('draws an adjective-animal name from the word lists', () => {
    expect(generateFunName({ random: () => 0 })).toBe('agile-alpaca')
  })

  it('redraws while a generated name is unavailable', () => {
    const draws = [0, 0, 0.03, 0.03]
    const checked: string[] = []
    const name = generateFunName({
      random: () => draws.shift() ?? 0.03,
      isAvailable(candidate) {
        checked.push(candidate)
        return candidate !== 'agile-alpaca'
      },
    })

    expect(name).not.toBe('agile-alpaca')
    expect(checked).toEqual(['agile-alpaca', name])
  })

  it('appends a short numeric suffix after bounded redraw collisions', () => {
    const checked: string[] = []
    const name = generateFunName({
      random: () => 0,
      isAvailable(candidate) {
        checked.push(candidate)
        return candidate === 'agile-alpaca-2'
      },
    })

    expect(name).toBe('agile-alpaca-2')
    expect(
      checked.filter((candidate) => candidate === 'agile-alpaca'),
    ).toHaveLength(5)
    expect(checked.at(-1)).toBe('agile-alpaca-2')
  })
})
