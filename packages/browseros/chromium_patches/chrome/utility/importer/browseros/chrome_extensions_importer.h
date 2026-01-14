diff --git a/chrome/utility/importer/browseros/chrome_extensions_importer.h b/chrome/utility/importer/browseros/chrome_extensions_importer.h
new file mode 100644
index 0000000000000..675ce725425a7
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_extensions_importer.h
@@ -0,0 +1,23 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome extensions importer
+
+#ifndef CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_EXTENSIONS_IMPORTER_H_
+#define CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_EXTENSIONS_IMPORTER_H_
+
+#include <string>
+#include <vector>
+
+#include "base/files/file_path.h"
+
+namespace browseros_importer {
+
+// Imports extension IDs from Chrome's preferences files.
+// Only imports user-installed extensions from the Chrome Web Store.
+// |profile_path| should be the Chrome profile directory (e.g., .../Default)
+// Returns a vector of extension IDs. Returns empty vector on failure.
+std::vector<std::string> ImportChromeExtensions(
+    const base::FilePath& profile_path);
+
+}  // namespace browseros_importer
+
+#endif  // CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_EXTENSIONS_IMPORTER_H_
