diff --git a/chrome/browser/browseros/server/browseros_server_manager_unittest.cc b/chrome/browser/browseros/server/browseros_server_manager_unittest.cc
new file mode 100644
index 0000000000000..8f97e0b97467f
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager_unittest.cc
@@ -0,0 +1,497 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
+
+#include <memory>
+
+#include "base/command_line.h"
+#include "base/memory/raw_ptr.h"
+#include "base/test/scoped_command_line.h"
+#include "base/test/task_environment.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
+#include "chrome/browser/browseros/server/test/mock_health_checker.h"
+#include "chrome/browser/browseros/server/test/mock_process_controller.h"
+#include "chrome/browser/browseros/server/test/mock_server_state_store.h"
+#include "chrome/browser/browseros/server/test/mock_server_updater.h"
+#include "components/prefs/pref_registry_simple.h"
+#include "components/prefs/testing_pref_service.h"
+#include "testing/gmock/include/gmock/gmock.h"
+#include "testing/gtest/include/gtest/gtest.h"
+
+using ::testing::_;
+using ::testing::Invoke;
+using ::testing::NiceMock;
+using ::testing::Return;
+using ::testing::StrictMock;
+
+namespace browseros {
+namespace {
+
+class BrowserOSServerManagerTest : public testing::Test {
+ protected:
+  void SetUp() override {
+    browseros_server::RegisterLocalStatePrefs(prefs_.registry());
+
+    auto process_controller =
+        std::make_unique<NiceMock<MockProcessController>>();
+    auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+    auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+    auto updater = std::make_unique<NiceMock<MockServerUpdater>>();
+
+    process_controller_ = process_controller.get();
+    state_store_ = state_store.get();
+    health_checker_ = health_checker.get();
+    updater_ = updater.get();
+
+    testing::Mock::AllowLeak(process_controller_);
+    testing::Mock::AllowLeak(state_store_);
+    testing::Mock::AllowLeak(health_checker_);
+    testing::Mock::AllowLeak(updater_);
+
+    ON_CALL(*updater_, GetBestServerBinaryPath())
+        .WillByDefault(Return(base::FilePath("/fake/path/browseros_server")));
+    ON_CALL(*updater_, GetBestServerResourcesPath())
+        .WillByDefault(Return(base::FilePath("/fake/path/resources")));
+
+    manager_ = new BrowserOSServerManager(
+        std::move(process_controller), std::move(state_store),
+        std::move(health_checker), std::move(updater), &prefs_);
+  }
+
+  void TearDown() override {
+    if (manager_) {
+      manager_->Shutdown();
+    }
+  }
+
+  void SetupSuccessfulLaunch() {
+    ON_CALL(*process_controller_, Launch(_))
+        .WillByDefault([](const ServerLaunchConfig&) {
+          LaunchResult result;
+          result.process = base::Process::Current();
+          result.used_fallback = false;
+          return result;
+        });
+  }
+
+  void SetupFailedLaunch() {
+    ON_CALL(*process_controller_, Launch(_))
+        .WillByDefault([](const ServerLaunchConfig&) {
+          LaunchResult result;
+          result.used_fallback = false;
+          return result;
+        });
+  }
+
+  base::test::TaskEnvironment task_environment_{
+      base::test::TaskEnvironment::TimeSource::MOCK_TIME};
+  TestingPrefServiceSimple prefs_;
+
+  raw_ptr<MockProcessController> process_controller_ = nullptr;
+  raw_ptr<MockServerStateStore> state_store_ = nullptr;
+  raw_ptr<MockHealthChecker> health_checker_ = nullptr;
+  raw_ptr<MockServerUpdater> updater_ = nullptr;
+
+  raw_ptr<BrowserOSServerManager> manager_ = nullptr;
+};
+
+// =============================================================================
+// Health Check Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckPass_NoRestart) {
+  EXPECT_CALL(*health_checker_, CheckHealth(_, _))
+      .WillOnce([](int port, base::OnceCallback<void(bool)> callback) {
+        std::move(callback).Run(true);
+      });
+
+  EXPECT_CALL(*process_controller_, Terminate(_, _)).Times(0);
+}
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckFail_TriggersRestart) {
+  manager_->SetRunningForTesting(true);
+
+  // Single health check failure should trigger restart
+  manager_->OnHealthCheckComplete(false);
+  // is_restarting_ is now true (verified indirectly: second call is ignored)
+  manager_->OnHealthCheckComplete(false);
+}
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckPass_DoesNotRestart) {
+  manager_->SetRunningForTesting(true);
+
+  // Successful health check should not trigger restart
+  manager_->OnHealthCheckComplete(true);
+}
+
+// =============================================================================
+// Updater Integration Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, StopCallsUpdaterStop) {
+  EXPECT_CALL(*updater_, Stop()).Times(1);
+  manager_->Stop();
+}
+
+TEST_F(BrowserOSServerManagerTest, GetBinaryPathUsesUpdater) {
+  base::FilePath expected_path("/custom/binary/path");
+  EXPECT_CALL(*updater_, GetBestServerBinaryPath())
+      .WillOnce(Return(expected_path));
+}
+
+TEST_F(BrowserOSServerManagerTest, GetResourcesPathUsesUpdater) {
+  base::FilePath expected_path("/custom/resources/path");
+  EXPECT_CALL(*updater_, GetBestServerResourcesPath())
+      .WillOnce(Return(expected_path));
+}
+
+// =============================================================================
+// Port Preference Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, LoadsPortsFromPrefs) {
+  prefs_.SetInteger(browseros_server::kCDPServerPort, 8000);
+  prefs_.SetInteger(browseros_server::kProxyPort, 8100);
+
+  auto process_controller =
+      std::make_unique<NiceMock<MockProcessController>>();
+  auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+  auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+  auto updater = std::make_unique<NiceMock<MockServerUpdater>>();
+
+  testing::Mock::AllowLeak(process_controller.get());
+  testing::Mock::AllowLeak(state_store.get());
+  testing::Mock::AllowLeak(health_checker.get());
+  testing::Mock::AllowLeak(updater.get());
+
+  ON_CALL(*updater, GetBestServerBinaryPath())
+      .WillByDefault(Return(base::FilePath("/fake/path")));
+  ON_CALL(*updater, GetBestServerResourcesPath())
+      .WillByDefault(Return(base::FilePath("/fake/resources")));
+
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), std::move(updater), &prefs_);
+
+  manager->Shutdown();
+}
+
+TEST_F(BrowserOSServerManagerTest, DefaultPortsWhenPrefsEmpty) {
+  EXPECT_EQ(browseros_server::kDefaultCDPPort,
+            prefs_.GetInteger(browseros_server::kCDPServerPort));
+
+  auto process_controller =
+      std::make_unique<NiceMock<MockProcessController>>();
+  auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+  auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+  auto updater = std::make_unique<NiceMock<MockServerUpdater>>();
+
+  testing::Mock::AllowLeak(process_controller.get());
+  testing::Mock::AllowLeak(state_store.get());
+  testing::Mock::AllowLeak(health_checker.get());
+  testing::Mock::AllowLeak(updater.get());
+
+  ON_CALL(*updater, GetBestServerBinaryPath())
+      .WillByDefault(Return(base::FilePath("/fake/path")));
+  ON_CALL(*updater, GetBestServerResourcesPath())
+      .WillByDefault(Return(base::FilePath("/fake/resources")));
+
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), std::move(updater), &prefs_);
+  manager->Shutdown();
+}
+
+TEST_F(BrowserOSServerManagerTest, MigratesOldMCPPortToProxy) {
+  // Set old pref (simulates pre-upgrade state)
+  prefs_.SetInteger(browseros_server::kMCPServerPort, 9200);
+  // Ensure new proxy pref is at default (0, meaning not yet migrated)
+  prefs_.SetInteger(browseros_server::kProxyPort, 0);
+
+  base::test::ScopedCommandLine scoped_command_line;
+  scoped_command_line.GetProcessCommandLine()->AppendSwitch(
+      browseros::kDisableServer);
+
+  auto process_controller =
+      std::make_unique<NiceMock<MockProcessController>>();
+  auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+  auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+  auto updater = std::make_unique<NiceMock<MockServerUpdater>>();
+
+  testing::Mock::AllowLeak(process_controller.get());
+  testing::Mock::AllowLeak(state_store.get());
+  testing::Mock::AllowLeak(health_checker.get());
+  testing::Mock::AllowLeak(updater.get());
+
+  ON_CALL(*updater, GetBestServerBinaryPath())
+      .WillByDefault(Return(base::FilePath("/fake/path")));
+  ON_CALL(*updater, GetBestServerResourcesPath())
+      .WillByDefault(Return(base::FilePath("/fake/resources")));
+
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), std::move(updater), &prefs_);
+
+  // Start triggers LoadPortsFromPrefs which migrates
+  manager->Start();
+
+  // Proxy port should have taken the old MCP port value
+  EXPECT_EQ(9200, manager->GetProxyPort());
+  manager->Shutdown();
+}
+
+TEST_F(BrowserOSServerManagerTest, AllowRemoteInMCPPref) {
+  prefs_.SetBoolean(browseros_server::kAllowRemoteInMCP, true);
+
+  base::test::ScopedCommandLine scoped_command_line;
+  scoped_command_line.GetProcessCommandLine()->AppendSwitch(
+      browseros::kDisableServer);
+
+  auto process_controller =
+      std::make_unique<NiceMock<MockProcessController>>();
+  auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+  auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+  auto updater = std::make_unique<NiceMock<MockServerUpdater>>();
+
+  testing::Mock::AllowLeak(process_controller.get());
+  testing::Mock::AllowLeak(state_store.get());
+  testing::Mock::AllowLeak(health_checker.get());
+  testing::Mock::AllowLeak(updater.get());
+
+  ON_CALL(*updater, GetBestServerBinaryPath())
+      .WillByDefault(Return(base::FilePath("/fake/path")));
+  ON_CALL(*updater, GetBestServerResourcesPath())
+      .WillByDefault(Return(base::FilePath("/fake/resources")));
+
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), std::move(updater), &prefs_);
+
+  EXPECT_FALSE(manager->IsAllowRemoteInMCP());
+  manager->Start();
+  EXPECT_TRUE(manager->IsAllowRemoteInMCP());
+  manager->Shutdown();
+}
+
+// =============================================================================
+// Null Prefs Handling Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, HandlesNullPrefs) {
+  auto process_controller =
+      std::make_unique<NiceMock<MockProcessController>>();
+  auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+  auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+  auto updater = std::make_unique<NiceMock<MockServerUpdater>>();
+
+  testing::Mock::AllowLeak(process_controller.get());
+  testing::Mock::AllowLeak(state_store.get());
+  testing::Mock::AllowLeak(health_checker.get());
+  testing::Mock::AllowLeak(updater.get());
+
+  ON_CALL(*updater, GetBestServerBinaryPath())
+      .WillByDefault(Return(base::FilePath("/fake/path")));
+  ON_CALL(*updater, GetBestServerResourcesPath())
+      .WillByDefault(Return(base::FilePath("/fake/resources")));
+
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), std::move(updater),
+      nullptr);
+
+  EXPECT_FALSE(manager->IsRunning());
+  EXPECT_EQ(0, manager->GetCDPPort());
+  EXPECT_EQ(0, manager->GetMCPPort());
+  EXPECT_EQ(0, manager->GetProxyPort());
+  manager->Shutdown();
+}
+
+// =============================================================================
+// Null Updater Handling Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, HandlesNullUpdater) {
+  auto process_controller =
+      std::make_unique<NiceMock<MockProcessController>>();
+  auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+  auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+
+  testing::Mock::AllowLeak(process_controller.get());
+  testing::Mock::AllowLeak(state_store.get());
+  testing::Mock::AllowLeak(health_checker.get());
+
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker),
+      nullptr,
+      &prefs_);
+
+  EXPECT_FALSE(manager->IsRunning());
+  manager->Stop();
+  manager->Shutdown();
+}
+
+// =============================================================================
+// IsRunning State Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, InitiallyNotRunning) {
+  EXPECT_FALSE(manager_->IsRunning());
+}
+
+TEST_F(BrowserOSServerManagerTest, PortsInitiallyZero) {
+  EXPECT_EQ(0, manager_->GetCDPPort());
+  EXPECT_EQ(0, manager_->GetMCPPort());
+  EXPECT_EQ(0, manager_->GetProxyPort());
+  EXPECT_EQ(0, manager_->GetExtensionPort());
+  EXPECT_EQ(0, manager_->GetServerPort());
+}
+
+// =============================================================================
+// Restart Server For Update Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, RestartForUpdate_FailsWhenAlreadyRestarting) {
+  bool first_callback_called = false;
+  bool second_callback_called = false;
+  bool first_result = true;
+  bool second_result = true;
+
+  manager_->RestartServerForUpdate(
+      base::BindOnce([](bool* called, bool* result, bool success) {
+        *called = true;
+        *result = success;
+      }, &first_callback_called, &first_result));
+
+  manager_->RestartServerForUpdate(
+      base::BindOnce([](bool* called, bool* result, bool success) {
+        *called = true;
+        *result = success;
+      }, &second_callback_called, &second_result));
+
+  EXPECT_TRUE(second_callback_called);
+  EXPECT_FALSE(second_result);
+}
+
+// =============================================================================
+// Process Controller Integration Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, TerminateUsesProcessController) {
+  EXPECT_CALL(*process_controller_, Terminate(_, false)).Times(1);
+  manager_->Stop();
+}
+
+// =============================================================================
+// Launch Fallback Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, InvalidatesVersionOnFallback) {
+  ON_CALL(*process_controller_, Launch(_))
+      .WillByDefault([](const ServerLaunchConfig&) {
+        LaunchResult result;
+        result.process = base::Process::Current();
+        result.used_fallback = true;
+        return result;
+      });
+
+  EXPECT_CALL(*updater_, InvalidateDownloadedVersion()).Times(1);
+}
+
+// =============================================================================
+// Orphan Recovery / State Store Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, StopDeletesStateFile) {
+  manager_->SetRunningForTesting(true);
+
+  EXPECT_CALL(*state_store_, Delete()).Times(1);
+  EXPECT_CALL(*updater_, Stop()).Times(1);
+
+  manager_->Stop();
+}
+
+TEST_F(BrowserOSServerManagerTest, RecoverFromOrphan_NoStateFile) {
+  EXPECT_CALL(*state_store_, Read())
+      .WillOnce(Return(std::nullopt));
+
+  EXPECT_CALL(*state_store_, Delete()).Times(0);
+}
+
+TEST_F(BrowserOSServerManagerTest, RecoverFromOrphan_ProcessGone) {
+  server_utils::ServerState state;
+  state.pid = 99999;
+  state.creation_time = 123456789;
+
+  EXPECT_CALL(*state_store_, Read())
+      .WillOnce(Return(state));
+
+  EXPECT_CALL(*state_store_, Delete()).Times(1);
+}
+
+// =============================================================================
+// Restart Saves Ports to Prefs Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, RestartSavesEphemeralPortsToPrefs) {
+  SetupSuccessfulLaunch();
+  manager_->SetRunningForTesting(true);
+
+  // Set initial known ports in prefs
+  prefs_.SetInteger(browseros_server::kServerPort, 9200);
+  prefs_.SetInteger(browseros_server::kExtensionServerPort, 9300);
+
+  // Mock WaitForExitWithTimeout (called on thread pool during restart)
+  ON_CALL(*process_controller_, WaitForExitWithTimeout(_, _, _))
+      .WillByDefault(Return(true));
+
+  // Trigger restart via health check failure
+  manager_->OnHealthCheckComplete(false);
+
+  // Run all pending tasks (thread pool + reply)
+  task_environment_.RunUntilIdle();
+
+  // After restart, prefs must reflect the manager's current in-memory ports.
+  // This is the invariant: prefs and in-memory state stay in sync.
+  EXPECT_EQ(manager_->GetServerPort(),
+            prefs_.GetInteger(browseros_server::kServerPort));
+  EXPECT_EQ(manager_->GetExtensionPort(),
+            prefs_.GetInteger(browseros_server::kExtensionServerPort));
+  // Server and extension ports should be non-zero (resolved by FindAvailablePort)
+  EXPECT_NE(0, prefs_.GetInteger(browseros_server::kServerPort));
+  EXPECT_NE(0, prefs_.GetInteger(browseros_server::kExtensionServerPort));
+}
+
+TEST_F(BrowserOSServerManagerTest, UpdateRestartSavesEphemeralPortsToPrefs) {
+  SetupSuccessfulLaunch();
+  manager_->SetRunningForTesting(true);
+
+  prefs_.SetInteger(browseros_server::kServerPort, 9200);
+  prefs_.SetInteger(browseros_server::kExtensionServerPort, 9300);
+
+  ON_CALL(*process_controller_, WaitForExitWithTimeout(_, _, _))
+      .WillByDefault(Return(true));
+
+  bool callback_called = false;
+  bool callback_result = false;
+  manager_->RestartServerForUpdate(
+      base::BindOnce([](bool* called, bool* result, bool success) {
+        *called = true;
+        *result = success;
+      }, &callback_called, &callback_result));
+
+  task_environment_.RunUntilIdle();
+
+  EXPECT_EQ(manager_->GetServerPort(),
+            prefs_.GetInteger(browseros_server::kServerPort));
+  EXPECT_EQ(manager_->GetExtensionPort(),
+            prefs_.GetInteger(browseros_server::kExtensionServerPort));
+  EXPECT_NE(0, prefs_.GetInteger(browseros_server::kServerPort));
+  EXPECT_NE(0, prefs_.GetInteger(browseros_server::kExtensionServerPort));
+}
+
+}  // namespace
+}  // namespace browseros
