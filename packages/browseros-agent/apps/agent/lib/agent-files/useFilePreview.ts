/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single-file preview hook used by the inline artifact card and the
 * Outputs rail's preview Sheet. Always opt-in (`enabled`) — the
 * preview is fetched only when the user clicks a row, never
 * eagerly.
 */

import { useQuery } from '@tanstack/react-query'
import {
  AGENT_QUERY_KEYS,
  agentsFetch,
} from '@/entrypoints/app/agents/useAgents'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import type { FilePreview } from './types'

export function useFilePreview(fileId: string | null, enabled = true) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<FilePreview, Error>({
    queryKey: [AGENT_QUERY_KEYS.filePreview, baseUrl, fileId],
    queryFn: async () => {
      return agentsFetch<FilePreview>(
        baseUrl as string,
        `/files/${encodeURIComponent(fileId as string)}/preview`,
      )
    },
    enabled: Boolean(baseUrl) && !urlLoading && enabled && Boolean(fileId),
    // Previews are immutable for a given fileId — once loaded, never
    // refetch on focus / reconnect. They go stale only when the
    // underlying file is removed (rare in v1; no rename / delete).
    staleTime: Infinity,
    gcTime: 5 * 60 * 1000,
  })

  return {
    preview: query.data ?? null,
    loading: query.isLoading || urlLoading,
    error: query.error ?? urlError,
    refetch: query.refetch,
  }
}
