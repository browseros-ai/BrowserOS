diff --git a/chrome/browser/browseros/metrics/browseros_metrics_service.cc b/chrome/browser/browseros/metrics/browseros_metrics_service.cc
new file mode 100644
index 0000000000000000000000000000000000000000..97180c8484b889a9eb8a77b832300cb8cf8b0543
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics_service.cc
@@ -0,0 +1,274 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/metrics/browseros_metrics_service.h"
+
+#include <cstddef>
+#include <memory>
+#include <optional>
+#include <string>
+#include <string_view>
+#include <utility>
+
+#include "base/check.h"
+#include "base/functional/bind.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/system/sys_info.h"
+#include "base/task/task_traits.h"
+#include "base/task/thread_pool.h"
+#include "chrome/browser/browseros/metrics/browseros_installation_id.h"
+#include "components/version_info/version_info.h"
+#include "content/public/browser/browser_thread.h"
+#include "net/base/load_flags.h"
+#include "net/http/http_status_code.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/shared_url_loader_factory.h"
+#include "services/network/public/mojom/url_response_head.mojom.h"
+#include "url/gurl.h"
+
+namespace browseros_metrics {
+namespace {
+
+constexpr std::string_view kBrowserOSPostHogApiKey =
+    "phc_PRrpVnBMVJgUumvaXzUnwKZ1dDs3L8MSICLhTdnc8jC";
+constexpr std::string_view kBrowserClawPostHogApiKey =
+    "phc_mafRaZD4djbUnfNzKx9iKWyzW583UAbmZGepvxDiPcZx";
+constexpr char kPostHogEndpoint[] = "https://us.i.posthog.com/i/v0/e/";
+constexpr size_t kMaxUploadSize = 256 * 1024;
+constexpr size_t kMaxPendingEvents = 256;
+
+BrowserOSMetricsService* g_process_metrics_service = nullptr;
+
+constexpr std::string_view GetPostHogApiKey(browseros::Product product) {
+  return product == browseros::Product::kBrowserClaw ? kBrowserClawPostHogApiKey
+                                                     : kBrowserOSPostHogApiKey;
+}
+
+constexpr std::string_view GetProductName(browseros::Product product) {
+  return product == browseros::Product::kBrowserClaw ? "browserclaw"
+                                                     : "browseros";
+}
+
+constexpr std::string_view GetEventPrefix(browseros::Product product) {
+  return product == browseros::Product::kBrowserClaw ? "browserclaw.native."
+                                                     : "browseros.native.";
+}
+
+constexpr net::NetworkTrafficAnnotationTag kBrowserOSMetricsTrafficAnnotation =
+    net::DefineNetworkTrafficAnnotation("browseros_metrics", R"(
+        semantics {
+          sender: "BrowserOS Metrics"
+          description:
+            "Sends anonymous product usage metrics to PostHog. This helps "
+            "improve BrowserOS and BrowserClaw by showing how features are "
+            "used. No personally identifiable information is collected."
+          trigger:
+            "Triggered when product features are used, such as extension "
+            "actions or settings changes."
+          data:
+            "Event name, timestamp, anonymous installation ID, product, "
+            "browser version, OS information, and feature-specific "
+            "properties without PII."
+          destination: OTHER
+          destination_other:
+            "PostHog analytics service at us.i.posthog.com"
+        }
+        policy {
+          cookies_allowed: NO
+          setting:
+            "This feature cannot be disabled through settings. Events are "
+            "sent anonymously without user identification."
+          policy_exception_justification:
+            "Not implemented. Analytics are anonymous and help improve "
+            "the browser experience."
+        })");
+
+}  // namespace
+
+BrowserOSMetricsService::BrowserOSMetricsService(
+    browseros::Product product,
+    base::FilePath product_state_directory,
+    scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory)
+    : product_(product), url_loader_factory_(std::move(url_loader_factory)) {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  CHECK(url_loader_factory_);
+
+  // installation.json is shared with sidecars, so filesystem access cannot
+  // block the UI sequence. The bounded queue below preserves startup events
+  // until this reply returns.
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_VISIBLE},
+      base::BindOnce(&LoadOrCreateInstallationId,
+                     std::move(product_state_directory)),
+      base::BindOnce(&BrowserOSMetricsService::OnInstallationIdReady,
+                     weak_factory_.GetWeakPtr()));
+}
+
+BrowserOSMetricsService::~BrowserOSMetricsService() {
+  Shutdown();
+}
+
+void BrowserOSMetricsService::CaptureEvent(std::string event_name,
+                                           base::DictValue properties) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  if (shutdown_) {
+    return;
+  }
+  if (event_name.empty()) {
+    LOG(WARNING) << "browseros: Attempted to capture event with empty name";
+    return;
+  }
+
+  if (!installation_load_complete_) {
+    if (pending_events_.size() >= kMaxPendingEvents) {
+      LOG(WARNING) << "browseros: Metrics startup queue is full; dropping "
+                   << event_name;
+      return;
+    }
+    pending_events_.push_back(
+        PendingEvent{std::move(event_name), std::move(properties)});
+    return;
+  }
+
+  if (install_id_.empty()) {
+    VLOG(1) << "browseros: Metrics disabled because installation identity "
+               "could not be loaded";
+    return;
+  }
+
+  SendEventToPostHog(std::move(event_name), std::move(properties));
+}
+
+void BrowserOSMetricsService::Shutdown() {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  if (shutdown_) {
+    return;
+  }
+  shutdown_ = true;
+
+  // Invalidate replies before releasing their state, then destroy every
+  // loader to cancel network callbacks at the process-lifecycle boundary.
+  weak_factory_.InvalidateWeakPtrs();
+  active_loaders_.clear();
+  pending_events_.clear();
+}
+
+void BrowserOSMetricsService::OnInstallationIdReady(
+    std::optional<std::string> install_id) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  if (shutdown_) {
+    return;
+  }
+
+  installation_load_complete_ = true;
+  if (!install_id.has_value()) {
+    LOG(ERROR) << "browseros: Native metrics disabled because installation "
+                  "identity is unavailable";
+    pending_events_.clear();
+    return;
+  }
+  install_id_ = std::move(*install_id);
+
+  // Move the queue aside so CaptureEvent() cannot invalidate iteration if a
+  // future delivery path becomes re-entrant.
+  std::deque<PendingEvent> pending_events = std::move(pending_events_);
+  for (PendingEvent& event : pending_events) {
+    SendEventToPostHog(std::move(event.event_name),
+                       std::move(event.properties));
+  }
+}
+
+void BrowserOSMetricsService::SendEventToPostHog(std::string event_name,
+                                                 base::DictValue properties) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  AddDefaultProperties(properties);
+
+  base::DictValue payload;
+  payload.Set("api_key", std::string(GetPostHogApiKey(product_)));
+  payload.Set("event", std::string(GetEventPrefix(product_)) + event_name);
+  payload.Set("distinct_id", install_id_);
+  payload.Set("properties", std::move(properties));
+
+  std::string json_payload;
+  if (!base::JSONWriter::Write(payload, &json_payload)) {
+    LOG(ERROR) << "browseros: Failed to serialize metrics payload";
+    return;
+  }
+
+  auto resource_request = std::make_unique<network::ResourceRequest>();
+  resource_request->url = GURL(kPostHogEndpoint);
+  resource_request->method = "POST";
+  resource_request->load_flags = net::LOAD_DISABLE_CACHE;
+  resource_request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+
+  auto loader = network::SimpleURLLoader::Create(
+      std::move(resource_request), kBrowserOSMetricsTrafficAnnotation);
+  loader->SetAllowHttpErrorResults(true);
+  loader->AttachStringForUpload(json_payload, "application/json");
+
+  network::SimpleURLLoader* loader_ptr = loader.get();
+  const bool inserted =
+      active_loaders_.emplace(loader_ptr, std::move(loader)).second;
+  CHECK(inserted);
+  loader_ptr->DownloadToString(
+      url_loader_factory_.get(),
+      base::BindOnce(&BrowserOSMetricsService::OnPostHogResponse,
+                     weak_factory_.GetWeakPtr(), loader_ptr),
+      kMaxUploadSize);
+}
+
+void BrowserOSMetricsService::OnPostHogResponse(
+    network::SimpleURLLoader* loader,
+    std::optional<std::string> response_body) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  auto loader_it = active_loaders_.find(loader);
+  if (loader_it == active_loaders_.end()) {
+    return;
+  }
+
+  int response_code = 0;
+  if (loader->ResponseInfo() && loader->ResponseInfo()->headers) {
+    response_code = loader->ResponseInfo()->headers->response_code();
+  }
+
+  if (response_code == net::HTTP_OK) {
+    VLOG(2) << "browseros: Metrics event sent successfully";
+  } else {
+    LOG(WARNING) << "browseros: Failed to send metrics event. Response code: "
+                 << response_code;
+    if (response_body.has_value() && !response_body->empty()) {
+      LOG(WARNING) << "browseros: Error response: " << *response_body;
+    }
+  }
+
+  active_loaders_.erase(loader_it);
+}
+
+void BrowserOSMetricsService::AddDefaultProperties(
+    base::DictValue& properties) const {
+  properties.Set("$browser_version", version_info::GetVersionNumber());
+  properties.Set("$os", base::SysInfo::OperatingSystemName());
+  properties.Set("$os_version", base::SysInfo::OperatingSystemVersion());
+  properties.Set("$process_person_profile", false);
+  properties.Set("$arch", base::SysInfo::OperatingSystemArchitecture());
+  properties.Set("install_id", install_id_);
+  properties.Set("product", std::string(GetProductName(product_)));
+  properties.Set("surface", "native");
+}
+
+BrowserOSMetricsService* GetBrowserOSMetricsServiceForProcess() {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  return g_process_metrics_service;
+}
+
+void SetBrowserOSMetricsServiceForProcess(BrowserOSMetricsService* service) {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  DCHECK(!service || !g_process_metrics_service ||
+         service == g_process_metrics_service);
+  g_process_metrics_service = service;
+}
+
+}  // namespace browseros_metrics
