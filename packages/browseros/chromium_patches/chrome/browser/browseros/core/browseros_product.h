diff --git a/chrome/browser/browseros/core/browseros_product.h b/chrome/browser/browseros/core/browseros_product.h
new file mode 100644
index 0000000000000..e35cbdaf72a25
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_product.h
@@ -0,0 +1,89 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_PRODUCT_H_
+#define CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_PRODUCT_H_
+
+#include <optional>
+#include <string>
+#include <string_view>
+
+#include "base/command_line.h"
+#include "base/logging.h"
+#include "chrome/browser/browseros/buildflags.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+
+namespace browseros {
+
+enum class Product {
+  kBrowserOS,
+  kBrowserClaw,
+};
+
+static_assert(BUILDFLAG(BROWSEROS_PRODUCT_BROWSEROS) !=
+                  BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW),
+              "Exactly one BrowserOS product must be selected");
+
+inline Product GetBakedProduct() {
+#if BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)
+  return Product::kBrowserClaw;
+#else
+  return Product::kBrowserOS;
+#endif
+}
+
+#if BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+inline constexpr char kBrowserOSProductValue[] = "browseros";
+inline constexpr char kBrowserClawProductValue[] = "browserclaw";
+
+inline std::optional<Product> ProductFromSwitchValue(std::string_view value) {
+  if (value == kBrowserOSProductValue) {
+    return Product::kBrowserOS;
+  }
+  if (value == kBrowserClawProductValue) {
+    return Product::kBrowserClaw;
+  }
+  return std::nullopt;
+}
+#endif
+
+inline Product GetProduct() {
+  const Product baked_product = GetBakedProduct();
+
+#if BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+  if (!base::CommandLine::InitializedForCurrentProcess()) {
+    return baked_product;
+  }
+
+  const base::CommandLine* command_line =
+      base::CommandLine::ForCurrentProcess();
+  if (!command_line->HasSwitch(kBrowserOSProduct)) {
+    return baked_product;
+  }
+
+  const std::string value =
+      command_line->GetSwitchValueASCII(kBrowserOSProduct);
+  std::optional<Product> product = ProductFromSwitchValue(value);
+  if (product.has_value()) {
+    return *product;
+  }
+
+  LOG(WARNING) << "browseros: Ignoring invalid --" << kBrowserOSProduct << "="
+               << value;
+#endif
+
+  return baked_product;
+}
+
+inline bool IsBrowserOSProduct() {
+  return GetProduct() == Product::kBrowserOS;
+}
+
+inline bool IsBrowserClawProduct() {
+  return GetProduct() == Product::kBrowserClaw;
+}
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_PRODUCT_H_
