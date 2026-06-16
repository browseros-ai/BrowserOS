/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * File-backed agent profile service. One profile per file at
 * <browserosDir>/mcp-interface/agents/<id>.json keyed by a nanoid;
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
import { toSlug, uniqueSlug } from '../../lib/slug'
import { listFiles, readJson, removeFile, writeJson } from '../../lib/storage'
import { getLocalServerUrl } from '../../local-server-url'
import {
  type AgentProfileSummary,
  type CreatedAgent,
  type NewAgentValues,
  type RegeneratedMcpUrl,
  type StoredAgentProfile,
  storedAgentProfileSchema,
} from './schemas'

const AGENTS_SUBDIR = 'agents'
const TOTAL_PROFILE_LOGINS = 47

function fileFor(id: string): string {
  return `${AGENTS_SUBDIR}/${id}.json`
}

function nowIso(): string {
  return new Date().toISOString()
}

function baseUrl(): string {
  return getLocalServerUrl() ?? `http://127.0.0.1:${env.port}`
}

function buildMcpUrl(slug: string): string {
  return `${baseUrl()}/mcp/${slug}`
}

function buildCliCommand(slug: string): string {
  return `mcp add ${slug}`
}

/** All stored profiles, in arbitrary order. */
async function loadAll(): Promise<StoredAgentProfile[]> {
  const names = await listFiles(AGENTS_SUBDIR)
  const profiles = await Promise.all(
    names.map((name) =>
      readJson(`${AGENTS_SUBDIR}/${name}`, storedAgentProfileSchema),
    ),
  )
  return profiles
}

/** Stored profile for an id, or null when the file is missing. */
async function loadById(id: string): Promise<StoredAgentProfile | null> {
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
  return {
    id,
    name: profile.name,
    harness: profile.harness,
    slug,
    mcpUrl: profile.mcpUrl,
    cliCommand: buildCliCommand(slug),
  }
}

export async function update(
  id: string,
  input: NewAgentValues,
): Promise<StoredAgentProfile | null> {
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
  return next
}

export async function remove(id: string): Promise<{ id: string } | null> {
  const existed = await removeFile(fileFor(id))
  return existed ? { id } : null
}

export async function regenerateMcpUrl(
  id: string,
): Promise<RegeneratedMcpUrl | null> {
  const existing = await loadById(id)
  if (!existing) return null
  const profiles = await loadAll()
  const taken = new Set(
    profiles
      .filter((profile) => profile.id !== id)
      .map((profile) => profile.slug),
  )
  const base = `${toSlug(existing.name)}-${nanoid(6).toLowerCase()}`
  const slug = uniqueSlug(base, taken)
  const next: StoredAgentProfile = {
    ...existing,
    slug,
    mcpUrl: buildMcpUrl(slug),
    updatedAt: nowIso(),
  }
  await writeJson(fileFor(id), next, storedAgentProfileSchema)
  return { id, mcpUrl: next.mcpUrl }
}
