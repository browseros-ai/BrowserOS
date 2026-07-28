diff --git a/chrome/browser/devtools/protocol/bookmarks_handler.cc b/chrome/browser/devtools/protocol/bookmarks_handler.cc
new file mode 100644
index 0000000000000..7b1e0ef01fe5e
--- /dev/null
+++ b/chrome/browser/devtools/protocol/bookmarks_handler.cc
@@ -0,0 +1,304 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/devtools/protocol/bookmarks_handler.h"
+
+#include <string>
+#include <vector>
+
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/bookmarks/bookmark_model_factory.h"
+#include "chrome/browser/profiles/profile.h"
+#include "components/bookmarks/browser/bookmark_model.h"
+#include "components/bookmarks/browser/bookmark_node.h"
+#include "components/bookmarks/browser/bookmark_utils.h"
+#include "components/bookmarks/browser/titled_url_match.h"
+#include "components/query_parser/query_parser.h"
+#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
+#include "chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h"
+#include "content/public/browser/devtools_agent_host.h"
+
+using protocol::Response;
+using bookmarks::BookmarkModel;
+using bookmarks::BookmarkNode;
+
+namespace {
+
+std::unique_ptr<protocol::Bookmarks::BookmarkNode> BuildBookmarkNode(
+    const BookmarkNode* node) {
+  auto result = protocol::Bookmarks::BookmarkNode::Create()
+                    .SetId(base::NumberToString(node->id()))
+                    .SetTitle(base::UTF16ToUTF8(node->GetTitle()))
+                    .SetType(node->is_folder()
+                                 ? protocol::Bookmarks::BookmarkNodeTypeEnum::Folder
+                                 : protocol::Bookmarks::BookmarkNodeTypeEnum::Url)
+                    .SetDateAdded(node->date_added().InMillisecondsFSinceUnixEpoch())
+                    .Build();
+
+  if (node->parent() && !node->parent()->is_permanent_node()) {
+    result->SetParentId(base::NumberToString(node->parent()->id()));
+  }
+
+  if (node->parent()) {
+    auto index = node->parent()->GetIndexOf(node);
+    if (index.has_value()) {
+      result->SetIndex(static_cast<int>(index.value()));
+    }
+  }
+
+  if (!node->is_folder()) {
+    result->SetUrl(node->url().spec());
+  }
+
+  if (!node->date_last_used().is_null()) {
+    result->SetDateLastUsed(
+        node->date_last_used().InMillisecondsFSinceUnixEpoch());
+  }
+
+  return result;
+}
+
+void FlattenBookmarkNode(
+    const BookmarkNode* node,
+    protocol::Array<protocol::Bookmarks::BookmarkNode>* out) {
+  for (const auto& child : node->children()) {
+    out->push_back(BuildBookmarkNode(child.get()));
+    if (child->is_folder()) {
+      FlattenBookmarkNode(child.get(), out);
+    }
+  }
+}
+
+const BookmarkNode* FindNodeById(BookmarkModel* model,
+                                 const std::string& id_str) {
+  int64_t node_id;
+  if (!base::StringToInt64(id_str, &node_id)) {
+    return nullptr;
+  }
+  return bookmarks::GetBookmarkNodeByID(model, node_id);
+}
+
+}  // namespace
+
+BookmarksHandler::BookmarksHandler(protocol::UberDispatcher* dispatcher,
+                                   const std::string& target_id)
+    : target_id_(target_id) {
+  protocol::Bookmarks::Dispatcher::wire(dispatcher, this);
+}
+
+BookmarksHandler::~BookmarksHandler() = default;
+
+Profile* BookmarksHandler::GetProfile() const {
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
+BookmarkModel* BookmarksHandler::GetBookmarkModel() const {
+  Profile* profile = GetProfile();
+  return profile ? BookmarkModelFactory::GetForBrowserContext(profile) : nullptr;
+}
+
+Response BookmarksHandler::GetBookmarks(
+    std::optional<std::string> folder_id,
+    std::unique_ptr<protocol::Array<protocol::Bookmarks::BookmarkNode>>*
+        out_nodes) {
+  BookmarkModel* model = GetBookmarkModel();
+  if (!model || !model->loaded()) {
+    return Response::ServerError("Bookmark model not loaded");
+  }
+
+  auto nodes =
+      std::make_unique<protocol::Array<protocol::Bookmarks::BookmarkNode>>();
+
+  if (folder_id.has_value()) {
+    const BookmarkNode* folder = FindNodeById(model, folder_id.value());
+    if (!folder) {
+      return Response::ServerError("No bookmark with given id");
+    }
+    if (!folder->is_folder()) {
+      return Response::InvalidParams("Node is not a folder");
+    }
+    FlattenBookmarkNode(folder, nodes.get());
+  } else {
+    FlattenBookmarkNode(model->bookmark_bar_node(), nodes.get());
+    FlattenBookmarkNode(model->other_node(), nodes.get());
+    FlattenBookmarkNode(model->mobile_node(), nodes.get());
+  }
+
+  *out_nodes = std::move(nodes);
+  return Response::Success();
+}
+
+Response BookmarksHandler::SearchBookmarks(
+    const std::string& query,
+    std::optional<int> max_results,
+    std::unique_ptr<protocol::Array<protocol::Bookmarks::BookmarkNode>>*
+        out_results) {
+  BookmarkModel* model = GetBookmarkModel();
+  if (!model || !model->loaded()) {
+    return Response::ServerError("Bookmark model not loaded");
+  }
+
+  size_t max_count = max_results.has_value()
+                         ? static_cast<size_t>(max_results.value())
+                         : 100;
+
+  std::vector<bookmarks::TitledUrlMatch> matches =
+      model->GetBookmarksMatching(
+          base::UTF8ToUTF16(query), max_count,
+          query_parser::MatchingAlgorithm::DEFAULT);
+
+  auto results =
+      std::make_unique<protocol::Array<protocol::Bookmarks::BookmarkNode>>();
+  for (const auto& match : matches) {
+    const BookmarkNode* node =
+        static_cast<const BookmarkNode*>(match.node);
+    results->push_back(BuildBookmarkNode(node));
+  }
+
+  *out_results = std::move(results);
+  return Response::Success();
+}
+
+Response BookmarksHandler::CreateBookmark(
+    const std::string& title,
+    std::optional<std::string> url,
+    std::optional<std::string> parent_id,
+    std::optional<int> index,
+    std::unique_ptr<protocol::Bookmarks::BookmarkNode>* out_node) {
+  BookmarkModel* model = GetBookmarkModel();
+  if (!model || !model->loaded()) {
+    return Response::ServerError("Bookmark model not loaded");
+  }
+
+  const BookmarkNode* parent = model->bookmark_bar_node();
+  if (parent_id.has_value()) {
+    parent = FindNodeById(model, parent_id.value());
+    if (!parent) {
+      return Response::ServerError("No bookmark with given id");
+    }
+    if (!parent->is_folder()) {
+      return Response::InvalidParams("Parent is not a folder");
+    }
+  }
+
+  size_t insert_index = index.has_value()
+                             ? static_cast<size_t>(index.value())
+                             : parent->children().size();
+
+  const BookmarkNode* new_node = nullptr;
+  if (url.has_value()) {
+    GURL bookmark_url(url.value());
+    if (!bookmark_url.is_valid()) {
+      return Response::InvalidParams("Invalid URL");
+    }
+    new_node = model->AddNewURL(parent, insert_index,
+                                base::UTF8ToUTF16(title), bookmark_url);
+  } else {
+    new_node =
+        model->AddFolder(parent, insert_index, base::UTF8ToUTF16(title));
+  }
+
+  if (!new_node) {
+    return Response::ServerError("Failed to create bookmark");
+  }
+
+  *out_node = BuildBookmarkNode(new_node);
+  return Response::Success();
+}
+
+Response BookmarksHandler::UpdateBookmark(
+    const std::string& id,
+    std::optional<std::string> title,
+    std::optional<std::string> url,
+    std::unique_ptr<protocol::Bookmarks::BookmarkNode>* out_node) {
+  BookmarkModel* model = GetBookmarkModel();
+  if (!model || !model->loaded()) {
+    return Response::ServerError("Bookmark model not loaded");
+  }
+
+  const BookmarkNode* node = FindNodeById(model, id);
+  if (!node) {
+    return Response::ServerError("No bookmark with given id");
+  }
+
+  if (title.has_value()) {
+    model->SetTitle(node, base::UTF8ToUTF16(title.value()),
+                    bookmarks::metrics::BookmarkEditSource::kOther);
+  }
+
+  if (url.has_value()) {
+    if (node->is_folder()) {
+      return Response::InvalidParams("Cannot set URL on a folder");
+    }
+    GURL new_url(url.value());
+    if (!new_url.is_valid()) {
+      return Response::InvalidParams("Invalid URL");
+    }
+    model->SetURL(node, new_url,
+                  bookmarks::metrics::BookmarkEditSource::kOther);
+  }
+
+  *out_node = BuildBookmarkNode(node);
+  return Response::Success();
+}
+
+Response BookmarksHandler::MoveBookmark(
+    const std::string& id,
+    std::optional<std::string> parent_id,
+    std::optional<int> index,
+    std::unique_ptr<protocol::Bookmarks::BookmarkNode>* out_node) {
+  BookmarkModel* model = GetBookmarkModel();
+  if (!model || !model->loaded()) {
+    return Response::ServerError("Bookmark model not loaded");
+  }
+
+  const BookmarkNode* node = FindNodeById(model, id);
+  if (!node) {
+    return Response::ServerError("No bookmark with given id");
+  }
+
+  const BookmarkNode* new_parent = node->parent();
+  if (parent_id.has_value()) {
+    new_parent = FindNodeById(model, parent_id.value());
+    if (!new_parent) {
+      return Response::ServerError("No bookmark with given parent id");
+    }
+    if (!new_parent->is_folder()) {
+      return Response::InvalidParams("Parent is not a folder");
+    }
+  }
+
+  size_t new_index = index.has_value()
+                         ? static_cast<size_t>(index.value())
+                         : new_parent->children().size();
+
+  model->Move(node, new_parent, new_index);
+
+  *out_node = BuildBookmarkNode(node);
+  return Response::Success();
+}
+
+Response BookmarksHandler::RemoveBookmark(const std::string& id) {
+  BookmarkModel* model = GetBookmarkModel();
+  if (!model || !model->loaded()) {
+    return Response::ServerError("Bookmark model not loaded");
+  }
+
+  const BookmarkNode* node = FindNodeById(model, id);
+  if (!node) {
+    return Response::ServerError("No bookmark with given id");
+  }
+
+  model->Remove(node, bookmarks::metrics::BookmarkEditSource::kOther,
+                FROM_HERE);
+  return Response::Success();
+}
