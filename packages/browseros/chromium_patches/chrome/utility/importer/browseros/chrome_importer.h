diff --git a/chrome/utility/importer/browseros/chrome_importer.h b/chrome/utility/importer/browseros/chrome_importer.h
new file mode 100644
index 0000000000000..da685413cee76
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_importer.h
@@ -0,0 +1,45 @@
+// Copyright 2023 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_H_
+#define CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_H_
+
+#include <stdint.h>
+
+#include "base/files/file_path.h"
+#include "chrome/utility/importer/importer.h"
+
+// ChromeImporter orchestrates importing user data from Chrome/Chromium browsers.
+// The actual data extraction is delegated to specialized importer modules:
+// - chrome_history_importer: browsing history
+// - chrome_bookmarks_importer: bookmarks and favicons
+// - chrome_password_importer: saved passwords
+// - chrome_cookie_importer: cookies
+// - chrome_autofill_importer: autofill form data
+// - chrome_extensions_importer: extension IDs
+class ChromeImporter : public Importer {
+ public:
+  ChromeImporter();
+  ChromeImporter(const ChromeImporter&) = delete;
+  ChromeImporter& operator=(const ChromeImporter&) = delete;
+
+  // Importer:
+  void StartImport(const user_data_importer::SourceProfile& source_profile,
+                   uint16_t items,
+                   ImporterBridge* bridge) override;
+
+ private:
+  ~ChromeImporter() override;
+
+  void ImportBookmarks();
+  void ImportHistory();
+  void ImportPasswords();
+  void ImportCookies();
+  void ImportAutofillFormData();
+  void ImportExtensions();
+
+  base::FilePath source_path_;
+};
+
+#endif  // CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_H_
