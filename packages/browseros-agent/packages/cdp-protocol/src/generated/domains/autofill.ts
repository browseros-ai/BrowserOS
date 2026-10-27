// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

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

// ══ Events ══

export interface AddressFormFilledEvent {
  filledFields: FilledField[]
  addressUi: AddressUI
}
