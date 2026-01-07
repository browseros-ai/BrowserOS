diff --git a/chrome/browser/browseros/extensions/browseros_extension_loader.h b/chrome/browser/browseros/extensions/browseros_extension_loader.h
new file mode 100644
index 0000000000000..c0b26b369ad07
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_loader.h
@@ -0,0 +1,81 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_LOADER_H_
+#define CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_LOADER_H_
+
+#include <memory>
+#include <set>
+#include <string>
+
+#include "base/files/file_path.h"
+#include "base/memory/weak_ptr.h"
+#include "chrome/browser/browseros/extensions/browseros_extension_installer.h"
+#include "chrome/browser/browseros/extensions/browseros_extension_maintainer.h"
+#include "chrome/browser/extensions/external_loader.h"
+#include "url/gurl.h"
+
+class Profile;
+
+namespace browseros {
+
+// Loads BrowserOS extensions from bundled CRX files or remote configuration.
+//
+// Lifecycle:
+//   1. STARTUP: Installer loads from bundled CRX (preferred) or remote
+//   2. POST-STARTUP: Both paths converge to start maintenance
+//   3. MAINTENANCE: Periodic tasks via Maintainer
+//
+// After startup, extensions receive updates via their manifest.json update_url,
+// triggered by ForceUpdateCheck() during maintenance.
+class BrowserOSExtensionLoader : public extensions::ExternalLoader {
+ public:
+  explicit BrowserOSExtensionLoader(Profile* profile);
+
+  BrowserOSExtensionLoader(const BrowserOSExtensionLoader&) = delete;
+  BrowserOSExtensionLoader& operator=(const BrowserOSExtensionLoader&) = delete;
+
+  // Sets config URL (for command-line override).
+  void SetConfigUrl(const GURL& url);
+
+ protected:
+  ~BrowserOSExtensionLoader() override;
+
+  // ExternalLoader:
+  void StartLoading() override;
+  const base::FilePath GetBaseCrxFilePath() override;
+
+ private:
+  friend class base::RefCountedThreadSafe<extensions::ExternalLoader>;
+
+  // Called when installer completes.
+  void OnInstallComplete(InstallResult result);
+
+  // Convergence point for both startup paths.
+  void OnStartupComplete(bool from_bundled);
+
+  // Triggers immediate download for remote-loaded extensions.
+  void TriggerImmediateInstallation(base::Value::Dict config);
+
+  // Adjusts prefs to match existing install locations. Extensions installed via
+  // kExternalPrefDownload must be claimed via external_update_url to avoid
+  // orphan detection when bundled prefs use external_crx.
+  void AdjustPrefsForExistingInstalls(base::Value::Dict& prefs);
+
+  raw_ptr<Profile> profile_;
+  GURL config_url_;
+  base::FilePath bundled_crx_base_path_;
+
+  std::set<std::string> extension_ids_;
+  base::Value::Dict last_config_;
+
+  std::unique_ptr<BrowserOSExtensionInstaller> installer_;
+  std::unique_ptr<BrowserOSExtensionMaintainer> maintainer_;
+
+  base::WeakPtrFactory<BrowserOSExtensionLoader> weak_ptr_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_LOADER_H_
