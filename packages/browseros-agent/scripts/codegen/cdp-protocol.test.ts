import { describe, expect, test } from 'bun:test'
import { join } from 'node:path'

interface ProtocolParameter {
  name: string
  type?: string
  optional?: boolean
}

interface ProtocolCommand {
  name: string
  parameters?: ProtocolParameter[]
}

interface ProtocolDomain {
  domain: string
  commands?: ProtocolCommand[]
}

describe('generated CDP protocol', () => {
  test('keeps closeTabGroup parameters synchronized with the native snapshot', async () => {
    const root = join(import.meta.dir, '../..')
    const protocol = (await Bun.file(
      join(root, 'crates/browseros-cdp/protocol/protocol.json'),
    ).json()) as { domains: ProtocolDomain[] }
    const browser = protocol.domains.find(({ domain }) => domain === 'Browser')
    const close = browser?.commands?.find(
      ({ name }) => name === 'closeTabGroup',
    )
    const parameter = close?.parameters?.find(
      ({ name }) => name === 'deleteSavedGroup',
    )
    expect(parameter).toBeDefined()
    expect(parameter?.type).toBe('boolean')

    const generated = await Bun.file(
      join(root, 'packages/cdp-protocol/src/generated/domains/browser.ts'),
    ).text()
    const optional = parameter?.optional ? '?' : ''
    expect(generated).toContain(`${parameter?.name}${optional}: boolean`)
  })
})
