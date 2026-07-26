diff --git a/chrome/browser/browseros/server/browseros_server_proxy.cc b/chrome/browser/browseros/server/browseros_server_proxy.cc
new file mode 100644
index 0000000000000..557eedb4779c4
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_proxy.cc
@@ -0,0 +1,649 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_proxy.h"
+
+#include <limits>
+#include <memory>
+#include <string>
+#include <utility>
+
+#include "base/check.h"
+#include "base/containers/span.h"
+#include "base/functional/bind.h"
+#include "base/functional/callback.h"
+#include "base/logging.h"
+#include "base/memory/ref_counted.h"
+#include "base/rand_util.h"
+#include "base/task/single_thread_task_runner.h"
+#include "base/time/time.h"
+#include "crypto/keypair.h"
+#include "net/base/address_list.h"
+#include "net/base/io_buffer.h"
+#include "net/base/ip_address.h"
+#include "net/base/ip_endpoint.h"
+#include "net/base/net_errors.h"
+#include "net/cert/x509_certificate.h"
+#include "net/cert/x509_util.h"
+#include "net/log/net_log_source.h"
+#include "net/socket/ssl_server_socket.h"
+#include "net/socket/stream_socket.h"
+#include "net/socket/tcp_client_socket.h"
+#include "net/socket/tcp_server_socket.h"
+#include "net/ssl/ssl_server_config.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+
+namespace browseros {
+
+namespace {
+
+constexpr int kBackLog = 10;
+constexpr int kBufferSize = 16 * 1024;
+constexpr char kServiceUnavailableResponse[] =
+    "HTTP/1.1 503 Service Unavailable\r\n"
+    "Content-Type: text/plain\r\n"
+    "Content-Length: 19\r\n"
+    "Connection: close\r\n"
+    "\r\n"
+    "Service Unavailable";
+
+net::NetworkTrafficAnnotationTag kBrowserOSProxyPumpTrafficAnnotation =
+    net::DefineNetworkTrafficAnnotation("browseros_proxy_pump", R"(
+      semantics {
+        sender: "BrowserOS Server Proxy"
+        description:
+          "Copies raw HTTP bytes between clients connected to the BrowserOS "
+          "proxy port and the BrowserOS sidecar server on loopback."
+        trigger: "A client connects to the BrowserOS proxy port."
+        data: "HTTP requests and responses for BrowserOS server functionality."
+        destination: LOCAL
+      }
+      policy {
+        cookies_allowed: NO
+        setting: "This feature cannot be disabled by settings."
+        policy_exception_justification:
+          "Local proxy for BrowserOS server functionality."
+      })");
+
+}  // namespace
+
+struct BrowserOSServerProxy::ListenerState {
+  explicit ListenerState(bool is_https) : is_https(is_https) {}
+
+  std::unique_ptr<net::TCPServerSocket> socket;
+  std::unique_ptr<net::StreamSocket> pending_accept_socket;
+  net::IPEndPoint pending_peer_address;
+  bool is_https;
+};
+
+class BrowserOSServerProxy::Connection {
+ public:
+  Connection(int id,
+             std::unique_ptr<net::StreamSocket> client_socket,
+             int backend_port,
+             base::RepeatingCallback<void(int)> finish_callback)
+      : id_(id),
+        client_socket_(std::move(client_socket)),
+        backend_port_(backend_port),
+        finish_callback_(std::move(finish_callback)) {}
+
+  Connection(const Connection&) = delete;
+  Connection& operator=(const Connection&) = delete;
+
+  ~Connection() { DCHECK_CALLED_ON_VALID_THREAD(thread_checker_); }
+
+  void Start() {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    if (backend_port_ <= 0) {
+      WriteServiceUnavailableAndClose();
+      return;
+    }
+
+    backend_socket_ = std::make_unique<net::TCPClientSocket>(
+        net::AddressList::CreateFromIPAddress(net::IPAddress::IPv4Localhost(),
+                                              backend_port_),
+        nullptr, nullptr, nullptr, net::NetLogSource());
+    int result = backend_socket_->Connect(
+        base::BindOnce(&Connection::OnConnected, base::Unretained(this)));
+    if (result != net::ERR_IO_PENDING) {
+      OnConnected(result);
+      return;
+    }
+  }
+
+ private:
+  void OnConnected(int result) {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    if (result != net::OK) {
+      WriteServiceUnavailableAndClose();
+      return;
+    }
+
+    ++pending_writes_;
+    Pump(client_socket_.get(), backend_socket_.get());
+    --pending_writes_;
+    if (pending_destruction_) {
+      RequestFinish();
+      return;
+    }
+
+    Pump(backend_socket_.get(), client_socket_.get());
+  }
+
+  void Pump(net::StreamSocket* from, net::StreamSocket* to) {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    auto buffer = base::MakeRefCounted<net::IOBufferWithSize>(kBufferSize);
+    int result =
+        from->Read(buffer.get(), kBufferSize,
+                   base::BindOnce(&Connection::OnRead, base::Unretained(this),
+                                  from, to, buffer));
+    if (result != net::ERR_IO_PENDING) {
+      OnRead(from, to, std::move(buffer), result);
+      return;
+    }
+  }
+
+  void OnRead(net::StreamSocket* from,
+              net::StreamSocket* to,
+              scoped_refptr<net::IOBuffer> buffer,
+              int result) {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    if (result <= 0) {
+      RequestFinish();
+      return;
+    }
+
+    auto drainable =
+        base::MakeRefCounted<net::DrainableIOBuffer>(std::move(buffer), result);
+
+    ++pending_writes_;
+    result =
+        to->Write(drainable.get(), result,
+                  base::BindOnce(&Connection::OnWritten, base::Unretained(this),
+                                 drainable, from, to),
+                  kBrowserOSProxyPumpTrafficAnnotation);
+    if (result != net::ERR_IO_PENDING) {
+      OnWritten(drainable, from, to, result);
+      return;
+    }
+  }
+
+  void OnWritten(scoped_refptr<net::DrainableIOBuffer> drainable,
+                 net::StreamSocket* from,
+                 net::StreamSocket* to,
+                 int result) {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    --pending_writes_;
+    if (result <= 0) {
+      RequestFinish();
+      return;
+    }
+
+    drainable->DidConsume(result);
+    if (drainable->BytesRemaining() > 0) {
+      ++pending_writes_;
+      result =
+          to->Write(drainable.get(), drainable->BytesRemaining(),
+                    base::BindOnce(&Connection::OnWritten,
+                                   base::Unretained(this), drainable, from, to),
+                    kBrowserOSProxyPumpTrafficAnnotation);
+      if (result != net::ERR_IO_PENDING) {
+        OnWritten(drainable, from, to, result);
+        return;
+      }
+      return;
+    }
+
+    if (pending_destruction_) {
+      RequestFinish();
+      return;
+    }
+
+    Pump(from, to);
+  }
+
+  void WriteServiceUnavailableAndClose() {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    service_unavailable_mode_ = true;
+    service_unavailable_write_done_ = false;
+    service_unavailable_drain_done_ = false;
+
+    auto buffer = base::MakeRefCounted<net::DrainableIOBuffer>(
+        base::MakeRefCounted<net::StringIOBuffer>(
+            std::string(kServiceUnavailableResponse)),
+        sizeof(kServiceUnavailableResponse) - 1);
+    DrainClientForClose();
+    WriteCloseBuffer(buffer);
+  }
+
+  void WriteCloseBuffer(scoped_refptr<net::DrainableIOBuffer> buffer) {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    ++pending_writes_;
+    int result =
+        client_socket_->Write(buffer.get(), buffer->BytesRemaining(),
+                              base::BindOnce(&Connection::OnCloseBufferWritten,
+                                             base::Unretained(this), buffer),
+                              kBrowserOSProxyPumpTrafficAnnotation);
+    if (result != net::ERR_IO_PENDING) {
+      OnCloseBufferWritten(buffer, result);
+      return;
+    }
+  }
+
+  void OnCloseBufferWritten(scoped_refptr<net::DrainableIOBuffer> buffer,
+                            int result) {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    --pending_writes_;
+    if (result <= 0) {
+      RequestFinish();
+      return;
+    }
+
+    buffer->DidConsume(result);
+    if (buffer->BytesRemaining() > 0) {
+      WriteCloseBuffer(buffer);
+      return;
+    }
+
+    if (!service_unavailable_mode_) {
+      RequestFinish();
+      return;
+    }
+
+    service_unavailable_write_done_ = true;
+    MaybeFinishServiceUnavailable();
+  }
+
+  void DrainClientForClose() {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    auto buffer = base::MakeRefCounted<net::IOBufferWithSize>(kBufferSize);
+    int result =
+        client_socket_->Read(buffer.get(), buffer->size(),
+                             base::BindOnce(&Connection::OnClientDrainRead,
+                                            base::Unretained(this), buffer));
+    if (result != net::ERR_IO_PENDING) {
+      OnClientDrainRead(buffer, result);
+      return;
+    }
+  }
+
+  void OnClientDrainRead(scoped_refptr<net::IOBuffer> buffer, int result) {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    if (result > 0) {
+      DrainClientForClose();
+      return;
+    }
+
+    service_unavailable_drain_done_ = true;
+    MaybeFinishServiceUnavailable();
+  }
+
+  void MaybeFinishServiceUnavailable() {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    if (service_unavailable_write_done_ && service_unavailable_drain_done_) {
+      RequestFinish();
+    }
+  }
+
+  // Running the callback erases this Connection from the proxy; callers must
+  // return immediately after requesting finish.
+  void RequestFinish() {
+    DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+    if (pending_writes_ > 0) {
+      pending_destruction_ = true;
+      return;
+    }
+
+    finish_callback_.Run(id_);
+  }
+
+  const int id_;
+  std::unique_ptr<net::StreamSocket> client_socket_;
+  std::unique_ptr<net::TCPClientSocket> backend_socket_;
+  const int backend_port_;
+  base::RepeatingCallback<void(int)> finish_callback_;
+  int pending_writes_ = 0;
+  bool pending_destruction_ = false;
+  bool service_unavailable_mode_ = false;
+  bool service_unavailable_write_done_ = false;
+  bool service_unavailable_drain_done_ = false;
+
+  THREAD_CHECKER(thread_checker_);
+};
+
+BrowserOSServerProxy::BrowserOSServerProxy() {
+  DETACH_FROM_THREAD(thread_checker_);
+}
+
+BrowserOSServerProxy::~BrowserOSServerProxy() {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+  Stop();
+}
+
+bool BrowserOSServerProxy::Start(int http_port, int https_port) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  if (http_listener_) {
+    LOG(WARNING) << "browseros: Proxy already started on port "
+                 << bound_http_port_;
+    return false;
+  }
+
+  if (!StartHttpListener(http_port)) {
+    return false;
+  }
+
+  if (https_port != 0) {
+    StartHttpsListener(https_port);
+  }
+
+  return true;
+}
+
+bool BrowserOSServerProxy::StartHttpListener(int http_port) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  if (http_port < 0 || http_port > 65535) {
+    LOG(ERROR) << "browseros: Proxy invalid HTTP port " << http_port;
+    return false;
+  }
+
+  auto listener = std::make_unique<ListenerState>(/*is_https=*/false);
+  listener->socket =
+      std::make_unique<net::TCPServerSocket>(nullptr, net::NetLogSource());
+  int result = listener->socket->ListenWithAddressAndPort(
+      "0.0.0.0", static_cast<uint16_t>(http_port), kBackLog);
+  if (result != net::OK) {
+    LOG(ERROR) << "browseros: Proxy failed to bind 0.0.0.0:" << http_port
+               << " - " << net::ErrorToString(result);
+    return false;
+  }
+
+  net::IPEndPoint local_address;
+  result = listener->socket->GetLocalAddress(&local_address);
+  if (result != net::OK) {
+    LOG(ERROR) << "browseros: Proxy failed to read bound port - "
+               << net::ErrorToString(result);
+    return false;
+  }
+
+  http_listener_ = std::move(listener);
+  bound_http_port_ = local_address.port();
+  StartAccept(http_listener_.get());
+
+  LOG(INFO) << "browseros: MCP proxy listening on 0.0.0.0:" << bound_http_port_;
+  return true;
+}
+
+bool BrowserOSServerProxy::StartHttpsListener(int https_port) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  if (https_port < 0 || https_port > 65535) {
+    LOG(ERROR) << "browseros: Proxy invalid HTTPS port " << https_port
+               << ", continuing HTTP-only";
+    return false;
+  }
+
+  if (!GenerateTlsCredentials()) {
+    return false;
+  }
+
+  auto listener = std::make_unique<ListenerState>(/*is_https=*/true);
+  listener->socket =
+      std::make_unique<net::TCPServerSocket>(nullptr, net::NetLogSource());
+  int result = listener->socket->ListenWithAddressAndPort(
+      "0.0.0.0", static_cast<uint16_t>(https_port), kBackLog);
+  if (result != net::OK) {
+    LOG(ERROR) << "browseros: HTTPS proxy failed to bind 0.0.0.0:" << https_port
+               << " - " << net::ErrorToString(result)
+               << ", continuing HTTP-only";
+    tls_context_.reset();
+    tls_cert_.reset();
+    tls_key_.reset();
+    return false;
+  }
+
+  net::IPEndPoint local_address;
+  result = listener->socket->GetLocalAddress(&local_address);
+  if (result != net::OK) {
+    LOG(ERROR) << "browseros: HTTPS proxy failed to read bound port - "
+               << net::ErrorToString(result) << ", continuing HTTP-only";
+    tls_context_.reset();
+    tls_cert_.reset();
+    tls_key_.reset();
+    return false;
+  }
+
+  https_listener_ = std::move(listener);
+  bound_https_port_ = local_address.port();
+  StartAccept(https_listener_.get());
+
+  LOG(INFO) << "browseros: HTTPS MCP proxy listening on 0.0.0.0:"
+            << bound_https_port_;
+  return true;
+}
+
+bool BrowserOSServerProxy::GenerateTlsCredentials() {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  tls_key_ = std::make_unique<crypto::keypair::PrivateKey>(
+      crypto::keypair::PrivateKey::GenerateEcP256());
+
+  std::string der_cert;
+  const base::Time now = base::Time::Now();
+  const uint32_t serial_number = static_cast<uint32_t>(
+      base::RandIntInclusive(1, std::numeric_limits<int>::max()));
+  if (!net::x509_util::CreateSelfSignedCert(
+          tls_key_->key(), net::x509_util::DIGEST_SHA256, "CN=localhost",
+          serial_number, now - base::Days(1), now + base::Days(3650), {},
+          &der_cert)) {
+    LOG(ERROR) << "browseros: HTTPS proxy failed to create certificate, "
+                  "continuing HTTP-only";
+    tls_key_.reset();
+    return false;
+  }
+
+  tls_cert_ =
+      net::X509Certificate::CreateFromBytes(base::as_byte_span(der_cert));
+  if (!tls_cert_) {
+    LOG(ERROR) << "browseros: HTTPS proxy failed to parse certificate, "
+                  "continuing HTTP-only";
+    tls_key_.reset();
+    return false;
+  }
+
+  tls_context_ = net::CreateSSLServerContext(tls_cert_.get(), tls_key_->key(),
+                                             net::SSLServerConfig());
+  if (!tls_context_) {
+    LOG(ERROR) << "browseros: HTTPS proxy failed to create SSL context, "
+                  "continuing HTTP-only";
+    tls_cert_.reset();
+    tls_key_.reset();
+    return false;
+  }
+
+  return true;
+}
+
+void BrowserOSServerProxy::Stop() {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  weak_factory_.InvalidateWeakPtrs();
+  connections_.clear();
+  tls_handshakes_.clear();
+
+  if (http_listener_) {
+    LOG(INFO) << "browseros: Stopping MCP proxy on port " << bound_http_port_;
+    http_listener_.reset();
+  }
+
+  if (https_listener_) {
+    LOG(INFO) << "browseros: Stopping HTTPS MCP proxy on port "
+              << bound_https_port_;
+    https_listener_.reset();
+  }
+
+  tls_context_.reset();
+  tls_cert_.reset();
+  tls_key_.reset();
+  bound_http_port_ = 0;
+  bound_https_port_ = 0;
+}
+
+void BrowserOSServerProxy::SetBackendPort(int port) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  backend_port_ = port;
+  LOG(INFO) << "browseros: Proxy backend port set to " << port;
+}
+
+void BrowserOSServerProxy::SetAllowRemote(bool allow) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  allow_remote_ = allow;
+  LOG(INFO) << "browseros: Proxy allow_remote set to "
+            << (allow ? "true" : "false");
+}
+
+void BrowserOSServerProxy::StartAccept(ListenerState* listener) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  if (!listener || !listener->socket) {
+    return;
+  }
+
+  listener->pending_accept_socket.reset();
+  listener->pending_peer_address = net::IPEndPoint();
+  int result = listener->socket->Accept(
+      &listener->pending_accept_socket,
+      base::BindOnce(&BrowserOSServerProxy::OnAccept,
+                     weak_factory_.GetWeakPtr(), listener),
+      &listener->pending_peer_address);
+  if (result != net::ERR_IO_PENDING) {
+    OnAccept(listener, result);
+    return;
+  }
+}
+
+void BrowserOSServerProxy::OnAccept(ListenerState* listener, int result) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  if (!listener || !listener->socket) {
+    return;
+  }
+
+  if (result != net::OK) {
+    LOG(ERROR) << "browseros: " << (listener->is_https ? "HTTPS " : "")
+               << "Proxy accept failed - " << net::ErrorToString(result);
+    base::SingleThreadTaskRunner::GetCurrentDefault()->PostDelayedTask(
+        FROM_HERE,
+        base::BindOnce(&BrowserOSServerProxy::StartAccept,
+                       weak_factory_.GetWeakPtr(), listener),
+        base::Milliseconds(100));
+    return;
+  }
+
+  std::unique_ptr<net::StreamSocket> client_socket =
+      std::move(listener->pending_accept_socket);
+  net::IPEndPoint peer_address = listener->pending_peer_address;
+  listener->pending_peer_address = net::IPEndPoint();
+
+  if (!client_socket) {
+    StartAccept(listener);
+    return;
+  }
+
+  if (!allow_remote_ && !peer_address.address().IsLoopback()) {
+    StartAccept(listener);
+    return;
+  }
+
+  if (listener->is_https) {
+    StartTlsHandshake(std::move(client_socket));
+  } else {
+    StartConnection(std::move(client_socket));
+  }
+
+  StartAccept(listener);
+}
+
+void BrowserOSServerProxy::StartConnection(
+    std::unique_ptr<net::StreamSocket> client_socket) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  int connection_id = next_connection_id_++;
+  auto connection = std::make_unique<Connection>(
+      connection_id, std::move(client_socket), backend_port_,
+      base::BindRepeating(&BrowserOSServerProxy::OnConnectionFinished,
+                          base::Unretained(this)));
+  Connection* connection_ptr = connection.get();
+  connections_[connection_id] = std::move(connection);
+  connection_ptr->Start();
+}
+
+void BrowserOSServerProxy::StartTlsHandshake(
+    std::unique_ptr<net::StreamSocket> client_socket) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  if (!tls_context_) {
+    return;
+  }
+
+  std::unique_ptr<net::SSLServerSocket> ssl_socket =
+      tls_context_->CreateSSLServerSocket(std::move(client_socket));
+  if (!ssl_socket) {
+    return;
+  }
+
+  int handshake_id = next_tls_handshake_id_++;
+  net::SSLServerSocket* ssl_socket_ptr = ssl_socket.get();
+  tls_handshakes_[handshake_id] = std::move(ssl_socket);
+  int result = ssl_socket_ptr->Handshake(
+      base::BindOnce(&BrowserOSServerProxy::OnTlsHandshakeDone,
+                     weak_factory_.GetWeakPtr(), handshake_id));
+  if (result != net::ERR_IO_PENDING) {
+    OnTlsHandshakeDone(handshake_id, result);
+    return;
+  }
+}
+
+void BrowserOSServerProxy::OnTlsHandshakeDone(int handshake_id, int result) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  auto it = tls_handshakes_.find(handshake_id);
+  if (it == tls_handshakes_.end()) {
+    return;
+  }
+
+  if (result != net::OK) {
+    VLOG(1) << "browseros: HTTPS proxy handshake failed - "
+            << net::ErrorToString(result);
+    tls_handshakes_.erase(it);
+    return;
+  }
+
+  std::unique_ptr<net::StreamSocket> client_socket = std::move(it->second);
+  tls_handshakes_.erase(it);
+  StartConnection(std::move(client_socket));
+}
+
+void BrowserOSServerProxy::OnConnectionFinished(int connection_id) {
+  DCHECK_CALLED_ON_VALID_THREAD(thread_checker_);
+
+  connections_.erase(connection_id);
+}
+
+}  // namespace browseros
