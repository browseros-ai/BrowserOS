// fallow-ignore-next-line unused-files
/**
 * Catalog validator. Pure functions. Runs at test time (see
 * `test/unit/catalog-validate.test.ts`) and blocks a build whenever a
 * populated entry is missing a required field, has an inconsistent
 * transport declaration, or carries a stale `verified` date.
 *
 * Consumers of the library never see this module. It exists purely to
 * enforce the schema-plus-citation invariants on `client-configs.ts`.
 */

import type { ClientConfig, ClientId } from './types'

export interface ValidationError {
  clientId: ClientId | '<duplicate>'
  path: string
  message: string
}

/** Maximum age of a `sources.verified` date before it counts as stale. */
export const MAX_VERIFIED_AGE_DAYS = 365

/**
 * Validate every populated entry against the catalog's invariants.
 * Returns an empty array on success. Never throws.
 */
export function validateCatalog(
  entries: ReadonlyArray<ClientConfig>,
  now: Date,
): ValidationError[] {
  const errors: ValidationError[] = []
  const seenIds = new Set<ClientId>()

  for (const entry of entries) {
    validateEntry(entry, now, errors)
    if (seenIds.has(entry.id)) {
      errors.push({
        clientId: '<duplicate>',
        path: `id`,
        message: `duplicate client id "${entry.id}" appears more than once in CATALOG`,
      })
    }
    seenIds.add(entry.id)
  }
  return errors
}

function validateEntry(
  entry: ClientConfig,
  now: Date,
  errors: ValidationError[],
): void {
  validateSources(entry, now, errors)
  validatePaths(entry, errors)
  validateTransportShape(entry, errors)
  validateProjectShape(entry, errors)
}

function validateSources(
  entry: ClientConfig,
  now: Date,
  errors: ValidationError[],
): void {
  const push = mkPush(entry, errors)
  if (!entry.sources.firstParty?.trim()) {
    push(
      'sources.firstParty',
      'first-party docs URL is required (no populated entry may ship without one)',
    )
  } else if (!isLikelyUrl(entry.sources.firstParty)) {
    push('sources.firstParty', 'value must be an http(s) URL')
  }
  if (entry.sources.smithery && !isLikelyUrl(entry.sources.smithery)) {
    push('sources.smithery', 'value must be an http(s) URL')
  }
  if (!isIsoDate(entry.sources.verified)) {
    push(
      'sources.verified',
      `must be a YYYY-MM-DD ISO date, got ${JSON.stringify(entry.sources.verified)}`,
    )
  } else if (isStaleVerifiedDate(entry.sources.verified, now)) {
    push(
      'sources.verified',
      `entry has not been re-verified in over ${MAX_VERIFIED_AGE_DAYS} days (last verified ${entry.sources.verified})`,
    )
  }
}

function validatePaths(entry: ClientConfig, errors: ValidationError[]): void {
  const anyPath = (['darwin', 'linux', 'win32'] as const).some(
    (os) => (entry.systemPaths[os] ?? []).length > 0,
  )
  if (!anyPath) {
    mkPush(entry, errors)(
      'systemPaths',
      'at least one of darwin / linux / win32 must have a non-empty path list',
    )
  }
}

function validateTransportShape(
  entry: ClientConfig,
  errors: ValidationError[],
): void {
  const push = mkPush(entry, errors)
  const systemHasRemote = includesRemote(entry.supportedTransports.system)
  if (systemHasRemote && !entry.http) {
    push(
      'http',
      'system supportedTransports includes http or sse but no http shape is declared',
    )
  } else if (!systemHasRemote && entry.http) {
    push(
      'http',
      'http shape declared but supportedTransports.system does not include http or sse',
    )
  }
}

function validateProjectShape(
  entry: ClientConfig,
  errors: ValidationError[],
): void {
  const projectTransports = entry.supportedTransports.project
  if (!projectTransports) return
  const push = mkPush(entry, errors)
  if (!entry.projectFile) {
    push(
      'projectFile',
      'must be set when supportedTransports.project is declared',
    )
  }
  if (!entry.project?.stdio) {
    push(
      'project.stdio',
      'must be set when supportedTransports.project is declared',
    )
  }
  const projHasRemote = includesRemote(projectTransports)
  if (projHasRemote && !entry.project?.http) {
    push(
      'project.http',
      'project supportedTransports includes http or sse but no project http shape is declared',
    )
  } else if (!projHasRemote && entry.project?.http) {
    push(
      'project.http',
      'project http shape declared but project supportedTransports does not include http or sse',
    )
  }
}

function mkPush(entry: ClientConfig, errors: ValidationError[]) {
  return (path: string, message: string) =>
    errors.push({ clientId: entry.id, path, message })
}

function includesRemote(transports: ReadonlyArray<string>): boolean {
  return transports.includes('http') || transports.includes('sse')
}

function isLikelyUrl(value: string): boolean {
  return /^https?:\/\//.test(value)
}

function isIsoDate(value: string): boolean {
  return (
    /^\d{4}-\d{2}-\d{2}$/.test(value) &&
    !Number.isNaN(new Date(value).getTime())
  )
}

function isStaleVerifiedDate(iso: string, now: Date): boolean {
  const then = new Date(iso).getTime()
  const ms = now.getTime() - then
  const days = ms / (1000 * 60 * 60 * 24)
  return days > MAX_VERIFIED_AGE_DAYS
}
