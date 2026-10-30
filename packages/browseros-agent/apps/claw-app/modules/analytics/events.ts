/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The cockpit's event catalog. Events are metadata about which screens
 * and controls get used; they never carry user content (no urls,
 * titles, prompts, tool i/o). `track` only accepts scalar properties.
 * neo Pro events never carry a licence key, an email, an amount or
 * cancellation feedback.
 * Explicit Ultrafast email submissions use a separate request in
 * screens/ultrafast/ultrafast-signup.ts, independent of usage analytics.
 */

import { capture } from './posthog'

export const AnalyticsEvent = {
  AppOpened: 'app_opened',
  AuditViewed: 'audit_viewed',
  ReplayViewed: 'replay_viewed',
  TaskDetailViewed: 'task_detail_viewed',
  OptOutToggled: 'analytics_opt_out_toggled',
  OnboardingVideoPlayed: 'onboarding_video_played',
  OnboardingVideoPaused: 'onboarding_video_paused',
  OnboardingVideoResumed: 'onboarding_video_resumed',
  OnboardingVideoCompleted: 'onboarding_video_completed',
  OnboardingTaskCopied: 'onboarding_task_copied',
  OnboardingLinkClicked: 'onboarding_link_clicked',
  ProductHuntBannerShown: 'product_hunt_banner_shown',
  ProductHuntBannerClicked: 'product_hunt_banner_clicked',
  ProductHuntBannerDismissed: 'product_hunt_banner_dismissed',
  FeedbackInviteShown: 'feedback_invite_shown',
  FeedbackInviteClicked: 'feedback_invite_clicked',
  FeedbackInviteDismissed: 'feedback_invite_dismissed',
  InstallGuideOpened: 'install_guide_opened',
  InstallGuideDownloadClicked: 'install_guide_download_clicked',
  ManualSetupOpened: 'manual_setup_opened',
  ManualSetupCommandCopied: 'manual_setup_command_copied',
  UltrafastBannerClicked: 'ultrafast_banner_clicked',
  UltrafastWaitlistViewed: 'ultrafast_waitlist_viewed',
  HumanHelpToggled: 'human_help_toggled',
  ProPageViewed: 'pro_page_viewed',
  ProCheckoutStarted: 'pro_checkout_started',
  ProCheckoutCompleted: 'pro_checkout_completed',
  ProCheckoutAbandoned: 'pro_checkout_abandoned',
  ProCheckoutRefused: 'pro_checkout_refused',
  ProCancelConfirmed: 'pro_cancel_confirmed',
  ProResumed: 'pro_resumed',
  ProBillingOpened: 'pro_billing_opened',
  ProKeyDialogOpened: 'pro_key_dialog_opened',
  ProKeyActivated: 'pro_key_activated',
  ProKeyActivationFailed: 'pro_key_activation_failed',
  ProComputerRemoved: 'pro_computer_removed',
  ProComputerRenamed: 'pro_computer_renamed',
  ProKeyReset: 'pro_key_reset',
} as const

export type AnalyticsEventName =
  (typeof AnalyticsEvent)[keyof typeof AnalyticsEvent]

export function track(
  event: AnalyticsEventName,
  properties?: Record<string, boolean | number | string>,
): void {
  capture(event, properties)
}

/**
 * Maps a cockpit route to its view event. The audit table, a task
 * detail, a replay and the neo Pro page are the surfaces we care about;
 * the home and mcp routes are covered by `app_opened`.
 */
export function screenEventForPath(path: string): AnalyticsEventName | null {
  if (path === '/audit') return AnalyticsEvent.AuditViewed
  if (path === '/pro') return AnalyticsEvent.ProPageViewed
  if (/^\/audit\/[^/]+\/replay$/.test(path)) return AnalyticsEvent.ReplayViewed
  if (/^\/audit\/[^/]+$/.test(path)) return AnalyticsEvent.TaskDetailViewed
  return null
}
