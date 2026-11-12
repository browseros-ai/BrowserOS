/**
 * The prelude's half of neo-playwright's op contract: every handle it sends, every op's
 * input after the shared `call` header, and every result it reads back.
 *
 * Rust decodes each input once with `deny_unknown_fields`: the header and the targets in
 * crates/neo-sdk/neo-playwright/src/target.rs, the rest in each family's `*_op.rs`. A field added
 * or renamed here must change there in the same commit, or the op fails `script /
 * invalid_input` before it sends anything. Types the probe consumes unchanged (expect params,
 * select options, URL patterns) are imported from the probe protocol, so the three halves
 * cannot drift apart.
 *
 * The op maps are being moved into Rust one family at a time: a family on route rows has its
 * inputs and results generated from its handlers' types (`./generated`, written by
 * crates/neo-sdk/neo-playwright/tests/wire_generated.rs), and its hand rows leave
 * `HandInputs` and `HandOutputs` here in the same change. `_Partition` at the end fails to
 * compile while an op is described by both, or by neither.
 */

import type { SerializedValue } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'
import type { GeneratedInputs, GeneratedOutputs } from './generated'
import type { OpName } from './generated/ops'
import type { AllOpsChecked } from './upstream-checks'

// Rust owns these shared shapes now. LocatorBrand is a projection of the tagged Target
// union, so its fields are never repeated in TypeScript.
export type {
  ConsoleInfo,
  DialogInfo,
  DownloadInfo,
  ElementBrand,
  EventRecord,
  FileChooserInfo,
  FilesInput,
  FillOptions,
  FrameInfo,
  InitSource,
  KeyOptions,
  LoadState,
  MouseButton,
  NavigationCut,
  NavigationPublication,
  NavigationResult,
  PageBrand,
  PageErrorInfo,
  PageHandle,
  PageScript,
  PartialFrame,
  ResponseInfo,
  ScrollOption,
  SnapshotResult,
  Strictness,
  Target,
} from './generated'
/** Playwright's in-page serialized value (`utilityScriptSerializers.ts`), as JSON. */
export type { SerializedValue }

import type { EventRecord, PageHandle, Strictness, Target } from './generated'

/** Script options are the flat Rust input with its host addressing removed. */
export type PointerOptions = Omit<
  GeneratedInputs['locator.click'],
  'target' | 'strict'
>
export type DragOptions = Omit<
  GeneratedInputs['locator.dragTo'],
  'target' | 'strict' | 'to'
>

export type LocatorBrand = Extract<Target, { $neo: 'locator' }>
export type UnopenedPageBrand = PageHandle & { tab: null }

/**
 * Opens every op input. `api` is the method the script called (`page.click` for the
 * `locator.click` op), which every message from the op starts with. `timeoutMs` is the
 * Playwright timeout the prelude resolved; 0 means none, and the run deadline bounds the op.
 */
export type CallHeader = { api: string; timeoutMs: number }

/**
 * The empty object still returned by host bookkeeping. Null results are spelled out per op:
 * changing this alias would misdescribe existing run-record payloads.
 */
export type Done = Record<string, never>

/**
 * The Playwright events Neo reports. `page` is the context's; the rest are a page's. Rust takes
 * the name as a string and refuses one it does not report; the prelude sends only these
 * (`isReported`).
 */
export type EventName =
  | 'worker'
  | 'workerclosed'
  | 'workerstate'
  | 'popup'
  | 'close'
  | 'dialog'
  | 'dialogclosed'
  | 'console'
  | 'pageerror'
  | 'download'
  | 'filechooser'
  | 'load'
  | 'domcontentloaded'
  | 'frameattached'
  | 'framenavigated'
  | 'framedetached'
  | 'pageload'
  | 'page'

/**
 * A page's call of a binding the run exposed (`bindingCall.resolve` answers it): Rust's
 * `binding` record, picked out of the generated union.
 */
export type BindingRecord = Extract<EventRecord, { event: 'binding' }>

export type Modifier = 'Alt' | 'Control' | 'ControlOrMeta' | 'Meta' | 'Shift'

export type WaitForState = 'attached' | 'detached' | 'visible' | 'hidden'
export type ElementStateName =
  | 'visible'
  | 'hidden'
  | 'stable'
  | 'enabled'
  | 'disabled'
  | 'editable'

/** The screenshot options Neo captures with; the prelude refuses the others before sending. */
export type ScreenshotOptions = Omit<
  GeneratedInputs['page.screenshot'],
  'page' | 'target' | 'strict'
>

/**
 * PDF channel fields accepted by pdf.rs. pdf-options.ts keeps the script's numeric lengths
 * separate and normalizes them to px strings before sending, as Playwright's client does.
 */
export type PdfOptions = Omit<GeneratedInputs['page.pdf'], 'page'>

export type TargetArgs = { target: Target; strict: Strictness }
export type EveryMatchArgs = { target: Target }
export type PageArgs = { page: PageHandle }
/** A page op on one of the page's frames; `frame` is left out for the main frame. */
export type FrameArgs = PageArgs & { frame?: string }
/**
 * Each op's input after `call`, for the ops Rust does not describe yet: none since the events
 * family moved. The close-out deletes both hand maps with `_Partition`.
 */
export type HandInputs = Record<never, never>

/** Each op's result, for the ops Rust does not describe yet: none. */
export type HandOutputs = Record<never, never>

/** Each op's input after `call`: Rust's generated type, or the hand row above. */
export type OpInputs = GeneratedInputs & HandInputs

/**
 * Each op's result. Page and element handles arrive revived by the bootstrap
 * (`__neo.brands.register`); the revivers and the classes read the raw brands.
 */
export type OpOutputs = GeneratedOutputs & HandOutputs

type Exact<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false
type Assert<T extends true> = T
/**
 * Fails to compile when an op is described both by Rust and by hand, or by neither: every op
 * in `OPS` has exactly one input and one result type, with an upstream classification. Keep
 * the remaining hand checks connected here until the last family leaves both hand maps.
 */
type _Partition = [
  AllOpsChecked,
  Assert<Exact<keyof GeneratedInputs & keyof HandInputs, never>>,
  Assert<Exact<keyof GeneratedOutputs & keyof HandOutputs, never>>,
  Assert<Exact<keyof OpInputs, OpName>>,
  Assert<Exact<keyof OpOutputs, OpName>>,
]
