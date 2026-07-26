import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

interface FrameTreeNode {
  frame: { id: string }
  childFrames?: FrameTreeNode[]
}

/** Finds a frame's depth in the page tree so annotation projection only handles safe frame cases. */
export async function frameDepth(
  pageSession: ProtocolApi,
  frameId: string,
): Promise<number | undefined> {
  const result = await pageSession.Page.getFrameTree()

  function visit(node: FrameTreeNode): number | undefined {
    if (node.frame.id === frameId) return 0
    for (const child of node.childFrames ?? []) {
      const depth = visit(child)
      if (depth !== undefined) return depth + 1
    }
    return undefined
  }

  return visit(result.frameTree as FrameTreeNode)
}
