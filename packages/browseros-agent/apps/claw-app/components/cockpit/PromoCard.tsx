/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReactNode } from 'react'
import {
  Alert,
  AlertAction,
  AlertClose,
  AlertContent,
  AlertDescription,
  AlertTitle,
} from '@/components/motion/alert'

interface PromoCardProps {
  icon: ReactNode
  title: ReactNode
  description: ReactNode
  /** Buttons or links; the dismiss control is added by the card. */
  action: ReactNode
  onDismiss: () => void
  dismissLabel?: string
  'aria-label'?: string
}

/** The one compact promo look shared by every promotion in Home's top strip. */
export function PromoCard({
  icon,
  title,
  description,
  action,
  onDismiss,
  dismissLabel = 'Dismiss',
  'aria-label': ariaLabel,
}: PromoCardProps) {
  return (
    <Alert
      variant="info"
      aria-label={ariaLabel}
      onOpenChange={(open) => {
        if (!open) onDismiss()
      }}
      className="h-full rounded-2xl px-4 py-3.5"
      contentClassName="h-full flex-wrap items-center gap-x-3.5 gap-y-2.5 md:flex-nowrap"
    >
      <span
        aria-hidden
        className="grid size-[42px] shrink-0 place-items-center rounded-full bg-card text-primary [&>svg]:size-[18px]"
      >
        {icon}
      </span>
      <AlertContent className="min-w-0 basis-[calc(100%-56px)] space-y-0.5 md:basis-auto">
        <AlertTitle className="font-bold text-[15.5px] text-cyanotype-ink">
          {title}
        </AlertTitle>
        <AlertDescription className="text-[14px] text-ink-2">
          {description}
        </AlertDescription>
      </AlertContent>
      <AlertAction className="ml-14 shrink-0 flex-nowrap pt-0 md:ml-0">
        {action}
      </AlertAction>
      <AlertClose
        aria-label={dismissLabel}
        title={dismissLabel}
        className="absolute top-2 right-2 m-0 size-9 rounded-full md:static"
      />
    </Alert>
  )
}
