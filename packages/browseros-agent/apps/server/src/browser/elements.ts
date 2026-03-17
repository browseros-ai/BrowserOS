import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

function quadCenter(q: number[]): { x: number; y: number } {
  const x = ((q[0] ?? 0) + (q[2] ?? 0) + (q[4] ?? 0) + (q[6] ?? 0)) / 4
  const y = ((q[1] ?? 0) + (q[3] ?? 0) + (q[5] ?? 0) + (q[7] ?? 0)) / 4
  return { x, y }
}

/** 3-tier fallback: getContentQuads -> getBoxModel -> getBoundingClientRect */
export async function getElementCenter(
  session: ProtocolApi,
  backendNodeId: number,
): Promise<{ x: number; y: number }> {
  try {
    const quadsResult = await session.DOM.getContentQuads({ backendNodeId })
    if (quadsResult.quads?.length) {
      const q = quadsResult.quads[0] as unknown as number[]
      if (q && q.length >= 8) return quadCenter(q)
    }
  } catch {
    // fall through
  }

  try {
    const boxResult = await session.DOM.getBoxModel({ backendNodeId })
    const content = boxResult.model?.content as unknown as number[] | undefined
    if (content && content.length >= 8) return quadCenter(content)
  } catch {
    // fall through
  }

  const resolved = await session.DOM.resolveNode({ backendNodeId })
  const objectId = resolved.object?.objectId
  if (!objectId) {
    throw new Error(
      'Could not resolve element — it may have been removed from the page.',
    )
  }

  const boundsResult = await session.Runtime.callFunctionOn({
    functionDeclaration:
      'function(){var r=this.getBoundingClientRect();return{x:r.left,y:r.top,w:r.width,h:r.height}}',
    objectId,
    returnByValue: true,
  })

  const rect = boundsResult.result?.value as
    | { x: number; y: number; w: number; h: number }
    | undefined
  if (!rect) throw new Error('Could not get element bounds.')
  return { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 }
}

export async function scrollIntoView(
  session: ProtocolApi,
  backendNodeId: number,
): Promise<void> {
  try {
    await session.DOM.scrollIntoViewIfNeeded({ backendNodeId })
  } catch {
    // not critical
  }
}

export async function focusElement(
  session: ProtocolApi,
  backendNodeId: number,
): Promise<void> {
  const pushResult = await session.DOM.pushNodesByBackendIdsToFrontend({
    backendNodeIds: [backendNodeId],
  })
  await session.DOM.focus({ nodeId: pushResult.nodeIds[0] })
}

export async function jsClick(
  session: ProtocolApi,
  backendNodeId: number,
): Promise<void> {
  const objectId = await resolveObjectId(session, backendNodeId)
  await session.Runtime.callFunctionOn({
    functionDeclaration: 'function(){this.click()}',
    objectId,
  })
}

export async function resolveObjectId(
  session: ProtocolApi,
  backendNodeId: number,
): Promise<string> {
  const resolved = await session.DOM.resolveNode({ backendNodeId })
  const objectId = resolved.object?.objectId
  if (!objectId)
    throw new Error('Element not found in DOM. Take a new snapshot.')
  return objectId
}

/** Read the current value/textContent of an input, textarea, or contenteditable element. */
export async function getInputValue(
  session: ProtocolApi,
  backendNodeId: number,
): Promise<string> {
  try {
    const value = await callOnElement(
      session,
      backendNodeId,
      'function(){return this.value??this.textContent??""}',
    )
    return (value as string) ?? ''
  } catch {
    return ''
  }
}

export async function callOnElement(
  session: ProtocolApi,
  backendNodeId: number,
  fn: string,
  args?: unknown[],
): Promise<unknown> {
  const objectId = await resolveObjectId(session, backendNodeId)
  const result = await session.Runtime.callFunctionOn({
    functionDeclaration: fn,
    objectId,
    returnByValue: true,
    ...(args && {
      arguments: args.map((v) => ({ value: v })),
    }),
  })
  return result.result?.value
}
