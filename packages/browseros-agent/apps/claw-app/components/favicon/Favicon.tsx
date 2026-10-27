/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Globe } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { cn } from '@/lib/utils'
import { useFaviconSources } from '@/modules/favicon/favicon.hooks'

interface FaviconProps {
  /** "x.com" or a full address; only its host is used. */
  site: string
  /** The size asked of each source, in CSS pixels. */
  size?: number
  className?: string
  /** Shown while looking up and when no source has an icon. Defaults to a globe. */
  fallback?: ReactNode
}

/** A site's icon from the browser's cache, then DuckDuckGo, then Google, then a globe. */
export function Favicon({
  site,
  size = 32,
  className,
  fallback = <Globe aria-hidden className={cn('size-[18px]', className)} />,
}: FaviconProps) {
  const sources = useFaviconSources({ variables: { site, size } })
  if (sources.isPending || sources.isError || sources.data.length === 0) {
    return fallback
  }
  return (
    <FirstLoadingImage
      key={sources.data.join(' ')}
      sources={sources.data}
      className={className}
      fallback={fallback}
    />
  )
}

function FirstLoadingImage({
  sources,
  className,
  fallback,
}: {
  sources: string[]
  className?: string
  fallback: ReactNode
}) {
  const [index, setIndex] = useState(0)
  const src = sources[index]
  if (!src) return fallback
  return (
    <img
      src={src}
      alt=""
      aria-hidden
      className={cn('size-[18px] rounded-[4px]', className)}
      onError={() => setIndex((current) => current + 1)}
    />
  )
}
