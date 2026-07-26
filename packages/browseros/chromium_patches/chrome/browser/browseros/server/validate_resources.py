diff --git a/chrome/browser/browseros/server/validate_resources.py b/chrome/browser/browseros/server/validate_resources.py
new file mode 100644
index 0000000000000..0665cc5ce0337
--- /dev/null
+++ b/chrome/browser/browseros/server/validate_resources.py
@@ -0,0 +1,35 @@
+#!/usr/bin/env python3
+# Copyright 2024 The Chromium Authors
+# Use of this source code is governed by a BSD-style license that can be
+# found in the LICENSE file.
+
+"""Validates that required BrowserOS server resources exist."""
+
+import os
+import sys
+
+script_dir = os.path.dirname(os.path.abspath(__file__))
+resources_dir = os.path.join(script_dir, "resources")
+required_resources = sys.argv[1:] or ["bin/browseros_server"]
+
+all_valid = True
+for resource in required_resources:
+  resource_path = os.path.join(resources_dir, resource)
+
+  if not os.path.exists(resource_path):
+    print(f"ERROR: Required BrowserOS resource not found: {resource_path}")
+    all_valid = False
+    continue
+
+  if not os.path.isfile(resource_path):
+    print(f"ERROR: Resource exists but is not a file: {resource_path}")
+    all_valid = False
+
+if not all_valid:
+  print(f"\nEnsure all required resources exist in resources/ directory:")
+  for resource in required_resources:
+    print(f"  - resources/{resource}")
+  sys.exit(1)
+
+print(f"BrowserOS resources validated ({len(required_resources)} resources)")
+sys.exit(0)
