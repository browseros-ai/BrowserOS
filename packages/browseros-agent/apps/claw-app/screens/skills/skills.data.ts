/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single data-aggregation hook for the Tasks list screen. The screen calls only
 * this hook and renders what it returns.
 */

import type { Skill } from '@browseros/claw-api'
import { useSkills } from '@/modules/api/skills.hooks'

export interface SkillsScreenData {
  skills: Skill[]
  isLoading: boolean
  isError: boolean
}

export function useSkillsScreenData(): SkillsScreenData {
  const query = useSkills()
  return {
    skills: query.data?.items ?? [],
    isLoading: query.isPending,
    isError: query.isError,
  }
}
