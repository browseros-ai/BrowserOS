diff --git a/chrome/browser/browseros/server/server_process_launcher_unittest.cc b/chrome/browser/browseros/server/server_process_launcher_unittest.cc
new file mode 100644
index 0000000000000000000000000000000000000000..1446939a28f483e91db85293ad4021cc36c35a91
--- /dev/null
+++ b/chrome/browser/browseros/server/server_process_launcher_unittest.cc
@@ -0,0 +1,58 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/server_process_launcher.h"
+
+#include "build/build_config.h"
+
+#if BUILDFLAG(IS_WIN)
+#include <windows.h>
+#endif
+
+#include "base/test/multiprocess_test.h"
+#include "base/test/test_timeouts.h"
+#include "testing/gtest/include/gtest/gtest.h"
+#include "testing/multiprocess_func_list.h"
+
+namespace browseros {
+namespace {
+
+MULTIPROCESS_TEST_MAIN(ManagedServerChild) {
+  return 0;
+}
+
+#if BUILDFLAG(IS_WIN)
+MULTIPROCESS_TEST_MAIN(ManagedServerNoConsoleChild) {
+  return ::GetConsoleWindow() == nullptr ? 0 : 1;
+}
+#endif
+
+class ServerProcessLauncherTest : public base::MultiProcessTest {};
+
+TEST_F(ServerProcessLauncherTest, ReturnsManagedProcess) {
+  base::Process process =
+      LaunchServerProcess(MakeCmdLine("ManagedServerChild"));
+  ASSERT_TRUE(process.IsValid());
+
+  int exit_code = -1;
+  ASSERT_TRUE(process.WaitForExitWithTimeout(TestTimeouts::action_max_timeout(),
+                                             &exit_code));
+  EXPECT_EQ(0, exit_code);
+}
+
+#if BUILDFLAG(IS_WIN)
+TEST_F(ServerProcessLauncherTest, LaunchesWithoutConsoleWindow) {
+  base::Process process =
+      LaunchServerProcess(MakeCmdLine("ManagedServerNoConsoleChild"));
+  ASSERT_TRUE(process.IsValid());
+
+  int exit_code = -1;
+  ASSERT_TRUE(process.WaitForExitWithTimeout(TestTimeouts::action_max_timeout(),
+                                             &exit_code));
+  EXPECT_EQ(0, exit_code);
+}
+#endif
+
+}  // namespace
+}  // namespace browseros
