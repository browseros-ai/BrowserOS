diff --git a/chrome/browser/browseros/extensions/browseros_extension_maintainer.h b/chrome/browser/browseros/extensions/browseros_extension_maintainer.h
new file mode 100644
index 0000000000000000000000000000000000000000..91309ee51786772b70a1576f9148f646844fddb6
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_maintainer.h
@@ -0,0 +1,74 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_MAINTAINER_H_
+#define CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_MAINTAINER_H_
+
+#include <memory>
+#include <optional>
+#include <string>
+#include <vector>
+
+#include "base/functional/callback.h"
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
+// Fetches bounded, validated remote provider metadata. The loader owns when to
+// recover/update and publishes the result through Chromium's external provider;
+// this helper never enqueues installs or independently schedules maintenance.
+// Local housekeeping runs only when requested by the same coordinator.
+class BrowserOSExtensionMaintainer {
+ public:
+  using UpdateCallback = base::OnceCallback<void(base::DictValue prefs)>;
+
+  explicit BrowserOSExtensionMaintainer(Profile* profile);
+  ~BrowserOSExtensionMaintainer();
+
+  BrowserOSExtensionMaintainer(const BrowserOSExtensionMaintainer&) = delete;
+  BrowserOSExtensionMaintainer& operator=(const BrowserOSExtensionMaintainer&) =
+      delete;
+
+  // Concurrent callers join the current request. An empty result indicates
+  // failure or no usable metadata; it must never be interpreted as readiness.
+  void CheckForUpdates(const GURL& config_url, UpdateCallback callback);
+
+  // Preserves periodic product housekeeping without a second install owner.
+  // The coordinator calls this after READY and on each maintenance cycle.
+  void MaintainInstalledExtensions();
+
+  // Drops pending replies as well as the request, allowing Retry to start
+  // fresh.
+  void Cancel();
+
+ private:
+  void OnConfigFetched(std::optional<std::string> response_body);
+  static base::DictValue ParseConfigJson(const std::string& json_content);
+  void Complete(base::DictValue prefs);
+  void UninstallInactiveProductExtensions();
+  void ReenableDisabledExtensions();
+  void LogExtensionHealth(const std::string& context);
+
+  raw_ptr<Profile> profile_;
+  std::vector<UpdateCallback> callbacks_;
+  std::unique_ptr<network::SimpleURLLoader> url_loader_;
+  scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory_;
+
+  base::WeakPtrFactory<BrowserOSExtensionMaintainer> weak_ptr_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_MAINTAINER_H_
