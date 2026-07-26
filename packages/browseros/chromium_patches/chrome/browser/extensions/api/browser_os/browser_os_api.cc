diff --git a/chrome/browser/extensions/api/browser_os/browser_os_api.cc b/chrome/browser/extensions/api/browser_os/browser_os_api.cc
new file mode 100644
index 0000000000000..ea477521a09d7
--- /dev/null
+++ b/chrome/browser/extensions/api/browser_os/browser_os_api.cc
@@ -0,0 +1,341 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/extensions/api/browser_os/browser_os_api.h"
+
+#include <algorithm>
+#include <memory>
+#include <optional>
+#include <string>
+#include <utility>
+
+#include "base/files/file_util.h"
+#include "base/logging.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/time/time.h"
+#include "base/values.h"
+#include "base/version_info/version_info.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+#include "chrome/browser/infobars/confirm_infobar_creator.h"
+#include "chrome/browser/platform_util.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/browser.h"
+#include "chrome/browser/ui/browser_finder.h"
+#include "chrome/browser/ui/browser_window/public/browser_window_features.h"
+#include "chrome/browser/ui/select_file_policy/chrome_select_file_policy.h"
+#include "chrome/browser/ui/tabs/tab_strip_model.h"
+#include "chrome/browser/ui/toasts/api/toast_id.h"
+#include "chrome/browser/ui/toasts/toast_controller.h"
+#include "chrome/common/extensions/api/browser_os.h"
+#include "components/infobars/content/content_infobar_manager.h"
+#include "components/infobars/core/infobar.h"
+#include "components/infobars/core/infobar_delegate.h"
+#include "components/infobars/core/simple_alert_infobar_delegate.h"
+#include "components/prefs/pref_service.h"
+#include "content/public/browser/web_contents.h"
+#include "ui/shell_dialogs/selected_file_info.h"
+
+namespace extensions {
+namespace api {
+
+namespace {
+
+PrefService* FindPrefService(const std::string& pref_name, Profile* profile) {
+  PrefService* local_state = g_browser_process->local_state();
+  if (local_state && local_state->FindPreference(pref_name)) {
+    return local_state;
+  }
+
+  PrefService* profile_prefs = profile->GetPrefs();
+  if (profile_prefs && profile_prefs->FindPreference(pref_name)) {
+    return profile_prefs;
+  }
+
+  return nullptr;
+}
+
+std::string GetPrefTypeName(const base::Value* value) {
+  switch (value->type()) {
+    case base::Value::Type::BOOLEAN:
+      return "boolean";
+    case base::Value::Type::INTEGER:
+    case base::Value::Type::DOUBLE:
+      return "number";
+    case base::Value::Type::STRING:
+      return "string";
+    case base::Value::Type::LIST:
+      return "list";
+    case base::Value::Type::DICT:
+      return "dictionary";
+    default:
+      return "unknown";
+  }
+}
+
+}  // namespace
+
+ExtensionFunction::ResponseAction BrowserOSGetPrefFunction::Run() {
+  std::optional<browser_os::GetPref::Params> params =
+      browser_os::GetPref::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  Profile* profile = Profile::FromBrowserContext(browser_context());
+  PrefService* prefs = FindPrefService(params->name, profile);
+
+  if (!prefs) {
+    return RespondNow(Error("Preference not found: " + params->name));
+  }
+
+  browser_os::PrefObject pref_obj;
+  pref_obj.key = params->name;
+
+  const base::Value* value = prefs->GetUserPrefValue(params->name);
+  if (!value) {
+    value = prefs->GetDefaultPrefValue(params->name);
+  }
+
+  pref_obj.type = GetPrefTypeName(value);
+  pref_obj.value = value->Clone();
+
+  return RespondNow(
+      ArgumentList(browser_os::GetPref::Results::Create(pref_obj)));
+}
+
+ExtensionFunction::ResponseAction BrowserOSSetPrefFunction::Run() {
+  std::optional<browser_os::SetPref::Params> params =
+      browser_os::SetPref::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Security: only allow modifying browseros.* prefs
+  if (!params->name.starts_with("browseros.")) {
+    return RespondNow(Error("Only browseros.* preferences can be modified"));
+  }
+
+  Profile* profile = Profile::FromBrowserContext(browser_context());
+  PrefService* prefs = FindPrefService(params->name, profile);
+
+  if (!prefs) {
+    return RespondNow(Error("Preference not found: " + params->name));
+  }
+
+  prefs->Set(params->name, params->value);
+
+  return RespondNow(ArgumentList(browser_os::SetPref::Results::Create(true)));
+}
+
+ExtensionFunction::ResponseAction BrowserOSLogMetricFunction::Run() {
+  std::optional<browser_os::LogMetric::Params> params =
+      browser_os::LogMetric::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  const std::string& event_name = params->event_name;
+
+  // Add "extension." prefix to distinguish from native events
+  std::string prefixed_event = "extension." + event_name;
+
+  if (params->properties.has_value()) {
+    // The properties parameter is a Properties struct with
+    // additional_properties member
+    base::DictValue properties =
+        params->properties->additional_properties.Clone();
+
+    properties.Set("extension_id", extension_id());
+
+    browseros_metrics::BrowserOSMetrics::Log(prefixed_event,
+                                             std::move(properties));
+  } else {
+    browseros_metrics::BrowserOSMetrics::Log(
+        prefixed_event, {{"extension_id", base::Value(extension_id())}});
+  }
+
+  return RespondNow(NoArguments());
+}
+
+ExtensionFunction::ResponseAction BrowserOSGetVersionNumberFunction::Run() {
+  std::string version = std::string(version_info::GetVersionNumber());
+
+  return RespondNow(
+      ArgumentList(browser_os::GetVersionNumber::Results::Create(version)));
+}
+
+ExtensionFunction::ResponseAction
+BrowserOSGetBrowserosVersionNumberFunction::Run() {
+  std::string version = std::string(version_info::GetBrowserOSVersionNumber());
+
+  return RespondNow(ArgumentList(
+      browser_os::GetBrowserosVersionNumber::Results::Create(version)));
+}
+
+namespace {
+
+constexpr char kCouldNotShowSelectFileDialogError[] =
+    "Could not show file dialog";
+
+ui::SelectFileDialog::Type GetDialogType(
+    const std::optional<browser_os::SelectionType>& type) {
+  if (type.has_value() && *type == browser_os::SelectionType::kFolder) {
+    return ui::SelectFileDialog::SELECT_FOLDER;
+  }
+  return ui::SelectFileDialog::SELECT_OPEN_FILE;
+}
+
+}  // namespace
+
+BrowserOSChoosePathFunction::BrowserOSChoosePathFunction() = default;
+
+BrowserOSChoosePathFunction::~BrowserOSChoosePathFunction() {
+  // Clean up pending file dialogs to prevent callbacks to a destroyed object.
+  if (select_file_dialog_.get()) {
+    select_file_dialog_->ListenerDestroyed();
+  }
+}
+
+ExtensionFunction::ResponseAction BrowserOSChoosePathFunction::Run() {
+  std::optional<browser_os::ChoosePath::Params> params =
+      browser_os::ChoosePath::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  content::WebContents* web_contents = GetSenderWebContents();
+  if (!web_contents) {
+    return RespondNow(Error(kCouldNotShowSelectFileDialogError));
+  }
+
+  ui::SelectFileDialog::Type dialog_type =
+      ui::SelectFileDialog::SELECT_OPEN_FILE;
+  std::u16string title;
+  base::FilePath starting_path;
+
+  if (params->options) {
+    dialog_type = GetDialogType(params->options->type);
+
+    if (params->options->title) {
+      title = base::UTF8ToUTF16(*params->options->title);
+    }
+
+    if (params->options->starting_directory) {
+      starting_path =
+          base::FilePath::FromUTF8Unsafe(*params->options->starting_directory);
+      // Validate path exists; if not, use empty path (OS default)
+      if (!base::DirectoryExists(starting_path)) {
+        starting_path = base::FilePath();
+      }
+    }
+  }
+
+  gfx::NativeWindow owning_window =
+      platform_util::GetTopLevel(web_contents->GetNativeView());
+
+  select_file_dialog_ = ui::SelectFileDialog::Create(
+      this, std::make_unique<ChromeSelectFilePolicy>(web_contents));
+
+  select_file_dialog_->SelectFile(
+      dialog_type, title, starting_path,
+      nullptr,                       // file_types
+      0,                             // file_type_index
+      base::FilePath::StringType(),  // default_extension
+      owning_window);
+
+  // prevent destruction while dialog is showing
+  AddRef();
+  return RespondLater();
+}
+
+void BrowserOSChoosePathFunction::FileSelected(const ui::SelectedFileInfo& file,
+                                               int index) {
+  browser_os::SelectedPath result;
+  result.path = file.path().AsUTF8Unsafe();
+  result.name = file.path().BaseName().AsUTF8Unsafe();
+
+  Respond(ArgumentList(browser_os::ChoosePath::Results::Create(result)));
+  Release();
+}
+
+void BrowserOSChoosePathFunction::FileSelectionCanceled() {
+  // Return null to indicate cancellation (not an error)
+  base::ListValue results;
+  results.Append(base::Value());
+  Respond(ArgumentList(std::move(results)));
+  Release();
+}
+
+ExtensionFunction::ResponseAction BrowserOSShowToastFunction::Run() {
+  std::optional<browser_os::ShowToast::Params> params =
+      browser_os::ShowToast::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  if (params->message.empty()) {
+    return RespondNow(Error("Toast message must not be empty"));
+  }
+
+  Profile* profile = Profile::FromBrowserContext(browser_context());
+  Browser* browser = chrome::FindLastActiveWithProfile(profile);
+  if (!browser) {
+    return RespondNow(Error("No active browser window"));
+  }
+
+  ToastController* toast_controller = browser->GetFeatures().toast_controller();
+  if (!toast_controller) {
+    return RespondNow(Error("Toast controller unavailable"));
+  }
+
+  ToastParams toast_params(ToastId::kBrowserOSToast);
+  toast_params.body_string_override = base::UTF8ToUTF16(params->message);
+  if (params->duration_ms) {
+    constexpr int kMinDurationMs = 1000;
+    constexpr int kMaxDurationMs = 30000;
+    toast_params.timeout_override = base::Milliseconds(
+        std::clamp(*params->duration_ms, kMinDurationMs, kMaxDurationMs));
+  }
+
+  const bool shown = toast_controller->MaybeShowToast(std::move(toast_params));
+  return RespondNow(
+      ArgumentList(browser_os::ShowToast::Results::Create(shown)));
+}
+
+// Bridges chrome.browserOS.showInfoBar to Chrome's tab-scoped alert infobar.
+ExtensionFunction::ResponseAction BrowserOSShowInfoBarFunction::Run() {
+  std::optional<browser_os::ShowInfoBar::Params> params =
+      browser_os::ShowInfoBar::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  if (params->message.empty()) {
+    return RespondNow(Error("InfoBar message must not be empty"));
+  }
+
+  Profile* profile = Profile::FromBrowserContext(browser_context());
+  Browser* browser = chrome::FindLastActiveWithProfile(profile);
+  if (!browser) {
+    return RespondNow(Error("No active browser window"));
+  }
+
+  content::WebContents* contents =
+      browser->tab_strip_model()->GetActiveWebContents();
+  if (!contents) {
+    return RespondNow(Error("No active tab"));
+  }
+
+  infobars::ContentInfoBarManager* infobar_manager =
+      infobars::ContentInfoBarManager::FromWebContents(contents);
+  if (!infobar_manager) {
+    return RespondNow(Error("InfoBar manager unavailable"));
+  }
+
+  const bool shown =
+      infobar_manager
+          ->AddInfoBar(CreateConfirmInfoBar(
+              std::make_unique<SimpleAlertInfoBarDelegate>(
+                  infobars::InfoBarDelegate::
+                      BROWSEROS_EXTENSION_INFOBAR_DELEGATE,
+                  nullptr, base::UTF8ToUTF16(params->message),
+                  /*auto_expire=*/true, /*should_animate=*/true,
+                  /*closeable=*/true)))
+          != nullptr;
+
+  return RespondNow(
+      ArgumentList(browser_os::ShowInfoBar::Results::Create(shown)));
+}
+
+}  // namespace api
+}  // namespace extensions
