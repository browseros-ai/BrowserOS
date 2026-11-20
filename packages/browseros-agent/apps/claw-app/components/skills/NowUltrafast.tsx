/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The one-time "now ultrafast" moment for a skill made from a task: a calm ink strip on the skill
 * page and a toast on Skills. Each surface shows it once per skill, remembered in browser storage.
 */

import type { Skill } from '@browseros/claw-api'
import { skipToken } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { INK, PANEL_SHADOW } from '@/components/cockpit/InkPanel'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { useSessionDetail } from '@/modules/api/audit.hooks'
import { toast } from '@/modules/toast/toast'
import {
  durationLabel,
  runsByItself,
  turnedUltrafast,
  upgradeSeconds,
} from '@/screens/skills/skills.helpers'

type UpgradeSurface = 'skills' | 'skill_page'

const SEEN_PREFIX = 'skillUpgradeSeen:v1'

/** Keeps the moment to once per app load when browser storage is unavailable. */
const seenThisLoad = new Set<string>()

// The creation time keeps a deleted and remade skill of the same name from inheriting the old one.
function seenKey(surface: UpgradeSurface, skill: Skill): string {
  return `${SEEN_PREFIX}:${surface}:${skill.name}:${skill.createdAt}`
}

function upgradeSeen(surface: UpgradeSurface, skill: Skill): boolean {
  const key = seenKey(surface, skill)
  if (seenThisLoad.has(key)) return true
  try {
    return localStorage.getItem(key) !== null
  } catch {
    return false
  }
}

function markUpgradeSeen(surface: UpgradeSurface, skill: Skill): void {
  const key = seenKey(surface, skill)
  seenThisLoad.add(key)
  try {
    localStorage.setItem(key, String(Date.now()))
  } catch {
    // The in-memory set still keeps it to once for this app load.
  }
}

/** The calm ink strip on the skill page, shown the first time it is opened after the upgrade. */
export function NowUltrafastStrip({ skill }: { skill: Skill }) {
  // Read once: a strip already seen stays away, but one that turns ultrafast while open appears.
  const [seenBefore] = useState(() => upgradeSeen('skill_page', skill))
  const [dismissed, setDismissed] = useState(false)
  const show = turnedUltrafast(skill) && !seenBefore && !dismissed
  const source = useSessionDetail({
    variables:
      show && skill.sourceSessionId
        ? { sessionId: skill.sourceSessionId }
        : skipToken,
  })

  // Showing it is what uses up the moment, and nothing the person does marks that point.
  useEffect(() => {
    if (!show) return
    markUpgradeSeen('skill_page', skill)
    track(AnalyticsEvent.SkillUpgradeSeen, { surface: 'skill_page' })
  }, [show, skill])

  const p50Ms = skill.ultrafast?.p50Ms
  if (!show || !p50Ms) return null
  const was = durationLabel(source.data?.session.durationMs)
  return (
    <div
      role="status"
      className={cn(
        'flex flex-wrap items-center justify-between gap-x-6 gap-y-3 rounded-2xl px-6 py-5',
        INK.panel,
        PANEL_SHADOW,
      )}
    >
      <div className="flex min-w-0 flex-col">
        <UltrafastWordmark className={cn('text-[18px]', INK.ink)} />
        <h3
          className={cn(
            'mt-1.5 mb-0.5 font-bold text-[20px] tracking-[-0.015em]',
            INK.ink,
          )}
        >
          Your skill is now ultrafast.
        </h3>
        <p className={cn('m-0 text-[14.5px]', INK.ink2)}>
          {runsByItself(skill)
            ? 'neo learned it on the first run. Next time neo does it by itself.'
            : 'neo learned the fast version on the first run.'}
        </p>
      </div>
      <div className="flex items-baseline gap-[18px]">
        <span
          className={cn(
            'font-extrabold text-[40px] tabular-nums leading-none tracking-[-0.03em]',
            INK.ink,
          )}
        >
          {upgradeSeconds(p50Ms)}
          <small className={cn('ml-[3px] font-semibold text-[16px]', INK.ink2)}>
            s
          </small>
        </span>{' '}
        <span className={cn('text-[14px]', INK.ink3)}>
          {was ? (
            <>
              a run, down from <br />
              {was}
            </>
          ) : (
            'a run'
          )}
        </span>
        <button
          type="button"
          onClick={() => setDismissed(true)}
          className={cn(
            'h-[30px] rounded-lg px-3 font-semibold text-[13.5px] outline-none hover:bg-[rgb(238_243_255/0.07)] focus-visible:ring-2 focus-visible:ring-[#9cc0ff] active:translate-y-px',
            INK.ink,
          )}
        >
          Got it
        </button>
      </div>
    </div>
  )
}

/** On Skills, a one-time toast for each task skill that turned ultrafast since the list last saw it. */
export function useUpgradeToasts(skills: Skill[]): void {
  const navigate = useNavigate()
  // The list arrives from the server after the first render, and each toast is raised once.
  useEffect(() => {
    for (const skill of skills) {
      if (!turnedUltrafast(skill) || upgradeSeen('skills', skill)) continue
      markUpgradeSeen('skills', skill)
      track(AnalyticsEvent.SkillUpgradeSeen, { surface: 'skills' })
      toast({
        title: (
          <>
            <UltrafastWordmark className="mr-1.5" />
            {skill.description} is now ultrafast.
          </>
        ),
        status: 'success',
        action: {
          label: 'Open',
          onClick: () => navigate(`/skills/${encodeURIComponent(skill.name)}`),
        },
      })
    }
  }, [skills, navigate])
}
