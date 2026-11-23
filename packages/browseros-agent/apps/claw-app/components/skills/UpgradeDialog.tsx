/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check, Sparkles } from 'lucide-react'
import { NavLink } from 'react-router'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { Button } from '@/components/motion/button/base'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'

const BENEFITS = [
  'Your skills run in seconds',
  'Turn any task into a skill in one click',
  'Robot checks solved for you',
]

interface UpgradeDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** The one place a free plan hears what ultrafast adds: speed, then skills from tasks. */
export function UpgradeDialog({ open, onOpenChange }: UpgradeDialogProps) {
  return (
    <CenterMorphModal open={open} onOpenChange={onOpenChange}>
      <CenterMorphModalContent
        ariaLabel="Run your skills in seconds"
        showCloseButton={false}
        className="max-w-[460px] overflow-hidden border-border-2 bg-card"
      >
        <div className="flex items-center gap-3.5 border-border-2 border-b bg-accent-tint px-6 pt-6 pb-5">
          <img
            src="/icons/browserclaw.svg"
            alt=""
            aria-hidden
            className="size-[46px] rounded-xl"
          />
          <UltrafastWordmark className="text-[24px]" />
        </div>
        <div className="flex flex-col gap-4 p-6">
          <div className="flex flex-col gap-2">
            <h2 className="font-extrabold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              Run your skills in seconds
            </h2>
            <p className="text-[15px] text-ink-2">
              On the free plan, your skills run at normal speed. With ultrafast,
              neo learns each skill and runs it in seconds.
            </p>
          </div>
          <ul className="flex flex-col gap-2.5">
            {BENEFITS.map((benefit) => (
              <li
                key={benefit}
                className="flex items-center gap-2.5 text-[14.5px] text-cyanotype-ink"
              >
                <span
                  aria-hidden
                  className="grid size-[22px] shrink-0 place-items-center rounded-full bg-accent-tint text-primary"
                >
                  <Check className="size-[13px]" strokeWidth={2.4} />
                </span>
                {benefit}
              </li>
            ))}
          </ul>
          <div className="mt-2 flex flex-wrap justify-end gap-2">
            <CenterMorphModalClose>
              <Button variant="secondary" className="h-[38px] font-semibold">
                Not now
              </Button>
            </CenterMorphModalClose>
            <NavLink
              to="/pro"
              onClick={() => onOpenChange(false)}
              className="inline-flex h-[38px] items-center gap-2 rounded-md bg-primary px-4 font-semibold text-[14px] text-primary-foreground outline-none hover:bg-primary/90 focus-visible:ring-2 focus-visible:ring-ring"
            >
              <Sparkles aria-hidden className="size-4" />
              Get ultrafast
            </NavLink>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
