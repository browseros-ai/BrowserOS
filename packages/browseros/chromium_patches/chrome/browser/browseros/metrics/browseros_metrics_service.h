diff --git a/chrome/browser/browseros/metrics/browseros_metrics_service.h b/chrome/browser/browseros/metrics/browseros_metrics_service.h
new file mode 100644
index 0000000000000000000000000000000000000000..8d3abe12d84e95c100a48182435e075611b187ab
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics_service.h
@@ -0,0 +1,90 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_H_
+#define CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_H_
+
+#include <deque>
+#include <map>
+#include <memory>
+#include <optional>
+#include <string>
+
+#include "base/files/file_path.h"
+#include "base/memory/scoped_refptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/sequence_checker.h"
+#include "base/values.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+
+namespace network {
+class SharedURLLoaderFactory;
+}  // namespace network
+
+namespace browseros_metrics {
+
+// Owns native analytics for the entire browser process. Installation identity
+// is loaded off the UI sequence, while event ordering and network request
+// lifetime remain serialized on the UI sequence.
+class BrowserOSMetricsService {
+ public:
+  BrowserOSMetricsService(
+      browseros::Product product,
+      base::FilePath product_state_directory,
+      scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory);
+
+  BrowserOSMetricsService(const BrowserOSMetricsService&) = delete;
+  BrowserOSMetricsService& operator=(const BrowserOSMetricsService&) = delete;
+
+  ~BrowserOSMetricsService();
+
+  // Captures an event on the UI sequence. Events received while the
+  // installation UUID is loading are held in a bounded FIFO.
+  void CaptureEvent(std::string event_name, base::DictValue properties);
+
+  // Cancels identity callbacks and in-flight uploads. This is idempotent so
+  // the owning browser-main extra part can enforce cleanup on every exit path.
+  void Shutdown();
+
+ private:
+  struct PendingEvent {
+    std::string event_name;
+    base::DictValue properties;
+  };
+
+  void OnInstallationIdReady(std::optional<std::string> install_id);
+  void SendEventToPostHog(std::string event_name, base::DictValue properties);
+  void OnPostHogResponse(network::SimpleURLLoader* loader,
+                         std::optional<std::string> response_body);
+  void AddDefaultProperties(base::DictValue& properties) const;
+
+  const browseros::Product product_;
+  const scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory_;
+
+  std::string install_id_;
+  bool installation_load_complete_ = false;
+  bool shutdown_ = false;
+
+  // Identity initialization is asynchronous because installation.json lives
+  // outside the profile and requires blocking filesystem I/O.
+  std::deque<PendingEvent> pending_events_;
+
+  // SimpleURLLoader does not own itself. Keeping loaders here makes shutdown
+  // cancellation explicit and prevents callbacks from outliving the service.
+  std::map<network::SimpleURLLoader*, std::unique_ptr<network::SimpleURLLoader>>
+      active_loaders_;
+
+  SEQUENCE_CHECKER(sequence_checker_);
+  base::WeakPtrFactory<BrowserOSMetricsService> weak_factory_{this};
+};
+
+// The browser-main extra part owns this pointer. Access is restricted to the
+// UI sequence so callers cannot race process startup or teardown.
+BrowserOSMetricsService* GetBrowserOSMetricsServiceForProcess();
+void SetBrowserOSMetricsServiceForProcess(BrowserOSMetricsService* service);
+
+}  // namespace browseros_metrics
+
+#endif  // CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_H_
