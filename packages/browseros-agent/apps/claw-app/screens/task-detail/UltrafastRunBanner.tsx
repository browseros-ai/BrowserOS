/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionUltrafastRun } from '@browseros/claw-api'
import { Zap } from 'lucide-react'
import {
  Alert,
  AlertContent,
  AlertDescription,
} from '@/components/motion/alert'
import {
  bannerTokens,
  pagesLabel,
  ultrafastTime,
} from '@/screens/audit/audit.helpers'

/** What neo did on its own while a skill ran without the agent. */
export function UltrafastRunBanner({ run }: { run: SessionUltrafastRun }) {
  return (
    <Alert
      variant="info"
      className="rounded-2xl px-5 py-4"
      contentClassName="items-start gap-3.5"
    >
      <Zap aria-hidden className="mt-0.5 size-5 shrink-0" />
      <AlertContent className="min-w-0 space-y-1">
        <AlertDescription className="text-[15px] text-ink">
          neo read {pagesLabel(run.pagesRead)} in{' '}
          {ultrafastTime(run.durationMs)}
        </AlertDescription>
        <AlertDescription className="text-[13.5px] text-ink-3">
          {bannerTokens(run.tokensSaved)} tokens saved with ultrafast
        </AlertDescription>
      </AlertContent>
    </Alert>
  )
}
