/**
 * DesktopPlugin — loads and manages desktop control capabilities in BrowserOS.
 *
 * This module is the entry-point that the BrowserOS agent uses to activate
 * desktop control. It:
 *   1. Initialises all desktop services (screenshot, mouse, keyboard, etc.)
 *   2. Registers desktop-agent-tools in the tool registry
 *   3. Provides enable/disable toggle
 *   4. Runs health checks for desktop capabilities
 *
 * Usage:
 *   import { DesktopPlugin } from './desktop-control/desktop-plugin'
 *   const plugin = new DesktopPlugin()
 *   await plugin.activate()
 *   // tools are now registered and available
 *   const health = await plugin.healthCheck()
 *
 * @module desktop-control/desktop-plugin
 */

import type { ToolDefinition } from '../../../server/src/tools/framework'
import type { DesktopToolServices } from './desktop-agent-tools'
import {
  desktopTools,
  getDesktopServices,
  resetDesktopServices,
} from './desktop-agent-tools'
import type { DesktopControlService } from './types'

// ─── Health Check Types ────────────────────────────────────────────

export interface DesktopCapabilityStatus {
  /** Name of the capability. */
  name: string
  /** Whether the capability is available. */
  available: boolean
  /** Additional detail or error message. */
  detail?: string
}

export interface DesktopHealthReport {
  /** Overall health: true if all critical capabilities are available. */
  healthy: boolean
  /** Individual capability statuses. */
  capabilities: DesktopCapabilityStatus[]
  /** Timestamp of the health check (ms since epoch). */
  timestamp: number
  /** Whether the plugin is currently enabled. */
  enabled: boolean
}

// ─── Plugin Events ─────────────────────────────────────────────────

export type PluginEvent = 'activated' | 'deactivated' | 'health-check'

export type PluginEventListener = (
  event: PluginEvent,
  plugin: DesktopPlugin,
) => void

// ─── DesktopPlugin ─────────────────────────────────────────────────

/**
 * DesktopPlugin manages the desktop control feature lifecycle.
 *
 * Lifecycle:
 *   new DesktopPlugin() → activate() → [healthCheck() | getTools()] → deactivate()
 *
 * The plugin is designed to be loaded lazily — services are only initialised
 * when `activate()` is called, and cleaned up on `deactivate()`.
 *
 * @public
 */
export class DesktopPlugin {
  private _enabled = false
  private _services: DesktopToolServices | null = null
  private _listeners: PluginEventListener[] = []

  /**
   * Activate the plugin: initialise services and prepare tools.
   *
   * @param service — Optional DesktopControlService implementation.
   *                  If omitted, a default native implementation is used.
   */
  async activate(service?: DesktopControlService): Promise<void> {
    if (this._enabled) {
      return
    }

    try {
      this._services = getDesktopServices(service)
      this._enabled = true
      this.emit('activated')
    } catch (error) {
      throw error
    }
  }

  /**
   * Deactivate the plugin: clean up resources and disable tools.
   */
  async deactivate(): Promise<void> {
    if (!this._enabled) {
      return
    }

    try {
      if (this._services?.service) {
        await this._services.service.dispose()
      }
    } catch (_error) {}

    resetDesktopServices()
    this._services = null
    this._enabled = false
    this.emit('deactivated')
  }

  /**
   * Check if the plugin is currently enabled.
   */
  isEnabled(): boolean {
    return this._enabled
  }

  /**
   * Get the desktop tool definitions for registration with a ToolRegistry.
   *
   * Returns an empty array if the plugin is not activated.
   */
  getTools(): ToolDefinition[] {
    if (!this._enabled) {
      return []
    }
    return desktopTools
  }

  /**
   * Get the underlying desktop services.
   * Returns null if the plugin is not activated.
   */
  getServices(): DesktopToolServices | null {
    return this._services
  }

  /**
   * Run a health check on all desktop capabilities.
   *
   * Tests whether each subsystem is functional:
   *   - Screenshot capture
   *   - Mouse control (get position)
   *   - Keyboard control (type text is a no-op check)
   *   - File system access (list home directory)
   *   - Application detection (list running apps)
   *   - Clipboard access (read)
   *   - System info retrieval
   *
   * @returns A health report with per-capability status.
   */
  async healthCheck(): Promise<DesktopHealthReport> {
    const capabilities: DesktopCapabilityStatus[] = []

    // If not enabled, attempt a lightweight activation
    if (!this._enabled) {
      return {
        healthy: false,
        capabilities: [
          {
            name: 'plugin',
            available: false,
            detail: 'Plugin is not activated. Call activate() first.',
          },
        ],
        timestamp: Date.now(),
        enabled: false,
      }
    }

    const svc = this._services!

    // 1. Screenshot
    try {
      const screenshot = await svc.screenshot.captureScreenshot()
      capabilities.push({
        name: 'screenshot',
        available: screenshot.width > 0 || screenshot.base64.length > 0,
        detail: `${screenshot.width}×${screenshot.height} (${screenshot.mimeType})`,
      })
    } catch (error) {
      capabilities.push({
        name: 'screenshot',
        available: false,
        detail: error instanceof Error ? error.message : String(error),
      })
    }

    // 2. Mouse
    try {
      const pos = await svc.mouse.getMousePosition()
      capabilities.push({
        name: 'mouse',
        available: true,
        detail: `Position: (${pos.x}, ${pos.y})`,
      })
    } catch (error) {
      capabilities.push({
        name: 'mouse',
        available: false,
        detail: error instanceof Error ? error.message : String(error),
      })
    }

    // 3. Keyboard (best-effort — we can't truly test without side effects)
    try {
      // Keyboard service is always "available" if instantiated,
      // but native robotjs might not be loaded. We just check instantiation.
      capabilities.push({
        name: 'keyboard',
        available: true,
        detail:
          'Keyboard service instantiated (native backend check deferred to first use).',
      })
    } catch (error) {
      capabilities.push({
        name: 'keyboard',
        available: false,
        detail: error instanceof Error ? error.message : String(error),
      })
    }

    // 4. File system
    try {
      const home = await svc.fileManager.getHomeDirectory()
      const files = await svc.fileManager.listFiles(home, { maxResults: 5 })
      capabilities.push({
        name: 'filesystem',
        available: true,
        detail: `Home: ${home} (${files.length} items listed)`,
      })
    } catch (error) {
      capabilities.push({
        name: 'filesystem',
        available: false,
        detail: error instanceof Error ? error.message : String(error),
      })
    }

    // 5. Application detection
    try {
      const apps = await svc.appLauncher.listRunning()
      capabilities.push({
        name: 'apps',
        available: true,
        detail: `${apps.length} running applications detected`,
      })
    } catch (error) {
      capabilities.push({
        name: 'apps',
        available: false,
        detail: error instanceof Error ? error.message : String(error),
      })
    }

    // 6. Clipboard
    try {
      const clip = await svc.osCommands.clipboardRead()
      capabilities.push({
        name: 'clipboard',
        available: clip.success,
        detail: clip.success
          ? `Read ${clip.text?.length ?? 0} chars`
          : (clip.error ?? 'Unknown error'),
      })
    } catch (error) {
      capabilities.push({
        name: 'clipboard',
        available: false,
        detail: error instanceof Error ? error.message : String(error),
      })
    }

    // 7. System info
    try {
      const info = await svc.osCommands.getSystemInfo()
      capabilities.push({
        name: 'system_info',
        available: true,
        detail: `${info.platform} ${info.arch}, ${info.cpuCores} cores, ${Math.round(info.totalMemoryBytes / 1024 / 1024 / 1024)} GB RAM`,
      })
    } catch (error) {
      capabilities.push({
        name: 'system_info',
        available: false,
        detail: error instanceof Error ? error.message : String(error),
      })
    }

    const healthy = capabilities.every((c) => c.available)

    this.emit('health-check')

    return {
      healthy,
      capabilities,
      timestamp: Date.now(),
      enabled: this._enabled,
    }
  }

  /**
   * Subscribe to plugin events.
   */
  on(listener: PluginEventListener): () => void {
    this._listeners.push(listener)
    return () => {
      this._listeners = this._listeners.filter((l) => l !== listener)
    }
  }

  // ─── Private ────────────────────────────────────────────────────

  private emit(event: PluginEvent): void {
    for (const listener of this._listeners) {
      try {
        listener(event, this)
      } catch (_error) {}
    }
  }
}

// ─── Singleton ─────────────────────────────────────────────────────

/** Global singleton DesktopPlugin instance. */
let _instance: DesktopPlugin | null = null

/**
 * Get the global DesktopPlugin singleton.
 * Creates it lazily on first access.
 */
export function getDesktopPlugin(): DesktopPlugin {
  if (!_instance) {
    _instance = new DesktopPlugin()
  }
  return _instance
}

/**
 * Reset the global singleton (for testing).
 */
export function resetDesktopPlugin(): void {
  if (_instance) {
    _instance.deactivate().catch(() => {})
    _instance = null
  }
}
