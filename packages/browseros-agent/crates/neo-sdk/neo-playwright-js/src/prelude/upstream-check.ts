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

/** The report as `true`, or the op with only the kinds of drift that are not empty. */
type Checked<
  Op extends string,
  Neo,
  Up,
  Adds extends string,
  Omits extends string,
  Retypes extends string,
> =
  IsAny<Neo | Up> extends true
    ? { op: Op; unresolvedType: 'any' }
    : Report<Op, Neo, Up, Adds, Omits, Retypes> extends infer R extends {
          op: Op
        }
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

/** A narrower Neo field is compatible; every extra, missing or wider field needs a reason. */
export type UpstreamCheck<
  Op extends string,
  Neo,
  Up,
  Adds extends string = never,
  Omits extends string = never,
  Retypes extends string = never,
> =
  IsAny<Neo> extends true
    ? Checked<Op, Neo, Up, Adds, Omits, Retypes>
    : Checked<Op, Flatten<Neo>, Up, Adds, Omits, Retypes>

/**
 * The same check for an input that spells upstream's params at its top level, as Playwright's
 * are (task 006, `options!(Input, flat)` in Rust): nothing is lifted, so an `options` bag left
 * on an aligned input fails as `neoOnly: 'options'` with its keys as `upstreamOnly`.
 */
export type ExactUpstreamCheck<
  Op extends string,
  Neo,
  Up,
  Adds extends string = never,
  Omits extends string = never,
  Retypes extends string = never,
> = Checked<Op, Neo, Up, Adds, Omits, Retypes>

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

/**
 * Where upstream's command answers nothing (`void`), Neo answers `null`, never `{}`. The generated
 * family files assert it for each command row whose Rust result has no stated reason to answer
 * more (the row's `.result(..)`).
 */
export type VoidIsNull<Op extends string, UpRes, NeoRes> =
  IsAny<UpRes | NeoRes> extends true
    ? { op: Op; unresolvedType: 'any' }
    : [keyof UpRes] extends [never]
      ? [NeoRes] extends [null]
        ? true
        : { op: Op; voidButAnswers: NeoRes }
      : true

export type Assert<T extends true> = T
