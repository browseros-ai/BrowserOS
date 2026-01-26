diff --git a/chrome/browser/browseros/ghost_mode/sensitive_detector_unittest.cc b/chrome/browser/browseros/ghost_mode/sensitive_detector_unittest.cc
new file mode 100644
index 0000000000000..1a2b3c4d5e6f7
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/sensitive_detector_unittest.cc
@@ -0,0 +1,186 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/sensitive_detector.h"
+
+#include "testing/gtest/include/gtest/gtest.h"
+
+namespace browseros::ghost_mode {
+
+class SensitiveDetectorTest : public testing::Test {
+ protected:
+  void SetUp() override {
+    detector_ = std::make_unique<SensitiveDetector>();
+  }
+
+  std::unique_ptr<SensitiveDetector> detector_;
+};
+
+// Test password field detection
+TEST_F(SensitiveDetectorTest, DetectsPasswordInputType) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "password", "", "", "", "", ""));
+}
+
+TEST_F(SensitiveDetectorTest, DetectsPasswordByName) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "password", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "user_password", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "passwd", "", "", "", ""));
+}
+
+TEST_F(SensitiveDetectorTest, DetectsPasswordById) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "", "login-password", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "", "pwd-input", "", "", ""));
+}
+
+// Test credit card detection
+TEST_F(SensitiveDetectorTest, DetectsCreditCardByAutocomplete) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "", "", "cc-number", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "", "", "cc-csc", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "", "", "cc-exp", "", ""));
+}
+
+TEST_F(SensitiveDetectorTest, DetectsCreditCardByName) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "credit-card", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "cardNumber", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "cvv", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "cvc", "", "", "", ""));
+}
+
+// Test SSN detection
+TEST_F(SensitiveDetectorTest, DetectsSSN) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "ssn", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "social-security", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "", "taxId", "", "", ""));
+}
+
+// Test other sensitive fields
+TEST_F(SensitiveDetectorTest, DetectsPIN) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "pin", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "security-pin", "", "", "", ""));
+}
+
+TEST_F(SensitiveDetectorTest, DetectsBankAccount) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "routing-number", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "account-number", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "", "", "", "", "Enter your bank account"));
+}
+
+// Test non-sensitive fields
+TEST_F(SensitiveDetectorTest, AllowsRegularTextFields) {
+  EXPECT_FALSE(detector_->IsSensitiveField(
+      "text", "username", "", "", "", ""));
+  EXPECT_FALSE(detector_->IsSensitiveField(
+      "text", "email", "", "", "", ""));
+  EXPECT_FALSE(detector_->IsSensitiveField(
+      "text", "search", "", "", "", ""));
+  EXPECT_FALSE(detector_->IsSensitiveField(
+      "text", "firstName", "", "", "", ""));
+}
+
+TEST_F(SensitiveDetectorTest, AllowsSearchFields) {
+  EXPECT_FALSE(detector_->IsSensitiveField(
+      "search", "q", "", "", "", "Search..."));
+}
+
+// Test URL sensitivity
+TEST_F(SensitiveDetectorTest, DetectsSensitiveBankingUrls) {
+  EXPECT_TRUE(detector_->IsSensitiveUrl(
+      "https://www.chase.com/login"));
+  EXPECT_TRUE(detector_->IsSensitiveUrl(
+      "https://banking.example.com/accounts"));
+  EXPECT_TRUE(detector_->IsSensitiveUrl(
+      "https://example.com/payment/checkout"));
+}
+
+TEST_F(SensitiveDetectorTest, DetectsSensitiveHealthcareUrls) {
+  EXPECT_TRUE(detector_->IsSensitiveUrl(
+      "https://mychart.example.com/portal"));
+  EXPECT_TRUE(detector_->IsSensitiveUrl(
+      "https://example.com/health/records"));
+}
+
+TEST_F(SensitiveDetectorTest, AllowsRegularUrls) {
+  EXPECT_FALSE(detector_->IsSensitiveUrl(
+      "https://www.google.com/search"));
+  EXPECT_FALSE(detector_->IsSensitiveUrl(
+      "https://news.example.com/article"));
+  EXPECT_FALSE(detector_->IsSensitiveUrl(
+      "https://github.com/user/repo"));
+}
+
+// Test selector sensitivity
+TEST_F(SensitiveDetectorTest, DetectsSensitiveSelectors) {
+  EXPECT_TRUE(detector_->IsSensitiveSelector(
+      "#password-input"));
+  EXPECT_TRUE(detector_->IsSensitiveSelector(
+      ".login-form input[type='password']"));
+  EXPECT_TRUE(detector_->IsSensitiveSelector(
+      "[data-testid='credit-card-field']"));
+}
+
+TEST_F(SensitiveDetectorTest, AllowsRegularSelectors) {
+  EXPECT_FALSE(detector_->IsSensitiveSelector(
+      "#search-input"));
+  EXPECT_FALSE(detector_->IsSensitiveSelector(
+      ".nav-menu .menu-item"));
+  EXPECT_FALSE(detector_->IsSensitiveSelector(
+      "[data-testid='submit-button']"));
+}
+
+// Test label sensitivity
+TEST_F(SensitiveDetectorTest, DetectsSensitiveLabels) {
+  EXPECT_TRUE(detector_->IsSensitiveLabel("Enter your password"));
+  EXPECT_TRUE(detector_->IsSensitiveLabel("Credit Card Number"));
+  EXPECT_TRUE(detector_->IsSensitiveLabel("Social Security Number"));
+  EXPECT_TRUE(detector_->IsSensitiveLabel("CVV/CVC"));
+}
+
+TEST_F(SensitiveDetectorTest, AllowsRegularLabels) {
+  EXPECT_FALSE(detector_->IsSensitiveLabel("First Name"));
+  EXPECT_FALSE(detector_->IsSensitiveLabel("Email Address"));
+  EXPECT_FALSE(detector_->IsSensitiveLabel("Submit"));
+}
+
+// Test convenience function
+TEST_F(SensitiveDetectorTest, ShouldSkipRecordingIntegration) {
+  // Should skip password
+  EXPECT_TRUE(ShouldSkipRecording(
+      "password", "", "", "", "", "", "", "https://example.com"));
+  
+  // Should skip credit card on any URL
+  EXPECT_TRUE(ShouldSkipRecording(
+      "text", "cc-number", "", "", "", "", "", "https://shop.example.com"));
+  
+  // Should allow regular search
+  EXPECT_FALSE(ShouldSkipRecording(
+      "text", "search", "", "", "", "", "#search-box", "https://google.com"));
+}
+
+// Test case insensitivity
+TEST_F(SensitiveDetectorTest, IsCaseInsensitive) {
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "PASSWORD", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "CreditCard", "", "", "", ""));
+  EXPECT_TRUE(detector_->IsSensitiveField(
+      "text", "", "SSN_INPUT", "", "", ""));
+}
+
+}  // namespace browseros::ghost_mode
