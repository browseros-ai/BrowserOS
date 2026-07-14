diff --git a/chrome/browser/browseros/server/browseros_server_prefs.h b/chrome/browser/browseros/server/browseros_server_prefs.h
new file mode 100644
index 0000000000000..4e88e044d184d
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_prefs.h
@@ -0,0 +1,45 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PREFS_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PREFS_H_
+
+#include "chrome/browser/browseros/buildflags.h"
+
+class PrefRegistrySimple;
+
+namespace browseros_server {
+
+// Default ports for the baked server product.
+#if BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)
+inline constexpr int kDefaultCDPPort = 9110;
+inline constexpr int kDefaultProxyPort = 9010;
+inline constexpr int kDefaultProxyHttpsPort = 9011;
+inline constexpr int kDefaultServerPort = 9210;
+#else
+inline constexpr int kDefaultCDPPort = 9100;
+inline constexpr int kDefaultProxyPort = 9000;
+inline constexpr int kDefaultProxyHttpsPort = 9001;
+inline constexpr int kDefaultServerPort = 9200;
+#endif
+
+// Preference keys for BrowserOS server configuration
+extern const char kCDPServerPort[];
+extern const char kProxyPort[];
+extern const char kProxyHttpsPort[];
+extern const char kServerPort[];
+extern const char kAllowRemoteInMCP[];
+extern const char kRestartServerRequested[];
+extern const char kServerVersion[];
+
+// Deprecated prefs (kept for migration, will be removed in future)
+extern const char kMCPServerPort[];     // DEPRECATED: migrated to kProxyPort
+extern const char kMCPServerEnabled[];  // DEPRECATED: no longer used
+
+// Registers BrowserOS server preferences in Local State (browser-wide prefs)
+void RegisterLocalStatePrefs(PrefRegistrySimple* registry);
+
+}  // namespace browseros_server
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PREFS_H_
