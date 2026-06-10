// biome-ignore-all lint/suspicious/noExplicitAny: Low-level browser API adapter requires flexible types
/// <reference path="./chrome-browser-os.d.ts" />

export type InteractiveSnapshot = chrome.browserOS.InteractiveSnapshot
export type InteractiveSnapshotOptions =
  chrome.browserOS.InteractiveSnapshotOptions
export type PageLoadStatus = chrome.browserOS.PageLoadStatus
export type Key = chrome.browserOS.Key
export type AccessibilityTree = chrome.browserOS.AccessibilityTree
export type Snapshot = chrome.browserOS.Snapshot
export type SnapshotOptions = chrome.browserOS.SnapshotOptions
export type PrefObject = chrome.browserOS.PrefObject
export type ChoosePathOptions = chrome.browserOS.ChoosePathOptions
export type SelectedPath = chrome.browserOS.SelectedPath

const SCREENSHOT_SIZES = {
  small: 512,
  medium: 768,
  large: 1028,
} as const

export type ScreenshotSizeKey = keyof typeof SCREENSHOT_SIZES

export class BrowserOSAdapter {
  private static instance: BrowserOSAdapter | null = null

  private constructor() {}

  static getInstance(): BrowserOSAdapter {
    if (!BrowserOSAdapter.instance) {
      BrowserOSAdapter.instance = new BrowserOSAdapter()
    }
    return BrowserOSAdapter.instance
  }

  async getInteractiveSnapshot(
    tabId: number,
    options?: InteractiveSnapshotOptions,
  ): Promise<InteractiveSnapshot> {
    return new Promise<InteractiveSnapshot>((resolve, reject) => {
      const callback = (snapshot: InteractiveSnapshot) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(snapshot)
        }
      }

      if (options) {
        chrome.browserOS.getInteractiveSnapshot(tabId, options, callback)
      } else {
        chrome.browserOS.getInteractiveSnapshot(tabId, callback)
      }
    })
  }

  async click(tabId: number, nodeId: number): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      chrome.browserOS.click(tabId, nodeId, () => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve()
        }
      })
    })
  }

  async inputText(tabId: number, nodeId: number, text: string): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      chrome.browserOS.inputText(tabId, nodeId, text, () => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve()
        }
      })
    })
  }

  async clear(tabId: number, nodeId: number): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      chrome.browserOS.clear(tabId, nodeId, () => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve()
        }
      })
    })
  }

  async scrollToNode(tabId: number, nodeId: number): Promise<boolean> {
    return new Promise<boolean>((resolve, reject) => {
      chrome.browserOS.scrollToNode(tabId, nodeId, (scrolled: boolean) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(scrolled)
        }
      })
    })
  }

  async sendKeys(tabId: number, keys: Key): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      chrome.browserOS.sendKeys(tabId, keys, () => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve()
        }
      })
    })
  }

  async getPageLoadStatus(tabId: number): Promise<PageLoadStatus> {
    return new Promise<PageLoadStatus>((resolve, reject) => {
      chrome.browserOS.getPageLoadStatus(tabId, (status: PageLoadStatus) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(status)
        }
      })
    })
  }

  async getAccessibilityTree(tabId: number): Promise<AccessibilityTree> {
    return new Promise<AccessibilityTree>((resolve, reject) => {
      chrome.browserOS.getAccessibilityTree(
        tabId,
        (tree: AccessibilityTree) => {
          if (chrome.runtime.lastError) {
            reject(
              new Error(chrome.runtime.lastError.message || 'Unknown error'),
            )
          } else {
            resolve(tree)
          }
        },
      )
    })
  }

  async captureScreenshot(
    tabId: number,
    size?: ScreenshotSizeKey,
    showHighlights?: boolean,
    width?: number,
    height?: number,
  ): Promise<string> {
    return new Promise<string>((resolve, reject) => {
      const callback = (dataUrl: string) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(dataUrl)
        }
      }

      if (width !== undefined && height !== undefined) {
        chrome.browserOS.captureScreenshot(
          tabId,
          0,
          showHighlights || false,
          width,
          height,
          callback,
        )
      } else if (size !== undefined || showHighlights !== undefined) {
        const pixelSize = size ? SCREENSHOT_SIZES[size] : 0
        if (showHighlights !== undefined) {
          chrome.browserOS.captureScreenshot(
            tabId,
            pixelSize,
            showHighlights,
            callback,
          )
        } else {
          chrome.browserOS.captureScreenshot(tabId, pixelSize, callback)
        }
      } else {
        chrome.browserOS.captureScreenshot(tabId, callback)
      }
    })
  }

  async getSnapshot(
    tabId: number,
    options?: SnapshotOptions,
  ): Promise<Snapshot> {
    return new Promise<Snapshot>((resolve, reject) => {
      const callback = (snapshot: Snapshot) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(snapshot)
        }
      }

      if (options) {
        chrome.browserOS.getSnapshot(tabId, options, callback)
      } else {
        chrome.browserOS.getSnapshot(tabId, callback)
      }
    })
  }

  async getVersion(): Promise<string | null> {
    return new Promise<string | null>((resolve, reject) => {
      if (typeof chrome.browserOS.getVersionNumber !== 'function') {
        resolve(null)
        return
      }

      chrome.browserOS.getVersionNumber((version: string) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(version)
        }
      })
    })
  }

  async getBrowserosVersion(): Promise<string | null> {
    return new Promise<string | null>((resolve, reject) => {
      if (typeof chrome.browserOS.getBrowserosVersionNumber !== 'function') {
        resolve(null)
        return
      }

      chrome.browserOS.getBrowserosVersionNumber((version: string) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(version)
        }
      })
    })
  }

  async logMetric(
    eventName: string,
    properties?: Record<string, any>,
  ): Promise<void> {
    if (typeof chrome.browserOS.logMetric !== 'function') {
      return
    }

    return new Promise<void>((resolve, reject) => {
      const callback = () => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve()
        }
      }

      if (properties) {
        chrome.browserOS.logMetric(eventName, properties, callback)
      } else {
        chrome.browserOS.logMetric(eventName, callback)
      }
    })
  }

  async executeJavaScript(tabId: number, code: string): Promise<any> {
    if (typeof chrome.browserOS.executeJavaScript !== 'function') {
      throw new Error('executeJavaScript API not available')
    }

    return new Promise<any>((resolve, reject) => {
      chrome.browserOS.executeJavaScript(tabId, code, (result: any) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(result)
        }
      })
    })
  }

  async clickCoordinates(tabId: number, x: number, y: number): Promise<void> {
    if (typeof chrome.browserOS.clickCoordinates !== 'function') {
      throw new Error('clickCoordinates API not available')
    }

    return new Promise<void>((resolve, reject) => {
      chrome.browserOS.clickCoordinates(tabId, x, y, () => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve()
        }
      })
    })
  }

  async typeAtCoordinates(
    tabId: number,
    x: number,
    y: number,
    text: string,
  ): Promise<void> {
    if (typeof chrome.browserOS.typeAtCoordinates !== 'function') {
      throw new Error('typeAtCoordinates API not available')
    }

    return new Promise<void>((resolve, reject) => {
      chrome.browserOS.typeAtCoordinates(tabId, x, y, text, () => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve()
        }
      })
    })
  }

  async getPref(name: string): Promise<PrefObject> {
    if (typeof chrome.browserOS?.getPref !== 'function') {
      throw new Error('getPref API not available')
    }

    return new Promise<PrefObject>((resolve, reject) => {
      chrome.browserOS.getPref(name, (pref: PrefObject) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(pref)
        }
      })
    })
  }

  async setPref(name: string, value: any, pageId?: string): Promise<boolean> {
    if (typeof chrome.browserOS?.setPref !== 'function') {
      throw new Error('setPref API not available')
    }

    return new Promise<boolean>((resolve, reject) => {
      const callback = (success: boolean) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(success)
        }
      }

      if (pageId !== undefined) {
        chrome.browserOS.setPref(name, value, pageId, callback)
      } else {
        chrome.browserOS.setPref(name, value, callback)
      }
    })
  }

  async getAllPrefs(): Promise<PrefObject[]> {
    if (typeof chrome.browserOS?.getAllPrefs !== 'function') {
      throw new Error('getAllPrefs API not available')
    }

    return new Promise<PrefObject[]>((resolve, reject) => {
      chrome.browserOS.getAllPrefs((prefs: PrefObject[]) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(prefs)
        }
      })
    })
  }

  async choosePath(options?: ChoosePathOptions): Promise<SelectedPath | null> {
    if (typeof chrome.browserOS?.choosePath !== 'function') {
      throw new Error('choosePath API not available')
    }

    return new Promise<SelectedPath | null>((resolve, reject) => {
      const callback = (result: SelectedPath | null) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message || 'Unknown error'))
        } else {
          resolve(result)
        }
      }

      if (options) {
        chrome.browserOS.choosePath(options, callback)
      } else {
        chrome.browserOS.choosePath(callback)
      }
    })
  }

  isAPIAvailable(method: string): boolean {
    return method in chrome.browserOS
  }

  getAvailableAPIs(): string[] {
    return Object.keys(chrome.browserOS).filter(
      (key) => typeof (chrome.browserOS as any)[key] === 'function',
    )
  }
}

/** @public */
export const getBrowserOSAdapter = () => BrowserOSAdapter.getInstance()
