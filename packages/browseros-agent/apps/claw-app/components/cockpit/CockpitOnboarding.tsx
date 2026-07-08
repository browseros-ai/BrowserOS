/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * First-run guidance rendered by the Cockpit screen when the reader
 * has no session activity yet. Anchors on a short Remotion motion
 * demo that establishes the mental model (this cockpit watches;
 * your agent acts), then a single definitive CTA to set up the MCP
 * endpoint, then the copyable starter prompt tile, then a tight
 * three-step reminder strip, then a docs link.
 *
 * Two visual variants keyed off the `state` prop.
 *
 *   first-run  no connections + no activity. Primary CTA is the MCP
 *              install; step 01 pulses active.
 *   waiting    at least one MCP connection + no activity. Primary
 *              CTA becomes "View MCP endpoint"; step 01 renders
 *              done; step 02 pulses active; a waiting banner tells
 *              the reader we are listening.
 *
 * State transitions are handled by the parent (Cockpit) via query
 * refetches; the component is a stateless presenter.
 */

import { useState } from 'react'
import {
  FOOTER_COPY,
  HERO_COPY,
  type OnboardingState,
  PRIMARY_ACTION_COPY,
  STARTER_PROMPT,
  STARTER_PROMPT_LABEL,
  STEP_COPY,
  WAITING_COPY,
} from '@/screens/cockpit/cockpit-onboarding.helpers'
import { FirstRunPrimaryActions } from './FirstRunPrimaryActions'
import { FirstRunStrip } from './FirstRunStrip'
import { FirstRunVideo } from './FirstRunVideo'
import { FirstRunWaitingBanner } from './FirstRunWaitingBanner'
import { StarterPromptTile } from './StarterPromptTile'

interface CockpitOnboardingProps {
  state: Exclude<OnboardingState, 'ready'>
}

export function CockpitOnboarding({ state }: CockpitOnboardingProps) {
  const [promptCopied, setPromptCopied] = useState(false)
  const isWaiting = state === 'waiting'
  const showWaitingBanner = isWaiting || promptCopied
  const waitingMessage = promptCopied
    ? WAITING_COPY.promptCopied
    : WAITING_COPY.connectedNoActivity
  const flagCopied = () => {
    setPromptCopied(true)
    window.setTimeout(() => setPromptCopied(false), 8000)
  }
  return (
    <section
      className="flex flex-col gap-8"
      aria-label={HERO_COPY.eyebrow.toLowerCase()}
    >
      <OnboardingHero />
      <FirstRunVideo />
      <FirstRunPrimaryActions
        installHref={PRIMARY_ACTION_COPY.install.href}
        installLabel={
          isWaiting
            ? PRIMARY_ACTION_COPY.install.doneLabel
            : PRIMARY_ACTION_COPY.install.activeLabel
        }
        installStatus={isWaiting ? 'done' : 'active'}
      />
      {showWaitingBanner && <FirstRunWaitingBanner message={waitingMessage} />}
      <div className="flex flex-col gap-2">
        <div className="font-bold text-[12.5px] text-ink-2">
          {STARTER_PROMPT_LABEL}
        </div>
        <StarterPromptTile prompt={STARTER_PROMPT} onCopied={flagCopied} />
      </div>
      <FirstRunStrip
        steps={[
          {
            number: '01',
            title: isWaiting
              ? STEP_COPY.install.doneTitle
              : STEP_COPY.install.activeTitle,
            status: isWaiting ? 'done' : 'active',
          },
          {
            number: '02',
            title: STEP_COPY.ask.title,
            status: isWaiting ? 'active' : 'upcoming',
          },
          {
            number: '03',
            title: STEP_COPY.watch.title,
            status: 'upcoming',
          },
        ]}
      />
      <OnboardingFooter />
    </section>
  )
}

function OnboardingHero() {
  return (
    <header className="flex flex-col gap-3 pt-1">
      <span className="font-mono text-[11px] text-ink-3 uppercase tracking-[0.14em]">
        {HERO_COPY.eyebrow}
      </span>
      <h1 className="font-extrabold text-3xl leading-[1.15] tracking-tight md:text-4xl">
        {HERO_COPY.h1Prefix}{' '}
        <span className="font-medium font-serif text-accent italic">
          {HERO_COPY.h1Accent}
        </span>
      </h1>
      <p className="text-ink-3 text-sm">{HERO_COPY.subhead}</p>
    </header>
  )
}

function OnboardingFooter() {
  return (
    <div className="pt-1 text-[12.5px] text-ink-3">
      <a
        href={FOOTER_COPY.docsHref}
        target="_blank"
        rel="noopener noreferrer"
        className="text-ink-2 underline decoration-border-2 underline-offset-2 transition hover:decoration-ink-2"
      >
        {FOOTER_COPY.docs}
      </a>
    </div>
  )
}
