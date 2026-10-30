/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * neo Pro: this computer's plan, the prices on sale, a checkout's progress,
 * the licence key this computer is on, and the actions on them. Mutations
 * write their reply into the cache at the call site via `Hook.getKey()`.
 */

import type {
  ActivatePlanKeyRequest,
  CancelPlanRequest,
  Plan,
  PlanCheckout,
  PlanCheckoutStatus,
  PlanKey,
  PlanKeyChange,
  PlanPortal,
  PlanPortalRequest,
  PlanPricing,
  StartPlanCheckoutRequest,
} from '@browseros/claw-api'
import type {
  PlanCheckoutRequest,
  PlanComputerRequest,
  RenamePlanComputerOperationRequest,
} from '@browseros/claw-api-client'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

export const usePlan = createQuery<Plan>({
  queryKey: ['api', 'plan'],
  fetcher: async () => (await apiClient()).getPlan(),
})

export const usePlanPricing = createQuery<PlanPricing>({
  queryKey: ['api', 'plan-pricing'],
  fetcher: async () => (await apiClient()).getPlanPricing(),
})

/** Pass `skipToken` as the variables unless the plan says `onKey`. */
export const usePlanKey = createQuery<PlanKey, void>({
  queryKey: ['api', 'plan-key'],
  fetcher: async () => (await apiClient()).getPlanKey(),
})

export const usePlanCheckout = createQuery<
  PlanCheckoutStatus,
  PlanCheckoutRequest
>({
  queryKey: ['api', 'plan-checkout'],
  fetcher: async (request) => (await apiClient()).getPlanCheckout(request),
})

export const useStartCheckout = createMutation<
  PlanCheckout,
  StartPlanCheckoutRequest
>({
  mutationFn: async (body) => (await apiClient()).startPlanCheckout(body),
})

export const useActivateKey = createMutation<
  PlanKeyChange,
  ActivatePlanKeyRequest
>({
  mutationFn: async (body) => (await apiClient()).activatePlanKey(body),
  // The variables hold the pasted key, so the finished mutation is dropped at once.
  gcTime: 0,
})

export const useRemoveComputer = createMutation<
  PlanKeyChange,
  PlanComputerRequest
>({
  mutationFn: async (request) =>
    (await apiClient()).removePlanComputer(request),
})

export const useRenameComputer = createMutation<
  PlanKey,
  RenamePlanComputerOperationRequest
>({
  mutationFn: async (request) =>
    (await apiClient()).renamePlanComputer(request),
})

export const useResetKey = createMutation<PlanKeyChange>({
  mutationFn: async () => (await apiClient()).resetPlanKey(),
})

export const useCancelPlan = createMutation<Plan, CancelPlanRequest>({
  mutationFn: async (body) => (await apiClient()).cancelPlan(body),
})

export const useResumePlan = createMutation<Plan>({
  mutationFn: async () => (await apiClient()).resumePlan(),
})

export const useOpenPortal = createMutation<PlanPortal, PlanPortalRequest>({
  mutationFn: async (body) => (await apiClient()).openPlanPortal(body),
})

export const useRefreshPlan = createMutation<Plan>({
  mutationFn: async () => (await apiClient()).refreshPlan(),
})
