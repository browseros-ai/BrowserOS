diff --git a/chrome/browser/browseros/onboarding/browseros_onboarding.h b/chrome/browser/browseros/onboarding/browseros_onboarding.h
new file mode 100644
index 0000000000000..6d84599152fc6
--- /dev/null
+++ b/chrome/browser/browseros/onboarding/browseros_onboarding.h
@@ -0,0 +1,37 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_ONBOARDING_BROWSEROS_ONBOARDING_H_
+#define CHROME_BROWSER_BROWSEROS_ONBOARDING_BROWSEROS_ONBOARDING_H_
+
+#include "base/functional/callback_forward.h"
+#include "base/memory/raw_ptr.h"
+#include "content/public/browser/web_ui_controller.h"
+#include "content/public/browser/webui_config.h"
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
+  void SetCompletionCallback(base::RepeatingClosure completion_callback);
+
+ private:
+  raw_ptr<BrowserOSOnboardingHandler> handler_ = nullptr;
+
+  WEB_UI_CONTROLLER_TYPE_DECL();
+};
+
+#endif  // CHROME_BROWSER_BROWSEROS_ONBOARDING_BROWSEROS_ONBOARDING_H_
