/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The cockpit's first run, until the person's first agent activity. neo
 * carries on the chat the onboarding page started: which AI apps it connected
 * (Cockpit passes the real installed, user-facing harnesses), the one extra
 * step Claude Cowork needs, then a demo of a chore and two tasks to try.
 */

import { Check } from 'lucide-react'
import { type CSSProperties, type ReactNode, useState } from 'react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { Harness } from '@/components/harness/harness.types'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  FIRST_RUN_COPY,
  GET_CLAUDE_URL,
  TRY_PROMPTS,
} from '@/screens/cockpit/cockpit-onboarding.helpers'
import { InstallExtensionDialog } from '@/screens/mcp/InstallExtensionDialog'
import { OnboardingDemo } from './OnboardingDemo'
import { PromptPill } from './PromptPill'

interface CockpitOnboardingProps {
  connectedHarnesses?: readonly Harness[]
}

export function CockpitOnboarding({
  connectedHarnesses = [],
}: CockpitOnboardingProps) {
  const [coworkGuideOpen, setCoworkGuideOpen] = useState(false)
  const found = connectedHarnesses.length > 0

  return (
    <section className="flex flex-col gap-3" aria-label="Getting started">
      <h1 className="font-extrabold text-[36px] text-cyanotype-ink leading-[1.13] tracking-[-0.035em]">
        {FIRST_RUN_COPY.headingLead}{' '}
        <em className="font-medium font-serif text-[1.087em] tracking-[-0.015em]">
          {FIRST_RUN_COPY.headingAccent}
        </em>{' '}
        {FIRST_RUN_COPY.headingTail}
      </h1>

      <div className="flex items-start gap-3">
        <img
          src="/icons/browserclaw.svg"
          alt=""
          aria-hidden
          className="mt-0.5 size-[38px] shrink-0 rounded-[10px]"
        />
        <div className="flex min-w-0 flex-col gap-1.5">
          <Line order={0}>
            <Bubble lead>
              {found ? FIRST_RUN_COPY.connected : FIRST_RUN_COPY.noneFound}
            </Bubble>
            {found ? (
              <>
                {connectedHarnesses.map((harness) => (
                  <span
                    key={harness}
                    className="inline-flex h-8 items-center gap-2 rounded-full border border-border-2 bg-card pr-3 pl-2.5 font-semibold text-[14px] text-cyanotype-ink"
                  >
                    <HarnessIcon harness={harness} className="size-4" />
                    {harness}
                    <Check
                      aria-label="connected"
                      className="size-3.5 text-accent"
                      strokeWidth={3}
                    />
                  </span>
                ))}
                <span className="text-[13px] text-ink-3">
                  {FIRST_RUN_COPY.restartHint}
                </span>
              </>
            ) : (
              <a
                href={GET_CLAUDE_URL}
                target="_blank"
                rel="noopener noreferrer"
                onClick={() =>
                  track(AnalyticsEvent.OnboardingLinkClicked, {
                    link: 'get-claude',
                  })
                }
                className="px-1 font-semibold text-[15px] text-ink-2 transition-colors hover:text-cyanotype-ink"
              >
                {FIRST_RUN_COPY.getClaude}
              </a>
            )}
          </Line>
          <Line order={1}>
            <Bubble>{FIRST_RUN_COPY.cowork}</Bubble>
            <button
              type="button"
              onClick={() => {
                track(AnalyticsEvent.OnboardingLinkClicked, { link: 'cowork' })
                setCoworkGuideOpen(true)
              }}
              className={cn(
                'h-9 cursor-pointer rounded-full px-4 font-semibold text-[14px] transition-colors focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-accent/25',
                // With no app found, Cowork is the main way in.
                found
                  ? 'border border-border bg-card text-cyanotype-ink hover:bg-bg-canvas'
                  : 'bg-accent text-accent-foreground hover:bg-accent-2',
              )}
            >
              {FIRST_RUN_COPY.coworkAction}
            </button>
          </Line>
          <Line order={2}>
            <Bubble>{FIRST_RUN_COPY.watch}</Bubble>
          </Line>
        </div>
      </div>

      <OnboardingDemo />

      <div className="flex flex-col gap-2">
        <div className="flex items-start gap-3.5">
          <span className="shrink-0 whitespace-nowrap pt-[11px] font-semibold text-[15px] text-ink leading-5">
            {FIRST_RUN_COPY.tryLabel}
          </span>
          <div className="flex min-w-0 flex-1 flex-wrap gap-2.5">
            {TRY_PROMPTS.map((prompt) => (
              <PromptPill
                key={prompt}
                prompt={prompt}
                onCopied={() => track(AnalyticsEvent.OnboardingTaskCopied)}
              />
            ))}
          </div>
        </div>
        <p className="text-[14px] text-ink-3 leading-5">
          {FIRST_RUN_COPY.tryHint}
        </p>
      </div>

      <InstallExtensionDialog
        open={coworkGuideOpen}
        onOpenChange={setCoworkGuideOpen}
      />
    </section>
  )
}

/** One of neo's lines, landing a beat after the one above it. */
function Line({ order, children }: { order: number; children: ReactNode }) {
  const style: CSSProperties = { animationDelay: `${150 + order * 350}ms` }
  return (
    <div
      style={style}
      className="flex animate-fade-up flex-wrap items-center gap-2 motion-reduce:animate-none"
    >
      {children}
    </div>
  )
}

function Bubble({ lead, children }: { lead?: boolean; children: ReactNode }) {
  return (
    <span
      className={cn(
        'rounded-[20px] bg-accent-tint px-4 py-2 font-medium text-[17px] text-cyanotype-ink leading-[22px]',
        lead && 'rounded-tl-[6px]',
      )}
    >
      {children}
    </span>
  )
}
