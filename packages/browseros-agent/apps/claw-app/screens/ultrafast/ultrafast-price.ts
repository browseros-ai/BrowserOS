/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Price test for the Ultrafast waitlist. Each installation sees one monthly
 * price, and the analytics events carry it so signups can be compared per
 * price. The price is derived from the server's analytics UUID, the same
 * identity PostHog uses, so every captured event for a user agrees on it
 * across tabs, reloads and cleared extension storage.
 */

import { useTelemetryState } from '@/modules/analytics/telemetry.hooks'

const ULTRAFAST_PRICES_USD = [10, 20] as const
export type UltrafastPrice = (typeof ULTRAFAST_PRICES_USD)[number]

const FALLBACK_PRICE_KEY = 'ultrafastWaitlistPrice:v1'
const JOINED_KEY = 'ultrafastWaitlistJoined:v1'

/** FNV-1a, so the split is even and stable without a crypto dependency. */
export function priceForId(id: string): UltrafastPrice {
  let hash = 0x811c9dc5
  for (let i = 0; i < id.length; i++) {
    hash ^= id.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193)
  }
  return ULTRAFAST_PRICES_USD[(hash >>> 0) % ULTRAFAST_PRICES_USD.length]
}

function isPrice(value: number): value is UltrafastPrice {
  return (ULTRAFAST_PRICES_USD as readonly number[]).includes(value)
}

/**
 * Only used when the local server cannot be reached. Capture is off without
 * the server's UUID, so this price never reaches analytics; it just keeps the
 * page stable for the reader.
 */
export function fallbackPrice(random: () => number = Math.random) {
  try {
    const stored = Number(localStorage.getItem(FALLBACK_PRICE_KEY))
    if (isPrice(stored)) return stored
    const price =
      ULTRAFAST_PRICES_USD[
        Math.floor(random() * ULTRAFAST_PRICES_USD.length)
      ] ?? ULTRAFAST_PRICES_USD[0]
    localStorage.setItem(FALLBACK_PRICE_KEY, String(price))
    return price
  } catch {
    return ULTRAFAST_PRICES_USD[0]
  }
}

/** The reader's price, or null while the server's identity is loading. */
export function useUltrafastPrice(): UltrafastPrice | null {
  const telemetry = useTelemetryState()
  const distinctId = telemetry.data?.distinctId
  if (distinctId) return priceForId(distinctId)
  if (telemetry.isPending) return null
  return fallbackPrice()
}

export function readJoined(): boolean {
  try {
    return localStorage.getItem(JOINED_KEY) === 'true'
  } catch {
    return false
  }
}

export function rememberJoined(): void {
  try {
    localStorage.setItem(JOINED_KEY, 'true')
  } catch {
    // The signup is already counted; the page just forgets it on reload.
  }
}
