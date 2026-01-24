diff --git a/chrome/browser/browseros/server/server_state_store_impl.cc b/chrome/browser/browseros/server/server_state_store_impl.cc
new file mode 100644
index 0000000000000..76805b00dee0d
--- /dev/null
+++ b/chrome/browser/browseros/server/server_state_store_impl.cc
@@ -0,0 +1,25 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/server_state_store_impl.h"
+
+namespace browseros {
+
+ServerStateStoreImpl::ServerStateStoreImpl() = default;
+
+ServerStateStoreImpl::~ServerStateStoreImpl() = default;
+
+std::optional<server_utils::ServerState> ServerStateStoreImpl::Read() {
+  return server_utils::ReadStateFile();
+}
+
+bool ServerStateStoreImpl::Write(const server_utils::ServerState& state) {
+  return server_utils::WriteStateFile(state);
+}
+
+bool ServerStateStoreImpl::Delete() {
+  return server_utils::DeleteStateFile();
+}
+
+}  // namespace browseros
