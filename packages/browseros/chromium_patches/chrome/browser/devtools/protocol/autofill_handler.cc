diff --git a/chrome/browser/devtools/protocol/autofill_handler.cc b/chrome/browser/devtools/protocol/autofill_handler.cc
index bd7972bc4a5a4d52782724e1b9794e31deca0525..c8832d087c05c5e1267ff0c05de52d5ae991c7ab 100644
--- a/chrome/browser/devtools/protocol/autofill_handler.cc
+++ b/chrome/browser/devtools/protocol/autofill_handler.cc
@@ -4,14 +4,28 @@
 
 #include "chrome/browser/devtools/protocol/autofill_handler.h"
 
+#include <algorithm>
+#include <map>
 #include <optional>
+#include <set>
+#include <tuple>
 #include <variant>
 
 #include "base/check_deref.h"
 #include "base/containers/to_vector.h"
+#include "base/hash/sha1.h"
+#include "base/i18n/case_conversion.h"
 #include "base/memory/scoped_refptr.h"
+#include "base/strings/string_number_conversions.h"
 #include "base/strings/utf_string_conversions.h"
+#include "base/supports_user_data.h"
+#include "base/timer/timer.h"
+#include "base/uuid.h"
+#include "chrome/browser/devtools/devtools_browser_context_manager.h"
 #include "chrome/browser/devtools/protocol/autofill.h"
+#include "chrome/browser/password_manager/chrome_password_manager_client.h"
+#include "chrome/browser/password_manager/factories/account_password_store_factory.h"
+#include "chrome/browser/password_manager/factories/profile_password_store_factory.h"
 #include "components/autofill/content/browser/content_autofill_client.h"
 #include "components/autofill/content/browser/content_autofill_driver.h"
 #include "components/autofill/content/browser/content_autofill_driver_factory.h"
@@ -31,9 +45,18 @@
 #include "components/autofill/core/common/form_field_data.h"
 #include "components/autofill/core/common/mojom/autofill_types.mojom-shared.h"
 #include "components/autofill/core/common/unique_ids.h"
+#include "components/device_reauth/device_authenticator.h"
+#include "components/password_manager/core/browser/password_form_cache.h"
+#include "components/password_manager/core/browser/password_form_manager.h"
+#include "components/password_manager/core/browser/password_manager_driver.h"
+#include "components/password_manager/core/browser/password_manager_interface.h"
+#include "components/password_manager/core/browser/password_manager_util.h"
+#include "components/password_manager/core/browser/password_store/password_store_consumer.h"
 #include "content/public/browser/devtools_agent_host.h"
 #include "content/public/browser/render_frame_host.h"
+#include "content/public/browser/weak_document_ptr.h"
 #include "third_party/blink/public/common/features.h"
+#include "url/origin.h"
 
 namespace {
 
@@ -71,6 +94,809 @@ std::optional<std::string> GetRenderFrameDevtoolsToken(
 
 }  // namespace
 
+namespace {
+
+using Credential = password_manager::StoredCredential;
+using Store = password_manager::PasswordStoreInterface;
+using CredentialResult = protocol::Autofill::CredentialResult;
+using CredentialInfo = protocol::Autofill::CredentialInfo;
+constexpr char kCredentialRegistryKey[] = "browseros.cdp.credentials";
+
+Response CredentialError(const char* code) {
+  // Fixed codes only: neither native exceptions nor supplied secrets reach CDP.
+  return Response::ServerError(std::string("credentials:") + code);
+}
+
+std::string DocumentId(content::RenderFrameHost* frame) {
+  return frame->GetDevToolsFrameToken().ToString() + "." +
+         base::NumberToString(frame->GetNavigationId());
+}
+
+bool IsPassword(const Credential& credential) {
+  return credential.scheme == password_manager::PasswordForm::Scheme::kHtml &&
+         !credential.blocked_by_user && !credential.password_value.empty() &&
+         !credential.federation_origin.IsValid() &&
+         credential.url.SchemeIsHTTPOrHTTPS();
+}
+
+// Profile owns handles, not a page or DevTools connection. The registry retains
+// salted fingerprints only, never password strings. Store notifications retire
+// changed/deleted handles and invalidate pagination, including between
+// requests.
+class CredentialRegistry : public base::SupportsUserData::Data,
+                           public Store::Observer {
+ public:
+  static CredentialRegistry& Get(Profile* profile) {
+    auto* registry = static_cast<CredentialRegistry*>(
+        profile->GetUserData(kCredentialRegistryKey));
+    if (!registry) {
+      auto owned = std::make_unique<CredentialRegistry>(profile);
+      registry = owned.get();
+      profile->SetUserData(kCredentialRegistryKey, std::move(owned));
+    }
+    return *registry;
+  }
+  explicit CredentialRegistry(Profile* profile)
+      : profile_store(ProfilePasswordStoreFactory::GetForProfile(
+            profile,
+            ServiceAccessType::EXPLICIT_ACCESS)),
+        account_store(AccountPasswordStoreFactory::GetForProfile(
+            profile,
+            ServiceAccessType::EXPLICIT_ACCESS)) {
+    if (profile_store) {
+      profile_store->AddObserver(this);
+    }
+    if (account_store) {
+      account_store->AddObserver(this);
+    }
+  }
+  ~CredentialRegistry() override;
+  std::string Fingerprint(const Credential& c) const {
+    return Hash({c.signon_realm, base::UTF16ToUTF8(c.username_value),
+                 base::UTF16ToUTF8(c.password_value.value())});
+  }
+  std::string RowKey(Store* store, const Credential& c) const {
+    return Hash({store == account_store.get() ? "account" : "profile",
+                 c.signon_realm, c.url.spec(),
+                 base::UTF16ToUTF8(c.username_element),
+                 base::UTF16ToUTF8(c.username_value),
+                 base::UTF16ToUTF8(c.password_element)});
+  }
+  std::string Identify(const Credential& c) {
+    auto fingerprint = Fingerprint(c);
+    auto [it, inserted] = handles.try_emplace(fingerprint);
+    if (inserted) {
+      it->second = "cred_" + base::Uuid::GenerateRandomV4().AsLowercaseString();
+    }
+    Store* store =
+        c.IsUsingAccountStore() ? account_store.get() : profile_store.get();
+    const auto key = RowKey(store, c);
+    rows[key] = fingerprint;
+    (store == account_store.get() ? account_rows : profile_rows).insert(key);
+    return it->second;
+  }
+  bool Matches(const std::string& id, const Credential& c) const {
+    auto it = handles.find(Fingerprint(c));
+    return it != handles.end() && it->second == id;
+  }
+  void OnLoginsChanged(
+      Store* store,
+      const password_manager::PasswordStoreChangeList& changes) override;
+  void OnLoginsRetained(Store* store,
+                        const std::vector<Credential>& retained) override;
+  scoped_refptr<Store> profile_store;
+  scoped_refptr<Store> account_store;
+  uint64_t revision = 0;
+  bool adding = false;
+  const std::string nonce = base::Uuid::GenerateRandomV4().AsLowercaseString();
+
+ private:
+  std::string Hash(std::initializer_list<std::string> parts) const {
+    std::string input = nonce;
+    for (const auto& part : parts) {
+      input += base::NumberToString(part.size()) + ":" + part;
+    }
+    return base::SHA1HashString(input);
+  }
+  std::map<std::string, std::string> handles;
+  std::map<std::string, std::string> rows;
+  std::set<std::string> account_rows;
+  std::set<std::string> profile_rows;
+};
+
+CredentialRegistry::~CredentialRegistry() {
+  if (profile_store) {
+    profile_store->RemoveObserver(this);
+  }
+  if (account_store) {
+    account_store->RemoveObserver(this);
+  }
+}
+
+void CredentialRegistry::OnLoginsChanged(
+    Store* store,
+    const password_manager::PasswordStoreChangeList& changes) {
+  ++revision;
+  for (const auto& change : changes) {
+    const auto key = RowKey(store, change.credential());
+    auto it = rows.find(key);
+    if (it != rows.end() &&
+        (change.type() == password_manager::PasswordStoreChange::REMOVE ||
+         it->second != Fingerprint(change.credential()))) {
+      handles.erase(it->second);
+      rows.erase(it);
+    }
+  }
+}
+
+void CredentialRegistry::OnLoginsRetained(
+    Store* store,
+    const std::vector<Credential>& retained) {
+  ++revision;
+  std::map<std::string, std::string> live;
+  for (const auto& c : retained) {
+    live[RowKey(store, c)] = Fingerprint(c);
+  }
+  // Only keys belonging to this store are checked. Other-store entries may
+  // still back a merged account; retiring its old handle is conservative.
+  auto& known = store == account_store.get() ? account_rows : profile_rows;
+  for (const auto& key : known) {
+    auto it = rows.find(key);
+    if (it != rows.end() && (!live.contains(key) || live[key] != it->second)) {
+      handles.erase(it->second);
+      rows.erase(it);
+    }
+  }
+  known.clear();
+  for (const auto& [key, value] : live) {
+    known.insert(key);
+  }
+}
+
+}  // namespace
+
+// Owns one asynchronous operation through store and renderer handoffs. Every
+// handoff rechecks the captured document and regular profile. Handler teardown,
+// explicit cancellation and the deadline invalidate all outstanding callbacks;
+// cancellation after write dispatch reports an uncertain save outcome.
+class CredentialOperation : public password_manager::PasswordStoreConsumer {
+ public:
+  using Completion =
+      base::OnceCallback<void(Response, std::unique_ptr<CredentialResult>)>;
+  CredentialOperation(std::string id,
+                      Profile* profile,
+                      content::WebContents* contents,
+                      Completion completion)
+      : id_(std::move(id)),
+        profile_(profile->GetWeakPtr()),
+        contents_(contents ? contents->GetWeakPtr() : nullptr),
+        document_(contents
+                      ? contents->GetPrimaryMainFrame()->GetWeakDocumentPtr()
+                      : content::WeakDocumentPtr()),
+        origin_(contents
+                    ? contents->GetPrimaryMainFrame()->GetLastCommittedOrigin()
+                    : url::Origin()),
+        completion_(std::move(completion)) {}
+  ~CredentialOperation() override;
+  bool pending() const { return !completion_.is_null(); }
+  const std::string& id() const { return id_; }
+  void Cancel() {
+    Fail(write_dispatched_ ? "save_outcome_unknown" : "operation_cancelled");
+  }
+  void Start(std::string action,
+             std::optional<std::string> credential_id,
+             std::optional<std::string> username,
+             std::optional<std::string> password,
+             std::string query,
+             int limit,
+             std::optional<std::string> cursor);
+  void OnGetPasswordStoreResultsOrErrorFrom(
+      Store* store,
+      base::expected<std::vector<Credential>,
+                     password_manager::PasswordStoreBackendError> result)
+      override;
+
+ private:
+  bool Validate() {
+    if (!pending()) {
+      return false;
+    }
+    if (!profile_ ||
+        !DevToolsBrowserContextManager::GetInstance().GetBrowserProfileId(
+            profile_.get())) {
+      Fail("operation_not_allowed");
+      return false;
+    }
+    if (!origin_.opaque()) {
+      auto* frame = document_.AsRenderFrameHostIfValid();
+      if (!contents_ || !frame || contents_->GetPrimaryMainFrame() != frame ||
+          !frame->IsActive() || frame->GetLastCommittedOrigin() != origin_) {
+        Fail("page_changed");
+        return false;
+      }
+      auto* client =
+          ChromePasswordManagerClient::FromWebContents(contents_.get());
+      if (!client || !client->IsFillingEnabled(origin_) ||
+          (action_ == "add" && !client->IsSavingAndFillingEnabled(origin_))) {
+        Fail("operation_not_allowed");
+        return false;
+      }
+    }
+    return true;
+  }
+  std::unique_ptr<CredentialResult> Result() {
+    auto result =
+        CredentialResult::Create()
+            .SetBrowserProfileId(*DevToolsBrowserContextManager::GetInstance()
+                                      .GetBrowserProfileId(profile_.get()))
+            .Build();
+    if (!origin_.opaque()) {
+      result->SetOrigin(origin_.Serialize());
+    }
+    return result;
+  }
+  std::unique_ptr<CredentialInfo> Info(const Credential& c) {
+    auto stores = std::make_unique<protocol::Array<String>>();
+    if (c.IsUsingProfileStore()) {
+      stores->push_back("profile");
+    }
+    if (c.IsUsingAccountStore()) {
+      stores->push_back("account");
+    }
+    return CredentialInfo::Create()
+        .SetCredentialId(CredentialRegistry::Get(profile_.get()).Identify(c))
+        .SetUsername(base::UTF16ToUTF8(c.username_value))
+        .SetSite(c.signon_realm)
+        .SetStores(std::move(stores))
+        .Build();
+  }
+  void List() {
+    auto& registry = CredentialRegistry::Get(profile_.get());
+    std::map<std::string, std::unique_ptr<CredentialInfo>> merged;
+    for (const auto& c : credentials_) {
+      if (!query_.empty() &&
+          base::i18n::ToLower(c.username_value).find(query_) ==
+              std::u16string::npos &&
+          base::i18n::ToLower(base::UTF8ToUTF16(c.signon_realm)).find(query_) ==
+              std::u16string::npos) {
+        continue;
+      }
+      auto info = Info(c);
+      std::string id = info->GetCredentialId();
+      if (merged.contains(id)) {
+        auto* stores = merged[id]->GetStores();
+        for (const auto& store : *info->GetStores()) {
+          if (!std::ranges::contains(*stores, store)) {
+            stores->push_back(store);
+          }
+        }
+      } else {
+        merged[id] = std::move(info);
+      }
+    }
+    std::vector<std::unique_ptr<CredentialInfo>> sorted;
+    for (auto& [id, info] : merged) {
+      sorted.push_back(std::move(info));
+    }
+    std::ranges::sort(sorted, [](const auto& a, const auto& b) {
+      return std::tuple(a->GetSite(), a->GetUsername(), a->GetCredentialId()) <
+             std::tuple(b->GetSite(), b->GetUsername(), b->GetCredentialId());
+    });
+    // The token contains only a salted context hash, inventory revision and an
+    // offset. Re-reading native stores on every page detects concurrent edits.
+    const std::string prefix =
+        base::HexEncode(base::SHA1HashString(
+            registry.nonce + origin_.Serialize() + base::UTF16ToUTF8(query_))) +
+        "." + base::NumberToString(registry.revision) + ".";
+    size_t offset = 0;
+    if (cursor_ &&
+        (!cursor_->starts_with(prefix) ||
+         !base::StringToSizeT(cursor_->substr(prefix.size()), &offset) ||
+         offset >= sorted.size())) {
+      Fail("stale_cursor");
+      return;
+    }
+    auto result = Result();
+    result->SetTotal(static_cast<int>(sorted.size()));
+    auto entries = std::make_unique<protocol::Array<CredentialInfo>>();
+    size_t end = std::min(sorted.size(), offset + limit_);
+    for (; offset < end; ++offset) {
+      entries->push_back(std::move(sorted[offset]));
+    }
+    if (end < sorted.size()) {
+      result->SetNextCursor(prefix + base::NumberToString(end));
+    }
+    result->SetCredentials(std::move(entries));
+    Finish(Response::Success(), std::move(result));
+  }
+  void Add() {
+    const Credential* identical = nullptr;
+    for (const auto& c : credentials_) {
+      if (c.signon_realm != origin_.GetURL().spec() ||
+          c.username_value != username_) {
+        continue;
+      }
+      if (c.password_value != password_) {
+        Fail("credential_conflict");
+        return;
+      }
+      identical = &c;
+    }
+    if (identical) {
+      auto result = Result();
+      result->SetStatus("unchanged");
+      result->SetCredential(Info(*identical));
+      Finish(Response::Success(), std::move(result));
+      return;
+    }
+    if (!Validate()) {
+      return;
+    }
+    Credential c;
+    c.url = origin_.GetURL();
+    c.signon_realm = c.url.spec();
+    c.username_value = username_;
+    c.password_value = password_;
+    c.date_created = c.date_password_modified = base::Time::Now();
+    c.in_store = password_manager::PasswordForm::Store::kProfileStore;
+    auto result = Result();
+    result->SetStatus("saved");
+    result->SetCredential(Info(c));
+    write_dispatched_ = true;
+    // The registry serializes this read/check/write flow across all CDP
+    // sessions in the profile. Ordinary password-manager mutations still
+    // invalidate reads.
+    CredentialRegistry::Get(profile_.get())
+        .profile_store->AddLoginWithResult(
+            std::move(c),
+            base::BindOnce(&CredentialOperation::Saved, weak_.GetWeakPtr(),
+                           std::move(result)));
+  }
+  void Saved(std::unique_ptr<CredentialResult> result,
+             password_manager::PasswordStoreAddStatus status) {
+    using Status = password_manager::PasswordStoreAddStatus;
+    if (status == Status::kOutcomeUnknown) {
+      Fail("save_outcome_unknown");
+      return;
+    }
+    if (status == Status::kConflict) {
+      Fail("credential_conflict");
+      return;
+    }
+    if (status == Status::kError) {
+      Fail("store_error");
+      return;
+    }
+    result->SetStatus(status == Status::kUnchanged ? "unchanged" : "saved");
+    // Once dispatched, the transaction belongs to its captured site, even if
+    // the page navigated. Only a successful database commit reports saved.
+    Finish(Response::Success(), std::move(result));
+  }
+  void Fill() {
+    if (!Validate()) {
+      return;
+    }
+    auto* client =
+        ChromePasswordManagerClient::FromWebContents(contents_.get());
+    auto authenticator = client->GetDeviceAuthenticator();
+    if (client->IsReauthBeforeFillingRequired(authenticator.get())) {
+      Fail("reauth_required");
+      return;
+    }
+    auto* cache = client->GetPasswordManager()->GetPasswordFormCache();
+    const password_manager::PasswordFormManager* chosen = nullptr;
+    for (const auto& manager : cache->GetFormManagers()) {
+      auto driver = manager->GetDriver();
+      const auto* form = manager->GetParsedObservedForm();
+      if (!driver || !driver->IsInPrimaryMainFrame() || !form ||
+          (!(form->HasPasswordElement() && !form->HasNewPasswordElement()) &&
+           !form->IsSingleUsername())) {
+        continue;
+      }
+      if (chosen) {
+        Fail("no_login_form");
+        return;
+      }  // Ambiguous form: never guess an account target.
+      chosen = manager.get();
+    }
+    if (!chosen) {
+      Fail("no_login_form");
+      return;
+    }
+    auto driver = chosen->GetDriver();
+    const auto* form = chosen->GetParsedObservedForm();
+    std::vector<std::pair<autofill::FieldRendererId, std::string>> fields;
+    if (!form->username_element_renderer_id.is_null()) {
+      fields.emplace_back(form->username_element_renderer_id, "username");
+    }
+    if (!form->password_element_renderer_id.is_null()) {
+      fields.emplace_back(form->password_element_renderer_id, "password");
+    }
+    if (fields.empty()) {
+      Fail("no_fillable_fields");
+      return;
+    }
+    remaining_ = fields.size();
+    for (const auto& [field, type] : fields) {
+      if (!Validate()) {
+        return;
+      }
+      driver->FillField(field,
+                        type == "username" ? username_ : password_.value(),
+                        autofill::FieldPropertiesFlags::kAutofilledActorLogin,
+                        base::BindOnce(&CredentialOperation::FieldFilled,
+                                       weak_.GetWeakPtr(), type));
+    }
+  }
+  void FieldFilled(std::string field, bool filled) {
+    if (!Validate()) {
+      return;
+    }
+    if (filled) {
+      filled_.insert(field);
+    }
+    if (--remaining_ != 0) {
+      return;
+    }
+    if (filled_.empty()) {
+      Fail("no_fillable_fields");
+      return;
+    }
+    auto fields = std::make_unique<protocol::Array<String>>();
+    if (filled_.contains("username")) {
+      fields->push_back("username");
+    }
+    if (filled_.contains("password")) {
+      fields->push_back("password");
+    }
+    auto result = Result();
+    result->SetStatus("filled");
+    result->SetFilledFields(std::move(fields));
+    Finish(Response::Success(), std::move(result));
+  }
+  void ReleaseAdd() {
+    if (owns_add_ && profile_) {
+      CredentialRegistry::Get(profile_.get()).adding = false;
+    }
+    owns_add_ = false;
+  }
+  void Fail(const char* code) {
+    if (!pending()) {
+      return;
+    }
+    if (write_dispatched_ && std::string_view(code) == "timeout") {
+      code = "save_outcome_unknown";
+    }
+    Finish(CredentialError(code), nullptr);
+  }
+  void Finish(Response response, std::unique_ptr<CredentialResult> result) {
+    timer_.Stop();
+    weak_.InvalidateWeakPtrs();
+    ReleaseAdd();
+    credentials_.clear();
+    password_.clear();
+    username_.clear();
+    std::move(completion_).Run(response, std::move(result));
+  }
+  const std::string id_;
+  base::WeakPtr<Profile> profile_;
+  base::WeakPtr<content::WebContents> contents_;
+  content::WeakDocumentPtr document_;
+  url::Origin origin_;
+  Completion completion_;
+  std::string action_;
+  std::optional<std::string> credential_id_;
+  std::u16string username_;
+  password_manager::PasswordString password_;
+  std::u16string query_;
+  int limit_ = 50;
+  std::optional<std::string> cursor_;
+  int remaining_ = 0;
+  uint64_t revision_ = 0;
+  bool owns_add_ = false;
+  bool write_dispatched_ = false;
+  std::vector<Credential> credentials_;
+  std::set<std::string> filled_;
+  base::OneShotTimer timer_;
+  base::WeakPtrFactory<CredentialOperation> weak_{this};
+};
+
+void CredentialOperation::Start(std::string action,
+                                std::optional<std::string> credential_id,
+                                std::optional<std::string> username,
+                                std::optional<std::string> password,
+                                std::string query,
+                                int limit,
+                                std::optional<std::string> cursor) {
+  action_ = std::move(action);
+  credential_id_ = std::move(credential_id);
+  username_ = username ? base::UTF8ToUTF16(*username) : std::u16string();
+  if (password) {
+    password_ = password_manager::PasswordString(base::UTF8ToUTF16(*password));
+  }
+  query_ = base::i18n::ToLower(base::UTF8ToUTF16(query));
+  limit_ = limit;
+  cursor_ = std::move(cursor);
+  timer_.Start(FROM_HERE, base::Seconds(30),
+               base::BindOnce(&CredentialOperation::Fail, weak_.GetWeakPtr(),
+                              "timeout"));
+  if (!Validate()) {
+    return;
+  }
+  auto& registry = CredentialRegistry::Get(profile_.get());
+  if (action_ == "add") {
+    if (registry.adding) {
+      Fail("busy");
+      return;
+    }
+    registry.adding = owns_add_ = true;
+  }
+  if (action_ == "fill" && !credential_id_) {
+    Fill();
+    return;
+  }
+  if (!registry.profile_store) {
+    Fail("store_unavailable");
+    return;
+  }
+  revision_ = registry.revision;
+  remaining_ = registry.account_store ? 2 : 1;
+  for (Store* store :
+       {registry.profile_store.get(), registry.account_store.get()}) {
+    if (!store) {
+      continue;
+    }
+    if (contents_ && action_ != "add") {
+      store->GetLoginsStrict(password_manager::PasswordFormDigest(
+                                 password_manager::PasswordForm::Scheme::kHtml,
+                                 origin_.GetURL().spec(), origin_.GetURL()),
+                             weak_.GetWeakPtr());
+    } else {
+      store->GetAllLoginsStrict(weak_.GetWeakPtr());
+    }
+  }
+}
+
+CredentialOperation::~CredentialOperation() {
+  ReleaseAdd();
+}
+
+void CredentialOperation::OnGetPasswordStoreResultsOrErrorFrom(
+    Store* store,
+    base::expected<std::vector<Credential>,
+                   password_manager::PasswordStoreBackendError> result) {
+  if (!Validate()) {
+    return;
+  }
+  if (!result.has_value()) {
+    Fail("store_error");
+    return;
+  }
+  auto& registry = CredentialRegistry::Get(profile_.get());
+  for (auto& c : *result) {
+    if (!IsPassword(c) ||
+        (c.match_type.has_value() &&
+         password_manager_util::GetMatchType(c) ==
+             password_manager_util::GetLoginMatchType::kGrouped)) {
+      continue;
+    }
+    c.in_store = store == registry.account_store.get()
+                     ? password_manager::PasswordForm::Store::kAccountStore
+                     : password_manager::PasswordForm::Store::kProfileStore;
+    credentials_.push_back(std::move(c));
+  }
+  if (--remaining_ != 0) {
+    return;
+  }
+  if (registry.revision != revision_) {
+    Fail(action_ == "list" ? "stale_cursor" : "busy");
+    return;
+  }
+  if (action_ == "list") {
+    List();
+    return;
+  }
+  if (action_ == "add") {
+    Add();
+    return;
+  }
+  for (const auto& c : credentials_) {
+    if (registry.Matches(*credential_id_, c)) {
+      username_ = c.username_value;
+      password_ = c.password_value;
+      Fill();
+      return;
+    }
+  }
+  Fail("credential_not_found");
+}
+
+namespace {
+
+// Page commands never accept a context override. Root inventory requires an
+// explicit loaded regular profile; there is no active-window fallback.
+Response ResolveCredentialContext(const std::string& target_id,
+                                  const std::optional<String>& document_id,
+                                  const std::optional<String>& context_id,
+                                  Profile** profile,
+                                  content::WebContents** contents) {
+  auto host = content::DevToolsAgentHost::GetForId(target_id);
+  if (!host) {
+    return CredentialError("page_changed");
+  }
+  *contents = host->GetWebContents();
+  if (*contents) {
+    auto* frame = (*contents)->GetPrimaryMainFrame();
+    if (host->GetType() != content::DevToolsAgentHost::kTypePage ||
+        context_id || !document_id || !frame ||
+        DocumentId(frame) != *document_id || !frame->IsActive()) {
+      return CredentialError("page_changed");
+    }
+    if (!frame->GetLastCommittedURL().SchemeIsHTTPOrHTTPS() ||
+        frame->GetLastCommittedOrigin().opaque()) {
+      return CredentialError("operation_not_allowed");
+    }
+    *profile = Profile::FromBrowserContext((*contents)->GetBrowserContext());
+  } else {
+    if (host->GetType() != content::DevToolsAgentHost::kTypeBrowser ||
+        !context_id || document_id) {
+      return CredentialError("operation_not_allowed");
+    }
+    *profile = DevToolsBrowserContextManager::GetInstance().GetProfileById(
+        *context_id);
+  }
+  if (!*profile ||
+      !DevToolsBrowserContextManager::GetInstance().GetBrowserProfileId(
+          *profile)) {
+    return CredentialError("operation_not_allowed");
+  }
+  return Response::Success();
+}
+
+template <typename Callback>
+CredentialOperation::Completion CredentialCompletion(
+    std::unique_ptr<Callback> callback) {
+  return base::BindOnce(
+      [](std::unique_ptr<Callback> cb, Response response,
+         std::unique_ptr<CredentialResult> result) {
+        if (response.IsSuccess()) {
+          cb->sendSuccess(std::move(result));
+        } else {
+          cb->sendFailure(response);
+        }
+      },
+      std::move(callback));
+}
+}  // namespace
+
+Response AutofillHandler::GetCredentialContext(
+    std::unique_ptr<protocol::Autofill::CredentialContext>* context) {
+  auto host = content::DevToolsAgentHost::GetForId(target_id_);
+  auto* contents = host ? host->GetWebContents() : nullptr;
+  auto* frame = contents ? contents->GetPrimaryMainFrame() : nullptr;
+  if (!frame) {
+    return CredentialError("page_changed");
+  }
+  Profile* profile = nullptr;
+  content::WebContents* admitted = nullptr;
+  auto response = ResolveCredentialContext(target_id_, DocumentId(frame),
+                                           std::nullopt, &profile, &admitted);
+  if (!response.IsSuccess()) {
+    return response;
+  }
+  *context =
+      protocol::Autofill::CredentialContext::Create()
+          .SetBrowserProfileId(
+              *DevToolsBrowserContextManager::GetInstance().GetBrowserProfileId(
+                  profile))
+          .SetOrigin(frame->GetLastCommittedOrigin().Serialize())
+          .SetDocumentId(DocumentId(frame))
+          .Build();
+  return Response::Success();
+}
+
+void AutofillHandler::ListCredentials(
+    const String& operation_id,
+    std::optional<String> document_id,
+    std::optional<String> browser_context_id,
+    std::optional<String> query,
+    std::optional<int> limit,
+    std::optional<String> cursor,
+    std::unique_ptr<ListCredentialsCallback> callback) {
+  Profile* profile = nullptr;
+  content::WebContents* contents = nullptr;
+  auto response = ResolveCredentialContext(
+      target_id_, document_id, browser_context_id, &profile, &contents);
+  if (!response.IsSuccess()) {
+    callback->sendFailure(response);
+    return;
+  }
+  if (operation_id.empty() || limit.value_or(50) < 1 ||
+      limit.value_or(50) > 100) {
+    callback->sendFailure(CredentialError("invalid_arguments"));
+    return;
+  }
+  if (credential_operation_ && credential_operation_->pending()) {
+    callback->sendFailure(CredentialError("busy"));
+    return;
+  }
+  credential_operation_ = std::make_unique<CredentialOperation>(
+      operation_id, profile, contents,
+      CredentialCompletion(std::move(callback)));
+  credential_operation_->Start("list", std::nullopt, std::nullopt, std::nullopt,
+                               query.value_or(""), limit.value_or(50), cursor);
+}
+
+void AutofillHandler::AddCredential(
+    const String& operation_id,
+    const String& document_id,
+    const String& username,
+    const String& password,
+    std::unique_ptr<AddCredentialCallback> callback) {
+  Profile* profile = nullptr;
+  content::WebContents* contents = nullptr;
+  auto response = ResolveCredentialContext(target_id_, document_id,
+                                           std::nullopt, &profile, &contents);
+  if (!response.IsSuccess()) {
+    callback->sendFailure(response);
+    return;
+  }
+  if (operation_id.empty() || password.empty() || !contents) {
+    callback->sendFailure(CredentialError("invalid_arguments"));
+    return;
+  }
+  if (credential_operation_ && credential_operation_->pending()) {
+    callback->sendFailure(CredentialError("busy"));
+    return;
+  }
+  credential_operation_ = std::make_unique<CredentialOperation>(
+      operation_id, profile, contents,
+      CredentialCompletion(std::move(callback)));
+  credential_operation_->Start("add", std::nullopt, username, password, "", 50,
+                               std::nullopt);
+}
+
+void AutofillHandler::FillCredential(
+    const String& operation_id,
+    const String& document_id,
+    std::optional<String> credential_id,
+    std::optional<String> username,
+    std::optional<String> password,
+    std::unique_ptr<FillCredentialCallback> callback) {
+  Profile* profile = nullptr;
+  content::WebContents* contents = nullptr;
+  auto response = ResolveCredentialContext(target_id_, document_id,
+                                           std::nullopt, &profile, &contents);
+  if (!response.IsSuccess()) {
+    callback->sendFailure(response);
+    return;
+  }
+  if (operation_id.empty() || !contents ||
+      (credential_id ? (credential_id->empty() || username || password)
+                     : (!username || !password || password->empty()))) {
+    callback->sendFailure(CredentialError("invalid_arguments"));
+    return;
+  }
+  if (credential_operation_ && credential_operation_->pending()) {
+    callback->sendFailure(CredentialError("busy"));
+    return;
+  }
+  credential_operation_ = std::make_unique<CredentialOperation>(
+      operation_id, profile, contents,
+      CredentialCompletion(std::move(callback)));
+  credential_operation_->Start("fill", credential_id, username, password, "",
+                               50, std::nullopt);
+}
+
+Response AutofillHandler::CancelCredentialOperation(
+    const String& operation_id) {
+  if (credential_operation_ && credential_operation_->id() == operation_id &&
+      credential_operation_->pending()) {
+    credential_operation_->Cancel();
+  }
+  return Response::Success();
+}
+
 AutofillHandler::AutofillHandler(protocol::UberDispatcher* dispatcher,
                                  const std::string& target_id)
     : target_id_(target_id) {
@@ -90,8 +916,8 @@ void AutofillHandler::Trigger(
     std::unique_ptr<protocol::Autofill::Address> address,
     std::unique_ptr<TriggerCallback> callback) {
   auto host = content::DevToolsAgentHost::GetForId(target_id_);
-  if (!host) {
-    callback->sendFailure(Response::ServerError("Target not found"));
+  if (!host || !host->GetWebContents()) {
+    callback->sendFailure(Response::ServerError("Page target not found"));
     return;
   }
 
