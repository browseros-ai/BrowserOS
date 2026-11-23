/// One vendored Playwright file that a part of this crate mirrors, with its sha256 when the
/// mirror was last brought level with it.
#[derive(Debug)]
pub struct PortedFile {
    /// The file's path under neo-playwright's `third_party/playwright`, as `SHA256SUMS` there
    /// names it.
    pub path: &'static str,
    /// The file's sha256, in lower-case hex, when it was last ported.
    pub sha256: &'static str,
    /// The file of this crate that mirrors it, where a change upstream makes to it is ported.
    pub mirror: &'static str,
}

/// The vendored files this crate was ported from. neo-playwright's `engine_sources` test compares
/// each hash with the vendored tree's `SHA256SUMS`, so an engine upgrade whose `sync.sh` changes
/// one of them fails there and names it. Then: read upstream's diff of that file, port what changed
/// into its mirror, regenerate the cases and the test bundles (`bun scripts/build.ts` in
/// neo-playwright-js, whose diff is what upstream now prints differently), and record the new hash
/// here.
pub const PORTED_FROM: &[PortedFile] = &[
    PortedFile {
        path: "packages/isomorphic/ariaSnapshotRenderer.ts",
        sha256: "f9fb30f2895e89b23c9bf75ccbb32100676f834b648785ee629c0065b4611d20",
        mirror: "src/playwright/node_text.rs",
    },
    PortedFile {
        path: "packages/isomorphic/yaml.ts",
        sha256: "805b5de857749599fcf979b1e00b35ad89beb807c189e163fd264a891110dd29",
        mirror: "src/playwright/yaml_escape.rs",
    },
    // The tree's shape and the page's writer of it: a change to either changes what `AriaNode`
    // has to read.
    PortedFile {
        path: "packages/isomorphic/ariaSnapshot.ts",
        sha256: "8a6d5d90933b786bcbf6407359702257ca8d499eec568215830afc9fd25e1fec",
        mirror: "src/aria_node.rs",
    },
    PortedFile {
        path: "packages/injected/src/ariaSnapshot.ts",
        sha256: "7caadae854a544c5f0da7bdb7392b0b8962bb67c40bf906c755cc52b3ec262d7",
        mirror: "src/aria_node.rs",
    },
];
