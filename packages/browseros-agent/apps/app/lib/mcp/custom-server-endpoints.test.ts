import { expect, test } from 'bun:test'
import { customServerEndpoints } from './custom-server-endpoints'

test('filters incomplete legacy entries while retaining valid custom URL bytes', () => {
  expect(
    customServerEndpoints([
      {
        type: 'custom',
        displayName: 'Ready',
        config: { url: 'http://localhost:9012/mcp?token=fixture' },
      },
      { type: 'custom', displayName: 'Missing' },
      { type: 'custom', displayName: 'Empty', config: { url: '' } },
      { type: 'custom', displayName: 'Invalid', config: { url: 'not a URL' } },
      {
        type: 'managed',
        displayName: 'Managed',
        config: { url: 'https://example.com/mcp' },
      },
    ]),
  ).toEqual([{ name: 'Ready', url: 'http://localhost:9012/mcp?token=fixture' }])
})
