#!/usr/bin/env bash
# Builds the Neo SDK image from this checkout: the SDK server compiled from this commit and the
# browser pinned in browser.lock, with this image's own supervisor (supervisor/ beside this script).
# It only ever loads the image into the local Docker engine; it never pushes.
#
#   build.sh [--platform linux/amd64|linux/arm64] [--tag NAME]
#            [--capsolver-key-file FILE] [--twocaptcha-key-file FILE]
#
#   --platform   default: the Docker engine's own architecture. The server is cross-compiled
#                with zig either way; only the image's apt/dpkg steps run on the target arch.
#   --tag        default: neo-sdk:<browser version>-<9-char commit>[-dirty]-<arch>. All but the
#                arch is also the image's org.opencontainers.image.version label. -dirty means
#                uncommitted changes in packages/browseros-agent, which holds every input.
#                The arch keeps an amd64 build (cells) and an arm64 one (a Mac) apart.
#   --*-key-file bake a CAPTCHA provider key into the image (the neo-sdk-keyed target). Off by
#                default: the image is keyless and reads key files a host puts at the paths in
#                README.md. A baked key is readable by anyone who has the image. neo-sdk-image.yml bakes
#                both into the image it publishes, and only into that one.
#
# The image is tagged only once its server is proven to be this checkout's: the binary in the
# image must carry the digest of the sources this build copied (provenance.json source_digest),
# and its sha256 and the revision must match provenance.json. Otherwise it exits 1, untagged.
# NEO_SDK_BUILD_JOBS caps Cargo's parallel jobs; NEO_SDK_CACHE_SCOPE gives this build a Cargo
# cache of its own instead of the one shared by every checkout (for measurements).
#
# Prints TAG=, IMAGE_ID= and the image's provenance.json last, for scripts to read.
set -euo pipefail

usage() { sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

here=$(cd -- "$(dirname -- "$0")" && pwd)
agent=$(cd -- "$here/../../../.." && pwd)
# Remote jobs carry working files and explicit source metadata, without Git history.
# Require all three fields and select the declared source tree. An installed build
# driver can use a frozen older worktree without modifying its scripts; the image's
# source/binary digest checks below remain the authority for the actual input bytes.
if [ -n "${NEO_SDK_SOURCE_ROOT+x}${NEO_SDK_SOURCE_REVISION+x}${NEO_SDK_SOURCE_DIRTY+x}" ]; then
  : "${NEO_SDK_SOURCE_ROOT:?source metadata requires ROOT, REVISION and DIRTY}"
  : "${NEO_SDK_SOURCE_REVISION:?source metadata requires ROOT, REVISION and DIRTY}"
  : "${NEO_SDK_SOURCE_DIRTY:?source metadata requires ROOT, REVISION and DIRTY}"
  repo=$(cd -- "$NEO_SDK_SOURCE_ROOT" && pwd -P)
  agent=$(cd -- "$repo/packages/browseros-agent" && pwd -P)
  [ "$repo/packages/browseros-agent" = "$agent" ] \
    || { echo 'build.sh: source root must contain the actual build inputs' >&2; exit 2; }
  here=$(cd -- "$agent/crates/neo-sdk/neo-server/docker" && pwd -P)
  [[ "$NEO_SDK_SOURCE_REVISION" =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] \
    || { echo 'build.sh: source revision must be a full Git object id' >&2; exit 2; }
  case "$NEO_SDK_SOURCE_DIRTY" in true | false) ;; *) echo 'build.sh: dirty must be true or false' >&2; exit 2 ;; esac
  revision=$NEO_SDK_SOURCE_REVISION dirty=$NEO_SDK_SOURCE_DIRTY
else
  repo=$(git -C "$agent" rev-parse --show-toplevel)
  revision=$(git -C "$repo" rev-parse HEAD)
  dirty=false
  if [ -n "$(git -C "$repo" status --porcelain -- "$agent")" ]; then dirty=true; fi
fi

platform="" tag="" capsolver="" twocaptcha=""
while [ $# -gt 0 ]; do
  case "$1" in
    --platform) platform=${2:?--platform needs a value}; shift 2 ;;
    --tag) tag=${2:?--tag needs a value}; shift 2 ;;
    --capsolver-key-file) capsolver=${2:?--capsolver-key-file needs a file}; shift 2 ;;
    --twocaptcha-key-file) twocaptcha=${2:?--twocaptcha-key-file needs a file}; shift 2 ;;
    -h | --help) usage ;;
    *) echo "build.sh: unknown argument '$1'" >&2; usage ;;
  esac
done

command -v jq >/dev/null || { echo "build.sh: needs jq, to check the image's provenance" >&2; exit 2; }
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

if [ -z "$platform" ]; then
  platform="linux/$(docker version --format '{{.Server.Arch}}')"
fi
case "$platform" in
  linux/amd64 | linux/arm64) ;;
  *) echo "build.sh: --platform must be linux/amd64 or linux/arm64, got '$platform'" >&2; exit 2 ;;
esac

# shellcheck source=browser.lock
. "$here/browser.lock"
version="$NEO_BROWSER_VERSION-${revision:0:9}"
[ "$dirty" = false ] || version+="-dirty"
: "${tag:=neo-sdk:$version-${platform#linux/}}"

# The keyless target never mounts a secret, so it cannot pick a key up from a cached layer. The
# keyed one is rebuilt without its cache, so a rotated or removed key never survives either.
target=neo-sdk
key_args=()
for pair in "capsolver_api_key=$capsolver" "twocaptcha_api_key=$twocaptcha"; do
  id=${pair%%=*} file=${pair#*=}
  [ -n "$file" ] || continue
  [ -s "$file" ] || { echo "build.sh: key file '$file' is missing or empty" >&2; exit 2; }
  key_args+=(--secret "id=$id,src=$file")
  target=neo-sdk-keyed
done
[ "$target" = neo-sdk ] || key_args+=(--no-cache-filter neo-sdk-keyed)

echo "build.sh: $tag for $platform from $revision (dirty=$dirty, target $target)" >&2
iidfile=$(mktemp)
binary=$(mktemp)
trap 'rm -f "$iidfile" "$binary"' EXIT
# Older frozen worktrees use a named context for five shared supervisor files.
# The remote manifest includes those exact inputs; current images own supervisor/.
# Retain for pre-split sources such as a22477b21e. Delete this block together with
# remote.py:LEGACY_SUPERVISOR once every supported image-source worktree owns
# supervisor/ and support for pre-split snapshots is retired (remote-builds README).
context_args=()
if [ -f "$here/Dockerfile" ] && grep -q -- '--from=neo-docker' "$here/Dockerfile"; then
  neo_docker="$repo/packages/browseros/docker/neo"
  for file in neo-server-info.sh openbox-rc.xml entrypoint.sh healthcheck.sh neo-quit.py; do
    [ -f "$neo_docker/$file" ] || { echo "build.sh: missing legacy supervisor input $file" >&2; exit 2; }
  done
  context_args+=(--build-context "neo-docker=$neo_docker")
fi
# ${key_args[@]+...}: bash 3.2 (macOS) treats an empty array as unset under set -u.
docker buildx build \
  --platform "$platform" \
  --file "$here/Dockerfile" \
  --build-arg "NEO_SDK_REVISION=$revision" \
  --build-arg "NEO_SDK_DIRTY=$dirty" \
  --build-arg "NEO_SDK_VERSION=$version" \
  --build-arg "NEO_SDK_CACHE_SCOPE=${NEO_SDK_CACHE_SCOPE:-}" \
  --build-arg "NEO_SDK_BUILD_JOBS=${NEO_SDK_BUILD_JOBS:-}" \
  --target "$target" \
  ${key_args[@]+"${key_args[@]}"} \
  ${context_args[@]+"${context_args[@]}"} \
  --iidfile "$iidfile" \
  --load \
  "$agent"
image=$(cat "$iidfile")

# Read the image without starting it, which would run under emulation for a foreign arch: its
# provenance, and the server binary it actually installed.
container=$(docker create "$image")
provenance=$(docker cp "$container:/usr/share/neo-sdk/provenance.json" - | tar -xO)
docker cp "$container:$(jq -r .server.path <<<"$provenance")" - | tar -xO >"$binary"
docker rm "$container" >/dev/null

# The server must be this build's: compiled from the sources this build copied (the digest the
# server step computed), as installed, under the revision this script read. A server from Cargo's
# shared cache carries the digest of the sources it was compiled from, not these.
fail() { echo "build.sh: $*; image $image left untagged" >&2; exit 1; }
built_from=$(LC_ALL=C grep -aoE 'browseros-claw-sdk-source=[0-9a-f]{64}' "$binary" | head -1 | cut -d= -f2 || true)
[ "$(jq -r .revision <<<"$provenance")" = "$revision" ] || fail "provenance names revision $(jq -r .revision <<<"$provenance"), not $revision"
[ "$(jq -r .dirty <<<"$provenance")" = "$dirty" ] || fail "provenance says dirty=$(jq -r .dirty <<<"$provenance"), not $dirty"
[ -n "$built_from" ] || fail "the image's server carries no source digest"
[ "$built_from" = "$(jq -r .source_digest <<<"$provenance")" ] \
  || fail "the image's server was compiled from sources $built_from, not this build's $(jq -r .source_digest <<<"$provenance")"
[ "$(sha256 "$binary")" = "$(jq -r .server.sha256 <<<"$provenance")" ] \
  || fail "the image's server is not the binary provenance.json describes"

docker tag "$image" "$tag"
echo "TAG=$tag"
echo "IMAGE_ID=$image"
echo "$provenance"
