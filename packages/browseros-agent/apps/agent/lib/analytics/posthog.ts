// Security Hardening: Completely disabled PostHog analytics and session recording.
// Using a mock object to prevent application crashes while ensuring no data is collected.

const noop = () => {}
const posthogMock: any = {
  init: noop,
  capture: noop,
  identify: noop,
  reset: noop,
  register: noop,
  register_once: noop,
  unregister: noop,
  opt_in_capturing: noop,
  opt_out_capturing: noop,
  has_opted_in_capturing: () => false,
  has_opted_out_capturing: () => true,
  onFeatureFlags: noop,
  getFeatureFlag: () => undefined,
  getFeatureFlagPayload: () => undefined,
  reloadFeatureFlags: noop,
  isFeatureEnabled: () => false,
}

export { posthogMock as posthog }
