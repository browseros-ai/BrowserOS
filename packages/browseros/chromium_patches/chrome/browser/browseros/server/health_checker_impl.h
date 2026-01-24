diff --git a/chrome/browser/browseros/server/health_checker_impl.h b/chrome/browser/browseros/server/health_checker_impl.h
new file mode 100644
index 0000000000000..8637887651184
--- /dev/null
+++ b/chrome/browser/browseros/server/health_checker_impl.h
@@ -0,0 +1,49 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_IMPL_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_IMPL_H_
+
+#include <memory>
+
+#include "base/memory/scoped_refptr.h"
+#include "chrome/browser/browseros/server/health_checker.h"
+
+namespace net {
+class HttpResponseHeaders;
+}
+
+namespace network {
+class SimpleURLLoader;
+}
+
+namespace browseros {
+
+// Production implementation of HealthChecker.
+// Uses network::SimpleURLLoader to make HTTP requests.
+class HealthCheckerImpl : public HealthChecker {
+ public:
+  HealthCheckerImpl();
+  ~HealthCheckerImpl() override;
+
+  HealthCheckerImpl(const HealthCheckerImpl&) = delete;
+  HealthCheckerImpl& operator=(const HealthCheckerImpl&) = delete;
+
+  // HealthChecker implementation:
+  void CheckHealth(int port,
+                   base::OnceCallback<void(bool success)> callback) override;
+
+ private:
+  void OnHealthCheckComplete(
+      std::unique_ptr<network::SimpleURLLoader> url_loader,
+      base::OnceCallback<void(bool success)> callback,
+      scoped_refptr<net::HttpResponseHeaders> headers);
+
+  // The active URL loader (kept alive during the request)
+  std::unique_ptr<network::SimpleURLLoader> url_loader_;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_IMPL_H_
