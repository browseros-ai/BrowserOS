export interface AgentsSettledInput {
  capabilitiesLoading: boolean
  agentsSupported: boolean
  urlLoading: boolean
  agentsQuerySucceeded: boolean
}

/**
 * Whether the ACP agent list is authoritative enough to repair a stored
 * selection against. Uses the query's success (not merely "fetched"), so a
 * failed agents request does not make an empty list look authoritative and wipe
 * a persisted third-party-agent default. Not-supported means nothing to load.
 */
export function computeAgentsSettled({
  capabilitiesLoading,
  agentsSupported,
  urlLoading,
  agentsQuerySucceeded,
}: AgentsSettledInput): boolean {
  if (capabilitiesLoading) return false
  if (!agentsSupported) return true
  return !urlLoading && agentsQuerySucceeded
}
