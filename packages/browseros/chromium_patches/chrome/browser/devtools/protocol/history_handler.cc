diff --git a/chrome/browser/devtools/protocol/history_handler.cc b/chrome/browser/devtools/protocol/history_handler.cc
new file mode 100644
index 0000000000000..4087a679a527f
--- /dev/null
+++ b/chrome/browser/devtools/protocol/history_handler.cc
@@ -0,0 +1,188 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/devtools/protocol/history_handler.h"
+
+#include <string>
+#include <vector>
+
+#include "base/functional/bind.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/time/time.h"
+#include "chrome/browser/history/history_service_factory.h"
+#include "chrome/browser/profiles/profile.h"
+#include "components/history/core/browser/history_service.h"
+#include "components/history/core/browser/history_types.h"
+#include "components/history/core/browser/url_row.h"
+#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
+#include "chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h"
+#include "content/public/browser/devtools_agent_host.h"
+
+using protocol::Response;
+
+namespace {
+
+std::unique_ptr<protocol::History::HistoryEntry> BuildHistoryEntry(
+    const history::URLResult& result) {
+  return protocol::History::HistoryEntry::Create()
+      .SetId(base::NumberToString(result.id()))
+      .SetUrl(result.url().spec())
+      .SetTitle(base::UTF16ToUTF8(result.title()))
+      .SetLastVisitTime(result.visit_time().InMillisecondsFSinceUnixEpoch())
+      .SetVisitCount(result.visit_count())
+      .SetTypedCount(result.typed_count())
+      .Build();
+}
+
+}  // namespace
+
+HistoryHandler::HistoryHandler(protocol::UberDispatcher* dispatcher,
+                               const std::string& target_id)
+    : target_id_(target_id) {
+  protocol::History::Dispatcher::wire(dispatcher, this);
+}
+
+HistoryHandler::~HistoryHandler() = default;
+
+Profile* HistoryHandler::GetProfile() const {
+  auto host = content::DevToolsAgentHost::GetForId(target_id_);
+  if (host && host->GetBrowserContext()) {
+    return Profile::FromBrowserContext(host->GetBrowserContext());
+  }
+  // Browser-level targets have no BrowserContext; fall back to active window.
+  BrowserWindowInterface* bwi =
+      GetLastActiveBrowserWindowInterfaceWithAnyProfile();
+  return bwi ? bwi->GetProfile() : nullptr;
+}
+
+history::HistoryService* HistoryHandler::GetHistoryService() const {
+  Profile* profile = GetProfile();
+  return profile ? HistoryServiceFactory::GetForProfile(
+                       profile, ServiceAccessType::EXPLICIT_ACCESS)
+                 : nullptr;
+}
+
+void HistoryHandler::Search(const std::string& query,
+                            std::optional<int> max_results,
+                            std::optional<double> start_time,
+                            std::optional<double> end_time,
+                            std::unique_ptr<SearchCallback> callback) {
+  history::HistoryService* service = GetHistoryService();
+  if (!service) {
+    std::move(callback)->sendFailure(
+        Response::ServerError("History service not available"));
+    return;
+  }
+
+  history::QueryOptions options;
+  options.max_count = max_results.value_or(100);
+  options.duplicate_policy = history::QueryOptions::REMOVE_ALL_DUPLICATES;
+
+  if (start_time.has_value()) {
+    options.begin_time =
+        base::Time::FromMillisecondsSinceUnixEpoch(start_time.value());
+  }
+  if (end_time.has_value()) {
+    options.end_time =
+        base::Time::FromMillisecondsSinceUnixEpoch(end_time.value());
+  }
+
+  service->QueryHistory(
+      base::UTF8ToUTF16(query), options,
+      base::BindOnce(&HistoryHandler::OnSearchComplete,
+                     weak_factory_.GetWeakPtr(), std::move(callback)),
+      &task_tracker_);
+}
+
+void HistoryHandler::GetRecent(std::optional<int> max_results,
+                               std::unique_ptr<GetRecentCallback> callback) {
+  history::HistoryService* service = GetHistoryService();
+  if (!service) {
+    std::move(callback)->sendFailure(
+        Response::ServerError("History service not available"));
+    return;
+  }
+
+  history::QueryOptions options;
+  options.max_count = max_results.value_or(100);
+  options.duplicate_policy = history::QueryOptions::REMOVE_ALL_DUPLICATES;
+
+  service->QueryHistory(
+      std::u16string(), options,
+      base::BindOnce(&HistoryHandler::OnGetRecentComplete,
+                     weak_factory_.GetWeakPtr(), std::move(callback)),
+      &task_tracker_);
+}
+
+void HistoryHandler::DeleteUrl(const std::string& url,
+                               std::unique_ptr<DeleteUrlCallback> callback) {
+  history::HistoryService* service = GetHistoryService();
+  if (!service) {
+    std::move(callback)->sendFailure(
+        Response::ServerError("History service not available"));
+    return;
+  }
+
+  GURL gurl(url);
+  if (!gurl.is_valid()) {
+    std::move(callback)->sendFailure(
+        Response::InvalidParams("URL must not be empty"));
+    return;
+  }
+
+  service->DeleteURLs({gurl});
+  std::move(callback)->sendSuccess();
+}
+
+void HistoryHandler::DeleteRange(
+    double start_time,
+    double end_time,
+    std::unique_ptr<DeleteRangeCallback> callback) {
+  history::HistoryService* service = GetHistoryService();
+  if (!service) {
+    std::move(callback)->sendFailure(
+        Response::ServerError("History service not available"));
+    return;
+  }
+
+  base::Time begin =
+      base::Time::FromMillisecondsSinceUnixEpoch(start_time);
+  base::Time end = base::Time::FromMillisecondsSinceUnixEpoch(end_time);
+
+  service->ExpireHistoryBetween(
+      /*restrict_urls=*/{},
+      /*restrict_app_id=*/std::nullopt, begin, end,
+      /*user_initiated=*/true,
+      base::BindOnce(&HistoryHandler::OnDeleteRangeComplete,
+                     weak_factory_.GetWeakPtr(), std::move(callback)),
+      &task_tracker_);
+}
+
+void HistoryHandler::OnSearchComplete(
+    std::unique_ptr<SearchCallback> callback,
+    history::QueryResults results) {
+  auto entries =
+      std::make_unique<protocol::Array<protocol::History::HistoryEntry>>();
+  for (const auto& result : results) {
+    entries->push_back(BuildHistoryEntry(result));
+  }
+  std::move(callback)->sendSuccess(std::move(entries));
+}
+
+void HistoryHandler::OnGetRecentComplete(
+    std::unique_ptr<GetRecentCallback> callback,
+    history::QueryResults results) {
+  auto entries =
+      std::make_unique<protocol::Array<protocol::History::HistoryEntry>>();
+  for (const auto& result : results) {
+    entries->push_back(BuildHistoryEntry(result));
+  }
+  std::move(callback)->sendSuccess(std::move(entries));
+}
+
+void HistoryHandler::OnDeleteRangeComplete(
+    std::unique_ptr<DeleteRangeCallback> callback) {
+  std::move(callback)->sendSuccess();
+}
