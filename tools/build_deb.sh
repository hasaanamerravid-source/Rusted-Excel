#!/usr/bin/env bash
# Build a Debian binary package for Rusted Excel without sudo.
set -euo pipefail
IFS=$'\n\t'

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)

version=""
target_dir="/home/rha/rusted-excel-target"
out_dir="/home/rha"
skip_build=0

arch="amd64"
maintainer="Hasaan Amer <hasaanamerravid-source@users.noreply.github.com>"
homepage="https://github.com/hasaanamerravid-source/Rusted-Excel"
icon_sizes=(16 22 24 32 48 64 128 256)

usage() {
    cat <<EOF
Usage: $(basename -- "$0") [OPTIONS]

Build a Debian package for Rusted Excel into OUT-DIR.

Options:
  --version VERSION    Upstream version to package (default: Cargo.toml version).
                       A Debian revision is added when none is present.
  --target-dir DIR     Cargo target directory (default: /home/rha/rusted-excel-target).
  --out-dir DIR        Directory for the finished .deb (default: /home/rha).
  --skip-build         Reuse the binary in TARGET-DIR/release instead of building.
  -h, --help           Show this help and exit.
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

while (($#)); do
    case "$1" in
        --version)
            [[ $# -ge 2 ]] || die "--version requires a value"
            version=$2
            shift 2
            ;;
        --version=*)
            version=${1#*=}
            shift
            ;;
        --target-dir)
            [[ $# -ge 2 ]] || die "--target-dir requires a value"
            target_dir=$2
            shift 2
            ;;
        --target-dir=*)
            target_dir=${1#*=}
            shift
            ;;
        --out-dir)
            [[ $# -ge 2 ]] || die "--out-dir requires a value"
            out_dir=$2
            shift 2
            ;;
        --out-dir=*)
            out_dir=${1#*=}
            shift
            ;;
        --skip-build)
            skip_build=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1 (try --help)"
            ;;
    esac
done

[[ -n $version ]] || version=$(awk -F'"' '
    /^\[package\]/ { in_package = 1; next }
    /^\[/          { in_package = 0 }
    in_package && /^version[[:space:]]*=/ { print $2; exit }
' "$repo_root/Cargo.toml")
[[ -n $version ]] || die "could not determine version from Cargo.toml"

[[ -n $target_dir ]] || die "--target-dir must not be empty"
[[ -n $out_dir ]] || die "--out-dir must not be empty"

# Debian package version: add the initial revision unless one was supplied.
if [[ $version == *-* ]]; then
    deb_version=$version
else
    deb_version="${version}-1"
fi

mkdir -p -- "$target_dir" "$out_dir"
target_dir=$(cd -- "$target_dir" && pwd -P)
out_dir=$(cd -- "$out_dir" && pwd -P)

for tool in install gzip du awk sed fakeroot dpkg-deb dpkg-shlibdeps; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool not found: $tool"
done
if ((skip_build == 0)); then
    command -v cargo >/dev/null 2>&1 || die "required tool not found: cargo"
fi

binary="$target_dir/release/rusted-excel"
if ((skip_build == 0)); then
    printf '==> Building release binary (CARGO_TARGET_DIR=%s)\n' "$target_dir"
    CARGO_TARGET_DIR="$target_dir" cargo build --release --manifest-path "$repo_root/Cargo.toml"
fi
[[ -f $binary && -x $binary ]] || die "binary not found or not executable: $binary"

work=$(mktemp -d "$out_dir/.rusted-excel-deb.XXXXXX")
cleanup() {
    rm -rf -- "$work"
}
trap cleanup EXIT
stage="$work/stage"
mkdir -p -- "$stage/DEBIAN" "$work/debian"

printf '==> Staging package files\n'
install -D -m 0755 -- "$binary" \
    "$stage/usr/bin/rusted-excel"
desktop_id="io.github.hasaanamerravid.RustedExcel.desktop"
install -D -m 0644 -- "$repo_root/packaging/$desktop_id" \
    "$stage/usr/share/applications/$desktop_id"
install -D -m 0644 -- "$repo_root/packaging/rusted-excel.metainfo.xml" \
    "$stage/usr/share/metainfo/io.github.hasaanamerravid.RustedExcel.metainfo.xml"
install -D -m 0644 -- "$repo_root/packaging/copyright" \
    "$stage/usr/share/doc/rusted-excel/copyright"
install -d -m 0755 -- "$stage/usr/share/man/man1"
gzip -n -9 -c -- "$repo_root/packaging/rusted-excel.1" \
    > "$stage/usr/share/man/man1/rusted-excel.1.gz"
chmod 0644 -- "$stage/usr/share/man/man1/rusted-excel.1.gz"
gzip -n -9 -c -- "$repo_root/packaging/changelog" \
    > "$stage/usr/share/doc/rusted-excel/changelog.gz"
chmod 0644 -- "$stage/usr/share/doc/rusted-excel/changelog.gz"
install -D -m 0644 -- "$repo_root/packaging/icons/hicolor/48/apps/rusted-excel.png" \
    "$stage/usr/share/pixmaps/rusted-excel.png"

for size in "${icon_sizes[@]}"; do
    icon="$repo_root/packaging/icons/hicolor/$size/apps/rusted-excel.png"
    [[ -f $icon ]] || die "missing icon $icon (run tools/make_logo.py first)"
    install -D -m 0644 -- "$icon" \
        "$stage/usr/share/icons/hicolor/$size/apps/rusted-excel.png"
done

if command -v desktop-file-validate >/dev/null 2>&1; then
    printf '==> Validating desktop entry\n'
    desktop-file-validate "$stage/usr/share/applications/$desktop_id"
fi

# dpkg-shlibdeps needs a source-style control file with an Architecture field
# and a Depends field containing the substitution variables.
cat > "$work/debian/control" <<EOF
Source: rusted-excel
Section: office
Priority: optional
Maintainer: $maintainer
Standards-Version: 4.6.2

Package: rusted-excel
Architecture: $arch
Depends: \${shlibs:Depends}, \${misc:Depends}
Description: lightweight desktop spreadsheet
 Placeholder control file used only by dpkg-shlibdeps.
EOF

printf '==> Computing shared-library dependencies\n'
(
    cd -- "$work"
    dpkg-shlibdeps -O -e "$stage/usr/bin/rusted-excel" > "$work/substvars"
)

shlibs_depends=$(sed -n 's/^shlibs:Depends=//p' "$work/substvars" | awk 'NR == 1 { print; exit }')
misc_depends=$(sed -n 's/^misc:Depends=//p' "$work/substvars" | awk 'NR == 1 { print; exit }')
depends=$shlibs_depends
if [[ -n $misc_depends ]]; then
    depends="${depends:+$depends, }$misc_depends"
fi
[[ -n $depends ]] || die "dpkg-shlibdeps produced no dependency information"

installed_size=$(du -sk -- "$stage/usr" | awk '{ print $1 }')

# Use the changelog date for reproducible archive timestamps when available.
if [[ -z ${SOURCE_DATE_EPOCH:-} ]]; then
    changelog_date=$(sed -n 's/^ -- .*  //p' "$repo_root/packaging/changelog" | awk 'NR == 1 { print; exit }')
    if [[ -n $changelog_date ]]; then
        if parsed_epoch=$(date -u -d "$changelog_date" +%s 2>/dev/null); then
            export SOURCE_DATE_EPOCH="$parsed_epoch"
        fi
    fi
fi

cat > "$stage/DEBIAN/control" <<EOF
Package: rusted-excel
Version: $deb_version
Architecture: $arch
Maintainer: $maintainer
Installed-Size: $installed_size
Depends: $depends
Section: office
Priority: optional
Homepage: $homepage
Description: lightweight desktop spreadsheet
 Rusted Excel is a compact spreadsheet application with a Rust desktop GUI and
 a native C/C++ calculation core. It supports multiple sheets, keyboard-first
 editing, cell formatting, common spreadsheet formulas and CSV import/export
 without a browser engine or a heavyweight toolkit.
EOF

deb="$out_dir/rusted-excel_${deb_version}_${arch}.deb"
printf '==> Building %s\n' "$deb"
fakeroot dpkg-deb --build --root-owner-group -Zxz -z9 --uniform-compression \
    "$stage" "$deb"

printf '\nCreated %s\n' "$deb"
