diff --git a/chrome/browser/browseros/server/server_state_store.h b/chrome/browser/browseros/server/server_state_store.h
new file mode 100644
index 0000000000000..634ed7b5fb6eb
--- /dev/null
+++ b/chrome/browser/browseros/server/server_state_store.h
@@ -0,0 +1,36 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_SERVER_STATE_STORE_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_SERVER_STATE_STORE_H_
+
+#include <optional>
+
+#include "chrome/browser/browseros/server/browseros_server_utils.h"
+
+namespace browseros {
+
+// Interface for state file persistence used in orphan recovery.
+// The state file records the PID and creation time of the running server
+// so that if Chromium crashes, a new instance can detect and kill orphans.
+class ServerStateStore {
+ public:
+  virtual ~ServerStateStore() = default;
+
+  // Read state file.
+  // Returns nullopt if file doesn't exist or has invalid format.
+  virtual std::optional<server_utils::ServerState> Read() = 0;
+
+  // Write state file with pid and creation_time.
+  // Returns true on success.
+  virtual bool Write(const server_utils::ServerState& state) = 0;
+
+  // Delete state file.
+  // Returns true on success (or if file didn't exist).
+  virtual bool Delete() = 0;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_SERVER_STATE_STORE_H_
