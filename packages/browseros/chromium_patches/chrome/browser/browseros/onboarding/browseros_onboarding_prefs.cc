diff --git a/chrome/browser/browseros/onboarding/browseros_onboarding_prefs.cc b/chrome/browser/browseros/onboarding/browseros_onboarding_prefs.cc
new file mode 100644
index 0000000000000..ecf479d743ba8
--- /dev/null
+++ b/chrome/browser/browseros/onboarding/browseros_onboarding_prefs.cc
@@ -0,0 +1,37 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/onboarding/browseros_onboarding_prefs.h"
+
+#include "chrome/browser/browseros/core/browseros_prefs.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/common/chrome_constants.h"
+#include "components/prefs/pref_service.h"
+
+namespace browseros::onboarding {
+
+bool ShouldShow(Profile* profile) {
+  if (!profile || !profile->IsRegularProfile() || profile->IsOffTheRecord()) {
+    return false;
+  }
+
+  if (profile->GetBaseName() !=
+      base::FilePath().AppendASCII(chrome::kInitialProfile)) {
+    return false;
+  }
+
+  return !profile->GetPrefs()->GetBoolean(
+      browseros::prefs::kOnboardingCompleted);
+}
+
+void MarkCompleted(Profile* profile) {
+  if (!profile || !profile->IsRegularProfile()) {
+    return;
+  }
+
+  profile->GetPrefs()->SetBoolean(browseros::prefs::kOnboardingCompleted,
+                                  true);
+}
+
+}  // namespace browseros::onboarding
