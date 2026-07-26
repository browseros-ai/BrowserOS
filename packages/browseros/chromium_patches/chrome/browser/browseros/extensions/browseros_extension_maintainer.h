diff --git a/chrome/browser/browseros/extensions/browseros_extension_maintainer.h b/chrome/browser/browseros/extensions/browseros_extension_maintainer.h
new file mode 100644
index 0000000000000..c70cbd4076100
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_maintainer.h
@@ -0,0 +1,69 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_MAINTAINER_H_
+#define CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_MAINTAINER_H_
+
+#include <memory>
+#include <optional>
+#include <set>
+#include <string>
+
+#include "base/memory/raw_ptr.h"
+#include "base/memory/scoped_refptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/values.h"
+#include "url/gurl.h"
+
+namespace network {
+class SharedURLLoaderFactory;
+class SimpleURLLoader;
+}  // namespace network
+
+class Profile;
+
+namespace browseros {
+
+class BrowserOSExtensionMaintainer {
+ public:
+  explicit BrowserOSExtensionMaintainer(Profile* profile);
+  ~BrowserOSExtensionMaintainer();
+
+  BrowserOSExtensionMaintainer(const BrowserOSExtensionMaintainer&) = delete;
+  BrowserOSExtensionMaintainer& operator=(const BrowserOSExtensionMaintainer&) =
+      delete;
+
+  void Start(const GURL& config_url,
+             std::set<std::string> extension_ids,
+             base::DictValue initial_config);
+
+  void UpdateExtensionIds(std::set<std::string> ids);
+
+ private:
+  void RunMaintenanceCycle();
+  void OnConfigFetched(std::unique_ptr<network::SimpleURLLoader> loader,
+                       std::optional<std::string> response_body);
+  base::DictValue ParseConfigJson(const std::string& json_content);
+  void ExecuteMaintenanceTasks();
+  void ScheduleNextMaintenance();
+  void UninstallInactiveProductExtensions();
+  void UninstallDeprecatedExtensions();
+  void ReinstallMissingExtensions();
+  void ReenableDisabledExtensions();
+  void ForceUpdateCheck();
+  void LogExtensionHealth(const std::string& context);
+
+  raw_ptr<Profile> profile_;
+  GURL config_url_;
+  std::set<std::string> extension_ids_;
+  base::DictValue last_config_;
+
+  scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory_;
+
+  base::WeakPtrFactory<BrowserOSExtensionMaintainer> weak_ptr_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_MAINTAINER_H_
