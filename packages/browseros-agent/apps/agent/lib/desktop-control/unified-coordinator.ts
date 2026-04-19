/**
 * UnifiedCoordinator — smart task router for browser + desktop control.
 *
 * This is the BRAIN that decides:
 *   - Browser tasks → use BrowserOSAdapter (CDP-based control)
 *   - Desktop tasks → use DesktopControlService (native control)
 *   - File upload tasks → browser until file dialog opens, then desktop, then back
 *
 * The coordinator handles the seamless handoff between browser and desktop
 * control modes so the caller (agent) can simply say "upload file X to website Y"
 * without worrying about the mode switching.
 *
 * @module desktop-control/unified-coordinator
 */

import { DialogDetector, DialogState } from './dialog-detector'
import { FileDialogService } from './file-dialog'
import { FileManager } from './file-manager'
import { DesktopOrchestrator } from './orchestrator'
import type { DesktopControlService, OrchestratorConfig, Point } from './types'

// ─── Task Types ────────────────────────────────────────────────────

/** The mode the coordinator is currently operating in. */
export enum CoordinatorMode {
  /** Browser control via CDP / BrowserOSAdapter. */
  BROWSER = 'BROWSER',
  /** Desktop control via native APIs. */
  DESKTOP = 'DESKTOP',
  /** File dialog is open — desktop control for the dialog. */
  FILE_DIALOG = 'FILE_DIALOG',
}

/** A task to be routed by the coordinator. */
export interface UnifiedTask {
  /** Human-readable description of the task. */
  description: string
  /** Task category. */
  category: TaskCategory
  /** Optional file path for file-related tasks. */
  filePath?: string
  /** Optional URL for browser tasks. */
  url?: string
  /** Tab ID for browser operations. */
  tabId?: number
  /** Optional coordinates for desktop click targets. */
  clickTarget?: Point
}

/** Categories of tasks the coordinator can handle. */
export enum TaskCategory {
  /** General browser interaction (navigate, click, type). */
  BROWSER_ACTION = 'BROWSER_ACTION',
  /** File upload: browser click triggers file dialog → desktop handles dialog. */
  FILE_UPLOAD = 'FILE_UPLOAD',
  /** File download: browser click triggers save dialog → desktop handles dialog. */
  FILE_DOWNLOAD = 'FILE_DOWNLOAD',
  /** Pure desktop action (no browser involved). */
  DESKTOP_ACTION = 'DESKTOP_ACTION',
  /** Form fill that might trigger a file picker. */
  FORM_WITH_FILE = 'FORM_WITH_FILE',
}

/** Result of a coordinated task execution. */
export interface CoordinatorResult {
  /** Whether the task completed successfully. */
  success: boolean
  /** Current operating mode (may differ from start). */
  finalMode: CoordinatorMode
  /** Human-readable summary of what happened. */
  summary: string
  /** Steps that were executed. */
  steps: CoordinatorStep[]
  /** Error message if task failed. */
  error?: string
}

/** A single step executed by the coordinator. */
export interface CoordinatorStep {
  /** Which mode was active for this step. */
  mode: CoordinatorMode
  /** Description of what was done. */
  action: string
  /** Whether this step succeeded. */
  success: boolean
  /** Timestamp (ms since epoch). */
  timestamp: number
}

// ─── Options ───────────────────────────────────────────────────────

export interface UnifiedCoordinatorConfig {
  /** Agent server URL for vision analysis in desktop mode. */
  agentServerUrl: string
  /** Provider ID for LLM calls. */
  providerId?: string
  /** Max orchestrator iterations for desktop mode. Default 20. */
  maxIterations?: number
  /** Whether to automatically detect file dialogs after browser clicks. Default true. */
  autoDetectDialogs?: boolean
  /** AbortSignal for cancellation. */
  signal?: AbortSignal
}

// ─── BrowserAdapter Interface ──────────────────────────────────────

/**
 * Minimal interface for browser control operations.
 * The caller provides this — typically wrapping BrowserOSAdapter.
 * This decouples the coordinator from the concrete browser API.
 */
export interface BrowserControl {
  /** Click an element by its accessibility node ID. */
  clickElement(tabId: number, nodeId: number): Promise<void>
  /** Type text into an element. */
  inputText(tabId: number, nodeId: number, text: string): Promise<void>
  /** Get a snapshot of interactive elements. */
  getInteractiveSnapshot(tabId: number): Promise<unknown>
  /** Navigate to a URL. */
  navigate(tabId: number, url: string): Promise<void>
}

// ─── UnifiedCoordinator ────────────────────────────────────────────

/**
 * UnifiedCoordinator is the smart task router that seamlessly
 * switches between browser control and desktop control.
 *
 * Usage:
 *   const coordinator = new UnifiedCoordinator({ service, config })
 *   const result = await coordinator.executeUploadTask({
 *     tabId: 1,
 *     uploadButtonNodeId: 42,
 *     filePath: '/home/user/photo.jpg',
 *     browserControl: myAdapter,
 *   })
 *
 * @public
 */
export class UnifiedCoordinator {
  private service: DesktopControlService
  private config: UnifiedCoordinatorConfig
  private dialogDetector: DialogDetector
  private fileManager: FileManager
  private fileDialog: FileDialogService
  private currentMode: CoordinatorMode = CoordinatorMode.BROWSER

  constructor(deps: {
    service: DesktopControlService
    config: UnifiedCoordinatorConfig
  }) {
    this.service = deps.service
    this.config = deps.config
    this.dialogDetector = new DialogDetector(deps.service)
    this.fileManager = new FileManager(deps.service)
    this.fileDialog = new FileDialogService()
  }

  /**
   * Execute a file upload task end-to-end:
   * 1. In browser mode, click the upload button.
   * 2. Detect if a native file dialog appeared.
   * 3. If yes → switch to desktop mode, type path, confirm.
   * 4. Switch back to browser mode.
   */
  async executeUploadTask(params: {
    tabId: number
    /** Node ID of the upload/file input button to click. */
    uploadButtonNodeId: number
    /** Absolute or relative path to the file to upload. */
    filePath: string
    /** Browser control interface. */
    browserControl: BrowserControl
  }): Promise<CoordinatorResult> {
    const steps: CoordinatorStep[] = []
    const _startTime = Date.now()

    try {
      // Step 1: Validate the file path
      const resolvedPath = await this.fileManager.validateFilePath(
        params.filePath,
      )
      if (!resolvedPath) {
        return {
          success: false,
          finalMode: this.currentMode,
          summary: `File not found: ${params.filePath}`,
          steps,
          error: `File does not exist: ${params.filePath}`,
        }
      }

      steps.push({
        mode: CoordinatorMode.BROWSER,
        action: `Validated file exists: ${resolvedPath}`,
        success: true,
        timestamp: Date.now(),
      })

      // Step 2: Capture baseline screenshot for dialog detection
      if (this.config.autoDetectDialogs !== false) {
        await this.dialogDetector.captureBaseline()

        steps.push({
          mode: CoordinatorMode.BROWSER,
          action: 'Captured baseline screenshot for dialog detection',
          success: true,
          timestamp: Date.now(),
        })
      }

      // Step 3: Click the upload button in browser
      this.setMode(CoordinatorMode.BROWSER)
      await params.browserControl.clickElement(
        params.tabId,
        params.uploadButtonNodeId,
      )

      steps.push({
        mode: CoordinatorMode.BROWSER,
        action: `Clicked upload button (nodeId=${params.uploadButtonNodeId})`,
        success: true,
        timestamp: Date.now(),
      })

      // Step 4: Detect if a native file dialog opened
      if (this.config.autoDetectDialogs !== false) {
        const detection = await this.dialogDetector.detect({
          waitMs: 600,
          differenceThreshold: 0.02,
          retryWaitMs: 400,
        })

        if (detection.detected && detection.state !== DialogState.NO_DIALOG) {
          steps.push({
            mode: CoordinatorMode.FILE_DIALOG,
            action: `Detected ${detection.state} dialog (confidence: ${detection.confidence.toFixed(2)})`,
            success: true,
            timestamp: Date.now(),
          })

          // Step 5: Handle the file dialog in desktop mode
          this.setMode(CoordinatorMode.FILE_DIALOG)

          const dialogResult = await this.fileDialog.handleFileDialog({
            filePath: resolvedPath,
            confirm: true,
            timeout: 5000,
          })

          steps.push({
            mode: CoordinatorMode.FILE_DIALOG,
            action: `Typed path "${resolvedPath}" into dialog, confirmed=${dialogResult.confirmed}`,
            success: dialogResult.pathEntered && dialogResult.confirmed,
            timestamp: Date.now(),
          })

          // Step 6: Return to browser mode
          this.setMode(CoordinatorMode.BROWSER)

          // Wait for browser to process the file selection
          await this.sleep(500)

          steps.push({
            mode: CoordinatorMode.BROWSER,
            action: 'Returned to browser mode after file dialog',
            success: true,
            timestamp: Date.now(),
          })
        } else {
          steps.push({
            mode: CoordinatorMode.BROWSER,
            action:
              'No native file dialog detected — browser may have handled the file picker',
            success: true,
            timestamp: Date.now(),
          })
        }
      }

      return {
        success: true,
        finalMode: this.currentMode,
        summary: `Successfully uploaded file: ${resolvedPath}`,
        steps,
      }
    } catch (error) {
      steps.push({
        mode: this.currentMode,
        action: `Error: ${error instanceof Error ? error.message : String(error)}`,
        success: false,
        timestamp: Date.now(),
      })

      return {
        success: false,
        finalMode: this.currentMode,
        summary: `Upload failed: ${error instanceof Error ? error.message : String(error)}`,
        steps,
        error: error instanceof Error ? error.message : String(error),
      }
    }
  }

  /**
   * Execute a file download task:
   * 1. In browser mode, trigger the download (click save button).
   * 2. If a save dialog appears → switch to desktop, handle dialog.
   * 3. Return to browser mode.
   */
  async executeDownloadTask(params: {
    tabId: number
    /** Node ID of the download/save button. */
    downloadButtonNodeId: number
    /** Where to save the file. If omitted, uses default download location. */
    savePath?: string
    /** Browser control interface. */
    browserControl: BrowserControl
  }): Promise<CoordinatorResult> {
    const steps: CoordinatorStep[] = []

    try {
      // Capture baseline
      if (this.config.autoDetectDialogs !== false) {
        await this.dialogDetector.captureBaseline()
      }

      // Click download button
      this.setMode(CoordinatorMode.BROWSER)
      await params.browserControl.clickElement(
        params.tabId,
        params.downloadButtonNodeId,
      )

      steps.push({
        mode: CoordinatorMode.BROWSER,
        action: `Clicked download button (nodeId=${params.downloadButtonNodeId})`,
        success: true,
        timestamp: Date.now(),
      })

      // Detect save dialog
      if (this.config.autoDetectDialogs !== false && params.savePath) {
        const detection = await this.dialogDetector.detect({
          waitMs: 600,
          differenceThreshold: 0.02,
        })

        if (detection.detected) {
          this.setMode(CoordinatorMode.FILE_DIALOG)

          const resolvedSavePath = await this.fileManager.resolvePath(
            params.savePath,
          )
          const dialogResult = await this.fileDialog.handleFileDialog({
            filePath: resolvedSavePath,
            confirm: true,
            timeout: 5000,
          })

          steps.push({
            mode: CoordinatorMode.FILE_DIALOG,
            action: `Save dialog: typed "${resolvedSavePath}", confirmed=${dialogResult.confirmed}`,
            success: dialogResult.confirmed,
            timestamp: Date.now(),
          })

          this.setMode(CoordinatorMode.BROWSER)
          await this.sleep(500)
        }
      }

      return {
        success: true,
        finalMode: this.currentMode,
        summary: params.savePath
          ? `Download initiated, save to: ${params.savePath}`
          : 'Download initiated with default save location',
        steps,
      }
    } catch (error) {
      return {
        success: false,
        finalMode: this.currentMode,
        summary: `Download failed: ${error instanceof Error ? error.message : String(error)}`,
        steps,
        error: error instanceof Error ? error.message : String(error),
      }
    }
  }

  /**
   * Execute a pure desktop task using the DesktopOrchestrator.
   *
   * This bypasses browser control entirely and runs the full
   * vision-action loop on the desktop.
   */
  async executeDesktopTask(
    task: string,
    config?: OrchestratorConfig,
  ): Promise<CoordinatorResult> {
    this.setMode(CoordinatorMode.DESKTOP)

    const orchestrator = new DesktopOrchestrator({
      service: this.service,
      agentServerUrl: this.config.agentServerUrl,
      providerId: this.config.providerId,
    })

    const mergedConfig: OrchestratorConfig = {
      maxIterations: config?.maxIterations ?? this.config.maxIterations ?? 20,
      iterationDelay: config?.iterationDelay ?? 1000,
      actionDelay: config?.actionDelay ?? 500,
      providerId: config?.providerId ?? this.config.providerId,
      signal: config?.signal ?? this.config.signal,
    }

    try {
      const result = await orchestrator.run(task, mergedConfig)

      this.setMode(CoordinatorMode.BROWSER)

      return {
        success: result.completed,
        finalMode: CoordinatorMode.BROWSER,
        summary: result.completed
          ? `Desktop task completed in ${result.iterations} steps`
          : `Desktop task incomplete after ${result.iterations} steps: ${result.finalReasoning}`,
        steps: result.actions.map((action, i) => ({
          mode: CoordinatorMode.DESKTOP,
          action: `Step ${i + 1}: ${action.type}`,
          success: true,
          timestamp: Date.now(),
        })),
      }
    } catch (error) {
      this.setMode(CoordinatorMode.BROWSER)
      return {
        success: false,
        finalMode: CoordinatorMode.BROWSER,
        summary: `Desktop task failed: ${error instanceof Error ? error.message : String(error)}`,
        steps: [],
        error: error instanceof Error ? error.message : String(error),
      }
    }
  }

  /**
   * Analyze a task description and determine the best execution strategy.
   *
   * Returns a TaskCategory indicating which mode(s) will be needed.
   */
  analyzeTask(description: string): TaskCategory {
    const lower = description.toLowerCase()

    // File upload patterns
    if (
      lower.includes('upload') ||
      lower.includes('attach file') ||
      lower.includes('choose file') ||
      lower.includes('select file')
    ) {
      return TaskCategory.FILE_UPLOAD
    }

    // File download patterns
    if (
      lower.includes('download') ||
      lower.includes('save file') ||
      lower.includes('save as')
    ) {
      return TaskCategory.FILE_DOWNLOAD
    }

    // Desktop-only patterns
    if (
      lower.includes('open app') ||
      lower.includes('launch') ||
      lower.includes('desktop') ||
      lower.includes('system settings')
    ) {
      return TaskCategory.DESKTOP_ACTION
    }

    // Form with file patterns
    if (
      (lower.includes('form') || lower.includes('submit')) &&
      (lower.includes('file') || lower.includes('document'))
    ) {
      return TaskCategory.FORM_WITH_FILE
    }

    // Default to browser action
    return TaskCategory.BROWSER_ACTION
  }

  /**
   * Get the current operating mode.
   */
  getMode(): CoordinatorMode {
    return this.currentMode
  }

  /**
   * Get the file manager for direct file operations.
   */
  getFileManager(): FileManager {
    return this.fileManager
  }

  /**
   * Get the dialog detector for custom detection sequences.
   */
  getDialogDetector(): DialogDetector {
    return this.dialogDetector
  }

  // ─── Private Helpers ──────────────────────────────────────────────

  private setMode(mode: CoordinatorMode): void {
    const prev = this.currentMode
    this.currentMode = mode
    if (prev !== mode) {
    }
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
