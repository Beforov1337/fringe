#!/usr/bin/env bash
set -e

# fringe installer script
# Installs fringe binary to ~/.local/bin

INSTALL_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.config/fringe"

echo "==> Building fringe (release mode)..."
cargo build --release

echo "==> Installing binary to ${INSTALL_DIR}..."
mkdir -p "${INSTALL_DIR}"
install -m 755 target/release/fringe "${INSTALL_DIR}/fringe"

# Ensure default config exists
if [ ! -f "${CONFIG_DIR}/fringe.conf" ]; then
    echo "==> Creating default config in ${CONFIG_DIR}/fringe.conf..."
    mkdir -p "${CONFIG_DIR}"
    cat << 'EOF' > "${CONFIG_DIR}/fringe.conf"
# fringe configuration
# Location: ~/.config/fringe/fringe.conf
# Syntax: key = value

# Theme: monochrome, synthwave, matrix, amber, custom
theme = monochrome

# Visualizer (CAVA or custom program)
# To embed an external program inside the visualizer window (e.g. lavat, cava, pipes.sh):
# visualizer_cmd = lavat -c cyan -R 1
visualizer_cmd = 

bar_width = 2
bar_gap = 1
framerate = 60
sensitivity = 1.0
gravity = 28.0
noise_reduction = 0.77
monstercat = 1.0
show_peaks = false

# Lyrics active word effect: glow, underline, box, dots, bold
lyric_effect = glow

# Custom colors (hex #rrggbb or names: white, black, gray, cyan, green, yellow, red, blue, magenta)
# color_primary = #e0e0e0
# color_accent = #ffffff
# color_active_word = #ffffff
# color_sung_word = #909090
# color_inactive = #505050
EOF
fi

echo ""
echo "✔ Installation complete!"
echo "Run 'fringe' to start listening."
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo "Notice: Make sure ${INSTALL_DIR} is in your PATH."
fi
