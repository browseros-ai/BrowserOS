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

/** The thin bar above Home's greeting that every promotion shares. */
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
      className="rounded-xl py-1.5 pr-2 pl-4"
      contentClassName="min-h-9 flex-wrap items-center gap-x-3.5 gap-y-1 md:flex-nowrap"
    >
      <span aria-hidden className="shrink-0 text-primary [&>svg]:size-4">
        {icon}
      </span>
      <AlertContent className="flex min-w-0 flex-wrap items-center gap-x-3.5 gap-y-0.5 space-y-0 md:flex-nowrap">
        <AlertTitle className="whitespace-nowrap font-bold text-[14px] text-cyanotype-ink md:border-border-2 md:border-r md:pr-3.5">
          {title}
        </AlertTitle>
        <AlertDescription className="basis-full text-[14px] text-ink-2 leading-5 md:basis-auto">
          {description}
        </AlertDescription>
      </AlertContent>
      <AlertAction className="order-last basis-full flex-nowrap pt-0 pl-[22px] md:order-none md:basis-auto md:pl-0">
        {action}
      </AlertAction>
      <AlertClose
        aria-label={dismissLabel}
        title={dismissLabel}
        className="m-0 size-8 rounded-full"
      />
    </Alert>
  )
}
