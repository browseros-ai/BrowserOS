/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * File-backed site-rules service. All rules live in a single array
 * file at <browserosDir>/claw-server/site-rules.json (typical user
 * count is under 20, full-array scans on every read are fine and the
 * single-file shape lets the UI's list view round-trip in one I/O
 * call).
 *
 * Mutations go through a per-service AsyncMutex so two concurrent
 * `add` calls cannot drop one (the read-then-rewrite window is the
 * same shape the agents service guards in Phase 1). Reads are
 * lock-free.
 */

import { nanoid } from 'nanoid'
import { AsyncMutex } from '../../lib/async-mutex'
import { matchDomain } from '../../lib/match-domain'
import {
  fileExists,
  readJson,
  StorageNotFoundError,
  writeJson,
} from '../../lib/storage'
import {
  type AddSiteRuleVariables,
  type SiteRule,
  siteRulesFileSchema,
} from './schemas'

const FILE = 'site-rules.json'
const ID_PATTERN = /^[A-Za-z0-9_-]+$/
const MAX_ID_LENGTH = 64

/**
 * `id` is server-generated nanoid; we validate it here too so a
 * traversal-shaped value handed to the DELETE handler resolves to
 * not-found before the storage layer ever sees it. Same defence-in-
 * depth pattern as the agents service.
 */
function isValidId(id: string): boolean {
  return id.length > 0 && id.length <= MAX_ID_LENGTH && ID_PATTERN.test(id)
}

const mutex = new AsyncMutex()

async function loadAll(): Promise<SiteRule[]> {
  try {
    return await readJson(FILE, siteRulesFileSchema)
  } catch (err) {
    if (err instanceof StorageNotFoundError) return []
    throw err
  }
}

export async function list(): Promise<SiteRule[]> {
  return loadAll()
}

export async function add(input: AddSiteRuleVariables): Promise<SiteRule> {
  return mutex.run(async () => {
    const existing = await loadAll()
    const rule: SiteRule = {
      id: nanoid(8),
      label: input.label,
      domain: input.domain,
      action: input.action,
    }
    const next = [...existing, rule]
    await writeJson(FILE, next, siteRulesFileSchema)
    return rule
  })
}

export async function remove(id: string): Promise<{ id: string } | null> {
  if (!isValidId(id)) return null
  return mutex.run(async () => {
    if (!(await fileExists(FILE))) return null
    const existing = await loadAll()
    const next = existing.filter((rule) => rule.id !== id)
    if (next.length === existing.length) return null
    await writeJson(FILE, next, siteRulesFileSchema)
    return { id }
  })
}

/**
 * In-process helper used by `permissions.check`. Returns every rule
 * whose `(domain, action)` pair matches the request. Caller decides
 * how to combine multiple matches (Phase 5: any match with a
 * configured verb yields a clamp).
 */
export async function findMatching(
  domain: string,
  action: SiteRule['action'],
): Promise<SiteRule[]> {
  const rules = await loadAll()
  return rules.filter(
    (rule) => rule.action === action && matchDomain(rule.domain, domain),
  )
}
