diff --git a/chrome/browser/devtools/protocol/history_handler.h b/chrome/browser/devtools/protocol/history_handler.h
new file mode 100644
index 0000000000000..7cde94b91e242
--- /dev/null
+++ b/chrome/browser/devtools/protocol/history_handler.h
@@ -0,0 +1,64 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_DEVTOOLS_PROTOCOL_HISTORY_HANDLER_H_
+#define CHROME_BROWSER_DEVTOOLS_PROTOCOL_HISTORY_HANDLER_H_
+
+#include <memory>
+#include <string>
+
+#include "base/memory/weak_ptr.h"
+#include "base/task/cancelable_task_tracker.h"
+#include "chrome/browser/devtools/protocol/history.h"
+
+class Profile;
+
+namespace history {
+class HistoryService;
+class QueryResults;
+}  // namespace history
+
+class HistoryHandler : public protocol::History::Backend {
+ public:
+  HistoryHandler(protocol::UberDispatcher* dispatcher,
+                 const std::string& target_id);
+
+  HistoryHandler(const HistoryHandler&) = delete;
+  HistoryHandler& operator=(const HistoryHandler&) = delete;
+
+  ~HistoryHandler() override;
+
+  // History::Backend (all async):
+  void Search(const std::string& query,
+              std::optional<int> max_results,
+              std::optional<double> start_time,
+              std::optional<double> end_time,
+              std::unique_ptr<SearchCallback> callback) override;
+  void GetRecent(std::optional<int> max_results,
+                 std::unique_ptr<GetRecentCallback> callback) override;
+  void DeleteUrl(const std::string& url,
+                 std::unique_ptr<DeleteUrlCallback> callback) override;
+  void DeleteRange(double start_time,
+                   double end_time,
+                   std::unique_ptr<DeleteRangeCallback> callback) override;
+
+ private:
+  Profile* GetProfile() const;
+  history::HistoryService* GetHistoryService() const;
+
+  void OnSearchComplete(
+      std::unique_ptr<SearchCallback> callback,
+      history::QueryResults results);
+  void OnGetRecentComplete(
+      std::unique_ptr<GetRecentCallback> callback,
+      history::QueryResults results);
+  void OnDeleteRangeComplete(
+      std::unique_ptr<DeleteRangeCallback> callback);
+
+  const std::string target_id_;
+  base::CancelableTaskTracker task_tracker_;
+  base::WeakPtrFactory<HistoryHandler> weak_factory_{this};
+};
+
+#endif  // CHROME_BROWSER_DEVTOOLS_PROTOCOL_HISTORY_HANDLER_H_
