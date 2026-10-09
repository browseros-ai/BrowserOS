# Azure Windows server signing trial

The trial uses Azure Artifact Signing **Basic / Public Trust** for Windows
servers. Browser executables and installers remain on SSL.com EV. Azure is
not EV. Its USD 9.99 monthly base includes 5,000 signatures; extra signatures
cost USD 0.005 each. A paid Azure subscription is required: Sponsorship is
unsupported. Keep the existing SSL.com account, certificate and secrets.

## Release controls

| Mode | `windows_signing_provider` | `windows_server_signing_provider` |
|---|---|---|
| Existing behavior / rollback | `sslcom` | `inherit` |
| Server trial in a full browser release | `sslcom` | `azure` |
| Future all-Azure experiment | `azure` | `inherit` |

Full BrowserOS and BrowserClaw release workflows forward the server choice
to **both** server OTA and the server embedded in the installer. Standalone
server workflows accept just `windows_signing_provider`; set it to `azure`.
Omitted inputs and nightly callers retain SSL.com. A provider error stops
the run; it never silently falls back. `sign=false` still means unsigned.

For local tooling use `WINDOWS_SIGNING_PROVIDER` and
`WINDOWS_SERVER_SIGNING_PROVIDER` with the same values. Local Azure signing
requires Windows and `AZURE_SIGNING_AUTH=azure-cli`; CI always uses GitHub
OIDC. Run a signed stage again after changing providers. Compile checkpoints
can be reused, but a completed signing checkpoint binds both identities.

## Azure prerequisites

1. Use a separate paid subscription in the intended tenant. Do not upgrade
   or replace the Sponsorship subscription.
2. Register `Microsoft.CodeSigning`, create one **Basic** account in a
   supported region, and complete organization identity validation for the
   legal entity. Create one **PublicTrust** profile after approval. Test and
   Private Trust profiles are unsuitable for the Windows trust trial.
3. Create a dedicated Entra application/service principal with no client
   secret. Add GitHub federation for issuer
   `https://token.actions.githubusercontent.com`, audience
   `api://AzureADTokenExchange`, and the exact repository/branch subject.
   Confirm the repository's actual OIDC subject configuration first.
4. Assign **Artifact Signing Certificate Profile Signer** only at the new
   certificate profile scope. The onboarding operator needs the separate
   identity-verifier role; Owner alone is not a signing grant.
5. Configure the following repository **variables**, which are public
   identifiers rather than credentials. Keep the profile stable; do not pin
   an individual certificate thumbprint, because certificates rotate.

| Variable | Value |
|---|---|
| `AZURE_TENANT_ID` | Entra tenant containing the service principal |
| `AZURE_SIGNING_CLIENT_ID` | Dedicated application's client ID |
| `AZURE_SIGNING_ENDPOINT` | Account's regional HTTPS endpoint |
| `AZURE_SIGNING_ACCOUNT` | Basic account name |
| `AZURE_SIGNING_PROFILE` | Public Trust profile name |
| `AZURE_SIGNING_PROFILE_OID` | Profile's unique EnhancedKeyUsage OID from Azure profile properties |

The Python signer consumes `AZURE_CLIENT_ID`; workflows map the repository
variable to it. The expected publisher is `Felafax, Inc.` through
`WINDOWS_SIGNING_PUBLISHER`. Verify this matches Microsoft's validated
certificate common name before enabling the trial; change it deliberately
if the approved legal name differs.

GitHub callers pass `id-token: write` through the nested Windows/server
release workflows. The signer fetches a fresh assertion immediately before
each batch, rather than logging in before a potentially long Chromium build.
Dlib reads a private temporary token file; the token is removed with its
staging directory. Azure CLI and other credential sources are disabled in CI.
Pinned x64 SDK/Dlib packages are checked by SHA-256, and .NET 8 and the VC++
runtime are checked before the build.

## Private trial before publication

The existing server workflows expose `windows_signing_trial=true`, so the
trial can run from a reviewed feature branch. That mode skips release
preparation/finalization and calls `trial-windows-server-signing.yml`.
It resolves the existing resource version once, checks its source binding
and SHA-256, signs a copy, and retains the resulting ZIP and JSON evidence
as GitHub Actions artifacts. It does not allocate versions, write R2
objects, publish releases, update feeds, or execute the server.

```bash
gh workflow run release-server.yml --ref feat/azure-windows-signing \
  -f windows_signing_trial=true -f windows_signing_provider=azure \
  -f publish_ota=false

gh workflow run release-claw-server.yml --ref feat/azure-windows-signing \
  -f windows_signing_trial=true -f windows_signing_provider=azure \
  -f publish_ota=false
```

An optional `version` pins a specific existing resource version. Use a new
workflow run for each trial: signing evidence records artifact hashes,
source SHA, provider/profile, issuer, publisher, timestamp certificate and
verification results. Azure verification also checks the profile EKU and
SignTool's chain/timestamp verification. No successful signing report is
proof of SmartScreen reputation.

After private signing succeeds, test normally downloaded artifacts on clean
supported Windows machines with default security and internet-origin
metadata intact. Check Authenticode, UAC publisher, SmartScreen, Defender,
and Smart App Control; launch the server and verify health. Then test a mixed
browser build, including the server extracted from the installer. Prove
current SSL release → Azure server A → Azure server B → SSL rollback C with
fresh increasing versions and dedicated client profiles. Preserve the
existing Sparkle/Ed25519 update-signing keys throughout.

Only then dispatch a limited alpha release with the Azure provider selected.
Full release workflows publish server alpha updates, so they are not private
smoke tests. Production promotion remains a separate release operation.

## Rollback and immutable payloads

Select SSL.com for standalone servers; select `sslcom` + `inherit` for full
releases. Build a higher version and follow the existing publication path.
Do not rewrite an existing R2 payload. Azure same-version live-feed reuse is
rejected, and canonical Windows payloads bind the provider identity so a
race cannot quietly reuse another provider's output. Legacy SSL payloads
remain compatible with existing SSL reuse behavior.

Disabling Azure affects future signatures. It does not alter binaries
already installed. Once the trial is retired, remove the feature-branch
federated credential; keep the main-branch credential only if Azure stays
in use. Cancelling the Basic account is a separate billing action.

## Sources

- [Azure pricing](https://azure.microsoft.com/en-us/pricing/details/artifact-signing/)
- [EV and subscription restrictions](https://learn.microsoft.com/en-us/azure/artifact-signing/faq)
- [Account and identity setup](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)
- [SignTool/Dlib integration](https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-signing-integrations)
- [Certificate lifecycle and profile identity](https://learn.microsoft.com/en-us/azure/artifact-signing/concept-certificate-management)
- [Windows SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
