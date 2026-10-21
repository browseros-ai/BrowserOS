// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BrowserContextID, BrowserProfileID } from './browser'
import type { BackendNodeId } from './dom'
import type { FrameId } from './page'

// ══ Types ══

export interface CreditCard {
  number: string
  name: string
  expiryMonth: string
  expiryYear: string
  cvc: string
}

export interface AddressField {
  name: string
  value: string
}

export interface AddressFields {
  fields: AddressField[]
}

export interface Address {
  fields: AddressField[]
}

export interface AddressUI {
  addressFields: AddressFields[]
}

export type FillingStrategy = 'autocompleteAttribute' | 'autofillInferred'

export interface FilledField {
  htmlType: string
  id: string
  name: string
  value: string
  autofillType: string
  fillingStrategy: FillingStrategy
  frameId: FrameId
  fieldId: BackendNodeId
}

export interface CredentialInfo {
  credentialId: string
  username: string
  site: string
  stores: string[]
}

export interface CredentialContext {
  browserProfileId: BrowserProfileID
  origin: string
  documentId: string
}

export interface CredentialResult {
  browserProfileId: BrowserProfileID
  origin?: string
  status?: string
  credentials?: CredentialInfo[]
  credential?: CredentialInfo
  total?: number
  nextCursor?: string
  filledFields?: string[]
}

// ══ Commands ══

export interface TriggerParams {
  fieldId: BackendNodeId
  frameId?: FrameId
  card?: CreditCard
  address?: Address
}

export interface SetAddressesParams {
  addresses: Address[]
}

export interface GetCredentialContextResult {
  context: CredentialContext
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

export interface CancelCredentialOperationParams {
  operationId: string
}

// ══ Events ══

export interface AddressFormFilledEvent {
  filledFields: FilledField[]
  addressUi: AddressUI
}
