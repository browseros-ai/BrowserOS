diff --git a/chrome/browser/browseros/ghost_mode/sensitive_detector.cc b/chrome/browser/browseros/ghost_mode/sensitive_detector.cc
new file mode 100644
index 0000000000000..6e7f8a9b0c1d2
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/sensitive_detector.cc
@@ -0,0 +1,186 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/sensitive_detector.h"
+
+#include <algorithm>
+
+#include "base/no_destructor.h"
+#include "base/strings/string_util.h"
+
+namespace browseros::ghost_mode {
+
+// Input types that are ALWAYS sensitive - never record values
+const std::vector<std::string> SensitiveDetector::kSensitiveInputTypes = {
+    "password",
+    "hidden",  // Often contains tokens
+};
+
+// Name/ID patterns that indicate sensitivity (case-insensitive)
+const std::vector<std::string> SensitiveDetector::kSensitiveNamePatterns = {
+    // Authentication
+    "password",
+    "passwd",
+    "pwd",
+    "pass",
+    "secret",
+    "token",
+    "apikey",
+    "api_key",
+    "api-key",
+    "auth",
+    "credential",
+    "otp",
+    "2fa",
+    "mfa",
+    "totp",
+    "pin",
+    "verification",
+    "security_code",
+    "security-code",
+    "securitycode",
+    
+    // Financial
+    "ssn",
+    "social_security",
+    "social-security",
+    "socialsecurity",
+    "tax_id",
+    "taxid",
+    "ein",
+    "routing",
+    "account_number",
+    "accountnumber",
+    "account-number",
+    "bank_account",
+    
+    // Credit card
+    "card_number",
+    "cardnumber",
+    "card-number",
+    "ccnum",
+    "cc_num",
+    "cc-num",
+    "cvc",
+    "cvv",
+    "csc",
+    "expiry",
+    "exp_date",
+    "expiration",
+    
+    // Personal identifiers
+    "dob",
+    "date_of_birth",
+    "dateofbirth",
+    "birthdate",
+    "passport",
+    "license_number",
+    "driver_license",
+};
+
+// Autocomplete values that indicate sensitive fields
+const std::vector<std::string> SensitiveDetector::kSensitiveAutocompleteValues = {
+    "current-password",
+    "new-password",
+    "one-time-code",
+    "cc-number",
+    "cc-csc",
+    "cc-exp",
+    "cc-exp-month",
+    "cc-exp-year",
+    "cc-type",
+    "transaction-amount",
+    "bday",
+    "bday-day",
+    "bday-month",
+    "bday-year",
+};
+
+// CSS selector patterns that indicate sensitive forms/areas
+const std::vector<std::string> SensitiveDetector::kSensitiveSelectorPatterns = {
+    "login",
+    "signin",
+    "sign-in",
+    "sign_in",
+    "signup",
+    "sign-up",
+    "sign_up",
+    "password",
+    "auth",
+    "payment",
+    "checkout",
+    "billing",
+    "credit-card",
+    "creditcard",
+};
+
+// URL patterns that indicate sensitive pages
+const std::vector<std::string> SensitiveDetector::kSensitiveUrlPatterns = {
+    "/login",
+    "/signin",
+    "/sign-in",
+    "/signup",
+    "/sign-up",
+    "/auth",
+    "/oauth",
+    "/password",
+    "/reset-password",
+    "/forgot-password",
+    "/payment",
+    "/checkout",
+    "/billing",
+    "/account/security",
+    "/settings/security",
+    "/2fa",
+    "/mfa",
+};
+
+// Label patterns that suggest sensitivity
+const std::vector<std::string> SensitiveDetector::kSensitiveLabelPatterns = {
+    "password",
+    "secret",
+    "pin",
+    "security code",
+    "verification code",
+    "card number",
+    "cvv",
+    "cvc",
+    "expiration",
+    "social security",
+    "ssn",
+};
+
+SensitiveDetector::SensitiveDetector() = default;
+SensitiveDetector::~SensitiveDetector() = default;
+
+bool SensitiveDetector::ContainsAnyPattern(
+    const std::string& str,
+    const std::vector<std::string>& patterns) const {
+  std::string lower_str = base::ToLowerASCII(str);
+  for (const auto& pattern : patterns) {
+    if (lower_str.find(pattern) != std::string::npos) {
+      return true;
+    }
+  }
+  return false;
+}
+
+bool SensitiveDetector::IsSensitiveField(
+    const std::string& input_type,
+    const std::string& name,
+    const std::string& id,
+    const std::string& autocomplete,
+    const std::string& aria_label,
+    const std::string& placeholder) const {
+  
+  // Check input type first (always sensitive types)
+  std::string lower_type = base::ToLowerASCII(input_type);
+  for (const auto& sensitive_type : kSensitiveInputTypes) {
+    if (lower_type == sensitive_type) {
+      return true;
+    }
+  }
+  
+  // Check autocomplete attribute
+  if (ContainsAnyPattern(autocomplete, kSensitiveAutocompleteValues)) {
+    return true;
+  }
+  
+  // Check name attribute
+  if (ContainsAnyPattern(name, kSensitiveNamePatterns)) {
+    return true;
+  }
+  
+  // Check ID attribute
+  if (ContainsAnyPattern(id, kSensitiveNamePatterns)) {
+    return true;
+  }
+  
+  // Check aria-label
+  if (ContainsAnyPattern(aria_label, kSensitiveLabelPatterns)) {
+    return true;
+  }
+  
+  // Check placeholder
+  if (ContainsAnyPattern(placeholder, kSensitiveLabelPatterns)) {
+    return true;
+  }
+  
+  return false;
+}
+
+bool SensitiveDetector::IsSensitiveSelector(const std::string& selector) const {
+  return ContainsAnyPattern(selector, kSensitiveSelectorPatterns);
+}
+
+bool SensitiveDetector::IsSensitiveUrl(const std::string& url) const {
+  return ContainsAnyPattern(url, kSensitiveUrlPatterns);
+}
+
+bool SensitiveDetector::IsSensitiveLabel(const std::string& label) const {
+  return ContainsAnyPattern(label, kSensitiveLabelPatterns);
+}
+
+SensitiveDetector& GetSensitiveDetector() {
+  static base::NoDestructor<SensitiveDetector> instance;
+  return *instance;
+}
+
+bool ShouldSkipRecording(const std::string& input_type,
+                         const std::string& name,
+                         const std::string& id,
+                         const std::string& autocomplete,
+                         const std::string& aria_label,
+                         const std::string& placeholder,
+                         const std::string& selector,
+                         const std::string& url) {
+  const auto& detector = GetSensitiveDetector();
+  
+  // Check field attributes
+  if (detector.IsSensitiveField(input_type, name, id, autocomplete,
+                                 aria_label, placeholder)) {
+    return true;
+  }
+  
+  // Check selector
+  if (detector.IsSensitiveSelector(selector)) {
+    return true;
+  }
+  
+  // Check URL
+  if (detector.IsSensitiveUrl(url)) {
+    return true;
+  }
+  
+  return false;
+}
+
+}  // namespace browseros::ghost_mode
