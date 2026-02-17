#!/bin/bash
#
# Seaview Desktop Integration Install Script
#
# This script installs Seaview as a desktop application on Linux systems
# following the freedesktop.org specifications.
#
# Usage:
#   ./install-desktop.sh [--uninstall] [--system]
#
# Options:
#   --uninstall  Remove desktop integration
#   --system     Install system-wide (requires root)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Default to user installation
INSTALL_PREFIX="$HOME/.local"
SYSTEM_INSTALL=false
UNINSTALL=false

# Parse arguments
for arg in "$@"; do
    case $arg in
        --uninstall)
            UNINSTALL=true
            ;;
        --system)
            SYSTEM_INSTALL=true
            INSTALL_PREFIX="/usr/local"
            ;;
        --help|-h)
            echo "Usage: $0 [--uninstall] [--system]"
            echo ""
            echo "Options:"
            echo "  --uninstall  Remove desktop integration"
            echo "  --system     Install system-wide (requires root)"
            exit 0
            ;;
        *)
            echo "Unknown option: $arg"
            exit 1
            ;;
    esac
done

# Check for root if system install
if $SYSTEM_INSTALL && [ "$EUID" -ne 0 ]; then
    echo "Error: System-wide installation requires root privileges."
    echo "Please run with sudo or as root."
    exit 1
fi

# Paths
BINARY_SRC="$PROJECT_ROOT/target/release/seaview"
BINARY_DEST="$INSTALL_PREFIX/bin/seaview"
DESKTOP_FILE="$SCRIPT_DIR/seaview.desktop"
MIME_FILE="$SCRIPT_DIR/seaview-mimetypes.xml"
ICON_DIR="$PROJECT_ROOT/assets/icons"

if $SYSTEM_INSTALL; then
    APPLICATIONS_DIR="/usr/share/applications"
    ICONS_DIR="/usr/share/icons/hicolor"
    MIME_DIR="/usr/share/mime/packages"
else
    APPLICATIONS_DIR="$HOME/.local/share/applications"
    ICONS_DIR="$HOME/.local/share/icons/hicolor"
    MIME_DIR="$HOME/.local/share/mime/packages"
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Uninstall function
do_uninstall() {
    info "Uninstalling Seaview desktop integration..."

    # Remove binary
    if [ -f "$BINARY_DEST" ]; then
        rm -f "$BINARY_DEST"
        info "Removed binary: $BINARY_DEST"
    fi

    # Remove desktop file
    if [ -f "$APPLICATIONS_DIR/seaview.desktop" ]; then
        rm -f "$APPLICATIONS_DIR/seaview.desktop"
        info "Removed desktop file"
    fi

    # Remove icons
    for size in 16 32 48 128 256 512; do
        icon_path="$ICONS_DIR/${size}x${size}/apps/seaview.png"
        if [ -f "$icon_path" ]; then
            rm -f "$icon_path"
        fi
    done

    # Remove scalable icon
    if [ -f "$ICONS_DIR/scalable/apps/seaview.svg" ]; then
        rm -f "$ICONS_DIR/scalable/apps/seaview.svg"
    fi
    info "Removed icons"

    # Remove MIME types
    if [ -f "$MIME_DIR/seaview-mimetypes.xml" ]; then
        rm -f "$MIME_DIR/seaview-mimetypes.xml"
        info "Removed MIME types"
    fi

    # Update caches
    if command -v update-desktop-database &> /dev/null; then
        update-desktop-database "$APPLICATIONS_DIR" 2>/dev/null || true
    fi
    if command -v update-mime-database &> /dev/null; then
        update-mime-database "$(dirname "$MIME_DIR")" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache &> /dev/null; then
        gtk-update-icon-cache -f -t "$ICONS_DIR" 2>/dev/null || true
    fi

    info "Seaview desktop integration removed successfully!"
}

# Install function
do_install() {
    info "Installing Seaview desktop integration..."

    # Check for binary
    if [ ! -f "$BINARY_SRC" ]; then
        error "Binary not found at $BINARY_SRC"
        error "Please build the project first: cargo build --release"
        exit 1
    fi

    # Create directories
    mkdir -p "$INSTALL_PREFIX/bin"
    mkdir -p "$APPLICATIONS_DIR"
    mkdir -p "$MIME_DIR"
    for size in 16 32 48 128 256 512; do
        mkdir -p "$ICONS_DIR/${size}x${size}/apps"
    done
    mkdir -p "$ICONS_DIR/scalable/apps"

    # Install binary
    cp "$BINARY_SRC" "$BINARY_DEST"
    chmod +x "$BINARY_DEST"
    info "Installed binary to $BINARY_DEST"

    # Install desktop file
    if [ -f "$DESKTOP_FILE" ]; then
        # Update Exec path for user installs
        if ! $SYSTEM_INSTALL; then
            sed "s|Exec=seaview|Exec=$BINARY_DEST|g" "$DESKTOP_FILE" > "$APPLICATIONS_DIR/seaview.desktop"
        else
            cp "$DESKTOP_FILE" "$APPLICATIONS_DIR/seaview.desktop"
        fi
        chmod 644 "$APPLICATIONS_DIR/seaview.desktop"
        info "Installed desktop file"
    else
        warn "Desktop file not found: $DESKTOP_FILE"
    fi

    # Install icons
    icon_installed=false

    # Install SVG (scalable)
    if [ -f "$ICON_DIR/seaview.svg" ]; then
        cp "$ICON_DIR/seaview.svg" "$ICONS_DIR/scalable/apps/seaview.svg"
        chmod 644 "$ICONS_DIR/scalable/apps/seaview.svg"
        icon_installed=true
    fi

    # Install PNG at various sizes
    for size in 16 32 48 128 256 512; do
        png_file="$ICON_DIR/seaview-${size}.png"
        if [ -f "$png_file" ]; then
            cp "$png_file" "$ICONS_DIR/${size}x${size}/apps/seaview.png"
            chmod 644 "$ICONS_DIR/${size}x${size}/apps/seaview.png"
            icon_installed=true
        fi
    done

    if $icon_installed; then
        info "Installed icons"
    else
        warn "No icons found in $ICON_DIR"
        warn "Consider creating seaview.svg or seaview-{16,32,48,128,256,512}.png"
    fi

    # Install MIME types
    if [ -f "$MIME_FILE" ]; then
        cp "$MIME_FILE" "$MIME_DIR/seaview-mimetypes.xml"
        chmod 644 "$MIME_DIR/seaview-mimetypes.xml"
        info "Installed MIME types"
    else
        warn "MIME file not found: $MIME_FILE"
    fi

    # Update caches
    info "Updating desktop databases..."

    if command -v update-desktop-database &> /dev/null; then
        update-desktop-database "$APPLICATIONS_DIR" 2>/dev/null || true
    fi

    if command -v update-mime-database &> /dev/null; then
        update-mime-database "$(dirname "$MIME_DIR")" 2>/dev/null || true
    fi

    if command -v gtk-update-icon-cache &> /dev/null; then
        gtk-update-icon-cache -f -t "$ICONS_DIR" 2>/dev/null || true
    fi

    # Set as default handler for mesh files (user install only)
    if ! $SYSTEM_INSTALL; then
        if command -v xdg-mime &> /dev/null; then
            xdg-mime default seaview.desktop model/stl 2>/dev/null || true
            xdg-mime default seaview.desktop model/gltf+json 2>/dev/null || true
            xdg-mime default seaview.desktop model/gltf-binary 2>/dev/null || true
            xdg-mime default seaview.desktop application/sla 2>/dev/null || true
            info "Set Seaview as default handler for mesh files"
        fi
    fi

    echo ""
    info "Seaview desktop integration installed successfully!"
    echo ""
    echo "You can now:"
    echo "  • Launch Seaview from your application menu"
    echo "  • Open .stl, .gltf, .glb files with Seaview"
    echo "  • Right-click mesh files and select 'Open with Seaview'"
    echo ""

    if ! $SYSTEM_INSTALL; then
        echo "Note: Make sure $INSTALL_PREFIX/bin is in your PATH"
        if [[ ":$PATH:" != *":$INSTALL_PREFIX/bin:"* ]]; then
            warn "$INSTALL_PREFIX/bin is not in your PATH"
            echo "Add this to your ~/.bashrc or ~/.profile:"
            echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        fi
    fi
}

do_build() {
    cargo build --release
}

# Main
if $UNINSTALL; then
    do_uninstall
else
    do_build
    do_install
fi
