#!/bin/bash
#
# Generate PNG icons from the SVG source
#
# Requires either:
#   - Inkscape (recommended)
#   - ImageMagick (convert/magick)
#   - rsvg-convert (librsvg)
#
# Usage:
#   ./generate-icons.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
ICON_DIR="$PROJECT_ROOT/assets/icons"
SVG_FILE="$ICON_DIR/seaview.svg"

# Icon sizes to generate (standard freedesktop sizes)
SIZES=(16 32 48 64 128 256 512)

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check for SVG file
if [ ! -f "$SVG_FILE" ]; then
    error "SVG file not found: $SVG_FILE"
    exit 1
fi

# Detect available tool
TOOL=""

if command -v inkscape &> /dev/null; then
    TOOL="inkscape"
    info "Using Inkscape for conversion"
elif command -v rsvg-convert &> /dev/null; then
    TOOL="rsvg"
    info "Using rsvg-convert for conversion"
elif command -v magick &> /dev/null; then
    TOOL="magick"
    info "Using ImageMagick (magick) for conversion"
elif command -v convert &> /dev/null; then
    TOOL="convert"
    info "Using ImageMagick (convert) for conversion"
else
    error "No suitable tool found for SVG to PNG conversion."
    echo ""
    echo "Please install one of the following:"
    echo "  - Inkscape:    sudo apt install inkscape"
    echo "  - librsvg:     sudo apt install librsvg2-bin"
    echo "  - ImageMagick: sudo apt install imagemagick"
    exit 1
fi

# Generate icons
info "Generating PNG icons from $SVG_FILE"

for size in "${SIZES[@]}"; do
    output="$ICON_DIR/seaview-${size}.png"

    case $TOOL in
        inkscape)
            # Inkscape 1.0+ syntax
            inkscape "$SVG_FILE" --export-type=png --export-filename="$output" \
                     --export-width=$size --export-height=$size 2>/dev/null
            ;;
        rsvg)
            rsvg-convert -w $size -h $size "$SVG_FILE" -o "$output"
            ;;
        magick)
            magick "$SVG_FILE" -resize ${size}x${size} "$output"
            ;;
        convert)
            convert -background none "$SVG_FILE" -resize ${size}x${size} "$output"
            ;;
    esac

    if [ -f "$output" ]; then
        info "Generated: seaview-${size}.png"
    else
        warn "Failed to generate: seaview-${size}.png"
    fi
done

# Also generate a favicon.ico if ImageMagick is available
if [ "$TOOL" = "magick" ] || [ "$TOOL" = "convert" ]; then
    info "Generating favicon.ico..."
    ico_file="$ICON_DIR/seaview.ico"

    # Create ICO with multiple sizes embedded
    if [ "$TOOL" = "magick" ]; then
        magick "$ICON_DIR/seaview-16.png" "$ICON_DIR/seaview-32.png" \
               "$ICON_DIR/seaview-48.png" "$ICON_DIR/seaview-64.png" \
               "$ico_file" 2>/dev/null || warn "Failed to create favicon.ico"
    else
        convert "$ICON_DIR/seaview-16.png" "$ICON_DIR/seaview-32.png" \
                "$ICON_DIR/seaview-48.png" "$ICON_DIR/seaview-64.png" \
                "$ico_file" 2>/dev/null || warn "Failed to create favicon.ico"
    fi

    if [ -f "$ico_file" ]; then
        info "Generated: seaview.ico"
    fi
fi

echo ""
info "Icon generation complete!"
echo ""
echo "Generated icons:"
ls -la "$ICON_DIR"/*.png 2>/dev/null || true
ls -la "$ICON_DIR"/*.ico 2>/dev/null || true
