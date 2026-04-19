/**
 * Desktop Agent Tools — register all desktop capabilities as BrowserOS agent tools.
 *
 * Provides 20 tool definitions that the BrowserOS agent can call to control
 * the desktop: screenshots, mouse/keyboard, file management, app control,
 * clipboard, system info, and end-to-end file upload/download flows.
 *
 * Each tool follows the existing `defineTool` / `defineToolWithCategory` pattern
 * from `apps/server/src/tools/framework.ts`, using Zod schemas for input
 * validation and `ToolResponse` for structured output.
 *
 * @module desktop-control/desktop-agent-tools
 */

import { z } from 'zod'
import type { ToolDefinition } from '../../../server/src/tools/framework'
import { defineToolWithCategory } from '../../../server/src/tools/framework'
import { AppLauncher } from './app-launcher'
import { DesktopExplorer } from './desktop-explorer'
import { FileManager } from './file-manager'
import { DesktopKeyboardService } from './keyboard'
import { DesktopMouseService } from './mouse'
import { OSCommands } from './os-commands'
import { DesktopScreenshotService } from './screenshot'
import type { DesktopControlService } from './types'

// ─── Approval Category ─────────────────────────────────────────────
// Desktop tools are classified under 'input' since they perform
// real-world actions (mouse clicks, keyboard, file ops).
const defineDesktopTool = defineToolWithCategory('input')

// ─── Desktop Tool Context ──────────────────────────────────────────

/**
 * Extended context for desktop tools.
 * Holds lazily-initialised service singletons.
 */
export interface DesktopToolServices {
  screenshot: DesktopScreenshotService
  mouse: DesktopMouseService
  keyboard: DesktopKeyboardService
  appLauncher: AppLauncher
  explorer: DesktopExplorer
  osCommands: OSCommands
  fileManager: FileManager
  service: DesktopControlService
}

let _services: DesktopToolServices | null = null

/**
 * Get (or create) the singleton DesktopToolServices instance.
 * Accepts an optional DesktopControlService — if none is provided,
 * a minimal stub implementation is used.
 */
export function getDesktopServices(
  service?: DesktopControlService,
): DesktopToolServices {
  if (!_services) {
    const screenshot = new DesktopScreenshotService()
    const mouse = new DesktopMouseService()
    const keyboard = new DesktopKeyboardService()
    const appLauncher = new AppLauncher()
    const osCommands = new OSCommands()

    // Create a stub DesktopControlService that delegates to the real services.
    // If a real service is provided, we still wrap it for convenience.
    const svc: DesktopControlService = service ?? {
      captureScreenshot: (opts) => screenshot.captureScreenshot(opts),
      getDisplays: () => screenshot.getDisplays(),
      mouseMove: (p, o) => mouse.mouseMove(p, o),
      mouseClick: (p, o) => mouse.mouseClick(p, o),
      mouseDrag: (f, t, o) => mouse.mouseDrag(f, t, o),
      mouseScroll: (p, o) => mouse.mouseScroll(p, o),
      getMousePosition: () => mouse.getMousePosition(),
      typeText: (t, o) => keyboard.typeText(t, o),
      pressKey: (c) => keyboard.pressKey(c),
      handleFileDialog: () =>
        Promise.resolve({
          detected: false,
          pathEntered: false,
          confirmed: false,
        }),
      dispose: () => Promise.resolve(),
    }

    const fileManager = new FileManager(svc)
    const explorer = new DesktopExplorer(svc)

    _services = {
      screenshot,
      mouse,
      keyboard,
      appLauncher,
      explorer,
      osCommands,
      fileManager,
      service: svc,
    }
  }
  return _services
}

/** Reset services (for testing). */
export function resetDesktopServices(): void {
  _services = null
}

// ─── 1. desktop_screenshot ────────────────────────────────────────

export const desktop_screenshot = defineDesktopTool({
  name: 'desktop_screenshot',
  description:
    'Capture a screenshot of the desktop. Returns the image as base64. ' +
    'Use this to see the current state of the desktop, detect dialogs, or ' +
    'verify the result of a desktop action.',
  input: z.object({
    displayId: z
      .number()
      .optional()
      .default(0)
      .describe('Display index (0 = primary).'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.screenshot.captureScreenshot({
        displayId: args.displayId,
      })
      response.text(
        `Desktop screenshot captured (${result.width}×${result.height}, display ${result.displayId}).`,
      )
      response.image(result.base64, result.mimeType)
      response.data({
        width: result.width,
        height: result.height,
        displayId: result.displayId,
        mimeType: result.mimeType,
      })
    } catch (error) {
      response.error(
        `Screenshot failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 2. desktop_click ─────────────────────────────────────────────

export const desktop_click = defineDesktopTool({
  name: 'desktop_click',
  description:
    'Click at screen coordinates (x, y) on the desktop. ' +
    'Use desktop_screenshot first to see where to click.',
  input: z.object({
    x: z.number().describe('X coordinate on screen.'),
    y: z.number().describe('Y coordinate on screen.'),
    button: z
      .enum(['left', 'right', 'middle'])
      .optional()
      .default('left')
      .describe('Mouse button.'),
    clickType: z
      .enum(['single', 'double'])
      .optional()
      .default('single')
      .describe('Single or double click.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      await svc.mouse.mouseClick(
        { x: args.x, y: args.y },
        { button: args.button, clickType: args.clickType },
      )
      response.text(
        `Clicked at (${args.x}, ${args.y}) with ${args.button} button (${args.clickType}).`,
      )
      response.data({
        x: args.x,
        y: args.y,
        button: args.button,
        clickType: args.clickType,
      })
    } catch (error) {
      response.error(
        `Click failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 3. desktop_type ──────────────────────────────────────────────

export const desktop_type = defineDesktopTool({
  name: 'desktop_type',
  description:
    'Type text on the desktop. Sends keystrokes to whatever window currently has focus. ' +
    'Use this to enter text into fields after clicking on them with desktop_click.',
  input: z.object({
    text: z.string().describe('Text to type.'),
    keyDelay: z
      .number()
      .optional()
      .default(10)
      .describe('Delay in ms between keystrokes.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      await svc.keyboard.typeText(args.text, { keyDelay: args.keyDelay })
      response.text(`Typed ${args.text.length} characters.`)
      response.data({ textLength: args.text.length })
    } catch (error) {
      response.error(
        `Type failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 4. desktop_hotkey ────────────────────────────────────────────

export const desktop_hotkey = defineDesktopTool({
  name: 'desktop_hotkey',
  description:
    'Press a key combination (hotkey) on the desktop. ' +
    'Examples: Ctrl+C (copy), Alt+Tab (switch window), Ctrl+S (save), Cmd+Q (quit on macOS).',
  input: z.object({
    key: z
      .string()
      .describe("Key name: 'c', 'enter', 'tab', 'escape', 'f1', etc."),
    modifiers: z
      .array(z.enum(['alt', 'control', 'shift', 'meta']))
      .optional()
      .default([])
      .describe('Modifier keys to hold.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      await svc.keyboard.pressKey({ key: args.key, modifiers: args.modifiers })
      const combo =
        args.modifiers.length > 0
          ? `${args.modifiers.join('+')}+${args.key}`
          : args.key
      response.text(`Pressed hotkey: ${combo}`)
      response.data({ key: args.key, modifiers: args.modifiers })
    } catch (error) {
      response.error(
        `Hotkey failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 5. desktop_scroll ────────────────────────────────────────────

export const desktop_scroll = defineDesktopTool({
  name: 'desktop_scroll',
  description: 'Scroll at a position on the desktop.',
  input: z.object({
    x: z.number().describe('X coordinate.'),
    y: z.number().describe('Y coordinate.'),
    direction: z
      .enum(['up', 'down', 'left', 'right'])
      .optional()
      .default('down')
      .describe('Scroll direction.'),
    amount: z
      .number()
      .optional()
      .default(3)
      .describe('Number of scroll ticks.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      await svc.mouse.mouseScroll(
        { x: args.x, y: args.y },
        { direction: args.direction, amount: args.amount },
      )
      response.text(
        `Scrolled ${args.direction} by ${args.amount} at (${args.x}, ${args.y}).`,
      )
      response.data({
        x: args.x,
        y: args.y,
        direction: args.direction,
        amount: args.amount,
      })
    } catch (error) {
      response.error(
        `Scroll failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 6. desktop_drag ──────────────────────────────────────────────

export const desktop_drag = defineDesktopTool({
  name: 'desktop_drag',
  description:
    'Drag from one screen position to another. Useful for moving files, resizing windows, etc.',
  input: z.object({
    fromX: z.number().describe('Start X coordinate.'),
    fromY: z.number().describe('Start Y coordinate.'),
    toX: z.number().describe('End X coordinate.'),
    toY: z.number().describe('End Y coordinate.'),
    duration: z
      .number()
      .optional()
      .default(300)
      .describe('Drag duration in ms.'),
    steps: z
      .number()
      .optional()
      .default(20)
      .describe('Number of interpolation steps.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      await svc.mouse.mouseDrag(
        { x: args.fromX, y: args.fromY },
        { x: args.toX, y: args.toY },
        { duration: args.duration, steps: args.steps },
      )
      response.text(
        `Dragged from (${args.fromX}, ${args.fromY}) to (${args.toX}, ${args.toY}).`,
      )
      response.data({
        fromX: args.fromX,
        fromY: args.fromY,
        toX: args.toX,
        toY: args.toY,
      })
    } catch (error) {
      response.error(
        `Drag failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 7. desktop_open_file ─────────────────────────────────────────

export const desktop_open_file = defineDesktopTool({
  name: 'desktop_open_file',
  description:
    'Open a file with the default application. Accepts an absolute path or ~-prefixed path.',
  input: z.object({
    path: z.string().describe('Absolute path to the file to open.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.explorer.openFile(args.path)
      if (result.success) {
        response.text(`Opened file: ${result.path}`)
        response.data({ path: result.path })
      } else {
        response.error(`Failed to open file: ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Open file failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 8. desktop_open_folder ───────────────────────────────────────

export const desktop_open_folder = defineDesktopTool({
  name: 'desktop_open_folder',
  description:
    'Open a folder in the native file explorer (Finder, Explorer, Nautilus, etc.).',
  input: z.object({
    path: z.string().describe('Path to the folder.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.explorer.openFolder(args.path)
      if (result.success) {
        response.text(`Opened folder: ${result.path}`)
        response.data({ path: result.path })
      } else {
        response.error(`Failed to open folder: ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Open folder failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 9. desktop_open_app ──────────────────────────────────────────

export const desktop_open_app = defineDesktopTool({
  name: 'desktop_open_app',
  description:
    'Launch a desktop application by name. Supports common aliases: ' +
    'chrome, firefox, vscode, terminal, finder, spotify, slack, discord, steam.',
  input: z.object({
    appName: z.string().describe('Application name or alias.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.appLauncher.launch(args.appName)
      if (result.success) {
        response.text(`Launched application: ${result.appName}`)
        response.data({ appName: result.appName })
      } else {
        response.error(`Failed to launch "${args.appName}": ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Launch failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 10. desktop_close_app ────────────────────────────────────────

export const desktop_close_app = defineDesktopTool({
  name: 'desktop_close_app',
  description:
    'Quit/kill a running application by name. Sends SIGTERM first, then SIGKILL after a timeout.',
  input: z.object({
    appName: z.string().describe('Application name or alias.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.appLauncher.quit(args.appName)
      if (result.success) {
        response.text(
          `Quit ${args.appName} (${result.terminatedCount} process(es) terminated).`,
        )
        response.data({
          appName: args.appName,
          terminatedCount: result.terminatedCount,
        })
      } else {
        response.error(`Failed to quit "${args.appName}": ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Quit failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 11. desktop_list_apps ────────────────────────────────────────

export const desktop_list_apps = defineDesktopTool({
  name: 'desktop_list_apps',
  description:
    'List currently running desktop applications with PID, CPU, and memory usage.',
  input: z.object({
    filter: z
      .string()
      .optional()
      .describe('Optional name filter (case-insensitive substring match).'),
    limit: z
      .number()
      .optional()
      .default(30)
      .describe('Maximum number of results.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      let apps = await svc.appLauncher.listRunning()

      if (args.filter) {
        const lower = args.filter.toLowerCase()
        apps = apps.filter((a) => a.name.toLowerCase().includes(lower))
      }

      apps = apps.slice(0, args.limit)

      if (apps.length === 0) {
        response.text('No running applications found.')
        response.data({ apps: [], total: 0 })
        return
      }

      const lines = apps.map(
        (a) =>
          `  ${a.name} (PID ${a.pid}) — CPU: ${a.cpuPercent.toFixed(1)}%, MEM: ${formatBytes(a.memoryBytes)}`,
      )
      response.text(`Running applications:\n${lines.join('\n')}`)
      response.data({
        apps: apps.map((a) => ({
          name: a.name,
          pid: a.pid,
          cpuPercent: a.cpuPercent,
          memoryBytes: a.memoryBytes,
        })),
        total: apps.length,
      })
    } catch (error) {
      response.error(
        `List apps failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 12. desktop_list_files ───────────────────────────────────────

export const desktop_list_files = defineDesktopTool({
  name: 'desktop_list_files',
  description:
    'List files and directories in a folder. Accepts an absolute path or a quick-dir name ' +
    '(home, desktop, documents, downloads, pictures, music, videos).',
  input: z.object({
    path: z
      .string()
      .optional()
      .default('home')
      .describe('Directory path or quick-dir name.'),
    includeHidden: z
      .boolean()
      .optional()
      .default(false)
      .describe('Include hidden (dot) files.'),
    limit: z
      .number()
      .optional()
      .default(100)
      .describe('Maximum number of results.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const files = await svc.explorer.listFiles(args.path ?? 'home', {
        includeHidden: args.includeHidden,
        maxResults: args.limit,
      })

      if (files.length === 0) {
        response.text(`No files found in "${args.path}".`)
        response.data({ files: [], total: 0 })
        return
      }

      const lines = files.map((f) => {
        const icon = f.isDirectory ? '📁' : '📄'
        const size = f.isDirectory ? '' : ` (${formatBytes(f.size)})`
        return `  ${icon} ${f.name}${size}`
      })
      response.text(
        `Files in "${args.path}" (${files.length}):\n${lines.join('\n')}`,
      )
      response.data({
        files: files.map((f) => ({
          name: f.name,
          path: f.path,
          isDirectory: f.isDirectory,
          size: f.size,
        })),
        total: files.length,
      })
    } catch (error) {
      response.error(
        `List files failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 13. desktop_search_files ─────────────────────────────────────

export const desktop_search_files = defineDesktopTool({
  name: 'desktop_search_files',
  description:
    'Search for files by name pattern in a directory. Supports * wildcard. ' +
    'Set recursive to true for depth-limited recursive search.',
  input: z.object({
    path: z.string().default('home').describe('Directory to search in.'),
    pattern: z
      .string()
      .describe('File name pattern (e.g. "*.pdf", "report*").'),
    recursive: z
      .boolean()
      .optional()
      .default(false)
      .describe('Search subdirectories (max depth 3).'),
    limit: z.number().optional().default(50).describe('Max results.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = args.recursive
        ? await svc.explorer.searchRecursive(args.path, args.pattern, {
            maxResults: args.limit,
            maxDepth: 3,
          })
        : await svc.explorer.searchFiles(args.path, args.pattern, {
            maxResults: args.limit,
          })

      if (result.matches.length === 0) {
        response.text(
          `No files matching "${args.pattern}" found in "${args.path}".`,
        )
        response.data({ matches: [], total: 0, truncated: result.truncated })
        return
      }

      const lines = result.matches.map((f) => `  ${f.name} — ${f.path}`)
      const suffix = result.truncated ? '\n(Results truncated.)' : ''
      response.text(
        `Found ${result.matches.length} match(es) for "${args.pattern}":\n${lines.join('\n')}${suffix}`,
      )
      response.data({
        matches: result.matches.map((f) => ({
          name: f.name,
          path: f.path,
          isDirectory: f.isDirectory,
          size: f.size,
        })),
        total: result.matches.length,
        truncated: result.truncated,
      })
    } catch (error) {
      response.error(
        `Search failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 14. desktop_copy_file ────────────────────────────────────────

export const desktop_copy_file = defineDesktopTool({
  name: 'desktop_copy_file',
  description: 'Copy a file or directory to a new location.',
  input: z.object({
    source: z.string().describe('Source path.'),
    destination: z.string().describe('Destination path.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.osCommands.copyFile(
        args.source,
        args.destination,
      )
      if (result.success) {
        response.text(`Copied: ${args.source} → ${args.destination}`)
        response.data({ source: args.source, destination: args.destination })
      } else {
        response.error(`Copy failed: ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Copy failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 15. desktop_move_file ────────────────────────────────────────

export const desktop_move_file = defineDesktopTool({
  name: 'desktop_move_file',
  description: 'Move or rename a file or directory.',
  input: z.object({
    source: z.string().describe('Source path.'),
    destination: z.string().describe('Destination path.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.osCommands.moveFile(
        args.source,
        args.destination,
      )
      if (result.success) {
        response.text(`Moved: ${args.source} → ${args.destination}`)
        response.data({ source: args.source, destination: args.destination })
      } else {
        response.error(`Move failed: ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Move failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 16. desktop_delete_file ──────────────────────────────────────

export const desktop_delete_file = defineDesktopTool({
  name: 'desktop_delete_file',
  description:
    'Delete a file or directory. Directories are deleted recursively. ' +
    'IMPORTANT: This is a permanent delete — prefer moving to trash when possible.',
  input: z.object({
    path: z.string().describe('Path to delete.'),
    useTrash: z
      .boolean()
      .optional()
      .default(true)
      .describe('Move to trash instead of permanent delete (when supported).'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()

    // If useTrash is requested, attempt `trash-put` / `trash` command
    if (args.useTrash) {
      try {
        const { execFile } = await import('node:child_process')
        const resolved = await svc.fileManager.resolvePath(args.path)

        // Try trash-put (Linux), trash (macOS via homebrew), or moveToTrash
        const trashCommands = ['trash-put', 'trash']
        let trashed = false

        for (const cmd of trashCommands) {
          try {
            await new Promise<void>((resolve, reject) => {
              execFile(cmd, [resolved], (error) => {
                if (error) reject(error)
                else resolve()
              })
            })
            trashed = true
            break
          } catch {}
        }

        if (trashed) {
          response.text(`Moved to trash: ${resolved}`)
          response.data({ path: resolved, method: 'trash' })
          return
        }
      } catch {
        // Fall through to permanent delete
      }
    }

    try {
      const result = await svc.explorer.delete(args.path)
      if (result.success) {
        response.text(`Deleted: ${args.path}`)
        response.data({ path: args.path, method: 'permanent' })
      } else {
        response.error(`Delete failed: ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Delete failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 17. desktop_clipboard_read ───────────────────────────────────

export const desktop_clipboard_read = defineDesktopTool({
  name: 'desktop_clipboard_read',
  description: 'Read the current text content of the system clipboard.',
  input: z.object({}),
  handler: async (_args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.osCommands.clipboardRead()
      if (result.success) {
        response.text(`Clipboard content:\n${result.text ?? '(empty)'}`)
        response.data({ text: result.text ?? '' })
      } else {
        response.error(`Clipboard read failed: ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Clipboard read failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 18. desktop_clipboard_write ──────────────────────────────────

export const desktop_clipboard_write = defineDesktopTool({
  name: 'desktop_clipboard_write',
  description: 'Write text to the system clipboard.',
  input: z.object({
    text: z.string().describe('Text to copy to clipboard.'),
  }),
  handler: async (args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const result = await svc.osCommands.clipboardWrite(args.text)
      if (result.success) {
        response.text(`Copied ${args.text.length} characters to clipboard.`)
        response.data({ textLength: args.text.length })
      } else {
        response.error(`Clipboard write failed: ${result.error}`)
      }
    } catch (error) {
      response.error(
        `Clipboard write failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 19. desktop_system_info ──────────────────────────────────────

export const desktop_system_info = defineDesktopTool({
  name: 'desktop_system_info',
  description:
    'Get system information: OS, CPU, memory, disk space, hostname, uptime.',
  input: z.object({}),
  handler: async (_args, _ctx, response) => {
    const svc = getDesktopServices()
    try {
      const info = await svc.osCommands.getSystemInfo()
      const memUsedPct = (
        ((info.totalMemoryBytes - info.freeMemoryBytes) /
          info.totalMemoryBytes) *
        100
      ).toFixed(1)
      const diskUsedPct =
        info.disk.totalBytes > 0
          ? ((info.disk.usedBytes / info.disk.totalBytes) * 100).toFixed(1)
          : '?'

      response.text(
        `System Info:\n` +
          `  Platform: ${info.platform} (${info.arch})\n` +
          `  OS: ${info.release}\n` +
          `  Hostname: ${info.hostname}\n` +
          `  CPU: ${info.cpuModel} (${info.cpuCores} cores)\n` +
          `  Memory: ${formatBytes(info.freeMemoryBytes)} free / ${formatBytes(info.totalMemoryBytes)} total (${memUsedPct}% used)\n` +
          `  Disk: ${formatBytes(info.disk.availableBytes)} free / ${formatBytes(info.disk.totalBytes)} total (${diskUsedPct}% used)\n` +
          `  Uptime: ${formatDuration(info.uptimeSeconds)}`,
      )
      response.data({
        platform: info.platform,
        arch: info.arch,
        release: info.release,
        hostname: info.hostname,
        cpuModel: info.cpuModel,
        cpuCores: info.cpuCores,
        totalMemoryBytes: info.totalMemoryBytes,
        freeMemoryBytes: info.freeMemoryBytes,
        diskTotalBytes: info.disk.totalBytes,
        diskAvailableBytes: info.disk.availableBytes,
        uptimeSeconds: info.uptimeSeconds,
      })
    } catch (error) {
      response.error(
        `System info failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 20. desktop_file_upload ──────────────────────────────────────

export const desktop_file_upload = defineDesktopTool({
  name: 'desktop_file_upload',
  description:
    'Upload a file to a web page. This is a full end-to-end flow: ' +
    'clicks the browser upload button, detects the native file dialog, ' +
    'types the file path, and confirms. Requires a browser context.',
  input: z.object({
    page: z.number().describe('Page ID of the browser tab.'),
    uploadElementId: z
      .number()
      .describe('Element ID of the upload button/input from snapshot.'),
    filePath: z.string().describe('Absolute path to the file to upload.'),
  }),
  handler: async (args, ctx, response) => {
    const svc = getDesktopServices()

    if (!ctx.browser) {
      response.error('desktop_file_upload requires a browser context.')
      return
    }

    try {
      // Validate file exists
      const resolvedPath = await svc.fileManager.resolvePath(args.filePath)
      const exists = await svc.fileManager.fileExists(resolvedPath)
      if (!exists) {
        response.error(`File not found: ${resolvedPath}`)
        return
      }

      // Click the upload element in the browser to trigger the file dialog
      try {
        await ctx.browser.click(args.page, args.uploadElementId)
      } catch (clickErr) {
        response.error(
          `Failed to click upload element: ${clickErr instanceof Error ? clickErr.message : String(clickErr)}`,
        )
        return
      }

      // Wait for native dialog to appear
      await sleep(600)

      // Type the file path into the dialog
      await svc.keyboard.pressKey({ key: 'a', modifiers: ['control'] })
      await sleep(50)
      await svc.keyboard.typeText(resolvedPath, { keyDelay: 5 })
      await sleep(100)
      await svc.keyboard.pressKey({ key: 'enter' })

      // Wait for dialog to close and browser to process
      await sleep(500)

      response.text(`File upload initiated: ${resolvedPath}`)
      response.data({
        filePath: resolvedPath,
        page: args.page,
        elementId: args.uploadElementId,
      })
    } catch (error) {
      response.error(
        `File upload failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── 21. desktop_file_download ────────────────────────────────────

export const desktop_file_download = defineDesktopTool({
  name: 'desktop_file_download',
  description:
    'Download a file from a web page. Clicks the download button in the browser, ' +
    'and if a save dialog appears, types the save path and confirms. ' +
    'If no dialog appears, the file downloads to the default location.',
  input: z.object({
    page: z.number().describe('Page ID of the browser tab.'),
    downloadElementId: z
      .number()
      .describe('Element ID of the download button from snapshot.'),
    savePath: z
      .string()
      .optional()
      .describe(
        'Desired save path. If omitted, uses default download location.',
      ),
  }),
  handler: async (args, ctx, response) => {
    const svc = getDesktopServices()

    if (!ctx.browser) {
      response.error('desktop_file_download requires a browser context.')
      return
    }

    try {
      // Click the download element
      try {
        await ctx.browser.click(args.page, args.downloadElementId)
      } catch (clickErr) {
        response.error(
          `Failed to click download element: ${clickErr instanceof Error ? clickErr.message : String(clickErr)}`,
        )
        return
      }

      // Wait briefly to see if a save dialog appears
      await sleep(600)

      if (args.savePath) {
        const resolvedPath = await svc.fileManager.resolvePath(args.savePath)

        // Attempt to type into save dialog
        await svc.keyboard.pressKey({ key: 'a', modifiers: ['control'] })
        await sleep(50)
        await svc.keyboard.typeText(resolvedPath, { keyDelay: 5 })
        await sleep(100)
        await svc.keyboard.pressKey({ key: 'enter' })

        await sleep(500)
        response.text(`Download initiated, saving to: ${resolvedPath}`)
        response.data({ savePath: resolvedPath, page: args.page })
      } else {
        response.text('Download initiated (default save location).')
        response.data({ page: args.page, savePath: null })
      }
    } catch (error) {
      response.error(
        `File download failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  },
})

// ─── Helpers ───────────────────────────────────────────────────────

/** Format bytes to a human-readable string. */
function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  return `${(bytes / 1024 ** i).toFixed(1)} ${units[i]}`
}

/** Format seconds into a human-readable duration. */
function formatDuration(seconds: number): string {
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const mins = Math.floor((seconds % 3600) / 60)
  const parts: string[] = []
  if (days > 0) parts.push(`${days}d`)
  if (hours > 0) parts.push(`${hours}h`)
  if (mins > 0) parts.push(`${mins}m`)
  return parts.join(' ') || '< 1m'
}

/** Promise-based sleep. */
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

// ─── Aggregate export ──────────────────────────────────────────────

/**
 * All desktop tool definitions as an array.
 * Import this to register them in a ToolRegistry.
 */
export const desktopTools: ToolDefinition[] = [
  // Observation (1)
  desktop_screenshot,

  // Input (5)
  desktop_click,
  desktop_type,
  desktop_hotkey,
  desktop_scroll,
  desktop_drag,

  // Application Control (3)
  desktop_open_app,
  desktop_close_app,
  desktop_list_apps,

  // File Management (7)
  desktop_open_file,
  desktop_open_folder,
  desktop_list_files,
  desktop_search_files,
  desktop_copy_file,
  desktop_move_file,
  desktop_delete_file,

  // Clipboard (2)
  desktop_clipboard_read,
  desktop_clipboard_write,

  // System (1)
  desktop_system_info,

  // Browser ↔ Desktop integration (2)
  desktop_file_upload,
  desktop_file_download,
]
