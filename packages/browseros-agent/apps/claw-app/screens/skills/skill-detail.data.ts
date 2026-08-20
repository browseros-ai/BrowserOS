/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single data-aggregation hook for the task detail screen. Resolves the skill
 * name from the route and returns the SKILL.md, its stats, and its run history.
 */

import type { SkillDetail } from '@browseros/claw-api'
import { useParams } from 'react-router'
import { useSkill } from '@/modules/api/skills.hooks'

export interface SkillDetailScreenData {
  name: string
  detail: SkillDetail | undefined
  isLoading: boolean
  isError: boolean
}

export function useSkillDetailData(): SkillDetailScreenData {
  const { name = '' } = useParams()
  const query = useSkill({
    variables: { name },
    enabled: name.length > 0,
  })
  return {
    name,
    detail: query.data,
    isLoading: query.isPending,
    isError: query.isError,
  }
}
