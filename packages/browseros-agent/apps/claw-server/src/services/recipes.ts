/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Domain recipes cache structural context per host so the agent does
 * not have to rediscover the page on every session. Recipes are
 * Markdown files under `<browserclawDir>/recipes/`, laid out as:
 *
 *   recipes/shared/<host-bucket>/*.md          default, cross-agent
 *   recipes/agents/<slug>/<host-bucket>/*.md   per-agent overlay
 *
 * On read, the discovery layer merges both paths with the agent
 * overlay winning on filename collision. Agents write to `shared/` by
 * default; the overlay is opt-in for observations that only apply to
 * that agent (differing filesystem shape, tool signature, etc.).
 */

import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import matter from 'gray-matter'
import { resolveClawServerPath } from '../lib/browserclaw-dir'

export const RECIPES_DIR_NAME = 'recipes'
export const RECIPES_SHARED_DIR = 'shared'
export const RECIPES_AGENTS_DIR = 'agents'
export const RECIPE_FILE_EXTENSION = '.md'
export const MAX_RECIPES_SURFACED = 10
export const STALE_THRESHOLD_DAYS = 60

/**
 * Reduces a URL to the host bucket used for recipe routing. Keeps the
 * full hostname minus a leading `www.` so subdomain-scoped apps
 * (docs.google.com vs mail.google.com, regional AWS consoles) do not
 * collide. Returns null for non-http(s) schemes or malformed URLs.
 */
export function hostBucketFromUrl(url: string): string | null {
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return null
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null
  const host = parsed.hostname
  if (!host) return null
  const bucket = host.replace(/^www\./, '')
  return bucket.length > 0 ? bucket : null
}

/** Directory where cross-agent recipes for a host live. */
export function sharedRecipesDirFor(hostBucket: string): string {
  return resolveClawServerPath(RECIPES_DIR_NAME, RECIPES_SHARED_DIR, hostBucket)
}

/** Directory where a specific agent's overlay recipes for a host live. */
export function agentRecipesDirFor(slug: string, hostBucket: string): string {
  return resolveClawServerPath(
    RECIPES_DIR_NAME,
    RECIPES_AGENTS_DIR,
    slug,
    hostBucket,
  )
}

export type RecipeSource = 'shared' | 'agent'

export interface RecipeFileEntry {
  /** Basename of the recipe file, e.g. `invitation-manager.md`. */
  name: string
  /** Absolute path on disk. */
  absolutePath: string
  /** Which layer the file came from. */
  source: RecipeSource
}

/**
 * Merges shared and agent-overlay recipe files for a host. Agent
 * files win on filename collision so an agent-specific overlay masks
 * the shared file with the same name. Returns entries sorted by name
 * and capped at MAX_RECIPES_SURFACED.
 */
export function listRecipeFiles(
  slug: string,
  hostBucket: string,
): RecipeFileEntry[] {
  const byName = new Map<string, RecipeFileEntry>()
  for (const entry of listMarkdownEntries(sharedRecipesDirFor(hostBucket))) {
    byName.set(entry.name, { ...entry, source: 'shared' })
  }
  for (const entry of listMarkdownEntries(
    agentRecipesDirFor(slug, hostBucket),
  )) {
    byName.set(entry.name, { ...entry, source: 'agent' })
  }
  return Array.from(byName.values())
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    .slice(0, MAX_RECIPES_SURFACED)
}

interface RawEntry {
  name: string
  absolutePath: string
}

function listMarkdownEntries(dir: string): RawEntry[] {
  let entries: string[]
  try {
    entries = readdirSync(dir)
  } catch {
    return []
  }
  const out: RawEntry[] = []
  for (const name of entries) {
    if (!name.endsWith(RECIPE_FILE_EXTENSION)) continue
    const absolutePath = join(dir, name)
    try {
      if (!statSync(absolutePath).isFile()) continue
    } catch {
      continue
    }
    out.push({ name, absolutePath })
  }
  return out
}

export interface RecipeFrontmatter {
  /** ISO date (YYYY-MM-DD) or null if the recipe has no verification stamp. */
  lastVerified: string | null
  /** Agent slug that last confirmed the recipe against a live page. */
  verifiedBy: string | null
  /** Selectors the agent should cross-check before trusting a stale recipe. */
  usesSelectors: string[]
}

export interface RecipeMetadata {
  frontmatter: RecipeFrontmatter
  /** Days since lastVerified, or null when the timestamp is absent/invalid. */
  ageDays: number | null
  /** True when ageDays >= STALE_THRESHOLD_DAYS. Soft signal only. */
  isStale: boolean
}

const EMPTY_METADATA: RecipeMetadata = {
  frontmatter: { lastVerified: null, verifiedBy: null, usesSelectors: [] },
  ageDays: null,
  isStale: false,
}

/**
 * Reads a recipe's YAML frontmatter and derives its age. Any read or
 * parse failure yields empty metadata so a malformed recipe never
 * breaks discovery; the file's prose is still surfaced without an age
 * stamp.
 */
export function readRecipeMetadata(
  absolutePath: string,
  now: Date = new Date(),
): RecipeMetadata {
  let raw: string
  try {
    raw = readFileSync(absolutePath, 'utf8')
  } catch {
    return EMPTY_METADATA
  }
  let data: Record<string, unknown>
  try {
    const parsed = matter(raw)
    data = parsed.data as Record<string, unknown>
  } catch {
    return EMPTY_METADATA
  }
  const frontmatter: RecipeFrontmatter = {
    lastVerified: normalizeDateField(data.last_verified),
    verifiedBy: typeof data.verified_by === 'string' ? data.verified_by : null,
    usesSelectors: Array.isArray(data.uses_selectors)
      ? data.uses_selectors.filter((s): s is string => typeof s === 'string')
      : [],
  }
  const ageDays = computeAgeDays(frontmatter.lastVerified, now)
  return {
    frontmatter,
    ageDays,
    isStale: ageDays !== null && ageDays >= STALE_THRESHOLD_DAYS,
  }
}

function normalizeDateField(value: unknown): string | null {
  if (typeof value === 'string') return value
  // YAML unquoted dates like `last_verified: 2026-07-20` parse to Date.
  if (value instanceof Date && !Number.isNaN(value.getTime())) {
    return value.toISOString().split('T')[0] ?? null
  }
  return null
}

function computeAgeDays(lastVerified: string | null, now: Date): number | null {
  if (!lastVerified) return null
  const then = new Date(lastVerified)
  const t = then.getTime()
  if (Number.isNaN(t)) return null
  const diffMs = now.getTime() - t
  if (diffMs < 0) return 0
  return Math.floor(diffMs / (1000 * 60 * 60 * 24))
}

const HOSTNAME_SHAPE =
  /^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$/i

/**
 * Resolves a caller-supplied host to a bucket. Accepts either a full
 * http(s) URL or a bare hostname; returns null when the input is
 * neither. Used by the on-demand list_recipes tool where the agent
 * may pass whichever form is convenient.
 */
export function hostBucketFromInput(input: string): string | null {
  const fromUrl = hostBucketFromUrl(input)
  if (fromUrl) return fromUrl
  const trimmed = input.trim().toLowerCase()
  if (!trimmed) return null
  if (!HOSTNAME_SHAPE.test(trimmed)) return null
  const bucket = trimmed.replace(/^www\./, '')
  return bucket.length > 0 ? bucket : null
}

export interface RecipeListing {
  name: string
  source: RecipeSource
  age_days: number | null
  stale: boolean
}

export interface RecipeDiscovery {
  files: RecipeListing[]
  /** Default write path for new cross-agent recipes. */
  workspace_dir: string
  shared_dir: string
  agent_dir: string
  /** One-line summary suitable for the model's context window. */
  summary: string
}

/**
 * Builds the on-demand discovery payload for a caller + host. Same
 * data the navigate effect surfaces via structuredContent.domain_skills,
 * exposed as a helper so the list_recipes MCP tool and the effect
 * share one summary format and one wire shape.
 */
export function describeRecipesForHost(
  slug: string,
  hostBucket: string,
  now: Date = new Date(),
): RecipeDiscovery {
  const entries = listRecipeFiles(slug, hostBucket)
  const enriched = entries.map((entry) => ({
    entry,
    metadata: readRecipeMetadata(entry.absolutePath, now),
  }))
  const files: RecipeListing[] = enriched.map(({ entry, metadata }) => ({
    name: entry.name,
    source: entry.source,
    age_days: metadata.ageDays,
    stale: metadata.isStale,
  }))
  const shared_dir = sharedRecipesDirFor(hostBucket)
  const agent_dir = agentRecipesDirFor(slug, hostBucket)
  const summary = buildRecipeSummary(hostBucket, files, shared_dir)
  return {
    files,
    workspace_dir: shared_dir,
    shared_dir,
    agent_dir,
    summary,
  }
}

function buildRecipeSummary(
  host: string,
  files: RecipeListing[],
  workspaceDir: string,
): string {
  if (files.length === 0) {
    return `Recipes for ${host}: none yet. Save prose notes as kebab-case Markdown files in ${workspaceDir} with a last_verified frontmatter stamp so future sessions know how fresh the guidance is.`
  }
  const names = files.map((f) => f.name).join(', ')
  const stale = files
    .filter((f) => f.stale)
    .map((f) => `${f.name} (${f.age_days}d)`)
  const parts = [`Recipes for ${host}: ${names}.`]
  if (stale.length > 0) {
    parts.push(
      `Stale (>=${STALE_THRESHOLD_DAYS}d, verify selectors before trusting): ${stale.join(', ')}.`,
    )
  }
  parts.push(`Shared workspace: ${workspaceDir}.`)
  return parts.join(' ')
}
