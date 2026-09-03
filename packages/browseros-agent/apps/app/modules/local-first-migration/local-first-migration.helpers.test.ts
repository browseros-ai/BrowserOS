import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ScheduledJob } from '@/lib/schedules/scheduleTypes'
import {
  isImportableJob,
  isImportableProvider,
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

describe('isImportableProvider', () => {
  it('accepts a well formed provider', () => {
    expect(isImportableProvider(provider())).toBe(true)
  })

  // A single entry the server rejects returns 400 for the whole batch, and
  // because the pref backup has no migration path that failure would repeat on
  // every startup with nothing the user could do about it.
  it.each([
    ['no id', { id: '' }],
    ['no type', { type: '' }],
    ['no name', { name: '' }],
    ['no model', { modelId: '' }],
  ])('rejects a provider with %s', (_label, overrides) => {
    expect(isImportableProvider(provider(overrides as never))).toBe(false)
  })

  it('rejects a provider whose context window is not a number', () => {
    expect(
      isImportableProvider({ ...provider(), contextWindow: '200000' }),
    ).toBe(false)
    expect(isImportableProvider({ ...provider(), contextWindow: NaN })).toBe(
      false,
    )
  })

  // Storage migrations drop these; the pref backup never gets that treatment.
  it('rejects provider types that no longer exist', () => {
    for (const type of [
      'remote-hermes',
      'claude-code',
      'codex',
      'acp-custom',
    ]) {
      expect(isImportableProvider(provider({ type } as never))).toBe(false)
    }
  })

  it('rejects values that are not objects', () => {
    expect(isImportableProvider(null)).toBe(false)
    expect(isImportableProvider('provider')).toBe(false)
  })
})

describe('isImportableJob', () => {
  it('accepts a well formed job', () => {
    expect(isImportableJob(job())).toBe(true)
  })

  it('rejects a job missing the fields the server requires', () => {
    expect(isImportableJob(job({ name: '' }))).toBe(false)
    expect(isImportableJob(job({ query: '' }))).toBe(false)
  })

  it('rejects an unrecognised schedule type', () => {
    expect(isImportableJob(job({ scheduleType: 'weekly' } as never))).toBe(
      false,
    )
  })
})

describe('optional field sanitising', () => {
  // The provider is valid where it matters, so it should still import; the
  // junk field is dropped and the server applies its own default.
  it('drops an optional field holding the wrong type', () => {
    const imported = toProviderImport({
      ...provider(),
      temperature: 'warm',
      supportsImages: 'yes',
      baseUrl: 42,
    } as never)

    expect(imported.temperature).toBeUndefined()
    expect(imported.supportsImages).toBeUndefined()
    expect(imported.baseUrl).toBeUndefined()
    expect(imported.modelId).toBe('gpt-5.5')
  })

  it('keeps optional fields that are the right type', () => {
    const imported = toProviderImport(
      provider({ baseUrl: 'https://api.openai.com/v1' }),
    )
    expect(imported.baseUrl).toBe('https://api.openai.com/v1')
    expect(imported.temperature).toBe(0.2)
    expect(imported.supportsImages).toBe(true)
  })

  it('drops a job field holding the wrong type', () => {
    const imported = toScheduledJobImport({
      ...job(),
      scheduleInterval: 'hourly',
      enabled: 'true',
    } as never)

    expect(imported.scheduleInterval).toBeUndefined()
    expect(imported.enabled).toBeUndefined()
    expect(imported.name).toBe('Morning digest')
  })
})
