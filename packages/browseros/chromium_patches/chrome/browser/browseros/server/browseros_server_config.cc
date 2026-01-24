diff --git a/chrome/browser/browseros/server/browseros_server_config.cc b/chrome/browser/browseros/server/browseros_server_config.cc
new file mode 100644
index 0000000000000..c8e53682b97e3
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_config.cc
@@ -0,0 +1,80 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+
+#include "base/strings/stringprintf.h"
+
+namespace browseros {
+
+bool ServerPorts::IsValid() const {
+  return cdp > 0 && mcp > 0 && extension > 0;
+}
+
+std::string ServerPorts::DebugString() const {
+  return base::StringPrintf(
+      "ServerPorts{\n"
+      "  cdp=%d\n"
+      "  mcp=%d\n"
+      "  ext=%d\n"
+      "}",
+      cdp, mcp, extension);
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
+      exe.AsUTF8Unsafe().c_str(),
+      fallback_exe.AsUTF8Unsafe().c_str(),
+      resources.AsUTF8Unsafe().c_str(),
+      execution.AsUTF8Unsafe().c_str());
+}
+
+std::string ServerIdentity::DebugString() const {
+  return base::StringPrintf(
+      "ServerIdentity{\n"
+      "  install_id=%s\n"
+      "  browseros=%s\n"
+      "  chromium=%s\n"
+      "}",
+      install_id.c_str(),
+      browseros_version.c_str(),
+      chromium_version.c_str());
+}
+
+bool ServerLaunchConfig::IsValid() const {
+  return ports.IsValid() && paths.IsValid();
+}
+
+std::string ServerLaunchConfig::DebugString() const {
+  return base::StringPrintf(
+      "ServerLaunchConfig{\n"
+      "  %s\n"
+      "  %s\n"
+      "  %s\n"
+      "  allow_remote=%s\n"
+      "}",
+      ports.DebugString().c_str(),
+      paths.DebugString().c_str(),
+      identity.DebugString().c_str(),
+      allow_remote_in_mcp ? "true" : "false");
+}
+
+}  // namespace browseros
