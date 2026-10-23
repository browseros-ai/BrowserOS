type Point = { x: number; y: number };
type Box = { left: number; top: number; right: number; bottom: number };

/**
 * The corridor a pointer may cross between the trigger and the open tooltip
 * without closing it: the convex hull of where it left and the box it is
 * heading for. A fixed timeout either cuts a slow diagonal move short or keeps
 * the label alive after the pointer has plainly gone elsewhere; this follows
 * the path itself.
 */
export function graceArea(exit: Point, target: Box): Point[] {
  // A few pixels either side of the exit point give the corridor some width at
  // its narrow end, so a hand that drifts sideways while it moves stays inside.
  const pad = 5;
  return hull([
    { x: exit.x - pad, y: exit.y - pad },
    { x: exit.x + pad, y: exit.y - pad },
    { x: exit.x - pad, y: exit.y + pad },
    { x: exit.x + pad, y: exit.y + pad },
    { x: target.left, y: target.top },
    { x: target.right, y: target.top },
    { x: target.right, y: target.bottom },
    { x: target.left, y: target.bottom },
  ]);
}

export function inBox(point: Point, box: Box) {
  return (
    point.x >= box.left && point.x <= box.right && point.y >= box.top && point.y <= box.bottom
  );
}

export function inPolygon(point: Point, polygon: Point[]) {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i];
    const b = polygon[j];
    if (
      a.y > point.y !== b.y > point.y &&
      point.x < ((b.x - a.x) * (point.y - a.y)) / (b.y - a.y) + a.x
    ) {
      inside = !inside;
    }
  }
  return inside;
}

// Monotone chain; the corridor is always convex, so a hull is all it needs.
function hull(points: Point[]) {
  const sorted = [...points].sort((a, b) => a.x - b.x || a.y - b.y);
  const cross = (o: Point, a: Point, b: Point) =>
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
  const lower: Point[] = [];
  for (const p of sorted) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) {
      lower.pop();
    }
    lower.push(p);
  }
  const upper: Point[] = [];
  for (let i = sorted.length - 1; i >= 0; i--) {
    const p = sorted[i];
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) {
      upper.pop();
    }
    upper.push(p);
  }
  lower.pop();
  upper.pop();
  return lower.concat(upper);
}

/**
 * How the user last reached for the page. A focus that a mouse press caused is
 * not a request to read the label: the pointer is already on the control and
 * hovering showed it. A focus that followed a key — Tab, or a roving arrow key
 * moving focus in script — is the only way a keyboard user sees it at all.
 */
let modality: "keyboard" | "pointer" = "keyboard";
let listening = false;

export function watchModality() {
  if (listening || typeof window === "undefined") return;
  listening = true;
  window.addEventListener("keydown", () => (modality = "keyboard"), true);
  window.addEventListener("pointerdown", () => (modality = "pointer"), true);
}

export function focusFromKeyboard() {
  return modality === "keyboard";
}
