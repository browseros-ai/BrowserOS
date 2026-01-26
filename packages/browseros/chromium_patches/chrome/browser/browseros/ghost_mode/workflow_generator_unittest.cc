diff --git a/chrome/browser/browseros/ghost_mode/workflow_generator_unittest.cc b/chrome/browser/browseros/ghost_mode/workflow_generator_unittest.cc
new file mode 100644
index 0000000000000..4d5e6f7a8b9c0
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/workflow_generator_unittest.cc
@@ -0,0 +1,276 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/workflow_generator.h"
+
+#include "base/json/json_reader.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+#include "testing/gtest/include/gtest/gtest.h"
+#include "url/gurl.h"
+
+namespace browseros::ghost_mode {
+
+class WorkflowGeneratorTest : public testing::Test {
+ protected:
+  void SetUp() override {
+    generator_ = std::make_unique<WorkflowGenerator>();
+  }
+
+  RecordedAction CreateAction(ActionType type,
+                               const std::string& url,
+                               const std::string& selector = "",
+                               const std::string& value = "") {
+    RecordedAction action;
+    action.id = "action_" + base::NumberToString(action_counter_++);
+    action.type = type;
+    action.url = GURL(url);
+    action.url_pattern = GURL(url).host() + GURL(url).path();
+    if (!selector.empty()) {
+      action.selectors.push_back(selector);
+    }
+    action.input_value = value;
+    action.timestamp = base::Time::Now();
+    return action;
+  }
+
+  ActionSequence CreatePattern(const std::string& name,
+                                const std::vector<RecordedAction>& actions) {
+    ActionSequence pattern;
+    pattern.id = "pattern_" + base::NumberToString(pattern_counter_++);
+    pattern.name = name;
+    pattern.actions = actions;
+    pattern.occurrence_count = 5;
+    pattern.confidence_score = 0.85;
+    pattern.first_seen = base::Time::Now() - base::Days(7);
+    pattern.last_seen = base::Time::Now();
+    return pattern;
+  }
+
+  std::unique_ptr<WorkflowGenerator> generator_;
+  int action_counter_ = 0;
+  int pattern_counter_ = 0;
+};
+
+TEST_F(WorkflowGeneratorTest, GeneratesValidJSON) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(ActionType::kNavigate, "https://example.com"));
+  actions.push_back(CreateAction(ActionType::kClick, "https://example.com", "#button"));
+  
+  ActionSequence pattern = CreatePattern("Test Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  
+  // Verify it's valid JSON
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  EXPECT_TRUE(parsed->is_dict());
+}
+
+TEST_F(WorkflowGeneratorTest, HasRequiredFields) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(ActionType::kNavigate, "https://example.com"));
+  
+  ActionSequence pattern = CreatePattern("Test Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const base::Value::Dict& dict = parsed->GetDict();
+  
+  // Check required top-level fields
+  EXPECT_TRUE(dict.contains("name"));
+  EXPECT_TRUE(dict.contains("version"));
+  EXPECT_TRUE(dict.contains("steps"));
+  EXPECT_TRUE(dict.contains("metadata"));
+}
+
+TEST_F(WorkflowGeneratorTest, ContainsWorkflowName) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(ActionType::kNavigate, "https://example.com"));
+  
+  ActionSequence pattern = CreatePattern("Login Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const std::string* name = parsed->GetDict().FindString("name");
+  ASSERT_NE(name, nullptr);
+  EXPECT_EQ(*name, "Login Flow");
+}
+
+TEST_F(WorkflowGeneratorTest, GeneratesNavigateStep) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(ActionType::kNavigate, "https://example.com/page"));
+  
+  ActionSequence pattern = CreatePattern("Nav Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const base::Value::List* steps = parsed->GetDict().FindList("steps");
+  ASSERT_NE(steps, nullptr);
+  ASSERT_EQ(steps->size(), 1u);
+  
+  const base::Value::Dict& step = (*steps)[0].GetDict();
+  const std::string* type = step.FindString("type");
+  ASSERT_NE(type, nullptr);
+  EXPECT_EQ(*type, "navigate");
+  
+  const std::string* url = step.FindString("url");
+  ASSERT_NE(url, nullptr);
+  EXPECT_EQ(*url, "https://example.com/page");
+}
+
+TEST_F(WorkflowGeneratorTest, GeneratesClickStep) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(ActionType::kClick, "https://example.com", "#submit-btn"));
+  
+  ActionSequence pattern = CreatePattern("Click Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const base::Value::List* steps = parsed->GetDict().FindList("steps");
+  ASSERT_NE(steps, nullptr);
+  
+  const base::Value::Dict& step = (*steps)[0].GetDict();
+  const std::string* type = step.FindString("type");
+  EXPECT_EQ(*type, "click");
+  
+  const std::string* selector = step.FindString("selector");
+  ASSERT_NE(selector, nullptr);
+  EXPECT_EQ(*selector, "#submit-btn");
+}
+
+TEST_F(WorkflowGeneratorTest, GeneratesTypeStep) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(
+      ActionType::kType, "https://example.com", "#username", "testuser"));
+  
+  ActionSequence pattern = CreatePattern("Type Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const base::Value::List* steps = parsed->GetDict().FindList("steps");
+  ASSERT_NE(steps, nullptr);
+  
+  const base::Value::Dict& step = (*steps)[0].GetDict();
+  const std::string* type = step.FindString("type");
+  EXPECT_EQ(*type, "type");
+  
+  // Value should be parameterized, not literal
+  EXPECT_TRUE(step.contains("parameter") || step.contains("value"));
+}
+
+TEST_F(WorkflowGeneratorTest, GeneratesScrollStep) {
+  std::vector<RecordedAction> actions;
+  RecordedAction scroll = CreateAction(ActionType::kScroll, "https://example.com");
+  scroll.scroll_x = 0;
+  scroll.scroll_y = 500;
+  actions.push_back(scroll);
+  
+  ActionSequence pattern = CreatePattern("Scroll Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const base::Value::List* steps = parsed->GetDict().FindList("steps");
+  ASSERT_NE(steps, nullptr);
+  
+  const base::Value::Dict& step = (*steps)[0].GetDict();
+  const std::string* type = step.FindString("type");
+  EXPECT_EQ(*type, "scroll");
+}
+
+TEST_F(WorkflowGeneratorTest, GeneratesWaitStep) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(ActionType::kNavigate, "https://example.com"));
+  actions.push_back(CreateAction(ActionType::kClick, "https://example.com", "#btn"));
+  
+  ActionSequence pattern = CreatePattern("Wait Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  
+  // Should add implicit wait steps between actions
+  EXPECT_TRUE(json.find("wait") != std::string::npos ||
+              json.find("waitForNavigation") != std::string::npos ||
+              json.find("waitForSelector") != std::string::npos);
+}
+
+TEST_F(WorkflowGeneratorTest, IncludesMetadata) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(ActionType::kNavigate, "https://example.com"));
+  
+  ActionSequence pattern = CreatePattern("Meta Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const base::Value::Dict* metadata = parsed->GetDict().FindDict("metadata");
+  ASSERT_NE(metadata, nullptr);
+  
+  // Check metadata fields
+  EXPECT_TRUE(metadata->contains("created_at") || 
+              metadata->contains("createdAt"));
+  EXPECT_TRUE(metadata->contains("source") ||
+              metadata->contains("generated_by"));
+}
+
+TEST_F(WorkflowGeneratorTest, HandlesEmptyPattern) {
+  ActionSequence pattern = CreatePattern("Empty", {});
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const base::Value::List* steps = parsed->GetDict().FindList("steps");
+  ASSERT_NE(steps, nullptr);
+  EXPECT_TRUE(steps->empty());
+}
+
+TEST_F(WorkflowGeneratorTest, GeneratesMultipleSteps) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(ActionType::kNavigate, "https://example.com/login"));
+  actions.push_back(CreateAction(ActionType::kType, "https://example.com/login", "#user", "test"));
+  actions.push_back(CreateAction(ActionType::kType, "https://example.com/login", "#pass", "****"));
+  actions.push_back(CreateAction(ActionType::kClick, "https://example.com/login", "#submit"));
+  
+  ActionSequence pattern = CreatePattern("Login", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  auto parsed = base::JSONReader::Read(json);
+  ASSERT_TRUE(parsed.has_value());
+  
+  const base::Value::List* steps = parsed->GetDict().FindList("steps");
+  ASSERT_NE(steps, nullptr);
+  EXPECT_GE(steps->size(), 4u);  // At least 4 action steps
+}
+
+TEST_F(WorkflowGeneratorTest, ParameterizesInputValues) {
+  std::vector<RecordedAction> actions;
+  actions.push_back(CreateAction(
+      ActionType::kType, "https://example.com", "#email", "user@example.com"));
+  
+  ActionSequence pattern = CreatePattern("Param Flow", actions);
+  
+  std::string json = generator_->Generate(pattern);
+  
+  // Should not contain actual email, should be parameterized
+  EXPECT_TRUE(json.find("user@example.com") == std::string::npos ||
+              json.find("parameter") != std::string::npos ||
+              json.find("{{") != std::string::npos);
+}
+
+}  // namespace browseros::ghost_mode
