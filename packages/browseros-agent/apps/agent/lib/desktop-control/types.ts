/**
 * Desktop Control Module — TypeScript types and interfaces.
 *
 * Provides native desktop control (mouse, keyboard, screenshots, file dialogs)
 * for AI agent vision–action loops similar to Claude Computer Use.
 *
 * @module desktop-control
 */

// ─── Coordinate & Geometry ─────────────────────────────────────────

/** A point on the primary or a specific display. */
export interface Point {
  x: number
  y: number
  /** Display index (0-based). Defaults to 0 (primary). */
  displayId?: number
}

/** A rectangular region on screen. */
export interface ScreenRegion {
  x: number
  y: number
  width: number
  height: number
  displayId?: number
}

// ─── Display / Monitor ─────────────────────────────────────────────

export interface DisplayInfo {
  id: number
  /** Pixel width of the display. */
  width: number
  /** Pixel height of the display. */
  height: number
  /** Pixel offset from left edge of virtual desktop. */
  xOffset: number
  /** Pixel offset from top edge of virtual desktop. */
  yOffset: number
  /** Scaling factor (e.g. 2 for Retina). */
  scaleFactor: number
  /** Whether this is the primary display. */
  isPrimary: boolean
}

// ─── Mouse ─────────────────────────────────────────────────────────

export type MouseButton = 'left' | 'right' | 'middle'

export type ClickType = 'single' | 'double'

export interface MouseMoveOptions {
  /** Delay in ms between micro-steps for smooth movement (default 2). */
  smoothDuration?: number
}

export interface MouseClickOptions {
  button?: MouseButton
  clickType?: ClickType
}

export interface MouseDragOptions {
  /** Delay in ms for the entire drag gesture (default 300). */
  duration?: number
  /** Steps to break the drag into (default 20). */
  steps?: number
}

export type ScrollDirection = 'up' | 'down' | 'left' | 'right'

export interface MouseScrollOptions {
  /** Number of "clicks" to scroll (default 3). */
  amount?: number
  direction?: ScrollDirection
}

// ─── Keyboard ──────────────────────────────────────────────────────

/** Modifier keys that can be held during a key press. */
export type KeyModifier = 'alt' | 'control' | 'shift' | 'meta'

export interface KeyCombination {
  /** Key name (e.g. 'c', 'enter', 'tab', 'escape', 'f1'). */
  key: string
  /** Modifiers held while pressing the key. */
  modifiers?: KeyModifier[]
}

export interface TypeTextOptions {
  /** Delay in ms between each keystroke (default 10). */
  keyDelay?: number
}

// ─── Screenshot ────────────────────────────────────────────────────

export type ScreenshotFormat = 'png' | 'jpg'

export interface ScreenshotOptions {
  /** Specific display to capture (omit for all displays). */
  displayId?: number
  /** Crop region (in screen coordinates). */
  region?: ScreenRegion
  /** Output format. Default 'png'. */
  format?: ScreenshotFormat
  /** JPEG quality 1-100 (ignored for PNG). Default 80. */
  quality?: number
}

export interface ScreenshotResult {
  /** Base64-encoded image data (no data-uri prefix). */
  base64: string
  /** Width of the captured image in pixels. */
  width: number
  /** Height of the captured image in pixels. */
  height: number
  /** MIME type, e.g. 'image/png'. */
  mimeType: string
  /** Display index that was captured. */
  displayId: number
}

// ─── File Dialog ───────────────────────────────────────────────────

export type FileDialogAction = 'open' | 'save'

export interface FileDialogOptions {
  /** Full path to type into the dialog's filename field. */
  filePath: string
  /** Whether to press Enter to confirm (true) or Escape to cancel (false). Default true. */
  confirm?: boolean
  /** Timeout in ms to wait for the dialog to appear. Default 5000. */
  timeout?: number
}

export interface FileDialogResult {
  /** Whether the dialog was detected. */
  detected: boolean
  /** Whether the path was entered. */
  pathEntered: boolean
  /** Whether the dialog was confirmed or cancelled. */
  confirmed: boolean
}

// ─── Orchestrator / Vision-Action Loop ─────────────────────────────

export type DesktopActionType =
  | 'mouse_move'
  | 'mouse_click'
  | 'mouse_double_click'
  | 'mouse_right_click'
  | 'mouse_drag'
  | 'mouse_scroll'
  | 'keyboard_type'
  | 'keyboard_key'
  | 'keyboard_hotkey'
  | 'file_dialog'
  | 'screenshot'
  | 'wait'
  | 'done'

export interface DesktopAction {
  type: DesktopActionType
  params: Record<string, unknown>
}

export interface VisionAnalysisRequest {
  /** Base64 screenshot to analyse. */
  screenshot: string
  /** MIME type of the screenshot. */
  mimeType: string
  /** The user's original task / goal. */
  task: string
  /** Previous actions taken in this loop (for context). */
  previousActions: DesktopAction[]
}

export interface VisionAnalysisResponse {
  /** Reasoning about current screen state (for debugging / logging). */
  reasoning: string
  /** The next action to execute, or 'done' when finished. */
  action: DesktopAction
}

export interface OrchestratorConfig {
  /** Max iterations before forcing a stop. Default 20. */
  maxIterations?: number
  /** Delay in ms between iterations. Default 1000. */
  iterationDelay?: number
  /** Delay in ms after executing an action before taking next screenshot. Default 500. */
  actionDelay?: number
  /** Provider ID to use for vision analysis. Falls back to default provider. */
  providerId?: string
  /** AbortSignal to cancel the loop. */
  signal?: AbortSignal
}

export interface OrchestratorResult {
  /** Whether the task was completed. */
  completed: boolean
  /** Total iterations executed. */
  iterations: number
  /** All actions taken. */
  actions: DesktopAction[]
  /** Final reasoning from the vision model. */
  finalReasoning: string
}

// ─── DesktopControl Service ────────────────────────────────────────

/**
 * Unified interface for the desktop control service.
 * All native operations are behind async methods so implementations
 * can be swapped (real native, mock, or remote).
 */
export interface DesktopControlService {
  // Screenshot
  captureScreenshot(options?: ScreenshotOptions): Promise<ScreenshotResult>
  getDisplays(): Promise<DisplayInfo[]>

  // Mouse
  mouseMove(point: Point, options?: MouseMoveOptions): Promise<void>
  mouseClick(point: Point, options?: MouseClickOptions): Promise<void>
  mouseDrag(from: Point, to: Point, options?: MouseDragOptions): Promise<void>
  mouseScroll(point: Point, options?: MouseScrollOptions): Promise<void>
  getMousePosition(): Promise<Point>

  // Keyboard
  typeText(text: string, options?: TypeTextOptions): Promise<void>
  pressKey(combination: KeyCombination): Promise<void>

  // File dialog
  handleFileDialog(options: FileDialogOptions): Promise<FileDialogResult>

  /** Clean up native resources. */
  dispose(): Promise<void>
}
