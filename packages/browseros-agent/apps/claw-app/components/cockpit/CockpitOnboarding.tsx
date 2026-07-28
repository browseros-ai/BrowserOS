/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * First-run guidance rendered by the Cockpit screen when the reader
 * has no session activity yet. Leads with a how-to video bento (agents
 * auto-connect at launch, so onboarding teaches how to put BrowserClaw
 * to work rather than how to install it), then the MCP endpoint CTA,
 * the copyable starter prompt, a connected-agents line, and a docs link.
 *
 * Two visual variants keyed off the `state` prop.
 *
 *   first-run  no connections + no activity. Primary CTA sets up the
 *              MCP endpoint.
 *   waiting    at least one connection + no activity. Primary CTA
 *              becomes "View MCP endpoint" and a waiting banner tells
 *              the reader we are listening.
 *
 * State transitions are handled by the parent (Cockpit) via query
 * refetches; the component is a stateless presenter.
 */

import { Check } from 'lucide-react'
import { useState } from 'react'
import {
  CONNECTED_COPY,
  FOOTER_COPY,
  HERO_COPY,
  type OnboardingState,
  PRIMARY_ACTION_COPY,
  STARTER_PROMPT,
  STARTER_PROMPT_LABEL,
  WAITING_COPY,
} from '@/screens/cockpit/cockpit-onboarding.helpers'
import { FirstRunPrimaryActions } from './FirstRunPrimaryActions'
import { FirstRunWaitingBanner } from './FirstRunWaitingBanner'
import { StarterPromptTile } from './StarterPromptTile'
import { VideoBento } from './VideoBento'

interface CockpitOnboardingProps {
  state: Exclude<OnboardingState, 'ready'>
  /** User-facing harnesses BrowserClaw is registered in (from the live list). */
  connectedHarnesses?: readonly string[]
}

export function CockpitOnboarding({
  state,
  connectedHarnesses = [],
}: CockpitOnboardingProps) {
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
      <VideoBento />
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
      <ConnectedAgentsRow harnesses={connectedHarnesses} />
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

function ConnectedAgentsRow({ harnesses }: { harnesses: readonly string[] }) {
  if (harnesses.length === 0) return null
  return (
    <div className="flex items-center gap-2 rounded-2xl border border-border-2 bg-card p-4">
      <Check className="size-4 shrink-0 text-accent" />
      <span className="font-mono text-[12.5px] text-ink-2 uppercase tracking-[0.06em]">
        {harnesses.join(', ')} {CONNECTED_COPY.suffix}
      </span>
    </div>
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
