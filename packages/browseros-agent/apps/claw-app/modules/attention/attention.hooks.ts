/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useEffect } from 'react'
import { createQuery } from 'react-query-kit'
import { attentionTitle } from './attention.helpers'
import { notificationPermission } from './attention-alerts'

const BASE_TITLE = 'BrowserOS neo'

/** Puts "(1) Needs you" in the neo tab's title while agents are waiting. */
export function useAttentionTitle(count: number): void {
  // The tab title is document state outside React, so it is set from here.
  useEffect(() => {
    document.title = attentionTitle(count, BASE_TITLE)
    return () => {
      document.title = BASE_TITLE
    }
  }, [count])
}

/** Whether macOS (or the OS) lets the browser show notifications. */
export const useNotificationPermission = createQuery<'granted' | 'denied'>({
  queryKey: ['attention', 'permission'],
  fetcher: notificationPermission,
  refetchOnWindowFocus: true,
})
