/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ElicitRequestFormParams } from '@modelcontextprotocol/sdk/types.js'

const SMALL_NAME_WORD_LIMIT = 3
const SMALL_NAME_MAX_LEN = 32
const SESSION_NAME_INPUT_MAX_LEN = 64
const NAME_GUIDANCE =
  'a small lowercase 2-3 word name for what this session is doing'

/** Normalizes a user-provided session label into a short tab-group slug. */
export function normalizeSmallName(raw: string): string {
  const cleaned = raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  if (cleaned.length === 0) return ''
  return cleaned
    .split('-')
    .filter((part) => part.length > 0)
    .slice(0, SMALL_NAME_WORD_LIMIT)
    .join('-')
    .slice(0, SMALL_NAME_MAX_LEN)
    .replace(/^-+|-+$/g, '')
}

/** Returns the short client namespace used before the session label. */
export function clientPrefixFromSlug(slug: string): string {
  return slug.split('-').find((part) => part.length > 0) ?? 'agent'
}

/** Builds the BrowserOS tab-group title for a named MCP session. */
export function buildSessionGroupTitle(
  prefix: string,
  smallName: string,
): string {
  return `${prefix}/${smallName}`
}

/** Builds the elicitation message with the concrete client prefix. */
export function buildSessionNamePrompt(prefix: string): string {
  return `Name this browser session: ${NAME_GUIDANCE} (e.g. "invoice processing"). Tabs will be grouped as ${prefix}/<name>.`
}

export const sessionNameRequestedSchema = {
  type: 'object',
  properties: {
    name: {
      type: 'string',
      title: 'Session name',
      description: `Use ${NAME_GUIDANCE}, such as "invoice processing".`,
      maxLength: SESSION_NAME_INPUT_MAX_LEN,
    },
  },
  required: ['name'],
} satisfies ElicitRequestFormParams['requestedSchema']
