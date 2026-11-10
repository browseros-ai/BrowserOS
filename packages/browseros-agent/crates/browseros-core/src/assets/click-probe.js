// fallow-ignore-file unused-file
// Rust embeds this function with include_str!; JavaScript imports do not reach it.
// biome-ignore lint/correctness/noUnusedVariables: Rust embeds this function as a CDP functionDeclaration.
function browserosProbeClick(quads, forcedPoints) {
  // Quads and input share the session viewport: same-process frames use its top
  // document, while an OOPIF has its own coordinate basis and parent checks.
  const style = this.ownerDocument.defaultView.getComputedStyle(this)
  if (style.visibility !== 'visible' || !this.getClientRects().length)
    return { unavailable: 'Element has no visible click area.' }
  let doc = this.ownerDocument
  while (doc.defaultView.frameElement)
    doc = doc.defaultView.frameElement.ownerDocument
  const width = doc.defaultView.innerWidth
  const height = doc.defaultView.innerHeight
  const up = (node) => node && (node.parentNode || node.host)
  const reaches = (from, to) => {
    for (let node = from; node; node = up(node)) if (node === to) return true
    return false
  }
  // A pinned node exposes each of its enclosing closed roots. Remember the
  // whole chain so document hit-test retargeting can be undone at every host.
  const enclosingRoots = () => {
    const roots = new Map()
    for (let node = this; node; ) {
      const root = node.getRootNode()
      if (!root.host) break
      roots.set(root.host, root)
      node = root.host
    }
    return roots
  }
  const knownRoots = enclosingRoots()
  const framePoint = (frame, x, y) => {
    const frameStyle = frame.ownerDocument.defaultView.getComputedStyle(frame)
    const number = (value) => Number.parseFloat(value) || 0
    const boxSize = (axis, sides) =>
      number(frameStyle[axis]) +
      (frameStyle.boxSizing === 'border-box'
        ? 0
        : sides.reduce((sum, side) => sum + number(frameStyle[side]), 0))
    const width =
      boxSize('width', [
        'paddingLeft',
        'paddingRight',
        'borderLeftWidth',
        'borderRightWidth',
      ]) || frame.offsetWidth
    const height =
      boxSize('height', [
        'paddingTop',
        'paddingBottom',
        'borderTopWidth',
        'borderBottomWidth',
      ]) || frame.offsetHeight
    let matrix = new DOMMatrix()
    for (
      let node = frame;
      node;
      node = node.parentElement || node.getRootNode().host
    ) {
      const style = node.ownerDocument.defaultView.getComputedStyle(node)
      if (style.perspective !== 'none') return null
      matrix = new DOMMatrix(
        style.transform === 'none' ? undefined : style.transform,
      )
        .scale(Number.parseFloat(style.zoom) || 1)
        .multiply(matrix)
    }
    if (matrix.m14 || matrix.m24 || matrix.m44 !== 1) return null
    const corners = [
      [0, 0],
      [width, 0],
      [0, height],
      [width, height],
    ].map(([x, y]) => matrix.transformPoint({ x, y }))
    const [origin, across, down] = corners
    const ax = (across.x - origin.x) / width,
      ay = (across.y - origin.y) / width
    const bx = (down.x - origin.x) / height,
      by = (down.y - origin.y) / height
    const determinant = ax * by - ay * bx
    if (!Number.isFinite(determinant) || Math.abs(determinant) < 1e-8)
      return null
    // Ancestor transforms supply the linear part; the viewport bounds anchor
    // its translation, including layout, scroll and transform-origin offsets.
    // Convert only the child hit-test query, retaining session points for input.
    const rect = frame.getBoundingClientRect()
    const dx = x - rect.left + Math.min(...corners.map((p) => p.x)) - origin.x
    const dy = y - rect.top + Math.min(...corners.map((p) => p.y)) - origin.y
    return {
      x: (dx * by - dy * bx) / determinant - frame.clientLeft,
      y: (dy * ax - dx * ay) / determinant - frame.clientTop,
    }
  }
  const hitAt = (x, y) => {
    let currentDoc = doc
    let hit = currentDoc.elementFromPoint(x, y)
    for (let depth = 0; hit && depth < 32; depth++) {
      const root = hit.shadowRoot || knownRoots.get(hit)
      if (root) {
        const inner = root.elementFromPoint(x, y)
        if (inner && inner !== hit) {
          hit = inner
          continue
        }
      }
      let child = null
      try {
        child = hit.contentDocument
      } catch (_) {}
      if (!child || hit === this) break
      const local = framePoint(hit, x, y)
      if (!local) break
      x = local.x
      y = local.y
      currentDoc = child
      hit = currentDoc.elementFromPoint(x, y)
    }
    return hit
  }
  const accepts = (hit) => {
    if (!hit) return false
    if (hit === this || reaches(hit, this)) return true
    const label = hit.closest?.('label')
    return Boolean(label && (label.control === this || label.contains(this)))
  }
  // Clip convex content quads before sampling: the center of a very large or
  // partially off-screen element can lie outside the viewport entirely.
  const clip = (polygon, axis, boundary, sign) => {
    const output = []
    for (let i = 0; i < polygon.length; i++) {
      const a = polygon[i],
        b = polygon[(i + 1) % polygon.length]
      const insideA = sign * (a[axis] - boundary) >= 0
      const insideB = sign * (b[axis] - boundary) >= 0
      if (insideA) output.push(a)
      if (insideA !== insideB) {
        const t = (boundary - a[axis]) / (b[axis] - a[axis])
        output.push({ x: a.x + t * (b.x - a.x), y: a.y + t * (b.y - a.y) })
      }
    }
    return output
  }
  const sample = (quad) => {
    let polygon = quad
    for (const [axis, edge, sign] of [
      ['x', 0, 1],
      ['y', 0, 1],
      ['x', width, -1],
      ['y', height, -1],
    ]) {
      polygon = clip(polygon, axis, edge, sign)
    }
    if (polygon.length < 3) return []
    const center = polygon.reduce(
      (sum, p) => ({
        x: sum.x + p.x / polygon.length,
        y: sum.y + p.y / polygon.length,
      }),
      { x: 0, y: 0 },
    )
    // Center first, then inset vertices and edge midpoints. Every candidate stays
    // inside the actual transformed quad rather than its bounding rectangle.
    const samples = [center]
    for (let i = 0; i < polygon.length; i++) {
      const a = polygon[i],
        b = polygon[(i + 1) % polygon.length]
      for (const p of [a, { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }]) {
        samples.push({
          x: center.x + (p.x - center.x) * 0.9,
          y: center.y + (p.y - center.y) * 0.9,
        })
      }
    }
    return samples
  }
  const points = []
  let blocker = null
  let blockedPoint = null
  const candidates = forcedPoints || []
  if (!forcedPoints) {
    // Limit visible samples, rather than the first quads: a long wrapped link
    // can have many off-screen lines before any visible content.
    for (const quad of quads) {
      candidates.push(...sample(quad))
      if (candidates.length >= 288) break
    }
  }
  for (const point of candidates) {
    if (point.x < 0 || point.y < 0 || point.x >= width || point.y >= height)
      continue
    const hit = hitAt(point.x, point.y)
    if (accepts(hit)) points.push(point)
    else if (!blockedPoint) {
      blockedPoint = point
      blocker = hit
        ? `${hit.tagName.toLowerCase()}${hit.id ? `#${hit.id}` : ''}`
        : null
    }
  }
  return { points, blocker, blockedPoint, viewport: { width, height } }
}
