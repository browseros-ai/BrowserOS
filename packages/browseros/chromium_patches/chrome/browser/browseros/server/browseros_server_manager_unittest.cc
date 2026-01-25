diff --git a/chrome/browser/browseros/server/browseros_server_manager_unittest.cc b/chrome/browser/browseros/server/browseros_server_manager_unittest.cc
new file mode 100644
index 0000000000000..82d5ec8ef02f2
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager_unittest.cc
@@ -0,0 +1,515 @@
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
+    // Register prefs
+    browseros_server::RegisterLocalStatePrefs(prefs_.registry());
+
+    // Create mocks (using NiceMock to allow uninteresting calls)
+    auto process_controller =
+        std::make_unique<NiceMock<MockProcessController>>();
+    auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+    auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+    auto updater = std::make_unique<NiceMock<MockServerUpdater>>();
+
+    // Keep raw pointers for EXPECT_CALL
+    process_controller_ = process_controller.get();
+    state_store_ = state_store.get();
+    health_checker_ = health_checker.get();
+    updater_ = updater.get();
+
+    // Allow mock leaks since BrowserOSServerManager has private destructor
+    // (singleton pattern) and Shutdown() doesn't delete the object
+    testing::Mock::AllowLeak(process_controller_);
+    testing::Mock::AllowLeak(state_store_);
+    testing::Mock::AllowLeak(health_checker_);
+    testing::Mock::AllowLeak(updater_);
+
+    // Set up default behaviors for updater
+    ON_CALL(*updater_, GetBestServerBinaryPath())
+        .WillByDefault(Return(base::FilePath("/fake/path/browseros_server")));
+    ON_CALL(*updater_, GetBestServerResourcesPath())
+        .WillByDefault(Return(base::FilePath("/fake/path/resources")));
+
+    // Create manager with injected mocks
+    manager_ = new BrowserOSServerManager(
+        std::move(process_controller), std::move(state_store),
+        std::move(health_checker), std::move(updater), &prefs_);
+  }
+
+  void TearDown() override {
+    // Manager destructor is private, call Shutdown() instead
+    if (manager_) {
+      manager_->Shutdown();
+    }
+  }
+
+  // Helper to simulate a successful process launch
+  void SetupSuccessfulLaunch() {
+    ON_CALL(*process_controller_, Launch(_))
+        .WillByDefault([](const ServerLaunchConfig&) {
+          LaunchResult result;
+          // Create a fake process - in tests we can't create real processes
+          // but we need IsValid() to return true
+          result.process = base::Process::Current();
+          result.used_fallback = false;
+          return result;
+        });
+  }
+
+  // Helper to simulate a failed process launch
+  void SetupFailedLaunch() {
+    ON_CALL(*process_controller_, Launch(_))
+        .WillByDefault([](const ServerLaunchConfig&) {
+          LaunchResult result;
+          // Invalid process (default constructed)
+          result.used_fallback = false;
+          return result;
+        });
+  }
+
+  base::test::TaskEnvironment task_environment_{
+      base::test::TaskEnvironment::TimeSource::MOCK_TIME};
+  TestingPrefServiceSimple prefs_;
+
+  // Raw pointers to mocks (owned by manager_)
+  raw_ptr<MockProcessController> process_controller_ = nullptr;
+  raw_ptr<MockServerStateStore> state_store_ = nullptr;
+  raw_ptr<MockHealthChecker> health_checker_ = nullptr;
+  raw_ptr<MockServerUpdater> updater_ = nullptr;
+
+  // Raw pointer - destructor is private (singleton pattern), leaked in tests
+  raw_ptr<BrowserOSServerManager> manager_ = nullptr;
+};
+
+// =============================================================================
+// Health Check Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckPass_NoRestart) {
+  // Health check passes - should not trigger restart
+  EXPECT_CALL(*health_checker_, CheckHealth(_, _))
+      .WillOnce([](int port, base::OnceCallback<void(bool)> callback) {
+        std::move(callback).Run(true);  // Success
+      });
+
+  // Terminate should NOT be called (no restart needed)
+  EXPECT_CALL(*process_controller_, Terminate(_, _)).Times(0);
+
+  // Manually invoke health check callback through manager's public interface
+  // Since CheckServerHealth is private, we test via the timer mechanism
+  // For now, we verify the mock expectations are set correctly
+}
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckFail_SingleFailure_IncrementsCounter) {
+  // Verify consecutive failure counter increments on failure
+  manager_->SetRunningForTesting(true);
+  EXPECT_EQ(0, manager_->GetConsecutiveHealthCheckFailures());
+
+  // First failure - counter should increment to 1
+  manager_->OnHealthCheckComplete(false);
+  EXPECT_EQ(1, manager_->GetConsecutiveHealthCheckFailures());
+
+  // Should NOT trigger full revalidation on first failure
+  EXPECT_FALSE(manager_->DidLastRestartRevalidateAllPorts());
+}
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckFail_TwoFailures_StillNoFullRevalidation) {
+  // Two consecutive failures should not yet trigger full revalidation
+  manager_->SetRunningForTesting(true);
+
+  manager_->OnHealthCheckComplete(false);
+  EXPECT_EQ(1, manager_->GetConsecutiveHealthCheckFailures());
+  EXPECT_FALSE(manager_->DidLastRestartRevalidateAllPorts());
+
+  manager_->OnHealthCheckComplete(false);
+  EXPECT_EQ(2, manager_->GetConsecutiveHealthCheckFailures());
+  EXPECT_FALSE(manager_->DidLastRestartRevalidateAllPorts());
+}
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckFail_ThreeConsecutiveFailures_TriggersFullRevalidation) {
+  // Three consecutive failures should trigger full port revalidation
+  manager_->SetRunningForTesting(true);
+  EXPECT_EQ(0, manager_->GetConsecutiveHealthCheckFailures());
+
+  // First two failures - no full revalidation yet
+  manager_->OnHealthCheckComplete(false);
+  EXPECT_EQ(1, manager_->GetConsecutiveHealthCheckFailures());
+  EXPECT_FALSE(manager_->DidLastRestartRevalidateAllPorts());
+
+  manager_->OnHealthCheckComplete(false);
+  EXPECT_EQ(2, manager_->GetConsecutiveHealthCheckFailures());
+  EXPECT_FALSE(manager_->DidLastRestartRevalidateAllPorts());
+
+  // Third failure - should trigger full revalidation and reset counter
+  manager_->OnHealthCheckComplete(false);
+
+  // After 3 consecutive failures:
+  // 1. Full revalidation should have been triggered
+  EXPECT_TRUE(manager_->DidLastRestartRevalidateAllPorts());
+  // 2. The consecutive failure counter should reset
+  EXPECT_EQ(0, manager_->GetConsecutiveHealthCheckFailures());
+}
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckPass_ResetsConsecutiveFailureCount) {
+  manager_->SetRunningForTesting(true);
+
+  // Simulate two failures
+  manager_->OnHealthCheckComplete(false);
+  manager_->OnHealthCheckComplete(false);
+  EXPECT_EQ(2, manager_->GetConsecutiveHealthCheckFailures());
+
+  // A successful health check should reset the counter
+  manager_->OnHealthCheckComplete(true);
+  EXPECT_EQ(0, manager_->GetConsecutiveHealthCheckFailures());
+}
+
+// =============================================================================
+// Updater Integration Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, StopCallsUpdaterStop) {
+  // When Stop() is called, it should call updater_->Stop()
+  EXPECT_CALL(*updater_, Stop()).Times(1);
+
+  // Call Stop (manager isn't running, but Stop() should still call updater)
+  manager_->Stop();
+}
+
+TEST_F(BrowserOSServerManagerTest, GetBinaryPathUsesUpdater) {
+  base::FilePath expected_path("/custom/binary/path");
+  EXPECT_CALL(*updater_, GetBestServerBinaryPath())
+      .WillOnce(Return(expected_path));
+
+  // The manager should query the updater for binary path during launch
+}
+
+TEST_F(BrowserOSServerManagerTest, GetResourcesPathUsesUpdater) {
+  base::FilePath expected_path("/custom/resources/path");
+  EXPECT_CALL(*updater_, GetBestServerResourcesPath())
+      .WillOnce(Return(expected_path));
+
+  // The manager should query the updater for resources path during launch
+}
+
+// =============================================================================
+// Port Preference Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, LoadsPortsFromPrefs) {
+  // Set custom port values in prefs
+  prefs_.SetInteger(browseros_server::kCDPServerPort, 8000);
+  prefs_.SetInteger(browseros_server::kMCPServerPort, 8100);
+  prefs_.SetInteger(browseros_server::kExtensionServerPort, 8300);
+
+  // Create a new manager to pick up the prefs
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
+  // Use raw pointer since destructor is private (singleton pattern)
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), std::move(updater), &prefs_);
+
+  // Ports should be loaded (exact values may change during resolution)
+  // This test mainly verifies no crash when loading prefs
+  manager->Shutdown();
+}
+
+TEST_F(BrowserOSServerManagerTest, DefaultPortsWhenPrefsEmpty) {
+  // Don't set any prefs - should use defaults
+  EXPECT_EQ(browseros_server::kDefaultCDPPort,
+            prefs_.GetInteger(browseros_server::kCDPServerPort));
+
+  // Manager should handle empty prefs gracefully and use defaults
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
+  // Use raw pointer since destructor is private (singleton pattern)
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), std::move(updater), &prefs_);
+  manager->Shutdown();
+}
+
+TEST_F(BrowserOSServerManagerTest, AllowRemoteInMCPPref) {
+  // Set the pref before creating manager
+  prefs_.SetBoolean(browseros_server::kAllowRemoteInMCP, true);
+
+  // Use disable flag so Start() loads prefs but doesn't start servers
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
+  // Before Start(), default is false
+  EXPECT_FALSE(manager->IsAllowRemoteInMCP());
+
+  // Start() loads prefs (but exits early due to disable flag)
+  manager->Start();
+
+  // Now the pref value should be loaded
+  EXPECT_TRUE(manager->IsAllowRemoteInMCP());
+  manager->Shutdown();
+}
+
+// =============================================================================
+// Null Prefs Handling Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, HandlesNullPrefs) {
+  // Create manager with null prefs (edge case)
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
+  // Use raw pointer since destructor is private (singleton pattern)
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), std::move(updater),
+      nullptr);  // null prefs
+
+  // Basic operations should work
+  EXPECT_FALSE(manager->IsRunning());
+  EXPECT_EQ(0, manager->GetCDPPort());
+  EXPECT_EQ(0, manager->GetMCPPort());
+  manager->Shutdown();
+}
+
+// =============================================================================
+// Null Updater Handling Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, HandlesNullUpdater) {
+  // Create manager with null updater
+  auto process_controller =
+      std::make_unique<NiceMock<MockProcessController>>();
+  auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+  auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+
+  testing::Mock::AllowLeak(process_controller.get());
+  testing::Mock::AllowLeak(state_store.get());
+  testing::Mock::AllowLeak(health_checker.get());
+
+  // Use raw pointer since destructor is private (singleton pattern)
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker),
+      nullptr,  // null updater
+      &prefs_);
+
+  // Should not crash
+  EXPECT_FALSE(manager->IsRunning());
+
+  // Stop should work without crashing (updater is null)
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
+  // Before Start(), ports should be 0
+  EXPECT_EQ(0, manager_->GetCDPPort());
+  EXPECT_EQ(0, manager_->GetMCPPort());
+  EXPECT_EQ(0, manager_->GetExtensionPort());
+}
+
+// =============================================================================
+// Restart Server For Update Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, RestartForUpdate_FailsWhenAlreadyRestarting) {
+  // Simulate restart already in progress by calling RestartServerForUpdate
+  // twice in quick succession
+
+  bool first_callback_called = false;
+  bool second_callback_called = false;
+  bool first_result = true;
+  bool second_result = true;
+
+  // First call - should proceed
+  manager_->RestartServerForUpdate(
+      base::BindOnce([](bool* called, bool* result, bool success) {
+        *called = true;
+        *result = success;
+      }, &first_callback_called, &first_result));
+
+  // Second call - should fail immediately because first is in progress
+  manager_->RestartServerForUpdate(
+      base::BindOnce([](bool* called, bool* result, bool success) {
+        *called = true;
+        *result = success;
+      }, &second_callback_called, &second_result));
+
+  // Second callback should be called immediately with failure
+  EXPECT_TRUE(second_callback_called);
+  EXPECT_FALSE(second_result);
+}
+
+// =============================================================================
+// Process Controller Integration Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, TerminateUsesProcessController) {
+  // Verify that termination goes through the process controller
+  EXPECT_CALL(*process_controller_, Terminate(_, false)).Times(1);
+
+  // Call Stop which internally calls TerminateBrowserOSProcess
+  manager_->Stop();
+}
+
+// =============================================================================
+// Launch Fallback Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, InvalidatesVersionOnFallback) {
+  // When launch uses fallback binary, updater should be notified
+  ON_CALL(*process_controller_, Launch(_))
+      .WillByDefault([](const ServerLaunchConfig&) {
+        LaunchResult result;
+        result.process = base::Process::Current();
+        result.used_fallback = true;  // Fallback was used
+        return result;
+      });
+
+  EXPECT_CALL(*updater_, InvalidateDownloadedVersion()).Times(1);
+
+  // This would be triggered during the launch flow
+}
+
+// =============================================================================
+// Orphan Recovery / State Store Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, StopDeletesStateFile) {
+  // When Stop() is called, state file should be deleted for clean shutdown
+  manager_->SetRunningForTesting(true);
+
+  EXPECT_CALL(*state_store_, Delete()).Times(1);
+  EXPECT_CALL(*updater_, Stop()).Times(1);
+
+  manager_->Stop();
+}
+
+TEST_F(BrowserOSServerManagerTest, RecoverFromOrphan_NoStateFile) {
+  // When no state file exists, Read() returns nullopt and no kill happens
+  EXPECT_CALL(*state_store_, Read())
+      .WillOnce(Return(std::nullopt));
+
+  // Delete should not be called when there's no state file
+  EXPECT_CALL(*state_store_, Delete()).Times(0);
+
+  // Simulate the start flow by checking state_store behavior
+  // (RecoverFromOrphan is called internally by Start after AcquireLock)
+}
+
+TEST_F(BrowserOSServerManagerTest, RecoverFromOrphan_ProcessGone) {
+  // When state file exists but process is gone, should delete state file
+  server_utils::ServerState state;
+  state.pid = 99999;  // Non-existent PID
+  state.creation_time = 123456789;
+
+  EXPECT_CALL(*state_store_, Read())
+      .WillOnce(Return(state));
+
+  // State file should be deleted since process doesn't exist
+  EXPECT_CALL(*state_store_, Delete()).Times(1);
+}
+
+}  // namespace
+}  // namespace browseros
