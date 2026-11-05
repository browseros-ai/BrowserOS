// Type-only stand-in for upstream packages/protocol/src/structs.d.ts (generated, 1,000+
// lines), of which the vendored engine imports three types with `import type`. The type check
// maps `@protocol/structs` here (tsconfig.json paths); the shapes are upstream's at v1.64.1.
export type ExpectedTextValue = {
  string?: string
  regexSource?: string
  regexFlags?: string
  matchSubstring?: boolean
  ignoreCase?: boolean
  normalizeWhiteSpace?: boolean
}

export type Point = {
  x: number
  y: number
}

export type Rect = {
  x: number
  y: number
  width: number
  height: number
}
