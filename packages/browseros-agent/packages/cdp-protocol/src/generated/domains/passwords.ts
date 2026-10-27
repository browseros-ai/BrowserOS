// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BrowserContextID, BrowserProfileID } from './browser'

// ══ Types ══

export interface Credential {
  credentialId: string
  username: string
  site: string
  stores: string[]
}

export interface Context {
  browserProfileId: BrowserProfileID
  origin: string
  documentId: string
}

export interface CredentialResult {
  browserProfileId: BrowserProfileID
  origin?: string
  status?: string
  credentials?: Credential[]
  credential?: Credential
  total?: number
  nextCursor?: string
  filledFields?: string[]
}

// ══ Commands ══

export interface GetContextResult {
  context: Context
}

export interface ListCredentialsParams {
  operationId: string
  documentId?: string
  browserContextId?: BrowserContextID
  query?: string
  limit?: number
  cursor?: string
}

export interface ListCredentialsResult {
  result: CredentialResult
}

export interface AddCredentialParams {
  operationId: string
  documentId: string
  username: string
  password: string
}

export interface AddCredentialResult {
  result: CredentialResult
}

export interface FillCredentialParams {
  operationId: string
  documentId: string
  credentialId?: string
  username?: string
  password?: string
}

export interface FillCredentialResult {
  result: CredentialResult
}

export interface CancelOperationParams {
  operationId: string
}
