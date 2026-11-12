/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createQuery } from 'react-query-kit'
import { faviconSources } from './favicon'

/** A site's icon addresses, best first. Icons rarely change, so one lookup per site is enough. */
export const useFaviconSources = createQuery<
  string[],
  { site: string; size: number }
>({
  queryKey: ['favicon', 'sources'],
  fetcher: ({ site, size }) => faviconSources(site, size),
  staleTime: Number.POSITIVE_INFINITY,
  retry: false,
  refetchOnWindowFocus: false,
})
