import type { createAgentRoutes } from './api/routes/agents'
import type { createConversationRoutes } from './api/routes/conversations'

// Per-route client contracts for `hc`. Each protected route module is mounted at
// its own path in createApiRoutes, and the extension builds a small typed client
// per domain: hc<ConversationRoutes>(`${serverUrl}/conversations`).
//
// These are derived per-route (not from the whole app) on purpose: the full app
// type overflows the compiler (TS2589) because of /chat's AI-SDK types, and the
// Hono RPC guide recommends splitting so tsserver does not instantiate every
// route at once. Deriving from the factory's ReturnType keeps this type-only (no
// runtime, no wrapper) while tracking the route definitions automatically.
export type ConversationRoutes = ReturnType<typeof createConversationRoutes>
export type AgentRoutes = ReturnType<typeof createAgentRoutes>
