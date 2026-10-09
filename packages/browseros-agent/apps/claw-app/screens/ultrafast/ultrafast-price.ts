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
 *
 * The UUID is available even when usage analytics is off. There is no
 * stand-in price while it loads, which could change the offer mid-visit.
 */

import { useState, useSyncExternalStore } from 'react'
import { useTelemetryState } from '@/modules/analytics/telemetry.hooks'

const ULTRAFAST_PRICES_USD = [9, 19] as const
export type UltrafastPrice = (typeof ULTRAFAST_PRICES_USD)[number]

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

export type UltrafastPriceState =
  | { status: 'loading' }
  | { status: 'unavailable' }
  | { status: 'ready'; price: UltrafastPrice; distinctId: string }

/**
 * The reader's price, pinned for the life of the page once known: the server
 * can swap its analytics UUID mid-visit, and the offer must not change while
 * it is being read. The telemetry query keeps polling, so an unavailable
 * server recovers into a ready price without a reload.
 */
export function useUltrafastPrice(): UltrafastPriceState {
  const telemetry = useTelemetryState()
  const distinctId = telemetry.data?.distinctId
  const [pinned, setPinned] = useState<{
    price: UltrafastPrice
    distinctId: string
  } | null>(null)
  if (pinned !== null) return { status: 'ready', ...pinned }
  if (distinctId) {
    const price = priceForId(distinctId)
    setPinned({ price, distinctId })
    return { status: 'ready', price, distinctId }
  }
  return telemetry.isError ? { status: 'unavailable' } : { status: 'loading' }
}

const joinedListeners = new Set<() => void>()

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
  for (const listener of joinedListeners) listener()
}

/** Same-tab joins notify directly; other tabs arrive as storage events. */
function subscribeToJoined(listener: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === JOINED_KEY) listener()
  }
  joinedListeners.add(listener)
  window.addEventListener('storage', onStorage)
  return () => {
    joinedListeners.delete(listener)
    window.removeEventListener('storage', onStorage)
  }
}

export function useUltrafastJoined(): boolean {
  return useSyncExternalStore(subscribeToJoined, readJoined, () => false)
}
