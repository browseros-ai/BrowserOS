diff --git a/chrome/browser/browseros/customization/customization_prefs.h b/chrome/browser/browseros/customization/customization_prefs.h
new file mode 100644
index 0000000000000000000000000000000000000000..1c7eca87b78fa6b7f9dd19e011164dfacf69e8d6
--- /dev/null
+++ b/chrome/browser/browseros/customization/customization_prefs.h
@@ -0,0 +1,20 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_CUSTOMIZATION_PREFS_H_
+#define CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_CUSTOMIZATION_PREFS_H_
+
+namespace user_prefs {
+class PrefRegistrySyncable;
+}  // namespace user_prefs
+
+namespace browseros {
+
+// Registers the profile prefs in customization_pref_names.h.
+void RegisterCustomizationProfilePrefs(
+    user_prefs::PrefRegistrySyncable* registry);
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_CUSTOMIZATION_PREFS_H_
