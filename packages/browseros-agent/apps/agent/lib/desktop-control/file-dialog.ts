/**
 * FileDialogService — automate native file dialog interactions.
 *
 * Detects file dialogs (via window focus change or timing heuristic),
 * types the file path, and confirms (Enter) or cancels (Escape).
 * Uses DesktopKeyboardService internally for typing and key presses.
 *
 * @module desktop-control/file-dialog
 */

import { DesktopKeyboardService } from './keyboard'
import type { FileDialogOptions, FileDialogResult } from './types'

/**
 * FileDialogService handles native file open/save dialogs.
 * @public
 */
export class FileDialogService {
  private keyboard: DesktopKeyboardService

  constructor(keyboard?: DesktopKeyboardService) {
    this.keyboard = keyboard ?? new DesktopKeyboardService()
  }

  /**
   * Handle a file dialog by typing the path and confirming/cancelling.
   *
   * Strategy:
   * 1. Wait a short time for the dialog to gain focus.
   * 2. Type the file path into the filename field.
   * 3. Press Enter to confirm or Escape to cancel.
   *
   * Note: Detection of the dialog is best-effort. In practice, file
   * dialogs steal focus immediately on most OSes, so keyboard input
   * goes to the correct field.
   */
  async handleFileDialog(
    options: FileDialogOptions,
  ): Promise<FileDialogResult> {
    const timeout = options.timeout ?? 5000
    const confirm = options.confirm ?? true

    // Wait for dialog to appear (focus heuristic)
    const detected = await this.waitForDialog(timeout)

    if (!detected) {
      return {
        detected: false,
        pathEntered: false,
        confirmed: false,
      }
    }

    // Small delay to ensure the dialog's filename field has focus
    await this.sleep(200)

    // Select all existing text (Ctrl+A) and clear it before typing
    await this.keyboard.pressKey({ key: 'a', modifiers: ['control'] })
    await this.sleep(50)

    // Type the file path
    await this.keyboard.typeText(options.filePath, { keyDelay: 5 })
    const pathEntered = true

    // Confirm or cancel
    if (confirm) {
      await this.keyboard.pressKey({ key: 'enter' })
    } else {
      await this.keyboard.pressKey({ key: 'escape' })
    }

    return {
      detected: true,
      pathEntered,
      confirmed: confirm,
    }
  }

  /**
   * Wait for the file dialog to appear.
   *
   * Uses a simple polling approach: after triggering a file dialog,
   * the calling code should invoke this method which waits a fixed
   * time. True native detection would require platform-specific APIs.
   */
  private async waitForDialog(timeoutMs: number): Promise<boolean> {
    // Best-effort: we wait for a reasonable time for the dialog to render.
    // In practice, file dialogs appear almost instantly on all major OSes.
    const pollInterval = 200
    const maxPolls = Math.ceil(timeoutMs / pollInterval)

    for (let i = 0; i < maxPolls; i++) {
      await this.sleep(pollInterval)
      // Heuristic: after a few polls, assume the dialog is visible.
      // A real implementation could check active window title via
      // a native addon, but this works for most cases.
      if (i >= 2) return true
    }

    return false
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
