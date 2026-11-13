/**
 * The prelude's view of Rust's op contract. Every input and result comes from its handler's
 * serde/ts-rs types, rendered by neo-playwright/tests/wire_generated.rs into ./generated.
 * Change the handler or route row and regenerate; the sender checks and upstream checks then
 * follow the same contract. The aliases below adapt those types for script options and
 * prelude bookkeeping. The run envelope is generated separately by neo-server (see bridge.ts).
 */

import type { SerializedValue } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'
import type { GeneratedInputs } from './generated'

// Rust owns these shared shapes now. LocatorBrand is a projection of the tagged Target
// union, so its fields are never repeated in TypeScript.
export type {
  ConsoleArgInfo,
  ConsoleInfo,
  DialogInfo,
  DownloadInfo,
  ElementBrand,
  EventRecord,
  FileChooserInfo,
  FilesInput,
  FrameInfo,
  GeneratedInputs as OpInputs,
  GeneratedOutputs as OpOutputs,
  LoadState,
  MouseButton,
  NavigationCut,
  NavigationPublication,
  NavigationResult,
  PageBrand,
  PageErrorInfo,
  PageHandle,
  PartialFrame,
  ResponseInfo,
  ScrollOption,
  SnapshotBaseline,
  SnapshotForm,
  SnapshotResult,
  Target,
  TextForm,
} from './generated'
/** Playwright's in-page serialized value (`utilityScriptSerializers.ts`), as JSON. */
export type { SerializedValue }

import type { EventRecord, PageHandle, Target } from './generated'

/** Script options are the flat Rust input with its host addressing removed. */
export type PointerOptions = Omit<
  GeneratedInputs['locator.click'],
  'target' | 'strict'
>
export type DragOptions = Omit<
  GeneratedInputs['locator.dragTo'],
  'target' | 'strict' | 'to'
>
export type FillOptions = Omit<
  GeneratedInputs['locator.fill'],
  'target' | 'strict' | 'value'
>
export type KeyOptions = Omit<
  GeneratedInputs['locator.press'],
  'target' | 'strict' | 'key'
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
 * The Playwright events Neo reports. `page` and `pageload` are the context's; `dialog`,
 * `dialogclosed` and the frame events are both; the rest are a page's. Rust takes
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
  | 'crash'
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

/** The addressing shared by typed locator actions, projected from Rust's click input. */
export type TargetArgs = Pick<
  GeneratedInputs['locator.click'],
  'target' | 'strict'
>
