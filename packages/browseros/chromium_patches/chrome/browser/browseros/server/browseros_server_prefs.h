diff --git a/chrome/browser/browseros/server/browseros_server_prefs.h b/chrome/browser/browseros/server/browseros_server_prefs.h
new file mode 100644
index 0000000000000..8e1bf54357cf9
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_prefs.h
@@ -0,0 +1,34 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PREFS_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PREFS_H_
+
+class PrefRegistrySimple;
+
+namespace browseros_server {
+
+// Default port values for BrowserOS server (10-port spacing)
+inline constexpr int kDefaultCDPPort = 9000;
+inline constexpr int kDefaultMCPPort = 9100;
+inline constexpr int kDefaultAgentPort = 9200;
+inline constexpr int kDefaultExtensionPort = 9300;
+
+// Preference keys for BrowserOS server configuration
+extern const char kCDPServerPort[];
+extern const char kMCPServerPort[];
+extern const char kAgentServerPort[];
+extern const char kExtensionServerPort[];
+extern const char kAllowRemoteInMCP[];
+extern const char kRestartServerRequested[];
+
+// Deprecated prefs (kept for migration, will be removed in future)
+extern const char kMCPServerEnabled[];  // DEPRECATED: no longer used
+
+// Registers BrowserOS server preferences in Local State (browser-wide prefs)
+void RegisterLocalStatePrefs(PrefRegistrySimple* registry);
+
+}  // namespace browseros_server
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PREFS_H_
