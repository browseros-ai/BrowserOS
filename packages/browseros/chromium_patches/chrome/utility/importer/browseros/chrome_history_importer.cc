diff --git a/chrome/utility/importer/browseros/chrome_history_importer.cc b/chrome/utility/importer/browseros/chrome_history_importer.cc
new file mode 100644
index 0000000000000..ec7b96bb6b854
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_history_importer.cc
@@ -0,0 +1,92 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome history importer implementation
+
+#include "chrome/utility/importer/browseros/chrome_history_importer.h"
+
+#include "base/files/file_util.h"
+#include "base/logging.h"
+#include "chrome/utility/importer/browseros/chrome_importer_utils.h"
+#include "sql/database.h"
+#include "sql/statement.h"
+#include "ui/base/page_transition_types.h"
+#include "url/gurl.h"
+
+namespace browseros_importer {
+
+namespace {
+
+inline constexpr sql::Database::Tag kDatabaseTag{"ChromeImporter"};
+constexpr char kHistoryFilename[] = "History";
+
+}  // namespace
+
+std::vector<user_data_importer::ImporterURLRow> ImportChromeHistory(
+    const base::FilePath& profile_path) {
+  std::vector<user_data_importer::ImporterURLRow> rows;
+
+  base::FilePath history_path = profile_path.AppendASCII(kHistoryFilename);
+  if (!base::PathExists(history_path)) {
+    LOG(WARNING) << "browseros: History file not found";
+    return rows;
+  }
+
+  base::FilePath temp_path = CopyToTempFile(history_path);
+  if (temp_path.empty()) {
+    return rows;
+  }
+
+  sql::Database db(kDatabaseTag);
+  if (!db.Open(temp_path)) {
+    LOG(WARNING) << "browseros: Failed to open database";
+    base::DeleteFile(temp_path);
+    return rows;
+  }
+
+  // Query URLs with visit information, filtering out internal navigation types
+  // - CHAIN_END: Only get final URLs in redirect chains
+  // - Exclude SUBFRAME and KEYWORD_GENERATED transitions
+  // Use scope block to ensure statement is destroyed before db.Close()
+  {
+    const char kQuery[] =
+        "SELECT u.url, u.title, v.visit_time, u.typed_count, u.visit_count "
+        "FROM urls u JOIN visits v ON u.id = v.url "
+        "WHERE hidden = 0 "
+        "AND (transition & ?) != 0 "
+        "AND (transition & ?) NOT IN (?, ?, ?)";
+
+    sql::Statement statement(db.GetUniqueStatement(kQuery));
+    if (!statement.is_valid()) {
+      LOG(WARNING) << "browseros: Failed to prepare query";
+      base::DeleteFile(temp_path);
+      return rows;
+    }
+
+    statement.BindInt64(0, ui::PAGE_TRANSITION_CHAIN_END);
+    statement.BindInt64(1, ui::PAGE_TRANSITION_CORE_MASK);
+    statement.BindInt64(2, ui::PAGE_TRANSITION_AUTO_SUBFRAME);
+    statement.BindInt64(3, ui::PAGE_TRANSITION_MANUAL_SUBFRAME);
+    statement.BindInt64(4, ui::PAGE_TRANSITION_KEYWORD_GENERATED);
+
+    while (statement.Step()) {
+      GURL url(statement.ColumnString(0));
+      if (!url.is_valid()) {
+        continue;
+      }
+
+      user_data_importer::ImporterURLRow row(url);
+      row.title = statement.ColumnString16(1);
+      row.last_visit = ChromeTimeToBaseTime(statement.ColumnInt64(2));
+      row.hidden = false;
+      row.typed_count = statement.ColumnInt(3);
+      row.visit_count = statement.ColumnInt(4);
+
+      rows.push_back(std::move(row));
+    }
+  }  // statement destroyed here
+
+  base::DeleteFile(temp_path);
+
+  return rows;
+}
+
+}  // namespace browseros_importer
