/** Compiler tests for the guard itself; typecheck must reject drift and obsolete exceptions. */
import type {
  Assert,
  ExactUpstreamCheck,
  Flatten,
  KeyList,
  KeysCheck,
  UpstreamCheck,
  VoidIsNull,
} from '../src/prelude/upstream-check'

type Empty = Record<never, never>
// biome-ignore lint/suspicious/noExplicitAny: model skipLibCheck's unresolved imports.
type Unresolved = any

type Equal<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false

export type GuardChecks = [
  Assert<
    UpstreamCheck<
      'nested',
      { target: object; options: { flag: true } },
      { selector: string; flag?: boolean }
    >
  >,
  Assert<Equal<Flatten<{ options: string[] }>, { options: string[] }>>,
  Assert<
    Equal<
      UpstreamCheck<'extra', { extra: string }, Empty>,
      { op: 'extra'; neoOnly: 'extra' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'missing', Empty, { referer?: string }>,
      { op: 'missing'; upstreamOnly: 'referer' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'wide', { value: string }, { value: 'one' }>,
      { op: 'wide'; typeDiffers: 'value' }
    >
  >,
  Assert<
    UpstreamCheck<
      'reasoned',
      { extra: string; value: string },
      { missing: number; value: 'one' },
      'extra',
      'missing',
      'value'
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'stale-add', { extra: string }, { extra: string }, 'extra'>,
      { op: 'stale-add'; staleAdds: 'extra' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'removed-add', Empty, Empty, 'extra'>,
      { op: 'removed-add'; staleAdds: 'extra' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'stale-omit', Empty, Empty, never, 'missing'>,
      { op: 'stale-omit'; staleOmits: 'missing' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<
        'filled-omit',
        { missing: string },
        { missing: string },
        never,
        'missing'
      >,
      { op: 'filled-omit'; staleOmits: 'missing' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<
        'stale-retype',
        { value: string },
        { value: string },
        never,
        never,
        'value'
      >,
      { op: 'stale-retype'; staleRetypes: 'value' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'missing-retype', Empty, Empty, never, never, 'value'>,
      { op: 'missing-retype'; staleRetypes: 'value' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'selector', { selector: string }, { selector: string }>,
      { op: 'selector'; neoOnly: 'selector' }
    >
  >,
  Assert<
    UpstreamCheck<
      'selector',
      { selector: string },
      { selector: string },
      'selector'
    >
  >,
  // An unresolved vendored import must fail even when skipLibCheck suppresses its diagnostic.
  Assert<
    Equal<
      UpstreamCheck<'any-field', { value: string }, { value: Unresolved }>,
      { op: 'any-field'; typeDiffers: 'value' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'any-neo', Unresolved, Empty>,
      { op: 'any-neo'; unresolvedType: 'any' }
    >
  >,
  Assert<
    Equal<
      UpstreamCheck<'any-upstream', Empty, Unresolved>,
      { op: 'any-upstream'; unresolvedType: 'any' }
    >
  >,
  Assert<
    Equal<
      KeysCheck<'hover', 'steps', { force?: boolean }>,
      { op: 'hover'; sentButUpstreamLacks: 'steps' }
    >
  >,
  Assert<KeysCheck<'temporary', 'noWaitAfter', Empty, 'noWaitAfter'>>,
  Assert<
    Equal<
      KeysCheck<
        'stale-key',
        'noWaitAfter',
        { noWaitAfter?: boolean },
        'noWaitAfter'
      >,
      { op: 'stale-key'; staleAdds: 'noWaitAfter' }
    >
  >,
  Assert<Equal<KeyList<{ force?: boolean }>, readonly 'force'[]>>,
  // An aligned (flat) input passes as it is; the same input still nested in a bag fails.
  Assert<
    ExactUpstreamCheck<
      'flat',
      { target: object; strict: 'strict'; force?: boolean },
      { selector: string; strict?: boolean; force?: boolean }
    >
  >,
  Assert<
    Equal<
      ExactUpstreamCheck<
        'still-nested',
        { target: object; options: { force?: boolean } },
        { selector: string; force?: boolean }
      >,
      { op: 'still-nested'; neoOnly: 'options'; upstreamOnly: 'force' }
    >
  >,
  Assert<
    Equal<
      ExactUpstreamCheck<'any-exact', Unresolved, Empty>,
      { op: 'any-exact'; unresolvedType: 'any' }
    >
  >,
  Assert<VoidIsNull<'void', void, null>>,
  Assert<
    Equal<
      VoidIsNull<'answers', void, Empty>,
      { op: 'answers'; voidButAnswers: Empty }
    >
  >,
  Assert<VoidIsNull<'valued', { value: string }, string>>,
  Assert<
    Equal<
      VoidIsNull<'any-result', Unresolved, null>,
      { op: 'any-result'; unresolvedType: 'any' }
    >
  >,
]
