import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const fixtureRules = [
  {
    id: 'checkout-submit',
    sitePattern: 'amazon.com',
    description: 'payments and checkout',
    enabled: true,
  },
  {
    id: 'disabled',
    sitePattern: 'amazon.com',
    description: 'disabled',
    enabled: false,
  },
]

describe('GlobalAclPolicyService', () => {
  let rootDir: string
  let service: {
    setRules(rules: typeof fixtureRules): Promise<unknown>
    getRules(): unknown
    getEnabledRules(): unknown
    load(): Promise<void>
  }

  beforeEach(async () => {
    rootDir = await mkdtemp(join(tmpdir(), 'browseros-acl-test-'))
    mock.module('../../../src/lib/browseros-dir', () => ({
      getBrowserosDir: () => rootDir,
    }))
    const mod = await import('../../../src/api/services/acl/global-acl-policy')
    service = new mod.GlobalAclPolicyService()
  })

  afterEach(async () => {
    mock.restore()
    await rm(rootDir, { recursive: true, force: true })
  })

  it('persists all rules and exposes enabled rules separately', async () => {
    const saved = await service.setRules(fixtureRules)

    expect(saved).toHaveLength(2)
    expect(service.getEnabledRules()).toEqual([
      {
        id: 'checkout-submit',
        sitePattern: 'amazon.com',
        description: 'payments and checkout',
        enabled: true,
      },
    ])
    expect(saved[0]?.id).toBe('checkout-submit')

    const raw = await readFile(join(rootDir, 'acl-rules.json'), 'utf8')
    expect(raw).toContain('checkout-submit')
    expect(raw).toContain('"disabled"')
  })

  it('reloads persisted rules from disk', async () => {
    await service.setRules(fixtureRules)

    const mod = await import('../../../src/api/services/acl/global-acl-policy')
    const reloaded = new mod.GlobalAclPolicyService()
    await reloaded.load()

    expect(reloaded.getRules()).toEqual([
      {
        id: 'checkout-submit',
        sitePattern: 'amazon.com',
        description: 'payments and checkout',
        enabled: true,
      },
      {
        id: 'disabled',
        sitePattern: 'amazon.com',
        description: 'disabled',
        enabled: false,
      },
    ])
    expect(reloaded.getEnabledRules()).toEqual([
      {
        id: 'checkout-submit',
        sitePattern: 'amazon.com',
        description: 'payments and checkout',
        enabled: true,
      },
    ])
  })
})
