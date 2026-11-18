/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { PartyPopper } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/motion/button/base'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { cn } from '@/lib/utils'
import { ULTRAFAST_STEPS } from '@/screens/pro/pro.helpers'
import { UltrafastPicture } from './UltrafastPictures'

interface UltrafastTourProps {
  open: boolean
  /** The step to open on. Give the tour a new `key` per opening to start fresh. */
  startAt?: number
  /** Just paid: step one says welcome. */
  welcome?: boolean
  onClose: () => void
}

/** What ultrafast adds, as three pictures. Skip is always there. */
export function UltrafastTour({
  open,
  startAt = 0,
  welcome = false,
  onClose,
}: UltrafastTourProps) {
  const [step, setStep] = useState(startAt)
  const last = ULTRAFAST_STEPS.length - 1
  const current = ULTRAFAST_STEPS[step] ?? ULTRAFAST_STEPS[0]
  const greeting = welcome && step === 0
  const Icon = greeting ? PartyPopper : current.icon
  const go = (next: number) => setStep(Math.min(last, Math.max(0, next)))

  return (
    <CenterMorphModal
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
    >
      <CenterMorphModalContent
        ariaLabel={greeting ? 'Welcome to ultrafast' : current.name}
        showCloseButton={false}
        className="max-w-[540px] border-border-2 bg-card"
      >
        {/* biome-ignore lint/a11y/noStaticElementInteractions: arrow keys are a shortcut for the Back and Next buttons */}
        <div
          className="flex flex-col"
          onKeyDown={(event) => {
            if (event.key === 'ArrowRight') go(step + 1)
            if (event.key === 'ArrowLeft') go(step - 1)
          }}
        >
          <div className="flex items-center justify-between gap-3 pt-3.5 pr-4 pb-2.5 pl-[22px]">
            <span className="inline-flex items-center gap-2 font-semibold text-[13.5px] text-accent-ink">
              <Icon aria-hidden className="size-4" />
              {greeting ? 'Welcome to ultrafast' : current.name}
            </span>
            <CenterMorphModalClose>
              <Button variant="ghost" size="sm" className="font-semibold">
                Skip
              </Button>
            </CenterMorphModalClose>
          </div>
          <div className="mx-3 grid min-h-0 place-items-center rounded-2xl border border-border-2 bg-accent-tint px-3.5 pt-[18px] pb-3.5 sm:mx-4 sm:min-h-[236px] sm:px-[22px] sm:pt-[26px] sm:pb-[18px]">
            <UltrafastPicture key={step} step={step} />
          </div>
          <div className="flex flex-col gap-1.5 px-4 pt-4 pb-1 sm:px-[22px] sm:pt-5">
            <h2 className="font-bold text-[22px] text-cyanotype-ink leading-[1.15] tracking-[-0.02em] sm:text-[26px]">
              {current.title}
            </h2>
            <p className="text-[15px] text-ink-2 sm:text-base">
              {current.line}
            </p>
          </div>
          <div className="flex items-center justify-between gap-3 px-4 pt-3.5 pb-4 sm:px-[22px] sm:pt-4 sm:pb-5">
            <span className="inline-flex gap-1.5">
              {ULTRAFAST_STEPS.map((item, index) => (
                <button
                  key={item.name}
                  type="button"
                  aria-label={`Step ${index + 1} of ${ULTRAFAST_STEPS.length}`}
                  aria-current={index === step ? 'step' : undefined}
                  onClick={() => go(index)}
                  className={cn(
                    'h-2 rounded-full outline-none transition-[width,background-color] duration-200 focus-visible:ring-2 focus-visible:ring-ring',
                    index === step ? 'w-[22px] bg-primary' : 'w-2 bg-border',
                  )}
                />
              ))}
            </span>
            <span className="inline-flex items-center gap-2">
              {step > 0 ? (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => go(step - 1)}
                  className="font-semibold"
                >
                  Back
                </Button>
              ) : null}
              {step < last ? (
                <Button
                  size="sm"
                  onClick={() => go(step + 1)}
                  className="font-semibold"
                >
                  Next
                </Button>
              ) : (
                <CenterMorphModalClose>
                  <Button size="sm" className="font-semibold">
                    Start using ultrafast
                  </Button>
                </CenterMorphModalClose>
              )}
            </span>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
