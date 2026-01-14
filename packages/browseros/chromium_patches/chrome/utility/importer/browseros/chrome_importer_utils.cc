diff --git a/chrome/utility/importer/browseros/chrome_importer_utils.cc b/chrome/utility/importer/browseros/chrome_importer_utils.cc
new file mode 100644
index 0000000000000..3d8381d979541
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_importer_utils.cc
@@ -0,0 +1,37 @@
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
+    LOG(WARNING) << "ChromeImporter: Failed to create temp file for "
+                 << source_path.BaseName().value();
+    return base::FilePath();
+  }
+
+  if (!base::CopyFile(source_path, temp_path)) {
+    LOG(WARNING) << "ChromeImporter: Failed to copy "
+                 << source_path.BaseName().value() << " to temp";
+    base::DeleteFile(temp_path);
+    return base::FilePath();
+  }
+
+  return temp_path;
+}
+
+}  // namespace browseros_importer
