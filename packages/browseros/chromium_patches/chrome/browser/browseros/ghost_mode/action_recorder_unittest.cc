diff --git a/chrome/browser/browseros/ghost_mode/action_recorder_unittest.cc b/chrome/browser/browseros/ghost_mode/action_recorder_unittest.cc
new file mode 100644
index 0000000000000..5e6f7a8b9c0d1
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/action_recorder_unittest.cc
@@ -0,0 +1,312 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/action_recorder.h"
+
+#include "base/files/scoped_temp_dir.h"
+#include "base/test/task_environment.h"
+#include "chrome/browser/browseros/ghost_mode/action_store.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h"
+#include "chrome/browser/browseros/ghost_mode/sensitive_detector.h"
+#include "chrome/test/base/testing_profile.h"
+#include "components/prefs/testing_pref_service.h"
+#include "content/public/test/test_web_contents_factory.h"
+#include "content/public/test/web_contents_tester.h"
+#include "testing/gtest/include/gtest/gtest.h"
+#include "url/gurl.h"
+
+namespace browseros::ghost_mode {
+
+class ActionRecorderTest : public testing::Test {
+ protected:
+  void SetUp() override {
+    ASSERT_TRUE(temp_dir_.CreateUniqueTempDir());
+    
+    // Register prefs
+    prefs::RegisterProfilePrefs(pref_service_.registry());
+    
+    // Enable ghost mode
+    pref_service_.SetBoolean(prefs::kGhostModeEnabled, true);
+    
+    // Create action store
+    action_store_ = std::make_unique<ActionStore>(
+        temp_dir_.GetPath(), &pref_service_);
+    ASSERT_TRUE(action_store_->Initialize());
+    
+    // Create web contents
+    web_contents_ = web_contents_factory_.CreateWebContents(&profile_);
+    
+    // Create recorder
+    recorder_ = std::make_unique<ActionRecorder>(
+        web_contents_, action_store_.get(), &pref_service_);
+  }
+
+  void NavigateTo(const std::string& url) {
+    content::WebContentsTester::For(web_contents_)
+        ->NavigateAndCommit(GURL(url));
+  }
+
+  base::test::TaskEnvironment task_environment_;
+  base::ScopedTempDir temp_dir_;
+  TestingPrefServiceSimple pref_service_;
+  TestingProfile profile_;
+  content::TestWebContentsFactory web_contents_factory_;
+  content::WebContents* web_contents_;
+  std::unique_ptr<ActionStore> action_store_;
+  std::unique_ptr<ActionRecorder> recorder_;
+};
+
+TEST_F(ActionRecorderTest, RecordsNavigationAction) {
+  NavigateTo("https://example.com/page");
+  
+  // Check that navigation was recorded
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  
+  ASSERT_FALSE(actions.empty());
+  EXPECT_EQ(actions[0].type, ActionType::kNavigate);
+  EXPECT_EQ(actions[0].url.spec(), "https://example.com/page");
+}
+
+TEST_F(ActionRecorderTest, RecordsClickAction) {
+  NavigateTo("https://example.com");
+  
+  // Simulate click event
+  recorder_->OnClick("#submit-button", "button", "Submit");
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  
+  bool found_click = false;
+  for (const auto& action : actions) {
+    if (action.type == ActionType::kClick) {
+      found_click = true;
+      EXPECT_FALSE(action.selectors.empty());
+      EXPECT_EQ(action.selectors[0], "#submit-button");
+      break;
+    }
+  }
+  EXPECT_TRUE(found_click);
+}
+
+TEST_F(ActionRecorderTest, RecordsTypeAction) {
+  NavigateTo("https://example.com");
+  
+  // Simulate type event (non-sensitive field)
+  recorder_->OnInput("#search-box", "input", "search", "text", "query");
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  
+  bool found_type = false;
+  for (const auto& action : actions) {
+    if (action.type == ActionType::kType) {
+      found_type = true;
+      EXPECT_FALSE(action.selectors.empty());
+      break;
+    }
+  }
+  EXPECT_TRUE(found_type);
+}
+
+TEST_F(ActionRecorderTest, DoesNotRecordWhenDisabled) {
+  pref_service_.SetBoolean(prefs::kGhostModeEnabled, false);
+  
+  NavigateTo("https://example.com");
+  recorder_->OnClick("#btn", "button", "Click");
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  EXPECT_TRUE(actions.empty());
+}
+
+TEST_F(ActionRecorderTest, DoesNotRecordSensitiveFields) {
+  NavigateTo("https://example.com");
+  
+  // Simulate typing in password field
+  recorder_->OnInput("#password", "input", "password", "password", "secret123");
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  
+  // Should not record password input
+  bool found_password_input = false;
+  for (const auto& action : actions) {
+    if (action.type == ActionType::kType && 
+        !action.selectors.empty() &&
+        action.selectors[0].find("password") != std::string::npos) {
+      found_password_input = true;
+      break;
+    }
+  }
+  EXPECT_FALSE(found_password_input);
+}
+
+TEST_F(ActionRecorderTest, DoesNotRecordExcludedDomains) {
+  // Add excluded domain
+  base::Value::List excluded;
+  excluded.Append("excluded.com");
+  pref_service_.SetList(prefs::kGhostModeExcludedDomains, std::move(excluded));
+  
+  NavigateTo("https://excluded.com/page");
+  recorder_->OnClick("#btn", "button", "Click");
+  
+  auto actions = action_store_->GetActionsForDomain("excluded.com");
+  EXPECT_TRUE(actions.empty());
+}
+
+TEST_F(ActionRecorderTest, DoesNotRecordBankingSites) {
+  NavigateTo("https://www.bankofamerica.com/login");
+  recorder_->OnClick("#btn", "button", "Login");
+  
+  auto actions = action_store_->GetActionsForDomain("bankofamerica.com");
+  
+  // Banking sites are excluded by default
+  EXPECT_TRUE(actions.empty());
+}
+
+TEST_F(ActionRecorderTest, DoesNotRecordHealthcareSites) {
+  NavigateTo("https://www.mychart.com/appointments");
+  recorder_->OnClick("#btn", "button", "Schedule");
+  
+  auto actions = action_store_->GetActionsForDomain("mychart.com");
+  
+  // Healthcare sites are excluded by default
+  EXPECT_TRUE(actions.empty());
+}
+
+TEST_F(ActionRecorderTest, DoesNotRecordIncognitoMode) {
+  // In real implementation, this would check for incognito
+  // For test, we simulate by setting the appropriate flag
+  recorder_->SetIncognitoMode(true);
+  
+  NavigateTo("https://example.com");
+  recorder_->OnClick("#btn", "button", "Click");
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  EXPECT_TRUE(actions.empty());
+}
+
+TEST_F(ActionRecorderTest, RecordsScrollAction) {
+  NavigateTo("https://example.com");
+  
+  recorder_->OnScroll(0, 500);
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  
+  bool found_scroll = false;
+  for (const auto& action : actions) {
+    if (action.type == ActionType::kScroll) {
+      found_scroll = true;
+      EXPECT_EQ(action.scroll_y, 500);
+      break;
+    }
+  }
+  EXPECT_TRUE(found_scroll);
+}
+
+TEST_F(ActionRecorderTest, RecordsSelectAction) {
+  NavigateTo("https://example.com");
+  
+  recorder_->OnSelect("#country", "select", "United States");
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  
+  bool found_select = false;
+  for (const auto& action : actions) {
+    if (action.type == ActionType::kSelect) {
+      found_select = true;
+      EXPECT_EQ(action.input_value, "United States");
+      break;
+    }
+  }
+  EXPECT_TRUE(found_select);
+}
+
+TEST_F(ActionRecorderTest, MaintainsSessionId) {
+  NavigateTo("https://example.com/page1");
+  recorder_->OnClick("#btn1", "button", "First");
+  
+  NavigateTo("https://example.com/page2");
+  recorder_->OnClick("#btn2", "button", "Second");
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  ASSERT_GE(actions.size(), 2u);
+  
+  // All actions in same session should have same session ID
+  EXPECT_EQ(actions[0].session_id, actions[1].session_id);
+}
+
+TEST_F(ActionRecorderTest, GeneratesNewSessionAfterTimeout) {
+  NavigateTo("https://example.com");
+  recorder_->OnClick("#btn1", "button", "First");
+  
+  std::string first_session = recorder_->GetCurrentSessionId();
+  
+  // Simulate session timeout (30 minutes by default)
+  recorder_->ForceNewSession();
+  
+  recorder_->OnClick("#btn2", "button", "Second");
+  
+  std::string second_session = recorder_->GetCurrentSessionId();
+  
+  EXPECT_NE(first_session, second_session);
+}
+
+// Observer tests
+class TestRecorderObserver : public ActionRecorderObserver {
+ public:
+  void OnActionRecorded(const RecordedAction& action) override {
+    recorded_actions_.push_back(action);
+  }
+  
+  void OnRecordingPaused() override { is_paused_ = true; }
+  void OnRecordingResumed() override { is_paused_ = false; }
+  
+  std::vector<RecordedAction> recorded_actions_;
+  bool is_paused_ = false;
+};
+
+TEST_F(ActionRecorderTest, NotifiesObserversOnAction) {
+  TestRecorderObserver observer;
+  recorder_->AddObserver(&observer);
+  
+  NavigateTo("https://example.com");
+  recorder_->OnClick("#btn", "button", "Click");
+  
+  EXPECT_FALSE(observer.recorded_actions_.empty());
+  
+  recorder_->RemoveObserver(&observer);
+}
+
+TEST_F(ActionRecorderTest, PauseAndResume) {
+  TestRecorderObserver observer;
+  recorder_->AddObserver(&observer);
+  
+  recorder_->Pause();
+  EXPECT_TRUE(observer.is_paused_);
+  
+  NavigateTo("https://example.com");
+  recorder_->OnClick("#btn", "button", "Click");
+  
+  // Should not record while paused
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  EXPECT_TRUE(actions.empty());
+  
+  recorder_->Resume();
+  EXPECT_FALSE(observer.is_paused_);
+  
+  recorder_->OnClick("#btn2", "button", "Click2");
+  
+  // Should record after resume
+  actions = action_store_->GetActionsForDomain("example.com");
+  EXPECT_FALSE(actions.empty());
+  
+  recorder_->RemoveObserver(&observer);
+}
+
+TEST_F(ActionRecorderTest, RecordsElementMetadata) {
+  NavigateTo("https://example.com");
+  
+  recorder_->OnClick("#submit", "button", "Submit Form");
+  
+  auto actions = action_store_->GetActionsForDomain("example.com");
+  ASSERT_FALSE(actions.empty());
+  
+  // Should capture element text for click targets
+  EXPECT_EQ(actions.back().element_text, "Submit Form");
+}
+
+}  // namespace browseros::ghost_mode
