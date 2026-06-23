/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Module-local constants for the Remote Hermes wire layer. URLs live in
 * `@browseros/shared/constants/urls`, secrets live in
 * `apps/server/src/env.ts`. Anything in this file is purely a tunable
 * for the WS bridge / poll loop and has no value outside this module.
 */

export const JWT_TTL_SEC = 60 * 60

export const WS_SUBPROTOCOL = 'browserclaw.v1'

// WebSocket bridge tunables.
export const IDLE_CLOSE_MS = 30 * 60 * 1000
export const IDLE_SWEEP_INTERVAL_MS = 5 * 60 * 1000
export const OPEN_DEADLINE_MS = 15_000
export const TURN_REFCOUNT_GUARD_MS = 10 * 60 * 1000
export const PING_INTERVAL_MS = 25_000
export const PONG_TIMEOUT_MS = 60_000
export const MAX_ENQUEUED_MESSAGES = 1_000
// DO sends 4001 when a newer connection for the same browserosId replaces
// us. partysocket would otherwise auto-reconnect into a flap-loop.
export const CLOSE_CODE_REPLACED = 4001

// Cold-start poll tunables (used by RemoteHermesService.streamTurn).
//   ~90s Fly machine create + image pull
//   ~5s  agent-runtime-service startup inside the VM
//   ~25s Fly healthcheck + DO status flip to 'running'
// 180s gives a 50% safety margin over the observed worst case.
export const COLD_START_BUDGET_MS = 180_000
export const STATUS_POLL_INTERVAL_MS = 2_000
