// Type-only stand-in for upstream packages/protocol/src/structs.d.ts (generated, 1,000+
// lines), of which the vendored engine imports three types and the vendored protocol
// serializers one, `SerializedValue`, with `import type`. The type check maps
// `@protocol/structs` here (tsconfig.json paths); the shapes are upstream's at v1.64.1.
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

/** The protocol's value format, which the client and server read and write. */
export type SerializedValue = {
  n?: number
  b?: boolean
  s?: string
  v?: 'null' | 'undefined' | 'NaN' | 'Infinity' | '-Infinity' | '-0'
  d?: string
  u?: string
  bi?: string
  ta?: {
    b: Buffer
    k:
      | 'i8'
      | 'ui8'
      | 'ui8c'
      | 'i16'
      | 'ui16'
      | 'i32'
      | 'ui32'
      | 'f32'
      | 'f64'
      | 'bi64'
      | 'bui64'
  }
  e?: {
    m: string
    n: string
    s: string
  }
  r?: {
    p: string
    f: string
  }
  a?: SerializedValue[]
  o?: {
    k: string
    v: SerializedValue
  }[]
  h?: number
  fn?: string
  id?: number
  ref?: number
}
