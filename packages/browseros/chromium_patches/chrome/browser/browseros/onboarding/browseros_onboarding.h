diff --git a/chrome/browser/browseros/onboarding/browseros_onboarding.h b/chrome/browser/browseros/onboarding/browseros_onboarding.h
new file mode 100644
index 0000000000000000000000000000000000000000..7126c2accebe176d3803c2e3c5f4590c37cdcb02
--- /dev/null
+++ b/chrome/browser/browseros/onboarding/browseros_onboarding.h
@@ -0,0 +1,68 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_ONBOARDING_BROWSEROS_ONBOARDING_H_
+#define CHROME_BROWSER_BROWSEROS_ONBOARDING_BROWSEROS_ONBOARDING_H_
+
+#include "base/functional/callback_forward.h"
+#include "base/memory/raw_ptr.h"
+#include "base/memory/ref_counted.h"
+#include "content/public/browser/web_ui_controller.h"
+#include "content/public/browser/webui_config.h"
+
+// The first-run step owns this across WebUI reloads. The page observes native
+// completion state; it does not own the registry wait or infer readiness.
+struct BrowserOSOnboardingSetupState
+    : public base::RefCounted<BrowserOSOnboardingSetupState> {
+  BrowserOSOnboardingSetupState();
+  enum class Status { kIdle, kPreparing, kFailed, kReady };
+  Status status = Status::kIdle;
+  unsigned int attempt = 0;
+  bool completion_handled = false;
+
+ private:
+  friend class base::RefCounted<BrowserOSOnboardingSetupState>;
+  ~BrowserOSOnboardingSetupState();
+};
+
+using BrowserOSOnboardingEnsureReady =
+    base::RepeatingCallback<void(base::OnceCallback<void(bool)>)>;
+
+namespace content {
+class NavigationThrottleRegistry;
+}
+
+class BrowserOSOnboardingHandler;
+class BrowserOSOnboarding;
+
+class BrowserOSOnboardingUIConfig
+    : public content::DefaultWebUIConfig<BrowserOSOnboarding> {
+ public:
+  BrowserOSOnboardingUIConfig();
+};
+
+class BrowserOSOnboarding : public content::WebUIController {
+ public:
+  explicit BrowserOSOnboarding(content::WebUI* web_ui);
+  BrowserOSOnboarding(const BrowserOSOnboarding&) = delete;
+  BrowserOSOnboarding& operator=(const BrowserOSOnboarding&) = delete;
+  ~BrowserOSOnboarding() override;
+
+  // Legacy resources navigate immediately after COMPLETE. Retain the native
+  // setup page until the flow opens its destination after the READY callback.
+  static void MaybeCreateNavigationThrottle(
+      content::NavigationThrottleRegistry& registry);
+
+  void SetCompletionCallback(
+      base::RepeatingClosure completion_callback,
+      BrowserOSOnboardingEnsureReady ensure_ready,
+      scoped_refptr<BrowserOSOnboardingSetupState> setup_state);
+
+ private:
+  raw_ptr<BrowserOSOnboardingHandler> handler_ = nullptr;
+
+  WEB_UI_CONTROLLER_TYPE_DECL();
+};
+
+#endif  // CHROME_BROWSER_BROWSEROS_ONBOARDING_BROWSEROS_ONBOARDING_H_
