diff --git a/chrome/utility/importer/browseros/chrome_autofill_importer.cc b/chrome/utility/importer/browseros/chrome_autofill_importer.cc
new file mode 100644
index 0000000000000..3d1e95eaf3461
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_autofill_importer.cc
@@ -0,0 +1,79 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome autofill importer implementation
+
+#include "chrome/utility/importer/browseros/chrome_autofill_importer.h"
+
+#include "base/files/file_util.h"
+#include "base/logging.h"
+#include "chrome/utility/importer/browseros/chrome_importer_utils.h"
+#include "sql/database.h"
+#include "sql/statement.h"
+
+namespace browseros_importer {
+
+namespace {
+
+inline constexpr sql::Database::Tag kDatabaseTag{"ChromeImporter"};
+constexpr char kWebDataFilename[] = "Web Data";
+
+}  // namespace
+
+std::vector<ImporterAutofillFormDataEntry> ImportChromeAutofill(
+    const base::FilePath& profile_path) {
+  std::vector<ImporterAutofillFormDataEntry> entries;
+
+  // Web Data is in the parent directory of the profile
+  base::FilePath web_data_path =
+      profile_path.DirName().AppendASCII(kWebDataFilename);
+  if (!base::PathExists(web_data_path)) {
+    // Try profile directory as fallback
+    web_data_path = profile_path.AppendASCII(kWebDataFilename);
+    if (!base::PathExists(web_data_path)) {
+      LOG(WARNING) << "browseros: Web Data file not found";
+      return entries;
+    }
+  }
+
+  base::FilePath temp_path = CopyToTempFile(web_data_path);
+  if (temp_path.empty()) {
+    return entries;
+  }
+
+  sql::Database db(kDatabaseTag);
+  if (!db.Open(temp_path)) {
+    LOG(WARNING) << "browseros: Failed to open database";
+    base::DeleteFile(temp_path);
+    return entries;
+  }
+
+  // Use scope block to ensure statement is destroyed before db closes
+  {
+    const char kQuery[] =
+        "SELECT name, value, count, date_created, date_last_used "
+        "FROM autofill";
+
+    sql::Statement statement(db.GetUniqueStatement(kQuery));
+    if (!statement.is_valid()) {
+      LOG(WARNING) << "browseros: Failed to prepare query";
+      base::DeleteFile(temp_path);
+      return entries;
+    }
+
+    while (statement.Step()) {
+      ImporterAutofillFormDataEntry entry;
+      entry.name = statement.ColumnString16(0);
+      entry.value = statement.ColumnString16(1);
+      entry.times_used = statement.ColumnInt(2);
+      entry.first_used = ChromeTimeToBaseTime(statement.ColumnInt64(3));
+      entry.last_used = ChromeTimeToBaseTime(statement.ColumnInt64(4));
+
+      entries.push_back(std::move(entry));
+    }
+  }  // statement destroyed here
+
+  base::DeleteFile(temp_path);
+
+  return entries;
+}
+
+}  // namespace browseros_importer
