diff --git a/chrome/utility/importer/browseros/chrome_importer_utils.cc b/chrome/utility/importer/browseros/chrome_importer_utils.cc
new file mode 100644
index 0000000000000..3000a42871631
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_importer_utils.cc
@@ -0,0 +1,65 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome importer shared utilities
+
+#include "chrome/utility/importer/browseros/chrome_importer_utils.h"
+
+#include "base/files/file_util.h"
+#include "base/logging.h"
+
+namespace browseros_importer {
+
+base::Time ChromeTimeToBaseTime(int64_t chrome_time) {
+  if (chrome_time == 0) {
+    return base::Time();
+  }
+  return base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(chrome_time));
+}
+
+base::FilePath CopyToTempFile(const base::FilePath& source_path) {
+  base::FilePath temp_path;
+  if (!base::CreateTemporaryFile(&temp_path)) {
+    LOG(WARNING) << "browseros: Failed to create temp file for "
+                 << source_path.BaseName().value();
+    return base::FilePath();
+  }
+
+  if (!base::CopyFile(source_path, temp_path)) {
+    LOG(WARNING) << "browseros: Failed to copy "
+                 << source_path.BaseName().value() << " to temp";
+    base::DeleteFile(temp_path);
+    return base::FilePath();
+  }
+
+  return temp_path;
+}
+
+base::FilePath CopyDatabaseToTemp(const base::FilePath& db_path) {
+  base::FilePath temp_path = CopyToTempFile(db_path);
+  if (temp_path.empty()) {
+    return base::FilePath();
+  }
+
+  // Also copy the "-wal" sidecar if present. base::CopyFile is best-effort: a
+  // partial trailing frame is expected and handled by SQLite's WAL recovery,
+  // which reads up to the last valid commit. Per SQLite guidance the "-shm"
+  // index is not copied; it is regenerated on open.
+  const base::FilePath::StringType kWalSuffix = FILE_PATH_LITERAL("-wal");
+  base::FilePath wal_src(db_path.value() + kWalSuffix);
+  if (base::PathExists(wal_src)) {
+    base::CopyFile(wal_src, base::FilePath(temp_path.value() + kWalSuffix));
+  }
+
+  return temp_path;
+}
+
+void DeleteDatabaseTemp(const base::FilePath& temp_path) {
+  if (temp_path.empty()) {
+    return;
+  }
+  base::DeleteFile(temp_path);
+  base::DeleteFile(base::FilePath(temp_path.value() + FILE_PATH_LITERAL("-wal")));
+  base::DeleteFile(base::FilePath(temp_path.value() + FILE_PATH_LITERAL("-shm")));
+}
+
+}  // namespace browseros_importer
