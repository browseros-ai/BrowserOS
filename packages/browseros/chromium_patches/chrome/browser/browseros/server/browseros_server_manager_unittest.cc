diff --git a/chrome/browser/browseros/server/browseros_server_manager_unittest.cc b/chrome/browser/browseros/server/browseros_server_manager_unittest.cc
new file mode 100644
index 0000000000000..aee8a1fadceaf
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager_unittest.cc
@@ -0,0 +1,543 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
+
+#include <memory>
+#include <set>
+
+#include "base/command_line.h"
+#include "base/functional/bind.h"
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
+using ::testing::NiceMock;
+using ::testing::Return;
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
+  ServerPorts MakePorts(int server_port) {
+    ServerPorts ports;
+    ports.cdp = browseros_server::kDefaultCDPPort;
+    ports.proxy = browseros_server::kDefaultProxyPort;
+    ports.server = server_port;
+    return ports;
+  }
+
+  void UseFakePortFinder(int return_port = 0) {
+    fake_port_finder_return_port_ = return_port;
+    manager_->SetPortFinderForTesting(base::BindRepeating(
+        &BrowserOSServerManagerTest::FindPortForTesting,
+        base::Unretained(this)));
+  }
+
+  int FindPortForTesting(int starting_port,
+                         const std::set<int>& excluded,
+                         bool allow_reuse) {
+    port_finder_call_count_++;
+    port_finder_starting_port_ = starting_port;
+    port_finder_excluded_ = excluded;
+    port_finder_allow_reuse_ = allow_reuse;
+    return fake_port_finder_return_port_ == 0 ? starting_port
+                                              : fake_port_finder_return_port_;
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
+
+  int port_finder_call_count_ = 0;
+  int port_finder_starting_port_ = 0;
+  std::set<int> port_finder_excluded_;
+  bool port_finder_allow_reuse_ = false;
+  int fake_port_finder_return_port_ = 0;
+};
+
+// =============================================================================
+// Health Check Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckPass_NoRestart) {
+  manager_->SetRunningForTesting(true);
+  manager_->SetPortsForTesting(MakePorts(browseros_server::kDefaultServerPort));
+  UseFakePortFinder();
+
+  manager_->OnHealthCheckComplete(true);
+  task_environment_.RunUntilIdle();
+
+  EXPECT_EQ(0, port_finder_call_count_);
+}
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckFail_RestartsAfterTwoStrikes) {
+  SetupSuccessfulLaunch();
+  manager_->SetRunningForTesting(true);
+  manager_->SetPortsForTesting(MakePorts(browseros_server::kDefaultServerPort));
+  UseFakePortFinder();
+
+  // First failure is only a warning strike.
+  manager_->OnHealthCheckComplete(false);
+  task_environment_.RunUntilIdle();
+  EXPECT_EQ(0, port_finder_call_count_);
+
+  // Second consecutive failure triggers the managed restart.
+  manager_->OnHealthCheckComplete(false);
+  task_environment_.RunUntilIdle();
+  EXPECT_EQ(1, port_finder_call_count_);
+}
+
+TEST_F(BrowserOSServerManagerTest, HealthCheckPass_ResetsFailureStrikes) {
+  SetupSuccessfulLaunch();
+  manager_->SetRunningForTesting(true);
+  manager_->SetPortsForTesting(MakePorts(browseros_server::kDefaultServerPort));
+  UseFakePortFinder();
+
+  manager_->OnHealthCheckComplete(false);
+  manager_->OnHealthCheckComplete(true);
+  manager_->OnHealthCheckComplete(false);
+  task_environment_.RunUntilIdle();
+  EXPECT_EQ(0, port_finder_call_count_);
+
+  manager_->OnHealthCheckComplete(false);
+  task_environment_.RunUntilIdle();
+  EXPECT_EQ(1, port_finder_call_count_);
+}
+
+// =============================================================================
+// Updater Integration Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, StopCallsUpdaterStop) {
+  manager_->SetRunningForTesting(true);
+  EXPECT_CALL(*updater_, Stop()).Times(1);
+  manager_->Stop();
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
+  auto process_controller = std::make_unique<NiceMock<MockProcessController>>();
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
+  auto process_controller = std::make_unique<NiceMock<MockProcessController>>();
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
+  auto process_controller = std::make_unique<NiceMock<MockProcessController>>();
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
+  auto process_controller = std::make_unique<NiceMock<MockProcessController>>();
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
+  auto process_controller = std::make_unique<NiceMock<MockProcessController>>();
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
+      std::move(health_checker), std::move(updater), nullptr);
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
+  auto process_controller = std::make_unique<NiceMock<MockProcessController>>();
+  auto state_store = std::make_unique<NiceMock<MockServerStateStore>>();
+  auto health_checker = std::make_unique<NiceMock<MockHealthChecker>>();
+
+  testing::Mock::AllowLeak(process_controller.get());
+  testing::Mock::AllowLeak(state_store.get());
+  testing::Mock::AllowLeak(health_checker.get());
+
+  auto* manager = new BrowserOSServerManager(
+      std::move(process_controller), std::move(state_store),
+      std::move(health_checker), nullptr, &prefs_);
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
+  EXPECT_EQ(0, manager_->GetServerPort());
+}
+
+// =============================================================================
+// Restart Server For Update Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest,
+       RestartForUpdate_FailsWhenAlreadyRestarting) {
+  manager_->SetPortsForTesting(MakePorts(browseros_server::kDefaultServerPort));
+  UseFakePortFinder();
+
+  bool first_callback_called = false;
+  bool second_callback_called = false;
+  bool first_result = true;
+  bool second_result = true;
+
+  manager_->RestartServerForUpdate(base::BindOnce(
+      [](bool* called, bool* result, bool success) {
+        *called = true;
+        *result = success;
+      },
+      &first_callback_called, &first_result));
+
+  manager_->RestartServerForUpdate(base::BindOnce(
+      [](bool* called, bool* result, bool success) {
+        *called = true;
+        *result = success;
+      },
+      &second_callback_called, &second_result));
+
+  EXPECT_TRUE(second_callback_called);
+  EXPECT_FALSE(second_result);
+}
+
+// =============================================================================
+// Process Controller Integration Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, StopWithoutProcessDoesNotTerminate) {
+  EXPECT_CALL(*process_controller_, Terminate(_, _)).Times(0);
+  EXPECT_CALL(*process_controller_, Kill(_, _)).Times(0);
+  manager_->Stop();
+}
+
+// =============================================================================
+// State Store Tests
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
+// =============================================================================
+// Restart Saves Ports to Prefs Tests
+// =============================================================================
+
+TEST_F(BrowserOSServerManagerTest, ManagedRestartKeepsServerPortWithReuse) {
+  SetupSuccessfulLaunch();
+  manager_->SetRunningForTesting(true);
+  manager_->SetPortsForTesting(MakePorts(browseros_server::kDefaultServerPort));
+  UseFakePortFinder(browseros_server::kDefaultServerPort);
+
+  // Trigger restart via two consecutive health check failures.
+  manager_->OnHealthCheckComplete(false);
+  manager_->OnHealthCheckComplete(false);
+
+  // Run all pending tasks (thread pool + reply)
+  task_environment_.RunUntilIdle();
+
+  EXPECT_EQ(1, port_finder_call_count_);
+  EXPECT_EQ(browseros_server::kDefaultServerPort, port_finder_starting_port_);
+  EXPECT_EQ(1u, port_finder_excluded_.count(browseros_server::kDefaultCDPPort));
+  EXPECT_EQ(1u,
+            port_finder_excluded_.count(browseros_server::kDefaultProxyPort));
+  EXPECT_TRUE(port_finder_allow_reuse_);
+  EXPECT_EQ(manager_->GetServerPort(),
+            prefs_.GetInteger(browseros_server::kServerPort));
+  EXPECT_EQ(browseros_server::kDefaultServerPort,
+            prefs_.GetInteger(browseros_server::kServerPort));
+}
+
+TEST_F(BrowserOSServerManagerTest,
+       PortConflictExitAdvancesServerPortWithoutReuse) {
+  SetupSuccessfulLaunch();
+  manager_->SetRunningForTesting(true);
+  manager_->SetPortsForTesting(MakePorts(browseros_server::kDefaultServerPort));
+  UseFakePortFinder(browseros_server::kDefaultServerPort + 1);
+
+  manager_->OnProcessExitedForTesting(2);
+  task_environment_.RunUntilIdle();
+
+  EXPECT_EQ(1, port_finder_call_count_);
+  EXPECT_EQ(browseros_server::kDefaultServerPort + 1,
+            port_finder_starting_port_);
+  EXPECT_FALSE(port_finder_allow_reuse_);
+  EXPECT_EQ(browseros_server::kDefaultServerPort + 1,
+            manager_->GetServerPort());
+  EXPECT_EQ(browseros_server::kDefaultServerPort + 1,
+            prefs_.GetInteger(browseros_server::kServerPort));
+}
+
+TEST_F(BrowserOSServerManagerTest,
+       PortConflictAdvanceConsumedThenManagedRestartKeepsPort) {
+  SetupSuccessfulLaunch();
+  manager_->SetRunningForTesting(true);
+  manager_->SetPortsForTesting(MakePorts(browseros_server::kDefaultServerPort));
+  UseFakePortFinder(browseros_server::kDefaultServerPort + 1);
+
+  manager_->OnProcessExitedForTesting(2);
+  task_environment_.RunUntilIdle();
+
+  EXPECT_EQ(1, port_finder_call_count_);
+  EXPECT_EQ(browseros_server::kDefaultServerPort + 1,
+            port_finder_starting_port_);
+  EXPECT_FALSE(port_finder_allow_reuse_);
+  EXPECT_EQ(browseros_server::kDefaultServerPort + 1,
+            manager_->GetServerPort());
+
+  manager_->OnHealthCheckComplete(false);
+  manager_->OnHealthCheckComplete(false);
+  task_environment_.RunUntilIdle();
+
+  EXPECT_EQ(2, port_finder_call_count_);
+  EXPECT_EQ(browseros_server::kDefaultServerPort + 1,
+            port_finder_starting_port_);
+  EXPECT_TRUE(port_finder_allow_reuse_);
+  EXPECT_EQ(browseros_server::kDefaultServerPort + 1,
+            manager_->GetServerPort());
+}
+
+TEST_F(BrowserOSServerManagerTest, UpdateRestartKeepsServerPortWithReuse) {
+  SetupSuccessfulLaunch();
+  manager_->SetRunningForTesting(true);
+  manager_->SetPortsForTesting(MakePorts(browseros_server::kDefaultServerPort));
+  UseFakePortFinder(browseros_server::kDefaultServerPort);
+
+  bool callback_called = false;
+  bool callback_result = false;
+  manager_->RestartServerForUpdate(base::BindOnce(
+      [](bool* called, bool* result, bool success) {
+        *called = true;
+        *result = success;
+      },
+      &callback_called, &callback_result));
+
+  task_environment_.RunUntilIdle();
+
+  EXPECT_EQ(1, port_finder_call_count_);
+  EXPECT_EQ(browseros_server::kDefaultServerPort, port_finder_starting_port_);
+  EXPECT_TRUE(port_finder_allow_reuse_);
+  EXPECT_EQ(manager_->GetServerPort(),
+            prefs_.GetInteger(browseros_server::kServerPort));
+  EXPECT_EQ(browseros_server::kDefaultServerPort,
+            prefs_.GetInteger(browseros_server::kServerPort));
+}
+
+}  // namespace
+}  // namespace browseros
