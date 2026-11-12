// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BrowserContextID, BrowserProfileID } from './browser'

// ══ Types ══

export interface Card {
  cardId: string
  network: string
  lastFour: string
  expiryMonth: number
  expiryYear: number
  expired: boolean
  name: string
  nickname?: string
  kind: string
  cvcSaved: boolean
}

export interface SuppliedCard {
  number: string
  expiryMonth: number
  expiryYear: number
  name?: string
  cvc?: string
}

export interface Context {
  browserProfileId: BrowserProfileID
  origin: string
  documentId: string
}

export interface CardResult {
  browserProfileId: BrowserProfileID
  origin?: string
  status?: string
  cards?: Card[]
  card?: Card
  source?: string
  network?: string
  lastFour?: string
  filledFields?: string[]
  missingFields?: string[]
}

// ══ Commands ══

export interface GetContextResult {
  context: Context
}

export interface ListCardsParams {
  operationId: string
  documentId?: string
  browserContextId?: BrowserContextID
}

export interface ListCardsResult {
  result: CardResult
}

export interface AddCardParams {
  operationId: string
  documentId?: string
  browserContextId?: BrowserContextID
  card: SuppliedCard
}

export interface AddCardResult {
  result: CardResult
}

export interface FillCardParams {
  operationId: string
  documentId: string
  cardId?: string
  card?: SuppliedCard
}

export interface FillCardResult {
  result: CardResult
}

export interface CancelOperationParams {
  operationId: string
}
