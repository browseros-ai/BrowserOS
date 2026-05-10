// Copyright (c) 2021 The Brave Authors. All rights reserved.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this file,
// You can obtain one at https://mozilla.org/MPL/2.0/.

#ifndef BRAVE_COMPONENTS_BRAVE_SHIELDS_CONTENT_BROWSER_AD_BLOCK_ENGINE_H_
#define BRAVE_COMPONENTS_BRAVE_SHIELDS_CONTENT_BROWSER_AD_BLOCK_ENGINE_H_

#include <stdint.h>

#include <memory>
#include <optional>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "base/sequence_checker.h"
#include "base/values.h"
#include "brave/components/brave_component_updater/browser/dat_file_util.h"
#include "brave/components/brave_shields/core/common/adblock/rs/src/lib.rs.h"
#include "third_party/blink/public/mojom/loader/resource_load_info.mojom-shared.h"
#include "third_party/rust/cxx/v1/cxx.h"
#include "url/gurl.h"

using brave_component_updater::DATFileDataBuffer;

class AdBlockServiceTest;
class BraveAdBlockTPNetworkDelegateHelperTest;
class EphemeralStorage1pDomainBlockBrowserTest;
class PerfPredictorTabHelperTest;

namespace brave_shields {

// Service managing an adblock engine.
class AdBlockEngine {
 public:
  explicit AdBlockEngine(bool is_default_engine);
  AdBlockEngine(const AdBlockEngine&) = delete;
  AdBlockEngine& operator=(const AdBlockEngine&) = delete;
  ~AdBlockEngine();

  adblock::BlockerResult ShouldStartRequest(
      const GURL& url,
      blink::mojom::ResourceType resource_type,
      const std::string& tab_host,
      bool previously_matched_rule,
      bool previously_matched_exception,
      bool previously_matched_important);
  std::optional<std::string> GetCspDirectives(
      const GURL& url,
      blink::mojom::ResourceType resource_type,
      const std::string& tab_host);
  void UseResources(const adblock::BraveCoreResourceStorage& storage);
  void EnableTag(const std::string& tag, bool enabled);
  bool TagExists(const std::string& tag);

  base::DictValue GetDebugInfo();
  void DiscardRegex(uint64_t regex_id);
  void SetupDiscardPolicy(const adblock::RegexManagerDiscardPolicy& policy);

  base::DictValue UrlCosmeticResources(const std::string& url);
  base::ListValue HiddenClassIdSelectors(
      const std::vector<std::string>& classes,
      const std::vector<std::string>& ids,
      const std::vector<std::string>& exceptions);

  bool Load(bool deserialize,
            const DATFileDataBuffer& dat_buf,
            const adblock::BraveCoreResourceStorage& storage);
  bool Load(rust::Box<adblock::FilterSet> filter_set,
            const adblock::BraveCoreResourceStorage& storage);

  DATFileDataBuffer Serialize();

 protected:
  void AddKnownTagsToAdBlockInstance();
  void UpdateAdBlockClient(rust::Box<adblock::Engine> ad_block_client,
                           const adblock::BraveCoreResourceStorage& storage);

  bool OnFilterSetLoaded(rust::Box<adblock::FilterSet> filter_set,
                         const adblock::BraveCoreResourceStorage& storage);
  bool OnListSourceLoaded(const DATFileDataBuffer& filters,
                          const adblock::BraveCoreResourceStorage& storage);
  bool OnDATLoaded(const DATFileDataBuffer& dat_buf,
                   const adblock::BraveCoreResourceStorage& storage);

  rust::Box<adblock::Engine> ad_block_client_
      GUARDED_BY_CONTEXT(sequence_checker_);

 private:
  friend class ::AdBlockServiceTest;
  friend class ::BraveAdBlockTPNetworkDelegateHelperTest;
  friend class ::EphemeralStorage1pDomainBlockBrowserTest;
  friend class ::PerfPredictorTabHelperTest;

  std::set<std::string> tags_ GUARDED_BY_CONTEXT(sequence_checker_);
  std::optional<adblock::RegexManagerDiscardPolicy> regex_discard_policy_
      GUARDED_BY_CONTEXT(sequence_checker_);

  bool is_default_engine_;

  SEQUENCE_CHECKER(sequence_checker_);
};

}  // namespace brave_shields

#endif  // BRAVE_COMPONENTS_BRAVE_SHIELDS_CONTENT_BROWSER_AD_BLOCK_ENGINE_H_
