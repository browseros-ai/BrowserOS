/**
 * The original saved run had six failures. These identities are immutable provenance,
 * independent of the live strictFailures policy that assigned fixes may promote later.
 */
export const historicalFailures: ReadonlySet<string> = new Set([
  'capture-io-017',
  'isolation-001',
  'isolation-002',
  'labels-001',
  'sdk-input-022',
  'sdk-input-045',
])
