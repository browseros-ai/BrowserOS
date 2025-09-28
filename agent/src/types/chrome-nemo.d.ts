// Type definitions for chrome.nemo API

declare namespace chrome.nemo {
  // Page load status information
  interface PageLoadStatus {
    isResourcesLoading: boolean;
    isDOMContentLoaded: boolean;
    isPageComplete: boolean;
  }

  // Rectangle bounds
  interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
  }

  // Alias for backward compatibility
  type BoundingRect = Rect;

  // Interactive element types
  type InteractiveNodeType = "clickable" | "typeable" | "selectable" | "other";

  // Supported keyboard keys
  type Key =
    | "Enter"
    | "Delete"
    | "Backspace"
    | "Tab"
    | "Escape"
    | "ArrowUp"
    | "ArrowDown"
    | "ArrowLeft"
    | "ArrowRight"
    | "Home"
    | "End"
    | "PageUp"
    | "PageDown";

  // Interactive node in the snapshot
  interface InteractiveNode {
    nodeId: number;
    type: InteractiveNodeType;
    name?: string;
    rect?: Rect;
    attributes?: {
      in_viewport?: string;  // "true" if visible in viewport, "false" if not visible
      [key: string]: any;
    };
  }

  // Snapshot of interactive elements
  interface InteractiveSnapshot {
    snapshotId: number;
    timestamp: number;
    elements: InteractiveNode[];
    hierarchicalStructure?: string; // Hierarchical text representation with context
    processingTimeMs: number; // Performance metrics
  }

  // Options for getInteractiveSnapshot
  interface InteractiveSnapshotOptions {
    viewportOnly?: boolean;
  }

  // Accessibility node
  interface AccessibilityNode {
    id: number;
    role: string;
    name?: string;
    value?: string;
    attributes?: Record<string, any>;
    childIds?: number[];
  }

  // Accessibility tree
  interface AccessibilityTree {
    rootId: number;
    nodes: Record<string, AccessibilityNode>;
  }

  // API functions
  function getPageLoadStatus(
    tabId: number,
    callback: (status: PageLoadStatus) => void,
  ): void;

  function getPageLoadStatus(callback: (status: PageLoadStatus) => void): void;

  function getAccessibilityTree(
    tabId: number,
    callback: (tree: AccessibilityTree) => void,
  ): void;

  function getAccessibilityTree(
    callback: (tree: AccessibilityTree) => void,
  ): void;

  function getInteractiveSnapshot(
    tabId: number,
    options: InteractiveSnapshotOptions,
    callback: (snapshot: InteractiveSnapshot) => void,
  ): void;

  function getInteractiveSnapshot(
    tabId: number,
    callback: (snapshot: InteractiveSnapshot) => void,
  ): void;

  function getInteractiveSnapshot(
    options: InteractiveSnapshotOptions,
    callback: (snapshot: InteractiveSnapshot) => void,
  ): void;

  function getInteractiveSnapshot(
    callback: (snapshot: InteractiveSnapshot) => void,
  ): void;

  function click(tabId: number, nodeId: number, callback: () => void): void;

  function click(nodeId: number, callback: () => void): void;

  function inputText(
    tabId: number,
    nodeId: number,
    text: string,
    callback: () => void,
  ): void;

  function inputText(nodeId: number, text: string, callback: () => void): void;

  function clear(tabId: number, nodeId: number, callback: () => void): void;

  function clear(nodeId: number, callback: () => void): void;

  function scrollUp(tabId: number, callback: () => void): void;

  function scrollUp(callback: () => void): void;

  function scrollDown(tabId: number, callback: () => void): void;

  function scrollDown(callback: () => void): void;

  function scrollToNode(
    tabId: number,
    nodeId: number,
    callback: (scrolled: boolean) => void,
  ): void;

  function scrollToNode(
    nodeId: number,
    callback: (scrolled: boolean) => void,
  ): void;

  function sendKeys(
    tabId: number,
    key:
      | "Enter"
      | "Delete"
      | "Backspace"
      | "Tab"
      | "Escape"
      | "ArrowUp"
      | "ArrowDown"
      | "ArrowLeft"
      | "ArrowRight"
      | "Home"
      | "End"
      | "PageUp"
      | "PageDown",
    callback: () => void,
  ): void;

  function sendKeys(
    key:
      | "Enter"
      | "Delete"
      | "Backspace"
      | "Tab"
      | "Escape"
      | "ArrowUp"
      | "ArrowDown"
      | "ArrowLeft"
      | "ArrowRight"
      | "Home"
      | "End"
      | "PageUp"
      | "PageDown",
    callback: () => void,
  ): void;

  // Capture screenshot with all optional parameters
  function captureScreenshot(
    tabId: number,
    thumbnailSize: number,
    showHighlights: boolean,
    width: number,
    height: number,
    callback: (dataUrl: string) => void,
  ): void;

  // Capture screenshot with tab ID, thumbnail size, and highlights
  function captureScreenshot(
    tabId: number,
    thumbnailSize: number,
    showHighlights: boolean,
    callback: (dataUrl: string) => void,
  ): void;

  // Capture screenshot with tab ID and thumbnail size
  function captureScreenshot(
    tabId: number,
    thumbnailSize: number,
    callback: (dataUrl: string) => void,
  ): void;

  // Capture screenshot with tab ID only (backwards compatibility)
  function captureScreenshot(
    tabId: number,
    callback: (dataUrl: string) => void,
  ): void;

  // Capture screenshot of active tab with default size
  function captureScreenshot(callback: (dataUrl: string) => void): void;

  // Snapshot extraction types
  type SnapshotType = "text" | "links";

  // Context for snapshot extraction
  type SnapshotContext = "visible" | "full";

  // Section types based on ARIA landmarks
  type SectionType =
    | "main"
    | "navigation"
    | "footer"
    | "header"
    | "article"
    | "aside"
    | "complementary"
    | "contentinfo"
    | "form"
    | "search"
    | "region"
    | "other";

  // Text snapshot result for a section
  interface TextSnapshotResult {
    text: string;
    characterCount: number;
  }

  // Link information
  interface LinkInfo {
    text: string;
    url: string;
    title?: string;
    attributes?: Record<string, any>;
    isExternal: boolean;
  }

  // Links snapshot result for a section
  interface LinksSnapshotResult {
    links: LinkInfo[];
  }

  // Section with all possible snapshot results
  interface SnapshotSection {
    type: string;
    textResult?: TextSnapshotResult;
    linksResult?: LinksSnapshotResult;
  }

  // Main snapshot result
  interface Snapshot {
    type: SnapshotType;
    context: SnapshotContext;
    timestamp: number;
    sections: SnapshotSection[];
    processingTimeMs: number;
  }

  // Options for getSnapshot
  interface SnapshotOptions {
    context?: SnapshotContext;
    includeSections?: SectionType[];
  }

  function getSnapshot(
    tabId: number,
    type: SnapshotType,
    options: SnapshotOptions,
    callback: (snapshot: Snapshot) => void,
  ): void;

  function getSnapshot(
    tabId: number,
    type: SnapshotType,
    callback: (snapshot: Snapshot) => void,
  ): void;

  function getSnapshot(
    type: SnapshotType,
    options: SnapshotOptions,
    callback: (snapshot: Snapshot) => void,
  ): void;

  function getSnapshot(
    type: SnapshotType,
    callback: (snapshot: Snapshot) => void,
  ): void;

  // Get Nemo version number
  function getVersionNumber(callback: (version: string) => void): void;

  // Logs a metric event with optional properties
  function logMetric(
    eventName: string,
    properties: Record<string, any>,
    callback: () => void,
  ): void;

  function logMetric(eventName: string, callback: () => void): void;

  function logMetric(eventName: string, properties?: Record<string, any>): void;

  function logMetric(eventName: string): void;

  // Execute JavaScript in a tab
  function executeJavaScript(
    tabId: number,
    code: string,
    callback: (result: any) => void,
  ): void;

  function executeJavaScript(
    code: string,
    callback: (result: any) => void,
  ): void;

  // Click at specific viewport coordinates
  function clickCoordinates(
    tabId: number,
    x: number,
    y: number,
    callback: () => void,
  ): void;

  function clickCoordinates(
    x: number,
    y: number,
    callback: () => void,
  ): void;

  // Type text at specific viewport coordinates
  function typeAtCoordinates(
    tabId: number,
    x: number,
    y: number,
    text: string,
    callback: () => void,
  ): void;

  function typeAtCoordinates(
    x: number,
    y: number,
    text: string,
    callback: () => void,
  ): void;
}
