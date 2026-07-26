diff --git a/chrome/browser/browseros/onboarding/browseros_onboarding.cc b/chrome/browser/browseros/onboarding/browseros_onboarding.cc
new file mode 100644
index 0000000000000..372f36bcb4864f440e38a461e9cf4ab982b191dd
--- /dev/null
+++ b/chrome/browser/browseros/onboarding/browseros_onboarding.cc
@@ -0,0 +1,605 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/onboarding/browseros_onboarding.h"
+
+#include <stdint.h>
+
+#include <memory>
+#include <optional>
+#include <string>
+#include <string_view>
+#include <utility>
+
+#include "base/functional/bind.h"
+#include "base/functional/callback.h"
+#include "base/location.h"
+#include "base/notreached.h"
+#include "base/strings/stringprintf.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/task/sequenced_task_runner.h"
+#include "base/values.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/importer/external_process_importer_host.h"
+#include "chrome/browser/importer/importer_list.h"
+#include "chrome/browser/importer/importer_progress_observer.h"
+#include "chrome/browser/importer/profile_writer.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/common/webui_url_constants.h"
+#include "chrome/grit/browseros_onboarding_resources.h"
+#include "chrome/grit/browseros_onboarding_resources_map.h"
+#include "components/user_data_importer/common/importer_data_types.h"
+#include "content/public/browser/visibility.h"
+#include "content/public/browser/web_contents.h"
+#include "content/public/browser/web_ui.h"
+#include "content/public/browser/web_ui_data_source.h"
+#include "content/public/browser/web_ui_message_handler.h"
+#include "content/public/common/url_constants.h"
+#include "ui/webui/webui_util.h"
+
+namespace {
+
+constexpr int kBrowserOSOnboardingApiVersion = 1;
+constexpr uint16_t kBrowserOSImportableItems =
+    user_data_importer::HISTORY | user_data_importer::FAVORITES |
+    user_data_importer::COOKIES | user_data_importer::PASSWORDS |
+    user_data_importer::SEARCH_ENGINES |
+    user_data_importer::AUTOFILL_FORM_DATA | user_data_importer::EXTENSIONS;
+
+std::string SourceIdForIndex(size_t index) {
+  return base::StringPrintf("source-%zu", index);
+}
+
+const char* ImportItemToString(user_data_importer::ImportItem item) {
+  switch (item) {
+    case user_data_importer::HISTORY:
+      return "history";
+    case user_data_importer::FAVORITES:
+      return "bookmarks";
+    case user_data_importer::COOKIES:
+      return "cookies";
+    case user_data_importer::PASSWORDS:
+      return "passwords";
+    case user_data_importer::SEARCH_ENGINES:
+      return "searchEngines";
+    case user_data_importer::AUTOFILL_FORM_DATA:
+      return "autofill";
+    case user_data_importer::EXTENSIONS:
+      return "extensions";
+    case user_data_importer::NONE:
+    case user_data_importer::HOME_PAGE:
+    case user_data_importer::ALL:
+      return nullptr;
+  }
+}
+
+uint16_t ImportItemMaskFromString(std::string_view item) {
+  if (item == "history") {
+    return user_data_importer::HISTORY;
+  }
+  if (item == "bookmarks") {
+    return user_data_importer::FAVORITES;
+  }
+  if (item == "cookies") {
+    return user_data_importer::COOKIES;
+  }
+  if (item == "passwords") {
+    return user_data_importer::PASSWORDS;
+  }
+  if (item == "searchEngines") {
+    return user_data_importer::SEARCH_ENGINES;
+  }
+  if (item == "autofill") {
+    return user_data_importer::AUTOFILL_FORM_DATA;
+  }
+  if (item == "extensions") {
+    return user_data_importer::EXTENSIONS;
+  }
+  return user_data_importer::NONE;
+}
+
+void AppendImportItem(base::ListValue& items,
+                      uint16_t services,
+                      user_data_importer::ImportItem item) {
+  if ((services & item) == 0) {
+    return;
+  }
+
+  const char* name = ImportItemToString(item);
+  if (name) {
+    items.Append(name);
+  }
+}
+
+base::ListValue ImportItemsFromMask(uint16_t services) {
+  base::ListValue items;
+  AppendImportItem(items, services, user_data_importer::HISTORY);
+  AppendImportItem(items, services, user_data_importer::FAVORITES);
+  AppendImportItem(items, services, user_data_importer::COOKIES);
+  AppendImportItem(items, services, user_data_importer::PASSWORDS);
+  AppendImportItem(items, services, user_data_importer::SEARCH_ENGINES);
+  AppendImportItem(items, services, user_data_importer::AUTOFILL_FORM_DATA);
+  AppendImportItem(items, services, user_data_importer::EXTENSIONS);
+  return items;
+}
+
+}  // namespace
+
+class BrowserOSOnboardingHandler : public content::WebUIMessageHandler,
+                                   public importer::ImporterProgressObserver {
+ public:
+  BrowserOSOnboardingHandler() = default;
+  BrowserOSOnboardingHandler(const BrowserOSOnboardingHandler&) = delete;
+  BrowserOSOnboardingHandler& operator=(const BrowserOSOnboardingHandler&) =
+      delete;
+  ~BrowserOSOnboardingHandler() override {
+    if (importer_host_) {
+      importer_host_->set_observer(nullptr);
+    }
+  }
+
+  void SetCompletionCallback(base::RepeatingClosure completion_callback) {
+    completion_callback_ = std::move(completion_callback);
+  }
+
+ private:
+  enum class ImportSourceResultStatus {
+    kImporting,
+    kSucceeded,
+    kFailed,
+  };
+
+  struct ImportRequestSelection {
+    int source_index = 0;
+    std::string source_id;
+    uint16_t selected_items = user_data_importer::NONE;
+    bool has_selected_items = false;
+  };
+
+  struct ImportSourceResult {
+    std::string source_id;
+    std::string display_name;
+    ImportSourceResultStatus status;
+  };
+
+  void RegisterMessages() override {
+    web_ui()->RegisterMessageCallback(
+        "browserosOnboardingPageReady",
+        base::BindRepeating(&BrowserOSOnboardingHandler::HandlePageReady,
+                            base::Unretained(this)));
+    web_ui()->RegisterMessageCallback(
+        "browserosOnboardingRefreshSources",
+        base::BindRepeating(&BrowserOSOnboardingHandler::HandleRefreshSources,
+                            base::Unretained(this)));
+    web_ui()->RegisterMessageCallback(
+        "browserosOnboardingStartImport",
+        base::BindRepeating(&BrowserOSOnboardingHandler::HandleStartImport,
+                            base::Unretained(this)));
+    web_ui()->RegisterMessageCallback(
+        "browserosOnboardingComplete",
+        base::BindRepeating(&BrowserOSOnboardingHandler::HandleComplete,
+                            base::Unretained(this)));
+  }
+
+  void OnJavascriptDisallowed() override {
+    importer_list_.reset();
+    importer_list_loaded_ = false;
+    if (importer_host_) {
+      importer_host_->set_observer(nullptr);
+      importer_host_ = nullptr;
+    }
+    ResetImportState();
+  }
+
+  void HandlePageReady(const base::ListValue& args) {
+    if (!IsJavascriptAllowed()) {
+      AllowJavascript();
+    }
+    if (importer_host_) {
+      importer_host_->set_observer(nullptr);
+      importer_host_ = nullptr;
+    }
+    ResetImportState();
+    SendState("detecting");
+    DetectSources();
+  }
+
+  void HandleRefreshSources(const base::ListValue& args) {
+    if (IsImportRunning()) {
+      SendFailure("importing", "import_in_progress",
+                  "An import is already in progress.");
+      return;
+    }
+
+    ResetImportState();
+    SendState("detecting");
+    DetectSources();
+  }
+
+  void HandleStartImport(const base::ListValue& args) {
+    if (IsImportRunning()) {
+      SendFailure("importing", "import_in_progress",
+                  "An import is already in progress.");
+      return;
+    }
+
+    if (!CanStartImportFromCurrentWebContents()) {
+      SendFailure(
+          "ready", "user_interaction_required",
+          "Click Import from the visible onboarding window to continue.");
+      return;
+    }
+
+    ResetImportState();
+    if (!importer_list_loaded_ || !importer_list_ ||
+        importer_list_->count() == 0) {
+      SendFailure("no_sources", "No detected import source is ready.");
+      return;
+    }
+
+    ImportRequestSelection selection;
+    if (!BuildImportSelection(args, &selection)) {
+      SendFailure("invalid_source", "Selected import source is not valid.");
+      return;
+    }
+
+    const user_data_importer::SourceProfile& source_profile =
+        importer_list_->GetSourceProfileAt(selection.source_index);
+    imported_items_ = GetEffectiveImportItems(source_profile, selection);
+
+    if (!imported_items_) {
+      SendFailure("no_supported_items",
+                  "Selected source has no supported import items.");
+      return;
+    }
+
+    std::string display_name = GetDisplayName(source_profile);
+    import_result_ = ImportSourceResult{selection.source_id, display_name,
+                                        ImportSourceResultStatus::kImporting};
+    current_item_ = user_data_importer::NONE;
+    completed_items_ = user_data_importer::NONE;
+    import_did_succeed_ = false;
+
+    if (importer_host_) {
+      importer_host_->set_observer(nullptr);
+      importer_host_ = nullptr;
+    }
+    importer_host_ = new ExternalProcessImporterHost();
+    importer_host_->set_observer(this);
+    Profile* profile = Profile::FromWebUI(web_ui());
+    SendState("importing");
+    importer_host_->StartImportSettings(source_profile, profile,
+                                        imported_items_,
+                                        new ProfileWriter(profile));
+  }
+
+  void HandleComplete(const base::ListValue& args) {
+    if (completion_handled_) {
+      return;
+    }
+    completion_handled_ = true;
+
+    SendState("completed");
+
+    if (completion_callback_) {
+      base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
+          FROM_HERE, completion_callback_);
+    }
+  }
+
+  bool CanStartImportFromCurrentWebContents() {
+    content::WebContents* contents = web_ui()->GetWebContents();
+    return contents &&
+           contents->GetVisibility() == content::Visibility::VISIBLE &&
+           contents->HasRecentInteraction();
+  }
+
+  void DetectSources() {
+    importer_list_loaded_ = false;
+    ResetImportState();
+    importer_list_ = std::make_unique<ImporterList>();
+    importer_list_->DetectSourceProfiles(
+        g_browser_process->GetApplicationLocale(), false,
+        base::BindOnce(&BrowserOSOnboardingHandler::HandleSourcesDetected,
+                       base::Unretained(this)));
+  }
+
+  void HandleSourcesDetected() {
+    importer_list_loaded_ = true;
+    SendState("ready");
+  }
+
+  bool FindSourceIndex(const std::string& source_id, int* index) const {
+    for (size_t i = 0; importer_list_ && i < importer_list_->count(); ++i) {
+      if (SourceIdForIndex(i) == source_id) {
+        *index = static_cast<int>(i);
+        return true;
+      }
+    }
+    return false;
+  }
+
+  bool BuildImportSelection(const base::ListValue& args,
+                            ImportRequestSelection* selection) const {
+    if (!args.empty() && args[0].is_dict()) {
+      const base::DictValue& request = args[0].GetDict();
+      if (request.contains("selections")) {
+        return false;
+      }
+      return BuildImportSelectionFromDict(request, selection);
+    }
+
+    std::optional<int> browser_index = args.empty() ? 0 : args[0].GetIfInt();
+    if (!browser_index) {
+      return false;
+    }
+    if (!IsValidSourceIndex(*browser_index)) {
+      return false;
+    }
+
+    selection->source_index = *browser_index;
+    selection->source_id = SourceIdForIndex(static_cast<size_t>(*browser_index));
+    selection->selected_items = user_data_importer::NONE;
+    selection->has_selected_items = false;
+    return true;
+  }
+
+  bool BuildImportSelectionFromDict(const base::DictValue& request,
+                                    ImportRequestSelection* selection) const {
+    const std::string* source_id = request.FindString("sourceId");
+    if (!source_id || !FindSourceIndex(*source_id, &selection->source_index)) {
+      return false;
+    }
+
+    selection->source_id = *source_id;
+    if (const base::ListValue* items = request.FindList("items")) {
+      selection->has_selected_items = true;
+      for (const base::Value& item : *items) {
+        if (item.is_string()) {
+          selection->selected_items |=
+              ImportItemMaskFromString(item.GetString());
+        }
+      }
+    }
+    return true;
+  }
+
+  bool IsValidSourceIndex(int index) const {
+    return index >= 0 && importer_list_ &&
+           index < static_cast<int>(importer_list_->count());
+  }
+
+  uint16_t GetEffectiveImportItems(
+      const user_data_importer::SourceProfile& source_profile,
+      const ImportRequestSelection& selection) const {
+    uint16_t supported_items =
+        source_profile.services_supported & kBrowserOSImportableItems;
+    return selection.has_selected_items
+               ? (selection.selected_items & supported_items)
+               : supported_items;
+  }
+
+  std::string GetDisplayName(
+      const user_data_importer::SourceProfile& source_profile) const {
+    std::string browser_name = base::UTF16ToUTF8(source_profile.importer_name);
+    std::string profile_name = base::UTF16ToUTF8(source_profile.profile);
+    return profile_name.empty() ? browser_name
+                                : browser_name + " - " + profile_name;
+  }
+
+  base::ListValue BuildSources() const {
+    base::ListValue sources;
+    for (size_t i = 0; importer_list_ && i < importer_list_->count(); ++i) {
+      const user_data_importer::SourceProfile& source_profile =
+          importer_list_->GetSourceProfileAt(i);
+      uint16_t services = source_profile.services_supported;
+      std::string browser_name =
+          base::UTF16ToUTF8(source_profile.importer_name);
+      std::string profile_name = base::UTF16ToUTF8(source_profile.profile);
+      std::string account_name =
+          base::UTF16ToUTF8(source_profile.account_name);
+
+      base::DictValue source;
+      source.Set("id", SourceIdForIndex(i));
+      source.Set("displayName", GetDisplayName(source_profile));
+      source.Set("browserName", browser_name);
+      source.Set("profileName", profile_name);
+      source.Set("accountName", account_name);
+      source.Set("isManaged", source_profile.is_managed);
+      source.Set("supportedItems", ImportItemsFromMask(services));
+      source.Set("recommendedItems", ImportItemsFromMask(services));
+      sources.Append(std::move(source));
+    }
+    return sources;
+  }
+
+  const char* ImportSourceResultStatusToString(
+      ImportSourceResultStatus status) const {
+    switch (status) {
+      case ImportSourceResultStatus::kImporting:
+        return "importing";
+      case ImportSourceResultStatus::kSucceeded:
+        return "succeeded";
+      case ImportSourceResultStatus::kFailed:
+        return "failed";
+    }
+    NOTREACHED();
+  }
+
+  base::ListValue BuildResults() const {
+    base::ListValue results;
+    if (import_result_) {
+      base::DictValue result_value;
+      result_value.Set("sourceId", import_result_->source_id);
+      result_value.Set("displayName", import_result_->display_name);
+      result_value.Set(
+          "status", ImportSourceResultStatusToString(import_result_->status));
+      results.Append(std::move(result_value));
+    }
+    return results;
+  }
+
+  base::DictValue BuildProgress() const {
+    base::DictValue progress;
+    progress.Set("completedItems", ImportItemsFromMask(completed_items_));
+    progress.Set("totalItems",
+                 static_cast<int>(ImportItemsFromMask(imported_items_).size()));
+    progress.Set("completedSources", GetCompletedSourceCount());
+    progress.Set("totalSources", import_result_ ? 1 : 0);
+    if (IsImportRunning()) {
+      progress.Set("currentSourceId", import_result_->source_id);
+      progress.Set("currentSourceName", import_result_->display_name);
+    }
+    const char* current_item = ImportItemToString(current_item_);
+    if (current_item) {
+      progress.Set("currentItem", current_item);
+    }
+    return progress;
+  }
+
+  void SendState(std::string_view status) {
+    if (IsJavascriptAllowed()) {
+      base::DictValue state;
+      state.Set("apiVersion", kBrowserOSOnboardingApiVersion);
+      state.Set("status", std::string(status));
+      state.Set("sources", BuildSources());
+      if (import_result_) {
+        state.Set("results", BuildResults());
+      }
+      if (imported_items_ || import_result_) {
+        state.Set("progress", BuildProgress());
+      }
+      CallJavascriptFunction("browserosOnboarding.receiveState", state);
+    }
+  }
+
+  void SendFailure(const std::string& code, const std::string& message) {
+    SendFailure("failed", code, message);
+  }
+
+  void SendFailure(std::string_view status,
+                   const std::string& code,
+                   const std::string& message) {
+    if (IsJavascriptAllowed()) {
+      base::DictValue state;
+      state.Set("apiVersion", kBrowserOSOnboardingApiVersion);
+      state.Set("status", std::string(status));
+      state.Set("sources", BuildSources());
+      if (import_result_) {
+        state.Set("results", BuildResults());
+      }
+      if (imported_items_ || import_result_) {
+        state.Set("progress", BuildProgress());
+      }
+      base::DictValue error;
+      error.Set("code", code);
+      error.Set("message", message);
+      state.Set("error", std::move(error));
+      CallJavascriptFunction("browserosOnboarding.receiveState", state);
+    }
+  }
+
+  void ImportStarted() override { SendState("importing"); }
+
+  void ImportItemStarted(user_data_importer::ImportItem item) override {
+    current_item_ = item;
+    SendState("importing");
+  }
+
+  void ImportItemEnded(user_data_importer::ImportItem item) override {
+    completed_items_ |= static_cast<uint16_t>(item);
+    current_item_ = user_data_importer::NONE;
+    import_did_succeed_ = true;
+    SendState("importing");
+  }
+
+  void ImportEnded() override {
+    if (importer_host_) {
+      importer_host_->set_observer(nullptr);
+      importer_host_ = nullptr;
+    }
+    if (!import_result_) {
+      return;
+    }
+
+    current_item_ = user_data_importer::NONE;
+    import_result_->status = import_did_succeed_
+                                 ? ImportSourceResultStatus::kSucceeded
+                                 : ImportSourceResultStatus::kFailed;
+    SendState(GetTerminalImportStatus());
+  }
+
+  bool IsImportRunning() const {
+    return import_result_ &&
+           import_result_->status == ImportSourceResultStatus::kImporting;
+  }
+
+  int GetCompletedSourceCount() const {
+    if (!import_result_) {
+      return 0;
+    }
+    return (import_result_->status == ImportSourceResultStatus::kSucceeded ||
+            import_result_->status == ImportSourceResultStatus::kFailed)
+               ? 1
+               : 0;
+  }
+
+  int GetSucceededSourceCount() const {
+    if (!import_result_) {
+      return 0;
+    }
+    return import_result_->status == ImportSourceResultStatus::kSucceeded ? 1
+                                                                          : 0;
+  }
+
+  const char* GetTerminalImportStatus() const {
+    return GetSucceededSourceCount() > 0 ? "succeeded" : "failed";
+  }
+
+  void ResetImportState() {
+    current_item_ = user_data_importer::NONE;
+    completed_items_ = user_data_importer::NONE;
+    imported_items_ = user_data_importer::NONE;
+    import_did_succeed_ = false;
+    import_result_.reset();
+  }
+
+  std::unique_ptr<ImporterList> importer_list_;
+  raw_ptr<ExternalProcessImporterHost> importer_host_ = nullptr;
+  base::RepeatingClosure completion_callback_;
+  std::optional<ImportSourceResult> import_result_;
+  user_data_importer::ImportItem current_item_ = user_data_importer::NONE;
+  uint16_t completed_items_ = user_data_importer::NONE;
+  uint16_t imported_items_ = user_data_importer::NONE;
+  bool importer_list_loaded_ = false;
+  bool import_did_succeed_ = false;
+  bool completion_handled_ = false;
+};
+
+BrowserOSOnboardingUIConfig::BrowserOSOnboardingUIConfig()
+    : DefaultWebUIConfig(content::kChromeUIScheme,
+                         chrome::kChromeUIBrowserOSOnboardingHost) {}
+
+BrowserOSOnboarding::BrowserOSOnboarding(content::WebUI* web_ui)
+    : content::WebUIController(web_ui) {
+  content::WebUIDataSource* source = content::WebUIDataSource::CreateAndAdd(
+      Profile::FromWebUI(web_ui), chrome::kChromeUIBrowserOSOnboardingHost);
+  webui::SetupWebUIDataSource(source, kBrowserosOnboardingResources,
+                              IDR_BROWSEROS_ONBOARDING_INDEX_HTML);
+
+  auto handler = std::make_unique<BrowserOSOnboardingHandler>();
+  handler_ = handler.get();
+  web_ui->AddMessageHandler(std::move(handler));
+}
+
+BrowserOSOnboarding::~BrowserOSOnboarding() = default;
+
+void BrowserOSOnboarding::SetCompletionCallback(
+    base::RepeatingClosure completion_callback) {
+  if (handler_) {
+    handler_->SetCompletionCallback(std::move(completion_callback));
+  }
+}
+
+WEB_UI_CONTROLLER_TYPE_IMPL(BrowserOSOnboarding)
