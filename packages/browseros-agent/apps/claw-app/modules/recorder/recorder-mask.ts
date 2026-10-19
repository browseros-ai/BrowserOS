/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * True when an input holds a secret the session replay must not keep: a password
 * field, or one whose autocomplete marks it a password, one-time code, or payment
 * card. Mirrors the server-side classifier that redacts the audit log, so a value
 * masked in the ledger is masked in the replay too.
 */
export function isSensitiveInput(element: Element | null | undefined): boolean {
  if (!element) return false
  try {
    const type = (
      element.getAttribute('type') ||
      (element as HTMLInputElement).type ||
      ''
    ).toLowerCase()
    if (element.tagName === 'INPUT' && type === 'password') return true
    const autocomplete = (
      element.getAttribute('autocomplete') || ''
    ).toLowerCase()
    return autocomplete
      .split(/\s+/)
      .some(
        (token) =>
          token === 'current-password' ||
          token === 'new-password' ||
          token === 'one-time-code' ||
          token.startsWith('cc-'),
      )
  } catch {
    return false
  }
}

/**
 * rrweb `maskInputFn`. With `maskAllInputs` on, rrweb calls this for every input;
 * returning the original text leaves ordinary fields readable in the replay and
 * only sensitive fields are masked (rrweb cannot select inputs by attribute, only
 * by type, so the gate lives here).
 */
export function maskRecorderInput(text: string, element: HTMLElement): string {
  return isSensitiveInput(element) ? '*'.repeat(text.length) : text
}
