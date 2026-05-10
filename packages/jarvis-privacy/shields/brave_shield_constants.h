// Copyright (c) 2019 The Brave Authors. All rights reserved.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this file,
// You can obtain one at https://mozilla.org/MPL/2.0/.

#ifndef BRAVE_COMPONENTS_BRAVE_SHIELDS_CORE_COMMON_BRAVE_SHIELD_CONSTANTS_H_
#define BRAVE_COMPONENTS_BRAVE_SHIELDS_CORE_COMMON_BRAVE_SHIELD_CONSTANTS_H_

#include "base/containers/fixed_flat_map.h"
#include "base/containers/fixed_flat_set.h"
#include "base/containers/map_util.h"
#include "base/files/file_path.h"
#include "components/content_settings/core/common/content_settings_types.h"

namespace brave_shields {

// Content/Web settings:
inline constexpr char kAds[] = "shieldsAds";
inline constexpr char kCosmeticFiltering[] = "cosmeticFilteringV2";
inline constexpr char kTrackers[] = "trackers";
inline constexpr char kHTTPUpgradableResources[] = "httpUpgradableResources";
inline constexpr char kHTTPSUpgrades[] = "httpsUpgrades";
inline constexpr char kJavaScript[] = "javascript";
inline constexpr char kFingerprintingV2[] = "fingerprintingV2";
inline constexpr char kBraveShields[] = "braveShields";
inline constexpr char kBraveShieldsMetadata[] = "braveShieldsMetadata";
inline constexpr char kReferrers[] = "referrers";
inline constexpr char kCookies[] = "shieldsCookiesV3";
inline constexpr char kBraveAutoShred[] = "braveAutoShred";

// Prefs:
inline constexpr char kFacebookEmbeds[] = "fb-embeds";
inline constexpr char kTwitterEmbeds[] = "twitter-embeds";
inline constexpr char kLinkedInEmbeds[] = "linked-in-embeds";

inline constexpr auto kShieldsContentSettingsTypes =
    base::MakeFixedFlatSet<ContentSettingsType>({
        ContentSettingsType::BRAVE_ADS,
        ContentSettingsType::BRAVE_COSMETIC_FILTERING,
        ContentSettingsType::BRAVE_TRACKERS,
        ContentSettingsType::BRAVE_HTTP_UPGRADABLE_RESOURCES,
        ContentSettingsType::BRAVE_FINGERPRINTING_V2,
        ContentSettingsType::BRAVE_SHIELDS,
        ContentSettingsType::BRAVE_REFERRERS,
        ContentSettingsType::BRAVE_COOKIES,
        ContentSettingsType::BRAVE_AUTO_SHRED,
    });

using ShieldsContentSettingsTypes = decltype(kShieldsContentSettingsTypes);

inline constexpr auto kShieldsContentTypeNames =
    base::MakeFixedFlatMap<ContentSettingsType, const char*>({
        {ContentSettingsType::BRAVE_ADS, kAds},
        {ContentSettingsType::BRAVE_COSMETIC_FILTERING, kCosmeticFiltering},
        {ContentSettingsType::BRAVE_TRACKERS, kTrackers},
        {ContentSettingsType::BRAVE_HTTP_UPGRADABLE_RESOURCES,
         kHTTPUpgradableResources},
        {ContentSettingsType::BRAVE_HTTPS_UPGRADE, kHTTPSUpgrades},
        {ContentSettingsType::JAVASCRIPT, kJavaScript},
        {ContentSettingsType::BRAVE_FINGERPRINTING_V2, kFingerprintingV2},
        {ContentSettingsType::BRAVE_SHIELDS, kBraveShields},
        {ContentSettingsType::BRAVE_SHIELDS_METADATA, kBraveShieldsMetadata},
        {ContentSettingsType::BRAVE_REFERRERS, kReferrers},
        {ContentSettingsType::BRAVE_COOKIES, kCookies},
        {ContentSettingsType::BRAVE_AUTO_SHRED, kBraveAutoShred},
    });

using ShieldsContentTypeNames = decltype(kShieldsContentTypeNames);

namespace internal {
consteval bool CheckShieldsContentTypeNames() {
  for (const auto& [key_a, value_a] : kShieldsContentTypeNames) {
    for (const auto& [key_b, value_b] : kShieldsContentTypeNames) {
      if (key_a == key_b) {
        continue;
      }
      if (!value_a || !value_b || value_a == value_b) {
        // Name is null or not unique.
        return false;
      }
    }
  }
  return true;
}

static_assert(
    CheckShieldsContentTypeNames(),
    "Invalid kShieldsContentTypeNames. Name should be unique and non-null.");
}  // namespace internal

// Values used before the migration away from ResourceIdentifier, kept
// around for migration purposes only.
inline constexpr char kObsoleteAds[] = "ads";
inline constexpr char kObsoleteCookies[] = "cookies";
inline constexpr char kObsoleteShieldsCookies[] = "shieldsCookies";
inline constexpr char kObsoleteCosmeticFiltering[] = "cosmeticFiltering";

// Some users were not properly migrated from fingerprinting V1.
inline constexpr char kObsoleteFingerprinting[] = "fingerprinting";

// Key for procedural and action filters in the UrlCosmeticResources struct from
// adblock-rust
inline constexpr char kCosmeticResourcesProceduralActions[] =
    "procedural_actions";

// Filename for cached text from a custom filter list subscription
const base::FilePath::CharType kCustomSubscriptionListText[] =
    FILE_PATH_LITERAL("list_text.txt");

inline constexpr char kDefaultAdblockFiltersListUuid[] = "default";
inline constexpr char kFirstPartyAdblockFiltersListUuid[] =
    "E99CBD02-FFD1-4651-9BDD-6A9ED7B87819";
inline constexpr char kCookieListUuid[] =
    "AC023D22-AE88-4060-A978-4FEEEC4221693";
inline constexpr char kMobileNotificationsListUuid[] =
    "2F3DCE16-A19A-493C-A88F-2E110FBD37D6";
inline constexpr char kExperimentalListUuid[] =
    "564C3B75-8731-404C-AD7C-5683258BA0B0";
inline constexpr char kAdblockOnlySupplementalListUuid[] =
    "61C6046C-B040-4CE4-8C8E-F3E8F316AA72";

inline constexpr char kAdBlockResourceComponentName[] =
    "Brave Ad Block Resources Library";
inline constexpr char kAdBlockResourceComponentId[] =
    "mfddibmblmbccpadfndgakiopmmhebop";
inline constexpr char kAdBlockResourceComponentBase64PublicKey[] =
    "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA7Qk6xtml8Siq8RD6cCbd"
    "JpArt0kMci82W/KYw3KR96y67MZAsKJa8rOV2WC1BIpW539Qgl5b5lMS04cjw+sS"
    "B7f2ZKM1WOqKNij24nvEKVubunP32u8tbjtzQk9VYNcM2MZMs330eqk7iuBRTvRV"
    "iSMSeE3ymqp03HFpUGsdtjEBh1A5lroCg41eVnMn1I4GKPvuhT/Qc9Yem5gzXT/3"
    "n7H6vOGQ2dVBHz44mhgwtiDcsduh+Det6lCE2TgHOhHPdCewklgcoiNXP4zfXxfp"
    "Py1jbwb4w5KUnHSRelhfDnt+jI3jgHsD4IXdVNE5H5ZAnmcOJttbkRiT8kOVS0rJ"
    "XwIDAQAB";

inline constexpr char kAdBlockFilterListCatalogComponentName[] =
    "Brave Ad Block List Catalog";
inline constexpr char kAdBlockFilterListCatalogComponentId[] =
    "gkboaolpopklhgplhaaiboijnklogmbc";
inline constexpr char kAdBlockFilterListCatalogComponentBase64PublicKey[] =
    "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsAnb1lw5UA1Ww4JIVE8P"
    "jKNlPogAdFoie+Aczk6ppQ4OrHANxz6oAk1xFuT2W3uhGOc3b/1ydIUMqOIdRFvM"
    "dEDUvKVeFyNAVXNSouFF7EBLEzcZfFtqoxeIbwEplVISUm+WUbsdVB9MInY3a4O3"
    "kNNuUijY7bmHzAqWMTrBfenw0Lqv38OfREXCiNq/+Jm/gt7FhyBd2oviXWEGp6as"
    "UwNavFnj8gQDGVvCf+dse8HRMJn00QH0MOypsZSWFZRmF08ybOu/jTiUo/TuIaHL"
    "1H8y9SR970LqsUMozu3ioSHtFh/IVgq7Nqy4TljaKsTE+3AdtjiOyHpW9ZaOkA7j"
    "2QIDAQAB";

inline constexpr char kCookieListEnabledHistogram[] =
    "Brave.Shields.CookieListEnabled";
inline constexpr char kCookieListPromptHistogram[] =
    "Brave.Shields.CookieListPrompt";

// The list of UUIDs of filter lists that will be loaded by AdBlockOnlyMode.
inline constexpr auto kAdblockOnlyModeFilterListUUIDs =
    base::MakeFixedFlatSet<std::string_view>(
        {kDefaultAdblockFiltersListUuid, kFirstPartyAdblockFiltersListUuid,
         kAdblockOnlySupplementalListUuid});

// The list of language codes that are supported by Ad Block Only mode.
inline constexpr auto kAdblockOnlyModeSupportedLanguageCodes =
    base::MakeFixedFlatSet<std::string_view>({"en"});

}  // namespace brave_shields

#endif  // BRAVE_COMPONENTS_BRAVE_SHIELDS_CORE_COMMON_BRAVE_SHIELD_CONSTANTS_H_
