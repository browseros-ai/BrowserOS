/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  Bot,
  Check,
  Compass,
  Eye,
  type LucideIcon,
  ShieldCheck,
  ShoppingCart,
  UserRoundCheck,
  WandSparkles,
  Zap,
} from 'lucide-react'
import type { CSSProperties, ReactNode } from 'react'
import { cn } from '@/lib/utils'

// Every picture rests on its final frame. The animations only add the way in,
// so with reduced motion the finished picture shows straight away. Remount a
// picture (a new `key`) to play it again.

const delay = (seconds: number): CSSProperties => ({
  animationDelay: `${seconds}s`,
})

/** Picture for each ultrafast step, in the order of ULTRAFAST_STEPS. */
export function UltrafastPicture({ step }: { step: number }) {
  if (step === 0) return <StepRacePicture />
  if (step === 1) return <SkillPicture />
  return <RobotCheckPicture />
}

/** Your agent alone takes many paid steps; with ultrafast, neo takes a few. */
export function StepRacePicture() {
  return (
    <Picture caption="Illustration. Each dot is a step your agent pays for.">
      <Lane icon={Bot} label="Your agent alone" dots={10} />
      <Lane icon={Zap} label="With ultrafast" dots={4} fast />
    </Picture>
  )
}

/** A skill neo wrote, then how long it takes with and without your agent. */
export function SkillPicture() {
  return (
    <Picture caption="Illustration. You pick with or without your agent.">
      <div className="grid grid-cols-[36px_minmax(0,1fr)] items-center gap-3 rounded-xl border border-border-2 bg-card px-3.5 py-3 shadow-card motion-safe:animate-fade-up">
        <span className="grid size-9 place-items-center rounded-md bg-accent-tint text-accent-ink">
          <ShoppingCart className="size-4" />
        </span>
        <span className="flex min-w-0 flex-col gap-1">
          <span className="truncate font-semibold text-[14px] text-cyanotype-ink">
            Reorder my usual toothpaste
          </span>
          <span className="inline-flex items-center gap-1 self-start rounded-full bg-accent-tint px-2 py-0.5 font-semibold text-[12px] text-accent-ink">
            <WandSparkles className="size-3" />
            Written by neo
          </span>
        </span>
      </div>
      <div className="flex flex-col gap-2.5">
        <BarRow icon={UserRoundCheck} label="With your agent">
          <span
            className="block h-3 w-full origin-left rounded-full bg-ink-4 motion-safe:animate-uf-grow motion-safe:[animation-duration:3.4s]"
            style={delay(0.6)}
          />
        </BarRow>
        <BarRow icon={Zap} label="Without your agent" fast>
          <span
            className="block h-3 w-1/4 origin-left rounded-full bg-primary motion-safe:animate-uf-grow motion-safe:[animation-duration:0.8s]"
            style={delay(0.6)}
          />
          <Done style={delay(1.45)} />
        </BarRow>
      </div>
    </Picture>
  )
}

const CHECK_STEPS: { icon: LucideIcon; text: string; check?: boolean }[] = [
  { icon: Compass, text: 'Opened the product page' },
  { icon: ShieldCheck, text: 'Are you a robot?', check: true },
  { icon: Eye, text: 'Read today’s price' },
  { icon: Check, text: 'Noted the price' },
]

/** A task's steps, with the robot check turning to Solved. */
export function RobotCheckPicture() {
  return (
    <Picture caption="Illustration.">
      <ol className="flex flex-col gap-0.5 rounded-xl border border-border-2 bg-card p-2 shadow-card">
        {CHECK_STEPS.map((step, index) => (
          <li
            key={step.text}
            className={cn(
              'grid grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-md px-2 py-1.5 text-[14px] motion-safe:animate-fade-up',
              step.check
                ? 'bg-green-tint font-semibold text-cyanotype-ink motion-safe:[animation:fade-up_0.35s_cubic-bezier(0.16,1,0.3,1)_both,uf-flash_1.2s_ease_both]'
                : 'text-ink-2',
            )}
            style={
              step.check
                ? { animationDelay: `${0.2 + index * 0.55}s, 1.25s` }
                : delay(0.2 + index * 0.55)
            }
          >
            <span className="grid size-7 place-items-center rounded-full bg-accent-tint text-accent-ink">
              <step.icon className="size-3.5" />
            </span>
            <span className="truncate">{step.text}</span>
            {step.check ? (
              <span
                className="inline-flex items-center gap-1 rounded-full bg-card px-2 py-0.5 font-semibold text-[12px] text-green-ink motion-safe:animate-uf-pop"
                style={delay(1.35)}
              >
                <Check className="size-3" />
                Solved
              </span>
            ) : null}
          </li>
        ))}
      </ol>
    </Picture>
  )
}

// ---------------------------------------------------------------------------

function Picture({
  caption,
  children,
}: {
  caption: string
  children: ReactNode
}) {
  return (
    <figure className="m-0 flex w-full flex-col gap-3.5">
      <div aria-hidden className="flex flex-col gap-3.5">
        {children}
      </div>
      <figcaption className="text-[12.5px] text-ink-3">{caption}</figcaption>
    </figure>
  )
}

function Lane({
  icon: Icon,
  label,
  dots,
  fast = false,
}: {
  icon: LucideIcon
  label: string
  dots: number
  fast?: boolean
}) {
  return (
    <div className="grid grid-cols-1 items-center gap-1.5 sm:grid-cols-[136px_minmax(0,1fr)] sm:gap-3">
      <Label icon={Icon} label={label} fast={fast} />
      <span className="flex min-h-[26px] flex-wrap items-center gap-1.5">
        {Array.from({ length: dots }, (_, index) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: the dots never reorder
            key={index}
            className={cn(
              'size-3 rounded-full motion-safe:animate-uf-pop sm:size-3.5',
              fast ? 'bg-primary' : 'bg-ink-4',
            )}
            style={delay(0.25 + index * 0.3)}
          />
        ))}
        <Done label="Done" style={delay(0.35 + dots * 0.3)} />
      </span>
    </div>
  )
}

function BarRow({
  icon,
  label,
  fast = false,
  children,
}: {
  icon: LucideIcon
  label: string
  fast?: boolean
  children: ReactNode
}) {
  return (
    <div className="grid grid-cols-1 items-center gap-1.5 sm:grid-cols-[150px_minmax(0,1fr)] sm:gap-3">
      <Label icon={icon} label={label} fast={fast} />
      <span className="flex items-center gap-2">{children}</span>
    </div>
  )
}

function Label({
  icon: Icon,
  label,
  fast,
}: {
  icon: LucideIcon
  label: string
  fast: boolean
}) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 font-semibold text-[13.5px]',
        fast ? 'text-accent-ink' : 'text-ink-2',
      )}
    >
      <Icon className="size-3.5" />
      {label}
    </span>
  )
}

function Done({ label, style }: { label?: string; style: CSSProperties }) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-full bg-green-tint font-semibold text-[12.5px] text-green-ink motion-safe:animate-uf-pop',
        label ? 'ml-1 py-0.5 pr-2 pl-1.5' : 'size-[22px] justify-center',
      )}
      style={style}
    >
      <Check className="size-3" />
      {label}
    </span>
  )
}
