diff --git a/chrome/browser/browseros/server/browseros_server_prefs.cc b/chrome/browser/browseros/server/browseros_server_prefs.cc
new file mode 100644
index 0000000000000..631c25fe80c97
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_prefs.cc
@@ -0,0 +1,52 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
+
+#include "components/prefs/pref_registry_simple.h"
+
+namespace browseros_server {
+
+// CDP server port (0 = auto-assign random port on startup)
+const char kCDPServerPort[] = "browseros.server.cdp_port";
+
+// MCP server port (HTTP)
+const char kMCPServerPort[] = "browseros.server.mcp_port";
+
+// Extension server port
+const char kExtensionServerPort[] = "browseros.server.extension_port";
+
+// Allow remote connections to MCP server (security setting)
+const char kAllowRemoteInMCP[] = "browseros.server.allow_remote_in_mcp";
+
+// Whether server restart has been requested (auto-reset after restart)
+const char kRestartServerRequested[] = "browseros.server.restart_requested";
+
+// Current active browseros-server version (for observability)
+const char kServerVersion[] = "browseros.server.version";
+
+// DEPRECATED: kept for migration, no longer used
+const char kMCPServerEnabled[] = "browseros.server.mcp_enabled";
+
+void RegisterLocalStatePrefs(PrefRegistrySimple* registry) {
+  // CDP port
+  registry->RegisterIntegerPref(kCDPServerPort, kDefaultCDPPort);
+
+  // MCP port
+  registry->RegisterIntegerPref(kMCPServerPort, kDefaultMCPPort);
+
+  // Extension port
+  registry->RegisterIntegerPref(kExtensionServerPort, kDefaultExtensionPort);
+
+  // Allow remote MCP connections (default: false for security)
+  registry->RegisterBooleanPref(kAllowRemoteInMCP, false);
+
+  // Restart requested (default false, auto-reset after restart)
+  registry->RegisterBooleanPref(kRestartServerRequested, false);
+
+  // Current server version (empty = unknown/bundled)
+  registry->RegisterStringPref(kServerVersion, std::string());
+}
+
+}  // namespace browseros_server
