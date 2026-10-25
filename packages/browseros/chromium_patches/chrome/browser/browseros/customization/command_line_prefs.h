diff --git a/chrome/browser/browseros/customization/command_line_prefs.h b/chrome/browser/browseros/customization/command_line_prefs.h
new file mode 100644
index 0000000000000000000000000000000000000000..02b783c4ab27e0718b87d233f5eeec5c5f39b36f
--- /dev/null
+++ b/chrome/browser/browseros/customization/command_line_prefs.h
@@ -0,0 +1,40 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_COMMAND_LINE_PREFS_H_
+#define CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_COMMAND_LINE_PREFS_H_
+
+class WriteablePrefStore;
+
+namespace base {
+class CommandLine;
+}  // namespace base
+
+namespace browseros {
+
+namespace switches {
+
+// Turns BrowserOS UI pieces off for one run, e.g.
+// --disable-browseros-features=VerticalTabs,HomeButton. Same list syntax as
+// --disable-features. The names and the prefs each one forces are listed in
+// command_line_prefs.cc; unknown names are logged and skipped.
+inline constexpr char kDisableBrowserOSFeatures[] =
+    "disable-browseros-features";
+
+}  // namespace switches
+
+// BrowserOS's share of Chromium's command-line pref store: turns BrowserOS
+// switches into pref values in `store`, which must be that store.
+//
+// Values set there beat the user's saved values, lose to policy and
+// extensions, are never written to disk, and make their prefs read as not
+// user-modifiable, which is what locks the settings and commands bound to
+// them. Chromium builds the store once for local state and once per profile;
+// each PrefService ignores values for prefs it never registered.
+void ApplyCommandLinePrefs(const base::CommandLine& command_line,
+                           WriteablePrefStore& store);
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_CUSTOMIZATION_COMMAND_LINE_PREFS_H_
