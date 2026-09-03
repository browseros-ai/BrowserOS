diff --git a/chrome/browser/browseros/core/browseros_product_state.h b/chrome/browser/browseros/core/browseros_product_state.h
new file mode 100644
index 0000000000000000000000000000000000000000..c5704a2e04c8496d18102d5242dbc81dce44ed5b
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_product_state.h
@@ -0,0 +1,20 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_PRODUCT_STATE_H_
+#define CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_PRODUCT_STATE_H_
+
+#include "base/files/file_path.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+
+namespace browseros {
+
+// Resolves the durable state root shared by Chromium and the selected
+// product's sidecars. Product-specific environment overrides let development
+// launchers keep every process on the same non-production installation.
+base::FilePath GetProductStateDirectory(Product product);
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_PRODUCT_STATE_H_
