/**
 * `expect`: one poll of one retrying matcher, matched by Playwright's own per-element and
 * per-array checks (world.ts `expectChecks`). Rust's loop owns the timing, `.not`, the failure
 * and its message; a poll only reports what it saw, and never waits.
 *
 * Value matchers on a password, card or one-time-code field (design D14) never match the
 * masked value the engine reads in this world. `toHaveValue` and `toHaveValues` compare the
 * live value through `__clawSeal.valueEquals`, `toHaveAttribute('value', ...)` the markup's
 * attribute through `__clawSeal.attributeEquals`, and each reports the masked value as
 * received. A RegExp there is refused: each match would tell the script a
 * little more of the secret (`/^a/`, `/^b/`, ...), so the seal never evaluates one.
 */
import type { FrameExpectParams } from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'
import type {
  ExpectPoll,
  ExpectRequest,
  ReplyTo,
  SensitiveKind,
} from './protocol'
import { queryAll, resolveTarget } from './resolve'
import { type ExpectCheck, expectChecks, ProbeError, type World } from './world'

const REGEX_REFUSED =
  'a RegExp is not matched against a password, card or one-time-code field, because each match would reveal part of the secret; compare with the exact string'
const OPTIONS_REFUSED =
  'a password, card or one-time-code field is compared only as an exact string, without ignoreCase'

export async function expect(
  world: World,
  request: ExpectRequest,
  root: Node | undefined,
): Promise<ReplyTo<'expect'>> {
  const params: FrameExpectParams = request.params
  const checks = expectChecks(world.injected)
  // Upstream's split (InjectedScript.expect): count and the array forms read every match,
  // and zero matches is a value for them, not a missing element.
  if (
    params.expression === 'to.have.count' ||
    params.expression.endsWith('.array')
  ) {
    const elements = queryAll(world, request.target, root)
    const count = `${elements.length} element${elements.length === 1 ? '' : 's'}`
    return poll(checks.every(elements, params), count, null)
  }
  const found = resolveTarget(world, request.target, request.strictness, root)
  if (found.kind !== 'one') return found
  const { element } = found
  const resolved = world.injected.previewNode(element)
  const sealed = sealedCheck(world, element, params)
  if (sealed) return poll(sealed.check, resolved, sealed.kind)
  const check = await checks.single(element, params, request.frameVisible)
  return poll(check, resolved, null)
}

function poll(
  check: ExpectCheck,
  resolved: string,
  sensitive: SensitiveKind | null,
): ExpectPoll {
  return {
    kind: 'expect',
    matches: check.matches,
    resolved,
    received: check.received,
    sensitive,
  }
}

/**
 * A value matcher's check on a sensitive field, or undefined to let the engine match: any
 * other matcher, a field the seal does not classify, or an element the engine itself refuses
 * (`Not an input element`). The field is the one the engine would read: `toHaveValue` and
 * `toHaveValues` follow a label to its control, `toHaveAttribute` reads the element itself.
 */
function sealedCheck(
  world: World,
  element: Element,
  params: FrameExpectParams,
): { check: ExpectCheck; kind: SensitiveKind } | undefined {
  const { expression } = params
  const valueAttribute =
    expression === 'to.have.attribute.value' &&
    String(params.expressionArg).toLowerCase() === 'value'
  if (
    expression !== 'to.have.value' &&
    expression !== 'to.have.values' &&
    !valueAttribute
  )
    return undefined
  const field = valueAttribute
    ? element
    : world.injected.retarget(element, 'follow-label')
  const kind = field ? world.seal.kind(field) : null
  if (!field || !kind) return undefined
  const expected = exactStrings(params)
  if (expression === 'to.have.values')
    return selectCheck(world, field, expected, kind)
  const [wanted] = expected
  if (expected.length !== 1 || wanted === undefined)
    throw new ProbeError('internal', `${expression} takes one expected value`)
  if (valueAttribute)
    return { check: attributeCheck(world, field, wanted), kind }
  return {
    check: {
      matches: world.seal.valueEquals(field, wanted),
      received: sealedValue(field),
    },
    kind,
  }
}

/** The expected strings; anything but an exact string is refused before it is compared. */
function exactStrings(params: FrameExpectParams): string[] {
  return (params.expectedText ?? []).map((item) => {
    if (item.regexSource !== undefined)
      throw new ProbeError('unsupported', REGEX_REFUSED)
    if (item.ignoreCase || item.matchSubstring || item.normalizeWhiteSpace)
      throw new ProbeError('unsupported', OPTIONS_REFUSED)
    return item.string ?? ''
  })
}

/**
 * `toHaveAttribute('value', ...)` reads the attribute, the page's markup, which keeps its
 * value when the field is edited; the live value is another string. The seal compares the raw
 * attribute exactly, as upstream's string matcher does, and an absent one never matches. The
 * received value is the attribute as this world reads it: `null`, `''` (the seal masks only
 * non-empty values) or the mask.
 */
function attributeCheck(
  world: World,
  field: Element,
  wanted: string,
): ExpectCheck {
  return {
    matches: world.seal.attributeEquals(field, 'value', wanted),
    received: field.getAttribute('value'),
  }
}

/**
 * `toHaveValues` on a sensitive multiple select. The seal compares one value, the first
 * selected option's, so a field with several options selected is refused; the lengths are
 * compared first, as upstream does.
 */
function selectCheck(
  world: World,
  field: Element,
  expected: string[],
  kind: SensitiveKind,
): { check: ExpectCheck; kind: SensitiveKind } | undefined {
  if (field.nodeName !== 'SELECT') return undefined
  const select = field as HTMLSelectElement
  if (!select.multiple) return undefined
  const selected = select.selectedOptions.length
  if (selected > 1)
    throw new ProbeError(
      'unsupported',
      'toHaveValues compares a password, card or one-time-code select only while at most one option is selected',
    )
  const received = selected === 0 ? [] : [sealedValue(select)]
  const [wanted] = expected
  const matches =
    selected === expected.length &&
    (wanted === undefined || world.seal.valueEquals(select, wanted))
  return { check: { matches, received }, kind }
}

/** A field's value through this world's sealed getter: `••••` for a sensitive field. */
function sealedValue(field: Element): string {
  const value: unknown = Reflect.get(field, 'value')
  return typeof value === 'string' ? value : ''
}
