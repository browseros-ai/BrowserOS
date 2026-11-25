/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionSummary } from '@browseros/claw-api'
import { Check } from 'lucide-react'
import { useState } from 'react'
import { NavLink } from 'react-router'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { CopyButton } from '@/components/copy/CopyButton'
import { agentIdentity } from '@/components/harness/harness.types'
import { Button } from '@/components/motion/button/base'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import type { SkillCtaSurface } from '@/screens/skill-draft/skill-draft.helpers'
import { SAVE_PHRASE } from '@/screens/skills/skills.helpers'

interface FreeMakeSkillDialogProps {
  task: SessionSummary
  surface: SkillCtaSurface
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * "Make it a skill" on a free plan: the agent that ran the task saves it, so neo writes nothing
 * and calls no paid service. Ultrafast, which does it in one click, is one quiet link.
 */
export function FreeMakeSkillDialog({
  task,
  surface,
  open,
  onOpenChange,
}: FreeMakeSkillDialogProps) {
  const [copied, setCopied] = useState(false)
  const identity = agentIdentity(task.harness, task.label || task.slug)
  // A label the app cannot name (a random session id) reads worse than "your agent".
  const agent = identity.harness ? identity.name : 'your agent'
  return (
    <CenterMorphModal
      open={open}
      onOpenChange={(next) => {
        if (!next) setCopied(false)
        onOpenChange(next)
      }}
    >
      <CenterMorphModalContent
        ariaLabel="Make this a skill"
        closeButtonLabel="Close"
        className="max-w-[460px] border-border-2 bg-card"
      >
        <div className="flex flex-col gap-3 p-6">
          <h2 className="pr-8 font-extrabold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
            Make this a skill
          </h2>
          <p className="text-[15px] text-ink-2">
            Ask {agent} to save it. Paste this into the chat where it did this
            task:
          </p>
          <div className="flex items-center gap-3 rounded-xl border border-accent-tint-2 bg-accent-tint py-2 pr-2 pl-4">
            <code className="min-w-0 flex-1 font-mono text-[13.5px] text-accent-ink">
              {SAVE_PHRASE}
            </code>
            <CopyButton
              text={SAVE_PHRASE}
              what="the phrase"
              variant="secondary"
              size="sm"
              onCopied={() => {
                setCopied(true)
                track(AnalyticsEvent.FreeMakeSkillPhraseCopied, { surface })
              }}
              className="h-9 shrink-0 font-semibold text-[13.5px] text-accent-ink"
            />
          </div>
          {copied ? (
            <p
              aria-live="polite"
              className="flex items-center gap-2 font-semibold text-[14px] text-cyanotype-ink"
            >
              <Check
                aria-hidden
                className="size-[15px] text-primary"
                strokeWidth={2.4}
              />
              Copied. Now paste it into {agent}.
            </p>
          ) : null}
          <div className="mt-2 flex flex-wrap items-center justify-between gap-3 border-border-2 border-t pt-4">
            <p className="text-[13.5px] text-ink-3">
              With <UltrafastWordmark />, neo does it in one click.{' '}
              <NavLink
                to="/pro"
                onClick={() => onOpenChange(false)}
                className="font-semibold text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
              >
                Get ultrafast
              </NavLink>
            </p>
            <CenterMorphModalClose>
              <Button className="ml-auto h-[38px] px-4 font-semibold">
                Done
              </Button>
            </CenterMorphModalClose>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
