diff --git a/chrome/browser/browseros/metrics/browseros_installation_id.h b/chrome/browser/browseros/metrics/browseros_installation_id.h
new file mode 100644
index 0000000000000000000000000000000000000000..7c7a93c7f1f0e8a3ebf40bc934a7080e81664e20
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_installation_id.h
@@ -0,0 +1,23 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_INSTALLATION_ID_H_
+#define CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_INSTALLATION_ID_H_
+
+#include <optional>
+#include <string>
+
+#include "base/files/file_path.h"
+
+namespace browseros_metrics {
+
+// Loads the product installation UUID, creating installation.json when it is
+// absent. This performs blocking filesystem I/O and must run on a sequence
+// whose task traits include base::MayBlock().
+std::optional<std::string> LoadOrCreateInstallationId(
+    const base::FilePath& product_state_directory);
+
+}  // namespace browseros_metrics
+
+#endif  // CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_INSTALLATION_ID_H_
