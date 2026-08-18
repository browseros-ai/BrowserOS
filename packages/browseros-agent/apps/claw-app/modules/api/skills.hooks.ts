/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tasks (skills) surface: the list, one skill's detail, its run history, and
 * create / edit / delete. Mutations invalidate at the call site via
 * `Hook.getKey()`, so the cache surface stays grep-able.
 */

import type {
  Skill,
  SkillCreate,
  SkillDetail,
  SkillList,
  SkillRunList,
} from '@browseros/claw-api'
import type {
  SkillNameRequest,
  UpdateSkillRequest,
} from '@browseros/claw-api-client'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

export const useSkills = createQuery<SkillList>({
  queryKey: ['api', 'skills'],
  fetcher: async () => (await apiClient()).listSkills(),
})

export const useSkill = createQuery<SkillDetail, { name: string }, Error>({
  queryKey: ['api', 'skill'],
  fetcher: async ({ name }) => (await apiClient()).getSkill({ name }),
})

export const useSkillRuns = createQuery<SkillRunList, { name: string }, Error>({
  queryKey: ['api', 'skill-runs'],
  fetcher: async ({ name }) => (await apiClient()).listSkillRuns({ name }),
})

export const useCreateSkill = createMutation<Skill, SkillCreate>({
  mutationFn: async (body) => (await apiClient()).createSkill(body),
})

export const useUpdateSkill = createMutation<SkillDetail, UpdateSkillRequest>({
  mutationFn: async (request) => (await apiClient()).updateSkill(request),
})

export const useDeleteSkill = createMutation<void, SkillNameRequest>({
  mutationFn: async (request) => (await apiClient()).deleteSkill(request),
})
