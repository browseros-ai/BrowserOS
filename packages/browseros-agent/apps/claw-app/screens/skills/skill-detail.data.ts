/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single data-aggregation hook for the skill detail screen. Resolves the skill
 * name from the route and returns the SKILL.md, its stats, and its run history.
 */

import type { SkillDetail } from '@browseros/claw-api'
import { useState } from 'react'
import { useParams } from 'react-router'
import {
  HARNESSES,
  type Harness,
  isUserFacingHarness,
} from '@/components/harness/harness.types'
import { useConnections } from '@/modules/api/connections.hooks'
import { usePlan } from '@/modules/api/plan.hooks'
import { useSkill } from '@/modules/api/skills.hooks'
import { showsUltrafastNudge } from '@/modules/plan/plan.helpers'
import { stillVerifying, verifyingPoll } from '@/screens/skills/skills.helpers'

export interface SkillDetailScreenData {
  name: string
  detail: SkillDetail | undefined
  /** The connected agents the phrase can be pasted into. */
  agents: Harness[]
  isLoading: boolean
  isError: boolean
  /** A pending fast version is still being checked; false once the page stops checking. */
  checking: boolean
  /** A free plan where ultrafast is on sale hears that ultrafast runs this skill in seconds. */
  nudge: boolean
}

export function useSkillDetailData(): SkillDetailScreenData {
  const { name = '' } = useParams()
  const [openedAt] = useState(() => Date.now())
  const plan = usePlan()
  const connections = useConnections()
  const connected = (connections.data?.items ?? [])
    .filter((c) => c.installed && isUserFacingHarness(c.harness))
    .map((c) => c.harness)
  const query = useSkill({
    variables: { name },
    enabled: name.length > 0,
    refetchInterval: (query) =>
      verifyingPoll(query.state.data?.skill, openedAt, Date.now()),
  })
  return {
    name,
    detail: query.data,
    agents: HARNESSES.filter((harness) => connected.includes(harness)),
    isLoading: query.isPending,
    isError: query.isError,
    // Reading dataUpdatedAt re-renders on the check that crosses the window.
    checking: stillVerifying(openedAt, query.dataUpdatedAt),
    nudge: showsUltrafastNudge(plan.data),
  }
}
