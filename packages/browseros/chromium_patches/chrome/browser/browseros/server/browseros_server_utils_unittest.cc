diff --git a/chrome/browser/browseros/server/browseros_server_utils_unittest.cc b/chrome/browser/browseros/server/browseros_server_utils_unittest.cc
new file mode 100644
index 0000000000000..ad434d78b5e68
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_utils_unittest.cc
@@ -0,0 +1,91 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_utils.h"
+
+#include <set>
+
+#include "base/files/file_path.h"
+#include "base/files/file_util.h"
+#include "base/files/scoped_temp_dir.h"
+#include "testing/gtest/include/gtest/gtest.h"
+
+namespace browseros::server_utils {
+namespace {
+
+// =============================================================================
+// ServerState Struct Tests
+// =============================================================================
+
+TEST(ServerUtilsStateTest, DefaultStateValues) {
+  ServerState state;
+  EXPECT_EQ(0, state.pid);
+  EXPECT_EQ(0, state.creation_time);
+}
+
+// =============================================================================
+// Port Availability Tests
+// =============================================================================
+
+TEST(ServerUtilsPortTest, IsPortAvailable_RejectsInvalidPorts) {
+  EXPECT_FALSE(IsPortAvailable(0));
+  EXPECT_FALSE(IsPortAvailable(-1));
+  EXPECT_FALSE(IsPortAvailable(65536));
+}
+
+TEST(ServerUtilsPortTest, IsPortAvailable_RejectsWellKnownPorts) {
+  // Well-known ports (0-1023) should be rejected
+  EXPECT_FALSE(IsPortAvailable(80));
+  EXPECT_FALSE(IsPortAvailable(443));
+  EXPECT_FALSE(IsPortAvailable(22));
+}
+
+TEST(ServerUtilsPortTest, FindAvailablePort_RespectsExcludedPorts) {
+  std::set<int> excluded;
+  excluded.insert(9000);
+  excluded.insert(9001);
+  excluded.insert(9002);
+
+  int found = FindAvailablePort(9000, excluded);
+
+  // Should not return any excluded port
+  EXPECT_EQ(excluded.find(found), excluded.end());
+
+  // Should be >= starting port or fall back gracefully
+  EXPECT_GT(found, 0);
+}
+
+TEST(ServerUtilsPortTest, FindAvailablePort_StartsFromGivenPort) {
+  std::set<int> excluded;
+
+  int found = FindAvailablePort(10000, excluded);
+
+  // Should find a port >= starting port (assuming 10000+ is available)
+  EXPECT_GE(found, 10000);
+}
+
+// =============================================================================
+// Path Utility Tests
+// =============================================================================
+
+TEST(ServerUtilsPathTest, GetLockFilePath_EndsWithServerLock) {
+  base::FilePath lock_path = GetLockFilePath();
+
+  // Should end with "server.lock" (may be empty if execution dir fails)
+  if (!lock_path.empty()) {
+    EXPECT_EQ("server.lock", lock_path.BaseName().AsUTF8Unsafe());
+  }
+}
+
+TEST(ServerUtilsPathTest, GetStateFilePath_EndsWithServerState) {
+  base::FilePath state_path = GetStateFilePath();
+
+  // Should end with "server.state" (may be empty if execution dir fails)
+  if (!state_path.empty()) {
+    EXPECT_EQ("server.state", state_path.BaseName().AsUTF8Unsafe());
+  }
+}
+
+}  // namespace
+}  // namespace browseros::server_utils
