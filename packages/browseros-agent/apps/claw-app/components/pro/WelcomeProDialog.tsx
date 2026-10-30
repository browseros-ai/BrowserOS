/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import { KeyRound, Sparkles } from 'lucide-react'
import { Button } from '@/components/motion/button/base'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { formatDate, PRO_FEATURES } from '@/screens/pro/pro.helpers'

export interface ProWelcome {
  plan: Plan
  /** Paid again on a key whose Pro had ended, so no new key was made. */
  sameKey: boolean
}

interface WelcomeProDialogProps {
  open: boolean
  welcome: ProWelcome | null
  onClose: () => void
}

/** Shown once Pro is on after paying; the key itself is only in the email. */
export function WelcomeProDialog({
  open,
  welcome,
  onClose,
}: WelcomeProDialogProps) {
  const periodEnd = welcome?.plan.periodEnd ?? null
  return (
    <CenterMorphModal
      open={open && welcome !== null}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
    >
      <CenterMorphModalContent
        ariaLabel="You’re on neo Pro"
        showCloseButton={false}
        className="max-w-[460px] border-border-2 bg-card"
      >
        <div className="flex flex-col items-center gap-5 p-6 text-center">
          <span
            aria-hidden
            className="grid size-14 place-items-center rounded-full bg-accent-tint text-primary"
          >
            <Sparkles className="size-6" />
          </span>
          <div className="flex flex-col gap-2">
            <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              You’re on neo Pro
            </h2>
            {welcome?.sameKey ? (
              <p className="text-[14.5px] text-ink-2">
                Thank you! Stripe is emailing your receipt.
              </p>
            ) : (
              <>
                <p className="text-[14.5px] text-ink-2">
                  Thank you! We’ve emailed your licence key, and Stripe is
                  emailing your receipt.
                </p>
                <p className="text-[14.5px] text-ink-2">
                  Use the key in that email to turn on Pro on your other
                  computers.
                </p>
              </>
            )}
          </div>
          {welcome?.sameKey ? (
            <p className="flex gap-2.5 rounded-xl border border-border-2 bg-bg-sunken px-3.5 py-3 text-left text-[14px] text-ink-2">
              <KeyRound aria-hidden className="mt-0.5 size-4 shrink-0" />
              <span>
                Your key stays the same. Pro is back on every computer that uses
                it.
              </span>
            </p>
          ) : null}
          <ul className="flex w-full flex-col gap-2 text-left text-[14.5px] text-ink">
            {PRO_FEATURES.map((feature) => (
              <li key={feature.title} className="flex items-center gap-2.5">
                <feature.icon
                  aria-hidden
                  className="size-4 shrink-0 text-primary"
                />
                {feature.title}
              </li>
            ))}
          </ul>
          {periodEnd !== null ? (
            <p className="text-[13.5px] text-ink-3">
              Renews on {formatDate(periodEnd)}. Billing, receipts and
              cancelling live on the neo Pro page.
            </p>
          ) : null}
          <CenterMorphModalClose>
            <Button className="font-semibold">Start using Pro</Button>
          </CenterMorphModalClose>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
