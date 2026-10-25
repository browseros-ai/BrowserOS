diff --git a/chrome/browser/browseros/customization/customization_prefs.cc b/chrome/browser/browseros/customization/customization_prefs.cc
new file mode 100644
index 0000000000000000000000000000000000000000..4908509187b468adbcdfb7e565b600404b8993ff
--- /dev/null
+++ b/chrome/browser/browseros/customization/customization_prefs.cc
@@ -0,0 +1,18 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/customization/customization_prefs.h"
+
+#include "chrome/browser/browseros/customization/customization_pref_names.h"
+#include "components/pref_registry/pref_registry_syncable.h"
+
+namespace browseros {
+
+void RegisterCustomizationProfilePrefs(
+    user_prefs::PrefRegistrySyncable* registry) {
+  registry->RegisterBooleanPref(prefs::kShowAvatarButton, true);
+  registry->RegisterBooleanPref(prefs::kShowExtensionsToolbar, true);
+}
+
+}  // namespace browseros
