diff --git a/chrome/browser/browseros/server/test/mock_server_updater.h b/chrome/browser/browseros/server/test/mock_server_updater.h
new file mode 100644
index 0000000000000..c36962da26240
--- /dev/null
+++ b/chrome/browser/browseros/server/test/mock_server_updater.h
@@ -0,0 +1,31 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_SERVER_UPDATER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_SERVER_UPDATER_H_
+
+#include "chrome/browser/browseros/server/server_updater.h"
+#include "testing/gmock/include/gmock/gmock.h"
+
+namespace browseros {
+
+class MockServerUpdater : public ServerUpdater {
+ public:
+  MockServerUpdater();
+  ~MockServerUpdater() override;
+
+  MockServerUpdater(const MockServerUpdater&) = delete;
+  MockServerUpdater& operator=(const MockServerUpdater&) = delete;
+
+  MOCK_METHOD(void, Start, (), (override));
+  MOCK_METHOD(void, Stop, (), (override));
+  MOCK_METHOD(bool, IsUpdateInProgress, (), (const, override));
+  MOCK_METHOD(base::FilePath, GetBestServerBinaryPath, (), (override));
+  MOCK_METHOD(base::FilePath, GetBestServerResourcesPath, (), (override));
+  MOCK_METHOD(void, InvalidateDownloadedVersion, (), (override));
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_SERVER_UPDATER_H_
