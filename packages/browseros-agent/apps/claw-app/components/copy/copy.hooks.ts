/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useRef, useState } from 'react'
import { toastApiError } from '@/modules/toast/toast'

/** Long enough to read "Paste it into your agent" and switch windows. */
const COPIED_FOR_MS = 6000

/** How long the copied state takes to fade out, so its content stays until it has gone. */
const LEAVE_MS = 400

/**
 * Copies text for a surface that shows its own copied state, such as a skill card. `copied` stays
 * true for a few seconds after each copy; `shown` stays true a little longer, until the copied
 * state has faded out, so content mounted for it can leave with an animation. A copy that fails
 * says so in a toast and returns false.
 */
export function useCopyText(copiedForMs = COPIED_FOR_MS) {
  const [copied, setCopied] = useState(false)
  const [shown, setShown] = useState(false)
  const timer = useRef<number | undefined>(undefined)

  const copy = async (text: string): Promise<boolean> => {
    try {
      await navigator.clipboard.writeText(text)
    } catch (error) {
      toastApiError('Could not copy that', error)
      return false
    }
    window.clearTimeout(timer.current)
    setCopied(true)
    setShown(true)
    timer.current = window.setTimeout(() => {
      setCopied(false)
      timer.current = window.setTimeout(() => setShown(false), LEAVE_MS)
    }, copiedForMs)
    return true
  }

  return { copied, shown, copy }
}
