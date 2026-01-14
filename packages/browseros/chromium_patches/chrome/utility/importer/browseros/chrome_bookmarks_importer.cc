diff --git a/chrome/utility/importer/browseros/chrome_bookmarks_importer.cc b/chrome/utility/importer/browseros/chrome_bookmarks_importer.cc
new file mode 100644
index 0000000000000..6896bb7ab0f7c
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_bookmarks_importer.cc
@@ -0,0 +1,249 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome bookmarks importer implementation
+
+#include "chrome/utility/importer/browseros/chrome_bookmarks_importer.h"
+
+#include <map>
+#include <set>
+
+#include "base/files/file_util.h"
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/utf_string_conversions.h"
+#include "chrome/utility/importer/browseros/chrome_importer_utils.h"
+#include "components/user_data_importer/content/favicon_reencode.h"
+#include "sql/database.h"
+#include "sql/statement.h"
+#include "url/gurl.h"
+
+namespace browseros_importer {
+
+ChromeBookmarksResult::ChromeBookmarksResult() = default;
+ChromeBookmarksResult::~ChromeBookmarksResult() = default;
+ChromeBookmarksResult::ChromeBookmarksResult(ChromeBookmarksResult&&) = default;
+ChromeBookmarksResult& ChromeBookmarksResult::operator=(ChromeBookmarksResult&&) =
+    default;
+
+namespace {
+
+inline constexpr sql::Database::Tag kDatabaseTag{"ChromeImporter"};
+constexpr char kBookmarksFilename[] = "Bookmarks";
+constexpr char kFaviconsFilename[] = "Favicons";
+
+// Map of favicon ID to URLs that use that favicon.
+using FaviconMap = std::map<int64_t, std::set<GURL>>;
+
+void RecursiveReadBookmarksFolder(
+    const base::Value::Dict* folder,
+    const std::vector<std::u16string>& parent_path,
+    bool is_in_toolbar,
+    std::vector<user_data_importer::ImportedBookmarkEntry>* bookmarks) {
+  if (!folder) {
+    return;
+  }
+
+  const base::Value::List* children = folder->FindList("children");
+  if (!children) {
+    return;
+  }
+
+  for (const auto& value : *children) {
+    if (!value.is_dict()) {
+      continue;
+    }
+
+    const base::Value::Dict& item = value.GetDict();
+    const std::string* type = item.FindString("type");
+    if (!type) {
+      continue;
+    }
+
+    const std::string* name = item.FindString("name");
+    std::u16string title = base::UTF8ToUTF16(name ? *name : std::string());
+
+    const std::string* date_added_str = item.FindString("date_added");
+    int64_t date_added = 0;
+    if (date_added_str) {
+      base::StringToInt64(*date_added_str, &date_added);
+    }
+
+    if (*type == "folder") {
+      std::vector<std::u16string> path = parent_path;
+      path.push_back(title);
+
+      // Add empty folders as bookmark entries
+      const base::Value::List* inner_children = item.FindList("children");
+      if (inner_children && inner_children->empty()) {
+        user_data_importer::ImportedBookmarkEntry entry;
+        entry.is_folder = true;
+        entry.in_toolbar = is_in_toolbar;
+        entry.url = GURL();
+        entry.path = parent_path;
+        entry.title = title;
+        entry.creation_time = ChromeTimeToBaseTime(date_added);
+        bookmarks->push_back(std::move(entry));
+      }
+
+      RecursiveReadBookmarksFolder(&item, path, is_in_toolbar, bookmarks);
+    } else if (*type == "url") {
+      const std::string* url_str = item.FindString("url");
+      if (!url_str) {
+        continue;
+      }
+
+      GURL url(*url_str);
+      if (!url.is_valid()) {
+        continue;
+      }
+
+      user_data_importer::ImportedBookmarkEntry entry;
+      entry.is_folder = false;
+      entry.in_toolbar = is_in_toolbar;
+      entry.url = url;
+      entry.path = parent_path;
+      entry.title = title;
+      entry.creation_time = ChromeTimeToBaseTime(date_added);
+      bookmarks->push_back(std::move(entry));
+    }
+  }
+}
+
+void LoadFaviconURLMappings(sql::Database* db, FaviconMap* favicon_map) {
+  const char kQuery[] = "SELECT icon_id, page_url FROM icon_mapping";
+  sql::Statement statement(db->GetUniqueStatement(kQuery));
+
+  while (statement.Step()) {
+    int64_t icon_id = statement.ColumnInt64(0);
+    GURL url(statement.ColumnString(1));
+    if (url.is_valid()) {
+      (*favicon_map)[icon_id].insert(url);
+    }
+  }
+}
+
+void LoadFaviconData(sql::Database* db,
+                     const FaviconMap& favicon_map,
+                     favicon_base::FaviconUsageDataList* favicons) {
+  const char kQuery[] =
+      "SELECT f.url, fb.image_data "
+      "FROM favicons f "
+      "JOIN favicon_bitmaps fb ON f.id = fb.icon_id "
+      "WHERE f.id = ?";
+
+  sql::Statement statement(db->GetUniqueStatement(kQuery));
+  if (!statement.is_valid()) {
+    return;
+  }
+
+  for (const auto& [icon_id, urls] : favicon_map) {
+    statement.BindInt64(0, icon_id);
+    if (statement.Step()) {
+      GURL favicon_url(statement.ColumnString(0));
+      if (!favicon_url.is_valid()) {
+        statement.Reset(true);
+        continue;
+      }
+
+      std::vector<uint8_t> data;
+      statement.ColumnBlobAsVector(1, &data);
+      if (data.empty()) {
+        statement.Reset(true);
+        continue;
+      }
+
+      auto decoded = importer::ReencodeFavicon(base::span(data));
+      if (!decoded) {
+        statement.Reset(true);
+        continue;
+      }
+
+      favicon_base::FaviconUsageData usage;
+      usage.favicon_url = favicon_url;
+      usage.urls = urls;
+      usage.png_data = std::move(*decoded);
+      favicons->push_back(std::move(usage));
+    }
+    statement.Reset(true);
+  }
+}
+
+}  // namespace
+
+ChromeBookmarksResult ImportChromeBookmarks(const base::FilePath& profile_path) {
+  ChromeBookmarksResult result;
+
+  // Read bookmarks JSON file
+  base::FilePath bookmarks_path = profile_path.AppendASCII(kBookmarksFilename);
+  if (!base::PathExists(bookmarks_path)) {
+    LOG(WARNING) << "browseros: Bookmarks file not found";
+    return result;
+  }
+
+  std::string bookmarks_content;
+  if (!base::ReadFileToString(bookmarks_path, &bookmarks_content)) {
+    LOG(WARNING) << "browseros: Failed to read Bookmarks file";
+    return result;
+  }
+
+  std::optional<base::Value> bookmarks_value =
+      base::JSONReader::Read(bookmarks_content);
+  if (!bookmarks_value || !bookmarks_value->is_dict()) {
+    LOG(WARNING) << "browseros: Failed to parse Bookmarks JSON";
+    return result;
+  }
+
+  const base::Value::Dict* roots = bookmarks_value->GetDict().FindDict("roots");
+  if (!roots) {
+    LOG(WARNING) << "browseros: No roots in Bookmarks";
+    return result;
+  }
+
+  // Import bookmark bar
+  const base::Value::Dict* bookmark_bar = roots->FindDict("bookmark_bar");
+  if (bookmark_bar) {
+    std::vector<std::u16string> path;
+    const std::string* name = bookmark_bar->FindString("name");
+    path.push_back(base::UTF8ToUTF16(name ? *name : "Bookmarks Bar"));
+    RecursiveReadBookmarksFolder(bookmark_bar, path, /*is_in_toolbar=*/true,
+                                 &result.bookmarks);
+  }
+
+  // Import other bookmarks
+  const base::Value::Dict* other = roots->FindDict("other");
+  if (other) {
+    std::vector<std::u16string> path;
+    const std::string* name = other->FindString("name");
+    path.push_back(base::UTF8ToUTF16(name ? *name : "Other Bookmarks"));
+    RecursiveReadBookmarksFolder(other, path, /*is_in_toolbar=*/false,
+                                 &result.bookmarks);
+  }
+
+  // Import favicons from Favicons database
+  // Original code uses DirName() - try that first, then profile directory
+  base::FilePath favicons_path =
+      profile_path.DirName().AppendASCII(kFaviconsFilename);
+  if (!base::PathExists(favicons_path)) {
+    favicons_path = profile_path.AppendASCII(kFaviconsFilename);
+  }
+
+  if (base::PathExists(favicons_path)) {
+    base::FilePath temp_favicons = CopyToTempFile(favicons_path);
+    if (!temp_favicons.empty()) {
+      sql::Database db(kDatabaseTag);
+      if (db.Open(temp_favicons)) {
+        FaviconMap favicon_map;
+        LoadFaviconURLMappings(&db, &favicon_map);
+        if (!favicon_map.empty()) {
+          LoadFaviconData(&db, favicon_map, &result.favicons);
+        }
+        db.Close();
+      }
+      base::DeleteFile(temp_favicons);
+    }
+  }
+
+  return result;
+}
+
+}  // namespace browseros_importer
