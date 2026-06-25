/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 onboarding mount point. The real implementation lives under
 * `screens/onboarding-v2/`; this file is the router's import target
 * so the App.tsx route table stays unchanged across phases. Phase 0
 * shipped a placeholder here; Phase 3 swaps it for the real shell.
 */

export { OnboardingV2 } from '@/screens/onboarding-v2/OnboardingV2'
