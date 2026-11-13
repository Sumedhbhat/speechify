# Source this after scripts/bootstrap-linux.py to build against the unpacked
# libraries:  source scripts/local-env.sh [sysroot-dir]
SPEECHIFY_SYSROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/.build/sysroot}"
_lib="$SPEECHIFY_SYSROOT/usr/lib/x86_64-linux-gnu"
export PKG_CONFIG_SYSROOT_DIR="$SPEECHIFY_SYSROOT"
export PKG_CONFIG_PATH="$_lib/pkgconfig:$SPEECHIFY_SYSROOT/usr/share/pkgconfig:/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig"
export LIBRARY_PATH="$_lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$_lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export C_INCLUDE_PATH="$SPEECHIFY_SYSROOT/usr/include:$SPEECHIFY_SYSROOT/usr/include/x86_64-linux-gnu"
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-L native=$_lib -C link-arg=-Wl,-rpath-link,$_lib"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}"
unset _lib
