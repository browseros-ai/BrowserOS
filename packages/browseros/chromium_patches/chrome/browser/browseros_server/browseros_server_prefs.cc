diff --git a/chrome/browser/blockbrowser_server/blockbrowser_server_prefs.cc b/chrome/browser/blockbrowser_server/blockbrowser_server_prefs.cc
new file mode 100644
index 0000000000000..f9c7a9990cb01
--- /dev/null
+++ b/chrome/browser/blockbrowser_server/blockbrowser_server_prefs.cc
@@ -0,0 +1,49 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/blockbrowser_server/blockbrowser_server_prefs.h"
+
+#include "components/prefs/pref_registry_simple.h"
+
+namespace blockbrowser_server {
+
+// CDP server port (0 = auto-assign random port on startup)
+const char kCDPServerPort[] = "browseros.server.cdp_port";
+
+// MCP server port (HTTP)
+const char kMCPServerPort[] = "browseros.server.mcp_port";
+
+// Agent server port
+const char kAgentServerPort[] = "browseros.server.agent_port";
+
+// Extension server port
+const char kExtensionServerPort[] = "browseros.server.extension_port";
+
+// Whether MCP server is enabled
+const char kMCPServerEnabled[] = "browseros.server.mcp_enabled";
+
+// Whether server restart has been requested (auto-reset after restart)
+const char kRestartServerRequested[] = "browseros.server.restart_requested";
+
+void RegisterLocalStatePrefs(PrefRegistrySimple* registry) {
+  // CDP port
+  registry->RegisterIntegerPref(kCDPServerPort, kDefaultCDPPort);
+
+  // MCP port
+  registry->RegisterIntegerPref(kMCPServerPort, kDefaultMCPPort);
+
+  // Agent port
+  registry->RegisterIntegerPref(kAgentServerPort, kDefaultAgentPort);
+
+  // Extension port
+  registry->RegisterIntegerPref(kExtensionServerPort, kDefaultExtensionPort);
+
+  // MCP enabled
+  registry->RegisterBooleanPref(kMCPServerEnabled, true);
+
+  // Restart requested (default false, auto-reset after restart)
+  registry->RegisterBooleanPref(kRestartServerRequested, false);
+}
+
+}  // namespace blockbrowser_server
