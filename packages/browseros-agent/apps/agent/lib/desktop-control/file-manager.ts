/**
 * FileManager — desktop file operations for file dialog interaction.
 *
 * Provides utilities to list files, resolve paths, and navigate
 * folder structures through native file dialogs. Used by the
 * UnifiedCoordinator to prepare file paths before typing them
 * into file dialogs.
 *
 * @module desktop-control/file-manager
 */

import type { DesktopControlService } from './types'

// ─── File Info ─────────────────────────────────────────────────────

/** Basic file or directory information. */
export interface FileInfo {
  /** Absolute path. */
  path: string
  /** File or directory name. */
  name: string
  /** Whether this is a directory. */
  isDirectory: boolean
  /** File size in bytes (0 for directories). */
  size: number
  /** Last modified timestamp (ms since epoch). */
  modifiedAt: number
}

/** Options for listing directory contents. */
export interface ListFilesOptions {
  /** Whether to include hidden files (dotfiles). Default false. */
  includeHidden?: boolean
  /** Maximum number of results. Default 1000. */
  maxResults?: number
}

/** Result of a file search. */
export interface FileSearchResult {
  /** Files that matched the search. */
  matches: FileInfo[]
  /** Whether the search was truncated due to maxResults. */
  truncated: boolean
}

// ─── FileManager ───────────────────────────────────────────────────

/**
 * FileManager handles desktop file operations needed for
 * automating file dialog interactions.
 *
 * Uses Node.js `fs` and `path` modules for file system access.
 * All operations are scoped to the user's home directory by default.
 *
 * @public
 */
export class FileManager {
  private service: DesktopControlService

  constructor(_service: DesktopControlService) {
    this.service = _service
  }

  /**
   * List files and directories at the given path.
   *
   * @param dirPath — Directory to list. Defaults to user home.
   * @param options — Listing options (hidden files, max results).
   * @returns Array of file info objects.
   */
  async listFiles(dirPath: string, options?: ListFilesOptions): Promise<FileInfo[]> {
    const fs = await this.loadFs()
    const path = await this.loadPath()
    const includeHidden = options?.includeHidden ?? false
    const maxResults = options?.maxResults ?? 1000

    try {
      const entries = await fs.readdir(dirPath, { withFileTypes: true })
      const results: FileInfo[] = []

      for (const entry of entries) {
        if (results.length >= maxResults) break

        // Skip hidden files unless requested
        if (!includeHidden && entry.name.startsWith('.')) continue

        const fullPath = path.join(dirPath, entry.name)
        let stat: { size: number; mtimeMs: number }

        try {
          stat = await fs.stat(fullPath)
        } catch {
          // Skip entries we can't stat (permissions, broken symlinks)
          continue
        }

        results.push({
          path: fullPath,
          name: entry.name,
          isDirectory: entry.isDirectory(),
          size: stat.size,
          modifiedAt: stat.mtimeMs,
        })
      }

      // Sort: directories first, then alphabetical
      results.sort((a, b) => {
        if (a.isDirectory !== b.isDirectory) {
          return a.isDirectory ? -1 : 1
        }
        return a.name.localeCompare(b.name)
      })

      return results
    } catch (error) {
      throw new Error(
        `Failed to list directory "${dirPath}": ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Search for files matching a pattern in a directory (non-recursive).
   *
   * @param dirPath — Directory to search in.
   * @param pattern — Glob-like pattern (supports * wildcard).
   * @param options — Search options.
   * @returns Matching files.
   */
  async searchFiles(
    dirPath: string,
    pattern: string,
    options?: ListFilesOptions,
  ): Promise<FileSearchResult> {
    const files = await this.listFiles(dirPath, options)
    const regex = this.globToRegex(pattern)
    const maxResults = options?.maxResults ?? 1000

    const matches = files.filter((f) => regex.test(f.name))
    const truncated = matches.length > maxResults

    return {
      matches: matches.slice(0, maxResults),
      truncated,
    }
  }

  /**
   * Check if a file exists at the given path.
   */
  async fileExists(filePath: string): Promise<boolean> {
    const fs = await this.loadFs()
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  /**
   * Get the absolute path for a potentially relative path.
   * Resolves ~ to the user's home directory.
   */
  async resolvePath(filePath: string): Promise<string> {
    const path = await this.loadPath()
    const os = await this.loadOs()

    // Expand ~ to home directory
    if (filePath.startsWith('~')) {
      const home = os.homedir()
      filePath = home + filePath.slice(1)
    }

    return path.resolve(filePath)
  }

  /**
   * Get the user's home directory.
   */
  async getHomeDirectory(): Promise<string> {
    const os = await this.loadOs()
    return os.homedir()
  }

  /**
   * Get common directories (Desktop, Documents, Downloads, etc.).
   */
  async getCommonDirectories(): Promise<Record<string, string>> {
    const path = await this.loadPath()
    const home = await this.getHomeDirectory()

    return {
      home,
      desktop: path.join(home, 'Desktop'),
      documents: path.join(home, 'Documents'),
      downloads: path.join(home, 'Downloads'),
      pictures: path.join(home, 'Pictures'),
      music: path.join(home, 'Music'),
      videos: path.join(home, 'Videos'),
    }
  }

  /**
   * Validate a file path for use in a file dialog.
   * Checks that the file exists and returns its absolute path.
   *
   * @returns Absolute path if valid, null if file not found.
   */
  async validateFilePath(filePath: string): Promise<string | null> {
    const resolved = await this.resolvePath(filePath)

    if (await this.fileExists(resolved)) {
      return resolved
    }

    return null
  }

  /**
   * Get the parent directory of a file path.
   */
  async getParentDirectory(filePath: string): Promise<string> {
    const path = await this.loadPath()
    return path.dirname(await this.resolvePath(filePath))
  }

  // ─── Private Helpers ──────────────────────────────────────────────

  /**
   * Convert a simple glob pattern to a RegExp.
   * Supports * (any chars) and ? (single char).
   */
  private globToRegex(pattern: string): RegExp {
    const escaped = pattern
      .replace(/[.+^${}()|[\]\\]/g, '\\$&')
      .replace(/\*/g, '.*')
      .replace(/\?/g, '.')
    return new RegExp(`^${escaped}$`, 'i')
  }

  /** Lazy-load Node.js fs/promises. */
  private async loadFs() {
    return import('fs/promises') as Promise<typeof import('fs/promises')>
  }

  /** Lazy-load Node.js path module. */
  private async loadPath() {
    return import('path') as Promise<typeof import('path')>
  }

  /** Lazy-load Node.js os module. */
  private async loadOs() {
    return import('os') as Promise<typeof import('os')>
  }
}
