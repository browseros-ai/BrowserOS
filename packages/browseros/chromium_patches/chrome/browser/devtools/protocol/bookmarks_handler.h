diff --git a/chrome/browser/devtools/protocol/bookmarks_handler.h b/chrome/browser/devtools/protocol/bookmarks_handler.h
new file mode 100644
index 0000000000000..110f607602b08
--- /dev/null
+++ b/chrome/browser/devtools/protocol/bookmarks_handler.h
@@ -0,0 +1,64 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_DEVTOOLS_PROTOCOL_BOOKMARKS_HANDLER_H_
+#define CHROME_BROWSER_DEVTOOLS_PROTOCOL_BOOKMARKS_HANDLER_H_
+
+#include <memory>
+#include <string>
+
+#include "chrome/browser/devtools/protocol/bookmarks.h"
+
+class Profile;
+
+namespace bookmarks {
+class BookmarkModel;
+}
+
+class BookmarksHandler : public protocol::Bookmarks::Backend {
+ public:
+  BookmarksHandler(protocol::UberDispatcher* dispatcher,
+                   const std::string& target_id);
+
+  BookmarksHandler(const BookmarksHandler&) = delete;
+  BookmarksHandler& operator=(const BookmarksHandler&) = delete;
+
+  ~BookmarksHandler() override;
+
+  // Bookmarks::Backend:
+  protocol::Response GetBookmarks(
+      std::optional<std::string> folder_id,
+      std::unique_ptr<protocol::Array<protocol::Bookmarks::BookmarkNode>>*
+          out_nodes) override;
+  protocol::Response SearchBookmarks(
+      const std::string& query,
+      std::optional<int> max_results,
+      std::unique_ptr<protocol::Array<protocol::Bookmarks::BookmarkNode>>*
+          out_results) override;
+  protocol::Response CreateBookmark(
+      const std::string& title,
+      std::optional<std::string> url,
+      std::optional<std::string> parent_id,
+      std::optional<int> index,
+      std::unique_ptr<protocol::Bookmarks::BookmarkNode>* out_node) override;
+  protocol::Response UpdateBookmark(
+      const std::string& id,
+      std::optional<std::string> title,
+      std::optional<std::string> url,
+      std::unique_ptr<protocol::Bookmarks::BookmarkNode>* out_node) override;
+  protocol::Response MoveBookmark(
+      const std::string& id,
+      std::optional<std::string> parent_id,
+      std::optional<int> index,
+      std::unique_ptr<protocol::Bookmarks::BookmarkNode>* out_node) override;
+  protocol::Response RemoveBookmark(const std::string& id) override;
+
+ private:
+  Profile* GetProfile() const;
+  bookmarks::BookmarkModel* GetBookmarkModel() const;
+
+  const std::string target_id_;
+};
+
+#endif  // CHROME_BROWSER_DEVTOOLS_PROTOCOL_BOOKMARKS_HANDLER_H_
