diff --git a/chrome/browser/browseros/metrics/browseros_metrics_service.h b/chrome/browser/browseros/metrics/browseros_metrics_service.h
new file mode 100644
index 0000000000000..2157d1eb1698e
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics_service.h
@@ -0,0 +1,95 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_H_
+#define CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_H_
+
+#include <memory>
+#include <string>
+
+#include "base/functional/callback.h"
+#include "base/memory/weak_ptr.h"
+#include "base/values.h"
+#include "components/keyed_service/core/keyed_service.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+#include "url/gurl.h"
+
+class PrefService;
+
+namespace network {
+class SharedURLLoaderFactory;
+}  // namespace network
+
+namespace browseros_metrics {
+
+// Service for capturing and sending analytics events to PostHog.
+// This service manages a stable client ID (per-profile) and install ID
+// (per-installation) and sends events to the PostHog API.
+class BrowserOSMetricsService : public KeyedService {
+ public:
+  explicit BrowserOSMetricsService(
+      PrefService* pref_service,
+      PrefService* local_state_prefs,
+      scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory);
+
+  BrowserOSMetricsService(const BrowserOSMetricsService&) = delete;
+  BrowserOSMetricsService& operator=(const BrowserOSMetricsService&) = delete;
+
+  ~BrowserOSMetricsService() override;
+
+  // Captures a single event with the given name and properties.
+  // Properties should not contain PII. Common properties like client_id,
+  // browser version, and OS are added automatically.
+  void CaptureEvent(const std::string& event_name,
+                    base::Value::Dict properties);
+
+  // Returns the stable client ID for this profile.
+  std::string GetClientId() const;
+
+  // Returns the stable install ID for this browser installation.
+  std::string GetInstallId() const;
+
+  // KeyedService:
+  void Shutdown() override;
+
+ private:
+  // Initializes or retrieves the stable client ID from profile preferences.
+  void InitializeClientId();
+
+  // Initializes or retrieves the stable install ID from local state.
+  void InitializeInstallId();
+
+  // Sends the event to PostHog API.
+  void SendEventToPostHog(const std::string& event_name,
+                          base::Value::Dict properties);
+
+  // Handles the response from PostHog API.
+  void OnPostHogResponse(std::unique_ptr<network::SimpleURLLoader> loader,
+                         std::unique_ptr<std::string> response_body);
+
+  // Adds default properties to the event.
+  void AddDefaultProperties(base::Value::Dict& properties);
+
+  // PrefService for storing the stable client ID (profile prefs).
+  raw_ptr<PrefService> pref_service_;
+
+  // PrefService for storing the stable install ID (local state).
+  raw_ptr<PrefService> local_state_prefs_;
+
+  // Factory for creating URL loaders.
+  scoped_refptr<network::SharedURLLoaderFactory> url_loader_factory_;
+
+  // Stable client ID for this profile.
+  std::string client_id_;
+
+  // Stable install ID for this browser installation.
+  std::string install_id_;
+
+  // Weak pointer factory for callbacks.
+  base::WeakPtrFactory<BrowserOSMetricsService> weak_factory_{this};
+};
+
+}  // namespace browseros_metrics
+
+#endif  // CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_H_
