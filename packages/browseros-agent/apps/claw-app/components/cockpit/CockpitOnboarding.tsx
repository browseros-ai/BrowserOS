/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * First-run guidance rendered by the Cockpit screen when the reader
 * has no session activity yet. Walks the three actual steps to a
 * first agent run: install the MCP, ask an agent to try BrowserClaw,
 * watch the run land here.
 *
 * Two visual variants keyed off the `state` prop.
 *
 *   first-run  no connections + no activity
 *              Step 01 active, Steps 02/03 muted upcoming.
 *
 *   waiting    at least one MCP connection + no activity
 *              Step 01 dimmed done, Step 02 active, Step 03 muted.
 *
 * State transitions are handled by the parent (Cockpit) via query
 * refetches; the component is a stateless presenter.
 */

import {
  Activity,
  ArrowRight,
  Check,
  MessageSquare,
  PlugZap,
  RotateCcw,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { NavLink } from 'react-router'
import {
  FOOTER_COPY,
  HERO_COPY,
  type OnboardingState,
  STEP_COPY,
  STEP_KICKERS,
} from '@/screens/cockpit/cockpit-onboarding.helpers'

interface CockpitOnboardingProps {
  state: Exclude<OnboardingState, 'ready'>
  onRefresh: () => void
}

export function CockpitOnboarding({
  state,
  onRefresh,
}: CockpitOnboardingProps) {
  const kickers = STEP_KICKERS[state]
  const isWaiting = state === 'waiting'
  return (
    <section className="flex flex-col gap-8" aria-label={HERO_COPY.eyebrow}>
      <OnboardingHero />
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <StepCard
          number="01"
          kicker={kickers[0]}
          status={isWaiting ? 'done' : 'active'}
          icon={<PlugZap className="size-5" />}
          title={
            isWaiting ? STEP_COPY.install.doneTitle : STEP_COPY.install.title
          }
          body={isWaiting ? STEP_COPY.install.doneBody : STEP_COPY.install.body}
          action={
            <StepLink
              to={STEP_COPY.install.href}
              label={
                isWaiting ? STEP_COPY.install.doneCta : STEP_COPY.install.cta
              }
              variant={isWaiting ? 'muted' : 'accent'}
            />
          }
        />
        <StepCard
          number="02"
          kicker={kickers[1]}
          status={isWaiting ? 'active' : 'upcoming'}
          icon={<MessageSquare className="size-5" />}
          title={STEP_COPY.ask.title}
          body={STEP_COPY.ask.body}
          action={<Terminal lines={STEP_COPY.ask.terminal} />}
        />
        <StepCard
          number="03"
          kicker={kickers[2]}
          status="upcoming"
          icon={<Activity className="size-5" />}
          title={STEP_COPY.watch.title}
          body={STEP_COPY.watch.body}
        />
      </div>
      <OnboardingFooter onRefresh={onRefresh} />
    </section>
  )
}

/* ── Hero ─────────────────────────────────────────────────────────── */

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

/* ── Step card ────────────────────────────────────────────────────── */

type StepStatus = 'active' | 'upcoming' | 'done'

interface StepCardProps {
  number: string
  kicker: string
  status: StepStatus
  icon: ReactNode
  title: string
  body: string
  action?: ReactNode
}

function StepCard({
  number,
  kicker,
  status,
  icon,
  title,
  body,
  action,
}: StepCardProps) {
  const shell =
    status === 'active'
      ? 'border-accent bg-card shadow-card'
      : status === 'done'
        ? 'border-border-2 bg-bg-sunken'
        : 'border-border-2 border-dashed bg-bg-sunken'
  const iconTint =
    status === 'active'
      ? 'bg-accent-tint text-accent-ink'
      : 'bg-card-tint text-ink-3'
  const titleTone = status === 'active' ? 'text-ink' : 'text-ink-2'
  const bodyTone = status === 'done' ? 'text-ink-3' : 'text-ink-2'
  return (
    <article
      className={[
        'flex flex-col gap-4 rounded-2xl border p-6 transition-colors motion-reduce:transition-none',
        shell,
      ].join(' ')}
    >
      <StepBadge number={number} kicker={kicker} status={status} />
      <span
        aria-hidden
        className={[
          'flex size-10 items-center justify-center rounded-lg',
          iconTint,
        ].join(' ')}
      >
        {icon}
      </span>
      <div className="flex flex-col gap-2">
        <h3
          className={['font-bold text-base leading-tight', titleTone].join(' ')}
        >
          {title}
        </h3>
        <p className={['text-sm leading-relaxed', bodyTone].join(' ')}>
          {body}
        </p>
      </div>
      {action && <div className="mt-auto pt-1">{action}</div>}
    </article>
  )
}

interface StepBadgeProps {
  number: string
  kicker: string
  status: StepStatus
}

function StepBadge({ number, kicker, status }: StepBadgeProps) {
  const kickerTone =
    status === 'active'
      ? 'text-accent'
      : status === 'done'
        ? 'text-green'
        : 'text-ink-3'
  const numberTone = status === 'active' ? 'text-ink' : 'text-ink-3'
  return (
    <div className="flex items-center justify-between">
      <span
        className={[
          'font-bold font-mono text-sm tracking-tight',
          numberTone,
        ].join(' ')}
      >
        {number}
      </span>
      <span
        className={[
          'inline-flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-[0.12em]',
          kickerTone,
        ].join(' ')}
      >
        <StatusDot status={status} />
        {kicker}
      </span>
    </div>
  )
}

function StatusDot({ status }: { status: StepStatus }) {
  if (status === 'done') {
    return (
      <span
        aria-hidden
        className="flex size-3.5 items-center justify-center rounded-full bg-green-tint text-green"
      >
        <Check className="size-2.5" strokeWidth={3} />
      </span>
    )
  }
  if (status === 'active') {
    return (
      <span
        aria-hidden
        className="inline-block size-2 rounded-full bg-accent shadow-[0_0_8px_hsl(221_90%_55%/0.5)]"
      />
    )
  }
  return (
    <span
      aria-hidden
      className="inline-block size-2 rounded-full border border-ink-3/60"
    />
  )
}

/* ── Step action primitives ───────────────────────────────────────── */

function StepLink({
  to,
  label,
  variant,
}: {
  to: string
  label: string
  variant: 'accent' | 'muted'
}) {
  const tone =
    variant === 'accent'
      ? 'text-accent hover:text-accent-2'
      : 'text-ink-3 hover:text-ink'
  return (
    <NavLink
      to={to}
      className={[
        'group inline-flex items-center gap-1.5 font-mono text-[12px] uppercase tracking-[0.08em] transition-colors motion-reduce:transition-none',
        tone,
      ].join(' ')}
    >
      {label}
      <ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none" />
    </NavLink>
  )
}

function Terminal({ lines }: { lines: readonly string[] }) {
  return (
    <figure
      className="overflow-hidden rounded-lg bg-ink-deep p-3 font-mono text-[11.5px] text-white/90 leading-[1.6]"
      aria-label="Example prompt in Claude Code"
    >
      {lines.map((line) => (
        <div key={line}>{line || ' '}</div>
      ))}
    </figure>
  )
}

/* ── Footer ───────────────────────────────────────────────────────── */

function OnboardingFooter({ onRefresh }: { onRefresh: () => void }) {
  return (
    <footer className="flex flex-wrap items-center gap-x-3 gap-y-2 text-ink-3 text-sm">
      <span>{FOOTER_COPY.refreshQuestion}</span>
      <button
        type="button"
        onClick={onRefresh}
        className="inline-flex items-center gap-1 text-ink-2 underline decoration-border-2 underline-offset-4 transition-colors hover:decoration-ink motion-reduce:transition-none"
      >
        <RotateCcw className="size-3" />
        {FOOTER_COPY.refresh}
      </button>
      <span aria-hidden className="text-ink-3">
        ·
      </span>
      <a
        href={FOOTER_COPY.docsHref}
        target="_blank"
        rel="noopener noreferrer"
        className="group inline-flex items-center gap-1 text-ink-2 underline decoration-border-2 underline-offset-4 transition-colors hover:decoration-ink motion-reduce:transition-none"
      >
        {FOOTER_COPY.docs}
        <ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none" />
      </a>
    </footer>
  )
}
