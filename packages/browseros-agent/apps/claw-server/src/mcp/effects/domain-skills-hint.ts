/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Surfaces domain-recipe files to the agent on navigate. The effect
 * merges the shared and per-agent overlay directories for the target
 * host, stamps each file with its age (via last_verified frontmatter),
 * and annotates the navigate response so the agent can Read the files
 * with its own tools before deciding how to proceed on the page.
 *
 * Recipes are context caching: they do not replace the browser tools,
 * they make each tool call cheaper to reason about. The synthetic text
 * block appended below carries a small nudge that reflects this.
 */

import {
  agentRecipesDirFor,
  hostBucketFromUrl,
  listRecipeFiles,
  type RecipeFileEntry,
  type RecipeMetadata,
  readRecipeMetadata,
  sharedRecipesDirFor,
} from '../../services/recipes'
import type { ToolEffect } from '../dispatch'

interface AnnotatedEntry {
  entry: RecipeFileEntry
  metadata: RecipeMetadata
}

interface DomainSkillsPayload {
  files: Array<{
    name: string
    source: RecipeFileEntry['source']
    age_days: number | null
    stale: boolean
  }>
  /** Default write path for new cross-agent recipes. */
  workspace_dir: string
  shared_dir: string
  agent_dir: string
}

export const applyDomainSkillsHint: ToolEffect = ({ call, result }) => {
  if (call.tool.name !== 'navigate') return
  if (result.isError) return
  if (!call.identity) return

  const url = extractUrl(result, call)
  if (!url) return
  const host = hostBucketFromUrl(url)
  if (!host) return

  const slug = call.identity.slug
  const now = new Date()
  const entries = listRecipeFiles(slug, host)
  const annotated: AnnotatedEntry[] = entries.map((entry) => ({
    entry,
    metadata: readRecipeMetadata(entry.absolutePath, now),
  }))

  const payload: DomainSkillsPayload = {
    files: annotated.map(({ entry, metadata }) => ({
      name: entry.name,
      source: entry.source,
      age_days: metadata.ageDays,
      stale: metadata.isStale,
    })),
    workspace_dir: sharedRecipesDirFor(host),
    shared_dir: sharedRecipesDirFor(host),
    agent_dir: agentRecipesDirFor(slug, host),
  }

  const priorStructured =
    typeof result.structuredContent === 'object' &&
    result.structuredContent !== null
      ? (result.structuredContent as Record<string, unknown>)
      : {}
  const structuredContent = {
    ...priorStructured,
    domain_skills: payload,
  }

  return {
    ...result,
    content: [
      ...result.content,
      { type: 'text', text: buildSummary(host, annotated, payload) },
    ],
    structuredContent,
  }
}

function extractUrl(
  result: Parameters<ToolEffect>[0]['result'],
  call: Parameters<ToolEffect>[0]['call'],
): string | null {
  const fromResult = (result.structuredContent as { url?: unknown } | undefined)
    ?.url
  if (typeof fromResult === 'string' && fromResult.length > 0) return fromResult
  const fromArgs = (call.args as { url?: unknown } | null | undefined)?.url
  if (typeof fromArgs === 'string' && fromArgs.length > 0) return fromArgs
  return null
}

function buildSummary(
  host: string,
  annotated: AnnotatedEntry[],
  payload: DomainSkillsPayload,
): string {
  if (annotated.length === 0) {
    return `Recipes for ${host}: none yet. Save prose notes as kebab-case Markdown files in ${payload.workspace_dir} with a last_verified frontmatter stamp so future sessions know how fresh the guidance is.`
  }
  const names = annotated.map(({ entry }) => entry.name).join(', ')
  const stale = annotated
    .filter(({ metadata }) => metadata.isStale)
    .map(({ entry, metadata }) => `${entry.name} (${metadata.ageDays}d)`)
  const parts = [`Recipes for ${host}: ${names}.`]
  if (stale.length > 0) {
    parts.push(
      `Stale (>=60d, verify selectors before trusting): ${stale.join(', ')}.`,
    )
  }
  parts.push(`Shared workspace: ${payload.workspace_dir}.`)
  return parts.join(' ')
}
