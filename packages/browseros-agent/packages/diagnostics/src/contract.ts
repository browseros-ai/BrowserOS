/**
 * Diagnostics v1 wire contract shared by the app, Reporter and feedback service.
 * Independent releases keep a copy of this small allowlist; unknown fields are
 * discarded at each boundary so preferences or account data cannot hitch a ride.
 */
export interface DiagnosticsSnapshot {
  schemaVersion: 1
  product: 'browseros' | 'browseros-neo' | 'browseros-feedback'
  collectedAt: string
  versions: {
    browseros: string | null
    chromium: string | null
    server: string | null
    appExtension: string | null
    reporterExtension: string | null
  }
  system: {
    os: string | null
    osVersion: string | null
    architecture: string | null
    processor: string | null
    logicalCores: number | null
  }
  memory: { totalBytes: number | null; availableBytes: number | null }
}

export const DIAGNOSTICS_REQUEST = 'browseros:diagnostics:get'
export const REPORTER_VERSION_REQUEST = 'browseros:diagnostics:reporter-version'
export const APP_EXTENSION_IDS = [
  'bflpfmnmnokmjhmgnolecpppdbdophmk',
  'pjimfkbpehlcllblajnpfamdfjhhlgkc',
] as const
export const REPORTER_EXTENSION_ID = 'adlpneommgkgeanpaekgoaolcpncohkf'
export const DIAGNOSTICS_MAX_AGE_MS = 60_000
export const DIAGNOSTICS_TIMEOUT_MS = 3_000

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Expected object')
  return value as Record<string, unknown>
}

function text(value: unknown): string | null {
  if (value === null) return null
  if (typeof value !== 'string' || value.length > 200)
    throw new Error('Invalid text')
  // One-line fields keep clipboard and notification formatting predictable.
  return (
    Array.from(value, (char) => {
      const code = char.charCodeAt(0)
      return code < 32 || code === 127 ? ' ' : char
    })
      .join('')
      .trim() || null
  )
}

function count(value: unknown): number | null {
  if (value === null) return null
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0)
    throw new Error('Invalid count')
  return value
}

/** Returns a new allowlisted object, never the untrusted input object. */
export function parseDiagnostics(value: unknown): DiagnosticsSnapshot | null {
  try {
    const data = record(value)
    if (
      data.schemaVersion !== 1 ||
      !['browseros', 'browseros-neo', 'browseros-feedback'].includes(
        String(data.product),
      )
    )
      return null
    if (
      typeof data.collectedAt !== 'string' ||
      data.collectedAt.length > 35 ||
      !/^\d{4}-\d{2}-\d{2}T/.test(data.collectedAt) ||
      !Number.isFinite(Date.parse(data.collectedAt))
    )
      return null
    const versions = record(data.versions)
    const system = record(data.system)
    const memory = record(data.memory)
    const snapshot: DiagnosticsSnapshot = {
      schemaVersion: 1,
      product: data.product as DiagnosticsSnapshot['product'],
      collectedAt: data.collectedAt,
      versions: {
        browseros: text(versions.browseros),
        chromium: text(versions.chromium),
        server: text(versions.server),
        appExtension: text(versions.appExtension),
        reporterExtension: text(versions.reporterExtension),
      },
      system: {
        os: text(system.os),
        osVersion: text(system.osVersion),
        architecture: text(system.architecture),
        processor: text(system.processor),
        logicalCores: count(system.logicalCores),
      },
      memory: {
        totalBytes: count(memory.totalBytes),
        availableBytes: count(memory.availableBytes),
      },
    }
    if (
      snapshot.system.logicalCores !== null &&
      snapshot.system.logicalCores > 65536
    )
      return null
    if (
      snapshot.memory.totalBytes !== null &&
      snapshot.memory.availableBytes !== null &&
      snapshot.memory.availableBytes > snapshot.memory.totalBytes
    )
      return null
    return snapshot
  } catch {
    return null
  }
}

export function productName(product: DiagnosticsSnapshot['product']): string {
  return product === 'browseros-neo'
    ? 'BrowserOS neo'
    : product === 'browseros-feedback'
      ? 'BrowserOS (Reporter fallback)'
      : 'BrowserOS'
}

export function formatMemory(bytes: number | null): string {
  return bytes === null
    ? 'Unavailable'
    : `${(bytes / 1024 ** 3).toFixed(1)} GiB`
}

export function diagnosticSections(snapshot: DiagnosticsSnapshot) {
  const { versions, system, memory } = snapshot
  return [
    {
      title: 'Versions',
      rows: [
        ['BrowserOS', versions.browseros],
        ['Chromium', versions.chromium],
        [
          snapshot.product === 'browseros-neo' ? 'MCP server' : 'Agent server',
          versions.server,
        ],
        ['App extension', versions.appExtension],
        ['Bug Reporter extension', versions.reporterExtension],
      ],
    },
    {
      title: 'System',
      rows: [
        [
          'Operating system',
          [system.os, system.osVersion].filter(Boolean).join(' ') || null,
        ],
        ['Architecture', system.architecture],
        ['Processor', system.processor],
        ['Logical cores', system.logicalCores?.toString() ?? null],
      ],
    },
    {
      title: 'Memory',
      rows: [
        ['Total system memory', formatMemory(memory.totalBytes)],
        ['Available system memory', formatMemory(memory.availableBytes)],
      ],
    },
  ]
}

/** Plain text is also the canonical content for the copy button and support messages. */
export function formatDiagnostics(snapshot: DiagnosticsSnapshot): string {
  return [
    `${productName(snapshot.product)} diagnostics (v1)`,
    `Collected at: ${snapshot.collectedAt}`,
    ...diagnosticSections(snapshot).flatMap((section) => [
      '',
      section.title,
      ...section.rows.map(
        ([label, value]) => `${label}: ${value ?? 'Unavailable'}`,
      ),
    ]),
    '',
    'Memory is a system-wide sample at collection time.',
  ].join('\n')
}
