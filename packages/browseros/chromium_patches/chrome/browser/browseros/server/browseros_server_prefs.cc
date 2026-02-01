diff --git a/chrome/browser/browseros/server/browseros_server_prefs.cc b/chrome/browser/browseros/server/browseros_server_prefs.cc
new file mode 100644
index 0000000000000..8c3a459477837
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_prefs.cc
@@ -0,0 +1,49 @@
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
+// CDP server port
+const char kCDPServerPort[] = "browseros.server.cdp_port";
+
+// Stable MCP proxy port
+const char kProxyPort[] = "browseros.server.proxy_port";
+
+// Sidecar backend server port
+const char kServerPort[] = "browseros.server.server_port";
+
+// Extension WebSocket server port
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
+// DEPRECATED prefs (kept for migration)
+const char kMCPServerPort[] = "browseros.server.mcp_port";
+const char kMCPServerEnabled[] = "browseros.server.mcp_enabled";
+
+void RegisterLocalStatePrefs(PrefRegistrySimple* registry) {
+  registry->RegisterIntegerPref(kCDPServerPort, kDefaultCDPPort);
+  registry->RegisterIntegerPref(kProxyPort, kDefaultProxyPort);
+  registry->RegisterIntegerPref(kServerPort, kDefaultServerPort);
+  registry->RegisterIntegerPref(kExtensionServerPort, kDefaultExtensionPort);
+  registry->RegisterBooleanPref(kAllowRemoteInMCP, false);
+  registry->RegisterBooleanPref(kRestartServerRequested, false);
+  registry->RegisterStringPref(kServerVersion, std::string());
+
+  // Deprecated prefs: register for migration reads
+  registry->RegisterIntegerPref(kMCPServerPort, 0);
+}
+
+}  // namespace browseros_server
