import { expect, test } from 'bun:test'
import { FRAME_CLICK_KEYS, pickParams } from '../../src/protocol'

test('params keep only schema keys, in schema order, without undefined values', () => {
  const params = pickParams(FRAME_CLICK_KEYS, {
    timeout: 2000,
    trial: undefined,
    button: 'right',
    signal: 'not a param',
    selector: '#go',
    position: { x: 1, y: 2 },
    strict: true,
  })
  expect(Object.entries(params)).toEqual([
    ['selector', '#go'],
    ['strict', true],
    ['position', { x: 1, y: 2 }],
    ['button', 'right'],
  ])
  expect(JSON.stringify(params)).toBe(
    '{"selector":"#go","strict":true,"position":{"x":1,"y":2},"button":"right"}',
  )
})
