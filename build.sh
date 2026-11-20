#!/bin/bash
# Build script for OmaFinances (Rust backend + Quickshell QML)
# Usage: ./build.sh [--release] [--live] [--clean] [--help]
#
#   --live   build a release binary and install it as the live app in /opt/omafinances
#            (override with OMAFINANCES_LIVE_DIR). Implies --release. The first run asks
#            for sudo once, to create /opt/omafinances owned by you; after that no sudo is
#            needed. Writes the Omarchy menu entry ~/.local/share/applications/OmaFinances.desktop
#            when it is missing or different (nothing else in the menu is touched). Stops
#            every running OmaFinances window and backend (live and dev copies) before the
#            new files go in, and afterwards starts the app again if it was open.
#
# Environment (test hooks, not for normal use):
#   OMAFINANCES_SKIP_BUILD=1 OMAFINANCES_PREBUILT=<path>
#            skip cargo and install <path> as the binary.
#
# Where cargo builds: ./target by default. To build elsewhere (a fast local disk when the
# clone is on a network share), set OMAFINANCES_BUILD_DIR, either in the environment or as a line
#   OMAFINANCES_BUILD_DIR=/some/fast/disk/omafinances-build
# in ${XDG_CONFIG_HOME:-~/.config}/omafinances/build.env (kept per machine, never committed).
# The build log (build.log) goes into that build dir as well.

set -e

# Ensure Rust toolchain is available
if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

usage() {
    cat <<'USAGE'
Usage: ./build.sh [--release] [--live] [--clean] [--help]

  --release  build target/release/omafinances (default: debug build)
  --live     release build, then install to ${OMAFINANCES_LIVE_DIR:-/opt/omafinances}
             and refresh the Omarchy menu entry (implies --release)
  --clean    cargo clean before building
  --help     show this text

Config: ${XDG_CONFIG_HOME:-~/.config}/omafinances/build.env may set OMAFINANCES_BUILD_DIR.
USAGE
}

# Detect if we're in the project root
if [ ! -f "Cargo.toml" ]; then
    echo -e "${RED}Error: build.sh must be run from the project root${NC}"
    exit 1
fi

PROJECT_DIR=$(pwd)
BUILD_TYPE="debug"
CLEAN_BUILD=false
LIVE=false
LIVE_DIR="${OMAFINANCES_LIVE_DIR:-/opt/omafinances}"

# Per-machine build location (see the header). An environment value wins over the file.
BUILD_ENV_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/omafinances/build.env"
if [ -z "${OMAFINANCES_BUILD_DIR:-}" ] && [ -f "$BUILD_ENV_FILE" ]; then
    OMAFINANCES_BUILD_DIR=$(sed -n 's/^[[:space:]]*OMAFINANCES_BUILD_DIR=//p' "$BUILD_ENV_FILE" | tail -n1 | sed -e 's/^["'"'"']//' -e 's/["'"'"']$//' -e "s|^~|$HOME|")
fi
BUILD_DIR="${OMAFINANCES_BUILD_DIR:-$PROJECT_DIR/target}"
export CARGO_TARGET_DIR="$BUILD_DIR"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --release)
            BUILD_TYPE="release"
            shift
            ;;
        --live)
            LIVE=true
            BUILD_TYPE="release"
            shift
            ;;
        --clean)
            CLEAN_BUILD=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

if [ "$LIVE" = true ] && [ ! -d "qml" ]; then
    echo -e "${RED}Error: --live needs the qml/ directory (not found in $PROJECT_DIR)${NC}"
    exit 1
fi

# === RUNNING INSTANCES ===
# Escape a path for use inside a pgrep -f extended regex.
re_escape() { printf '%s' "$1" | sed -e 's/[][\\.^$*+?(){}|/]/\\&/g'; }

# pids of processes whose command line is exactly `<exe>` or `<exe> <args>`.
pids_of_exe() {
    local exe="$1"
    pgrep -f "^$(re_escape "$exe")( |\$)" 2>/dev/null || true
}
# pids of the quickshell instance running a given qml dir (the window).
pids_of_ui() { pgrep -f "^quickshell -p $(re_escape "$1")( |\$)" 2>/dev/null || true; }

# Ask politely (TERM), wait up to 8 s, then KILL what is left.
stop_pids() {
    [ $# -gt 0 ] || return 0
    kill "$@" 2>/dev/null || true
    for _ in $(seq 1 40); do
        local alive=0 p
        for p in "$@"; do kill -0 "$p" 2>/dev/null && alive=1; done
        [ $alive = 0 ] && return 0
        sleep 0.2
    done
    kill -9 "$@" 2>/dev/null || true
}

echo -e "${GREEN}🔨 OmaFinances Build System${NC}"
echo "Build type: $BUILD_TYPE"
echo "Project dir: $PROJECT_DIR"
echo "Build dir:   $BUILD_DIR"

mkdir -p "$BUILD_DIR"

# === CLEAN ===
if [ "$CLEAN_BUILD" = true ]; then
    echo -e "\n${YELLOW}🧹 Cleaning build artifacts...${NC}"
    cargo clean
    echo -e "${GREEN}✓ Clean complete${NC}"
fi

# === BUILD ===
if [ "$BUILD_TYPE" = "release" ]; then
    CARGO_PROFILE_DIR="release"
    CARGO_FLAGS="--release"
else
    CARGO_PROFILE_DIR="debug"
    CARGO_FLAGS=""
fi

if [ -n "${OMAFINANCES_SKIP_BUILD:-}" ]; then
    BINARY="${OMAFINANCES_PREBUILT:?OMAFINANCES_SKIP_BUILD needs OMAFINANCES_PREBUILT}"
    echo -e "\n${YELLOW}Skipping cargo (OMAFINANCES_SKIP_BUILD); using $BINARY${NC}"
else
    echo -e "\n${YELLOW}🏗️  Building ($BUILD_TYPE)...${NC}"
    BINARY="$BUILD_DIR/$CARGO_PROFILE_DIR/omafinances"

    # pipefail makes a failed build fail here instead of being masked by `tee` (the old
    # binary would otherwise still exist and be reported as a success).
    set -o pipefail
    # shellcheck disable=SC2086
    cargo build $CARGO_FLAGS 2>&1 | tee "$BUILD_DIR/build.log" || { echo -e "${RED}❌ Build failed. See $BUILD_DIR/build.log.${NC}"; exit 1; }
    set +o pipefail
fi

# Check if build succeeded
if [ ! -f "$BINARY" ]; then
    echo -e "${RED}❌ Build failed. Check $BUILD_DIR/build.log for details.${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Build successful${NC}"
echo -e "${GREEN}Binary: $(realpath "$BINARY")${NC}"
echo -e "${GREEN}Size: $(du -h "$BINARY" | cut -f1)${NC}"

# === SUMMARY ===
echo -e "\n${GREEN}=== Build Summary ===${NC}"
echo "Output binary: $BINARY"
echo "To run the dev build (close the live app first):  QML_DIR=\$PWD/qml $BINARY"
echo "To install the live app:  ./build.sh --live   (then $LIVE_DIR/omafinances)"

# === LIVE INSTALL ===
if [ "$LIVE" = true ]; then
    echo -e "\n${YELLOW}🚀 Installing live to $LIVE_DIR...${NC}"

    # One-time: /opt is root's. Create the folder owned by you so later runs need no sudo.
    if [ ! -d "$LIVE_DIR" ] || [ ! -w "$LIVE_DIR" ]; then
        mkdir -p "$LIVE_DIR" 2>/dev/null || {
            echo "Creating $LIVE_DIR (needs sudo once)..."
            sudo install -d -o "$(id -un)" -g "$(id -gn)" -m 755 "$LIVE_DIR"
        }
    fi
    mkdir -p "$LIVE_DIR/bin" "$LIVE_DIR/qml" "$LIVE_DIR/share"

    # Remember what is running, then stop all of it: the live and the dev copies share the
    # config dir and the database, so none may be left over.
    UI_WAS_UP=false; BACKEND_WAS_UP=false
    LIVE_UI=$(pids_of_ui "$LIVE_DIR/qml")
    LIVE_BACKEND=$(pids_of_exe "$LIVE_DIR/bin/omafinances")
    DEV_UI=$(pids_of_ui "$PROJECT_DIR/qml")
    DEV_BACKEND=$(for b in "$BUILD_DIR/release/omafinances" "$BUILD_DIR/debug/omafinances"; do pids_of_exe "$b"; done)
    [ -n "$LIVE_UI" ] && UI_WAS_UP=true
    [ -n "$LIVE_BACKEND" ] && BACKEND_WAS_UP=true
    if [ -n "$LIVE_UI$LIVE_BACKEND$DEV_UI$DEV_BACKEND" ]; then
        echo -e "${YELLOW}Stopping running OmaFinances (window: $UI_WAS_UP, backend: $BACKEND_WAS_UP)...${NC}"
        # shellcheck disable=SC2086
        stop_pids $LIVE_UI $DEV_UI
        # shellcheck disable=SC2086
        stop_pids $LIVE_BACKEND $DEV_BACKEND
        sleep 0.5
    fi

    # Replace the binary by renaming a fresh copy over it: copying onto a running
    # executable fails ("Text file busy"), a rename does not and the running
    # instance keeps its old inode until it is restarted.
    install -m 755 "$BINARY" "$LIVE_DIR/bin/omafinances.new"
    mv -f "$LIVE_DIR/bin/omafinances.new" "$LIVE_DIR/bin/omafinances"

    # The UI. Commons/Ui are links to the Omarchy shell kit that the app creates
    # itself on first start, so leave them out of the copy and out of --delete.
    rsync -a --delete --exclude Commons --exclude Ui "$PROJECT_DIR/qml/" "$LIVE_DIR/qml/"

    install -m 644 "$PROJECT_DIR/packaging/omafinances.svg" "$LIVE_DIR/share/omafinances.svg"

    # Menu entry: ~/.local/share/applications/OmaFinances.desktop puts OmaFinances in the
    # Omarchy app launcher (Quickshell DesktopEntries reads that directory). Written when
    # missing, and rewritten when it differs from what this script expects. StartupWMClass
    # is the window class of the window; tools that map a window to its launcher need it.
    DESKTOP_FILE="${XDG_DATA_HOME:-$HOME/.local/share}/applications/OmaFinances.desktop"
    DESKTOP_WANT=$(cat <<DESKTOP
[Desktop Entry]
Type=Application
Name=OmaFinances
Comment=Finance manager
Exec=$LIVE_DIR/omafinances
TryExec=$LIVE_DIR/omafinances
Icon=$LIVE_DIR/share/omafinances.svg
Terminal=false
Categories=Office;Finance;
StartupNotify=true
StartupWMClass=org.quickshell
DESKTOP
)
    if [ ! -f "$DESKTOP_FILE" ]; then
        mkdir -p "$(dirname "$DESKTOP_FILE")"
        printf '%s\n' "$DESKTOP_WANT" > "$DESKTOP_FILE"
        echo -e "${GREEN}✓ Menu entry created: $DESKTOP_FILE${NC}"
    elif [ "$(cat "$DESKTOP_FILE")" != "$DESKTOP_WANT" ]; then
        printf '%s\n' "$DESKTOP_WANT" > "$DESKTOP_FILE"
        echo -e "${GREEN}✓ Menu entry updated: $DESKTOP_FILE${NC}"
    else
        echo "Menu entry present: $DESKTOP_FILE"
    fi

    # Launcher: points the app at the installed UI, and refuses to open a second window:
    # it focuses the one that is open instead.
    cat > "$LIVE_DIR/omafinances" <<'LAUNCHER'
#!/bin/bash
# OmaFinances launcher (written by build.sh --live)
DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"

# Single instance: an open window (quickshell on this qml dir) means "already running".
if pgrep -f "^quickshell -p $DIR/qml( |\$)" > /dev/null; then
    addr=$(hyprctl clients -j 2>/dev/null | jq -r '.[] | select(.class=="org.quickshell" and .title=="OmaFinances") | .address' | head -n1)
    [ -n "$addr" ] && hyprctl dispatch focuswindow "address:$addr" > /dev/null 2>&1
    exit 0
fi

export QML_DIR="$DIR/qml"
export RUST_LOG="${RUST_LOG:-omafinances=info,warn}"
exec "$DIR/bin/omafinances" "$@"
LAUNCHER
    chmod 755 "$LIVE_DIR/omafinances"

    echo -e "${GREEN}✓ Live install complete${NC}"
    echo "Launcher: $LIVE_DIR/omafinances"
    echo "Binary:   $LIVE_DIR/bin/omafinances ($(du -h "$LIVE_DIR/bin/omafinances" | cut -f1))"

    # Put back what was running. The launcher starts the backend and the window together.
    if [ "$UI_WAS_UP" = true ] || [ "$BACKEND_WAS_UP" = true ]; then
        RESTART_LOG="$BUILD_DIR/restart.log"
        echo -e "${YELLOW}Restarting OmaFinances...${NC}"
        setsid -f "$LIVE_DIR/omafinances" > "$RESTART_LOG" 2>&1 < /dev/null
        for _ in $(seq 1 100); do [ -n "$(pids_of_ui "$LIVE_DIR/qml")" ] && break; sleep 0.2; done
        [ -n "$(pids_of_ui "$LIVE_DIR/qml")" ] \
            && echo -e "${GREEN}✓ OmaFinances restarted${NC}" \
            || echo -e "${RED}⚠ OmaFinances did not come back; see $RESTART_LOG${NC}"
    fi
fi
