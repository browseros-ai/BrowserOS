diff --git a/chrome/browser/browseros/customization/customization_pref_names.h b/chrome/browser/browseros/customization/customization_pref_names.h
new file mode 100644
index 0000000000000000000000000000000000000000..ca13b8c417f11266b03602e8a379c1c1aff4688d
--- /dev/null
+++ b/chrome/browser/browseros/customization/customization_pref_names.h
@@ -0,0 +1,25 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_CUSTOMIZATION_PREF_NAMES_H_
+#define CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_CUSTOMIZATION_PREF_NAMES_H_
+
+// BrowserOS-owned knobs for UI that Chromium gives no pref of its own. UI code
+// reads these prefs and never the switch that can force them
+// (--disable-browseros-features), so policy, settings and the switch all agree.
+// The toolbar reads them once, when a window's toolbar is built.
+namespace browseros::prefs {
+
+// Boolean profile pref: show the profile avatar button in the toolbar.
+inline constexpr char kShowAvatarButton[] = "browseros.show_avatar_button";
+
+// Boolean profile pref: keep the toolbar's extensions area (the extensions menu
+// button and pinned extension icons) on screen. When off, the area appears
+// only while an extension popup or the extensions menu needs it.
+inline constexpr char kShowExtensionsToolbar[] =
+    "browseros.show_extensions_toolbar";
+
+}  // namespace browseros::prefs
+
+#endif  // CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_CUSTOMIZATION_PREF_NAMES_H_
