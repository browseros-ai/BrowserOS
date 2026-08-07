diff --git a/chrome/browser/browseros/server/browseros_server_proxy.h b/chrome/browser/browseros/server/browseros_server_proxy.h
new file mode 100644
index 0000000000000..0e62a9f7995e1
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_proxy.h
@@ -0,0 +1,93 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PROXY_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PROXY_H_
+
+#include <cstddef>
+#include <memory>
+
+#include "base/containers/flat_map.h"
+#include "base/memory/scoped_refptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/threading/thread_checker.h"
+
+namespace net {
+class SSLServerContext;
+class SSLServerSocket;
+class StreamSocket;
+class TCPServerSocket;
+class X509Certificate;
+}  // namespace net
+
+namespace crypto::keypair {
+class PrivateKey;
+}  // namespace crypto::keypair
+
+namespace browseros {
+
+// HTTP and HTTPS proxy that fronts the BrowserOS sidecar without parsing or
+// rewriting traffic.
+//
+// HTTPS uses a per-start in-memory self-signed certificate. HTTPS setup
+// failures log and leave the proxy running HTTP-only.
+//
+// Threading: The entire proxy runs on the IO thread. The manager creates the
+// proxy on the UI thread, then starts, updates, stops, and destroys it on the
+// IO thread.
+class BrowserOSServerProxy {
+ public:
+  BrowserOSServerProxy();
+  ~BrowserOSServerProxy();
+
+  BrowserOSServerProxy(const BrowserOSServerProxy&) = delete;
+  BrowserOSServerProxy& operator=(const BrowserOSServerProxy&) = delete;
+
+  // Binds the stable HTTP proxy port and starts accepting TCP connections.
+  bool Start(int http_port, int https_port);
+  void Stop();
+
+  void SetBackendPort(int port);
+  void SetAllowRemote(bool allow);
+
+  int GetPort() const { return bound_http_port_; }
+  int GetHttpsPort() const { return bound_https_port_; }
+  size_t GetConnectionCountForTesting() const { return connections_.size(); }
+
+ private:
+  class Connection;
+  struct ListenerState;
+
+  bool StartHttpListener(int http_port);
+  bool StartHttpsListener(int https_port);
+  bool GenerateTlsCredentials();
+  void StartAccept(ListenerState* listener);
+  void OnAccept(ListenerState* listener, int result);
+  void StartConnection(std::unique_ptr<net::StreamSocket> client_socket);
+  void StartTlsHandshake(std::unique_ptr<net::StreamSocket> client_socket);
+  void OnTlsHandshakeDone(int handshake_id, int result);
+  void OnConnectionFinished(int connection_id);
+
+  std::unique_ptr<ListenerState> http_listener_;
+  std::unique_ptr<ListenerState> https_listener_;
+  base::flat_map<int, std::unique_ptr<Connection>> connections_;
+  base::flat_map<int, std::unique_ptr<net::SSLServerSocket>> tls_handshakes_;
+  int next_connection_id_ = 1;
+  int next_tls_handshake_id_ = 1;
+  int backend_port_ = 0;
+  int bound_http_port_ = 0;
+  int bound_https_port_ = 0;
+  bool allow_remote_ = false;
+
+  std::unique_ptr<crypto::keypair::PrivateKey> tls_key_;
+  scoped_refptr<net::X509Certificate> tls_cert_;
+  std::unique_ptr<net::SSLServerContext> tls_context_;
+
+  THREAD_CHECKER(thread_checker_);
+  base::WeakPtrFactory<BrowserOSServerProxy> weak_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_PROXY_H_
