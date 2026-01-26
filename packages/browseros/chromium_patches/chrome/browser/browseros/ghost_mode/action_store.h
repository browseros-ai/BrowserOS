diff --git a/chrome/browser/browseros/ghost_mode/action_store.h b/chrome/browser/browseros/ghost_mode/action_store.h
new file mode 100644
index 0000000000000..9b0c1d2e3f4a5
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/action_store.h
@@ -0,0 +1,112 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_ACTION_STORE_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_ACTION_STORE_H_
+
+#include <memory>
+#include <string>
+#include <vector>
+
+#include "base/files/file_path.h"
+#include "base/memory/weak_ptr.h"
+#include "base/sequence_checker.h"
+#include "base/time/time.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+
+namespace sql {
+class Database;
+}
+
+class PrefService;
+
+namespace browseros::ghost_mode {
+
+// ActionStore persists recorded actions to a local SQLite database.
+// 
+// The database is stored in the user's profile directory and is
+// automatically cleaned up based on the retention period setting.
+//
+// Thread safety: All database operations happen on a background sequence.
+// Public methods can be called from any thread.
+class ActionStore {
+ public:
+  // Create an ActionStore that uses the given profile directory
+  explicit ActionStore(const base::FilePath& profile_path,
+                       PrefService* pref_service);
+  ~ActionStore();
+
+  // Initialize the database (must be called before other operations)
+  bool Initialize();
+
+  // Add a recorded action to the store
+  void AddAction(const RecordedAction& action);
+
+  // Get all actions within a time range
+  std::vector<RecordedAction> GetActionsInRange(base::Time start,
+                                                 base::Time end);
+
+  // Get all actions for a specific session
+  std::vector<RecordedAction> GetActionsForSession(const std::string& session_id);
+
+  // Get all actions matching a URL pattern
+  std::vector<RecordedAction> GetActionsForUrlPattern(
+      const std::string& url_pattern);
+
+  // Get action count for statistics
+  int GetTotalActionCount();
+
+  // Get unique session count
+  int GetSessionCount();
+
+  // Delete actions older than the retention period
+  void CleanupOldActions();
+
+  // Delete all stored actions (for privacy controls)
+  void DeleteAllActions();
+
+  // Delete actions for a specific URL pattern
+  void DeleteActionsForUrlPattern(const std::string& url_pattern);
+
+  // Get database file size in bytes
+  int64_t GetDatabaseSizeBytes();
+
+  // Pattern-related operations
+  
+  // Save a detected pattern
+  void SavePattern(const ActionSequence& pattern);
+  
+  // Get all saved patterns
+  std::vector<ActionSequence> GetAllPatterns();
+  
+  // Get pattern by ID
+  std::optional<ActionSequence> GetPattern(const std::string& pattern_id);
+  
+  // Update pattern (e.g., increment occurrence count)
+  void UpdatePattern(const ActionSequence& pattern);
+  
+  // Delete a pattern
+  void DeletePattern(const std::string& pattern_id);
+  
+  // Mark pattern as dismissed
+  void DismissPattern(const std::string& pattern_id);
+
+ private:
+  // Create database tables
+  bool CreateTables();
+  
+  // Database operations (run on background sequence)
+  void AddActionInternal(RecordedAction action);
+  void CleanupInternal(base::Time cutoff);
+  
+  // Database path
+  base::FilePath db_path_;
+  
+  // Pref service for retention settings
+  raw_ptr<PrefService> pref_service_;
+  
+  // SQLite database
+  std::unique_ptr<sql::Database> db_;
+  
+  // Weak pointer factory
+  base::WeakPtrFactory<ActionStore> weak_factory_{this};
+};
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_ACTION_STORE_H_
