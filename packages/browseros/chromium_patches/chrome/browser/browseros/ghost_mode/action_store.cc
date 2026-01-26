diff --git a/chrome/browser/browseros/ghost_mode/action_store.cc b/chrome/browser/browseros/ghost_mode/action_store.cc
new file mode 100644
index 0000000000000..a1b2c3d4e5f6a
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/action_store.cc
@@ -0,0 +1,428 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/action_store.h"
+
+#include "base/files/file_util.h"
+#include "base/json/json_reader.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/task/thread_pool.h"
+#include "base/uuid.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h"
+#include "components/prefs/pref_service.h"
+#include "sql/database.h"
+#include "sql/statement.h"
+#include "sql/transaction.h"
+
+namespace browseros::ghost_mode {
+
+namespace {
+
+inline constexpr sql::Database::Tag kDatabaseTag{"GhostMode"};
+constexpr char kDatabaseFilename[] = "ghost_mode.db";
+
+// Database schema version for migrations
+constexpr int kCurrentSchemaVersion = 1;
+
+// SQL for creating the actions table
+constexpr char kCreateActionsTableSql[] =
+    "CREATE TABLE IF NOT EXISTS actions ("
+    "  id TEXT PRIMARY KEY NOT NULL,"
+    "  type INTEGER NOT NULL,"
+    "  url TEXT NOT NULL,"
+    "  url_pattern TEXT NOT NULL,"
+    "  selectors TEXT NOT NULL,"
+    "  element_text TEXT,"
+    "  value TEXT,"
+    "  is_parameterizable INTEGER DEFAULT 0,"
+    "  timestamp INTEGER NOT NULL,"
+    "  tab_id INTEGER,"
+    "  session_id TEXT NOT NULL,"
+    "  time_since_previous INTEGER,"
+    "  metadata TEXT"
+    ")";
+
+// SQL for creating the patterns table
+constexpr char kCreatePatternsTableSql[] =
+    "CREATE TABLE IF NOT EXISTS patterns ("
+    "  id TEXT PRIMARY KEY NOT NULL,"
+    "  name TEXT NOT NULL,"
+    "  description TEXT,"
+    "  actions TEXT NOT NULL,"
+    "  occurrence_count INTEGER DEFAULT 0,"
+    "  first_seen INTEGER NOT NULL,"
+    "  last_seen INTEGER NOT NULL,"
+    "  confidence_score REAL DEFAULT 0.0,"
+    "  status INTEGER DEFAULT 0,"
+    "  url_pattern TEXT,"
+    "  metadata TEXT"
+    ")";
+
+// SQL for creating indices
+constexpr char kCreateIndicesSql[] =
+    "CREATE INDEX IF NOT EXISTS idx_actions_session ON actions(session_id);"
+    "CREATE INDEX IF NOT EXISTS idx_actions_timestamp ON actions(timestamp);"
+    "CREATE INDEX IF NOT EXISTS idx_actions_url_pattern ON actions(url_pattern);"
+    "CREATE INDEX IF NOT EXISTS idx_patterns_status ON patterns(status);";
+
+// SQL for schema version tracking
+constexpr char kCreateMetaTableSql[] =
+    "CREATE TABLE IF NOT EXISTS meta ("
+    "  key TEXT PRIMARY KEY NOT NULL,"
+    "  value TEXT"
+    ")";
+
+}  // namespace
+
+ActionStore::ActionStore(const base::FilePath& profile_path,
+                         PrefService* pref_service)
+    : db_path_(profile_path.AppendASCII(kDatabaseFilename)),
+      pref_service_(pref_service) {
+  CHECK(pref_service_);
+}
+
+ActionStore::~ActionStore() {
+  if (db_) {
+    db_->Close();
+  }
+}
+
+bool ActionStore::Initialize() {
+  VLOG(1) << "browseros: Initializing Ghost Mode action store at: " << db_path_;
+
+  db_ = std::make_unique<sql::Database>(kDatabaseTag);
+
+  if (!db_->Open(db_path_)) {
+    LOG(ERROR) << "browseros: Failed to open Ghost Mode database";
+    return false;
+  }
+
+  if (!CreateTables()) {
+    LOG(ERROR) << "browseros: Failed to create Ghost Mode tables";
+    return false;
+  }
+
+  VLOG(1) << "browseros: Ghost Mode action store initialized successfully";
+  return true;
+}
+
+bool ActionStore::CreateTables() {
+  sql::Transaction transaction(db_.get());
+  if (!transaction.Begin()) {
+    return false;
+  }
+
+  // Create meta table first
+  if (!db_->Execute(kCreateMetaTableSql)) {
+    LOG(ERROR) << "browseros: Failed to create meta table";
+    return false;
+  }
+
+  // Check schema version
+  sql::Statement version_stmt(
+      db_->GetUniqueStatement("SELECT value FROM meta WHERE key = 'version'"));
+  int stored_version = 0;
+  if (version_stmt.Step()) {
+    base::StringToInt(version_stmt.ColumnString(0), &stored_version);
+  }
+
+  if (stored_version < kCurrentSchemaVersion) {
+    // Create or upgrade tables
+    if (!db_->Execute(kCreateActionsTableSql)) {
+      LOG(ERROR) << "browseros: Failed to create actions table";
+      return false;
+    }
+
+    if (!db_->Execute(kCreatePatternsTableSql)) {
+      LOG(ERROR) << "browseros: Failed to create patterns table";
+      return false;
+    }
+
+    if (!db_->Execute(kCreateIndicesSql)) {
+      LOG(ERROR) << "browseros: Failed to create indices";
+      return false;
+    }
+
+    // Update version
+    sql::Statement update_version(db_->GetUniqueStatement(
+        "INSERT OR REPLACE INTO meta (key, value) VALUES ('version', ?)"));
+    update_version.BindString(0, base::NumberToString(kCurrentSchemaVersion));
+    if (!update_version.Run()) {
+      return false;
+    }
+  }
+
+  return transaction.Commit();
+}
+
+void ActionStore::AddAction(const RecordedAction& action) {
+  if (!db_) {
+    LOG(WARNING) << "browseros: Cannot add action - database not initialized";
+    return;
+  }
+
+  AddActionInternal(action);
+}
+
+void ActionStore::AddActionInternal(RecordedAction action) {
+  // Serialize selectors to JSON
+  base::Value::List selectors_list;
+  for (const auto& selector : action.selectors) {
+    selectors_list.Append(selector);
+  }
+  std::string selectors_json;
+  base::JSONWriter::Write(selectors_list, &selectors_json);
+
+  // Serialize metadata to JSON
+  std::string metadata_json;
+  base::JSONWriter::Write(action.metadata, &metadata_json);
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "INSERT INTO actions ("
+      "  id, type, url, url_pattern, selectors, element_text, value,"
+      "  is_parameterizable, timestamp, tab_id, session_id,"
+      "  time_since_previous, metadata"
+      ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"));
+
+  statement.BindString(0, action.id);
+  statement.BindInt(1, static_cast<int>(action.type));
+  statement.BindString(2, action.url.spec());
+  statement.BindString(3, action.url_pattern);
+  statement.BindString(4, selectors_json);
+  statement.BindString(5, action.element_text);
+  statement.BindString(6, action.value);
+  statement.BindBool(7, action.is_parameterizable);
+  statement.BindInt64(8, action.timestamp.InMillisecondsSinceUnixEpoch());
+  statement.BindInt(9, action.tab_id);
+  statement.BindString(10, action.session_id);
+  statement.BindInt64(11, action.time_since_previous.InMilliseconds());
+  statement.BindString(12, metadata_json);
+
+  if (!statement.Run()) {
+    LOG(ERROR) << "browseros: Failed to insert action into database";
+  }
+}
+
+std::vector<RecordedAction> ActionStore::GetActionsInRange(base::Time start,
+                                                            base::Time end) {
+  std::vector<RecordedAction> results;
+  if (!db_) {
+    return results;
+  }
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "SELECT id, type, url, url_pattern, selectors, element_text, value,"
+      "       is_parameterizable, timestamp, tab_id, session_id,"
+      "       time_since_previous, metadata "
+      "FROM actions "
+      "WHERE timestamp >= ? AND timestamp <= ? "
+      "ORDER BY timestamp ASC"));
+
+  statement.BindInt64(0, start.InMillisecondsSinceUnixEpoch());
+  statement.BindInt64(1, end.InMillisecondsSinceUnixEpoch());
+
+  while (statement.Step()) {
+    RecordedAction action;
+    action.id = statement.ColumnString(0);
+    action.type = static_cast<ActionType>(statement.ColumnInt(1));
+    action.url = GURL(statement.ColumnString(2));
+    action.url_pattern = statement.ColumnString(3);
+
+    // Parse selectors JSON
+    std::optional<base::Value> selectors_value =
+        base::JSONReader::Read(statement.ColumnString(4));
+    if (selectors_value && selectors_value->is_list()) {
+      for (const auto& item : selectors_value->GetList()) {
+        if (item.is_string()) {
+          action.selectors.push_back(item.GetString());
+        }
+      }
+    }
+
+    action.element_text = statement.ColumnString(5);
+    action.value = statement.ColumnString(6);
+    action.is_parameterizable = statement.ColumnBool(7);
+    action.timestamp = base::Time::FromMillisecondsSinceUnixEpoch(
+        statement.ColumnInt64(8));
+    action.tab_id = statement.ColumnInt(9);
+    action.session_id = statement.ColumnString(10);
+    action.time_since_previous =
+        base::Milliseconds(statement.ColumnInt64(11));
+
+    // Parse metadata JSON
+    std::optional<base::Value> metadata_value =
+        base::JSONReader::Read(statement.ColumnString(12));
+    if (metadata_value && metadata_value->is_dict()) {
+      action.metadata = std::move(metadata_value->GetDict());
+    }
+
+    results.push_back(std::move(action));
+  }
+
+  return results;
+}
+
+std::vector<RecordedAction> ActionStore::GetActionsForSession(
+    const std::string& session_id) {
+  std::vector<RecordedAction> results;
+  if (!db_) {
+    return results;
+  }
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "SELECT id, type, url, url_pattern, selectors, element_text, value,"
+      "       is_parameterizable, timestamp, tab_id, session_id,"
+      "       time_since_previous, metadata "
+      "FROM actions "
+      "WHERE session_id = ? "
+      "ORDER BY timestamp ASC"));
+
+  statement.BindString(0, session_id);
+
+  while (statement.Step()) {
+    RecordedAction action;
+    action.id = statement.ColumnString(0);
+    action.type = static_cast<ActionType>(statement.ColumnInt(1));
+    action.url = GURL(statement.ColumnString(2));
+    action.url_pattern = statement.ColumnString(3);
+
+    std::optional<base::Value> selectors_value =
+        base::JSONReader::Read(statement.ColumnString(4));
+    if (selectors_value && selectors_value->is_list()) {
+      for (const auto& item : selectors_value->GetList()) {
+        if (item.is_string()) {
+          action.selectors.push_back(item.GetString());
+        }
+      }
+    }
+
+    action.element_text = statement.ColumnString(5);
+    action.value = statement.ColumnString(6);
+    action.is_parameterizable = statement.ColumnBool(7);
+    action.timestamp = base::Time::FromMillisecondsSinceUnixEpoch(
+        statement.ColumnInt64(8));
+    action.tab_id = statement.ColumnInt(9);
+    action.session_id = statement.ColumnString(10);
+    action.time_since_previous =
+        base::Milliseconds(statement.ColumnInt64(11));
+
+    std::optional<base::Value> metadata_value =
+        base::JSONReader::Read(statement.ColumnString(12));
+    if (metadata_value && metadata_value->is_dict()) {
+      action.metadata = std::move(metadata_value->GetDict());
+    }
+
+    results.push_back(std::move(action));
+  }
+
+  return results;
+}
+
+std::vector<RecordedAction> ActionStore::GetActionsForUrlPattern(
+    const std::string& url_pattern) {
+  std::vector<RecordedAction> results;
+  if (!db_) {
+    return results;
+  }
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "SELECT id, type, url, url_pattern, selectors, element_text, value,"
+      "       is_parameterizable, timestamp, tab_id, session_id,"
+      "       time_since_previous, metadata "
+      "FROM actions "
+      "WHERE url_pattern LIKE ? "
+      "ORDER BY timestamp ASC"));
+
+  statement.BindString(0, "%" + url_pattern + "%");
+
+  while (statement.Step()) {
+    RecordedAction action;
+    action.id = statement.ColumnString(0);
+    action.type = static_cast<ActionType>(statement.ColumnInt(1));
+    action.url = GURL(statement.ColumnString(2));
+    action.url_pattern = statement.ColumnString(3);
+
+    std::optional<base::Value> selectors_value =
+        base::JSONReader::Read(statement.ColumnString(4));
+    if (selectors_value && selectors_value->is_list()) {
+      for (const auto& item : selectors_value->GetList()) {
+        if (item.is_string()) {
+          action.selectors.push_back(item.GetString());
+        }
+      }
+    }
+
+    action.element_text = statement.ColumnString(5);
+    action.value = statement.ColumnString(6);
+    action.is_parameterizable = statement.ColumnBool(7);
+    action.timestamp = base::Time::FromMillisecondsSinceUnixEpoch(
+        statement.ColumnInt64(8));
+    action.tab_id = statement.ColumnInt(9);
+    action.session_id = statement.ColumnString(10);
+    action.time_since_previous =
+        base::Milliseconds(statement.ColumnInt64(11));
+
+    std::optional<base::Value> metadata_value =
+        base::JSONReader::Read(statement.ColumnString(12));
+    if (metadata_value && metadata_value->is_dict()) {
+      action.metadata = std::move(metadata_value->GetDict());
+    }
+
+    results.push_back(std::move(action));
+  }
+
+  return results;
+}
+
+int ActionStore::GetTotalActionCount() {
+  if (!db_) {
+    return 0;
+  }
+
+  sql::Statement statement(
+      db_->GetUniqueStatement("SELECT COUNT(*) FROM actions"));
+  if (statement.Step()) {
+    return statement.ColumnInt(0);
+  }
+  return 0;
+}
+
+int ActionStore::GetSessionCount() {
+  if (!db_) {
+    return 0;
+  }
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "SELECT COUNT(DISTINCT session_id) FROM actions"));
+  if (statement.Step()) {
+    return statement.ColumnInt(0);
+  }
+  return 0;
+}
+
+void ActionStore::CleanupOldActions() {
+  if (!db_ || !pref_service_) {
+    return;
+  }
+
+  int retention_days =
+      pref_service_->GetInteger(prefs::kGhostModeRetentionDays);
+  base::Time cutoff = base::Time::Now() - base::Days(retention_days);
+
+  CleanupInternal(cutoff);
+}
+
+void ActionStore::CleanupInternal(base::Time cutoff) {
+  sql::Statement statement(db_->GetUniqueStatement(
+      "DELETE FROM actions WHERE timestamp < ?"));
+  statement.BindInt64(0, cutoff.InMillisecondsSinceUnixEpoch());
+
+  if (!statement.Run()) {
+    LOG(ERROR) << "browseros: Failed to cleanup old actions";
+  } else {
+    VLOG(1) << "browseros: Cleaned up actions older than " << cutoff;
+  }
+}
+
+void ActionStore::DeleteAllActions() {
+  if (!db_) {
+    return;
+  }
+
+  if (!db_->Execute("DELETE FROM actions")) {
+    LOG(ERROR) << "browseros: Failed to delete all actions";
+  } else {
+    LOG(INFO) << "browseros: Deleted all Ghost Mode actions";
+  }
+}
+
+void ActionStore::DeleteActionsForUrlPattern(const std::string& url_pattern) {
+  if (!db_) {
+    return;
+  }
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "DELETE FROM actions WHERE url_pattern LIKE ?"));
+  statement.BindString(0, "%" + url_pattern + "%");
+
+  if (!statement.Run()) {
+    LOG(ERROR) << "browseros: Failed to delete actions for pattern: "
+               << url_pattern;
+  }
+}
+
+int64_t ActionStore::GetDatabaseSizeBytes() {
+  if (!base::PathExists(db_path_)) {
+    return 0;
+  }
+
+  int64_t size = 0;
+  base::GetFileSize(db_path_, &size);
+  return size;
+}
+
+void ActionStore::SavePattern(const ActionSequence& pattern) {
+  if (!db_) {
+    return;
+  }
+
+  // Serialize actions to JSON
+  base::Value::List actions_list;
+  for (const auto& action : pattern.actions) {
+    actions_list.Append(action.ToValue());
+  }
+  std::string actions_json;
+  base::JSONWriter::Write(actions_list, &actions_json);
+
+  // Serialize metadata
+  std::string metadata_json;
+  base::JSONWriter::Write(pattern.metadata, &metadata_json);
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "INSERT OR REPLACE INTO patterns ("
+      "  id, name, description, actions, occurrence_count, first_seen,"
+      "  last_seen, confidence_score, status, url_pattern, metadata"
+      ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"));
+
+  statement.BindString(0, pattern.id);
+  statement.BindString(1, pattern.name);
+  statement.BindString(2, pattern.description);
+  statement.BindString(3, actions_json);
+  statement.BindInt(4, pattern.occurrence_count);
+  statement.BindInt64(5, pattern.first_seen.InMillisecondsSinceUnixEpoch());
+  statement.BindInt64(6, pattern.last_seen.InMillisecondsSinceUnixEpoch());
+  statement.BindDouble(7, pattern.confidence_score);
+  statement.BindInt(8, static_cast<int>(pattern.status));
+  statement.BindString(9, pattern.url_pattern);
+  statement.BindString(10, metadata_json);
+
+  if (!statement.Run()) {
+    LOG(ERROR) << "browseros: Failed to save pattern: " << pattern.id;
+  }
+}
+
+std::vector<ActionSequence> ActionStore::GetAllPatterns() {
+  std::vector<ActionSequence> results;
+  if (!db_) {
+    return results;
+  }
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "SELECT id, name, description, actions, occurrence_count, first_seen,"
+      "       last_seen, confidence_score, status, url_pattern, metadata "
+      "FROM patterns "
+      "ORDER BY last_seen DESC"));
+
+  while (statement.Step()) {
+    ActionSequence pattern;
+    pattern.id = statement.ColumnString(0);
+    pattern.name = statement.ColumnString(1);
+    pattern.description = statement.ColumnString(2);
+
+    // Parse actions JSON
+    std::optional<base::Value> actions_value =
+        base::JSONReader::Read(statement.ColumnString(3));
+    if (actions_value && actions_value->is_list()) {
+      for (const auto& item : actions_value->GetList()) {
+        if (item.is_dict()) {
+          auto action = RecordedAction::FromValue(item.GetDict());
+          if (action.has_value()) {
+            pattern.actions.push_back(std::move(*action));
+          }
+        }
+      }
+    }
+
+    pattern.occurrence_count = statement.ColumnInt(4);
+    pattern.first_seen = base::Time::FromMillisecondsSinceUnixEpoch(
+        statement.ColumnInt64(5));
+    pattern.last_seen = base::Time::FromMillisecondsSinceUnixEpoch(
+        statement.ColumnInt64(6));
+    pattern.confidence_score = statement.ColumnDouble(7);
+    pattern.status = static_cast<PatternStatus>(statement.ColumnInt(8));
+    pattern.url_pattern = statement.ColumnString(9);
+
+    std::optional<base::Value> metadata_value =
+        base::JSONReader::Read(statement.ColumnString(10));
+    if (metadata_value && metadata_value->is_dict()) {
+      pattern.metadata = std::move(metadata_value->GetDict());
+    }
+
+    results.push_back(std::move(pattern));
+  }
+
+  return results;
+}
+
+std::optional<ActionSequence> ActionStore::GetPattern(
+    const std::string& pattern_id) {
+  if (!db_) {
+    return std::nullopt;
+  }
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "SELECT id, name, description, actions, occurrence_count, first_seen,"
+      "       last_seen, confidence_score, status, url_pattern, metadata "
+      "FROM patterns "
+      "WHERE id = ?"));
+  statement.BindString(0, pattern_id);
+
+  if (!statement.Step()) {
+    return std::nullopt;
+  }
+
+  ActionSequence pattern;
+  pattern.id = statement.ColumnString(0);
+  pattern.name = statement.ColumnString(1);
+  pattern.description = statement.ColumnString(2);
+
+  std::optional<base::Value> actions_value =
+      base::JSONReader::Read(statement.ColumnString(3));
+  if (actions_value && actions_value->is_list()) {
+    for (const auto& item : actions_value->GetList()) {
+      if (item.is_dict()) {
+        auto action = RecordedAction::FromValue(item.GetDict());
+        if (action.has_value()) {
+          pattern.actions.push_back(std::move(*action));
+        }
+      }
+    }
+  }
+
+  pattern.occurrence_count = statement.ColumnInt(4);
+  pattern.first_seen = base::Time::FromMillisecondsSinceUnixEpoch(
+      statement.ColumnInt64(5));
+  pattern.last_seen = base::Time::FromMillisecondsSinceUnixEpoch(
+      statement.ColumnInt64(6));
+  pattern.confidence_score = statement.ColumnDouble(7);
+  pattern.status = static_cast<PatternStatus>(statement.ColumnInt(8));
+  pattern.url_pattern = statement.ColumnString(9);
+
+  std::optional<base::Value> metadata_value =
+      base::JSONReader::Read(statement.ColumnString(10));
+  if (metadata_value && metadata_value->is_dict()) {
+    pattern.metadata = std::move(metadata_value->GetDict());
+  }
+
+  return pattern;
+}
+
+void ActionStore::UpdatePattern(const ActionSequence& pattern) {
+  // SavePattern uses INSERT OR REPLACE, so it handles updates
+  SavePattern(pattern);
+}
+
+void ActionStore::DeletePattern(const std::string& pattern_id) {
+  if (!db_) {
+    return;
+  }
+
+  sql::Statement statement(
+      db_->GetUniqueStatement("DELETE FROM patterns WHERE id = ?"));
+  statement.BindString(0, pattern_id);
+
+  if (!statement.Run()) {
+    LOG(ERROR) << "browseros: Failed to delete pattern: " << pattern_id;
+  }
+}
+
+void ActionStore::DismissPattern(const std::string& pattern_id) {
+  if (!db_) {
+    return;
+  }
+
+  sql::Statement statement(db_->GetUniqueStatement(
+      "UPDATE patterns SET status = ? WHERE id = ?"));
+  statement.BindInt(0, static_cast<int>(PatternStatus::kDismissed));
+  statement.BindString(1, pattern_id);
+
+  if (!statement.Run()) {
+    LOG(ERROR) << "browseros: Failed to dismiss pattern: " << pattern_id;
+  }
+}
+
+}  // namespace browseros::ghost_mode
