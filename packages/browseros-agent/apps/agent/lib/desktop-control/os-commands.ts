/**
 * OSCommands — common OS operations for desktop control.
 *
 * Provides system-level utilities: open URLs, copy/move files,
 * extract archives, system info, screenshots, clipboard, and
 * volume/brightness control. Reuses DesktopScreenshotService for
 * screenshots and uses Node.js built-ins everywhere else.
 *
 * @module desktop-control/os-commands
 */

import { DesktopScreenshotService } from './screenshot'
import type { ScreenshotResult } from './types'

// ─── Types ─────────────────────────────────────────────────────────

/** Result of opening a URL. */
export interface OpenUrlResult {
  success: boolean
  url: string
  error?: string
}

/** Result of a file copy or move operation. */
export interface FileTransferResult {
  success: boolean
  source: string
  destination: string
  error?: string
}

/** Result of archive extraction. */
export interface ExtractResult {
  success: boolean
  archivePath: string
  targetDir: string
  error?: string
}

/** System information snapshot. */
export interface SystemInfo {
  /** OS platform (darwin, linux, win32). */
  platform: string
  /** OS release/version string. */
  release: string
  /** System architecture. */
  arch: string
  /** Hostname. */
  hostname: string
  /** CPU model name. */
  cpuModel: string
  /** Number of CPU cores. */
  cpuCores: number
  /** Total system memory in bytes. */
  totalMemoryBytes: number
  /** Free memory in bytes. */
  freeMemoryBytes: number
  /** System uptime in seconds. */
  uptimeSeconds: number
  /** Disk space info for the root/home volume. */
  disk: DiskInfo
}

/** Disk space information. */
export interface DiskInfo {
  /** Total disk space in bytes. */
  totalBytes: number
  /** Used disk space in bytes. */
  usedBytes: number
  /** Available disk space in bytes. */
  availableBytes: number
}

/** Result of clipboard operation. */
export interface ClipboardResult {
  success: boolean
  /** For read: the clipboard text content. */
  text?: string
  error?: string
}

/** Result of volume or brightness control. */
export interface ControlResult {
  success: boolean
  /** Current level (0-100). */
  level?: number
  error?: string
}

// ─── Platform detection ────────────────────────────────────────────

type Platform = 'darwin' | 'linux' | 'win32'

function getPlatform(): Platform {
  return process.platform as Platform
}

/** Execute a command and return { stdout, stderr, exitCode }. */
async function execCommand(
  command: string,
  args: string[],
  timeout = 15000,
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  const { execFile } = await import('node:child_process')
  return new Promise((resolve) => {
    const child = execFile(
      command,
      args,
      { timeout },
      (error, stdout, stderr) => {
        resolve({
          stdout: stdout ?? '',
          stderr: stderr ?? '',
          exitCode: error ? ((error as any).status ?? 1) : 0,
        })
      },
    )
    child.on('error', () => {
      resolve({ stdout: '', stderr: 'Command not found', exitCode: 127 })
    })
  })
}

// ─── OSCommands ────────────────────────────────────────────────────

/**
 * OSCommands provides common operating system operations
 * for the desktop control layer.
 *
 * @public
 */
export class OSCommands {
  private screenshotService: DesktopScreenshotService

  constructor() {
    this.screenshotService = new DesktopScreenshotService()
  }

  // ─── Open URL ────────────────────────────────────────────────────

  /**
   * Open a URL in the default browser.
   */
  async openUrl(url: string): Promise<OpenUrlResult> {
    // Ensure URL has a scheme
    const fullUrl = /^https?:\/\//i.test(url) ? url : `https://${url}`

    try {
      const platform = getPlatform()
      const { execFile } = await import('node:child_process')

      await new Promise<void>((resolve, reject) => {
        let cmd: string
        let args: string[]

        if (platform === 'darwin') {
          cmd = 'open'
          args = [fullUrl]
        } else if (platform === 'win32') {
          cmd = 'cmd'
          args = ['/c', 'start', '""', fullUrl]
        } else {
          cmd = 'xdg-open'
          args = [fullUrl]
        }

        execFile(cmd, args, (error) => {
          if (error) reject(error)
          else resolve()
        })
      })

      return { success: true, url: fullUrl }
    } catch (error) {
      return {
        success: false,
        url: fullUrl,
        error: `Failed to open URL: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  // ─── File Copy / Move ────────────────────────────────────────────

  /**
   * Copy a file or directory.
   */
  async copyFile(
    source: string,
    destination: string,
  ): Promise<FileTransferResult> {
    try {
      const fs = await import('node:fs/promises')
      const path = await import('node:path')

      // Ensure destination directory exists
      const destDir = path.dirname(destination)
      await fs.mkdir(destDir, { recursive: true })

      const srcStat = await fs.stat(source)

      if (srcStat.isDirectory()) {
        await this.copyDirRecursive(source, destination)
      } else {
        await fs.copyFile(source, destination)
      }

      return { success: true, source, destination }
    } catch (error) {
      return {
        success: false,
        source,
        destination,
        error: `Failed to copy: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  /**
   * Move a file or directory.
   */
  async moveFile(
    source: string,
    destination: string,
  ): Promise<FileTransferResult> {
    try {
      const fs = await import('node:fs/promises')
      const path = await import('node:path')

      const destDir = path.dirname(destination)
      await fs.mkdir(destDir, { recursive: true })

      await fs.rename(source, destination)

      return { success: true, source, destination }
    } catch (_error) {
      // Fall back to copy + delete if rename fails (cross-device)
      try {
        const copyResult = await this.copyFile(source, destination)
        if (!copyResult.success) return copyResult

        const fs = await import('node:fs/promises')
        await fs.rm(source, { recursive: true, force: true })

        return { success: true, source, destination }
      } catch (fallbackError) {
        return {
          success: false,
          source,
          destination,
          error: `Failed to move: ${fallbackError instanceof Error ? fallbackError.message : String(fallbackError)}`,
        }
      }
    }
  }

  // ─── Archive Extraction ──────────────────────────────────────────

  /**
   * Extract an archive (zip, tar.gz, tar.bz2, tar.xz).
   * The target directory defaults to the archive's parent directory.
   */
  async extract(
    archivePath: string,
    targetDir?: string,
  ): Promise<ExtractResult> {
    try {
      const fs = await import('node:fs/promises')
      const path = await import('node:path')

      // Verify archive exists
      try {
        await fs.access(archivePath)
      } catch {
        return {
          success: false,
          archivePath,
          targetDir: targetDir ?? '',
          error: `Archive not found: ${archivePath}`,
        }
      }

      const resolvedTarget = targetDir ?? path.dirname(archivePath)
      await fs.mkdir(resolvedTarget, { recursive: true })

      const ext = archivePath.toLowerCase()

      if (ext.endsWith('.zip')) {
        return await this.extractZip(archivePath, resolvedTarget)
      } else if (
        ext.endsWith('.tar.gz') ||
        ext.endsWith('.tgz') ||
        ext.endsWith('.tar.bz2') ||
        ext.endsWith('.tbz2') ||
        ext.endsWith('.tar.xz') ||
        ext.endsWith('.txz') ||
        ext.endsWith('.tar')
      ) {
        return await this.extractTar(archivePath, resolvedTarget)
      } else {
        return {
          success: false,
          archivePath,
          targetDir: resolvedTarget,
          error: `Unsupported archive format: ${path.extname(archivePath)}`,
        }
      }
    } catch (error) {
      return {
        success: false,
        archivePath,
        targetDir: targetDir ?? '',
        error: `Extraction failed: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  // ─── System Info ─────────────────────────────────────────────────

  /**
   * Get current system information.
   */
  async getSystemInfo(): Promise<SystemInfo> {
    const os = await import('node:os')

    const diskInfo = await this.getDiskInfo()

    return {
      platform: process.platform,
      release: os.release(),
      arch: process.arch,
      hostname: os.hostname(),
      cpuModel: os.cpus()[0]?.model ?? 'unknown',
      cpuCores: os.cpus().length,
      totalMemoryBytes: os.totalmem(),
      freeMemoryBytes: os.freemem(),
      uptimeSeconds: os.uptime(),
      disk: diskInfo,
    }
  }

  // ─── Screenshot (reuses DesktopScreenshotService) ────────────────

  /**
   * Take a desktop screenshot.
   * Delegates to DesktopScreenshotService.
   */
  async takeScreenshot(): Promise<ScreenshotResult> {
    return this.screenshotService.captureScreenshot()
  }

  // ─── Clipboard ───────────────────────────────────────────────────

  /**
   * Read text from the system clipboard.
   * Uses platform-specific commands (pbcopy/pbpaste, xclip, clip).
   */
  async clipboardRead(): Promise<ClipboardResult> {
    const platform = getPlatform()

    try {
      if (platform === 'darwin') {
        const result = await execCommand('pbpaste', [])
        if (result.exitCode !== 0) {
          return { success: false, error: 'pbpaste failed' }
        }
        return { success: true, text: result.stdout }
      } else if (platform === 'linux') {
        // Try xclip first, then xsel
        let result = await execCommand('xclip', [
          '-selection',
          'clipboard',
          '-o',
        ])
        if (result.exitCode !== 0) {
          result = await execCommand('xsel', ['--clipboard', '--output'])
        }
        if (result.exitCode !== 0) {
          return { success: false, error: 'Neither xclip nor xsel available' }
        }
        return { success: true, text: result.stdout }
      } else {
        // Windows
        const result = await execCommand('powershell', [
          '-Command',
          'Get-Clipboard',
        ])
        if (result.exitCode !== 0) {
          return { success: false, error: 'PowerShell Get-Clipboard failed' }
        }
        return { success: true, text: result.stdout }
      }
    } catch (error) {
      return {
        success: false,
        error: `Clipboard read failed: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  /**
   * Write text to the system clipboard.
   */
  async clipboardWrite(text: string): Promise<ClipboardResult> {
    const platform = getPlatform()

    try {
      if (platform === 'darwin') {
        const { execFile } = await import('node:child_process')
        await new Promise<void>((resolve, reject) => {
          const child = execFile('pbcopy', [], (error) => {
            if (error) reject(error)
            else resolve()
          })
          child.stdin?.write(text)
          child.stdin?.end()
        })
        return { success: true }
      } else if (platform === 'linux') {
        // Try xclip first, then xsel
        const { execFile } = await import('node:child_process')
        const tryXclip = (): Promise<void> =>
          new Promise((resolve, reject) => {
            const child = execFile(
              'xclip',
              ['-selection', 'clipboard'],
              (error) => {
                if (error) reject(error)
                else resolve()
              },
            )
            child.stdin?.write(text)
            child.stdin?.end()
          })
        const tryXsel = (): Promise<void> =>
          new Promise((resolve, reject) => {
            const child = execFile(
              'xsel',
              ['--clipboard', '--input'],
              (error) => {
                if (error) reject(error)
                else resolve()
              },
            )
            child.stdin?.write(text)
            child.stdin?.end()
          })

        try {
          await tryXclip()
        } catch {
          await tryXsel()
        }
        return { success: true }
      } else {
        // Windows
        const { exec } = await import('node:child_process')
        await new Promise<void>((resolve, reject) => {
          const escaped = text.replace(/'/g, "''")
          exec(
            `powershell -Command "Set-Clipboard -Value '${escaped}'"`,
            (error) => {
              if (error) reject(error)
              else resolve()
            },
          )
        })
        return { success: true }
      }
    } catch (error) {
      return {
        success: false,
        error: `Clipboard write failed: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  // ─── Volume Control ──────────────────────────────────────────────

  /**
   * Get or set system volume (0-100).
   * Best-effort; may not work on all platforms/configurations.
   */
  async volume(level?: number): Promise<ControlResult> {
    const platform = getPlatform()

    try {
      if (platform === 'linux') {
        if (level !== undefined) {
          const pct = Math.max(0, Math.min(100, level))
          const result = await execCommand('amixer', [
            'set',
            'Master',
            `${pct}%`,
          ])
          if (result.exitCode !== 0) {
            // Try pactl
            const vol = Math.round((pct / 100) * 65535)
            const presult = await execCommand('pactl', [
              'set-sink-volume',
              '@DEFAULT_SINK@',
              `${vol}`,
            ])
            if (presult.exitCode !== 0) {
              return {
                success: false,
                error: 'Neither amixer nor pactl available',
              }
            }
          }
          return { success: true, level }
        } else {
          // Get current volume
          const result = await execCommand('amixer', ['get', 'Master'])
          if (result.exitCode === 0) {
            const match = result.stdout.match(/\[(\d+)%\]/)
            if (match) {
              return { success: true, level: parseInt(match[1], 10) }
            }
          }
          return { success: false, error: 'Could not read volume' }
        }
      } else if (platform === 'darwin') {
        if (level !== undefined) {
          const pct = Math.max(0, Math.min(100, level))
          await execCommand('osascript', [
            '-e',
            `set volume output volume ${pct}`,
          ])
          return { success: true, level: pct }
        } else {
          const result = await execCommand('osascript', [
            '-e',
            'output volume of (get volume settings)',
          ])
          if (result.exitCode === 0) {
            const vol = parseInt(result.stdout.trim(), 10)
            return { success: true, level: vol }
          }
          return { success: false, error: 'Could not read volume' }
        }
      } else {
        return {
          success: false,
          error: 'Volume control not implemented for Windows',
        }
      }
    } catch (error) {
      return {
        success: false,
        error: `Volume control failed: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  // ─── Brightness Control ──────────────────────────────────────────

  /**
   * Get or set screen brightness (0-100).
   * Best-effort; highly platform-dependent.
   */
  async brightness(level?: number): Promise<ControlResult> {
    const platform = getPlatform()

    try {
      if (platform === 'linux') {
        // Try brightnessctl
        if (level !== undefined) {
          const pct = Math.max(0, Math.min(100, level))
          const result = await execCommand('brightnessctl', ['set', `${pct}%`])
          if (result.exitCode !== 0) {
            return { success: false, error: 'brightnessctl not available' }
          }
          return { success: true, level: pct }
        } else {
          const result = await execCommand('brightnessctl', ['info'])
          if (result.exitCode === 0) {
            const match = result.stdout.match(/\((\d+)%\)/)
            if (match) {
              return { success: true, level: parseInt(match[1], 10) }
            }
          }
          return { success: false, error: 'Could not read brightness' }
        }
      } else if (platform === 'darwin') {
        if (level !== undefined) {
          const pct = Math.max(0, Math.min(100, level))
          await execCommand('osascript', [
            '-e',
            `tell application "System Events" to tell appearance preferences to set brightness to ${pct / 100}`,
          ])
          return { success: true, level: pct }
        }
        return {
          success: false,
          error: 'Brightness read not supported on macOS via osascript',
        }
      } else {
        return {
          success: false,
          error: 'Brightness control not implemented for Windows',
        }
      }
    } catch (error) {
      return {
        success: false,
        error: `Brightness control failed: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  // ─── Private Helpers ─────────────────────────────────────────────

  /** Recursively copy a directory. */
  private async copyDirRecursive(
    source: string,
    destination: string,
  ): Promise<void> {
    const fs = await import('node:fs/promises')
    const path = await import('node:path')

    await fs.mkdir(destination, { recursive: true })
    const entries = await fs.readdir(source, { withFileTypes: true })

    for (const entry of entries) {
      const srcPath = path.join(source, entry.name)
      const destPath = path.join(destination, entry.name)

      if (entry.isDirectory()) {
        await this.copyDirRecursive(srcPath, destPath)
      } else {
        await fs.copyFile(srcPath, destPath)
      }
    }
  }

  /** Extract a ZIP archive using the `unzip` command. */
  private async extractZip(
    archivePath: string,
    targetDir: string,
  ): Promise<ExtractResult> {
    const result = await execCommand('unzip', [
      '-o',
      '-q',
      archivePath,
      '-d',
      targetDir,
    ])

    if (result.exitCode !== 0) {
      // Try python3 as fallback
      const pyResult = await execCommand('python3', [
        '-c',
        `import zipfile; zipfile.ZipFile('${archivePath}').extractall('${targetDir}')`,
      ])

      if (pyResult.exitCode !== 0) {
        return {
          success: false,
          archivePath,
          targetDir,
          error: `unzip failed: ${result.stderr}\npython3 fallback failed: ${pyResult.stderr}`,
        }
      }
    }

    return { success: true, archivePath, targetDir }
  }

  /** Extract a TAR archive. */
  private async extractTar(
    archivePath: string,
    targetDir: string,
  ): Promise<ExtractResult> {
    const result = await execCommand('tar', [
      'xf',
      archivePath,
      '-C',
      targetDir,
    ])

    if (result.exitCode !== 0) {
      return {
        success: false,
        archivePath,
        targetDir,
        error: `tar extraction failed: ${result.stderr}`,
      }
    }

    return { success: true, archivePath, targetDir }
  }

  /** Get disk space info for the root/home volume. */
  private async getDiskInfo(): Promise<DiskInfo> {
    try {
      const os = await import('node:os')
      const platform = getPlatform()

      if (platform === 'win32') {
        // Use Node.js built-in approach for Windows
        const result = await execCommand(
          'wmic',
          ['logicaldisk', 'get', 'size,freespace', '/format:csv'],
          10000,
        )
        if (result.exitCode === 0) {
          const lines = result.stdout.split('\n').filter((l) => l.trim())
          for (const line of lines.slice(1)) {
            const parts = line.trim().split(',')
            if (parts.length >= 3) {
              const free = parseInt(parts[1], 10) || 0
              const total = parseInt(parts[2], 10) || 0
              return {
                totalBytes: total,
                usedBytes: total - free,
                availableBytes: free,
              }
            }
          }
        }
      } else {
        // macOS / Linux: use df
        const home = os.homedir()
        const result = await execCommand('df', ['-k', home], 5000)
        if (result.exitCode === 0) {
          const lines = result.stdout.split('\n')
          if (lines.length >= 2) {
            const parts = lines[1].trim().split(/\s+/)
            if (parts.length >= 4) {
              const totalKB = parseInt(parts[1], 10) || 0
              const usedKB = parseInt(parts[2], 10) || 0
              const availKB = parseInt(parts[3], 10) || 0
              return {
                totalBytes: totalKB * 1024,
                usedBytes: usedKB * 1024,
                availableBytes: availKB * 1024,
              }
            }
          }
        }
      }
    } catch {
      // Fall through to default
    }

    return { totalBytes: 0, usedBytes: 0, availableBytes: 0 }
  }
}
