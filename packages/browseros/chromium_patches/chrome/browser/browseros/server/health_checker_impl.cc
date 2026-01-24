diff --git a/chrome/browser/browseros/server/health_checker_impl.cc b/chrome/browser/browseros/server/health_checker_impl.cc
new file mode 100644
index 0000000000000..97fa0ec9cefe2
--- /dev/null
+++ b/chrome/browser/browseros/server/health_checker_impl.cc
@@ -0,0 +1,101 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/health_checker_impl.h"
+
+#include "base/functional/bind.h"
+#include "base/logging.h"
+#include "base/strings/string_number_conversions.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/net/system_network_context_manager.h"
+#include "net/base/net_errors.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+#include "url/gurl.h"
+
+namespace browseros {
+
+namespace {
+
+constexpr base::TimeDelta kHealthCheckTimeout = base::Seconds(15);
+
+}  // namespace
+
+HealthCheckerImpl::HealthCheckerImpl() = default;
+
+HealthCheckerImpl::~HealthCheckerImpl() = default;
+
+void HealthCheckerImpl::CheckHealth(
+    int port,
+    base::OnceCallback<void(bool success)> callback) {
+  // Build health check URL
+  GURL health_url("http://127.0.0.1:" + base::NumberToString(port) + "/health");
+
+  // Create network traffic annotation
+  net::NetworkTrafficAnnotationTag traffic_annotation =
+      net::DefineNetworkTrafficAnnotation("browseros_health_check", R"(
+        semantics {
+          sender: "BrowserOS Server Manager"
+          description:
+            "Checks if the BrowserOS MCP server is healthy by querying its "
+            "/health endpoint."
+          trigger: "Periodic health check every 30 seconds while server is running."
+          data: "No user data sent, just an HTTP GET request."
+          destination: LOCAL
+        }
+        policy {
+          cookies_allowed: NO
+          setting: "This feature cannot be disabled by settings."
+          policy_exception_justification:
+            "Internal health check for BrowserOS server functionality."
+        })");
+
+  // Create resource request
+  auto resource_request = std::make_unique<network::ResourceRequest>();
+  resource_request->url = health_url;
+  resource_request->method = "GET";
+  resource_request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+
+  auto url_loader = network::SimpleURLLoader::Create(
+      std::move(resource_request), traffic_annotation);
+  url_loader->SetTimeoutDuration(kHealthCheckTimeout);
+
+  // Get URL loader factory from system network context
+  auto* url_loader_factory =
+      g_browser_process->system_network_context_manager()
+          ->GetURLLoaderFactory();
+
+  // Keep a raw pointer for the callback
+  auto* url_loader_ptr = url_loader.get();
+
+  // Download response headers only (we just need status code)
+  url_loader_ptr->DownloadHeadersOnly(
+      url_loader_factory,
+      base::BindOnce(&HealthCheckerImpl::OnHealthCheckComplete,
+                     base::Unretained(this), std::move(url_loader),
+                     std::move(callback)));
+}
+
+void HealthCheckerImpl::OnHealthCheckComplete(
+    std::unique_ptr<network::SimpleURLLoader> url_loader,
+    base::OnceCallback<void(bool success)> callback,
+    scoped_refptr<net::HttpResponseHeaders> headers) {
+  int response_code = 0;
+  if (headers) {
+    response_code = headers->response_code();
+  }
+
+  bool success = (response_code == 200);
+
+  if (!success) {
+    int net_error = url_loader->NetError();
+    LOG(WARNING) << "browseros: Health check failed - HTTP " << response_code
+                 << ", net error: " << net::ErrorToString(net_error);
+  }
+
+  std::move(callback).Run(success);
+}
+
+}  // namespace browseros
