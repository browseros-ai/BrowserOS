diff --git a/chrome/utility/importer/browseros/chrome_autofill_importer.h b/chrome/utility/importer/browseros/chrome_autofill_importer.h
new file mode 100644
index 0000000000000..915030187f327
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_autofill_importer.h
@@ -0,0 +1,22 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome autofill importer
+
+#ifndef CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_AUTOFILL_IMPORTER_H_
+#define CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_AUTOFILL_IMPORTER_H_
+
+#include <vector>
+
+#include "base/files/file_path.h"
+#include "chrome/common/importer/importer_autofill_form_data_entry.h"
+
+namespace browseros_importer {
+
+// Imports autofill form data from Chrome's Web Data database.
+// |profile_path| should be the Chrome profile directory (e.g., .../Default)
+// Returns a vector of ImporterAutofillFormDataEntry. Returns empty on failure.
+std::vector<ImporterAutofillFormDataEntry> ImportChromeAutofill(
+    const base::FilePath& profile_path);
+
+}  // namespace browseros_importer
+
+#endif  // CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_AUTOFILL_IMPORTER_H_
