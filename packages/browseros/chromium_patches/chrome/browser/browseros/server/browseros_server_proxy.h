diff --git a/chrome/browser/browseros/server/browseros_server_proxy.h b/chrome/browser/browseros/server/browseros_server_proxy.h
new file mode 100644
index 0000000000000..1c9b23683fdec
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_proxy.h
@@ -0,0 +1,76 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PROXY_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PROXY_H_
+
+#include <memory>
+#include <string>
+
+#include "base/memory/scoped_refptr.h"
+#include "net/server/http_server.h"
+
+namespace network {
+class PendingSharedURLLoaderFactory;
+class SharedURLLoaderFactory;
+class SimpleURLLoader;
+}  // namespace network
+
+namespace browseros {
+
+// HTTP proxy that binds a stable port and forwards all requests to the
+// sidecar's ephemeral backend port. Returns 503 when no backend is configured.
+//
+// Threading: The entire proxy runs on the IO thread. The manager obtains a
+// SharedURLLoaderFactory on the UI thread, calls Clone() to get a
+// PendingSharedURLLoaderFactory, and passes it to Start() on the IO thread.
+// Start() binds it into a new SharedURLLoaderFactory usable from IO.
+// This keeps net::HttpServer and SimpleURLLoader on the same thread.
+class BrowserOSServerProxy : public net::HttpServer::Delegate {
+ public:
+  BrowserOSServerProxy();
+  ~BrowserOSServerProxy() override;
+
+  BrowserOSServerProxy(const BrowserOSServerProxy&) = delete;
+  BrowserOSServerProxy& operator=(const BrowserOSServerProxy&) = delete;
+
+  // Bind proxy on the given port. |pending_factory| is a cloned factory
+  // that will be bound on the current (IO) thread. Returns true on success.
+  bool Start(int port,
+             std::unique_ptr<network::PendingSharedURLLoaderFactory>
+                 pending_factory);
+
+  void Stop();
+
+  void SetBackendPort(int port);
+  void SetAllowRemote(bool allow);
+
+  int GetPort() const { return bound_port_; }
+
+ private:
+  // net::HttpServer::Delegate
+  void OnConnect(int connection_id) override;
+  void OnHttpRequest(int connection_id,
+                     const net::HttpServerRequestInfo& info) override;
+  void OnWebSocketRequest(int connection_id,
+                          const net::HttpServerRequestInfo& info) override;
+  void OnWebSocketMessage(int connection_id, std::string data) override;
+  void OnClose(int connection_id) override;
+
+  void ForwardRequest(int connection_id,
+                      const net::HttpServerRequestInfo& info);
+  void OnBackendResponse(int connection_id,
+                         std::unique_ptr<network::SimpleURLLoader> loader,
+                         std::unique_ptr<std::string> response_body);
+
+  std::unique_ptr<net::HttpServer> server_;
+  scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory_;
+  int backend_port_ = 0;
+  int bound_port_ = 0;
+  bool allow_remote_ = false;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PROXY_H_
