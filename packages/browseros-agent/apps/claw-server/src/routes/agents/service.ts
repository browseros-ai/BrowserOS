/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * File-backed agent profile service. One profile per file at
 * <browserosDir>/claw-server/agents/<id>.json keyed by a nanoid;
 * the slug is the user-facing identifier and is unique across all
 * profiles. mcpUrl is recomputed from getLocalServerUrl() on every
 * read so a port change between boots doesn't strand the stored
 * value.
 *
 * Route handlers stay thin: they translate HTTP shape and surface
 * 404s; everything else (validation, persistence, slug resolution,
 * derivation) happens here.
 */

import { nanoid } from 'nanoid'
import { env } from '../../env'
import { AsyncMutex } from '../../lib/async-mutex'
import { logger } from '../../lib/logger'
import { toSlug, uniqueSlug } from '../../lib/slug'
import { listFiles, readJson, removeFile, writeJson } from '../../lib/storage'
import { getLocalServerUrl } from '../../local-server-url'
import {
  installForAgent,
  reconcileHarnessLink,
  uninstallForAgent,
} from '../../services/harness-install'
import { COCKPIT_MOUNT_PREFIX } from '../../shared/port'
import {
  type AgentProfileSummary,
  type CreatedAgent,
  type DeletedAgent,
  type NewAgentValues,
  type RegeneratedMcpUrl,
  type StoredAgentProfile,
  storedAgentProfileSchema,
} from './schemas'

const AGENTS_SUBDIR = 'agents'
const TOTAL_PROFILE_LOGINS = 47

/**
 * Serialises every slug-mutating operation (create / update /
 * regenerateMcpUrl) so the read-snapshot → uniqueSlug → write
 * window cannot race against itself. Reads (list / getDetail) stay
 * lock-free; concurrent writes to different ids that don't touch
 * the slug space could in principle drop the mutex too, but the cost
 * of the queue is negligible and keeping all three under the same
 * lock means the slug-uniqueness invariant holds without per-op
 * reasoning.
 */
const slugMutex = new AsyncMutex()

/**
 * `id` is always a server-generated nanoid(8). Validate it before
 * forwarding to the filesystem so a traversal-shaped value (e.g.
 * URL-encoded `..%2Fconfig`) can never reach the storage layer even
 * if a future route forwards user input directly. Nanoid's default
 * alphabet is `A-Za-z0-9_-`; we cap the length to keep the file name
 * predictable.
 */
const ID_PATTERN = /^[A-Za-z0-9_-]+$/
const MAX_ID_LENGTH = 64

export function isValidId(id: string): boolean {
  return id.length > 0 && id.length <= MAX_ID_LENGTH && ID_PATTERN.test(id)
}

function fileFor(id: string): string {
  return `${AGENTS_SUBDIR}/${id}.json`
}

function nowIso(): string {
  return new Date().toISOString()
}

function baseUrl(): string {
  return (
    getLocalServerUrl() ?? `http://127.0.0.1:${env.port}${COCKPIT_MOUNT_PREFIX}`
  )
}

function buildMcpUrl(slug: string): string {
  return `${baseUrl()}/mcp/${slug}`
}

function buildCliCommand(slug: string): string {
  return `mcp add ${slug}`
}

/**
 * All readable stored profiles, in arbitrary order. A single corrupt
 * file is logged + skipped rather than rejecting the whole list, so
 * one bad agent json (manual edit, partial migration, half-written
 * file on a weird FS) can't brick the whole package: the directory
 * still loads and `create` can still write new profiles.
 */
async function loadAll(): Promise<StoredAgentProfile[]> {
  const names = await listFiles(AGENTS_SUBDIR)
  const settled = await Promise.allSettled(
    names.map((name) =>
      readJson(`${AGENTS_SUBDIR}/${name}`, storedAgentProfileSchema),
    ),
  )
  const profiles: StoredAgentProfile[] = []
  for (let i = 0; i < settled.length; i++) {
    const result = settled[i]
    if (result.status === 'fulfilled') {
      profiles.push(result.value)
    } else {
      logger.warn('skipping unreadable agent profile', {
        file: `${AGENTS_SUBDIR}/${names[i]}`,
        error:
          result.reason instanceof Error
            ? result.reason.message
            : String(result.reason),
      })
    }
  }
  return profiles
}

/**
 * Stored profile for a slug, or null when no agent owns that slug.
 * Used by the MCP route to resolve `/mcp/:slug` against the agents
 * directory. Slugs are unique per Phase 1's `uniqueSlug` invariant so
 * the linear scan never collides.
 */
export async function findBySlug(
  slug: string,
): Promise<StoredAgentProfile | null> {
  const all = await loadAll()
  return all.find((profile) => profile.slug === slug) ?? null
}

/** Stored profile for an id, or null when the file is missing. */
async function loadById(id: string): Promise<StoredAgentProfile | null> {
  if (!isValidId(id)) return null
  try {
    return await readJson(fileFor(id), storedAgentProfileSchema)
  } catch (err) {
    if (
      err instanceof Error &&
      (err.name === 'StorageNotFoundError' ||
        err.name === 'StorageInvalidPathError')
    ) {
      return null
    }
    throw err
  }
}

function summariseProfile(profile: StoredAgentProfile): AgentProfileSummary {
  const blockedActionCount = Object.values(profile.approvals).filter(
    (verdict) => verdict === 'Block',
  ).length
  const loginCount =
    profile.loginMode === 'selective'
      ? profile.selectedSites.length
      : TOTAL_PROFILE_LOGINS
  const loginScopeLabel =
    profile.loginMode === 'selective'
      ? `Selective (${profile.selectedSites.length})`
      : profile.loginMode === 'all'
        ? `All my logins (${TOTAL_PROFILE_LOGINS})`
        : `Current profile (${TOTAL_PROFILE_LOGINS})`
  return {
    id: profile.id,
    name: profile.name,
    harness: profile.harness,
    loginScopeLabel,
    loginCount,
    aclRuleCount: profile.aclRuleIds.length,
    blockedActionCount,
    alwaysAllowCount: 0,
    lastRunAt: 'Never run',
    status: profile.status,
    mcpUrl: buildMcpUrl(profile.slug),
  }
}

function stripWizardShape(profile: StoredAgentProfile): NewAgentValues {
  return {
    name: profile.name,
    harness: profile.harness,
    loginMode: profile.loginMode,
    selectedSites: [...profile.selectedSites],
    approvals: { ...profile.approvals },
    aclRuleIds: [...profile.aclRuleIds],
    customAclRules: profile.customAclRules.map((rule) => ({ ...rule })),
  }
}

export async function list(): Promise<AgentProfileSummary[]> {
  const profiles = await loadAll()
  return profiles
    .sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : -1))
    .map((profile) => summariseProfile(profile))
}

export async function getDetail(id: string): Promise<NewAgentValues | null> {
  const profile = await loadById(id)
  return profile ? stripWizardShape(profile) : null
}

export async function create(input: NewAgentValues): Promise<CreatedAgent> {
  return slugMutex.run(async () => {
    const id = nanoid(8)
    const existing = await loadAll()
    const slug = uniqueSlug(
      toSlug(input.name),
      new Set(existing.map((p) => p.slug)),
    )
    const now = nowIso()
    const profile: StoredAgentProfile = {
      ...input,
      id,
      slug,
      mcpUrl: buildMcpUrl(slug),
      status: 'configured',
      createdAt: now,
      updatedAt: now,
    }
    await writeJson(fileFor(id), profile, storedAgentProfileSchema)
    // Best-effort harness install. A failure here does NOT roll back
    // the profile; the user can retry or fix the harness state and
    // we'll attempt again on the next create. The outcome rides
    // back in the response so the wizard can surface it.
    const harnessInstall = await installForAgent({
      slug: profile.slug,
      mcpUrl: profile.mcpUrl,
      harness: profile.harness,
    })
    return {
      id,
      name: profile.name,
      harness: profile.harness,
      slug,
      mcpUrl: profile.mcpUrl,
      cliCommand: buildCliCommand(slug),
      harnessInstall,
    }
  })
}

export async function update(
  id: string,
  input: NewAgentValues,
): Promise<StoredAgentProfile | null> {
  return slugMutex.run(async () => {
    const existing = await loadById(id)
    if (!existing) return null
    const existingProfiles = await loadAll()
    const nameSlug = toSlug(input.name)
    const slug =
      nameSlug === toSlug(existing.name)
        ? existing.slug
        : uniqueSlug(
            nameSlug,
            new Set(
              existingProfiles
                .filter((profile) => profile.id !== id)
                .map((profile) => profile.slug),
            ),
          )
    const next: StoredAgentProfile = {
      ...existing,
      ...input,
      id,
      slug,
      mcpUrl: buildMcpUrl(slug),
      status: existing.status,
      createdAt: existing.createdAt,
      updatedAt: nowIso(),
    }
    await writeJson(fileFor(id), next, storedAgentProfileSchema)
    // Best-effort harness reconcile: if the harness or slug rotated,
    // wire the new entry into the new harness and drop the old one.
    // Failures are logged inside the helpers and do NOT roll back the
    // profile rewrite.
    await reconcileHarnessLink({
      before: { slug: existing.slug, harness: existing.harness },
      after: { slug: next.slug, mcpUrl: next.mcpUrl, harness: next.harness },
    })
    return next
  })
}

export async function remove(id: string): Promise<DeletedAgent | null> {
  if (!isValidId(id)) return null
  // Load the profile before we wipe it so we can issue the uninstall
  // with the right harness + slug. A delete that races a parallel
  // delete may find no profile here; that's the "already gone" path
  // and we 404.
  const profile = await loadById(id)
  if (!profile) return null
  // Remove the file FIRST. Two concurrent deletes both observe the
  // same profile via loadById, but only the winner's removeFile
  // returns true; the loser exits with 404 here and never
  // side-effects the harness. Without this order, both calls would
  // run uninstallForAgent and the loser would still report 404.
  const existed = await removeFile(fileFor(id))
  if (!existed) return null
  const harnessUninstall = await uninstallForAgent({
    slug: profile.slug,
    harness: profile.harness,
  })
  return { id, harnessUninstall }
}

export async function regenerateMcpUrl(
  id: string,
): Promise<RegeneratedMcpUrl | null> {
  return slugMutex.run(async () => {
    const existing = await loadById(id)
    if (!existing) return null
    const profiles = await loadAll()
    const taken = new Set(
      profiles
        .filter((profile) => profile.id !== id)
        .map((profile) => profile.slug),
    )
    // Route the whole base through toSlug so the nanoid suffix can't
    // smuggle `_` or `-` characters into the slug; the rotated slug
    // stays in the canonical lowercase-alphanum-with-single-hyphens
    // shape.
    const base = toSlug(`${existing.name} ${nanoid(6)}`)
    const slug = uniqueSlug(base, taken)
    const next: StoredAgentProfile = {
      ...existing,
      slug,
      mcpUrl: buildMcpUrl(slug),
      updatedAt: nowIso(),
    }
    await writeJson(fileFor(id), next, storedAgentProfileSchema)
    // The whole point of rotating is to issue a fresh URL to the
    // harness; reconcile the link so the harness picks it up
    // automatically. Harness is unchanged so only the slug pair
    // differs.
    await reconcileHarnessLink({
      before: { slug: existing.slug, harness: existing.harness },
      after: { slug: next.slug, mcpUrl: next.mcpUrl, harness: next.harness },
    })
    return { id, mcpUrl: next.mcpUrl }
  })
}
