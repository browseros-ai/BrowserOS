/**
 * Low-level subpath. Re-exports the pure planner primitives and the
 * I/O boundary so consumers who need dry-run control (or want to batch
 * multiple verbs against one State) can compose them directly.
 *
 * The high-level `mcp-manager` verbs in `./api.ts` are just:
 *
 *   readState(workspaceDir, [agent]) -> plan* -> applyPlan
 *
 * Nothing is hidden. Import from here when you want to inspect the
 * Plan value before applyPlan, or when you want to run multiple
 * planner calls against the same State snapshot.
 */

export type { ApplyPlanResult, ReadStateOptions } from './io/index'
export { applyPlan, readState } from './io/index'
export {
  planDisconnect,
  planLink,
  planRemove,
  planRescan,
  planUnlink,
} from './planner/planner'
export type {
  AgentFileState,
  DisconnectInput,
  DisconnectPlanSummary,
  FsOp,
  LinkInput,
  LinkPlanSummary,
  Plan,
  RemoveInput,
  RemovePlanSummary,
  RescanInput,
  RescanReport,
  State,
  UnlinkInput,
  UnlinkPlanSummary,
} from './planner/types'
