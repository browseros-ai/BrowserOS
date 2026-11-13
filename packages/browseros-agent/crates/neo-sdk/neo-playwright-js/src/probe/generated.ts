// Generated from Rust's probe request/reply types and family-decoded value payloads by ts-rs.
// Do not edit: change Rust's probe types, then run `bun run gen:probe` in neo-playwright-js.
// PROTOCOL stays hand-bumped in protocol.ts; generating types never changes the wire version.

/**
 * Arms the hit-target interceptor. `block_all` is Playwright's `trial`.
 */
export type ArmPlan = { action: HitAction, blockAll: boolean, token: number, 
/**
 * How long the probe keeps it armed without a verdict: past the last event the op can
 * still send, so its expiry only frees a page whose Rust is gone.
 */
expiryMs: number, 
/**
 * The stability baseline: the probe arms only while the element still has this rect.
 */
expectRect?: Rect, 
/**
 * Root quads own area/viewport checks; the interceptor checks actual event coordinates.
 */
browserMapped?: boolean, };

/**
 * The install's boot argument. The same shape is generated for the page's parser; epochs
 * and document sequences retain their existing runtime validation on both sides.
 */
export type BootOptions = { frameSeq: FrameSeq, epoch: string, };

/**
 * What the look checks at one box. It carries the value's length, never the value.
 */
export type BoxAsk = { 
/**
 * The facts the host approved; absent for a check call made before any approval, which
 * checks no origin and no fact.
 */
expect?: BoxExpect, 
/**
 * The value's length in UTF-16 code units, as a box's `maxlength` counts it. Absent for
 * the host's check call, which has no value: that look skips the length check.
 */
length?: number, 
/**
 * A value the box's `maxlength` cuts is accepted.
 */
cut: boolean, 
/**
 * The field is typed key by key, so a box with no size is reached by focus alone.
 */
keys: boolean, 
/**
 * Every `<iframe>` on the way down to the box's frame is visible: a box in a hidden frame
 * does not show either.
 */
frameVisible: boolean, };

/**
 * What the host approved about a box. The look refuses the box when a fact the host sent no
 * longer holds (`box_changed`), so the box Neo types into is the box the host approved.
 *
 * The look in the page reads the same shape (probe request `look`), so the facts the host
 * approved travel to the box's isolated world unchanged.
 */
export type BoxExpect = { 
/**
 * The origin of the box's own frame, `https://pay.example`. Required: Neo never types
 * "wherever the ref ended up".
 */
origin: string, 
/**
 * The input's `type` attribute as the DOM reports it (`password`, `text`, ...).
 */
type?: string, 
/**
 * The `autocomplete` attribute, exactly as written.
 */
autocomplete?: string, 
/**
 * The box's `maxlength`.
 */
maxLength?: number, 
/**
 * The `inputmode` attribute, exactly as written.
 */
inputMode?: string, 
/**
 * The `name` attribute.
 */
name?: string, 
/**
 * The `id` attribute.
 */
id?: string, };

/**
 * An approved fact that no longer holds (`box_changed` names it). The input's `type` is not
 * one: a box of another type answers its own code, `wrong_type`.
 */
export type BoxFact = "autocomplete" | "maxLength" | "inputMode" | "name" | "id";

/**
 * A box's facts as the look read them in Neo's isolated world. The host approves these and
 * sends them back as `expect`, so the look at approval and the look at typing are one rule.
 * They describe the box, never what it holds beyond whether it is empty.
 */
export type BoxFacts = { 
/**
 * The origin of the box's own frame.
 */
origin: string, 
/**
 * The input's `type` as the DOM reports it.
 */
type: string, 
/**
 * The `autocomplete` attribute; empty when absent.
 */
autocomplete: string, 
/**
 * The `maxlength`; `None` when the box has none.
 */
maxLength: number | null, 
/**
 * The `inputmode` attribute; empty when absent.
 */
inputMode: string, 
/**
 * The `name` attribute; empty when absent.
 */
name: string, 
/**
 * The `id` attribute; empty when absent.
 */
id: string, 
/**
 * Whether the box has a size. A box with none skips the covered check: it has nothing to
 * show or cover, and is reached by focus.
 */
sized: boolean, 
/**
 * Whether the box holds nothing now.
 */
empty: boolean, };

/**
 * What the read-back found in the box.
 */
export type BoxState = "holds" | "other" | "left";

/**
 * A `checkable` read's value (the generated probe `Checkable`): upstream's `elementState(el,
 * 'checked')`, which follows a label to its control.
 */
export type Checkable = { checked: boolean, radio: boolean, };

/**
 * Which text of the document a `document` request reads.
 */
export type DocumentPart = "title" | "content" | "expectSnapshot";

/**
 * How a box's field drawing figured in its entry (rulings R55, R56): the read-back answers it,
 * and the operation's record notes every box entered under one.
 */
export type Drawing = "none" | "under" | "drawn";

/**
 * MIME data in the caller's insertion order, matching DataTransfer.types ordering.
 */
export type DropEntry = { mimeType: string, value: string, };

/**
 * An edit of a pinned element.
 */
export type Edit = { "kind": "fill", value: FillValue, } | { "kind": "selectOptions", options: Array<SelectOption>, } | { "kind": "selectText" } | { "kind": "focus", resetSelection: boolean, } | { "kind": "focusToType" } | { "kind": "blur" } | { "kind": "dispatchEvent", type: string, init: Record<string, unknown>, } | { "kind": "setFiles", payloads: Array<FilePayload>, } | { "kind": "drop", payloads: Array<FilePayload>, data: Array<DropEntry>, point: Point, };

/**
 * States the probe checks. `stable` is not one: Rust compares two measurements in time.
 */
export type ElementState = "visible" | "enabled" | "editable";

/**
 * How the probe classified an error thrown in the page.
 */
export type ErrorClass = "script" | "selector" | "unsupported" | "internal";

/**
 * Upstream's per-poll expect parameters minus the selector, passed through as the prelude
 * built them. They hold the script's expected value, which for `toHaveValue` on a password
 * field is the password, so they never print (STYLE P4).
 */
export type ExpectParams = Omit<import('../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript').FrameExpectParams, 'selector'>;

/**
 * A `fileInput` read's value (protocol.ts `FileInput`).
 */
export type FileInput = { 
/**
 * The `<input>` the files go to: the element, or the control its label names.
 */
pin: Pin, 
/**
 * The element the locator or handle named, which a directory's `input` wait listens on.
 */
target: Pin, 
/**
 * `type=file`, without which Chromium refuses paths.
 */
takesPaths: boolean, };

/**
 * One in-memory file as upstream's injected `setInputFiles` takes it, and as the prelude's
 * files.ts sends it: `mime_type` already defaulted from the name, and `buffer` the base64 text
 * upstream's server makes of the script's bytes (`buffer.toString('base64')`), which the page
 * decodes with `atob`. Local drop files also preserve their modification time; payloads use
 * the browser's current time when it is absent. A file's bytes may be large or
 * private, so they never print.
 */
export type FilePayload = { name: string, mimeType: string, buffer: string, lastModifiedMs?: number, };

/**
 * The probe's setFiles value: null on success, or the injected script's failure. Naming
 * this transparent wrapper lets the decoder and generated contract share its nullability.
 */
export type FilesSet = Threw | null;

/**
 * The text a fill puts into a field, which may be a password. It travels to the page in the
 * request, and never prints (STYLE P4).
 */
export type FillValue = string;

/**
 * A fill's answer (protocol.ts `Filled`).
 */
export type Filled = "done" | "needsinput";

/**
 * Where the focus is, as Playwright's focused rule reads it.
 */
export type FocusState = "held" | "noDocumentFocus" | "elsewhere" | "moving" | "gone";

/**
 * A `frameOrigin` read's value, also used to generate the page-side probe declaration.
 */
export type FrameOrigin = { x: number, y: number, scaleX: number, scaleY: number, } | Rotated;

/**
 * One frame document of a page. Refs print it (`f7e3` is element 3 of document 7; the main
 * frame's first document is 0 and prints bare, `e3`). A seq is never reused, and every new
 * document, of the main frame or a child frame, gets a new one (decisions.md), so a ref can
 * only ever reach the document that printed it.
 */
export type FrameSeq = number;

/**
 * `"done"`, or upstream's description of the element the pointer events reached instead,
 * which is the page's own markup.
 */
export type Hit = HitDone | { hitTargetDescription: string, };

/**
 * Which hit-target interceptor to arm.
 */
export type HitAction = "mouse" | "hover" | "drag";

/**
 * The string `"done"`.
 */
export type HitDone = "done";

/**
 * A `located` read's value: the first match, pinned, and what `waitForSelector` checks and
 * logs on each poll.
 */
export type Located = { pin: Pin, preview: string, visible: boolean, count: number, };

/**
 * The checks the page side makes, in the order it makes them (design section 5.3, steps 2
 * to 7). Step 1, the ref naming one element, and the tab's address of step 3 are Rust's.
 */
export type LookRefusal = "notAnInput" | "wrongOrigin" | "wrongType" | "boxChanged" | "boxNotEditable" | "boxHidden" | "boxCovered" | "valueTooLong";

/**
 * The first check a box fails, and for `boxChanged` the fact that differs.
 */
export type LookRefused = { code: LookRefusal, fact?: BoxFact, };

/**
 * The look's answer for one box (`value` payload of the `look` request): its facts whenever it
 * is a text input, and the first check it fails. A covered box answers both. A box whose pin
 * is gone answers the protocol's `detached` instead.
 */
export type Looked = { facts?: BoxFacts, refusal?: LookRefused, };

/**
 * A private value on its way to Neo's isolated world, where the box's listener and the
 * read-back compare with it: the whole value, or the part a `maxlength` lets in. It prints
 * as `[private]` and is zeroed when dropped, as [`PrivateText`] is. Only the entry builds one,
 * from a [`PrivateText`]; probe calls never log their requests (`probe/probe_call.rs`).
 */
export type PageSecret = string;

/**
 * A value as the probe sent it: the page's own text. It prints as `[page text]`, so no log line carries a value the page
 * shows (STYLE P4); [`PageText::into_value`] hands it to the op's output, and [`PageText::pin`]
 * takes only the probe's pin and drops the text unread.
 */
export type PageText = unknown;

/**
 * A vendored `ParsedSelector` as the page receives it. Rust never reads or builds selector
 * syntax in it, with one exception: the key of an iframe a frame hop went through
 * ([`ParsedSelector::frame_key`]).
 */
export type ParsedSelector = import('../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser').ParsedSelector;

/**
 * An element the probe pinned in one world, by number, so the calls of one op act on the
 * element its first call found (Playwright's inner retry loop does the same with a handle).
 * The probe keeps the last 512 and never reuses a number in a document, so an evicted or
 * removed pin reads as `detached` and sends the op back to re-resolve; it can never name
 * another element.
 */
export type Pin = number;

/**
 * One value of a `readAll` `pins`.
 */
export type PinnedMatch = { pin: Pin, preview: string, };

/**
 * A point in one frame's CSS pixels.
 */
export type Point = { x: number, y: number, };

/**
 * The answer to one step (`value` payload of the `private` request).
 */
export type PrivateAnswer = { "kind": "focus", focus: FocusState, } | { "kind": "holds", holds: boolean, } | { "kind": "readBack", state: BoxState, heldAtLastInput: boolean, moving: boolean, drawing: Drawing, } | { "kind": "forgot" } | { "kind": "markedFocused", marked: boolean, } | { "kind": "drawn", drawn: boolean, };

/**
 * One step of the entry on a pinned box.
 */
export type PrivateStep = { "kind": "focus", text: PageSecret, spaced: boolean, } | { "kind": "focused" } | { "kind": "holds", text: PageSecret, spaced: boolean, } | { "kind": "readBack" } | { "kind": "forget" } | { "kind": "markFocused" } | { "kind": "drawn", text: PageSecret, };

/**
 * The common private call envelope. Keeping the policy outside request variants prevents a
 * snapshot, frame hop, or pinned-node route from choosing a different diagnostic authority.
 */
export type ProbeCall = { request: ProbeRequest, policy: ProbePolicy, };

/**
 * One invocation's output policy. Worlds retain document state, while this value belongs to
 * the operation that may create diagnostics, including an interceptor's delayed callbacks.
 */
export type ProbePolicy = { conformance: boolean, 
/**
 * Only a current, complete observation can prove the frame has no adoption sources.
 */
mayHaveOtherFrame: boolean, };

/**
 * One reply; `kind` is the tag. Any request may also answer `error`, and `uninstalled` when
 * the world has no booted probe.
 */
export type ProbeReply = { "kind": "described", protocol: number, seq: FrameSeq, url: string, 
/**
 * Only from a boot that replaced the world's numbering: the seq it held before. Under
 * another epoch that is a seq an earlier `Playwright` printed refs under.
 */
retired?: FrameSeq, } | { "kind": "none", staleRef?: string, } | { "kind": "strict", message: string, } | { "kind": "element", pin: Pin, preview: string, resolved?: Resolution, } | { "kind": "frame", pin: Pin, key: string, visible: boolean, src: string, } | { "kind": "notFrame", preview: string, } | { "kind": "detached" } | { "kind": "state", missing: ElementState, resolved?: Resolution, } | { "kind": "measured", pin: Pin, rect: Rect, point: Point, inView: boolean, transformed: boolean, 
/**
 * The original element's border width plus explicit position, not a retarget's rect.
 */
borderOffset?: Point, resolved?: Resolution, } | { "kind": "covered", by: string, } | { "kind": "armed", rect: Rect, point: Point, token: number, } | { "kind": "verdict", hit: Hit, } | { "kind": "moves", moves: number, } | { "kind": "frameBox", origin: Point | null, hit: Hit | null, } | { "kind": "value", value: PageText, } | { "kind": "notFocused", holder?: string, } | { "kind": "read", value: PageText, resolved?: Resolution, } | { "kind": "values", count: number, values: Array<PageText>, } | { "kind": "selector", selector: string, } | { "kind": "snapshot", json: import('../../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot').AriaSnapshotJSON, iframes: Array<SnapshotIframe>, 
/**
 * Clickable marking threw on this page and the snapshot was taken again without it;
 * the op's call log says so.
 */
marksSkipped?: boolean, } | { "kind": "expect", matches: boolean, resolved: string, received?: PageText, ariaSnapshot?: string, } | { "kind": "urlMatch", matches: boolean, url: string, } | { "kind": "documentText", text: string, } | { "kind": "replaced" } | { "kind": "error", message: string, class: ErrorClass, 
/**
 * Only from a file input that refused the files: what its chunk resolved to.
 */
resolved?: Resolution, } | { "kind": "uninstalled" };

/**
 * One request; `op` is the tag.
 */
export type ProbeRequest = { "op": "describe" } | { "op": "resolve", target: ProbeTarget, strictness: Strictness, want: Want, } | { "op": "act", target: ProbeTarget, strictness: Strictness, retarget: Retarget, states: Array<ElementState>, 
/**
 * Every `<iframe>` on the way down is visible (the resolver's hops).
 */
frameVisible: boolean, scroll: ScrollPlan, position?: Point, arm?: ArmPlan, } | { "op": "verdict", token: number, } | { "op": "moves" } | { "op": "disarm" } | { "op": "frameBox", iframe: ProbeTarget, point?: Point, } | { "op": "edit", pin: Pin, edit: Edit, } | { "op": "look", pin: Pin, look: BoxAsk, } | { "op": "private", pin: Pin, step: PrivateStep, } | { "op": "privateShown" } | { "op": "read", target: ProbeTarget, strictness: Strictness, frameVisible: boolean, read: Read, } | { "op": "readAll", target: ProbeTarget, read: ReadAll, } | { "op": "screenshotMask", target: ProbeTarget, color: string, } | { "op": "screenshotMaskCleanup" } | { "op": "selector", target: ProbeTarget, } | { "op": "snapshot", root: ProbeTarget, mode: SnapshotMode, 
/**
 * Any integer, as upstream passes it: a child frame's depth can come out zero or
 * negative, and the engine reads those differently from no depth.
 */
depth?: number, 
/**
 * Upstream's `boxes`: each node's box in this frame's own viewport.
 */
boxes?: boolean, 
/**
 * How many of the call's nodes, first, are seeds of closed shadow roots; none outside
 * clickable marking.
 */
closedRoots?: number, } | { "op": "expect", target: ProbeTarget, strictness: Strictness, frameVisible: boolean, params: ExpectParams, } | { "op": "matchUrl", pattern: UrlPattern, url: string, } | { "op": "pinned", pin: Pin, } | { "op": "document", part: DocumentPart, includeShadow?: boolean, } | { "op": "setContent", html: string, } | { "op": "armInput", pin: Pin, } | { "op": "inputArrived", pin: Pin, } | { "op": "webStorage", kind: StorageKind, action: StorageAction, name?: string, value?: string, };

/**
 * What a request runs on inside the world.
 */
export type ProbeTarget = { "chunk": ParsedSelector } | { "pin": Pin } | "document" | "root";

/**
 * A state `read` checks, as `injected.elementState` names it.
 */
export type QueryState = "visible" | "hidden" | "enabled" | "disabled" | "editable" | "checked" | "unchecked";

/**
 * What a `read` reads from its element.
 */
export type Read = { "kind": "textContent" } | { "kind": "innerText" } | { "kind": "innerHtml" } | { "kind": "inputValue" } | { "kind": "ref" } | { "kind": "boundingBox" } | { "kind": "attribute", name: string, } | { "kind": "state", state: QueryState, } | { "kind": "frameOrigin" } | { "kind": "located" } | { "kind": "checkable" } | { "kind": "fileInput", multiple: boolean, directory: boolean, };

/**
 * What a `readAll` reads from every match.
 */
export type ReadAll = "count" | "textContents" | "innerTexts" | "pins";

/**
 * A rect in one frame's CSS pixels.
 */
export type Rect = { x: number, y: number, width: number, height: number, };

/**
 * What a request's selector chunk resolved to: the first match's preview and how many
 * matched. Only a reply to a request that targets a chunk carries one.
 */
export type Resolution = { preview: string, count: number, };

/**
 * Upstream's retarget behaviors (`InjectedScript.retarget`).
 */
export type Retarget = "none" | "follow-label" | "no-follow-label" | "button-link";

/**
 * The string `"rotated"`.
 */
export type Rotated = "rotated";

/**
 * Upstream's scroll rotation by attempt. `Protocol`, the first: Rust scrolls with
 * `DOM.scrollIntoViewIfNeeded` before the request (`protocol_scroll.rs`), and the probe only
 * measures. `End`, `Center` and `Start`: the probe's own instant `scrollIntoView`.
 */
export type ScrollPlan = "none" | "protocol" | "end" | "center" | "start";

/**
 * One `selectOptions` option, as upstream's client sends it: an option matches when every
 * attribute named here does (injected `selectOptions`), so `{ value, label }` picks the option
 * with both, and one that names none matches any option, as upstream's does.
 */
export type SelectOption = { valueOrLabel?: string, value?: string, label?: string, index?: number, };

/**
 * Why upstream's selectOptions asks for another attempt.
 */
export type SelectRetry = "error:optionsnotfound" | "error:optionnotenabled";

/**
 * A selectOptions' answer (protocol.ts `Selected`).
 */
export type Selected = Array<string> | SelectRetry;

/**
 * A rendered `<iframe>` in one frame's snapshot, for Rust to hop into.
 */
export type SnapshotIframe = { ref: string, depth: number, src: string, };

/**
 * `ariaSnapshot`'s mode.
 */
export type SnapshotMode = "ai" | "default";

/**
 * Upstream's `WebStorage` methods, the call a `webStorage` request makes.
 */
export type StorageAction = "items" | "getItem" | "setItem" | "removeItem" | "clear";

/**
 * One name/value pair in `Storage.items()`, in the document's existing iteration order.
 */
export type StorageItem = { name: string, value: string, };

/**
 * Which Storage a `webStorage` request uses.
 */
export type StorageKind = "local" | "session";

/**
 * The probe's storage results: item lists, a string, or null, without a wrapper object.
 */
export type StorageValue = Array<StorageItem> | string | null;

/**
 * Whether a resolution that matches several elements fails. The probe protocol carries it
 * with the same names.
 */
export type Strictness = "strict" | "firstMatch";

/**
 * A `setFiles` value other than null (protocol.ts `FilesSet`): what upstream's injected
 * `setInputFiles` threw, as CDP describes it.
 */
export type Threw = { threw: string, };

/**
 * A `waitForURL` pattern: a glob, or a `RegExp`'s source and flags.
 */
export type UrlPattern = { glob: string, } | { regexSource: string, regexFlags: string, };

/**
 * What a `resolve` must find.
 */
export type Want = "element" | "frame";

