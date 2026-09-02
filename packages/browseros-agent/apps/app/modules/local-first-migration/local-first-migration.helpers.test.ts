import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ScheduledJob } from '@/lib/schedules/scheduleTypes'
import {
  mergeProviderSources,
  parseProviderBackup,
  toProviderImport,
  toScheduledJobImport,
} from './local-first-migration.helpers'

function provider(
  overrides: Partial<LlmProviderConfig> = {},
): LlmProviderConfig {
  return {
    id: 'provider-1',
    type: 'openai',
    name: 'My OpenAI',
    modelId: 'gpt-5.5',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: 10,
    updatedAt: 20,
    ...overrides,
  }
}

function job(overrides: Partial<ScheduledJob> = {}): ScheduledJob {
  return {
    id: 'job-1',
    name: 'Morning digest',
    query: 'summarise my inbox',
    scheduleType: 'daily',
    scheduleTime: '09:00',
    enabled: true,
    createdAt: '2026-01-02T03:04:05.000Z',
    updatedAt: '2026-01-02T03:04:05.000Z',
    ...overrides,
  }
}

describe('parseProviderBackup', () => {
  it('reads the provider list out of the pref payload', () => {
    const raw = JSON.stringify({
      defaultProviderId: 'provider-1',
      providers: [provider()],
    })
    expect(parseProviderBackup(raw).map((p) => p.id)).toEqual(['provider-1'])
  })

  // The backup is a fallback source, so a corrupt one must not stop the
  // extension-storage providers from importing.
  it('yields nothing rather than throwing on unusable input', () => {
    expect(parseProviderBackup('not json')).toEqual([])
    expect(parseProviderBackup('null')).toEqual([])
    expect(parseProviderBackup(JSON.stringify({ providers: 'nope' }))).toEqual(
      [],
    )
    expect(parseProviderBackup(undefined)).toEqual([])
    expect(parseProviderBackup('')).toEqual([])
  })

  it('drops entries with no id', () => {
    const raw = JSON.stringify({ providers: [provider(), { name: 'junk' }] })
    expect(parseProviderBackup(raw)).toHaveLength(1)
  })
})

describe('mergeProviderSources', () => {
  // Extension storage is written on every save, so it is the current copy.
  it('keeps the stored provider when both sources have the id', () => {
    const merged = mergeProviderSources(
      [provider({ name: 'Current' })],
      [provider({ name: 'Stale backup' })],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].name).toBe('Current')
  })

  // The reinstall case: extension storage was cleared, the per-profile pref
  // outlived it, and the backup is the only remaining copy.
  it('contributes backup providers that storage no longer has', () => {
    const merged = mergeProviderSources([], [provider({ id: 'from-backup' })])
    expect(merged.map((p) => p.id)).toEqual(['from-backup'])
  })

  it('does not duplicate a provider repeated within the backup', () => {
    const merged = mergeProviderSources([], [provider(), provider()])
    expect(merged).toHaveLength(1)
  })
})

describe('toProviderImport', () => {
  it('carries the credentials across', () => {
    expect(
      toProviderImport(
        provider({
          apiKey: 'sk-test',
          accessKeyId: 'AKIA',
          secretAccessKey: 'secret',
          sessionToken: 'token',
        }),
      ),
    ).toMatchObject({
      apiKey: 'sk-test',
      accessKeyId: 'AKIA',
      secretAccessKey: 'secret',
      sessionToken: 'token',
    })
  })

  it('preserves the original creation time', () => {
    expect(toProviderImport(provider()).createdAt).toBe(10)
  })
})

describe('toScheduledJobImport', () => {
  it('converts the ISO timestamps the extension holds to epoch', () => {
    const imported = toScheduledJobImport(
      job({ lastRunAt: '2026-01-03T00:00:00.000Z' }),
    )
    expect(imported.createdAt).toBe(Date.parse('2026-01-02T03:04:05.000Z'))
    expect(imported.lastRunAt).toBe(Date.parse('2026-01-03T00:00:00.000Z'))
  })

  // NaN would fail validation and take the whole batch down with it, so the
  // job lands with the server's own timestamp instead.
  it('drops an unparseable timestamp rather than sending NaN', () => {
    const imported = toScheduledJobImport(job({ createdAt: 'whenever' }))
    expect(imported.createdAt).toBeUndefined()
    expect(imported.name).toBe('Morning digest')
  })

  it('leaves an absent lastRunAt absent', () => {
    expect(toScheduledJobImport(job()).lastRunAt).toBeUndefined()
  })
})
