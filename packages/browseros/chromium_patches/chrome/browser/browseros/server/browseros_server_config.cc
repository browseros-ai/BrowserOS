diff --git a/chrome/browser/browseros/server/browseros_server_config.cc b/chrome/browser/browseros/server/browseros_server_config.cc
new file mode 100644
index 0000000000000..6fe71062a9c96
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_config.cc
@@ -0,0 +1,178 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+
+#include "base/strings/stringprintf.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+
+namespace browseros {
+namespace {
+
+constexpr ManagedServerDescriptor kBrowserOSServerDescriptor = {
+    Product::kBrowserOS,
+    "BrowserOS server",
+    FILE_PATH_LITERAL("BrowserOSServer"),
+    FILE_PATH_LITERAL("browseros_server"),
+    FILE_PATH_LITERAL("config.json"),
+    "/system/health",
+    true,
+    {
+        // Empty state dir keeps the legacy .browseros/current_version layout.
+        FILE_PATH_LITERAL(""),
+        "https://cdn.browseros.com/appcast-server.xml",
+        "https://cdn.browseros.com/appcast-server.alpha.xml",
+        "/status",
+    },
+};
+
+constexpr ManagedServerDescriptor kBrowserClawServerDescriptor = {
+    Product::kBrowserClaw,
+    "BrowserClaw server",
+    FILE_PATH_LITERAL("BrowserClawServer"),
+    FILE_PATH_LITERAL("browseros-claw-server"),
+    FILE_PATH_LITERAL("config.json"),
+    "/system/health",
+    true,
+    {
+        // Isolate Claw OTA state under .browseros/BrowserClawServer/.
+        FILE_PATH_LITERAL("BrowserClawServer"),
+        "https://cdn.browseros.com/appcast-claw-server.xml",
+        "https://cdn.browseros.com/appcast-claw-server.alpha.xml",
+        // Claw does not expose the {can_update} readiness contract yet; empty
+        // skips readiness and restarts after verification.
+        "",
+    },
+};
+
+}  // namespace
+
+const ManagedServerDescriptor& GetBrowserOSServerDescriptor() {
+  return kBrowserOSServerDescriptor;
+}
+
+const ManagedServerDescriptor& GetBrowserClawServerDescriptor() {
+  return kBrowserClawServerDescriptor;
+}
+
+const ManagedServerDescriptor& GetManagedServerDescriptor() {
+  if (IsBrowserClawProduct()) {
+    return GetBrowserClawServerDescriptor();
+  }
+  return GetBrowserOSServerDescriptor();
+}
+
+// Single config.json shape shared by every managed server product. Any
+// product-specific server binary (e.g. the Claw server) must tolerate the full
+// key set below; it must not reject unknown keys.
+base::DictValue BuildServerConfigJson(
+    const ServerLaunchConfig& config,
+    const base::FilePath& actual_resources_dir) {
+  base::DictValue root;
+
+  base::DictValue ports_dict;
+  ports_dict.Set("cdp", config.ports.cdp);
+  ports_dict.Set("server", config.ports.server);
+  ports_dict.Set("proxy", config.ports.proxy);
+  root.Set("ports", std::move(ports_dict));
+
+  base::DictValue directories;
+  directories.Set("resources", actual_resources_dir.AsUTF8Unsafe());
+  directories.Set("execution", config.paths.execution.AsUTF8Unsafe());
+  root.Set("directories", std::move(directories));
+
+  base::DictValue flags;
+  flags.Set("allow_remote_in_mcp", config.allow_remote_in_mcp);
+  root.Set("flags", std::move(flags));
+
+  base::DictValue instance;
+  instance.Set("install_id", config.identity.install_id);
+  instance.Set("browseros_version", config.identity.browseros_version);
+  instance.Set("chromium_version", config.identity.chromium_version);
+  root.Set("instance", std::move(instance));
+
+  return root;
+}
+
+bool ServerPorts::IsValid() const {
+  return cdp > 0 && server > 0 && proxy > 0;
+}
+
+std::string ServerPorts::DebugString() const {
+  return base::StringPrintf(
+      "ServerPorts{\n"
+      "  cdp=%d\n"
+      "  server=%d\n"
+      "  proxy=%d\n"
+      "}",
+      cdp, server, proxy);
+}
+
+ServerPaths::ServerPaths() = default;
+ServerPaths::ServerPaths(const ServerPaths&) = default;
+ServerPaths& ServerPaths::operator=(const ServerPaths&) = default;
+ServerPaths::ServerPaths(ServerPaths&&) = default;
+ServerPaths& ServerPaths::operator=(ServerPaths&&) = default;
+ServerPaths::~ServerPaths() = default;
+
+bool ServerPaths::IsValid() const {
+  return !exe.empty() && !execution.empty();
+}
+
+std::string ServerPaths::DebugString() const {
+  return base::StringPrintf(
+      "ServerPaths{\n"
+      "  exe=%s\n"
+      "  fallback=%s\n"
+      "  resources=%s\n"
+      "  execution=%s\n"
+      "}",
+      exe.AsUTF8Unsafe().c_str(), fallback_exe.AsUTF8Unsafe().c_str(),
+      resources.AsUTF8Unsafe().c_str(), execution.AsUTF8Unsafe().c_str());
+}
+
+std::string ServerIdentity::DebugString() const {
+  return base::StringPrintf(
+      "ServerIdentity{\n"
+      "  install_id=%s\n"
+      "  browseros=%s\n"
+      "  chromium=%s\n"
+      "}",
+      install_id.c_str(), browseros_version.c_str(), chromium_version.c_str());
+}
+
+ServerLaunchConfig::ServerLaunchConfig() = default;
+ServerLaunchConfig::ServerLaunchConfig(const ServerLaunchConfig&) = default;
+ServerLaunchConfig& ServerLaunchConfig::operator=(const ServerLaunchConfig&) =
+    default;
+ServerLaunchConfig::ServerLaunchConfig(ServerLaunchConfig&&) = default;
+ServerLaunchConfig& ServerLaunchConfig::operator=(ServerLaunchConfig&&) =
+    default;
+ServerLaunchConfig::~ServerLaunchConfig() = default;
+
+bool ServerLaunchConfig::IsValid() const {
+  return ports.IsValid() && paths.IsValid() && !config_file_name.empty() &&
+         !health_path.empty();
+}
+
+std::string ServerLaunchConfig::DebugString() const {
+  std::string config_file = base::FilePath(config_file_name).AsUTF8Unsafe();
+  return base::StringPrintf(
+      "ServerLaunchConfig{\n"
+      "  log_name=%s\n"
+      "  config_file=%s\n"
+      "  health_path=%s\n"
+      "  enable_updater=%s\n"
+      "  %s\n"
+      "  %s\n"
+      "  %s\n"
+      "  allow_remote=%s\n"
+      "}",
+      log_name.c_str(), config_file.c_str(), health_path.c_str(),
+      enable_updater ? "true" : "false", ports.DebugString().c_str(),
+      paths.DebugString().c_str(), identity.DebugString().c_str(),
+      allow_remote_in_mcp ? "true" : "false");
+}
+
+}  // namespace browseros
