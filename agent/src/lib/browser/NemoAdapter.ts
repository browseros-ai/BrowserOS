/// <reference path="../../types/chrome-nemo.d.ts" />

// ============= Re-export types from chrome.nemo namespace =============

export type InteractiveNode = chrome.nemo.InteractiveNode;
export type InteractiveSnapshot = chrome.nemo.InteractiveSnapshot;
export type InteractiveSnapshotOptions =
  chrome.nemo.InteractiveSnapshotOptions;
export type PageLoadStatus = chrome.nemo.PageLoadStatus;
export type InteractiveNodeType = chrome.nemo.InteractiveNodeType;
export type Rect = chrome.nemo.BoundingRect;

// New snapshot types
export type SnapshotType = chrome.nemo.SnapshotType;
export type SnapshotContext = chrome.nemo.SnapshotContext;
export type SectionType = chrome.nemo.SectionType;
export type TextSnapshotResult = chrome.nemo.TextSnapshotResult;
export type LinkInfo = chrome.nemo.LinkInfo;
export type LinksSnapshotResult = chrome.nemo.LinksSnapshotResult;
export type SnapshotSection = chrome.nemo.SnapshotSection;
export type Snapshot = chrome.nemo.Snapshot;
export type SnapshotOptions = chrome.nemo.SnapshotOptions;

// ============= Nemo Adapter =============

// Screenshot size constants
export const SCREENSHOT_SIZES = {
  small: 512, // Low token usage
  medium: 768, // Balanced (default)
  large: 1028, // High detail (note: 1028 not 1024)
} as const;

export type ScreenshotSizeKey = keyof typeof SCREENSHOT_SIZES;

/**
 * Adapter for Chrome Nemo Extension APIs
 * Provides a clean interface to nemo functionality with extensibility
 */
export class NemoAdapter {
  private static instance: NemoAdapter | null = null;

  private constructor() {}

  /**
   * Get singleton instance
   */
  static getInstance(): NemoAdapter {
    if (!NemoAdapter.instance) {
      NemoAdapter.instance = new NemoAdapter();
    }
    return NemoAdapter.instance;
  }

  /**
   * Get interactive snapshot of the current page
   */
  async getInteractiveSnapshot(
    tabId: number,
    options?: InteractiveSnapshotOptions,
  ): Promise<InteractiveSnapshot> {
    try {
      console.log(
        `[NemoAdapter] Getting interactive snapshot for tab ${tabId} with options: ${JSON.stringify(options)}`,
      );

      return new Promise<InteractiveSnapshot>((resolve, reject) => {
        if (options) {
          chrome.nemo.getInteractiveSnapshot(
            tabId,
            options,
            (snapshot: InteractiveSnapshot) => {
              if (chrome.runtime.lastError) {
                reject(new Error(chrome.runtime.lastError.message));
              } else {
                console.log(
                  `[NemoAdapter] Retrieved snapshot with ${snapshot.elements.length} elements`,
                );
                resolve(snapshot);
              }
            },
          );
        } else {
          chrome.nemo.getInteractiveSnapshot(
            tabId,
            (snapshot: InteractiveSnapshot) => {
              if (chrome.runtime.lastError) {
                reject(new Error(chrome.runtime.lastError.message));
              } else {
                console.log(
                  `[NemoAdapter] Retrieved snapshot with ${snapshot.elements.length} elements`,
                );
                resolve(snapshot);
              }
            },
          );
        }
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to get interactive snapshot: ${errorMessage}`,
      );
      throw new Error(`Failed to get interactive snapshot: ${errorMessage}`);
    }
  }

  /**
   * Click an element by node ID
   */
  async click(tabId: number, nodeId: number): Promise<void> {
    try {
      console.log(`[NemoAdapter] Clicking node ${nodeId} in tab ${tabId}`);

      return new Promise<void>((resolve, reject) => {
        chrome.nemo.click(tabId, nodeId, () => {
          if (chrome.runtime.lastError) {
            reject(new Error(chrome.runtime.lastError.message));
          } else {
            resolve();
          }
        });
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(`[NemoAdapter] Failed to click node: ${errorMessage}`);
      throw new Error(`Failed to click node ${nodeId}: ${errorMessage}`);
    }
  }

  /**
   * Input text into an element
   */
  async inputText(tabId: number, nodeId: number, text: string): Promise<void> {
    try {
      console.log(
        `[NemoAdapter] Inputting text into node ${nodeId} in tab ${tabId}`,
      );

      return new Promise<void>((resolve, reject) => {
        chrome.nemo.inputText(tabId, nodeId, text, () => {
          if (chrome.runtime.lastError) {
            reject(new Error(chrome.runtime.lastError.message));
          } else {
            resolve();
          }
        });
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(`[NemoAdapter] Failed to input text: ${errorMessage}`);
      throw new Error(
        `Failed to input text into node ${nodeId}: ${errorMessage}`,
      );
    }
  }

  /**
   * Clear text from an element
   */
  async clear(tabId: number, nodeId: number): Promise<void> {
    try {
      console.log(`[NemoAdapter] Clearing node ${nodeId} in tab ${tabId}`);

      return new Promise<void>((resolve, reject) => {
        chrome.nemo.clear(tabId, nodeId, () => {
          if (chrome.runtime.lastError) {
            reject(new Error(chrome.runtime.lastError.message));
          } else {
            resolve();
          }
        });
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(`[NemoAdapter] Failed to clear node: ${errorMessage}`);
      throw new Error(`Failed to clear node ${nodeId}: ${errorMessage}`);
    }
  }

  /**
   * Scroll to a specific node
   */
  async scrollToNode(tabId: number, nodeId: number): Promise<boolean> {
    try {
      console.log(
        `[NemoAdapter] Scrolling to node ${nodeId} in tab ${tabId}`,
      );

      return new Promise<boolean>((resolve, reject) => {
        chrome.nemo.scrollToNode(tabId, nodeId, (scrolled: boolean) => {
          if (chrome.runtime.lastError) {
            reject(new Error(chrome.runtime.lastError.message));
          } else {
            resolve(scrolled);
          }
        });
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to scroll to node: ${errorMessage}`,
      );
      throw new Error(`Failed to scroll to node ${nodeId}: ${errorMessage}`);
    }
  }

  /**
   * Send keyboard keys
   */
  async sendKeys(tabId: number, keys: chrome.nemo.Key): Promise<void> {
    try {
      console.log(`[NemoAdapter] Sending keys "${keys}" to tab ${tabId}`);

      return new Promise<void>((resolve, reject) => {
        chrome.nemo.sendKeys(tabId, keys, () => {
          if (chrome.runtime.lastError) {
            reject(new Error(chrome.runtime.lastError.message));
          } else {
            resolve();
          }
        });
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(`[NemoAdapter] Failed to send keys: ${errorMessage}`);
      throw new Error(`Failed to send keys: ${errorMessage}`);
    }
  }

  /**
   * Get page load status
   */
  async getPageLoadStatus(tabId: number): Promise<PageLoadStatus> {
    try {
      console.log(
        `[NemoAdapter] Getting page load status for tab ${tabId}`,
      );

      return new Promise<PageLoadStatus>((resolve, reject) => {
        chrome.nemo.getPageLoadStatus(tabId, (status: PageLoadStatus) => {
          if (chrome.runtime.lastError) {
            reject(new Error(chrome.runtime.lastError.message));
          } else {
            resolve(status);
          }
        });
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to get page load status: ${errorMessage}`,
      );
      throw new Error(`Failed to get page load status: ${errorMessage}`);
    }
  }

  /**
   * Get accessibility tree (if available)
   */
  async getAccessibilityTree(
    tabId: number,
  ): Promise<chrome.nemo.AccessibilityTree> {
    try {
      console.log(
        `[NemoAdapter] Getting accessibility tree for tab ${tabId}`,
      );

      return new Promise<chrome.nemo.AccessibilityTree>(
        (resolve, reject) => {
          chrome.nemo.getAccessibilityTree(
            tabId,
            (tree: chrome.nemo.AccessibilityTree) => {
              if (chrome.runtime.lastError) {
                reject(new Error(chrome.runtime.lastError.message));
              } else {
                resolve(tree);
              }
            },
          );
        },
      );
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to get accessibility tree: ${errorMessage}`,
      );
      throw new Error(`Failed to get accessibility tree: ${errorMessage}`);
    }
  }

  /**
   * Capture a screenshot of the tab
   * @param tabId - The tab ID to capture
   * @param size - Optional screenshot size ('small', 'medium', or 'large')
   * @param showHighlights - Optional flag to show element highlights
   * @param width - Optional exact width for screenshot
   * @param height - Optional exact height for screenshot
   */
  async captureScreenshot(
    tabId: number,
    size?: ScreenshotSizeKey,
    showHighlights?: boolean,
    width?: number,
    height?: number,
  ): Promise<string> {
    try {
      const sizeDesc = size ? ` (${size})` : "";
      const highlightDesc = showHighlights ? " with highlights" : "";
      const dimensionsDesc = width && height ? ` (${width}x${height})` : "";
      console.log(
        `[NemoAdapter] Capturing screenshot for tab ${tabId}${sizeDesc}${highlightDesc}${dimensionsDesc}`,
      );

      return new Promise<string>((resolve, reject) => {
        // Use exact dimensions if provided
        if (width !== undefined && height !== undefined) {
          chrome.nemo.captureScreenshot(
            tabId,
            0, // thumbnailSize ignored when width/height specified
            showHighlights || false,
            width,
            height,
            (dataUrl: string) => {
              if (chrome.runtime.lastError) {
                reject(new Error(chrome.runtime.lastError.message));
              } else {
                console.log(
                  `[NemoAdapter] Screenshot captured for tab ${tabId} (${width}x${height})${highlightDesc}`,
                );
                resolve(dataUrl);
              }
            },
          );
        } else if (size !== undefined || showHighlights !== undefined) {
          const pixelSize = size ? SCREENSHOT_SIZES[size] : 0;
          // Use the API with thumbnail size and highlights
          if (showHighlights !== undefined) {
            chrome.nemo.captureScreenshot(
              tabId,
              pixelSize,
              showHighlights,
              (dataUrl: string) => {
                if (chrome.runtime.lastError) {
                  reject(new Error(chrome.runtime.lastError.message));
                } else {
                  console.log(
                    `[NemoAdapter] Screenshot captured for tab ${tabId}${sizeDesc}${highlightDesc}`,
                  );
                  resolve(dataUrl);
                }
              },
            );
          } else {
            chrome.nemo.captureScreenshot(
              tabId,
              pixelSize,
              (dataUrl: string) => {
                if (chrome.runtime.lastError) {
                  reject(new Error(chrome.runtime.lastError.message));
                } else {
                  console.log(
                    `[NemoAdapter] Screenshot captured for tab ${tabId} (${size}: ${pixelSize}px)`,
                  );
                  resolve(dataUrl);
                }
              },
            );
          }
        } else {
          // Use the original API without size (backwards compatibility)
          chrome.nemo.captureScreenshot(tabId, (dataUrl: string) => {
            if (chrome.runtime.lastError) {
              reject(new Error(chrome.runtime.lastError.message));
            } else {
              console.log(
                `[NemoAdapter] Screenshot captured for tab ${tabId}`,
              );
              resolve(dataUrl);
            }
          });
        }
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to capture screenshot: ${errorMessage}`,
      );
      throw new Error(`Failed to capture screenshot: ${errorMessage}`);
    }
  }

  /**
   * Get a content snapshot of the specified type from the page
   */
  async getSnapshot(
    tabId: number,
    type: SnapshotType,
    options?: SnapshotOptions,
  ): Promise<Snapshot> {
    try {
      console.log(
        `[NemoAdapter] Getting ${type} snapshot for tab ${tabId} with options: ${JSON.stringify(options)}`,
      );

      return new Promise<Snapshot>((resolve, reject) => {
        if (options) {
          chrome.nemo.getSnapshot(
            tabId,
            type,
            options,
            (snapshot: Snapshot) => {
              if (chrome.runtime.lastError) {
                reject(new Error(chrome.runtime.lastError.message));
              } else {
                console.log(
                  `[NemoAdapter] Retrieved ${type} snapshot with ${snapshot.sections.length} sections`,
                );
                resolve(snapshot);
              }
            },
          );
        } else {
          chrome.nemo.getSnapshot(tabId, type, (snapshot: Snapshot) => {
            if (chrome.runtime.lastError) {
              reject(new Error(chrome.runtime.lastError.message));
            } else {
              console.log(
                `[NemoAdapter] Retrieved ${type} snapshot with ${snapshot.sections.length} sections`,
              );
              resolve(snapshot);
            }
          });
        }
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to get ${type} snapshot: ${errorMessage}`,
      );
      throw new Error(`Failed to get ${type} snapshot: ${errorMessage}`);
    }
  }

  /**
   * Get text content snapshot from the page
   * Convenience method for text snapshot
   */
  async getTextSnapshot(
    tabId: number,
    options?: SnapshotOptions,
  ): Promise<Snapshot> {
    return this.getSnapshot(tabId, "text", options);
  }

  /**
   * Get links snapshot from the page
   * Convenience method for links snapshot
   */
  async getLinksSnapshot(
    tabId: number,
    options?: SnapshotOptions,
  ): Promise<Snapshot> {
    return this.getSnapshot(tabId, "links", options);
  }

  /**
   * Generic method to invoke any Nemo API
   * Useful for future APIs or experimental features
   */
  async invokeAPI(method: string, ...args: any[]): Promise<any> {
    try {
      console.log(`[NemoAdapter] Invoking Nemo API: ${method}`);

      if (!(method in chrome.nemo)) {
        throw new Error(`Unknown Nemo API method: ${method}`);
      }

      // @ts-expect-error - Dynamic API invocation
      const result = await chrome.nemo[method](...args);
      return result;
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to invoke API ${method}: ${errorMessage}`,
      );
      throw new Error(
        `Failed to invoke Nemo API ${method}: ${errorMessage}`,
      );
    }
  }

  /**
   * Check if a specific API is available
   */
  isAPIAvailable(method: string): boolean {
    return method in chrome.nemo;
  }

  /**
   * Get list of available Nemo
   */
  getAvailableAPIs(): string[] {
    return Object.keys(chrome.nemo).filter((key) => {
      // @ts-expect-error - Dynamic key access for API discovery
      return typeof chrome.nemo[key] === "function";
    });
  }

  /**
   * Get Nemoion information
   */
  async getVersion(): Promise<string | null> {
    try {
      console.log("[Nemoer] Getting Nemoion");

      return new Promise<string | null>((resolve, reject) => {
        // Check if getVersionNumber API is available
        if (
          "getVersionNumber" in chrome.nemo &&
          typeof chrome.nemo.getVersionNumber === "function"
        ) {
          chrome.nemo.getVersionNumber((version: string) => {
            if (chrome.runtime.lastError) {
              reject(new Error(chrome.runtime.lastError.message));
            } else {
              console.log(`[Nemoersion: $Nemo`);
              resolve(version);
            }
          });
        } else {
          // Fallback - return null if API not available
          resolve(null);
        }
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[Nemoailed to get version: ${errorMessage}`,
      );
      // Return null on error
      return null;
    }
  }

  /**
   * Log a metric event with optional properties
   */
  async logMetric(
    eventName: string,
    properties?: Record<string, any>,
  ): Promise<void> {
    try {
      console.log(
        `[Nemoer] Logging metric: ${eventName} with properties: ${JSON.stringify(properties)}`,
      );

      return new Promise<void>((resolve, reject) => {
        // Check if logMetric API is available
        if (
          "logMetric" in chrome.nemo &&
          typeof chrome.nemo.logMetric === "function"
        ) {
          if (properties) {
            chrome.nemo.logMetric(eventName, properties, () => {
              if (chrome.runtime.lastError) {
                reject(new Error(chrome.runtime.lastError.message));
              } else {
                console.log(`[NemoAdapter] Metric logged: ${eventName}`);
                resolve();
              }
            });
          } else {
            chrome.nemo.logMetric(eventName, () => {
              if (chrome.runtime.lastError) {
                reject(new Error(chrome.runtime.lastError.message));
              } else {
                console.log(`[NemoAdapter] Metric logged: ${eventName}`);
                resolve();
              }
            });
          }
        } else {
          // If API not available, log a warning but don't fail
          console.warn(
            `[NemoAdapter] logMetric API not available, skipping metric: ${eventName}`,
          );
          resolve();
        }
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(`[NemoAdapter] Failed to log metric: ${errorMessage}`);
      return;
    }
  }

  /**
   * Execute JavaScript code in the specified tab
   * @param tabId - The tab ID to execute code in
   * @param code - The JavaScript code to execute
   * @returns The result of the execution
   */
  async executeJavaScript(tabId: number, code: string): Promise<any> {
    try {
      console.log(
        `[NemoAdapter] Executing JavaScript in tab ${tabId}`,
      );

      return new Promise<any>((resolve, reject) => {
        // Check if executeJavaScript API is available
        if (
          "executeJavaScript" in chrome.nemo &&
          typeof chrome.nemo.executeJavaScript === "function"
        ) {
          chrome.nemo.executeJavaScript(tabId, code, (result: any) => {
            if (chrome.runtime.lastError) {
              reject(new Error(chrome.runtime.lastError.message));
            } else {
              console.log(
                `[NemoAdapter] JavaScript executed successfully in tab ${tabId}`,
              );
              resolve(result);
            }
          });
        } else {
          reject(new Error("executeJavaScript API not available"));
        }
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to execute JavaScript: ${errorMessage}`,
      );
      throw new Error(`Failed to execute JavaScript: ${errorMessage}`);
    }
  }

  /**
   * Click at specific viewport coordinates
   * @param tabId - The tab ID to click in
   * @param x - X coordinate in viewport pixels
   * @param y - Y coordinate in viewport pixels
   */
  async clickCoordinates(tabId: number, x: number, y: number): Promise<void> {
    try {
      console.log(
        `[NemoAdapter] Clicking at coordinates (${x}, ${y}) in tab ${tabId}`,
      );

      return new Promise<void>((resolve, reject) => {
        // Check if clickCoordinates API is available
        if (
          "clickCoordinates" in chrome.nemo &&
          typeof chrome.nemo.clickCoordinates === "function"
        ) {
          chrome.nemo.clickCoordinates(tabId, x, y, () => {
            if (chrome.runtime.lastError) {
              reject(new Error(chrome.runtime.lastError.message));
            } else {
              console.log(
                `[NemoAdapter] Successfully clicked at (${x}, ${y}) in tab ${tabId}`,
              );
              resolve();
            }
          });
        } else {
          reject(new Error("clickCoordinates API not available"));
        }
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to click at coordinates: ${errorMessage}`,
      );
      throw new Error(`Failed to click at coordinates (${x}, ${y}): ${errorMessage}`);
    }
  }

  /**
   * Type text at specific viewport coordinates
   * @param tabId - The tab ID to type in
   * @param x - X coordinate in viewport pixels
   * @param y - Y coordinate in viewport pixels
   * @param text - Text to type at the location
   */
  async typeAtCoordinates(
    tabId: number,
    x: number,
    y: number,
    text: string,
  ): Promise<void> {
    try {
      console.log(
        `[NemoAdapter] Typing at coordinates (${x}, ${y}) in tab ${tabId}`,
      );

      return new Promise<void>((resolve, reject) => {
        // Check if typeAtCoordinates API is available
        if (
          "typeAtCoordinates" in chrome.nemo &&
          typeof chrome.nemo.typeAtCoordinates === "function"
        ) {
          chrome.nemo.typeAtCoordinates(tabId, x, y, text, () => {
            if (chrome.runtime.lastError) {
              reject(new Error(chrome.runtime.lastError.message));
            } else {
              console.log(
                `[NemoAdapter] Successfully typed "${text}" at (${x}, ${y}) in tab ${tabId}`,
              );
              resolve();
            }
          });
        } else {
          reject(new Error("typeAtCoordinates API not available"));
        }
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      console.error(
        `[NemoAdapter] Failed to type at coordinates: ${errorMessage}`,
      );
      throw new Error(
        `Failed to type at coordinates (${x}, ${y}): ${errorMessage}`,
      );
    }
  }
}

// Export singleton instance getter for convenience
export const getNemoAdapter = () => NemoAdapter.getInstance();
