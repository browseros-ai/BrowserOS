#!/usr/bin/env bash
# Builds the Neo SDK image from this checkout: the SDK server compiled from this commit and the
# browser pinned in browser.lock, on top of the supervisor in packages/browseros/docker/neo.
# It only ever loads the image into the local Docker engine; it never pushes.
#
#   build.sh [--platform linux/amd64|linux/arm64] [--tag NAME]
#            [--capsolver-key-file FILE] [--twocaptcha-key-file FILE]
#
#   --platform   default: the Docker engine's own architecture. The server is cross-compiled
#                with zig either way; only the image's apt/dpkg steps run on the target arch.
#   --tag        default: neo-sdk:<browser version>-<9-char commit>[-dirty]-<arch>. All but the
#                arch is also the image's org.opencontainers.image.version label. -dirty means
#                uncommitted changes in packages/browseros-agent or packages/browseros/docker/neo.
#                The arch keeps an amd64 build (cells) and an arm64 one (a Mac) apart.
#   --*-key-file bake a CAPTCHA provider key into the image (the neo-sdk-keyed target). Off by
#                default: the image is keyless and reads key files a host puts at the paths in
#                README.md. A baked key is readable by anyone who has the image.
#
# Prints TAG=, IMAGE_ID= and the image's provenance.json last, for scripts to read.
set -euo pipefail

usage() { sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

here=$(cd -- "$(dirname -- "$0")" && pwd)
agent=$(cd -- "$here/../../../.." && pwd)
repo=$(git -C "$agent" rev-parse --show-toplevel)
neo_docker="$repo/packages/browseros/docker/neo"

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

if [ -z "$platform" ]; then
  platform="linux/$(docker version --format '{{.Server.Arch}}')"
fi
case "$platform" in
  linux/amd64 | linux/arm64) ;;
  *) echo "build.sh: --platform must be linux/amd64 or linux/arm64, got '$platform'" >&2; exit 2 ;;
esac

# shellcheck source=browser.lock
. "$here/browser.lock"
revision=$(git -C "$repo" rev-parse HEAD)
dirty=false
if [ -n "$(git -C "$repo" status --porcelain -- "$agent" "$neo_docker")" ]; then dirty=true; fi
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
trap 'rm -f "$iidfile"' EXIT
# ${key_args[@]+...}: bash 3.2 (macOS) treats an empty array as unset under set -u.
docker buildx build \
  --platform "$platform" \
  --file "$here/Dockerfile" \
  --build-context "neo-docker=$neo_docker" \
  --build-arg "NEO_SDK_REVISION=$revision" \
  --build-arg "NEO_SDK_DIRTY=$dirty" \
  --build-arg "NEO_SDK_VERSION=$version" \
  --target "$target" \
  ${key_args[@]+"${key_args[@]}"} \
  --iidfile "$iidfile" \
  --load \
  --tag "$tag" \
  "$agent"

# Read provenance without starting the image, which would run under emulation for a foreign arch.
container=$(docker create "$tag")
provenance=$(docker cp "$container:/usr/share/neo-sdk/provenance.json" - | tar -xO)
docker rm "$container" >/dev/null
echo "TAG=$tag"
echo "IMAGE_ID=$(cat "$iidfile")"
echo "$provenance"
