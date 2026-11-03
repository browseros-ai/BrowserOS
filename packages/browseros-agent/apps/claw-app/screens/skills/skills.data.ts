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
  type Harness,
  isUserFacingHarness,
} from '@/components/harness/harness.types'
import { useConnections } from '@/modules/api/connections.hooks'
import { useSkills } from '@/modules/api/skills.hooks'
import { worksInAgents } from './skills.helpers'

export interface SkillsScreenData {
  skills: Skill[]
  /** The connected agents the skills work in; empty hides the line about them. */
  agents: Harness[]
  isLoading: boolean
  isError: boolean
}

export function useSkillsScreenData(): SkillsScreenData {
  const query = useSkills()
  const connections = useConnections()
  const skills = query.data?.items ?? []
  const connected = (connections.data?.items ?? [])
    .filter((c) => c.installed && isUserFacingHarness(c.harness))
    .map((c) => c.harness)
  return {
    skills,
    agents: worksInAgents(skills, connected),
    isLoading: query.isPending,
    isError: query.isError,
  }
}
