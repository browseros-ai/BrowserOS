diff --git a/chrome/utility/importer/browseros/chrome_importer.h b/chrome/utility/importer/browseros/chrome_importer.h
new file mode 100644
index 0000000000000..736fc58be40bc
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_importer.h
@@ -0,0 +1,82 @@
+// Copyright 2023 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_H_
+#define CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_H_
+
+#include <stdint.h>
+
+#include <map>
+#include <set>
+#include <string>
+#include <vector>
+
+#include "base/files/file_path.h"
+#include "base/values.h"
+#include "build/build_config.h"
+#include "chrome/common/importer/importer_autofill_form_data_entry.h"
+#include "chrome/utility/importer/importer.h"
+#include "components/favicon_base/favicon_usage_data.h"
+
+namespace user_data_importer {
+struct ImportedBookmarkEntry;
+}  // namespace user_data_importer
+
+namespace sql {
+class Database;
+}
+
+
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
+  // Helper function to convert Chrome's time format to base::Time
+  base::Time ChromeTimeToBaseTime(int64_t time);
+
+  // Multiple URLs can share the same favicon; this is a map
+  // of favicon IDs -> URLs that we load as a temporary step before
+  // actually loading the icons.
+  using FaviconMap = std::map<int64_t, std::set<GURL>>;
+
+  // Loads the URLs associated with the favicons into favicon_map
+  void ImportFaviconURLs(sql::Database* db, FaviconMap* favicon_map);
+
+  // Loads and reencodes the individual favicons
+  void LoadFaviconData(sql::Database* db,
+                       const FaviconMap& favicon_map,
+                       favicon_base::FaviconUsageDataList* favicons);
+
+  // Recursively reads a bookmarks folder from the JSON structure
+  void RecursiveReadBookmarksFolder(
+      const base::Value::Dict* folder,
+      const std::vector<std::u16string>& parent_path,
+      bool is_in_toolbar,
+      std::vector<user_data_importer::ImportedBookmarkEntry>* bookmarks);
+
+  // Extracts extension IDs from Chrome preferences file
+  std::vector<std::string> GetExtensionsFromPreferencesFile(
+      const base::FilePath& preferences_path);
+
+  base::FilePath source_path_;
+};
+
+#endif  // CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_H_
