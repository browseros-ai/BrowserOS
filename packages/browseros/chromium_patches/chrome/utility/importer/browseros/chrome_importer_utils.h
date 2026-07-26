diff --git a/chrome/utility/importer/browseros/chrome_importer_utils.h b/chrome/utility/importer/browseros/chrome_importer_utils.h
new file mode 100644
index 0000000000000..d7334846286cd
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_importer_utils.h
@@ -0,0 +1,35 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome importer shared utilities
+
+#ifndef CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_UTILS_H_
+#define CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_UTILS_H_
+
+#include "base/files/file_path.h"
+#include "base/time/time.h"
+
+namespace browseros_importer {
+
+// Converts Chrome's internal time format (microseconds since Windows epoch)
+// to base::Time. Returns null time for zero input.
+base::Time ChromeTimeToBaseTime(int64_t chrome_time);
+
+// Copies a file to a temporary location to avoid locking issues when the
+// source browser is running. Returns empty path on failure.
+// Caller is responsible for deleting the temp file when done.
+base::FilePath CopyToTempFile(const base::FilePath& source_path);
+
+// Copies a SQLite database plus its "-wal" write-ahead-log sidecar (if present)
+// to a temporary location. Copying the WAL matters when the source browser is
+// running: recently written rows (e.g. fresh session cookies) may live only in
+// the WAL and would otherwise be missed. SQLite replays the WAL and rebuilds
+// the "-shm" index on open. Returns empty path on failure. Pair with
+// DeleteDatabaseTemp() for cleanup.
+base::FilePath CopyDatabaseToTemp(const base::FilePath& db_path);
+
+// Deletes a temp database created by CopyDatabaseToTemp(), including its "-wal"
+// and "-shm" sidecars. No-op for an empty path.
+void DeleteDatabaseTemp(const base::FilePath& temp_path);
+
+}  // namespace browseros_importer
+
+#endif  // CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_IMPORTER_UTILS_H_
