diff --git a/chrome/browser/browseros/server/test/mock_process_controller.h b/chrome/browser/browseros/server/test/mock_process_controller.h
new file mode 100644
index 0000000000000..ca55f68e86fd8
--- /dev/null
+++ b/chrome/browser/browseros/server/test/mock_process_controller.h
@@ -0,0 +1,34 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_PROCESS_CONTROLLER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_PROCESS_CONTROLLER_H_
+
+#include "chrome/browser/browseros/server/process_controller.h"
+#include "testing/gmock/include/gmock/gmock.h"
+
+namespace browseros {
+
+class MockProcessController : public ProcessController {
+ public:
+  MockProcessController();
+  ~MockProcessController() override;
+
+  MockProcessController(const MockProcessController&) = delete;
+  MockProcessController& operator=(const MockProcessController&) = delete;
+
+  MOCK_METHOD(LaunchResult,
+              Launch,
+              (const ServerLaunchConfig&),
+              (override));
+  MOCK_METHOD(void, Terminate, (base::Process*, bool), (override));
+  MOCK_METHOD(bool, Exists, (base::ProcessId), (override));
+  MOCK_METHOD(std::optional<int64_t>, GetCreationTime, (base::ProcessId),
+              (override));
+  MOCK_METHOD(bool, Kill, (base::ProcessId, base::TimeDelta), (override));
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_TEST_MOCK_PROCESS_CONTROLLER_H_
