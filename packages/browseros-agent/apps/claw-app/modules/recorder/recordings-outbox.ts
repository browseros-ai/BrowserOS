/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export interface StoredRecordingBatch {
  sequence: number
  batchId: string
  tabId: number
  documentId: string
  ndjson: string
  bytes: number
  createdAt: number
}

export type NewRecordingBatch = Omit<StoredRecordingBatch, 'sequence'>

export interface RecordingOutbox {
  add: (batch: NewRecordingBatch) => Promise<void>
  list: () => Promise<StoredRecordingBatch[]>
  remove: (batchId: string) => Promise<void>
  markGap: (documentId: string, tabId: number) => Promise<void>
  getGap: (documentId: string) => Promise<StoredRecordingGap | undefined>
  clearGap: (documentId: string, token: string) => Promise<void>
}

export interface StoredRecordingGap {
  documentId: string
  tabId: number
  token: string
}

const DATABASE_NAME = 'browseros-claw-recording-outbox'
const DATABASE_VERSION = 1
const BATCHES_STORE = 'batches'
const GAPS_STORE = 'gaps'
const BATCH_ID_INDEX = 'batchId'

function requestResult<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error)
  })
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve()
    transaction.onerror = () => reject(transaction.error)
    transaction.onabort = () => reject(transaction.error)
  })
}

/** Durable handoff between short-lived extension backgrounds and recording ingest. */
export function createIndexedDbRecordingOutbox(): RecordingOutbox {
  let databasePromise: Promise<IDBDatabase> | undefined

  function database(): Promise<IDBDatabase> {
    databasePromise ??= new Promise((resolve, reject) => {
      const request = indexedDB.open(DATABASE_NAME, DATABASE_VERSION)
      request.onupgradeneeded = () => {
        const database = request.result
        const batches = database.createObjectStore(BATCHES_STORE, {
          keyPath: 'sequence',
          autoIncrement: true,
        })
        batches.createIndex(BATCH_ID_INDEX, BATCH_ID_INDEX, { unique: true })
        database.createObjectStore(GAPS_STORE, { keyPath: 'documentId' })
      }
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => reject(request.error)
    })
    return databasePromise
  }

  return {
    async add(batch) {
      const db = await database()
      const transaction = db.transaction(BATCHES_STORE, 'readwrite')
      transaction.objectStore(BATCHES_STORE).add(batch)
      await transactionDone(transaction)
    },
    async list() {
      const db = await database()
      const transaction = db.transaction(BATCHES_STORE, 'readonly')
      const batches = await requestResult(
        transaction.objectStore(BATCHES_STORE).getAll() as IDBRequest<
          StoredRecordingBatch[]
        >,
      )
      await transactionDone(transaction)
      return batches.sort((left, right) => left.sequence - right.sequence)
    },
    async remove(batchId) {
      const db = await database()
      const transaction = db.transaction(BATCHES_STORE, 'readwrite')
      const store = transaction.objectStore(BATCHES_STORE)
      const key = await requestResult(
        store.index(BATCH_ID_INDEX).getKey(batchId),
      )
      if (key !== undefined) store.delete(key)
      await transactionDone(transaction)
    },
    async markGap(documentId, tabId) {
      const db = await database()
      const transaction = db.transaction(GAPS_STORE, 'readwrite')
      transaction.objectStore(GAPS_STORE).put({
        documentId,
        tabId,
        token: crypto.randomUUID(),
      } satisfies StoredRecordingGap)
      await transactionDone(transaction)
    },
    async getGap(documentId) {
      const db = await database()
      const transaction = db.transaction(GAPS_STORE, 'readonly')
      const gap = await requestResult(
        transaction.objectStore(GAPS_STORE).get(documentId) as IDBRequest<
          StoredRecordingGap | undefined
        >,
      )
      await transactionDone(transaction)
      return gap
    },
    async clearGap(documentId, token) {
      const db = await database()
      const transaction = db.transaction(GAPS_STORE, 'readwrite')
      const store = transaction.objectStore(GAPS_STORE)
      const gap = await requestResult(
        store.get(documentId) as IDBRequest<StoredRecordingGap | undefined>,
      )
      if (gap?.token === token) store.delete(documentId)
      await transactionDone(transaction)
    },
  }
}
