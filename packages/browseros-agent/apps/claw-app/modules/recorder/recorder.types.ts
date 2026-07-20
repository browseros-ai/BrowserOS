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

export type RecorderMessage = RecorderEventsMessage | RecorderResnapshotMessage
