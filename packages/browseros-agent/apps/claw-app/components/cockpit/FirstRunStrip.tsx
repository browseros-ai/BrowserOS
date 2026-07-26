/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tight one-line three-step reminder strip that sits below the
 * primary-action row on the cockpit first-run block. Replaces the
 * previous three-card grid with a single horizontal band; the
 * detail that used to live in each card body now lives on the
 * screens each step opens (`/mcp`, `/audit`).
 */

import { Check } from 'lucide-react'

interface Step {
  number: string
  title: string
  status: 'active' | 'upcoming' | 'done'
}

interface FirstRunStripProps {
  steps: readonly Step[]
}

export function FirstRunStrip({ steps }: FirstRunStripProps) {
  return (
    <ol className="grid grid-cols-1 gap-3 rounded-2xl border border-border-2 bg-card p-4 sm:grid-cols-3">
      {steps.map((step) => (
        <StepPill key={step.number} step={step} />
      ))}
    </ol>
  )
}

function StepPill({ step }: { step: Step }) {
  const numberTone =
    step.status === 'active'
      ? 'text-accent'
      : step.status === 'done'
        ? 'text-green'
        : 'text-ink-3'
  const titleTone = step.status === 'upcoming' ? 'text-ink-2' : 'text-ink'
  return (
    <li className="flex items-center gap-3">
      <StatusPip status={step.status} />
      <div className="min-w-0 flex-1">
        <div
          className={`font-mono text-[10.5px] uppercase tracking-[0.14em] ${numberTone}`}
        >
          {step.number}
        </div>
        <div className={`truncate font-semibold text-[13.5px] ${titleTone}`}>
          {step.title}
        </div>
      </div>
    </li>
  )
}

function StatusPip({ status }: { status: Step['status'] }) {
  if (status === 'done') {
    return (
      <span
        aria-hidden
        className="flex size-6 shrink-0 items-center justify-center rounded-full bg-green/12 text-green"
      >
        <Check className="size-3.5" />
      </span>
    )
  }
  if (status === 'active') {
    return (
      <span
        aria-hidden
        className="relative flex size-6 shrink-0 items-center justify-center"
      >
        <span className="absolute inline-flex size-6 animate-ping rounded-full bg-accent/30" />
        <span className="relative inline-flex size-3 rounded-full bg-accent" />
      </span>
    )
  }
  return (
    <span
      aria-hidden
      className="flex size-6 shrink-0 items-center justify-center rounded-full border border-border-2 border-dashed"
    >
      <span className="size-1.5 rounded-full bg-border-2" />
    </span>
  )
}
