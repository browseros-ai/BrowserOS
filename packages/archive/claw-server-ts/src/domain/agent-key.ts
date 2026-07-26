/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

declare const agentKeyBrand: unique symbol

export type AgentKey = string & { readonly [agentKeyBrand]: true }

/** Brands a normalized client slug as a durable ownership key. */
export function agentKeyFromSlug(slug: string): AgentKey {
  return slug as AgentKey
}
