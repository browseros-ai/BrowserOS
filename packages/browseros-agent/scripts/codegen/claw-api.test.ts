import { describe, expect, test } from 'bun:test'
import { readdir, readFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { parse } from 'yaml'
import {
  emitRustModuleIndex,
  emitTypeScriptModelIndex,
  extractModelGroups,
  mergeRustModels,
  mergeTypeScriptModels,
} from './claw-api'

interface OpenApiOperation {
  operationId?: string
  parameters?: Array<{
    in?: string
    name?: string
  }>
}

interface OpenApiDocument {
  openapi?: string
  paths?: Record<string, Record<string, OpenApiOperation> & { $ref?: string }>
  components?: {
    schemas?: Record<string, unknown>
  }
}

const contractPath = join(
  import.meta.dir,
  '../../contracts/claw-api/openapi.yaml',
)
const contractDirectory = dirname(contractPath)

const expectedPaths = [
  '/system/health',
  '/system/shutdown',
  '/api/v1/system',
  '/api/v1/settings/telemetry',
  '/api/v1/recordings/events',
  '/api/v1/sessions',
  '/api/v1/sessions/{sessionId}',
  '/api/v1/sessions/{sessionId}/cancel',
  '/api/v1/sessions/{sessionId}/recording',
  '/api/v1/sessions/{sessionId}/recording/events',
  '/api/v1/sessions/{sessionId}/preview',
  '/api/v1/sessions/{sessionId}/screenshots',
  '/api/v1/sessions/{sessionId}/screenshots/{screenshotId}',
  '/api/v1/connections',
  '/api/v1/connections/{harness}',
]

const expectedPathGroups: Record<string, string> = {
  '/system/health': 'system',
  '/system/shutdown': 'system',
  '/api/v1/system': 'system',
  '/api/v1/settings/telemetry': 'settings',
  '/api/v1/recordings/events': 'recordings',
  '/api/v1/sessions': 'sessions',
  '/api/v1/sessions/{sessionId}': 'sessions',
  '/api/v1/sessions/{sessionId}/cancel': 'sessions',
  '/api/v1/sessions/{sessionId}/recording': 'recordings',
  '/api/v1/sessions/{sessionId}/recording/events': 'recordings',
  '/api/v1/sessions/{sessionId}/preview': 'sessions',
  '/api/v1/sessions/{sessionId}/screenshots': 'sessions',
  '/api/v1/sessions/{sessionId}/screenshots/{screenshotId}': 'sessions',
  '/api/v1/connections': 'connections',
  '/api/v1/connections/{harness}': 'connections',
}

const expectedModelGroups = {
  ApiError: 'common',
  AppendRecordingEventsResponse: 'recordings',
  CancelSessionResponse: 'sessions',
  Connection: 'connections',
  ConnectionList: 'connections',
  Dispatch: 'dispatches',
  Harness: 'connections',
  HealthResponse: 'system',
  LiveSessionActivityState: 'sessions',
  LiveSessionState: 'sessions',
  RecordingMetadata: 'recordings',
  RecordingSegmentMetadata: 'recordings',
  RecordingTabMetadata: 'recordings',
  SessionBrowserTab: 'sessions',
  SessionDetail: 'sessions',
  SessionList: 'sessions',
  SessionScreenshot: 'sessions',
  SessionScreenshotList: 'sessions',
  SessionStatus: 'sessions',
  SessionSummary: 'sessions',
  ShutdownResponse: 'system',
  SystemCapabilities: 'system',
  SystemInfo: 'system',
  TelemetryState: 'settings',
  ToolEvent: 'sessions',
  UpdateTelemetryRequest: 'settings',
}

const expectedOperationGroups = {
  appendRecordingEvents: 'recordings',
  cancelSession: 'sessions',
  connectHarness: 'connections',
  disconnectHarness: 'connections',
  downloadRecordingEvents: 'recordings',
  getHealth: 'system',
  getRecording: 'recordings',
  getSession: 'sessions',
  getSessionPreview: 'sessions',
  getSessionScreenshot: 'sessions',
  getSystemInfo: 'system',
  getTelemetry: 'settings',
  listConnections: 'connections',
  listSessionScreenshots: 'sessions',
  listSessions: 'sessions',
  shutdown: 'system',
  updateTelemetry: 'settings',
}

describe('BrowserClaw OpenAPI contract', () => {
  test('defines only the approved canonical surface with unique operation IDs', async () => {
    const source = await readFile(contractPath, 'utf8')
    const document = parse(source) as OpenApiDocument

    expect(document.openapi).toBe('3.0.3')
    expect(Object.keys(document.paths ?? {}).sort()).toEqual(
      expectedPaths.toSorted(),
    )

    const pathItems = await resolvePathItems(document)
    const operations = pathItems.flatMap(({ pathItem }) =>
      Object.entries(pathItem).filter(([method]) =>
        ['get', 'put', 'post', 'delete', 'patch'].includes(method),
      ),
    )
    const operationIds = operations.map(
      ([, operation]) => operation.operationId,
    )
    expect(operationIds.every(Boolean)).toBe(true)
    expect(new Set(operationIds).size).toBe(operationIds.length)
  })

  test('does not expose legacy execution identities', async () => {
    const sources = await Promise.all(
      (await yamlFiles(contractDirectory)).map((path) =>
        readFile(path, 'utf8'),
      ),
    )
    expect(sources.join('\n')).not.toMatch(/\b(?:agentId|taskId|runId)\b/)
  })

  test('owns every model and operation through its grouped source file', async () => {
    const source = await readFile(contractPath, 'utf8')
    const document = parse(source) as OpenApiDocument
    expect(
      extractModelGroups(source, Object.keys(expectedModelGroups)),
    ).toEqual(expectedModelGroups)

    expect(
      (await readdir(join(contractDirectory, 'paths'))).toSorted(),
    ).toEqual([
      'connections.yaml',
      'recordings.yaml',
      'sessions.yaml',
      'settings.yaml',
      'system.yaml',
    ])
    for (const [path, pathItem] of Object.entries(document.paths ?? {})) {
      const group = pathItem.$ref?.match(
        /^\.\/paths\/([a-z][a-z0-9_]*)\.yaml#\/[A-Za-z][A-Za-z0-9]*$/,
      )?.[1]
      expect(group).toBe(expectedPathGroups[path])
    }

    const pathItems = await resolvePathItems(document)
    const operationGroups = pathItems.flatMap(({ group, pathItem }) =>
      Object.entries(pathItem)
        .filter(([method]) =>
          ['get', 'put', 'post', 'delete', 'patch'].includes(method),
        )
        .map(([, operation]) => [operation.operationId, group]),
    )
    expect(Object.fromEntries(operationGroups)).toEqual(expectedOperationGroups)
  })
})

interface ResolvedPathItem {
  group: string
  pathItem: Record<string, OpenApiOperation>
}

async function resolvePathItems(
  document: OpenApiDocument,
): Promise<ResolvedPathItem[]> {
  return Promise.all(
    Object.values(document.paths ?? {}).map(async (pathItem) => {
      const match = pathItem.$ref?.match(
        /^\.\/paths\/([a-z][a-z0-9_]*)\.yaml#\/([A-Za-z][A-Za-z0-9]*)$/,
      )
      if (!match) throw new Error(`Invalid grouped path ref: ${pathItem.$ref}`)

      const group = parse(
        await readFile(
          resolve(contractDirectory, `./paths/${match[1] as string}.yaml`),
          'utf8',
        ),
      ) as Record<string, Record<string, OpenApiOperation>>
      const resolved = group[match[2] as string]
      if (!resolved)
        throw new Error(`Missing grouped path item: ${pathItem.$ref}`)
      return { group: match[1] as string, pathItem: resolved }
    }),
  )
}

describe('TypeScript Claw API package boundary', () => {
  test('exports generated DTOs and wire enums without a transport client', async () => {
    const packageDirectory = resolve(import.meta.dir, '../../packages/claw-api')
    const generatedDirectory = join(packageDirectory, 'src/generated')
    const files = await treeFiles(generatedDirectory)
    const modelFiles = files.filter((file) => file.startsWith('models/'))

    expect(files).toContain('index.ts')
    expect(modelFiles).toEqual([
      'models/common.ts',
      'models/connections.ts',
      'models/dispatches.ts',
      'models/recordings.ts',
      'models/sessions.ts',
      'models/settings.ts',
      'models/system.ts',
    ])
    expect(
      files.every(
        (file) =>
          file === 'index.ts' || /^models\/[A-Za-z0-9]+\.ts$/.test(file),
      ),
    ).toBe(true)

    const generatedSource = (
      await Promise.all(
        files.map((file) => readFile(join(generatedDirectory, file), 'utf8')),
      )
    ).join('\n')
    expect(generatedSource).not.toMatch(
      /\b(?:Configuration|DefaultApi|ResponseError|FromJSON|ToJSON|runtime)\b/,
    )
    expect(await readFile(join(packageDirectory, 'src/index.ts'), 'utf8')).toBe(
      "export * from './generated/index.js'\n",
    )
  })
})

describe('TypeScript Claw API client generation boundary', () => {
  test('exports type-only paths, operations, and components', async () => {
    const generatedPath = resolve(
      import.meta.dir,
      '../../packages/claw-api-client/src/generated/openapi.ts',
    )
    const source = await readFile(generatedPath, 'utf8')

    expect(source).toStartWith(
      '// This file is generated from contracts/claw-api/openapi.yaml. Do not edit.\n',
    )
    expect(source).toMatch(/export interface paths \{/)
    expect(source).toMatch(/export interface operations \{/)
    expect(source).toMatch(/export interface components \{/)
    expect(source).not.toMatch(/\b(?:DefaultApi|Configuration|runtime)\b/)
  })
})

describe('extractModelGroups', () => {
  test('derives model groups from schema ref files', () => {
    const source = `components:
  schemas:
    ApiError:
      $ref: ./schemas/common.yaml#/ApiError
    SessionDetail:
      $ref: ./schemas/sessions.yaml#/SessionDetail
`

    expect(extractModelGroups(source, ['SessionDetail', 'ApiError'])).toEqual({
      ApiError: 'common',
      SessionDetail: 'sessions',
    })
  })

  test('rejects schema refs outside the group-file convention', () => {
    const source = `components:
  schemas:
    ApiError:
      $ref: ./shared.yaml#/ApiError
`

    expect(() => extractModelGroups(source, ['ApiError'])).toThrow(
      'ApiError schema ref must match ./schemas/<group>.yaml#/<model>',
    )
  })

  test('rejects contract and generated model drift in either direction', () => {
    const source = `components:
  schemas:
    ApiError:
      $ref: ./schemas/common.yaml#/ApiError
`

    expect(() => extractModelGroups(source, ['Unexpected'])).toThrow(
      'Generated model Unexpected has no schema group',
    )
    expect(() => extractModelGroups(source, [])).toThrow(
      'OpenAPI schema ApiError has no generated model',
    )
  })
})

describe('mergeRustModels', () => {
  const header = `/*
 * BrowserClaw API
 *
 * Generated by: https://openapi-generator.tech
 */`

  test('deduplicates the header and imports and sorts member bodies', () => {
    expect(
      mergeRustModels({
        Zebra: `${header}

use serde::{Deserialize, Serialize};
use crate::models;

pub struct Zebra;
`,
        Alpha: `${header}

use crate::models;
use serde::{Deserialize, Serialize};

pub struct Alpha;
`,
      }),
    ).toBe(`${header}

use crate::models;
use serde::{Deserialize, Serialize};

pub struct Alpha;

pub struct Zebra;
`)
  })

  test('rejects differing generated headers', () => {
    expect(() =>
      mergeRustModels({
        Alpha: `${header}\n\nuse crate::models;\n\npub struct Alpha;\n`,
        Beta: `/* changed */\n\nuse crate::models;\n\npub struct Beta;\n`,
      }),
    ).toThrow('Rust model Beta has a different generated header')
  })

  test('prefixes model-local helper types that lose their module namespace', () => {
    expect(
      mergeRustModels({
        Alpha: `${header}

use serde::{Deserialize, Serialize};

pub struct Alpha {
    pub status: Status,
}
pub enum Status { Ok }
impl Default for Status {
    fn default() -> Status { Status::Ok }
}
`,
        Beta: `${header}

use serde::{Deserialize, Serialize};

pub struct Beta {
    pub status: Status,
}
pub enum Status { Ok }
`,
      }),
    ).toContain('pub status: AlphaStatus')
    expect(
      mergeRustModels({
        Alpha: `${header}\n\nuse serde::Serialize;\n\npub struct Alpha;\npub enum Status { Ok }\n`,
        Beta: `${header}\n\nuse serde::Serialize;\n\npub struct Beta;\npub enum Status { Ok }\n`,
      }),
    ).toContain('pub enum BetaStatus')
    expect(
      mergeRustModels({
        Alpha: `${header}\n\nuse serde::Serialize;\n\npub struct Alpha;\npub enum Status { Ok }\n`,
      }),
    ).toContain('pub use super::AlphaStatus as Status;')
  })
})

describe('emitRustModuleIndex', () => {
  test('declares sorted group modules with flat re-exports', () => {
    expect(emitRustModuleIndex(['system', 'common', 'sessions'])).toBe(
      `pub mod common;
pub use self::common::*;
pub mod sessions;
pub use self::sessions::*;
pub mod system;
pub use self::system::*;
`,
    )
  })
})

describe('mergeTypeScriptModels', () => {
  const preamble = `/* tslint:disable */
/* eslint-disable */
/** Generated model. */`
  const modelGroups = {
    Alpha: 'sessions',
    Beta: 'sessions',
    Common: 'common',
    Zebra: 'connections',
  }

  test('rewrites and merges imports, then sorts member bodies', () => {
    expect(
      mergeTypeScriptModels(
        {
          Beta: `${preamble}

import { mapValues } from '../runtime.js';
import type { Alpha } from './Alpha.js';
import type { Zebra } from './Zebra.js';
import {
    ZebraFromJSON,
    ZebraToJSON,
} from './Zebra.js';
export interface Beta {}
`,
          Alpha: `${preamble}

import { mapValues } from '../runtime.js';
import type { Common } from './Common.js';
import { CommonFromJSON } from './Common.js';
import type { Zebra } from './Zebra.js';
import { ZebraFromJSON } from './Zebra.js';

export interface Alpha {}
`,
        },
        modelGroups,
      ),
    ).toBe(`${preamble}

import { mapValues } from '../runtime.js';
import type { Common } from './common.js';
import { CommonFromJSON } from './common.js';
import type { Zebra } from './connections.js';
import {
    ZebraFromJSON,
    ZebraToJSON,
} from './connections.js';

export interface Alpha {}

export interface Beta {}
`)
  })

  test('rejects imports outside the generated model shapes', () => {
    expect(() =>
      mergeTypeScriptModels(
        {
          Alpha: `${preamble}

import defaultValue from './Common.js';

export interface Alpha {}
`,
        },
        modelGroups,
      ),
    ).toThrow('TypeScript model Alpha has an unrecognized import')
  })

  test('rejects imports hidden behind generated comments', () => {
    expect(() =>
      mergeTypeScriptModels(
        {
          Alpha: `${preamble}

// A changed generator template may insert a comment before imports.
import AlphaValue from './AlphaValue.js';

export interface Alpha {}
`,
        },
        modelGroups,
      ),
    ).toThrow('TypeScript model Alpha has an unrecognized import')
  })

  test('merges generated enum models that have no imports', () => {
    expect(
      mergeTypeScriptModels(
        {
          Alpha: `${preamble}

export const Alpha = { Value: 'value' } as const;
`,
        },
        modelGroups,
      ),
    ).toBe(`${preamble}

export const Alpha = { Value: 'value' } as const;
`)
  })
})

describe('emitTypeScriptModelIndex', () => {
  test('exports one sorted module per group', () => {
    expect(emitTypeScriptModelIndex(['system', 'common', 'sessions'])).toBe(
      `export * from './models/common.js'
export * from './models/sessions.js'
export * from './models/system.js'
`,
    )
  })
})

async function yamlFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true })
  const nested = await Promise.all(
    entries.map((entry) => {
      const path = join(directory, entry.name)
      if (entry.isDirectory()) return yamlFiles(path)
      return Promise.resolve(entry.name.endsWith('.yaml') ? [path] : [])
    }),
  )
  return nested.flat()
}

async function treeFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true })
  const nested = await Promise.all(
    entries.map(async (entry) => {
      if (!entry.isDirectory()) return [entry.name]
      return (await treeFiles(join(directory, entry.name))).map((file) =>
        join(entry.name, file),
      )
    }),
  )
  return nested.flat().toSorted()
}
