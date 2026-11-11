/**
 * Compares Neo's hand-written wire inputs with Playwright's vendored protocol. Addressing is
 * translated by the host, so each row explains only payload differences. These types never
 * enter the prelude bundle; a failing assertion names the op and the keys that drifted.
 */
type NeoAddressing = 'target' | 'strict' | 'page' | 'frame' | 'call'
type UpstreamAddressing = 'selector' | 'strict'

/** Lift option bags, but preserve selectOption's array-valued `options` payload. */
export type Flatten<T> = T extends { options: infer O }
  ? O extends readonly unknown[]
    ? T
    : Omit<T, 'options'> & O
  : T

/** An open index signature is not an inventory of named protocol fields. */
type Keys<T> = string extends keyof T ? never : keyof T & string
type NeoKeys<T> = Exclude<Keys<T>, NeoAddressing>
type UpstreamKeys<T> = Exclude<Keys<T>, UpstreamAddressing>
type IsAny<T> = 0 extends 1 & T ? true : false
// skipLibCheck can hide an unresolved import in a vendored declaration as `any`. Refuse it
// on either side rather than allowing that broken import to certify a matching field.
type Fits<Neo, Up> =
  IsAny<Neo> extends true
    ? false
    : IsAny<Up> extends true
      ? false
      : [Neo] extends [Up]
        ? true
        : false
type Differing<K extends string, Neo, Up> = {
  [P in K]: P extends keyof Neo
    ? P extends keyof Up
      ? Fits<Neo[P], Up[P]> extends true
        ? never
        : P
      : never
    : never
}[K]

type Report<
  Op,
  Neo,
  Up,
  Adds extends string,
  Omits extends string,
  Retypes extends string,
> = {
  op: Op
  neoOnly: Exclude<NeoKeys<Neo>, Adds | UpstreamKeys<Up>>
  upstreamOnly: Exclude<UpstreamKeys<Up>, Omits | NeoKeys<Neo>>
  typeDiffers: Differing<
    Exclude<NeoKeys<Neo> & UpstreamKeys<Up>, Retypes>,
    Neo,
    Up
  >
  staleAdds: Exclude<Adds, Exclude<NeoKeys<Neo>, UpstreamKeys<Up>>>
  staleOmits: Exclude<Omits, Exclude<UpstreamKeys<Up>, NeoKeys<Neo>>>
  staleRetypes: Exclude<
    Retypes,
    Differing<NeoKeys<Neo> & UpstreamKeys<Up>, Neo, Up>
  >
}

/** A narrower Neo field is compatible; every extra, missing or wider field needs a reason. */
export type UpstreamCheck<
  Op extends string,
  Neo,
  Up,
  Adds extends string = never,
  Omits extends string = never,
  Retypes extends string = never,
> =
  IsAny<Neo | Up> extends true
    ? { op: Op; unresolvedType: 'any' }
    : Report<Op, Flatten<Neo>, Up, Adds, Omits, Retypes> extends infer R extends
          { op: Op }
      ? R[Exclude<keyof R, 'op'>] extends never
        ? true
        : {
            [K in keyof R as K extends 'op'
              ? K
              : R[K] extends never
                ? never
                : K]: R[K]
          }
      : never

/** Checks what `pick` sends, independently of a wire type that may declare more fields. */
export type KeysCheck<
  Op extends string,
  Sent extends string,
  Up,
  Adds extends string = never,
> = [Exclude<Sent, Keys<Up> | Adds>] extends [never]
  ? Exclude<Adds, Exclude<Sent, Keys<Up>>> extends never
    ? true
    : { op: Op; staleAdds: Exclude<Adds, Exclude<Sent, Keys<Up>>> }
  : { op: Op; sentButUpstreamLacks: Exclude<Sent, Keys<Up> | Adds> }

/** A type-only constraint for private/inline key lists; the same KeysCheck checks their keys. */
export type KeyList<Up> = readonly {
  [K in Keys<Up>]: KeysCheck<'pick', K, Up> extends true ? K : never
}[Keys<Up>][]

export type Assert<T extends true> = T
