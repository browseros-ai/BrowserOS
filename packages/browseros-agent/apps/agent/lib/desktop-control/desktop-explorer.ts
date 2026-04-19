/**
 * DesktopExplorer — browse desktop like a file manager.
 *
 * Provides high-level file system operations for navigating the user's
 * desktop, Documents, Downloads, Pictures etc. Supports opening files
 * with default applications, opening folders in the native file explorer,
 * getting file info, searching, and creating/deleting/renaming files.
 *
 * Uses FileManager for low-level path operations and Node.js child_process
 * for launching external applications.
 *
 * @module desktop-control/desktop-explorer
 */

import type {
  FileInfo,
  FileSearchResult,
  ListFilesOptions,
} from './file-manager'
import { FileManager } from './file-manager'
import type { DesktopControlService } from './types'

// ─── Explorer-specific types ───────────────────────────────────────

/** Detailed file information including MIME type. */
export interface DetailedFileInfo extends FileInfo {
  /** File extension (e.g. 'txt', 'png'). Empty string for directories. */
  extension: string
  /** Detected MIME type (best-effort). */
  mimeType: string
  /** Whether the file is a symlink. */
  isSymlink: boolean
}

/** Result of opening a file or folder. */
export interface OpenResult {
  /** Whether the open command was issued successfully. */
  success: boolean
  /** The path that was opened. */
  path: string
  /** Error message if success is false. */
  error?: string
}

/** Result of a create/delete/rename operation. */
export interface FileOperationResult {
  /** Whether the operation succeeded. */
  success: boolean
  /** The affected path(s). */
  paths: string[]
  /** Error message if success is false. */
  error?: string
}

/** Quick-access directory identifiers. */
export type QuickDir =
  | 'home'
  | 'desktop'
  | 'documents'
  | 'downloads'
  | 'pictures'
  | 'music'
  | 'videos'

// ─── Platform detection ────────────────────────────────────────────

type Platform = 'darwin' | 'linux' | 'win32'

function getPlatform(): Platform {
  return process.platform as Platform
}

// ─── DesktopExplorer ───────────────────────────────────────────────

/**
 * DesktopExplorer provides a file-manager-like interface for navigating
 * and managing the user's file system from the desktop control layer.
 *
 * @public
 */
export class DesktopExplorer {
  private fileManager: FileManager

  constructor(service: DesktopControlService) {
    this.service = service
    this.fileManager = new FileManager(service)
  }

  // ─── Navigation ──────────────────────────────────────────────────

  /**
   * List files and folders in a directory.
   * Accepts a QuickDir name or an absolute path.
   */
  async listFiles(
    dirOrQuickDir: string,
    options?: ListFilesOptions,
  ): Promise<FileInfo[]> {
    const resolved = await this.resolveDir(dirOrQuickDir)
    return this.fileManager.listFiles(resolved, options)
  }

  /**
   * List files with extended info (MIME type, symlink detection).
   */
  async listDetailed(
    dirOrQuickDir: string,
    options?: ListFilesOptions,
  ): Promise<DetailedFileInfo[]> {
    const resolved = await this.resolveDir(dirOrQuickDir)
    const files = await this.fileManager.listFiles(resolved, options)
    const path = await import('node:path')

    return Promise.all(
      files.map(async (f): Promise<DetailedFileInfo> => {
        const ext = f.isDirectory ? '' : path.extname(f.name).replace(/^\./, '')
        return {
          ...f,
          extension: ext,
          mimeType: this.guessMimeType(f.name, f.isDirectory),
          isSymlink: await this.isSymlink(f.path),
        }
      }),
    )
  }

  /**
   * Get the list of quick-access directories.
   */
  async getQuickDirs(): Promise<Record<QuickDir, string>> {
    return this.fileManager.getCommonDirectories() as Promise<
      Record<QuickDir, string>
    >
  }

  /**
   * Get the user's home directory.
   */
  async getHome(): Promise<string> {
    return this.fileManager.getHomeDirectory()
  }

  // ─── File Info ───────────────────────────────────────────────────

  /**
   * Get detailed information about a single file or directory.
   * Returns null if the file does not exist.
   */
  async getFileInfo(filePath: string): Promise<DetailedFileInfo | null> {
    const resolved = await this.fileManager.resolvePath(filePath)
    const exists = await this.fileManager.fileExists(resolved)
    if (!exists) return null

    const fs = await import('node:fs/promises')
    const path = await import('node:path')
    const stat = await fs.stat(resolved)
    const name = path.basename(resolved)
    const ext = stat.isDirectory() ? '' : path.extname(name).replace(/^\./, '')

    return {
      path: resolved,
      name,
      isDirectory: stat.isDirectory(),
      size: stat.size,
      modifiedAt: stat.mtimeMs,
      extension: ext,
      mimeType: this.guessMimeType(name, stat.isDirectory()),
      isSymlink: await this.isSymlink(resolved),
    }
  }

  // ─── Search ──────────────────────────────────────────────────────

  /**
   * Search files by name pattern in a directory.
   * Supports * wildcard. Non-recursive.
   */
  async searchFiles(
    dirOrQuickDir: string,
    pattern: string,
    options?: ListFilesOptions,
  ): Promise<FileSearchResult> {
    const resolved = await this.resolveDir(dirOrQuickDir)
    return this.fileManager.searchFiles(resolved, pattern, options)
  }

  /**
   * Recursive file search (depth-limited).
   */
  async searchRecursive(
    dirOrQuickDir: string,
    pattern: string,
    options?: ListFilesOptions & { maxDepth?: number },
  ): Promise<FileSearchResult> {
    const resolved = await this.resolveDir(dirOrQuickDir)
    const maxDepth = options?.maxDepth ?? 3
    const maxResults = options?.maxResults ?? 1000
    const matches: FileInfo[] = []
    let truncated = false

    const searchDir = async (dir: string, depth: number): Promise<void> => {
      if (depth > maxDepth || truncated) return

      try {
        const files = await this.fileManager.listFiles(dir, {
          ...options,
          maxResults: 500,
        })

        for (const f of files) {
          if (matches.length >= maxResults) {
            truncated = true
            return
          }

          const regex = this.globToRegex(pattern)
          if (regex.test(f.name)) {
            matches.push(f)
          }

          if (f.isDirectory && !truncated) {
            await searchDir(f.path, depth + 1)
          }
        }
      } catch {
        // Skip directories we can't read
      }
    }

    await searchDir(resolved, 0)

    return { matches, truncated }
  }

  // ─── Open ────────────────────────────────────────────────────────

  /**
   * Open a file with the default application.
   */
  async openFile(filePath: string): Promise<OpenResult> {
    const resolved = await this.fileManager.resolvePath(filePath)
    const exists = await this.fileManager.fileExists(resolved)
    if (!exists) {
      return {
        success: false,
        path: resolved,
        error: `File not found: ${resolved}`,
      }
    }

    try {
      const { execFile } = await import('node:child_process')
      const platform = getPlatform()

      await new Promise<void>((resolve, reject) => {
        let cmd: string
        let args: string[]

        if (platform === 'darwin') {
          cmd = 'open'
          args = [resolved]
        } else if (platform === 'win32') {
          cmd = 'cmd'
          args = ['/c', 'start', '""', resolved]
        } else {
          cmd = 'xdg-open'
          args = [resolved]
        }

        execFile(cmd, args, (error) => {
          if (error) reject(error)
          else resolve()
        })
      })

      return { success: true, path: resolved }
    } catch (error) {
      return {
        success: false,
        path: resolved,
        error: `Failed to open: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  /**
   * Open a folder in the native file explorer.
   */
  async openFolder(dirPath: string): Promise<OpenResult> {
    const resolved = await this.fileManager.resolvePath(dirPath)
    const exists = await this.fileManager.fileExists(resolved)
    if (!exists) {
      return {
        success: false,
        path: resolved,
        error: `Directory not found: ${resolved}`,
      }
    }

    try {
      const { execFile } = await import('node:child_process')
      const platform = getPlatform()

      await new Promise<void>((resolve, reject) => {
        let cmd: string
        let args: string[]

        if (platform === 'darwin') {
          cmd = 'open'
          args = [resolved]
        } else if (platform === 'win32') {
          cmd = 'explorer'
          args = [resolved]
        } else {
          cmd = 'xdg-open'
          args = [resolved]
        }

        execFile(cmd, args, (error) => {
          if (error) reject(error)
          else resolve()
        })
      })

      return { success: true, path: resolved }
    } catch (error) {
      return {
        success: false,
        path: resolved,
        error: `Failed to open folder: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  // ─── CRUD ────────────────────────────────────────────────────────

  /**
   * Create a new file with optional content.
   */
  async createFile(
    filePath: string,
    content?: string,
  ): Promise<FileOperationResult> {
    const resolved = await this.fileManager.resolvePath(filePath)

    try {
      const fs = await import('node:fs/promises')
      const path = await import('node:path')

      // Ensure parent directory exists
      const parent = path.dirname(resolved)
      await fs.mkdir(parent, { recursive: true })

      await fs.writeFile(resolved, content ?? '', 'utf-8')
      return { success: true, paths: [resolved] }
    } catch (error) {
      return {
        success: false,
        paths: [resolved],
        error: `Failed to create file: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  /**
   * Create a new directory (with recursive parent creation).
   */
  async createFolder(dirPath: string): Promise<FileOperationResult> {
    const resolved = await this.fileManager.resolvePath(dirPath)

    try {
      const fs = await import('node:fs/promises')
      await fs.mkdir(resolved, { recursive: true })
      return { success: true, paths: [resolved] }
    } catch (error) {
      return {
        success: false,
        paths: [resolved],
        error: `Failed to create folder: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  /**
   * Delete a file or directory.
   * Directories are deleted recursively.
   */
  async delete(pathStr: string): Promise<FileOperationResult> {
    const resolved = await this.fileManager.resolvePath(pathStr)
    const exists = await this.fileManager.fileExists(resolved)

    if (!exists) {
      return {
        success: false,
        paths: [resolved],
        error: `Not found: ${resolved}`,
      }
    }

    try {
      const fs = await import('node:fs/promises')
      const stat = await fs.stat(resolved)

      if (stat.isDirectory()) {
        await fs.rm(resolved, { recursive: true, force: true })
      } else {
        await fs.unlink(resolved)
      }

      return { success: true, paths: [resolved] }
    } catch (error) {
      return {
        success: false,
        paths: [resolved],
        error: `Failed to delete: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  /**
   * Rename a file or directory.
   */
  async rename(oldPath: string, newName: string): Promise<FileOperationResult> {
    const oldResolved = await this.fileManager.resolvePath(oldPath)
    const path = await import('node:path')
    const parent = path.dirname(oldResolved)
    const newResolved = path.join(parent, newName)

    try {
      const fs = await import('node:fs/promises')
      await fs.rename(oldResolved, newResolved)
      return { success: true, paths: [oldResolved, newResolved] }
    } catch (error) {
      return {
        success: false,
        paths: [oldResolved, newResolved],
        error: `Failed to rename: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  // ─── Private Helpers ─────────────────────────────────────────────

  /** Resolve a QuickDir name or absolute path to an absolute path. */
  private async resolveDir(dirOrQuickDir: string): Promise<string> {
    const quickDirs = await this.fileManager.getCommonDirectories()

    if (dirOrQuickDir in quickDirs) {
      return quickDirs[dirOrQuickDir as keyof typeof quickDirs]
    }

    return this.fileManager.resolvePath(dirOrQuickDir)
  }

  /** Check if a path is a symlink. */
  private async isSymlink(filePath: string): Promise<boolean> {
    try {
      const fs = await import('node:fs/promises')
      const stat = await fs.lstat(filePath)
      return stat.isSymbolicLink()
    } catch {
      return false
    }
  }

  /** Simple MIME type guess from file extension. */
  private guessMimeType(filename: string, isDirectory: boolean): string {
    if (isDirectory) return 'inode/directory'

    const ext = filename.split('.').pop()?.toLowerCase() ?? ''
    const mimeMap: Record<string, string> = {
      txt: 'text/plain',
      md: 'text/markdown',
      html: 'text/html',
      css: 'text/css',
      js: 'text/javascript',
      ts: 'text/typescript',
      json: 'application/json',
      xml: 'application/xml',
      pdf: 'application/pdf',
      zip: 'application/zip',
      gz: 'application/gzip',
      tar: 'application/x-tar',
      png: 'image/png',
      jpg: 'image/jpeg',
      jpeg: 'image/jpeg',
      gif: 'image/gif',
      svg: 'image/svg+xml',
      webp: 'image/webp',
      mp3: 'audio/mpeg',
      mp4: 'video/mp4',
      avi: 'video/x-msvideo',
      mkv: 'video/x-matroska',
      doc: 'application/msword',
      docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
      xls: 'application/vnd.ms-excel',
      xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
      ppt: 'application/vnd.ms-powerpoint',
      pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
    }

    return mimeMap[ext] ?? 'application/octet-stream'
  }

  /** Convert a simple glob pattern to a RegExp. */
  private globToRegex(pattern: string): RegExp {
    const escaped = pattern
      .replace(/[.+^${}()|[\]\\]/g, '\\$&')
      .replace(/\*/g, '.*')
      .replace(/\?/g, '.')
    return new RegExp(`^${escaped}$`, 'i')
  }
}
