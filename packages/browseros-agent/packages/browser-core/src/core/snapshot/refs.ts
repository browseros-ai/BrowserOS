export type FrameId = string
export type DocumentId = string

export interface RefEntry {
  ref: string
  backendNodeId: number
  role: string
  name: string
  /** Occurrence index of (frameId, role, name), used for stale-node re-resolution. */
  nth: number
  frameId: FrameId | undefined
}

/** Allocates public `eN` refs while preserving same-document backend-node identity. */
export class RefMap {
  readonly byRef = new Map<string, RefEntry>()
  private nextRefNum = 1
  private nextFallbackRefNum = 1
  private readonly byStableNode = new Map<string, string>()
  private readonly stableRefs = new Set<string>()
  private readonly nthCounter = new Map<string, number>()

  /** Starts a new snapshot without discarding same-document stable ref assignments. */
  beginSnapshot(): void {
    this.byRef.clear()
    this.nthCounter.clear()
    this.nextFallbackRefNum = 1
  }

  /** Returns an isolated working map for a new same-document snapshot capture. */
  forkForSnapshot(): RefMap {
    const fork = new RefMap()
    fork.nextRefNum = this.nextRefNum
    fork.byStableNode.clear()
    for (const [key, ref] of this.byStableNode) {
      fork.byStableNode.set(key, ref)
    }
    fork.stableRefs.clear()
    for (const ref of this.stableRefs) {
      fork.stableRefs.add(ref)
    }
    fork.beginSnapshot()
    return fork
  }

  /** Clears all current and stable refs for a new top-level document. */
  reset(): void {
    this.byRef.clear()
    this.byStableNode.clear()
    this.stableRefs.clear()
    this.nthCounter.clear()
    this.nextRefNum = 1
    this.nextFallbackRefNum = 1
  }

  /** Returns the public ref for a node and records its latest resolution metadata. */
  mint(node: {
    backendNodeId: number
    role: string
    name: string
    documentId?: DocumentId
    frameId?: FrameId
  }): string {
    const key = `${node.frameId ?? ''}\u0000${node.role}\u0000${node.name}`
    const nth = this.nthCounter.get(key) ?? 0
    this.nthCounter.set(key, nth + 1)

    const stableKey =
      node.documentId === undefined
        ? undefined
        : stableNodeKey({
            backendNodeId: node.backendNodeId,
            documentId: node.documentId,
            frameId: node.frameId,
          })
    const ref =
      stableKey === undefined
        ? this.nextFallbackRef()
        : this.refForStableNode(stableKey)
    this.byRef.set(ref, {
      ref,
      backendNodeId: node.backendNodeId,
      role: node.role,
      name: node.name,
      nth,
      frameId: node.frameId,
    })
    return ref
  }

  get(ref: string): RefEntry | undefined {
    return this.byRef.get(ref)
  }

  get size(): number {
    return this.byRef.size
  }

  private refForStableNode(key: string): string {
    const existing = this.byStableNode.get(key)
    if (existing !== undefined) return existing

    const ref = this.nextRef()
    this.byStableNode.set(key, ref)
    this.stableRefs.add(ref)
    return ref
  }

  private nextRef(): string {
    for (;;) {
      const ref = `e${this.nextRefNum++}`
      if (!this.isReserved(ref)) return ref
    }
  }

  private nextFallbackRef(): string {
    for (;;) {
      const ref = `e${this.nextFallbackRefNum++}`
      if (!this.isReserved(ref)) return ref
    }
  }

  private isReserved(ref: string): boolean {
    if (this.byRef.has(ref)) return true
    return this.stableRefs.has(ref)
  }
}

function stableNodeKey(node: {
  backendNodeId: number
  documentId: DocumentId
  frameId?: FrameId
}): string {
  return `${node.documentId}\u0000${node.frameId ?? ''}\u0000${node.backendNodeId}`
}
