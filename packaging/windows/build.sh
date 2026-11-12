#!/usr/bin/env bash
# Cross-build anamnesis for windows x86_64 on linux (the desktop app
# anamnesis-desktop.exe, a native window on webview2, and the command line anamnesis.exe
# for mcp and headless use) and package it three ways:
#
#   anamnesis-<version>-windows-x86_64-setup.exe   nsis installer (per-user, no admin)
#   anamnesis-<version>-windows-x86_64.msi         msi (wixl, per-user)
#   anamnesis-<version>-windows-x86_64-portable.zip  portable folder
#
# each with a .sha256 file. the binary targets x86_64-pc-windows-gnu (mingw-w64) and loads
# microsoft's onnxruntime.dll at run time (feature local-embed-dynamic), so no windows sdk
# or msvc is needed; the dll comes from the official microsoft/onnxruntime github release,
# pinned by version and sha256 below.
#
# needs: rustup (or a toolchain with the x86_64-pc-windows-gnu target), mingw-w64, nsis,
# wixl (msitools), zip, unzip, curl, sha256sum. no windows sdk, msvc or xwin. on debian /
# ubuntu:
#   sudo apt-get install -y mingw-w64 nsis wixl msitools zip unzip curl
#
# usage: packaging/windows/build.sh [--version X.Y.Z] [--out DIR] [--skip-build] [--icon]
#   --version     version in the file names (default: the workspace version)
#   --out         where the packages go (default: dist/windows)
#   --skip-build  package the two .exe files built earlier
#   --icon        regenerate anamnesis.ico and wizard.bmp from the phi mark (needs
#                 rsvg-convert, convert)
# environment: CARGO_TARGET_DIR (default: target), CARGO_BUILD_JOBS, WORK (download and
# staging folder, default: $CARGO_TARGET_DIR/winpkg).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
TARGET=x86_64-pc-windows-gnu
FEATURES=embed-ui,embed-demo,local-embed-dynamic
DESKTOP_FEATURES=local-embed-dynamic

# onnx runtime: the version ort 2.0.0-rc.15 is built against (any 1.24+ loads)
ORT_VERSION=1.31.0
ORT_SHA256=3958d8a44984160692c1b4ffd0de3cb48f8c39f9d34bb31e433db06e30b82195
ORT_URL="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-win-x64-${ORT_VERSION}.zip"

VERSION=""
OUT="$ROOT/dist/windows"
SKIP_BUILD=0
ICON=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --icon) ICON=1; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' "$ROOT/Cargo.toml" | head -1)"
fi
VERSION="${VERSION#v}"
# msi and the nsis version resource take only numbers: 0.1.0-rc.1 → 0.1.0
NUMERIC_VERSION="$(echo "$VERSION" | grep -oE '^[0-9]+(\.[0-9]+){0,2}')"
[[ -n "$NUMERIC_VERSION" ]] || { echo "version $VERSION does not start with a number" >&2; exit 2; }
while [[ "$(echo "$NUMERIC_VERSION" | tr -cd . | wc -c)" -lt 2 ]]; do NUMERIC_VERSION="$NUMERIC_VERSION.0"; done

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
WORK="${WORK:-$CARGO_TARGET_DIR/winpkg}"
NAME="anamnesis-${VERSION}-windows-x86_64"
STAGE="$WORK/stage/$NAME"

need() { command -v "$1" > /dev/null || { echo "missing tool: $1 ($2)" >&2; exit 1; }; }
need x86_64-w64-mingw32-gcc "apt-get install mingw-w64"
need makensis "apt-get install nsis"
need wixl "apt-get install wixl"
need zip "apt-get install zip"
need unzip "apt-get install unzip"
need curl "apt-get install curl"
need sha256sum "coreutils"

log() { printf '\n== %s\n' "$*"; }

# ---------------------------------------------------------------- icon
if [[ "$ICON" == 1 ]]; then
  need rsvg-convert "apt-get install librsvg2-bin"
  need convert "apt-get install imagemagick"
  log "icon"
  tmp="$(mktemp -d)"
  pngs=()
  for s in 16 24 32 48 64 128 256; do
    rsvg-convert -h $((s * 94 / 100)) "$ROOT/ui/theme/themes/aphesis/assets/phi.svg" -o "$tmp/raw$s.png"
    convert "$tmp/raw$s.png" -background none -gravity center -extent "${s}x${s}" "$tmp/p$s.png"
    pngs+=("$tmp/p$s.png")
  done
  convert "${pngs[@]}" "$HERE/anamnesis.ico"
  # the installer's welcome / finish panel: the phi on the dark theme's ground
  # (--bg oklch(0.11 0.004 25) ≈ #050404)
  rsvg-convert -h 120 "$ROOT/ui/theme/themes/aphesis/assets/phi.svg" -o "$tmp/phi120.png"
  convert -size 164x314 xc:'#050404' "$tmp/phi120.png" -gravity center -geometry +0-30 \
    -composite -type truecolor BMP3:"$HERE/wizard.bmp"
  rm -rf "$tmp"
fi

# ---------------------------------------------------------------- build
EXE="$CARGO_TARGET_DIR/$TARGET/release/anamnesis.exe"
DESKTOP_EXE="$CARGO_TARGET_DIR/$TARGET/release/anamnesis-desktop.exe"
if [[ "$SKIP_BUILD" == 0 ]]; then
  log "cargo build ($TARGET, $FEATURES)"
  if command -v rustup > /dev/null; then rustup target add "$TARGET" > /dev/null; fi
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
  (cd "$ROOT" && cargo build --release --locked -p anamnesis-server --target "$TARGET" \
    --no-default-features --features "$FEATURES")
  log "cargo build anamnesis-desktop ($TARGET, $DESKTOP_FEATURES)"
  (cd "$ROOT" && cargo build --release --locked -p anamnesis-desktop --target "$TARGET" \
    --no-default-features --features "$DESKTOP_FEATURES")
fi
[[ -f "$EXE" ]] || { echo "no $EXE; build first" >&2; exit 1; }
[[ -f "$DESKTOP_EXE" ]] || { echo "no $DESKTOP_EXE; build first" >&2; exit 1; }

# ---------------------------------------------------------------- onnx runtime
log "onnx runtime $ORT_VERSION"
mkdir -p "$WORK/download"
ORT_ZIP="$WORK/download/onnxruntime-win-x64-${ORT_VERSION}.zip"
if [[ ! -f "$ORT_ZIP" ]] || ! echo "$ORT_SHA256  $ORT_ZIP" | sha256sum -c --quiet - 2> /dev/null; then
  curl -fsSL --retry 3 -o "$ORT_ZIP.part" "$ORT_URL"
  mv "$ORT_ZIP.part" "$ORT_ZIP"
fi
echo "$ORT_SHA256  $ORT_ZIP" | sha256sum -c --quiet - || { echo "onnx runtime zip: sha256 mismatch" >&2; exit 1; }
ORT_DIR="$WORK/ort"
rm -rf "$ORT_DIR"
mkdir -p "$ORT_DIR"
# only what is shipped (the zip also carries a 430 mb pdb)
p="onnxruntime-win-x64-${ORT_VERSION}"
unzip -q -o "$ORT_ZIP" -d "$ORT_DIR" "$p/lib/onnxruntime.dll" "$p/lib/onnxruntime_providers_shared.dll" \
  "$p/LICENSE" "$p/ThirdPartyNotices.txt" "$p/Privacy.md"
ORT_DIR="$ORT_DIR/$p"

# ---------------------------------------------------------------- stage
log "stage $STAGE"
rm -rf "$WORK/stage"
mkdir -p "$STAGE/licenses"
crlf() { sed 's/\r$//; s/$/\r/' "$1" > "$2"; }
cp "$EXE" "$STAGE/anamnesis.exe"
cp "$DESKTOP_EXE" "$STAGE/anamnesis-desktop.exe"
x86_64-w64-mingw32-strip "$STAGE/anamnesis.exe" "$STAGE/anamnesis-desktop.exe" 2> /dev/null || true
cp "$ORT_DIR/lib/onnxruntime.dll" "$ORT_DIR/lib/onnxruntime_providers_shared.dll" "$STAGE/"
cp "$HERE/anamnesis.ico" "$STAGE/"
crlf "$ROOT/LICENSE" "$STAGE/LICENSE.txt"
crlf "$ROOT/NOTICE" "$STAGE/NOTICE.txt"
sed "s/@VERSION@/$VERSION/; s/@ORT_VERSION@/$ORT_VERSION/" "$HERE/README.txt" > "$WORK/README.txt"
crlf "$WORK/README.txt" "$STAGE/README.txt"
crlf "$ORT_DIR/LICENSE" "$STAGE/licenses/onnxruntime-LICENSE.txt"
crlf "$ORT_DIR/ThirdPartyNotices.txt" "$STAGE/licenses/onnxruntime-ThirdPartyNotices.txt"
crlf "$ORT_DIR/Privacy.md" "$STAGE/licenses/onnxruntime-Privacy.md"
crlf "$ROOT/ui/theme/themes/aphesis/assets/OFL.txt" "$STAGE/licenses/aphesis-OFL.txt"
crlf "$ROOT/ui/theme/themes/aphesis/assets/NOTICE.md" "$STAGE/licenses/aphesis-NOTICE.md"
crlf "$ROOT/ui/theme/fonts/LICENSE-SpecialElite.txt" "$STAGE/licenses/SpecialElite-LICENSE.txt"
crlf "$ROOT/ui/theme/fonts/NOTICE.md" "$STAGE/licenses/fonts-NOTICE.md"
# the mingw-w64 runtime (winpthreads, crt startup) is linked into the exe statically
MINGW_COPYRIGHT=/usr/share/doc/mingw-w64-common/copyright
[[ -f "$MINGW_COPYRIGHT" ]] || { echo "missing $MINGW_COPYRIGHT (mingw-w64-common)" >&2; exit 1; }
crlf "$MINGW_COPYRIGHT" "$STAGE/licenses/mingw-w64-runtime-copyright.txt"

# the licence page of both installers: LICENSE, then NOTICE (not installed itself)
PAGE="$WORK/license-page.txt"
{ cat "$ROOT/LICENSE"; printf '\n\n'; cat "$ROOT/NOTICE"; } > "$PAGE"
crlf "$PAGE" "$WORK/LICENSE-page.txt"
# the same as rtf for the msi's licence dialog (both files are plain ascii)
{
  printf '{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0\\fmodern Consolas;}}\\fs16\n'
  sed 's/\\/\\\\/g; s/{/\\{/g; s/}/\\}/g; s/$/\\par/' "$PAGE"
  printf '}\n'
} > "$WORK/LICENSE.rtf"

INSTALLED_KB="$(du -sk "$STAGE" | cut -f1)"

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

# ---------------------------------------------------------------- nsis
log "nsis"
NSIS_STAGE="$WORK/nsis"
rm -rf "$NSIS_STAGE"
cp -r "$STAGE" "$NSIS_STAGE"
cp "$WORK/LICENSE-page.txt" "$NSIS_STAGE/LICENSE.txt.page"
SETUP="$OUT/$NAME-setup.exe"
# the licence page shows LICENSE + NOTICE; the installed LICENSE.txt stays the plain licence
makensis -V2 -INPUTCHARSET UTF8 \
  -DVERSION="$NUMERIC_VERSION" -DDISPLAY_VERSION="$VERSION" -DSTAGE="$NSIS_STAGE" \
  -DLICENSE_PAGE="$NSIS_STAGE/LICENSE.txt.page" -DINSTALLED_KB="$INSTALLED_KB" \
  -DOUTFILE="$SETUP" "$HERE/anamnesis.nsi"

# ---------------------------------------------------------------- msi
log "msi"
MSI_STAGE="$WORK/msi"
rm -rf "$MSI_STAGE"
cp -r "$STAGE" "$MSI_STAGE"
cp "$WORK/LICENSE.rtf" "$MSI_STAGE/License.rtf"
MSI="$OUT/$NAME.msi"
(cd "$MSI_STAGE" && wixl --arch x64 --ext ui -D Version="$NUMERIC_VERSION" -D Stage="$MSI_STAGE" -o "$MSI" "$HERE/anamnesis.wxs")

# ---------------------------------------------------------------- zip
log "zip"
ZIP="$OUT/$NAME-portable.zip"
rm -f "$ZIP"
(cd "$WORK/stage" && zip -q -r -9 -X "$ZIP" "$NAME")

# ---------------------------------------------------------------- checksums
log "checksums"
for f in "$SETUP" "$MSI" "$ZIP"; do
  (cd "$OUT" && sha256sum "$(basename "$f")" > "$(basename "$f").sha256")
done
ls -l "$OUT"
cat "$OUT"/*.sha256
