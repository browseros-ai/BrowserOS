/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single data-aggregation hook for the Skills list screen. The screen calls only
 * this hook and renders what it returns.
 */

import type { Skill } from '@browseros/claw-api'
import {
  HARNESSES,
  type Harness,
  isUserFacingHarness,
} from '@/components/harness/harness.types'
import { useConnections } from '@/modules/api/connections.hooks'
import { usePlan } from '@/modules/api/plan.hooks'
import { useSkills } from '@/modules/api/skills.hooks'
import { showsUltrafastNudge } from '@/modules/plan/plan.helpers'
import { worksInAgents } from './skills.helpers'

export interface SkillsScreenData {
  skills: Skill[]
  /** The connected agents the skills work in, shown after a copy. */
  agents: Harness[]
  /** Every connected agent, for the empty state's "works in" line. */
  connected: Harness[]
  /** A free plan where ultrafast is on sale hears that ultrafast runs skills in seconds. */
  nudge: boolean
  isLoading: boolean
  isError: boolean
}

export function useSkillsScreenData(): SkillsScreenData {
  const query = useSkills()
  const connections = useConnections()
  const plan = usePlan()
  const skills = query.data?.items ?? []
  const connected = (connections.data?.items ?? [])
    .filter((c) => c.installed && isUserFacingHarness(c.harness))
    .map((c) => c.harness)
  return {
    skills,
    agents: worksInAgents(skills, connected),
    connected: HARNESSES.filter((harness) => connected.includes(harness)),
    nudge: showsUltrafastNudge(plan.data),
    isLoading: query.isPending,
    isError: query.isError,
  }
}
