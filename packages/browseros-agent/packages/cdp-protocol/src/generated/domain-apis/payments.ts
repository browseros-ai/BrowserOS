// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  AddCardParams,
  AddCardResult,
  CancelOperationParams,
  FillCardParams,
  FillCardResult,
  GetContextResult,
  ListCardsParams,
  ListCardsResult,
} from '../domains/payments'

export interface PaymentsApi {
  // ── Commands ──

  getContext(): Promise<GetContextResult>
  listCards(params: ListCardsParams): Promise<ListCardsResult>
  addCard(params: AddCardParams): Promise<AddCardResult>
  fillCard(params: FillCardParams): Promise<FillCardResult>
  cancelOperation(params: CancelOperationParams): Promise<void>
}
