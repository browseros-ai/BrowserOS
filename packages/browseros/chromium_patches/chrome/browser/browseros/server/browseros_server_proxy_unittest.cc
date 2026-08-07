diff --git a/chrome/browser/browseros/server/browseros_server_proxy_unittest.cc b/chrome/browser/browseros/server/browseros_server_proxy_unittest.cc
new file mode 100644
index 0000000000000..522e6b7c49265
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_proxy_unittest.cc
@@ -0,0 +1,1257 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_proxy.h"
+
+#include <deque>
+#include <memory>
+#include <optional>
+#include <string>
+#include <string_view>
+#include <utility>
+
+#include "build/build_config.h"
+
+#if BUILDFLAG(IS_POSIX)
+#include <sys/socket.h>
+#endif
+
+#include "base/byte_count.h"
+#include "base/containers/span.h"
+#include "base/functional/bind.h"
+#include "base/functional/callback.h"
+#include "base/memory/ref_counted.h"
+#include "base/memory/weak_ptr.h"
+#include "base/run_loop.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/synchronization/lock.h"
+#include "base/test/task_environment.h"
+#include "base/threading/thread.h"
+#include "base/time/time.h"
+#include "base/timer/timer.h"
+#include "net/base/address_list.h"
+#include "net/base/host_port_pair.h"
+#include "net/base/io_buffer.h"
+#include "net/base/ip_address.h"
+#include "net/base/ip_endpoint.h"
+#include "net/base/net_errors.h"
+#include "net/base/test_completion_callback.h"
+#include "net/cert/mock_cert_verifier.h"
+#include "net/http/http_response_headers.h"
+#include "net/http/http_status_code.h"
+#include "net/http/http_util.h"
+#include "net/http/transport_security_state.h"
+#include "net/log/net_log_source.h"
+#include "net/socket/socket_descriptor.h"
+#include "net/socket/ssl_client_socket.h"
+#include "net/socket/stream_socket.h"
+#include "net/socket/tcp_client_socket.h"
+#include "net/socket/tcp_server_socket.h"
+#include "net/ssl/ssl_client_session_cache.h"
+#include "net/ssl/ssl_config.h"
+#include "net/ssl/test_ssl_config_service.h"
+#include "net/test/embedded_test_server/embedded_test_server.h"
+#include "net/test/embedded_test_server/http_request.h"
+#include "net/test/embedded_test_server/http_response.h"
+#include "net/test/gtest_util.h"
+#include "net/traffic_annotation/network_traffic_annotation_test_helper.h"
+#include "testing/gmock/include/gmock/gmock.h"
+#include "testing/gtest/include/gtest/gtest.h"
+
+namespace browseros {
+namespace {
+
+using net::test::IsOk;
+using testing::HasSubstr;
+
+constexpr char kServiceUnavailableResponse[] =
+    "HTTP/1.1 503 Service Unavailable\r\n"
+    "Content-Type: text/plain\r\n"
+    "Content-Length: 19\r\n"
+    "Connection: close\r\n"
+    "\r\n"
+    "Service Unavailable";
+
+class ReadResultState : public base::RefCounted<ReadResultState> {
+ public:
+  void Complete(int read_result) {
+    result = read_result;
+    done = true;
+    if (quit) {
+      quit.Run();
+    }
+  }
+
+  int result = net::ERR_IO_PENDING;
+  bool done = false;
+  base::RepeatingClosure quit;
+
+ private:
+  friend class base::RefCounted<ReadResultState>;
+  ~ReadResultState() = default;
+};
+
+std::string ResponseWithBody(std::string_view body) {
+  return "HTTP/1.1 200 OK\r\nContent-Length: " +
+         base::NumberToString(body.size()) +
+         "\r\nContent-Type: text/plain\r\n\r\n" + std::string(body);
+}
+
+char PatternByte(size_t index) {
+  return static_cast<char>('!' + (index % 90));
+}
+
+std::string MakePattern(size_t size) {
+  std::string data(size, '\0');
+  for (size_t i = 0; i < data.size(); ++i) {
+    data[i] = PatternByte(i);
+  }
+  return data;
+}
+
+testing::AssertionResult HasPattern(std::string_view data) {
+  for (size_t i = 0; i < data.size(); ++i) {
+    if (data[i] != PatternByte(i)) {
+      return testing::AssertionFailure()
+             << "byte " << i << " expected " << static_cast<int>(PatternByte(i))
+             << " got " << static_cast<int>(data[i]);
+    }
+  }
+  return testing::AssertionSuccess();
+}
+
+class TestTcpClient {
+ public:
+  TestTcpClient() = default;
+
+  TestTcpClient(const TestTcpClient&) = delete;
+  TestTcpClient& operator=(const TestTcpClient&) = delete;
+
+  int Connect(int port) {
+    socket_ = std::make_unique<net::TCPClientSocket>(
+        net::AddressList(
+            net::IPEndPoint(net::IPAddress::IPv4Localhost(), port)),
+        nullptr, nullptr, nullptr, net::NetLogSource());
+
+    net::TestCompletionCallback callback;
+    int result = socket_->Connect(callback.callback());
+    return callback.GetResult(result);
+  }
+
+  bool Write(std::string_view data) {
+    auto buffer = base::MakeRefCounted<net::DrainableIOBuffer>(
+        base::MakeRefCounted<net::StringIOBuffer>(std::string(data)),
+        data.size());
+
+    while (buffer->BytesRemaining() > 0) {
+      net::TestCompletionCallback callback;
+      int result =
+          socket_->Write(buffer.get(), buffer->BytesRemaining(),
+                         callback.callback(), TRAFFIC_ANNOTATION_FOR_TESTS);
+      result = callback.GetResult(result);
+      if (result <= 0) {
+        return false;
+      }
+      buffer->DidConsume(result);
+    }
+
+    return true;
+  }
+
+  bool ReadExact(size_t byte_count,
+                 std::string* response,
+                 base::TimeDelta timeout = base::Seconds(10)) {
+    response->clear();
+    while (response->size() < byte_count) {
+      std::string chunk;
+      int result = ReadSome(&chunk, timeout);
+      if (result <= 0) {
+        return false;
+      }
+      response->append(chunk);
+    }
+    return true;
+  }
+
+  bool ReadHttpResponse(std::string* response) {
+    response->clear();
+    while (!IsCompleteHttpResponse(*response)) {
+      std::string chunk;
+      int result = ReadSome(&chunk, base::Seconds(10));
+      if (result <= 0) {
+        return false;
+      }
+      response->append(chunk);
+    }
+    return true;
+  }
+
+  std::string ReadUntilEOF() {
+    std::string response;
+    while (true) {
+      std::string chunk;
+      int result = ReadSome(&chunk, base::Seconds(10));
+      if (result <= 0) {
+        return response;
+      }
+      response.append(chunk);
+    }
+  }
+
+  bool ShutdownWrite() {
+#if BUILDFLAG(IS_POSIX)
+    return shutdown(socket_->SocketDescriptorForTesting(), SHUT_WR) == 0;
+#else
+    return false;
+#endif
+  }
+
+  void Close() { socket_.reset(); }
+
+ private:
+  int ReadSome(std::string* chunk, base::TimeDelta timeout) {
+    auto buffer = base::MakeRefCounted<net::IOBufferWithSize>(4096);
+    auto state = base::MakeRefCounted<ReadResultState>();
+    int result =
+        socket_->Read(buffer.get(), buffer->size(),
+                      base::BindOnce(&ReadResultState::Complete, state));
+    if (result == net::ERR_IO_PENDING) {
+      base::RunLoop run_loop;
+      base::OneShotTimer timer;
+      state->quit = run_loop.QuitClosure();
+      timer.Start(FROM_HERE, timeout, run_loop.QuitClosure());
+      run_loop.Run();
+      state->quit.Reset();
+      if (!state->done) {
+        socket_.reset();
+        chunk->clear();
+        return net::ERR_TIMED_OUT;
+      }
+      result = state->result;
+    }
+    if (result > 0) {
+      chunk->assign(buffer->data(), result);
+    } else {
+      chunk->clear();
+    }
+    return result;
+  }
+
+  bool IsCompleteHttpResponse(const std::string& response) {
+    size_t end_of_headers =
+        net::HttpUtil::LocateEndOfHeaders(base::as_byte_span(response));
+    if (end_of_headers == std::string::npos) {
+      return false;
+    }
+
+    auto headers = base::MakeRefCounted<net::HttpResponseHeaders>(
+        net::HttpUtil::AssembleRawHeaders(
+            std::string_view(response.data(), end_of_headers)));
+    std::optional<base::ByteCount> content_length = headers->GetContentLength();
+    if (!content_length.has_value()) {
+      return false;
+    }
+
+    return response.size() - end_of_headers >=
+           static_cast<size_t>(content_length->InBytes());
+  }
+
+  std::unique_ptr<net::TCPClientSocket> socket_;
+};
+
+class TestTlsClient {
+ public:
+  TestTlsClient()
+      : ssl_config_service_(net::SSLContextConfig()),
+        ssl_client_session_cache_(net::SSLClientSessionCache::Config()) {
+    cert_verifier_.set_default_result(net::OK);
+    ssl_client_context_ = std::make_unique<net::SSLClientContext>(
+        &ssl_config_service_, &cert_verifier_, &transport_security_state_,
+        &ssl_client_session_cache_, nullptr);
+  }
+
+  TestTlsClient(const TestTlsClient&) = delete;
+  TestTlsClient& operator=(const TestTlsClient&) = delete;
+
+  int Connect(int port) {
+    auto tcp_socket = std::make_unique<net::TCPClientSocket>(
+        net::AddressList(
+            net::IPEndPoint(net::IPAddress::IPv4Localhost(), port)),
+        nullptr, nullptr, nullptr, net::NetLogSource());
+
+    net::TestCompletionCallback tcp_callback;
+    int result = tcp_socket->Connect(tcp_callback.callback());
+    result = tcp_callback.GetResult(result);
+    if (result != net::OK) {
+      return result;
+    }
+
+    ssl_socket_ = ssl_client_context_->CreateSSLClientSocket(
+        std::move(tcp_socket), net::HostPortPair("localhost", port),
+        net::SSLConfig());
+    if (!ssl_socket_) {
+      return net::ERR_FAILED;
+    }
+
+    net::TestCompletionCallback ssl_callback;
+    result = ssl_socket_->Connect(ssl_callback.callback());
+    return ssl_callback.GetResult(result);
+  }
+
+  bool Write(std::string_view data) {
+    auto buffer = base::MakeRefCounted<net::DrainableIOBuffer>(
+        base::MakeRefCounted<net::StringIOBuffer>(std::string(data)),
+        data.size());
+
+    while (buffer->BytesRemaining() > 0) {
+      net::TestCompletionCallback callback;
+      int result =
+          ssl_socket_->Write(buffer.get(), buffer->BytesRemaining(),
+                             callback.callback(), TRAFFIC_ANNOTATION_FOR_TESTS);
+      result = callback.GetResult(result);
+      if (result <= 0) {
+        return false;
+      }
+      buffer->DidConsume(result);
+    }
+
+    return true;
+  }
+
+  bool ReadExact(size_t byte_count,
+                 std::string* response,
+                 base::TimeDelta timeout = base::Seconds(10)) {
+    response->clear();
+    while (response->size() < byte_count) {
+      std::string chunk;
+      int result = ReadSome(&chunk, timeout);
+      if (result <= 0) {
+        return false;
+      }
+      response->append(chunk);
+    }
+    return true;
+  }
+
+  std::string ReadUntilEOF() {
+    std::string response;
+    while (true) {
+      std::string chunk;
+      int result = ReadSome(&chunk, base::Seconds(10));
+      if (result <= 0) {
+        return response;
+      }
+      response.append(chunk);
+    }
+  }
+
+  void Close() { ssl_socket_.reset(); }
+
+ private:
+  int ReadSome(std::string* chunk, base::TimeDelta timeout) {
+    auto buffer = base::MakeRefCounted<net::IOBufferWithSize>(4096);
+    auto state = base::MakeRefCounted<ReadResultState>();
+    int result =
+        ssl_socket_->Read(buffer.get(), buffer->size(),
+                          base::BindOnce(&ReadResultState::Complete, state));
+    if (result == net::ERR_IO_PENDING) {
+      base::RunLoop run_loop;
+      base::OneShotTimer timer;
+      state->quit = run_loop.QuitClosure();
+      timer.Start(FROM_HERE, timeout, run_loop.QuitClosure());
+      run_loop.Run();
+      state->quit.Reset();
+      if (!state->done) {
+        ssl_socket_.reset();
+        chunk->clear();
+        return net::ERR_TIMED_OUT;
+      }
+      result = state->result;
+    }
+    if (result > 0) {
+      chunk->assign(buffer->data(), result);
+    } else {
+      chunk->clear();
+    }
+    return result;
+  }
+
+  net::TestSSLConfigService ssl_config_service_;
+  net::MockCertVerifier cert_verifier_;
+  net::TransportSecurityState transport_security_state_;
+  net::SSLClientSessionCache ssl_client_session_cache_;
+  std::unique_ptr<net::SSLClientContext> ssl_client_context_;
+  std::unique_ptr<net::SSLClientSocket> ssl_socket_;
+};
+
+class ScriptedTcpBackend {
+ public:
+  ScriptedTcpBackend() = default;
+
+  ScriptedTcpBackend(const ScriptedTcpBackend&) = delete;
+  ScriptedTcpBackend& operator=(const ScriptedTcpBackend&) = delete;
+
+  ~ScriptedTcpBackend() { Stop(); }
+
+  bool Start() {
+    server_socket_ =
+        std::make_unique<net::TCPServerSocket>(nullptr, net::NetLogSource());
+    int result = server_socket_->ListenWithAddressAndPort("127.0.0.1", 0, 1);
+    if (result != net::OK) {
+      return false;
+    }
+
+    net::IPEndPoint local_address;
+    result = server_socket_->GetLocalAddress(&local_address);
+    if (result != net::OK) {
+      return false;
+    }
+
+    port_ = local_address.port();
+    StartAccept();
+    return true;
+  }
+
+  void Stop() {
+    weak_factory_.InvalidateWeakPtrs();
+    accepted_socket_.reset();
+    pending_accept_socket_.reset();
+    server_socket_.reset();
+    read_buffer_.reset();
+    write_buffers_.clear();
+    writing_ = false;
+    close_after_writes_ = false;
+  }
+
+  bool QueueWrite(std::string data) {
+    if (!accepted_socket_) {
+      return false;
+    }
+
+    size_t data_size = data.size();
+    write_buffers_.push_back(base::MakeRefCounted<net::DrainableIOBuffer>(
+        base::MakeRefCounted<net::StringIOBuffer>(std::move(data)), data_size));
+    if (!writing_) {
+      WriteMore();
+    }
+    return true;
+  }
+
+  void CloseAfterPendingWrites() {
+    close_after_writes_ = true;
+    if (!writing_ && write_buffers_.empty()) {
+      accepted_socket_.reset();
+    }
+  }
+
+  int port() const { return port_; }
+  bool is_connected() const { return accepted_socket_ != nullptr; }
+  const std::string& received_bytes() const { return received_bytes_; }
+
+ private:
+  void StartAccept() {
+    int result = server_socket_->Accept(
+        &pending_accept_socket_, base::BindOnce(&ScriptedTcpBackend::OnAccept,
+                                                weak_factory_.GetWeakPtr()));
+    if (result != net::ERR_IO_PENDING) {
+      OnAccept(result);
+    }
+  }
+
+  void OnAccept(int result) {
+    if (result != net::OK) {
+      return;
+    }
+
+    accepted_socket_ = std::move(pending_accept_socket_);
+    StartRead();
+  }
+
+  void StartRead() {
+    if (!accepted_socket_) {
+      return;
+    }
+
+    read_buffer_ = base::MakeRefCounted<net::IOBufferWithSize>(4096);
+    int result =
+        accepted_socket_->Read(read_buffer_.get(), read_buffer_->size(),
+                               base::BindOnce(&ScriptedTcpBackend::OnRead,
+                                              weak_factory_.GetWeakPtr()));
+    if (result != net::ERR_IO_PENDING) {
+      OnRead(result);
+    }
+  }
+
+  void OnRead(int result) {
+    if (result <= 0) {
+      accepted_socket_.reset();
+      return;
+    }
+
+    received_bytes_.append(read_buffer_->data(), result);
+    StartRead();
+  }
+
+  void WriteMore() {
+    if (!accepted_socket_ || write_buffers_.empty()) {
+      writing_ = false;
+      if (close_after_writes_) {
+        accepted_socket_.reset();
+      }
+      return;
+    }
+
+    writing_ = true;
+    scoped_refptr<net::DrainableIOBuffer> buffer = write_buffers_.front();
+    int result =
+        accepted_socket_->Write(buffer.get(), buffer->BytesRemaining(),
+                                base::BindOnce(&ScriptedTcpBackend::OnWritten,
+                                               weak_factory_.GetWeakPtr()),
+                                TRAFFIC_ANNOTATION_FOR_TESTS);
+    if (result != net::ERR_IO_PENDING) {
+      OnWritten(result);
+    }
+  }
+
+  void OnWritten(int result) {
+    if (result <= 0) {
+      accepted_socket_.reset();
+      write_buffers_.clear();
+      writing_ = false;
+      return;
+    }
+
+    scoped_refptr<net::DrainableIOBuffer> buffer = write_buffers_.front();
+    buffer->DidConsume(result);
+    if (buffer->BytesRemaining() == 0) {
+      write_buffers_.pop_front();
+    }
+    WriteMore();
+  }
+
+  std::unique_ptr<net::TCPServerSocket> server_socket_;
+  std::unique_ptr<net::StreamSocket> pending_accept_socket_;
+  std::unique_ptr<net::StreamSocket> accepted_socket_;
+  scoped_refptr<net::IOBufferWithSize> read_buffer_;
+  std::deque<scoped_refptr<net::DrainableIOBuffer>> write_buffers_;
+  std::string received_bytes_;
+  int port_ = 0;
+  bool writing_ = false;
+  bool close_after_writes_ = false;
+  base::WeakPtrFactory<ScriptedTcpBackend> weak_factory_{this};
+};
+
+class KeepAliveBackend {
+ public:
+  KeepAliveBackend() = default;
+
+  KeepAliveBackend(const KeepAliveBackend&) = delete;
+  KeepAliveBackend& operator=(const KeepAliveBackend&) = delete;
+
+  ~KeepAliveBackend() { Stop(); }
+
+  bool Start() {
+    server_socket_ =
+        std::make_unique<net::TCPServerSocket>(nullptr, net::NetLogSource());
+    int result = server_socket_->ListenWithAddressAndPort("127.0.0.1", 0, 1);
+    if (result != net::OK) {
+      return false;
+    }
+
+    net::IPEndPoint local_address;
+    result = server_socket_->GetLocalAddress(&local_address);
+    if (result != net::OK) {
+      return false;
+    }
+
+    port_ = local_address.port();
+    StartAccept();
+    return true;
+  }
+
+  void Stop() {
+    weak_factory_.InvalidateWeakPtrs();
+    accepted_socket_.reset();
+    pending_accept_socket_.reset();
+    server_socket_.reset();
+    write_buffer_.reset();
+    read_buffer_.reset();
+  }
+
+  int port() const { return port_; }
+  int request_count() const { return request_count_; }
+
+ private:
+  void StartAccept() {
+    int result = server_socket_->Accept(
+        &pending_accept_socket_, base::BindOnce(&KeepAliveBackend::OnAccept,
+                                                weak_factory_.GetWeakPtr()));
+    if (result != net::ERR_IO_PENDING) {
+      OnAccept(result);
+    }
+  }
+
+  void OnAccept(int result) {
+    if (result != net::OK) {
+      return;
+    }
+
+    accepted_socket_ = std::move(pending_accept_socket_);
+    StartRead();
+  }
+
+  void StartRead() {
+    read_buffer_ = base::MakeRefCounted<net::IOBufferWithSize>(4096);
+    int result = accepted_socket_->Read(
+        read_buffer_.get(), read_buffer_->size(),
+        base::BindOnce(&KeepAliveBackend::OnRead, weak_factory_.GetWeakPtr()));
+    if (result != net::ERR_IO_PENDING) {
+      OnRead(result);
+    }
+  }
+
+  void OnRead(int result) {
+    if (result <= 0) {
+      accepted_socket_.reset();
+      return;
+    }
+
+    request_bytes_.append(read_buffer_->data(), result);
+    if (request_bytes_.find("\r\n\r\n") == std::string::npos) {
+      StartRead();
+      return;
+    }
+
+    request_bytes_.clear();
+    ++request_count_;
+    const bool close_after_write = request_count_ == 2;
+    std::string response = request_count_ == 1
+                               ? ResponseWithBody("first-body")
+                               : ResponseWithBody("second-body");
+    WriteResponse(std::move(response), close_after_write);
+  }
+
+  void WriteResponse(std::string response, bool close_after_write) {
+    size_t response_size = response.size();
+    write_buffer_ = base::MakeRefCounted<net::DrainableIOBuffer>(
+        base::MakeRefCounted<net::StringIOBuffer>(std::move(response)),
+        response_size);
+    close_after_write_ = close_after_write;
+    WriteMore();
+  }
+
+  void WriteMore() {
+    int result = accepted_socket_->Write(
+        write_buffer_.get(), write_buffer_->BytesRemaining(),
+        base::BindOnce(&KeepAliveBackend::OnWritten,
+                       weak_factory_.GetWeakPtr()),
+        TRAFFIC_ANNOTATION_FOR_TESTS);
+    if (result != net::ERR_IO_PENDING) {
+      OnWritten(result);
+    }
+  }
+
+  void OnWritten(int result) {
+    if (result <= 0) {
+      accepted_socket_.reset();
+      return;
+    }
+
+    write_buffer_->DidConsume(result);
+    if (write_buffer_->BytesRemaining() > 0) {
+      WriteMore();
+      return;
+    }
+
+    write_buffer_.reset();
+    if (close_after_write_) {
+      accepted_socket_.reset();
+      return;
+    }
+
+    StartRead();
+  }
+
+  std::unique_ptr<net::TCPServerSocket> server_socket_;
+  std::unique_ptr<net::StreamSocket> pending_accept_socket_;
+  std::unique_ptr<net::StreamSocket> accepted_socket_;
+  scoped_refptr<net::IOBufferWithSize> read_buffer_;
+  scoped_refptr<net::DrainableIOBuffer> write_buffer_;
+  std::string request_bytes_;
+  int port_ = 0;
+  int request_count_ = 0;
+  bool close_after_write_ = false;
+  base::WeakPtrFactory<KeepAliveBackend> weak_factory_{this};
+};
+
+class BrowserOSServerProxyTest : public testing::Test {
+ protected:
+  void SetUp() override {
+    ASSERT_TRUE(proxy_.Start(0, 0));
+    proxy_.SetAllowRemote(false);
+  }
+
+  void TearDown() override {
+    proxy_.Stop();
+    task_environment_.RunUntilIdle();
+  }
+
+  void StartEmbeddedBackend() {
+    ASSERT_TRUE(backend_.Start());
+    proxy_.SetBackendPort(backend_.port());
+  }
+
+  void RestartProxyWithHttps() {
+    proxy_.Stop();
+    task_environment_.RunUntilIdle();
+
+    for (int attempt = 0; attempt < 5; ++attempt) {
+      ASSERT_TRUE(proxy_.Start(0, GetUnusedLocalPort()));
+      proxy_.SetAllowRemote(false);
+      if (proxy_.GetHttpsPort() > 0) {
+        ASSERT_GT(proxy_.GetPort(), 0);
+        return;
+      }
+
+      proxy_.Stop();
+      task_environment_.RunUntilIdle();
+    }
+
+    FAIL() << "HTTPS listener failed to bind after retries";
+  }
+
+  int GetUnusedLocalPort() {
+    net::TCPServerSocket socket(nullptr, net::NetLogSource());
+    EXPECT_THAT(socket.ListenWithAddressAndPort("127.0.0.1", 0, 1), IsOk());
+
+    net::IPEndPoint local_address;
+    EXPECT_THAT(socket.GetLocalAddress(&local_address), IsOk());
+    return local_address.port();
+  }
+
+  template <typename Predicate>
+  bool RunUntil(Predicate predicate,
+                base::TimeDelta timeout = base::Seconds(10)) {
+    const base::TimeTicks deadline = base::TimeTicks::Now() + timeout;
+    while (!predicate()) {
+      if (base::TimeTicks::Now() >= deadline) {
+        return false;
+      }
+
+      base::RunLoop run_loop;
+      base::OneShotTimer timer;
+      timer.Start(FROM_HERE, base::Milliseconds(10), run_loop.QuitClosure());
+      run_loop.Run();
+    }
+    return true;
+  }
+
+  base::test::TaskEnvironment task_environment_{
+      base::test::TaskEnvironment::MainThreadType::IO};
+  BrowserOSServerProxy proxy_;
+  net::EmbeddedTestServer backend_;
+  base::Lock lock_;
+};
+
+TEST(BrowserOSServerProxyThreadingTest,
+     StartsOnIoThreadAfterConstructionOnAnotherThread) {
+  base::test::TaskEnvironment task_environment{
+      base::test::TaskEnvironment::MainThreadType::IO};
+  base::Thread construction_thread("proxy_construction_thread");
+  ASSERT_TRUE(construction_thread.Start());
+
+  std::unique_ptr<BrowserOSServerProxy> proxy;
+  base::RunLoop run_loop;
+  construction_thread.task_runner()->PostTask(
+      FROM_HERE, base::BindOnce(
+                     [](std::unique_ptr<BrowserOSServerProxy>* proxy,
+                        base::OnceClosure done) {
+                       *proxy = std::make_unique<BrowserOSServerProxy>();
+                       std::move(done).Run();
+                     },
+                     &proxy, run_loop.QuitClosure()));
+  run_loop.Run();
+  construction_thread.Stop();
+
+  ASSERT_TRUE(proxy);
+  ASSERT_TRUE(proxy->Start(0, 0));
+  EXPECT_GT(proxy->GetPort(), 0);
+  proxy->Stop();
+}
+
+TEST_F(BrowserOSServerProxyTest, GetForwardsRawResponseAndRequestHeaders) {
+  std::string received_custom_header;
+  backend_.RegisterRequestHandler(base::BindRepeating(
+      [](base::Lock* lock, std::string* received_custom_header,
+         const net::test_server::HttpRequest& request)
+          -> std::unique_ptr<net::test_server::HttpResponse> {
+        if (request.relative_url != "/get") {
+          return nullptr;
+        }
+
+        auto header = request.headers.find("x-custom-foo");
+        {
+          base::AutoLock auto_lock(*lock);
+          if (header != request.headers.end()) {
+            *received_custom_header = header->second;
+          }
+        }
+
+        auto response = std::make_unique<net::test_server::BasicHttpResponse>();
+        response->set_code(net::HTTP_CREATED);
+        response->set_content_type("text/plain");
+        response->set_content("get-body");
+        response->AddCustomHeader("X-Backend-Header", "get-value");
+        return response;
+      },
+      &lock_, &received_custom_header));
+  StartEmbeddedBackend();
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(
+      client.Write("GET /get HTTP/1.1\r\n"
+                   "Host: example.test\r\n"
+                   "x-custom-foo: bar\r\n"
+                   "Connection: close\r\n"
+                   "\r\n"));
+
+  EXPECT_EQ(
+      "HTTP/1.1 201 Created\r\n"
+      "Connection: close\r\n"
+      "Content-Length: 8\r\n"
+      "Content-Type: text/plain\r\n"
+      "X-Backend-Header: get-value\r\n"
+      "\r\n"
+      "get-body",
+      client.ReadUntilEOF());
+
+  {
+    base::AutoLock auto_lock(lock_);
+    EXPECT_EQ("bar", received_custom_header);
+  }
+}
+
+TEST_F(BrowserOSServerProxyTest, PostBodyReachesBackend) {
+  std::string received_body;
+  backend_.RegisterRequestHandler(base::BindRepeating(
+      [](base::Lock* lock, std::string* received_body,
+         const net::test_server::HttpRequest& request)
+          -> std::unique_ptr<net::test_server::HttpResponse> {
+        if (request.relative_url != "/post") {
+          return nullptr;
+        }
+
+        {
+          base::AutoLock auto_lock(*lock);
+          *received_body = request.content;
+        }
+
+        auto response = std::make_unique<net::test_server::BasicHttpResponse>();
+        response->set_code(net::HTTP_OK);
+        response->set_content_type("text/plain");
+        response->set_content("post-ok");
+        return response;
+      },
+      &lock_, &received_body));
+  StartEmbeddedBackend();
+
+  constexpr char kBody[] = "field=alpha&value=beta";
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(
+      client.Write("POST /post HTTP/1.1\r\n"
+                   "Host: example.test\r\n"
+                   "Content-Type: application/x-www-form-urlencoded\r\n"
+                   "Content-Length: 22\r\n"
+                   "Connection: close\r\n"
+                   "\r\n" +
+                   std::string(kBody)));
+
+  EXPECT_THAT(client.ReadUntilEOF(), HasSubstr("post-ok"));
+  {
+    base::AutoLock auto_lock(lock_);
+    EXPECT_EQ(kBody, received_body);
+  }
+}
+
+TEST_F(BrowserOSServerProxyTest, NoBackendConfiguredReturnsServiceUnavailable) {
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(
+      client.Write("GET /missing-backend HTTP/1.1\r\n"
+                   "Host: example.test\r\n"
+                   "Connection: close\r\n"
+                   "\r\n"));
+  ASSERT_TRUE(client.ShutdownWrite());
+
+  EXPECT_EQ(kServiceUnavailableResponse, client.ReadUntilEOF());
+}
+
+TEST_F(BrowserOSServerProxyTest,
+       BackendConnectFailureReturnsServiceUnavailable) {
+  proxy_.SetBackendPort(GetUnusedLocalPort());
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(
+      client.Write("GET /connect-failure HTTP/1.1\r\n"
+                   "Host: example.test\r\n"
+                   "Connection: close\r\n"
+                   "\r\n"));
+  ASSERT_TRUE(client.ShutdownWrite());
+
+  EXPECT_EQ(kServiceUnavailableResponse, client.ReadUntilEOF());
+}
+
+TEST_F(BrowserOSServerProxyTest, StartWithZeroPortBindsEphemeralPort) {
+  EXPECT_GT(proxy_.GetPort(), 0);
+  EXPECT_EQ(0, proxy_.GetHttpsPort());
+}
+
+TEST_F(BrowserOSServerProxyTest, HttpsForwardsBytesToBackend) {
+  RestartProxyWithHttps();
+
+  ScriptedTcpBackend backend;
+  ASSERT_TRUE(backend.Start());
+  proxy_.SetBackendPort(backend.port());
+
+  TestTlsClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetHttpsPort()), IsOk());
+  ASSERT_TRUE(RunUntil([&] { return backend.is_connected(); }));
+
+  const std::string request =
+      "GET /secure HTTP/1.1\r\n"
+      "Host: secure.test\r\n"
+      "Connection: close\r\n"
+      "\r\n";
+  ASSERT_TRUE(client.Write(request));
+  ASSERT_TRUE(RunUntil(
+      [&] { return backend.received_bytes().size() >= request.size(); }));
+  EXPECT_EQ(request, backend.received_bytes().substr(0, request.size()));
+
+  const std::string response =
+      "HTTP/1.1 200 OK\r\n"
+      "Content-Type: text/plain\r\n"
+      "Content-Length: 12\r\n"
+      "Connection: close\r\n"
+      "\r\n"
+      "secure-body!";
+  ASSERT_TRUE(backend.QueueWrite(response));
+  backend.CloseAfterPendingWrites();
+
+  std::string observed_response;
+  ASSERT_TRUE(client.ReadExact(response.size(), &observed_response));
+  EXPECT_EQ(response, observed_response);
+  EXPECT_EQ("", client.ReadUntilEOF());
+}
+
+TEST_F(BrowserOSServerProxyTest, HttpsNoBackendReturnsServiceUnavailable) {
+  RestartProxyWithHttps();
+
+  TestTlsClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetHttpsPort()), IsOk());
+  ASSERT_TRUE(
+      client.Write("GET /secure-missing-backend HTTP/1.1\r\n"
+                   "Host: secure.test\r\n"
+                   "Connection: close\r\n"
+                   "\r\n"));
+
+  std::string response;
+  ASSERT_TRUE(
+      client.ReadExact(sizeof(kServiceUnavailableResponse) - 1, &response));
+  EXPECT_EQ(kServiceUnavailableResponse, response);
+  client.Close();
+}
+
+TEST_F(BrowserOSServerProxyTest, HttpsEnabledLeavesHttpListenerLive) {
+  RestartProxyWithHttps();
+
+  ScriptedTcpBackend backend;
+  ASSERT_TRUE(backend.Start());
+  proxy_.SetBackendPort(backend.port());
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(RunUntil([&] { return backend.is_connected(); }));
+
+  const std::string request =
+      "GET /plain HTTP/1.1\r\n"
+      "Host: plain.test\r\n"
+      "Connection: close\r\n"
+      "\r\n";
+  ASSERT_TRUE(client.Write(request));
+  ASSERT_TRUE(RunUntil(
+      [&] { return backend.received_bytes().size() >= request.size(); }));
+  EXPECT_EQ(request, backend.received_bytes().substr(0, request.size()));
+
+  const std::string response = ResponseWithBody("plain-body");
+  ASSERT_TRUE(backend.QueueWrite(response));
+  backend.CloseAfterPendingWrites();
+
+  std::string observed_response;
+  ASSERT_TRUE(client.ReadExact(response.size(), &observed_response));
+  EXPECT_EQ(response, observed_response);
+  EXPECT_EQ("", client.ReadUntilEOF());
+}
+
+TEST_F(BrowserOSServerProxyTest,
+       HttpsBindFailureDegradesToHttpOnlyAndHttpStillWorks) {
+  proxy_.Stop();
+  task_environment_.RunUntilIdle();
+
+  net::TCPServerSocket occupied_socket(nullptr, net::NetLogSource());
+  ASSERT_THAT(occupied_socket.ListenWithAddressAndPort("0.0.0.0", 0, 1),
+              IsOk());
+  net::IPEndPoint occupied_address;
+  ASSERT_THAT(occupied_socket.GetLocalAddress(&occupied_address), IsOk());
+
+  ASSERT_TRUE(proxy_.Start(0, occupied_address.port()));
+  proxy_.SetAllowRemote(false);
+  EXPECT_GT(proxy_.GetPort(), 0);
+  EXPECT_EQ(0, proxy_.GetHttpsPort());
+
+  ScriptedTcpBackend backend;
+  ASSERT_TRUE(backend.Start());
+  proxy_.SetBackendPort(backend.port());
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(RunUntil([&] { return backend.is_connected(); }));
+
+  const std::string request =
+      "GET /http-after-https-failure HTTP/1.1\r\n"
+      "Host: plain.test\r\n"
+      "Connection: close\r\n"
+      "\r\n";
+  ASSERT_TRUE(client.Write(request));
+  ASSERT_TRUE(RunUntil(
+      [&] { return backend.received_bytes().size() >= request.size(); }));
+  EXPECT_EQ(request, backend.received_bytes().substr(0, request.size()));
+
+  const std::string response = ResponseWithBody("http-only-body");
+  ASSERT_TRUE(backend.QueueWrite(response));
+  backend.CloseAfterPendingWrites();
+
+  std::string observed_response;
+  ASSERT_TRUE(client.ReadExact(response.size(), &observed_response));
+  EXPECT_EQ(response, observed_response);
+  EXPECT_EQ("", client.ReadUntilEOF());
+}
+
+TEST_F(BrowserOSServerProxyTest, KeepAliveHandlesTwoRequestsOnOneConnection) {
+  KeepAliveBackend backend;
+  ASSERT_TRUE(backend.Start());
+  proxy_.SetBackendPort(backend.port());
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(
+      client.Write("GET /one HTTP/1.1\r\n"
+                   "Host: keep.test\r\n"
+                   "Connection: keep-alive\r\n"
+                   "\r\n"));
+
+  std::string first_response;
+  ASSERT_TRUE(client.ReadHttpResponse(&first_response));
+  EXPECT_THAT(first_response, HasSubstr("first-body"));
+
+  ASSERT_TRUE(
+      client.Write("GET /two HTTP/1.1\r\n"
+                   "Host: keep.test\r\n"
+                   "Connection: close\r\n"
+                   "\r\n"));
+
+  std::string second_response;
+  ASSERT_TRUE(client.ReadHttpResponse(&second_response));
+  EXPECT_THAT(second_response, HasSubstr("second-body"));
+  EXPECT_EQ(2, backend.request_count());
+}
+
+TEST_F(BrowserOSServerProxyTest, StreamsPartialResponseBeforeBackendCloses) {
+  ScriptedTcpBackend backend;
+  ASSERT_TRUE(backend.Start());
+  proxy_.SetBackendPort(backend.port());
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(RunUntil([&] { return backend.is_connected(); }));
+
+  ASSERT_TRUE(
+      client.Write("GET /stream HTTP/1.1\r\n"
+                   "Host: stream.test\r\n"
+                   "Accept: text/event-stream\r\n"
+                   "\r\n"));
+  ASSERT_TRUE(RunUntil([&] {
+    return backend.received_bytes().find("\r\n\r\n") != std::string::npos;
+  }));
+
+  const std::string first =
+      "HTTP/1.1 200 OK\r\n"
+      "Content-Type: text/event-stream\r\n"
+      "\r\n"
+      "data: first\n\n";
+  ASSERT_TRUE(backend.QueueWrite(first));
+
+  std::string observed_first;
+  ASSERT_TRUE(client.ReadExact(first.size(), &observed_first));
+  EXPECT_EQ(first, observed_first);
+  EXPECT_TRUE(backend.is_connected());
+  EXPECT_EQ(1u, proxy_.GetConnectionCountForTesting());
+
+  const std::string rest = "data: second\n\n";
+  ASSERT_TRUE(backend.QueueWrite(rest));
+  backend.CloseAfterPendingWrites();
+
+  std::string observed_rest;
+  ASSERT_TRUE(client.ReadExact(rest.size(), &observed_rest));
+  EXPECT_EQ(rest, observed_rest);
+  EXPECT_EQ("", client.ReadUntilEOF());
+}
+
+TEST_F(BrowserOSServerProxyTest, LargeRequestAndResponseRoundTripByteIntact) {
+  constexpr size_t kLargeBodySize = 6 * 1024 * 1024 + 137;
+
+  ScriptedTcpBackend backend;
+  ASSERT_TRUE(backend.Start());
+  proxy_.SetBackendPort(backend.port());
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(RunUntil([&] { return backend.is_connected(); }));
+
+  const std::string body = MakePattern(kLargeBodySize);
+  const std::string request_headers =
+      "POST /large HTTP/1.1\r\n"
+      "Host: large.test\r\n"
+      "Content-Type: application/octet-stream\r\n"
+      "Content-Length: " +
+      base::NumberToString(body.size()) + "\r\n\r\n";
+  ASSERT_TRUE(client.Write(request_headers + body));
+  ASSERT_TRUE(RunUntil(
+      [&] {
+        return backend.received_bytes().size() >=
+               request_headers.size() + body.size();
+      },
+      base::Seconds(30)));
+
+  const std::string& received = backend.received_bytes();
+  ASSERT_GE(received.size(), request_headers.size() + body.size());
+  EXPECT_EQ(request_headers, received.substr(0, request_headers.size()));
+  EXPECT_TRUE(HasPattern(
+      std::string_view(received).substr(request_headers.size(), body.size())));
+
+  const std::string response_headers =
+      "HTTP/1.1 200 OK\r\n"
+      "Content-Type: application/octet-stream\r\n"
+      "Content-Length: " +
+      base::NumberToString(body.size()) +
+      "\r\n"
+      "Connection: close\r\n"
+      "\r\n";
+  ASSERT_TRUE(backend.QueueWrite(response_headers + body));
+  backend.CloseAfterPendingWrites();
+
+  std::string response;
+  ASSERT_TRUE(client.ReadExact(response_headers.size() + body.size(), &response,
+                               base::Seconds(30)));
+  ASSERT_GE(response.size(), response_headers.size());
+  EXPECT_EQ(response_headers, response.substr(0, response_headers.size()));
+  EXPECT_TRUE(HasPattern(
+      std::string_view(response).substr(response_headers.size(), body.size())));
+  EXPECT_EQ("", client.ReadUntilEOF());
+}
+
+TEST_F(BrowserOSServerProxyTest, UpgradeStyleBidirectionalBytesAreTransparent) {
+  ScriptedTcpBackend backend;
+  ASSERT_TRUE(backend.Start());
+  proxy_.SetBackendPort(backend.port());
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(RunUntil([&] { return backend.is_connected(); }));
+
+  const std::string request =
+      "GET /socket HTTP/1.1\r\n"
+      "Host: upgrade.test\r\n"
+      "Connection: Upgrade\r\n"
+      "Upgrade: websocket\r\n"
+      "\r\n";
+  ASSERT_TRUE(client.Write(request));
+  ASSERT_TRUE(RunUntil(
+      [&] { return backend.received_bytes().size() >= request.size(); }));
+  EXPECT_EQ(request, backend.received_bytes().substr(0, request.size()));
+
+  const std::string switching =
+      "HTTP/1.1 101 Switching Protocols\r\n"
+      "Connection: Upgrade\r\n"
+      "Upgrade: websocket\r\n"
+      "\r\n";
+  ASSERT_TRUE(backend.QueueWrite(switching));
+
+  std::string observed_switching;
+  ASSERT_TRUE(client.ReadExact(switching.size(), &observed_switching));
+  EXPECT_EQ(switching, observed_switching);
+
+  std::string client_payload;
+  client_payload.push_back('\0');
+  client_payload.append("client-to-backend");
+  client_payload.push_back(static_cast<char>(0xff));
+  ASSERT_TRUE(client.Write(client_payload));
+  ASSERT_TRUE(RunUntil([&] {
+    return backend.received_bytes().size() >=
+           request.size() + client_payload.size();
+  }));
+  EXPECT_EQ(client_payload, backend.received_bytes().substr(
+                                request.size(), client_payload.size()));
+
+  std::string backend_payload;
+  backend_payload.push_back(static_cast<char>(0x81));
+  backend_payload.append("backend-to-client");
+  backend_payload.push_back('\0');
+  ASSERT_TRUE(backend.QueueWrite(backend_payload));
+
+  std::string observed_backend_payload;
+  ASSERT_TRUE(
+      client.ReadExact(backend_payload.size(), &observed_backend_payload));
+  EXPECT_EQ(backend_payload, observed_backend_payload);
+}
+
+TEST_F(BrowserOSServerProxyTest, ClientDisconnectMidStreamTearsDownConnection) {
+  ScriptedTcpBackend backend;
+  ASSERT_TRUE(backend.Start());
+  proxy_.SetBackendPort(backend.port());
+
+  TestTcpClient client;
+  ASSERT_THAT(client.Connect(proxy_.GetPort()), IsOk());
+  ASSERT_TRUE(RunUntil([&] { return backend.is_connected(); }));
+
+  ASSERT_TRUE(
+      client.Write("GET /stream HTTP/1.1\r\n"
+                   "Host: stream.test\r\n"
+                   "\r\n"));
+  ASSERT_TRUE(RunUntil([&] {
+    return backend.received_bytes().find("\r\n\r\n") != std::string::npos;
+  }));
+
+  const std::string partial =
+      "HTTP/1.1 200 OK\r\n"
+      "Content-Type: text/plain\r\n"
+      "\r\n"
+      "partial";
+  ASSERT_TRUE(backend.QueueWrite(partial));
+
+  std::string observed_partial;
+  ASSERT_TRUE(client.ReadExact(partial.size(), &observed_partial));
+  EXPECT_EQ(partial, observed_partial);
+  ASSERT_EQ(1u, proxy_.GetConnectionCountForTesting());
+  ASSERT_TRUE(backend.is_connected());
+
+  client.Close();
+  EXPECT_TRUE(
+      RunUntil([&] { return proxy_.GetConnectionCountForTesting() == 0; }));
+}
+
+}  // namespace
+}  // namespace browseros
