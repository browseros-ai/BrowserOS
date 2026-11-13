/**
 * One probe request's immutable diagnostic policy. It protects page-derived output before
 * CDP hands it to Rust, while explicit reads and value comparisons retain native values.
 */
import { anyPrivatelyTyped } from './private-entry'
import type { FrameExpectParams } from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'
import {
  isHtmlField,
  isPrivateField,
  PRIVATE_VALUE_ATTRIBUTES,
  withMaskedFieldValues,
  withMarkedFieldsMasked,
} from './private-field-values'
import type { ProbePolicy } from './protocol'
import {
  diagnosticEngine,
  type ExpectCheck,
  expectChecks,
  ProbeError,
  type World,
} from './world'

type CheckMode =
  | { kind: 'guardedTree' }
  | { kind: 'realValue' }
  | { kind: 'native' }

/** Action-style diagnostic lengths count code points; snapshot bullets still count UTF-16. */
function characterMarker(value: string): string {
  const length = Array.from(value).length
  return `<${length} character${length === 1 ? '' : 's'}>`
}

/**
 * These pinned vendor branches read synchronously before returning their Promise. Native
 * innerText variants did not leak in the source fixture and keep their native comparisons.
 */
function guardedComparison(params: FrameExpectParams): boolean {
  switch (params.expression) {
    case 'to.match.aria':
    case 'to.have.accessible.name':
    case 'to.have.accessible.description':
      return true
    case 'to.have.text':
    case 'to.have.text.array':
    case 'to.contain.text.array':
      return params.useInnerText !== true
    default:
      return false
  }
}

/** Owns this request's comparison/reporting split and the policy retained by a delayed arm. */
export class FailureOutput {
  private readonly engine
  private readonly checks
  private readonly policy: Readonly<ProbePolicy>

  constructor(
    private readonly world: World,
    policy: ProbePolicy,
  ) {
    // The delayed arm retains these values, never a caller-owned mutable wire object.
    this.policy = Object.freeze({
      conformance: policy.conformance,
      mayHaveOtherFrame: policy.mayHaveOtherFrame,
    })
    this.engine = diagnosticEngine(world.injected)
    this.checks = expectChecks(world.injected)
  }

  /** Installs only synchronous diagnostic seams, including vendor constructor closures. */
  run<T>(invoke: () => T): T {
    if (this.policy.conformance && !anyPrivatelyTyped()) return invoke()
    return this.engine.run(
      {
        previewNode: this.previewNode,
        // Only diagnostic aliases enter the guard. Actual selector matching and explicit
        // selector generation run against the page's real values.
        generateSelectors: (elements, generate) =>
          this.tree(
            elements[0] ?? this.world.injected.document.documentElement,
            generate,
          ),
      },
      invoke,
    )
  }

  /** The caller must finish every tree read synchronously; never await inside this guard. */
  tree<T>(root: Element, read: () => T): T {
    if (this.policy.conformance && !anyPrivatelyTyped()) return read()
    // Declared fields follow the captured conformance policy. Host-typed marks share the
    // discovered prototypes and remain masked in every mode; an unmarked spec world stays native.
    return this.run(() =>
      withMaskedFieldValues(
        { root, mayHaveOtherFrame: this.policy.mayHaveOtherFrame },
        read,
        { declared: !this.policy.conformance },
      ),
    )
  }

  async check(
    element: Element,
    params: FrameExpectParams,
    frameVisible: boolean,
  ): Promise<ExpectCheck> {
    const mode = this.checkMode(element, params)
    const compare = () => this.checks.single(element, params, frameVisible)
    // The guarded branches perform their reads before the async vendor method returns.
    // Restore getters now, then await its result; viewport's later read is never guarded.
    const check = await (mode.kind === 'guardedTree'
      ? this.tree(element, compare)
      : this.run(() => withMarkedFieldsMasked(compare)))
    return mode.kind === 'realValue' && typeof check.received === 'string'
      ? { ...check, received: characterMarker(check.received) }
      : check
  }

  checkMany(elements: Element[], params: FrameExpectParams): ExpectCheck {
    const compare = () => this.checks.every(elements, params)
    return !this.policy.conformance && guardedComparison(params)
      ? this.tree(
          elements[0] ?? this.world.injected.document.documentElement,
          compare,
        )
      : this.run(() => withMarkedFieldsMasked(compare))
  }

  /** Its vendor listener and stop closure retain this request's preview after dispatch ends. */
  arm(...args: Parameters<World['injected']['setupHitTargetInterceptor']>) {
    return this.engine.arm(this.previewNode, ...args)
  }

  private checkMode(element: Element, params: FrameExpectParams): CheckMode {
    if (this.policy.conformance) return { kind: 'native' }
    if (guardedComparison(params)) return { kind: 'guardedTree' }
    if (params.expression === 'to.have.value') {
      const effective = this.world.injected.retarget(element, 'follow-label')
      return effective && isPrivateField(effective)
        ? { kind: 'realValue' }
        : { kind: 'native' }
    }
    if (!isPrivateField(element)) return { kind: 'native' }
    const property: unknown = params.expressionArg
    const sensitive =
      typeof property === 'string' &&
      ((params.expression === 'to.have.attribute.value' &&
        PRIVATE_VALUE_ATTRIBUTES.has(property.toLowerCase())) ||
        (params.expression === 'to.have.css' &&
          property.toLowerCase() === 'content') ||
        // Rust continues to refuse JS-property assertions. Keep its future value reports
        // within the same redactor before that matcher can ever be enabled.
        (params.expression === 'to.have.property' &&
          (property === 'value' || property === 'defaultValue')))
    return { kind: sensitive ? 'realValue' : 'native' }
  }

  private readonly previewNode = (node: Node): string => {
    const parent = node.parentElement
    if (
      node.nodeType === 3 &&
      parent?.localName === 'textarea' &&
      isPrivateField(parent, { declared: !this.policy.conformance })
    )
      return this.engine.previewNode(parent.ownerDocument.createTextNode(''))
    if (!isPrivateField(node, { declared: !this.policy.conformance })) return this.engine.previewNode(node)
    // Attr.value bypasses the tree guard and preserves the original marker length even
    // while an alias is generated under it. A shallow clone also drops textarea child text;
    // cloning never changes the live DOM, stored values, or page-world getters.
    // Privacy belongs to the source; the detached projection only needs the field shape.
    const clone = node.cloneNode(false)
    if (!isHtmlField(clone))
      throw new ProbeError(
        'internal',
        'private-field preview could not be projected',
      )
    for (const attribute of clone.attributes) {
      if (PRIVATE_VALUE_ATTRIBUTES.has(attribute.name.toLowerCase()))
        attribute.value = characterMarker(attribute.value)
    }
    return this.engine.previewNode(clone)
  }
}
