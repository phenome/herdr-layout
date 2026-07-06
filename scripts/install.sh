#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root_dir=$(dirname -- "$script_dir")
version=$(awk -F'"' '/^[[:space:]]*version[[:space:]]*=/ { print $2; exit }' "$root_dir/herdr-plugin.toml")
[ -n "$version" ] || { echo "Missing version in herdr-plugin.toml" >&2; exit 1; }

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) echo "Unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch=x64 ;;
  arm64|aarch64) arch=arm64 ;;
  *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

asset="herdr-layout-$os-$arch"
if [ "$os" = linux ] && [ "$arch" = x64 ] && command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; then
  asset=herdr-layout-linux-musl-x64
fi

url="https://github.com/phenome/herdr-layout/releases/download/v$version/$asset"
bin_dir="$root_dir/bin"
out="$bin_dir/herdr-layout"
echo "Downloading Herdr Layout binary: $url"
shim="$bin_dir/herdr-layout.cmd"
mkdir -p "$bin_dir"

if command -v curl >/dev/null 2>&1; then
  curl -fL "$url" -o "$out"
elif command -v wget >/dev/null 2>&1; then
  wget -O "$out" "$url"
else
  echo "Need curl or wget to download $url" >&2
  exit 1
fi

echo "Installed Herdr Layout binary: $out"
chmod +x "$out"
cat > "$shim" <<'EOF'
#!/usr/bin/env sh
exec "$(dirname "$0")/herdr-layout" "$@"
EOF
chmod +x "$shim"
