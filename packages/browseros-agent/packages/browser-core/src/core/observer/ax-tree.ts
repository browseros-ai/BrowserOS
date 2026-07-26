import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { AXNode } from '../snapshot/ax-types'

/** Fetches a session's full accessibility tree (main frame). Frame stitching layers on later. */
export async function fetchAxTree(
  session: ProtocolApi,
  params: { frameId?: string } = {},
): Promise<AXNode[]> {
  const result = await session.Accessibility.getFullAXTree(params)
  return (result.nodes as AXNode[] | undefined) ?? []
}
