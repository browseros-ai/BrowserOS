/**
 * @license
 * Copyright 2025 BrowserOS
 */
export interface ScreenshotData {
  html: string
}

export const screenshots: Record<string, ScreenshotData> = {
  basic: {
    html: '<div>Hello MCP</div>',
  },
  viewportOverflow: {
    html: '<div style="height: 120vh; background-color: rebeccapurple;">View Port overflow</div>',
  },
  button: {
    html: '<button>I am button click me</button>',
  },
}
