/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * App-wide toasts on beUI's animated toast stack. Call sites use these
 * functions; the single ToastHost in the app layout renders them.
 */

import { ApiResponseError } from '@browseros/claw-api-client'
import type { ToastInput } from '@/components/motion/animated-toast-stack'

export interface ToastHostHandle {
  show: (input: ToastInput) => string
  update: (id: string, patch: Partial<ToastInput>) => void
}

/** How long a settled toast stays up, in ms. */
export const TOAST_DURATION_MS = 4200

let host: ToastHostHandle | null = null
let pending: ToastInput[] = []
let nextId = 0

/** Connects the mounted toast host. Toasts raised before it mounted are shown now. */
export function connectToastHost(next: ToastHostHandle): () => void {
  host = next
  const queued = pending
  pending = []
  for (const input of queued) next.show(input)
  return () => {
    if (host === next) host = null
  }
}

export function toast(input: ToastInput | string): void {
  const value = typeof input === 'string' ? { title: input } : input
  if (host) host.show(value)
  else pending.push(value)
}

export function toastSuccess(title: string, description?: string): void {
  toast({ title, description, status: 'success' })
}

/** Shows a failed request in plain words, with the server's message when it sent one. */
export function toastApiError(title: string, error?: unknown): void {
  toast({ title, description: errorDescription(error), status: 'error' })
}

function errorDescription(error: unknown): string | undefined {
  if (error instanceof ApiResponseError && error.apiMessage) {
    return error.apiMessage
  }
  return error instanceof Error && error.message ? error.message : undefined
}

export interface ToastPromiseMessages<T> {
  loading: string
  success: string | ((result: T) => string)
  error: string
}

/**
 * One toast that reads "loading" while the work runs, then says how it ended.
 * The failure is reported here, so the caller never sees a rejection.
 */
export function toastPromise<T>(
  work: Promise<T>,
  messages: ToastPromiseMessages<T>,
): void {
  nextId += 1
  const id = `toast-promise-${nextId}`
  const loadingShown = host !== null
  host?.show({ id, title: messages.loading, status: 'loading', duration: 0 })

  const settle = (input: ToastInput) => {
    if (loadingShown && host) {
      host.update(id, { ...input, duration: TOAST_DURATION_MS })
    } else {
      toast(input)
    }
  }

  work.then(
    (result) => {
      const title =
        typeof messages.success === 'function'
          ? messages.success(result)
          : messages.success
      settle({ title, status: 'success' })
    },
    (error: unknown) => {
      settle({
        title: messages.error,
        description: errorDescription(error),
        status: 'error',
      })
    },
  )
}
