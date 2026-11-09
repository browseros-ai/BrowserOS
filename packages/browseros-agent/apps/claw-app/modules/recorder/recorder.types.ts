/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export interface RecorderEventsMessage {
  type: 'recorder-events'
  ndjson: string
  hasGap: boolean
}

export interface RecorderResnapshotMessage {
  type: 'recorder-resnapshot'
}

export interface RecorderStopMessage {
  type: 'recorder-stop'
}

/** Asked by a new page before it starts recording; answered with `{ enabled }`. */
export interface RecorderEnabledMessage {
  type: 'recorder-enabled'
}

export type RecorderMessage =
  | RecorderEventsMessage
  | RecorderResnapshotMessage
  | RecorderStopMessage
  | RecorderEnabledMessage
