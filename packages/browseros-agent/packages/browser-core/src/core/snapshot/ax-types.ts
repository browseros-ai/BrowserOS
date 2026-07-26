// CDP Accessibility node shapes. Mirrors the subset of the protocol we render,
// kept local so the renderer is a pure function over plain data (no CDP import).

export interface AXValue {
  type: string
  value?: string | number | boolean
}

export interface AXProperty {
  name: string
  value: AXValue
}

export interface AXNode {
  nodeId: string
  ignored?: boolean
  role?: AXValue
  name?: AXValue
  description?: AXValue
  value?: AXValue
  properties?: AXProperty[]
  childIds?: string[]
  backendDOMNodeId?: number
}
