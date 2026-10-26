/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useEffect } from 'react'
import {
  AnimatedToastStack,
  useAnimatedToastStack,
} from '@/components/motion/animated-toast-stack'
import { connectToastHost, TOAST_DURATION_MS } from './toast'

const MAX_TOASTS = 4

/** Renders every toast raised through the toast helpers. Mounted once. */
export function ToastHost() {
  const { toasts, showToast, updateToast, dismissToast } =
    useAnimatedToastStack({
      limit: MAX_TOASTS,
      defaultDuration: TOAST_DURATION_MS,
    })

  // The toast helpers live outside React; this subscribes them to the stack.
  useEffect(
    () => connectToastHost({ show: showToast, update: updateToast }),
    [showToast, updateToast],
  )

  return (
    <AnimatedToastStack
      toasts={toasts}
      onDismiss={dismissToast}
      position="bottom-center"
      fixed
    />
  )
}
