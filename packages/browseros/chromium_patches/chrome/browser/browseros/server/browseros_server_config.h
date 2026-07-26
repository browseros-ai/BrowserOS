diff --git a/chrome/browser/browseros/server/browseros_server_config.h b/chrome/browser/browseros/server/browseros_server_config.h
new file mode 100644
index 0000000000000..a98e3b74de45e
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_config.h
@@ -0,0 +1,147 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_CONFIG_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_CONFIG_H_
+
+#include <string>
+#include <string_view>
+
+#include "base/files/file_path.h"
+#include "base/values.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+
+namespace browseros {
+
+struct ServerLaunchConfig;
+
+struct ManagedServerDescriptor {
+  // OTA contract consumed by BrowserOSServerUpdater so a single updater can
+  // serve every sidecar product. Grouped into a nested struct to keep the
+  // descriptor under the chromium-style implicit-ctor weight limit (>=10
+  // trivial fields would force an out-of-line ctor, which a constexpr
+  // aggregate cannot have).
+  struct UpdaterConfig {
+    // Subdirectory under .browseros holding this product's OTA state
+    // (current_version file + versions/). Empty keeps the legacy BrowserOS
+    // layout directly under .browseros for backward compatibility.
+    base::FilePath::StringViewType state_dir;
+    // Stable and alpha appcast feeds.
+    std::string_view appcast_url;
+    std::string_view alpha_appcast_url;
+    // Server endpoint returning {can_update} that gates a hot-swap. Empty skips
+    // the readiness fetch and restarts right after verification.
+    std::string_view readiness_path;
+  };
+
+  Product product;
+  std::string_view log_name;
+  base::FilePath::StringViewType bundle_dir;
+  base::FilePath::StringViewType binary_name;
+  base::FilePath::StringViewType config_file_name;
+  std::string_view health_path;
+  // Whether the manager starts an updater for this product at all.
+  bool enable_updater;
+  UpdaterConfig updater;
+};
+
+const ManagedServerDescriptor& GetBrowserOSServerDescriptor();
+const ManagedServerDescriptor& GetBrowserClawServerDescriptor();
+
+// Returns the descriptor selected by browseros::GetProduct().
+const ManagedServerDescriptor& GetManagedServerDescriptor();
+
+// Builds the JSON config expected by the selected server product.
+base::DictValue BuildServerConfigJson(
+    const ServerLaunchConfig& config,
+    const base::FilePath& actual_resources_dir);
+
+// Port assignments for all server endpoints.
+// This is the single source of truth for port configuration.
+struct ServerPorts {
+  int cdp = 0;
+  int server = 0;  // ephemeral backend port for sidecar (was "mcp")
+  int proxy = 0;   // stable MCP proxy port bound by Chromium
+
+  bool operator==(const ServerPorts&) const = default;
+
+  // Returns true if all ports are assigned (non-zero).
+  bool IsValid() const;
+
+  // Returns a debug string for logging.
+  std::string DebugString() const;
+};
+
+// Filesystem paths needed to launch the server.
+// Computed fresh before each launch since the updater can change paths.
+struct ServerPaths {
+  ServerPaths();
+  ServerPaths(const ServerPaths&);
+  ServerPaths& operator=(const ServerPaths&);
+  ServerPaths(ServerPaths&&);
+  ServerPaths& operator=(ServerPaths&&);
+  ~ServerPaths();
+
+  // Primary binary path (may be OTA-updated version).
+  base::FilePath exe;
+
+  // Bundled binary path (always available as fallback).
+  base::FilePath fallback_exe;
+
+  // Primary resources directory.
+  base::FilePath resources;
+
+  // Bundled resources directory (fallback).
+  base::FilePath fallback_resources;
+
+  // Runtime data directory (~/.browseros or equivalent).
+  base::FilePath execution;
+
+  // Returns true if required paths are set.
+  bool IsValid() const;
+
+  // Returns a debug string for logging.
+  std::string DebugString() const;
+};
+
+// Identity and versioning info written to the server config JSON.
+struct ServerIdentity {
+  std::string install_id;
+  std::string browseros_version;
+  std::string chromium_version;
+
+  // Returns a debug string for logging.
+  std::string DebugString() const;
+};
+
+// Complete configuration for a single server launch.
+// Assembled fresh before each ProcessController::Launch() call.
+struct ServerLaunchConfig {
+  ServerLaunchConfig();
+  ServerLaunchConfig(const ServerLaunchConfig&);
+  ServerLaunchConfig& operator=(const ServerLaunchConfig&);
+  ServerLaunchConfig(ServerLaunchConfig&&);
+  ServerLaunchConfig& operator=(ServerLaunchConfig&&);
+  ~ServerLaunchConfig();
+
+  std::string log_name;
+  base::FilePath::StringType config_file_name;
+  std::string health_path;
+  bool enable_updater = true;
+
+  ServerPorts ports;
+  ServerPaths paths;
+  ServerIdentity identity;
+  bool allow_remote_in_mcp = false;
+
+  // Returns true if the config is valid for launching.
+  bool IsValid() const;
+
+  // Returns a debug string for logging.
+  std::string DebugString() const;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_CONFIG_H_
