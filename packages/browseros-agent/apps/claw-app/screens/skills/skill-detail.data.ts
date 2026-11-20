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
import { useSkill } from '@/modules/api/skills.hooks'
import { stillVerifying, verifyingPoll } from '@/screens/skills/skills.helpers'

export interface SkillDetailScreenData {
  name: string
  detail: SkillDetail | undefined
  isLoading: boolean
  isError: boolean
  /** A pending fast version is still being checked; false once the page stops checking. */
  checking: boolean
}

export function useSkillDetailData(): SkillDetailScreenData {
  const { name = '' } = useParams()
  const [openedAt] = useState(() => Date.now())
  const query = useSkill({
    variables: { name },
    enabled: name.length > 0,
    refetchInterval: (query) =>
      verifyingPoll(query.state.data?.skill, openedAt, Date.now()),
  })
  return {
    name,
    detail: query.data,
    isLoading: query.isPending,
    isError: query.isError,
    // Reading dataUpdatedAt re-renders on the check that crosses the window.
    checking: stillVerifying(openedAt, query.dataUpdatedAt),
  }
}
