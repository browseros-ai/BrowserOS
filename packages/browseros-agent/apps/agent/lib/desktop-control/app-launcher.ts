/**
 * AppLauncher — open and manage desktop applications.
 *
 * Provides platform-aware application launching, detection, and management.
 * Supports macOS (open -a), Windows (start), and Linux (gtk-launch / flatpak / direct exec).
 *
 * @module desktop-control/app-launcher
 */

// ─── Types ─────────────────────────────────────────────────────────

/** Result of launching an application. */
export interface LaunchResult {
  /** Whether the launch command was issued successfully. */
  success: boolean
  /** Application name that was launched. */
  appName: string
  /** Error message if success is false. */
  error?: string
}

/** Information about a running application. */
export interface RunningAppInfo {
  /** Process name. */
  name: string
  /** Process ID (0 if unavailable). */
  pid: number
  /** Memory usage in bytes (0 if unavailable). */
  memoryBytes: number
  /** CPU usage percentage (0 if unavailable). */
  cpuPercent: number
}

/** Result of checking if an app is running. */
export interface AppStatusResult {
  /** Whether the application is currently running. */
  isRunning: boolean
  /** Number of processes found. */
  processCount: number
  /** Process IDs of matching processes. */
  pids: number[]
}

/** Result of quitting an application. */
export interface QuitResult {
  /** Whether the quit signal was sent. */
  success: boolean
  /** Application name. */
  appName: string
  /** Number of processes terminated. */
  terminatedCount: number
  /** Error message if success is false. */
  error?: string
}

// ─── Platform detection ────────────────────────────────────────────

type Platform = 'darwin' | 'linux' | 'win32'

function getPlatform(): Platform {
  return process.platform as Platform
}

/** Execute a command and return stdout/stderr. */
async function exec(
  command: string,
  args: string[],
  timeout = 10000,
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  const { execFile } = await import('child_process')
  return new Promise((resolve) => {
    const child = execFile(command, args, { timeout }, (error, stdout, stderr) => {
      resolve({
        stdout: stdout ?? '',
        stderr: stderr ?? '',
        exitCode: error ? (error as NodeJS.ErrnoException).code === 'ETIMEDOUT' ? -1 : (error as any).status ?? 1 : 0,
      })
    })
    // Ensure child is properly cleaned up
    child.on('error', () => {
      resolve({ stdout: '', stderr: 'Command not found', exitCode: 127 })
    })
  })
}

// ─── App name resolution ───────────────────────────────────────────

/**
 * Known application aliases mapping to platform-specific names.
 * This is a best-effort mapping; users can also pass the exact binary name.
 */
const APP_ALIASES: Record<string, Record<Platform, string[]>> = {
  'chrome': {
    darwin: ['Google Chrome'],
    linux: ['google-chrome', 'google-chrome-stable', 'chromium-browser', 'chromium'],
    win32: ['chrome'],
  },
  'chromium': {
    darwin: ['Chromium'],
    linux: ['chromium-browser', 'chromium'],
    win32: ['chromium'],
  },
  'firefox': {
    darwin: ['Firefox'],
    linux: ['firefox', 'firefox-esr'],
    win32: ['firefox'],
  },
  'vscode': {
    darwin: ['Visual Studio Code'],
    linux: ['code', 'code-oss'],
    win32: ['Code'],
  },
  'terminal': {
    darwin: ['Terminal'],
    linux: ['gnome-terminal', 'konsole', 'xterm', 'alacritty'],
    win32: ['cmd'],
  },
  'finder': {
    darwin: ['Finder'],
    linux: ['nautilus', 'dolphin', 'thunar', 'pcmanfm'],
    win32: ['explorer'],
  },
  'spotify': {
    darwin: ['Spotify'],
    linux: ['spotify'],
    win32: ['Spotify'],
  },
  'slack': {
    darwin: ['Slack'],
    linux: ['slack'],
    win32: ['slack'],
  },
  'discord': {
    darwin: ['Discord'],
    linux: ['discord'],
    win32: ['Discord'],
  },
  'steam': {
    darwin: ['Steam'],
    linux: ['steam'],
    win32: ['steam'],
  },
}

// ─── AppLauncher ───────────────────────────────────────────────────

/**
 * AppLauncher manages desktop application lifecycle:
 * launch, detect, quit, focus, and list running apps.
 *
 * @public
 */
export class AppLauncher {
  /**
   * Launch an application by name.
   *
   * On macOS: uses `open -a "AppName"`
   * On Windows: uses `start "" "AppName"`
   * On Linux: tries gtk-launch, flatpak, or direct execution
   */
  async launch(appName: string): Promise<LaunchResult> {
    const platform = getPlatform()
    const resolved = this.resolveAppName(appName, platform)

    try {
      if (platform === 'darwin') {
        return await this.launchMacOS(resolved)
      } else if (platform === 'win32') {
        return await this.launchWindows(resolved)
      } else {
        return await this.launchLinux(resolved)
      }
    } catch (error) {
      return {
        success: false,
        appName: resolved,
        error: `Failed to launch: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  /**
   * Check if an application is currently running.
   */
  async isRunning(appName: string): Promise<AppStatusResult> {
    const platform = getPlatform()
    const names = this.getProcessNames(appName, platform)

    try {
      if (platform === 'win32') {
        return await this.checkRunningWindows(names)
      } else {
        return await this.checkRunningUnix(names)
      }
    } catch {
      return { isRunning: false, processCount: 0, pids: [] }
    }
  }

  /**
   * Quit/kill an application by name.
   * On macOS/Linux: sends SIGTERM, then SIGKILL after timeout.
   * On Windows: uses taskkill.
   */
  async quit(appName: string): Promise<QuitResult> {
    const platform = getPlatform()
    const names = this.getProcessNames(appName, platform)

    try {
      if (platform === 'win32') {
        return await this.quitWindows(names)
      } else {
        return await this.quitUnix(names)
      }
    } catch (error) {
      return {
        success: false,
        appName,
        terminatedCount: 0,
        error: `Failed to quit: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  /**
   * List currently running applications.
   * Returns a deduplicated list of process names with their PIDs.
   */
  async listRunning(): Promise<RunningAppInfo[]> {
    const platform = getPlatform()

    try {
      if (platform === 'win32') {
        return await this.listRunningWindows()
      } else {
        return await this.listRunningUnix()
      }
    } catch {
      return []
    }
  }

  /**
   * Bring an application to the foreground / focus it.
   * Best-effort — may not work on all window managers.
   */
  async focus(appName: string): Promise<LaunchResult> {
    const platform = getPlatform()
    const resolved = this.resolveAppName(appName, platform)

    try {
      if (platform === 'darwin') {
        // Use osascript to bring app to front
        const { execFile } = await import('child_process')
        await new Promise<void>((resolve, reject) => {
          execFile('osascript', ['-e', `tell application "${resolved}" to activate`], (error) => {
            if (error) reject(error)
            else resolve()
          })
        })
        return { success: true, appName: resolved }
      } else if (platform === 'linux') {
        // Try wmctrl
        const { execFile } = await import('child_process')
        await new Promise<void>((resolve, reject) => {
          execFile('wmctrl', ['-a', resolved], (error) => {
            if (error) reject(error)
            else resolve()
          })
        })
        return { success: true, appName: resolved }
      } else {
        // Windows: use PowerShell
        const { exec } = await import('child_process')
        await new Promise<void>((resolve, reject) => {
          exec(`powershell -Command "(New-Object -ComObject WScript.Shell).AppActivate('${resolved}')"`, (error) => {
            if (error) reject(error)
            else resolve()
          })
        })
        return { success: true, appName: resolved }
      }
    } catch (error) {
      return {
        success: false,
        appName: resolved,
        error: `Failed to focus: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  // ─── Private: macOS ──────────────────────────────────────────────

  private async launchMacOS(appName: string): Promise<LaunchResult> {
    const { execFile } = await import('child_process')

    return new Promise((resolve) => {
      execFile('open', ['-a', appName], (error) => {
        if (error) {
          resolve({
            success: false,
            appName,
            error: error.message,
          })
        } else {
          resolve({ success: true, appName })
        }
      })
    })
  }

  // ─── Private: Windows ────────────────────────────────────────────

  private async launchWindows(appName: string): Promise<LaunchResult> {
    const { exec } = await import('child_process')

    return new Promise((resolve) => {
      exec(`start "" "${appName}"`, { shell: 'cmd.exe' }, (error) => {
        if (error) {
          resolve({
            success: false,
            appName,
            error: error.message,
          })
        } else {
          resolve({ success: true, appName })
        }
      })
    })
  }

  // ─── Private: Linux ──────────────────────────────────────────────

  private async launchLinux(appName: string): Promise<LaunchResult> {
    // Try strategies in order: gtk-launch → flatpak → direct exec → nohup
    const strategies = [
      () => this.tryExec('gtk-launch', [appName]),
      () => this.tryFlatpakLaunch(appName),
      () => this.tryExec('nohup', [appName], { detached: true }),
    ]

    for (const strategy of strategies) {
      try {
        const result = await strategy()
        if (result) {
          return { success: true, appName }
        }
      } catch {
        continue
      }
    }

    return {
      success: false,
      appName,
      error: `Could not launch "${appName}" on Linux. Tried gtk-launch, flatpak, and direct execution.`,
    }
  }

  private async tryExec(
    cmd: string,
    args: string[],
    options?: { detached?: boolean },
  ): Promise<boolean> {
    const { execFile } = await import('child_process')

    return new Promise((resolve) => {
      const child = execFile(cmd, args, (error) => {
        resolve(!error)
      })
      if (options?.detached && child.pid) {
        child.unref()
      }
      child.on('error', () => resolve(false))
    })
  }

  private async tryFlatpakLaunch(appName: string): Promise<boolean> {
    // Try to find a flatpak app ID matching the name
    try {
      const { execFile } = await import('child_process')
      const listOutput = await new Promise<string>((resolve) => {
        execFile('flatpak', ['list', '--app', '--columns=application'], (_, stdout) => {
          resolve(stdout ?? '')
        })
      })

      const lines = listOutput.split('\n').map((l) => l.trim()).filter(Boolean)
      const match = lines.find((l) =>
        l.toLowerCase().includes(appName.toLowerCase()),
      )

      if (match) {
        return this.tryExec('flatpak', ['run', match])
      }
    } catch {
      // flatpak not available
    }
    return false
  }

  // ─── Private: Running detection ──────────────────────────────────

  private async checkRunningUnix(names: string[]): Promise<AppStatusResult> {
    const pids: number[] = []

    for (const name of names) {
      const result = await exec('pgrep', ['-i', name], 5000)
      if (result.exitCode === 0) {
        const found = result.stdout
          .split('\n')
          .map((l) => parseInt(l.trim(), 10))
          .filter((n) => !isNaN(n))
        pids.push(...found)
      }
    }

    return {
      isRunning: pids.length > 0,
      processCount: pids.length,
      pids,
    }
  }

  private async checkRunningWindows(names: string[]): Promise<AppStatusResult> {
    const result = await exec('tasklist', ['/FO', 'CSV', '/NH'], 10000)
    const pids: number[] = []

    if (result.exitCode === 0) {
      for (const line of result.stdout.split('\n')) {
        const lower = line.toLowerCase()
        if (names.some((n) => lower.includes(n.toLowerCase()))) {
          const match = line.match(/"(\d+)"/)
          if (match) {
            pids.push(parseInt(match[1], 10))
          }
        }
      }
    }

    return {
      isRunning: pids.length > 0,
      processCount: pids.length,
      pids,
    }
  }

  private async quitUnix(names: string[]): Promise<QuitResult> {
    let terminatedCount = 0
    const status = await this.checkRunningUnix(names)

    if (status.pids.length === 0) {
      return { success: true, appName: names[0], terminatedCount: 0 }
    }

    // Send SIGTERM first
    for (const pid of status.pids) {
      try {
        process.kill(pid, 'SIGTERM')
        terminatedCount++
      } catch {
        // Process may have already exited
      }
    }

    // Wait and check, then SIGKILL if needed
    await new Promise((r) => setTimeout(r, 2000))

    const remaining = await this.checkRunningUnix(names)
    for (const pid of remaining.pids) {
      try {
        process.kill(pid, 'SIGKILL')
      } catch {
        // ignore
      }
    }

    return { success: true, appName: names[0], terminatedCount }
  }

  private async quitWindows(names: string[]): Promise<QuitResult> {
    let terminatedCount = 0

    for (const name of names) {
      const result = await exec('taskkill', ['/IM', name, '/F'], 10000)
      if (result.exitCode === 0) {
        terminatedCount++
      }
    }

    return { success: true, appName: names[0], terminatedCount }
  }

  // ─── Private: List running ───────────────────────────────────────

  private async listRunningUnix(): Promise<RunningAppInfo[]> {
    const result = await exec('ps', ['aux', '--sort=-pcpu'], 10000)
    if (result.exitCode !== 0) return []

    const seen = new Set<string>()
    const apps: RunningAppInfo[] = []

    for (const line of result.stdout.split('\n').slice(1)) {
      const parts = line.trim().split(/\s+/)
      if (parts.length < 11) continue

      const cpu = parseFloat(parts[2]) || 0
      const mem = parseFloat(parts[3]) || 0
      const pid = parseInt(parts[1], 10)
      const command = parts.slice(10).join(' ')
      const name = parts[10] ?? ''

      // Skip kernel threads and very short names
      if (!name || name === '[' || pid === 0) continue
      if (seen.has(name)) continue

      seen.add(name)
      // Convert RSS (parts[5]) from KB to bytes approximately
      const memBytes = (parseInt(parts[5], 10) || 0) * 1024

      apps.push({
        name,
        pid,
        memoryBytes: memBytes,
        cpuPercent: cpu,
      })
    }

    return apps.slice(0, 100) // Limit to top 100
  }

  private async listRunningWindows(): Promise<RunningAppInfo[]> {
    const result = await exec('tasklist', ['/FO', 'CSV', '/NH'], 15000)
    if (result.exitCode !== 0) return []

    const seen = new Set<string>()
    const apps: RunningAppInfo[] = []

    for (const line of result.stdout.split('\n')) {
      const match = line.match(/"([^"]+)","(\d+)"/)
      if (!match) continue

      const name = match[1]
      const pid = parseInt(match[2], 10)

      if (seen.has(name)) continue
      seen.add(name)

      apps.push({
        name,
        pid,
        memoryBytes: 0,
        cpuPercent: 0,
      })
    }

    return apps
  }

  // ─── Private: Name resolution ────────────────────────────────────

  /** Resolve a user-friendly app name to the platform-specific name. */
  private resolveAppName(appName: string, platform: Platform): string {
    const lower = appName.toLowerCase()
    const alias = APP_ALIASES[lower]
    if (alias) {
      const platformNames = alias[platform]
      return platformNames[0] ?? appName
    }
    return appName
  }

  /** Get all possible process names for an app alias. */
  private getProcessNames(appName: string, platform: Platform): string[] {
    const lower = appName.toLowerCase()
    const alias = APP_ALIASES[lower]
    if (alias) {
      return alias[platform]
    }
    return [appName]
  }
}
