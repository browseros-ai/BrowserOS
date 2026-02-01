diff --git a/chrome/browser/browseros/server/browseros_server_proxy.cc b/chrome/browser/browseros/server/browseros_server_proxy.cc
new file mode 100644
index 0000000000000..a0a6655ba0923
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_proxy.cc
@@ -0,0 +1,218 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_proxy.h"
+
+#include <optional>
+
+#include "base/functional/bind.h"
+#include "base/logging.h"
+#include "base/strings/string_number_conversions.h"
+#include "net/base/ip_address.h"
+#include "net/base/net_errors.h"
+#include "net/http/http_status_code.h"
+#include "net/log/net_log_source.h"
+#include "net/server/http_server_request_info.h"
+#include "net/server/http_server_response_info.h"
+#include "net/socket/tcp_server_socket.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/shared_url_loader_factory.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+#include "services/network/public/mojom/url_response_head.mojom.h"
+#include "url/gurl.h"
+
+namespace browseros {
+
+namespace {
+
+constexpr int kBackLog = 10;
+constexpr size_t kMaxResponseBodySize = 5 * 1024 * 1024;  // 5 MB
+
+net::NetworkTrafficAnnotationTag GetProxyTrafficAnnotation() {
+  return net::DefineNetworkTrafficAnnotation("browseros_mcp_proxy", R"(
+    semantics {
+      sender: "BrowserOS MCP Proxy"
+      description:
+        "Forwards MCP requests from the stable proxy port to the sidecar's "
+        "ephemeral backend port."
+      trigger: "External MCP client sends POST /mcp to the proxy port."
+      data: "MCP JSON-RPC request body."
+      destination: LOCAL
+    }
+    policy {
+      cookies_allowed: NO
+      setting: "This feature cannot be disabled by settings."
+      policy_exception_justification:
+        "Internal proxy for BrowserOS MCP server functionality."
+    })");
+}
+
+void Send503(net::HttpServer* server, int connection_id) {
+  net::HttpServerResponseInfo response(net::HTTP_SERVICE_UNAVAILABLE);
+  response.SetBody("Service Unavailable", "text/plain");
+  server->SendResponse(connection_id, response, GetProxyTrafficAnnotation());
+}
+
+}  // namespace
+
+BrowserOSServerProxy::BrowserOSServerProxy() = default;
+
+BrowserOSServerProxy::~BrowserOSServerProxy() {
+  Stop();
+}
+
+bool BrowserOSServerProxy::Start(
+    int port,
+    std::unique_ptr<network::PendingSharedURLLoaderFactory> pending_factory) {
+  if (server_) {
+    LOG(WARNING) << "browseros: Proxy already started on port " << bound_port_;
+    return false;
+  }
+
+  // Bind the cloned factory on this (IO) thread.
+  url_loader_factory_ =
+      network::SharedURLLoaderFactory::Create(std::move(pending_factory));
+
+  auto server_socket =
+      std::make_unique<net::TCPServerSocket>(nullptr, net::NetLogSource());
+  int result = server_socket->ListenWithAddressAndPort("0.0.0.0", port,
+                                                        kBackLog);
+  if (result != net::OK) {
+    LOG(ERROR) << "browseros: Proxy failed to bind 0.0.0.0:" << port
+               << " - " << net::ErrorToString(result);
+    return false;
+  }
+
+  server_ = std::make_unique<net::HttpServer>(std::move(server_socket), this);
+  bound_port_ = port;
+
+  LOG(INFO) << "browseros: MCP proxy listening on 0.0.0.0:" << bound_port_;
+  return true;
+}
+
+void BrowserOSServerProxy::Stop() {
+  if (server_) {
+    LOG(INFO) << "browseros: Stopping MCP proxy on port " << bound_port_;
+    server_.reset();
+    bound_port_ = 0;
+  }
+  url_loader_factory_.reset();
+}
+
+void BrowserOSServerProxy::SetBackendPort(int port) {
+  backend_port_ = port;
+  LOG(INFO) << "browseros: Proxy backend port set to " << port;
+}
+
+void BrowserOSServerProxy::SetAllowRemote(bool allow) {
+  allow_remote_ = allow;
+  LOG(INFO) << "browseros: Proxy allow_remote set to "
+            << (allow ? "true" : "false");
+}
+
+void BrowserOSServerProxy::OnConnect(int connection_id) {}
+
+void BrowserOSServerProxy::OnHttpRequest(
+    int connection_id,
+    const net::HttpServerRequestInfo& info) {
+  if (!allow_remote_ && !info.peer.address().IsLoopback()) {
+    net::HttpServerResponseInfo response(net::HTTP_FORBIDDEN);
+    response.SetBody("Remote connections not allowed", "text/plain");
+    server_->SendResponse(connection_id, response,
+                          GetProxyTrafficAnnotation());
+    server_->Close(connection_id);
+    return;
+  }
+
+  ForwardRequest(connection_id, info);
+}
+
+void BrowserOSServerProxy::OnWebSocketRequest(
+    int connection_id,
+    const net::HttpServerRequestInfo& info) {
+  server_->Close(connection_id);
+}
+
+void BrowserOSServerProxy::OnWebSocketMessage(int connection_id,
+                                               std::string data) {
+  server_->Close(connection_id);
+}
+
+void BrowserOSServerProxy::OnClose(int connection_id) {}
+
+void BrowserOSServerProxy::ForwardRequest(
+    int connection_id,
+    const net::HttpServerRequestInfo& info) {
+  if (backend_port_ <= 0 || !url_loader_factory_) {
+    Send503(server_.get(), connection_id);
+    return;
+  }
+
+  GURL backend_url("http://127.0.0.1:" + base::NumberToString(backend_port_) +
+                    info.path);
+
+  auto resource_request = std::make_unique<network::ResourceRequest>();
+  resource_request->url = backend_url;
+  resource_request->method = info.method;
+  resource_request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+
+  for (const auto& [name, value] : info.headers) {
+    if (name == "content-type" || name == "accept" || name == "authorization") {
+      resource_request->headers.SetHeader(name, value);
+    }
+  }
+
+  auto loader = network::SimpleURLLoader::Create(std::move(resource_request),
+                                                  GetProxyTrafficAnnotation());
+
+  if (!info.data.empty()) {
+    loader->AttachStringForUpload(info.data);
+  }
+
+  loader->SetTimeoutDuration(base::Seconds(300));
+
+  auto* loader_ptr = loader.get();
+  loader_ptr->DownloadToString(
+      url_loader_factory_.get(),
+      base::BindOnce(&BrowserOSServerProxy::OnBackendResponse,
+                     base::Unretained(this), connection_id, std::move(loader)),
+      kMaxResponseBodySize);
+}
+
+void BrowserOSServerProxy::OnBackendResponse(
+    int connection_id,
+    std::unique_ptr<network::SimpleURLLoader> loader,
+    std::unique_ptr<std::string> response_body) {
+  if (!server_) {
+    return;
+  }
+
+  int response_code = 0;
+  auto* response_info = loader->ResponseInfo();
+  if (response_info && response_info->headers) {
+    response_code = response_info->headers->response_code();
+  }
+
+  if (!response_body || response_code == 0) {
+    Send503(server_.get(), connection_id);
+    return;
+  }
+
+  std::string content_type = "application/json";
+  if (response_info && response_info->headers) {
+    std::optional<std::string> ct =
+        response_info->headers->GetNormalizedHeader("content-type");
+    if (ct.has_value()) {
+      content_type = std::move(*ct);
+    }
+  }
+
+  net::HttpServerResponseInfo response(
+      static_cast<net::HttpStatusCode>(response_code));
+  response.SetBody(*response_body, content_type);
+  server_->SendResponse(connection_id, response, GetProxyTrafficAnnotation());
+}
+
+}  // namespace browseros
