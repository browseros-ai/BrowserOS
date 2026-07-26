diff --git a/chrome/browser/browseros/server/browseros_server_config_unittest.cc b/chrome/browser/browseros/server/browseros_server_config_unittest.cc
new file mode 100644
index 0000000000000..9a25f32a0da56
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_config_unittest.cc
@@ -0,0 +1,228 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+
+#include <string_view>
+
+#include "base/test/scoped_command_line.h"
+#include "chrome/browser/browseros/buildflags.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "testing/gtest/include/gtest/gtest.h"
+
+namespace browseros {
+namespace {
+
+base::FilePath::StringType ToPathString(base::FilePath::StringViewType value) {
+  return base::FilePath::StringType(value.begin(), value.end());
+}
+
+Product BakedProduct() {
+#if BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)
+  return Product::kBrowserClaw;
+#else
+  return Product::kBrowserOS;
+#endif
+}
+
+Product OtherProduct() {
+  if (BakedProduct() == Product::kBrowserClaw) {
+    return Product::kBrowserOS;
+  }
+  return Product::kBrowserClaw;
+}
+
+std::string_view ProductSwitchValue(Product product) {
+  if (product == Product::kBrowserClaw) {
+    return "browserclaw";
+  }
+  return "browseros";
+}
+
+ServerLaunchConfig BuildLaunchConfig() {
+  ServerLaunchConfig config;
+  config.config_file_name = FILE_PATH_LITERAL("config.json");
+  config.health_path = "/system/health";
+  config.log_name = "test server";
+  config.enable_updater = true;
+
+  config.ports.cdp = 9222;
+  config.ports.server = 9230;
+  config.ports.proxy = 9250;
+
+  config.paths.exe = base::FilePath(FILE_PATH_LITERAL("server"));
+  config.paths.execution = base::FilePath(FILE_PATH_LITERAL("execution"));
+
+  config.identity.install_id = "install-id";
+  config.identity.browseros_version = "1.2.3";
+  config.identity.chromium_version = "140.0.0.0";
+  config.allow_remote_in_mcp = true;
+
+  return config;
+}
+
+TEST(BrowserOSProductTest, ReturnsBakedProductWithoutSwitch) {
+  EXPECT_EQ(BakedProduct(), GetProduct());
+}
+
+#if !BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+TEST(BrowserOSProductTest, IgnoresProductSwitchWhenOverrideDisabled) {
+  base::test::ScopedCommandLine scoped_command_line;
+  scoped_command_line.GetProcessCommandLine()->AppendSwitchASCII(
+      kBrowserOSProduct, ProductSwitchValue(OtherProduct()));
+
+  EXPECT_EQ(BakedProduct(), GetProduct());
+}
+#endif
+
+#if BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+TEST(BrowserOSProductTest, HonorsProductSwitchWhenOverrideEnabled) {
+  base::test::ScopedCommandLine scoped_command_line;
+  scoped_command_line.GetProcessCommandLine()->AppendSwitchASCII(
+      kBrowserOSProduct, ProductSwitchValue(OtherProduct()));
+
+  EXPECT_EQ(OtherProduct(), GetProduct());
+}
+
+TEST(BrowserOSProductTest, InvalidProductSwitchFallsBackToBakedProduct) {
+  base::test::ScopedCommandLine scoped_command_line;
+  scoped_command_line.GetProcessCommandLine()->AppendSwitchASCII(
+      kBrowserOSProduct, "invalid");
+
+  EXPECT_EQ(BakedProduct(), GetProduct());
+}
+#endif
+
+TEST(BrowserOSServerConfigTest, BrowserOSDescriptorMatchesLegacyServer) {
+  const ManagedServerDescriptor& descriptor = GetBrowserOSServerDescriptor();
+
+  EXPECT_EQ(Product::kBrowserOS, descriptor.product);
+  EXPECT_EQ(std::string_view("BrowserOS server"), descriptor.log_name);
+  EXPECT_EQ(base::FilePath::StringType(FILE_PATH_LITERAL("BrowserOSServer")),
+            ToPathString(descriptor.bundle_dir));
+  EXPECT_EQ(base::FilePath::StringType(FILE_PATH_LITERAL("browseros_server")),
+            ToPathString(descriptor.binary_name));
+  EXPECT_EQ(base::FilePath::StringType(FILE_PATH_LITERAL("config.json")),
+            ToPathString(descriptor.config_file_name));
+  EXPECT_EQ(std::string_view("/system/health"), descriptor.health_path);
+  EXPECT_TRUE(descriptor.enable_updater);
+
+  // Empty state dir preserves the legacy .browseros/current_version layout.
+  EXPECT_TRUE(descriptor.updater.state_dir.empty());
+  EXPECT_EQ(std::string_view("https://cdn.browseros.com/appcast-server.xml"),
+            descriptor.updater.appcast_url);
+  EXPECT_EQ(
+      std::string_view("https://cdn.browseros.com/appcast-server.alpha.xml"),
+      descriptor.updater.alpha_appcast_url);
+  EXPECT_EQ(std::string_view("/status"), descriptor.updater.readiness_path);
+}
+
+TEST(BrowserOSServerConfigTest, BrowserClawDescriptorMatchesClawServer) {
+  const ManagedServerDescriptor& descriptor = GetBrowserClawServerDescriptor();
+
+  EXPECT_EQ(Product::kBrowserClaw, descriptor.product);
+  EXPECT_EQ(std::string_view("BrowserClaw server"), descriptor.log_name);
+  EXPECT_EQ(base::FilePath::StringType(FILE_PATH_LITERAL("BrowserClawServer")),
+            ToPathString(descriptor.bundle_dir));
+  EXPECT_EQ(
+      base::FilePath::StringType(FILE_PATH_LITERAL("browseros-claw-server")),
+      ToPathString(descriptor.binary_name));
+  EXPECT_EQ(base::FilePath::StringType(FILE_PATH_LITERAL("config.json")),
+            ToPathString(descriptor.config_file_name));
+  EXPECT_EQ(std::string_view("/system/health"), descriptor.health_path);
+  EXPECT_TRUE(descriptor.enable_updater);
+
+  // Claw isolates OTA state under .browseros/BrowserClawServer/ and fetches its
+  // own feed.
+  EXPECT_EQ(base::FilePath::StringType(FILE_PATH_LITERAL("BrowserClawServer")),
+            ToPathString(descriptor.updater.state_dir));
+  EXPECT_EQ(
+      std::string_view("https://cdn.browseros.com/appcast-claw-server.xml"),
+      descriptor.updater.appcast_url);
+  EXPECT_EQ(std::string_view(
+                "https://cdn.browseros.com/appcast-claw-server.alpha.xml"),
+            descriptor.updater.alpha_appcast_url);
+  // No readiness contract yet: empty path skips the status fetch.
+  EXPECT_TRUE(descriptor.updater.readiness_path.empty());
+}
+
+TEST(BrowserOSServerConfigTest, ManagedDescriptorUsesSelectedProduct) {
+  const ManagedServerDescriptor& descriptor = GetManagedServerDescriptor();
+
+  EXPECT_EQ(GetProduct(), descriptor.product);
+}
+
+#if BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+TEST(BrowserOSServerConfigTest, ManagedDescriptorFollowsRuntimeOverride) {
+  base::test::ScopedCommandLine scoped_command_line;
+  scoped_command_line.GetProcessCommandLine()->AppendSwitchASCII(
+      kBrowserOSProduct, ProductSwitchValue(OtherProduct()));
+
+  const ManagedServerDescriptor& descriptor = GetManagedServerDescriptor();
+
+  EXPECT_EQ(OtherProduct(), descriptor.product);
+}
+#endif
+
+TEST(BrowserOSServerConfigTest, ServerConfigJsonHasUnifiedShape) {
+  ServerLaunchConfig config = BuildLaunchConfig();
+  base::FilePath resources(FILE_PATH_LITERAL("resources"));
+
+  base::DictValue root = BuildServerConfigJson(config, resources);
+
+  const base::DictValue* ports = root.FindDict("ports");
+  ASSERT_NE(nullptr, ports);
+  ASSERT_TRUE(ports->FindInt("cdp").has_value());
+  EXPECT_EQ(9222, ports->FindInt("cdp").value());
+  ASSERT_TRUE(ports->FindInt("server").has_value());
+  EXPECT_EQ(9230, ports->FindInt("server").value());
+  ASSERT_TRUE(ports->FindInt("proxy").has_value());
+  EXPECT_EQ(9250, ports->FindInt("proxy").value());
+
+  const base::DictValue* directories = root.FindDict("directories");
+  ASSERT_NE(nullptr, directories);
+  ASSERT_NE(nullptr, directories->FindString("resources"));
+  EXPECT_EQ(resources.AsUTF8Unsafe(), *directories->FindString("resources"));
+  ASSERT_NE(nullptr, directories->FindString("execution"));
+  EXPECT_EQ(config.paths.execution.AsUTF8Unsafe(),
+            *directories->FindString("execution"));
+
+  const base::DictValue* flags = root.FindDict("flags");
+  ASSERT_NE(nullptr, flags);
+  ASSERT_TRUE(flags->FindBool("allow_remote_in_mcp").has_value());
+  EXPECT_TRUE(flags->FindBool("allow_remote_in_mcp").value());
+
+  const base::DictValue* instance = root.FindDict("instance");
+  ASSERT_NE(nullptr, instance);
+  ASSERT_NE(nullptr, instance->FindString("install_id"));
+  EXPECT_EQ("install-id", *instance->FindString("install_id"));
+  ASSERT_NE(nullptr, instance->FindString("browseros_version"));
+  EXPECT_EQ("1.2.3", *instance->FindString("browseros_version"));
+  ASSERT_NE(nullptr, instance->FindString("chromium_version"));
+  EXPECT_EQ("140.0.0.0", *instance->FindString("chromium_version"));
+}
+
+// Both BrowserOS and BrowserClaw consume this same config.json. Guard against
+// re-stripping the shape per product: the keys the old Claw config omitted
+// (ports.proxy, directories.execution, flags, instance) must stay present.
+TEST(BrowserOSServerConfigTest, ServerConfigJsonIncludesSharedRuntimeKeys) {
+  ServerLaunchConfig config = BuildLaunchConfig();
+  base::FilePath resources(FILE_PATH_LITERAL("resources"));
+
+  base::DictValue root = BuildServerConfigJson(config, resources);
+
+  const base::DictValue* ports = root.FindDict("ports");
+  ASSERT_NE(nullptr, ports);
+  EXPECT_TRUE(ports->FindInt("proxy").has_value());
+
+  const base::DictValue* directories = root.FindDict("directories");
+  ASSERT_NE(nullptr, directories);
+  EXPECT_NE(nullptr, directories->FindString("execution"));
+
+  EXPECT_NE(nullptr, root.FindDict("flags"));
+  EXPECT_NE(nullptr, root.FindDict("instance"));
+}
+
+}  // namespace
+}  // namespace browseros
