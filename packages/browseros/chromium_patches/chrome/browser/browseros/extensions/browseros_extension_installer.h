diff --git a/chrome/browser/browseros/extensions/browseros_extension_installer.h b/chrome/browser/browseros/extensions/browseros_extension_installer.h
new file mode 100644
index 0000000000000000000000000000000000000000..8e742db3257743875031450614cb0ea831779ad5
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_installer.h
@@ -0,0 +1,61 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_INSTALLER_H_
+#define CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_INSTALLER_H_
+
+#include <string>
+#include <vector>
+
+#include "base/files/file_path.h"
+#include "base/functional/callback.h"
+#include "base/memory/weak_ptr.h"
+#include "base/values.h"
+
+namespace browseros {
+
+// Local provider metadata, not an installation result. Valid entries survive a
+// partial bundle so the coordinator can publish them and retain them for Retry.
+struct BundleDiscoveryResult {
+  BundleDiscoveryResult();
+  ~BundleDiscoveryResult();
+  BundleDiscoveryResult(BundleDiscoveryResult&&);
+  BundleDiscoveryResult& operator=(BundleDiscoveryResult&&);
+
+  base::DictValue prefs;
+  base::FilePath bundled_path;
+  bool complete = false;
+};
+
+// Discovers local CRXs for Chromium's external provider. This helper never
+// installs or fetches anything: provider readiness must not depend on network,
+// and the loader separately observes the primary extension's registry READY.
+class BrowserOSExtensionInstaller {
+ public:
+  using DiscoveryCallback =
+      base::OnceCallback<void(BundleDiscoveryResult result)>;
+
+  BrowserOSExtensionInstaller();
+  ~BrowserOSExtensionInstaller();
+
+  BrowserOSExtensionInstaller(const BrowserOSExtensionInstaller&) = delete;
+  BrowserOSExtensionInstaller& operator=(const BrowserOSExtensionInstaller&) =
+      delete;
+
+  // Replies on the calling UI sequence, including for an absent/invalid bundle.
+  // Destruction cancels the reply; it does not interrupt the bounded file read.
+  void DiscoverBundled(DiscoveryCallback callback);
+
+ private:
+  static BundleDiscoveryResult ReadBundledManifest(
+      std::vector<std::string> active_ids);
+  void OnBundledLoadComplete(DiscoveryCallback callback,
+                             BundleDiscoveryResult result);
+
+  base::WeakPtrFactory<BrowserOSExtensionInstaller> weak_ptr_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_INSTALLER_H_
