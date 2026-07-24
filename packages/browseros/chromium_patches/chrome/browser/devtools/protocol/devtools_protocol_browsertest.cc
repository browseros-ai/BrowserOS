diff --git a/chrome/browser/devtools/protocol/devtools_protocol_browsertest.cc b/chrome/browser/devtools/protocol/devtools_protocol_browsertest.cc
index 6a3a8ba7d79978dfd618056fe23f31fe4696c375..1910213b6b81c583aff4c5fbf172406ccd52432e 100644
--- a/chrome/browser/devtools/protocol/devtools_protocol_browsertest.cc
+++ b/chrome/browser/devtools/protocol/devtools_protocol_browsertest.cc
@@ -21,6 +21,7 @@
 #include "base/test/test_switches.h"
 #include "base/test/values_test_util.h"
 #include "base/threading/thread_restrictions.h"
+#include "base/time/time.h"
 #include "base/values.h"
 #include "build/build_config.h"
 #include "chrome/browser/apps/app_service/app_service_proxy.h"
@@ -30,6 +31,7 @@
 #include "chrome/browser/data_saver/data_saver.h"
 #include "chrome/browser/devtools/devtools_window.h"
 #include "chrome/browser/devtools/protocol/devtools_protocol_test_support.h"
+#include "chrome/browser/history/history_service_factory.h"
 #include "chrome/browser/preloading/preloading_prefs.h"
 #include "chrome/browser/privacy_sandbox/privacy_sandbox_attestations/privacy_sandbox_attestations_mixin.h"
 #include "chrome/browser/profiles/profile.h"
@@ -41,6 +43,8 @@
 #include "components/content_settings/core/browser/cookie_settings.h"
 #include "components/content_settings/core/common/pref_names.h"
 #include "components/custom_handlers/protocol_handler_registry.h"
+#include "components/history/core/browser/history_service.h"
+#include "components/history/core/test/history_service_test_util.h"
 #include "components/infobars/content/content_infobar_manager.h"
 #include "components/infobars/core/infobar.h"
 #include "components/infobars/core/infobar_delegate.h"
@@ -89,6 +93,7 @@
 #include "chrome/browser/ui/browser.h"
 #include "chrome/browser/ui/browser_commands.h"
 #include "chrome/browser/ui/browser_finder.h"
+#include "chrome/browser/ui/browser_window.h"
 #include "chrome/browser/ui/web_applications/test/isolated_web_app_test_utils.h"
 #include "chrome/browser/web_applications/isolated_web_apps/isolated_web_app_url_info.h"
 #include "chrome/browser/web_applications/isolated_web_apps/test/isolated_web_app_builder.h"
@@ -2121,6 +2126,215 @@ IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
   SendCommandSync("Target.getTargets");
   EXPECT_EQ(2u, result()->FindList("targetInfos")->size());
 }
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserCreateWindowRejectsHiddenWithoutSideEffects) {
+  AttachToBrowserTarget();
+  const size_t initial_browser_count = chrome::GetTotalBrowserCount();
+
+  SendCommandSync(
+      "Browser.createWindow",
+      base::DictValue().Set("url", "about:blank").Set("hidden", true));
+
+  ASSERT_TRUE(error());
+  EXPECT_THAT(error()->FindInt("code"), testing::Optional(-32602));
+  EXPECT_EQ("Hidden windows are no longer supported.",
+            *error()->FindString("message"));
+  EXPECT_EQ(initial_browser_count, chrome::GetTotalBrowserCount());
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest, BrowserCloseWindowClosesOnce) {
+  AttachToBrowserTarget();
+  const size_t initial_browser_count = chrome::GetTotalBrowserCount();
+
+  const base::DictValue* result = SendCommandSync(
+      "Browser.createWindow", base::DictValue().Set("url", "about:blank"));
+  ASSERT_TRUE(result);
+  const base::DictValue* window = result->FindDict("window");
+  ASSERT_TRUE(window);
+  const std::optional<int> window_id = window->FindInt("windowId");
+  ASSERT_TRUE(window_id.has_value());
+  ASSERT_EQ(initial_browser_count + 1, chrome::GetTotalBrowserCount());
+
+  ui_test_utils::BrowserDestroyedObserver observer;
+  SendCommandSync("Browser.closeWindow",
+                  base::DictValue().Set("windowId", *window_id));
+  ASSERT_FALSE(error());
+  observer.Wait();
+  EXPECT_EQ(initial_browser_count, chrome::GetTotalBrowserCount());
+}
+
+IN_PROC_BROWSER_TEST_F(
+    DevToolsProtocolTest,
+    BrowserSetWindowVisibilityRejectsHideWithoutMutation) {
+  AttachToBrowserTarget();
+  ASSERT_TRUE(browser()->window()->IsVisible());
+
+  SendCommandSync(
+      "Browser.setWindowVisibility",
+      base::DictValue()
+          .Set("windowId", browser()->session_id().id())
+          .Set("visible", false));
+
+  ASSERT_TRUE(error());
+  EXPECT_THAT(error()->FindInt("code"), testing::Optional(-32602));
+  EXPECT_EQ("Hidden windows are no longer supported.",
+            *error()->FindString("message"));
+  EXPECT_TRUE(browser()->window()->IsVisible());
+}
+
+IN_PROC_BROWSER_TEST_F(DevToolsProtocolTest,
+                       BrowserSetWindowVisibilityShowsInPlace) {
+  AttachToBrowserTarget();
+  const int window_id = browser()->session_id().id();
+
+  const base::DictValue* result = SendCommandSync(
+      "Browser.setWindowVisibility",
+      base::DictValue()
+          .Set("windowId", window_id)
+          .Set("visible", true)
+          .Set("activate", false));
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
+  SendCommandSync("Browser.showTab",
+                  base::DictValue().Set("tabId", *tab_id));
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
+      HistoryServiceFactory::GetForProfile(browser()->profile(),
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
