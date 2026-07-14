import { describe, expect, test } from 'bun:test'

import type { ClientConfig } from '../../src/_catalog/types'
import { validateCatalog } from '../../src/_catalog/validate'

const NOW = new Date('2026-07-06T00:00:00Z')

function baseline(overrides: Partial<ClientConfig> = {}): ClientConfig {
  return {
    id: 'cursor',
    displayName: 'Cursor',
    installCheckPaths: { darwin: ['/Applications/Cursor.app'] },
    systemPaths: { darwin: ['$HOME/.cursor/mcp.json'] },
    format: 'json',
    supportedTransports: { system: ['stdio'] },
    stdio: { topLevelKey: 'mcpServers' },
    sources: {
      firstParty: 'https://docs.cursor.com/context/model-context-protocol',
      verified: '2026-07-01',
    },
    ...overrides,
  }
}

describe('validateCatalog', () => {
  test('accepts a minimal valid entry', () => {
    const errors = validateCatalog([baseline()], NOW)
    expect(errors).toEqual([])
  })

  test('rejects missing firstParty citation', () => {
    const errors = validateCatalog(
      [
        baseline({
          sources: {
            firstParty: '',
            verified: '2026-07-01',
          },
        }),
      ],
      NOW,
    )
    expect(errors.map((e) => e.path)).toContain('sources.firstParty')
  })

  test('rejects a non-URL firstParty value', () => {
    const errors = validateCatalog(
      [
        baseline({
          sources: {
            firstParty: 'not a url',
            verified: '2026-07-01',
          },
        }),
      ],
      NOW,
    )
    expect(
      errors.find((e) => e.path === 'sources.firstParty')?.message,
    ).toContain('http(s) URL')
  })

  test('rejects a malformed verified date', () => {
    const errors = validateCatalog(
      [
        baseline({
          sources: {
            firstParty: 'https://example.com',
            verified: 'yesterday',
          },
        }),
      ],
      NOW,
    )
    expect(
      errors.find((e) => e.path === 'sources.verified')?.message,
    ).toContain('ISO date')
  })

  test('rejects a verified date more than 365 days old', () => {
    const errors = validateCatalog(
      [
        baseline({
          sources: {
            firstParty: 'https://example.com',
            verified: '2024-01-01',
          },
        }),
      ],
      NOW,
    )
    expect(
      errors.find((e) => e.path === 'sources.verified')?.message,
    ).toContain('re-verified')
  })

  test('accepts a smithery citation URL when present', () => {
    const errors = validateCatalog(
      [
        baseline({
          sources: {
            firstParty: 'https://example.com',
            smithery: 'https://github.com/smithery-ai/cli/blob/main/x',
            verified: '2026-07-01',
          },
        }),
      ],
      NOW,
    )
    expect(errors).toEqual([])
  })

  test('rejects a non-URL smithery value', () => {
    const errors = validateCatalog(
      [
        baseline({
          sources: {
            firstParty: 'https://example.com',
            smithery: 'not a url',
            verified: '2026-07-01',
          },
        }),
      ],
      NOW,
    )
    expect(
      errors.find((e) => e.path === 'sources.smithery')?.message,
    ).toContain('http(s) URL')
  })

  test('rejects an entry with no systemPaths for any OS', () => {
    const errors = validateCatalog(
      [
        baseline({
          systemPaths: {},
        }),
      ],
      NOW,
    )
    expect(errors.find((e) => e.path === 'systemPaths')).toBeDefined()
  })

  test('requires an http shape when system transports include http', () => {
    const errors = validateCatalog(
      [
        baseline({
          supportedTransports: { system: ['stdio', 'http'] },
        }),
      ],
      NOW,
    )
    expect(errors.find((e) => e.path === 'http')?.message).toContain(
      'no http shape is declared',
    )
  })

  test('requires an http shape when system transports include sse', () => {
    const errors = validateCatalog(
      [
        baseline({
          supportedTransports: { system: ['stdio', 'sse'] },
        }),
      ],
      NOW,
    )
    expect(errors.find((e) => e.path === 'http')).toBeDefined()
  })

  test('rejects an http shape when system supports stdio only', () => {
    const errors = validateCatalog(
      [
        baseline({
          supportedTransports: { system: ['stdio'] },
          http: { tagKey: 'type', tagValue: 'http' },
        }),
      ],
      NOW,
    )
    expect(errors.find((e) => e.path === 'http')?.message).toContain(
      'does not include http or sse',
    )
  })

  test('requires projectFile and project.stdio when project transports declared', () => {
    const errors = validateCatalog(
      [
        baseline({
          supportedTransports: { system: ['stdio'], project: ['stdio'] },
        }),
      ],
      NOW,
    )
    expect(errors.some((e) => e.path === 'projectFile')).toBe(true)
    expect(errors.some((e) => e.path === 'project.stdio')).toBe(true)
  })

  test('requires project.http when project transports include http', () => {
    const errors = validateCatalog(
      [
        baseline({
          supportedTransports: {
            system: ['stdio'],
            project: ['stdio', 'http'],
          },
          projectFile: '.foo/mcp.json',
          project: { stdio: { topLevelKey: 'mcpServers' } },
        }),
      ],
      NOW,
    )
    expect(errors.find((e) => e.path === 'project.http')).toBeDefined()
  })

  test('flags duplicate ids across the catalog', () => {
    const errors = validateCatalog([baseline(), baseline()], NOW)
    expect(errors.find((e) => e.clientId === '<duplicate>')).toBeDefined()
  })
})
