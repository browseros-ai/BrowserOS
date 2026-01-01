diff --git a/chrome/browser/browseros/extensions/browseros_extension_installer.h b/chrome/browser/browseros/extensions/browseros_extension_installer.h
new file mode 100644
index 0000000000000..7502da6d31ff5
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_installer.h
@@ -0,0 +1,99 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_INSTALLER_H_
+#define CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_INSTALLER_H_
+
+#include <memory>
+#include <set>
+#include <string>
+
+#include "base/files/file_path.h"
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
+// Result of initial extension installation.
+struct InstallResult {
+  InstallResult();
+  ~InstallResult();
+  InstallResult(InstallResult&&);
+  InstallResult& operator=(InstallResult&&);
+
+  base::Value::Dict prefs;           // Extension prefs for ExternalProviderImpl
+  base::Value::Dict config;          // Raw config for maintenance
+  std::set<std::string> extension_ids;
+  base::FilePath bundled_path;       // Set if loaded from bundled
+  bool from_bundled = false;
+};
+
+// Handles one-time initial installation of BrowserOS extensions.
+// Tries bundled CRX files first, falls back to remote config.
+class BrowserOSExtensionInstaller {
+ public:
+  using InstallCompleteCallback =
+      base::OnceCallback<void(InstallResult result)>;
+
+  explicit BrowserOSExtensionInstaller(Profile* profile);
+  ~BrowserOSExtensionInstaller();
+
+  BrowserOSExtensionInstaller(const BrowserOSExtensionInstaller&) = delete;
+  BrowserOSExtensionInstaller& operator=(const BrowserOSExtensionInstaller&) =
+      delete;
+
+  // Starts the installation process. Calls |callback| when complete.
+  void StartInstallation(const GURL& config_url,
+                         InstallCompleteCallback callback);
+
+ private:
+  // Attempts to load from bundled CRX files. Returns true if attempting.
+  bool TryLoadFromBundled();
+
+  // Reads bundled manifest on FILE thread.
+  static base::Value::Dict ReadBundledManifest(
+      const base::FilePath& manifest_path,
+      const base::FilePath& bundled_path);
+
+  // Called when bundled manifest read completes.
+  void OnBundledLoadComplete(const base::FilePath& bundled_path,
+                             base::Value::Dict prefs);
+
+  // Fetches config from remote URL.
+  void FetchFromRemote();
+
+  // Called when remote fetch completes.
+  void OnRemoteFetchComplete(std::unique_ptr<std::string> response_body);
+
+  // Parses config JSON and returns extensions dict.
+  base::Value::Dict ParseConfigJson(const std::string& json_content);
+
+  // Completes the installation with the given result.
+  void Complete(InstallResult result);
+
+  raw_ptr<Profile> profile_;
+  GURL config_url_;
+  InstallCompleteCallback callback_;
+  std::set<std::string> extension_ids_;
+
+  std::unique_ptr<network::SimpleURLLoader> url_loader_;
+  scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory_;
+
+  base::WeakPtrFactory<BrowserOSExtensionInstaller> weak_ptr_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_INSTALLER_H_
