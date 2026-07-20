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

import { readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { resolveClawServerPath } from '../lib/browserclaw-dir'

export const RECIPES_DIR_NAME = 'recipes'
export const RECIPES_SHARED_DIR = 'shared'
export const RECIPES_AGENTS_DIR = 'agents'
export const RECIPE_FILE_EXTENSION = '.md'
export const MAX_RECIPES_SURFACED = 10

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
