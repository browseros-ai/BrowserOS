diff --git a/chrome/browser/browseros/core/browseros_product_state.cc b/chrome/browser/browseros/core/browseros_product_state.cc
new file mode 100644
index 0000000000000000000000000000000000000000..2e72060f85302d065b2baa91d620fa1296052b31
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_product_state.cc
@@ -0,0 +1,66 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/core/browseros_product_state.h"
+
+#include <optional>
+#include <string>
+#include <string_view>
+
+#include "base/base_paths.h"
+#include "base/environment.h"
+#include "base/logging.h"
+#include "base/path_service.h"
+#include "base/strings/string_util.h"
+
+namespace browseros {
+namespace {
+
+constexpr base::cstring_view kBrowserOSDirectoryEnvironment = "BROWSEROS_DIR";
+constexpr base::cstring_view kBrowserClawDirectoryEnvironment =
+    "BROWSERCLAW_DIR";
+constexpr base::FilePath::StringViewType kBrowserOSDirectoryName =
+    FILE_PATH_LITERAL(".browseros");
+constexpr base::FilePath::StringViewType kBrowserClawDirectoryName =
+    FILE_PATH_LITERAL(".browserclaw");
+
+base::cstring_view GetDirectoryEnvironmentName(Product product) {
+  return product == Product::kBrowserClaw ? kBrowserClawDirectoryEnvironment
+                                          : kBrowserOSDirectoryEnvironment;
+}
+
+base::FilePath::StringViewType GetDefaultDirectoryName(Product product) {
+  return product == Product::kBrowserClaw ? kBrowserClawDirectoryName
+                                          : kBrowserOSDirectoryName;
+}
+
+}  // namespace
+
+base::FilePath GetProductStateDirectory(Product product) {
+  const base::cstring_view environment_name =
+      GetDirectoryEnvironmentName(product);
+  std::optional<std::string> override =
+      base::Environment::Create()->GetVar(environment_name);
+  if (override.has_value()) {
+    const std::string_view trimmed =
+        base::TrimWhitespaceASCII(*override, base::TRIM_ALL);
+    if (!trimmed.empty()) {
+      base::FilePath path = base::FilePath::FromUTF8Unsafe(trimmed);
+      if (path.IsAbsolute()) {
+        return path;
+      }
+      LOG(WARNING) << "browseros: Ignoring relative "
+                   << environment_name.c_str() << " override: " << path;
+    }
+  }
+
+  base::FilePath home;
+  if (!base::PathService::Get(base::DIR_HOME, &home)) {
+    LOG(ERROR) << "browseros: Failed to resolve the product state root";
+    return base::FilePath();
+  }
+  return home.Append(GetDefaultDirectoryName(product));
+}
+
+}  // namespace browseros
