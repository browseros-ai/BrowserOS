diff --git a/chrome/browser/browseros/server/test/mock_server_state_store.h b/chrome/browser/browseros/server/test/mock_server_state_store.h
new file mode 100644
index 0000000000000..eb357f9e7b01a
--- /dev/null
+++ b/chrome/browser/browseros/server/test/mock_server_state_store.h
@@ -0,0 +1,28 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_SERVER_STATE_STORE_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_SERVER_STATE_STORE_H_
+
+#include "chrome/browser/browseros/server/server_state_store.h"
+#include "testing/gmock/include/gmock/gmock.h"
+
+namespace browseros {
+
+class MockServerStateStore : public ServerStateStore {
+ public:
+  MockServerStateStore();
+  ~MockServerStateStore() override;
+
+  MockServerStateStore(const MockServerStateStore&) = delete;
+  MockServerStateStore& operator=(const MockServerStateStore&) = delete;
+
+  MOCK_METHOD(std::optional<server_utils::ServerState>, Read, (), (override));
+  MOCK_METHOD(bool, Write, (const server_utils::ServerState&), (override));
+  MOCK_METHOD(bool, Delete, (), (override));
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_SERVER_STATE_STORE_H_
