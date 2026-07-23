// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  CheckFormsIssuesResult,
  GetEncodedResponseParams,
  GetEncodedResponseResult,
  IssueAddedEvent,
} from '../domains/audits'

export interface AuditsApi {
  // ── Commands ──

  getEncodedResponse(
    params: GetEncodedResponseParams,
  ): Promise<GetEncodedResponseResult>
  disable(): Promise<void>
  enable(): Promise<void>
  checkFormsIssues(): Promise<CheckFormsIssuesResult>

  // ── Events ──

  on(
    event: 'issueAdded',
    handler: (params: IssueAddedEvent) => void,
  ): () => void
}
