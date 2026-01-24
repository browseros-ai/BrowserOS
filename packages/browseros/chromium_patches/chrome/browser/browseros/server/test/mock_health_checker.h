diff --git a/chrome/browser/browseros/server/test/mock_health_checker.h b/chrome/browser/browseros/server/test/mock_health_checker.h
new file mode 100644
index 0000000000000..0e771924d9181
--- /dev/null
+++ b/chrome/browser/browseros/server/test/mock_health_checker.h
@@ -0,0 +1,29 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_HEALTH_CHECKER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_HEALTH_CHECKER_H_
+
+#include "chrome/browser/browseros/server/health_checker.h"
+#include "testing/gmock/include/gmock/gmock.h"
+
+namespace browseros {
+
+class MockHealthChecker : public HealthChecker {
+ public:
+  MockHealthChecker();
+  ~MockHealthChecker() override;
+
+  MockHealthChecker(const MockHealthChecker&) = delete;
+  MockHealthChecker& operator=(const MockHealthChecker&) = delete;
+
+  MOCK_METHOD(void,
+              CheckHealth,
+              (int, base::OnceCallback<void(bool)>),
+              (override));
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_HEALTH_CHECKER_H_
