diff --git a/chrome/browser/devtools/protocol/devtools_protocol_browsertest.cc b/chrome/browser/devtools/protocol/devtools_protocol_browsertest.cc
index fdf0973f53f6b26814596bcdcd0e2a2caa29727c..abdb300bbe0e60fa83d2f19810dbe9867bc8c77a 100644
--- a/chrome/browser/devtools/protocol/devtools_protocol_browsertest.cc
+++ b/chrome/browser/devtools/protocol/devtools_protocol_browsertest.cc
@@ -18,11 +18,14 @@
 #include "base/strings/to_string.h"
 #include "base/strings/utf_string_conversions.h"
 #include "base/test/bind.h"
+#include "base/test/run_until.h"
 #include "base/test/scoped_feature_list.h"
 #include "base/test/test_future.h"
 #include "base/test/test_switches.h"
 #include "base/test/values_test_util.h"
 #include "base/threading/thread_restrictions.h"
+#include "base/time/time.h"
+#include "base/uuid.h"
 #include "base/values.h"
 #include "build/build_config.h"
 #include "chrome/browser/apps/app_service/app_service_proxy.h"
@@ -30,14 +33,17 @@
 #include "chrome/browser/apps/app_service/browser_app_launcher.h"
 #include "chrome/browser/custom_handlers/protocol_handler_registry_factory.h"
 #include "chrome/browser/data_saver/data_saver.h"
+#include "chrome/browser/devtools/devtools_browser_context_manager.h"
 #include "chrome/browser/devtools/devtools_window.h"
 #include "chrome/browser/devtools/protocol/devtools_protocol_test_support.h"
 #include "chrome/browser/first_party_sets/first_party_sets_policy_service.h"
 #include "chrome/browser/first_party_sets/first_party_sets_policy_service_factory.h"
+#include "chrome/browser/history/history_service_factory.h"
 #include "chrome/browser/preloading/preloading_prefs.h"
 #include "chrome/browser/privacy_sandbox/privacy_sandbox_attestations/privacy_sandbox_attestations_mixin.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ssl/https_upgrades_util.h"
+#include "chrome/browser/tab_group_sync/tab_group_sync_service_factory.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/common/chrome_paths.h"
 #include "chrome/common/chrome_switches.h"
@@ -48,12 +54,17 @@
 #include "components/content_settings/core/browser/cookie_settings.h"
 #include "components/content_settings/core/common/pref_names.h"
 #include "components/custom_handlers/protocol_handler_registry.h"
+#include "components/history/core/browser/history_service.h"
+#include "components/history/core/test/history_service_test_util.h"
 #include "components/infobars/content/content_infobar_manager.h"
 #include "components/infobars/core/infobar.h"
 #include "components/infobars/core/infobar_delegate.h"
 #include "components/optimization_guide/proto/features/common_quality_data.pb.h"
 #include "components/privacy_sandbox/privacy_sandbox_attestations/privacy_sandbox_attestations.h"
 #include "components/privacy_sandbox/privacy_sandbox_prefs.h"
+#include "components/saved_tab_groups/public/saved_tab_group.h"
+#include "components/saved_tab_groups/public/saved_tab_group_tab.h"
+#include "components/saved_tab_groups/public/tab_group_sync_service.h"
 #include "components/services/app_service/public/cpp/app_launch_params.h"
 #include "content/public/browser/btm_redirect.h"
 #include "content/public/browser/btm_service.h"
@@ -109,6 +120,7 @@
 #include "chrome/browser/sessions/session_restore_test_helper.h"
 #include "chrome/browser/sessions/session_service_test_helper.h"
 #include "chrome/browser/ui/browser_commands.h"
+#include "chrome/browser/ui/browser_window.h"
 #include "chrome/browser/ui/browser_window/public/global_browser_collection.h"
 #include "chrome/browser/ui/tabs/tab_strip_model.h"
 #include "chrome/browser/ui/web_applications/test/isolated_web_app_test_utils.h"
@@ -480,8 +492,8 @@ IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest, CreateTargetWithFocus) {
 }
 
 #if BUILDFLAG(IS_CHROMEOS) || BUILDFLAG(IS_MAC)
-// On ChromeOS and MacOS, the tabs are not backgrounded and unloaded in the same way as
-// on other platforms.
+// On ChromeOS and MacOS, the tabs are not backgrounded and unloaded in the same
+// way as on other platforms.
 #define MAYBE_AutoAttachToUnloadedTab DISABLED_AutoAttachToUnloadedTab
 #else
 #define MAYBE_AutoAttachToUnloadedTab AutoAttachToUnloadedTab
@@ -1497,8 +1509,9 @@ IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest, VisibleSecurityStateSecureState) {
   net::SSLCipherSuiteToStrings(&page_key_exchange_str, &page_cipher, &page_mac,
                                &is_aead, &is_tls13, page_cipher_suite);
   std::string page_key_exchange;
-  if (page_key_exchange_str)
+  if (page_key_exchange_str) {
     page_key_exchange = page_key_exchange_str;
+  }
 
   const char* page_key_exchange_group =
       SSL_get_curve_name(entry->GetSSL().key_exchange_group);
@@ -2681,6 +2694,442 @@ IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
   EXPECT_EQ(group_id.ToString(),
             *second_get_target_info_data.FindString("tabGroupId"));
 }
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       PRE_BrowserClosedTabGroupCanBeDeletedAfterRestart) {
+  auto* service = tab_groups::TabGroupSyncServiceFactory::GetForProfile(
+      browser()->GetProfile());
+  ASSERT_TRUE(service);
+  base::RunLoop().RunUntilIdle();
+  const auto guid =
+      base::Uuid::ParseLowercase("a12bdd40-84d3-49d2-9e59-bac416973651");
+  tab_groups::SavedTabGroup group(u"Saved before restart",
+                                  tab_groups::TabGroupColorId::kBlue, {},
+                                  std::nullopt, guid);
+  group.AddTabLocally(tab_groups::SavedTabGroupTab(GURL("https://example.com"),
+                                                   u"Example", guid, 0));
+  service->AddGroup(group);
+  ASSERT_TRUE(base::test::RunUntil(
+      [&] { return service->GetGroup(guid).has_value(); }));
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserClosedTabGroupCanBeDeletedAfterRestart) {
+  AttachToBrowserTarget();
+  auto* service = tab_groups::TabGroupSyncServiceFactory::GetForProfile(
+      browser()->GetProfile());
+  ASSERT_TRUE(service);
+  base::RunLoop().RunUntilIdle();
+  const auto guid =
+      base::Uuid::ParseLowercase("a12bdd40-84d3-49d2-9e59-bac416973651");
+  ASSERT_TRUE(base::test::RunUntil(
+      [&] { return service->GetGroup(guid).has_value(); }));
+  SendCommandSync("Browser.getClosedTabGroups");
+  ASSERT_FALSE(error());
+  const auto* groups = result()->FindList("groups");
+  ASSERT_TRUE(groups);
+  std::string profile_id;
+  for (const auto& value : *groups) {
+    const auto& group = value.GetDict();
+    if (group.FindString("savedGroupId") &&
+        *group.FindString("savedGroupId") == guid.AsLowercaseString()) {
+      profile_id = *group.FindString("browserProfileId");
+      break;
+    }
+  }
+  ASSERT_FALSE(profile_id.empty());
+  SendCommandSync("Browser.deleteClosedTabGroup",
+                  base::DictValue()
+                      .Set("browserProfileId", profile_id)
+                      .Set("savedGroupId", guid.AsLowercaseString()));
+  ASSERT_FALSE(error());
+  EXPECT_FALSE(service->GetGroup(guid));
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserClosedTabGroupDeletionUsesExactSavedIdentity) {
+  AttachToBrowserTarget();
+  auto* service = tab_groups::TabGroupSyncServiceFactory::GetForProfile(
+      browser()->GetProfile());
+  ASSERT_TRUE(service);
+  base::RunLoop().RunUntilIdle();
+
+  const auto make_group = [] {
+    tab_groups::SavedTabGroup group(u"Same title",
+                                    tab_groups::TabGroupColorId::kBlue, {});
+    group.AddTabLocally(tab_groups::SavedTabGroupTab(
+        GURL("https://example.com"), u"Example", group.saved_guid(), 0));
+    return group;
+  };
+  const auto first = make_group();
+  const auto second = make_group();
+  service->AddGroup(first);
+  service->AddGroup(second);
+  ASSERT_TRUE(base::test::RunUntil([&] {
+    return service->GetGroup(first.saved_guid()).has_value() &&
+           service->GetGroup(second.saved_guid()).has_value();
+  }));
+
+  SendCommandSync("Browser.getClosedTabGroups");
+  ASSERT_FALSE(error());
+  const auto* groups = result()->FindList("groups");
+  ASSERT_TRUE(groups);
+  std::string profile_id;
+  for (const auto& entry : *groups) {
+    const auto& group = entry.GetDict();
+    if (group.FindString("savedGroupId") &&
+        *group.FindString("savedGroupId") ==
+            first.saved_guid().AsLowercaseString()) {
+      ASSERT_TRUE(group.FindString("browserProfileId"));
+      profile_id = *group.FindString("browserProfileId");
+      EXPECT_EQ(group.FindInt("tabCount"), 1);
+      EXPECT_EQ(group.FindBool("shared"), false);
+    }
+  }
+  ASSERT_FALSE(profile_id.empty());
+
+  SendCommandSync(
+      "Browser.deleteClosedTabGroup",
+      base::DictValue()
+          .Set("browserProfileId", "unrelated-profile")
+          .Set("savedGroupId", first.saved_guid().AsLowercaseString()));
+  ASSERT_TRUE(error());
+  EXPECT_TRUE(service->GetGroup(first.saved_guid()));
+
+  SendCommandSync("Browser.deleteClosedTabGroup",
+                  base::DictValue()
+                      .Set("browserProfileId", profile_id)
+                      .Set("savedGroupId", "not-a-uuid"));
+  ASSERT_TRUE(error());
+  EXPECT_TRUE(service->GetGroup(first.saved_guid()));
+
+  SendCommandSync(
+      "Browser.deleteClosedTabGroup",
+      base::DictValue()
+          .Set("browserProfileId", profile_id)
+          .Set("savedGroupId", first.saved_guid().AsLowercaseString()));
+  ASSERT_FALSE(error());
+  EXPECT_FALSE(service->GetGroup(first.saved_guid()));
+  EXPECT_TRUE(service->GetGroup(second.saved_guid()));
+
+  // A stale/repeated delete fails without selecting a same-titled sibling.
+  SendCommandSync(
+      "Browser.deleteClosedTabGroup",
+      base::DictValue()
+          .Set("browserProfileId", profile_id)
+          .Set("savedGroupId", first.saved_guid().AsLowercaseString()));
+  ASSERT_TRUE(error());
+  EXPECT_TRUE(service->GetGroup(second.saved_guid()));
+}
+
+IN_PROC_BROWSER_TEST_F(
+    DevToolsProtocolTest,
+    BrowserClosedTabGroupDeletionRefusesOpenAndSharedGroups) {
+  AttachToBrowserTarget();
+  auto* service = tab_groups::TabGroupSyncServiceFactory::GetForProfile(
+      browser()->GetProfile());
+  ASSERT_TRUE(service);
+  base::RunLoop().RunUntilIdle();
+  ASSERT_TRUE(AddTabAtIndex(1, GURL("about:blank"),
+                            ui::PageTransition::PAGE_TRANSITION_TYPED));
+  const auto local_id = browser()->tab_strip_model()->AddToNewGroup({1});
+  base::RunLoop().RunUntilIdle();
+  ASSERT_TRUE(base::test::RunUntil(
+      [&] { return service->GetGroup(local_id).has_value(); }));
+  const auto saved = service->GetGroup(local_id);
+  ASSERT_TRUE(saved);
+  const std::string saved_id = saved->saved_guid().AsLowercaseString();
+
+  const auto profile_handle =
+      DevToolsBrowserContextManager::GetInstance().GetBrowserProfileId(
+          browser()->GetProfile());
+  ASSERT_TRUE(profile_handle);
+  const std::string profile_id = *profile_handle;
+  const auto delete_params = [&] {
+    return base::DictValue()
+        .Set("browserProfileId", profile_id)
+        .Set("savedGroupId", saved_id);
+  };
+  SendCommandSync("Browser.deleteClosedTabGroup", delete_params());
+  ASSERT_TRUE(error());
+  EXPECT_TRUE(service->GetGroup(local_id));
+  EXPECT_EQ(browser()->tab_strip_model()->count(), 2);
+
+  // Close keeps the saved record. It can then be found and explicitly deleted.
+  SendCommandSync(
+      "Browser.closeTabGroup",
+      base::DictValue().Set("groupId", local_id.token().ToString()));
+  ASSERT_FALSE(error());
+  base::RunLoop().RunUntilIdle();
+  ASSERT_TRUE(base::test::RunUntil([&] {
+    auto closed = service->GetGroup(saved->saved_guid());
+    return closed && !closed->local_group_id();
+  }));
+  EXPECT_EQ(browser()->tab_strip_model()->count(), 1);
+
+  // Verify that a real close is visible through the closed-group listing.
+  SendCommandSync("Browser.getClosedTabGroups");
+  ASSERT_FALSE(error());
+  ASSERT_TRUE(result());
+  const base::ListValue* closed_groups = result()->FindList("groups");
+  ASSERT_TRUE(closed_groups);
+  const base::DictValue* closed_group_info = nullptr;
+  for (const auto& entry : *closed_groups) {
+    const base::DictValue& candidate = entry.GetDict();
+    const std::string* candidate_id = candidate.FindString("savedGroupId");
+    if (candidate_id && *candidate_id == saved_id) {
+      closed_group_info = &candidate;
+      break;
+    }
+  }
+  ASSERT_TRUE(closed_group_info);
+  ASSERT_TRUE(closed_group_info->FindString("browserProfileId"));
+  EXPECT_EQ(profile_id, *closed_group_info->FindString("browserProfileId"));
+  EXPECT_THAT(closed_group_info->FindInt("tabCount"), testing::Optional(1));
+  EXPECT_THAT(closed_group_info->FindBool("shared"), testing::Optional(false));
+
+  SendCommandSync("Browser.deleteClosedTabGroup", delete_params());
+  ASSERT_FALSE(error());
+  EXPECT_FALSE(service->GetGroup(saved->saved_guid()));
+  EXPECT_EQ(browser()->tab_strip_model()->count(), 1);
+
+  ASSERT_TRUE(AddTabAtIndex(1, GURL("about:blank"),
+                            ui::PageTransition::PAGE_TRANSITION_TYPED));
+  const auto shared_local_id = browser()->tab_strip_model()->AddToNewGroup({1});
+  base::RunLoop().RunUntilIdle();
+  ASSERT_TRUE(base::test::RunUntil(
+      [&] { return service->GetGroup(shared_local_id).has_value(); }));
+  service->MakeTabGroupSharedForTesting(
+      shared_local_id, syncer::CollaborationId("test-collaboration"));
+  const auto shared = service->GetGroup(shared_local_id);
+  ASSERT_TRUE(shared);
+  SendCommandSync(
+      "Browser.closeTabGroup",
+      base::DictValue().Set("groupId", shared_local_id.token().ToString()));
+  ASSERT_FALSE(error());
+  base::RunLoop().RunUntilIdle();
+  ASSERT_TRUE(base::test::RunUntil([&] {
+    auto closed = service->GetGroup(shared->saved_guid());
+    return closed && !closed->local_group_id();
+  }));
+  SendCommandSync(
+      "Browser.deleteClosedTabGroup",
+      base::DictValue()
+          .Set("browserProfileId", profile_id)
+          .Set("savedGroupId", shared->saved_guid().AsLowercaseString()));
+  ASSERT_TRUE(error());
+  EXPECT_TRUE(service->GetGroup(shared->saved_guid()));
+  EXPECT_EQ(browser()->tab_strip_model()->count(), 1);
+}
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserCreateWindowRejectsHiddenWithoutSideEffects) {
+  AttachToBrowserTarget();
+  const size_t initial_browser_count =
+      GlobalBrowserCollection::GetInstance()->GetSize();
+
+  SendCommandSync(
+      "Browser.createWindow",
+      base::DictValue().Set("url", "about:blank").Set("hidden", true));
+
+  ASSERT_TRUE(error());
+  EXPECT_THAT(error()->FindInt("code"), testing::Optional(-32602));
+  EXPECT_EQ("Hidden windows are no longer supported.",
+            *error()->FindString("message"));
+  EXPECT_EQ(initial_browser_count,
+            GlobalBrowserCollection::GetInstance()->GetSize());
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest, BrowserCloseWindowClosesOnce) {
+  AttachToBrowserTarget();
+  const size_t initial_browser_count =
+      GlobalBrowserCollection::GetInstance()->GetSize();
+
+  const base::DictValue* result = SendCommandSync(
+      "Browser.createWindow", base::DictValue().Set("url", "about:blank"));
+  ASSERT_TRUE(result);
+  const base::DictValue* window = result->FindDict("window");
+  ASSERT_TRUE(window);
+  const std::optional<int> window_id = window->FindInt("windowId");
+  ASSERT_TRUE(window_id.has_value());
+  ASSERT_EQ(initial_browser_count + 1,
+            GlobalBrowserCollection::GetInstance()->GetSize());
+
+  ui_test_utils::BrowserDestroyedObserver observer;
+  SendCommandSync("Browser.closeWindow",
+                  base::DictValue().Set("windowId", *window_id));
+  ASSERT_FALSE(error());
+  observer.Wait();
+  EXPECT_EQ(initial_browser_count,
+            GlobalBrowserCollection::GetInstance()->GetSize());
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserSetWindowVisibilityRejectsHideWithoutMutation) {
+  AttachToBrowserTarget();
+  ASSERT_TRUE(browser()->GetWindow()->IsVisible());
+
+  SendCommandSync("Browser.setWindowVisibility",
+                  base::DictValue()
+                      .Set("windowId", browser()->GetSessionID().id())
+                      .Set("visible", false));
+
+  ASSERT_TRUE(error());
+  EXPECT_THAT(error()->FindInt("code"), testing::Optional(-32602));
+  EXPECT_EQ("Hidden windows are no longer supported.",
+            *error()->FindString("message"));
+  EXPECT_TRUE(browser()->GetWindow()->IsVisible());
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserSetWindowVisibilityShowsInPlace) {
+  AttachToBrowserTarget();
+  const int window_id = browser()->GetSessionID().id();
+
+  const base::DictValue* result = SendCommandSync(
+      "Browser.setWindowVisibility", base::DictValue()
+                                         .Set("windowId", window_id)
+                                         .Set("visible", true)
+                                         .Set("activate", false));
+
+  ASSERT_TRUE(result);
+  ASSERT_FALSE(error());
+  EXPECT_THAT(result->FindBool("replaced"), testing::Optional(false));
+  EXPECT_THAT(result->FindInt("previousWindowId"),
+              testing::Optional(window_id));
+  const base::DictValue* window = result->FindDict("window");
+  ASSERT_TRUE(window);
+  EXPECT_THAT(window->FindInt("windowId"), testing::Optional(window_id));
+  EXPECT_THAT(window->FindBool("isVisible"), testing::Optional(true));
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserGetTabsAcceptsRetiredIncludeHiddenFlag) {
+  AttachToBrowserTarget();
+
+  const base::DictValue* result = SendCommandSync(
+      "Browser.getTabs", base::DictValue().Set("includeHidden", true));
+
+  ASSERT_TRUE(result);
+  ASSERT_FALSE(error());
+  const base::ListValue* tabs = result->FindList("tabs");
+  ASSERT_TRUE(tabs);
+  ASSERT_FALSE(tabs->empty());
+  for (const auto& value : *tabs) {
+    const base::DictValue& tab = value.GetDict();
+    EXPECT_THAT(tab.FindBool("isHidden"), testing::Optional(false));
+    EXPECT_TRUE(tab.FindInt("windowId").has_value());
+    EXPECT_TRUE(tab.FindInt("index").has_value());
+  }
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserShowTabRejectsWithoutMutation) {
+  AttachToBrowserTarget();
+  const int initial_tab_count = browser()->tab_strip_model()->count();
+
+  const base::DictValue* tabs_result = SendCommandSync("Browser.getTabs");
+  ASSERT_TRUE(tabs_result);
+  const base::ListValue* tabs = tabs_result->FindList("tabs");
+  ASSERT_TRUE(tabs);
+  ASSERT_FALSE(tabs->empty());
+  const std::optional<int> tab_id = tabs->front().GetDict().FindInt("tabId");
+  ASSERT_TRUE(tab_id.has_value());
+
+  SendCommandSync("Browser.showTab", base::DictValue().Set("tabId", *tab_id));
+
+  ASSERT_TRUE(error());
+  EXPECT_THAT(error()->FindInt("code"), testing::Optional(-32602));
+  EXPECT_EQ("Hidden tabs are no longer supported.",
+            *error()->FindString("message"));
+  EXPECT_EQ(initial_tab_count, browser()->tab_strip_model()->count());
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       CreateTabGroupAcceptsUnsortedTabIds) {
+  AttachToBrowserTarget();
+
+  ASSERT_EQ(1, browser()->tab_strip_model()->count());
+
+  base::DictValue params;
+  params.Set("url", "about:blank");
+  params.Set("background", true);
+  ASSERT_TRUE(SendCommandSync("Browser.createTab", params.Clone()));
+  ASSERT_TRUE(SendCommandSync("Browser.createTab", std::move(params)));
+
+  const base::DictValue* tabs_result = SendCommandSync("Browser.getTabs");
+  ASSERT_TRUE(tabs_result);
+  const base::ListValue* tabs = tabs_result->FindList("tabs");
+  ASSERT_TRUE(tabs);
+  ASSERT_EQ(3u, tabs->size());
+
+  std::vector<int> tab_ids;
+  tab_ids.reserve(tabs->size());
+  for (const auto& tab : *tabs) {
+    tab_ids.push_back(*tab.GetDict().FindInt("tabId"));
+  }
+
+  base::ListValue unsorted_tab_ids;
+  unsorted_tab_ids.Append(tab_ids[2]);
+  unsorted_tab_ids.Append(tab_ids[0]);
+
+  base::DictValue create_group_params;
+  create_group_params.Set("tabIds", std::move(unsorted_tab_ids));
+  create_group_params.Set("title", "Unsorted");
+
+  const base::DictValue* create_group_result =
+      SendCommandSync("Browser.createTabGroup", std::move(create_group_params));
+  ASSERT_TRUE(create_group_result);
+  ASSERT_FALSE(error());
+
+  const base::DictValue* group = create_group_result->FindDict("group");
+  ASSERT_TRUE(group);
+  const base::ListValue* grouped_tab_ids = group->FindList("tabIds");
+  ASSERT_TRUE(grouped_tab_ids);
+  ASSERT_EQ(2u, grouped_tab_ids->size());
+  EXPECT_EQ(tab_ids[0], *grouped_tab_ids->front().GetIfInt());
+  EXPECT_EQ(tab_ids[2], *grouped_tab_ids->back().GetIfInt());
+  EXPECT_EQ("Unsorted", *group->FindString("title"));
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest, HistorySearchUsesVisitTime) {
+  AttachToBrowserTarget();
+
+  history::HistoryService* history_service =
+      HistoryServiceFactory::GetForProfile(browser()->GetProfile(),
+                                           ServiceAccessType::EXPLICIT_ACCESS);
+  ui_test_utils::WaitForHistoryToLoad(history_service);
+
+  const GURL url("https://history-timestamp-test.example/path");
+  const base::Time older_visit = base::Time::Now() - base::Days(2);
+  const base::Time newer_visit = base::Time::Now() - base::Hours(1);
+
+  history_service->AddPage(url, older_visit, history::SOURCE_BROWSED);
+  history_service->AddPage(url, newer_visit, history::SOURCE_BROWSED);
+  history::BlockUntilHistoryProcessesPendingRequests(history_service);
+
+  base::DictValue search_params;
+  search_params.Set("query", "");
+  search_params.Set(
+      "startTime",
+      (older_visit - base::Minutes(1)).InMillisecondsFSinceUnixEpoch());
+  search_params.Set(
+      "endTime",
+      (newer_visit - base::Minutes(1)).InMillisecondsFSinceUnixEpoch());
+
+  const base::DictValue* search_result =
+      SendCommandSync("History.search", std::move(search_params));
+  ASSERT_TRUE(search_result);
+  ASSERT_FALSE(error());
+
+  const base::ListValue* entries = search_result->FindList("entries");
+  ASSERT_TRUE(entries);
+  ASSERT_EQ(1u, entries->size());
+
+  const base::DictValue& entry = entries->front().GetDict();
+  EXPECT_EQ(url.spec(), *entry.FindString("url"));
+  EXPECT_EQ(older_visit.InMillisecondsFSinceUnixEpoch(),
+            *entry.FindDouble("lastVisitTime"));
+}
 #endif  // !BUILDFLAG(IS_ANDROID)
 
 #if !BUILDFLAG(IS_ANDROID)
