# Seaview

A modern reimplementation of mesh-ripper - a high-performance mesh sequence viewer for fluid simulations and time-series 3D data.

## System Requirements

### Linux

Install development libraries required by Bevy and seaview:

```bash
# Ubuntu/Debian
sudo apt-get install g++ pkg-config libx11-dev libasound2-dev libudev-dev \
    libxkbcommon-x11-0 libwayland-dev libxkbcommon-dev clang wild-linker

# Fedora
sudo dnf install gcc-c++ pkgconf libX11-devel alsa-lib-devel systemd-devel \
    libxkbcommon-devel wayland-devel clang

# Arch
sudo pacman -S gcc pkgconf libx11 alsa-lib udev libxkbcommon wayland clang
```

You also need a Vulkan driver for your GPU:

```bash
# Ubuntu/Debian — pick one:
sudo apt-get install vulkan-tools  # NVIDIA (driver includes Vulkan)
sudo apt-get install mesa-vulkan-drivers  # AMD / Intel
```

### Windows
No additional dependencies required.

### macOS
No additional dependencies required.

## Building

```bash
cargo build --release
```

### Installing Binary

You can install seaview as a system binary using the provided script:

```bash
./scripts/cargo_install.sh
```

This will install seaview to `~/.cargo/bin`. To uninstall:

```bash
./scripts/cargo_uninstall.sh
```

## Running

```bash
cargo run --release
```

### Testing with Example Sequences

To test with the provided example sequences (e.g., right-hander):

```bash
cargo run --release --bin seaview -- assets/test_sequences/right-hander/ --source-coordinates zup
```

### Mesh Conversion Tools

Seaview includes several tools for optimizing and converting mesh data:

#### STL to glTF/GLB Converter
Convert STL files to the more efficient glTF format:

```bash
# Convert a single file
cargo run --release --bin stl_to_gltf -- input.stl

# Convert a directory of STL files
cargo run --release --bin stl_to_gltf -- /path/to/stl/directory -o /path/to/output
```

#### Mesh Receiver Service
Network service for receiving triangle mesh data:

```bash
# Start the mesh receiver service
cargo run --release --bin mesh_receiver_service -- -p 9876 -o ./output

# Test with the mesh sender
cargo run --release --bin mesh_sender_test -- -n 10 -a
```

#### Network Testing Scripts
Simple scripts to test the mesh processing pipeline:

```bash
# Run complete send/receive test
./scripts/test_network_send_and_receive.sh

# Test seaview's network visualization
./scripts/test_network_send_and_visualize.sh

# Start receiver manually
./scripts/run_receiver.sh

# Send test data manually  
./scripts/send_test_mesh.sh
```

See [docs/mesh-conversion-tools.md](docs/mesh-conversion-tools.md) for detailed documentation.

## Project Structure

```
seaview/
├── Cargo.toml           # Workspace configuration
├── crates/
│   └── seaview/       # Main application crate
│       ├── Cargo.toml
│       └── src/
│           └── main.rs  # Application entry point
├── research/            # Documentation and planning
│   └── reimplementation.md
└── vendor/              # Third-party code for reference
    └── mesh-ripper/
```

## Development Plan

See [research/reimplementation.md](research/reimplementation.md) for the detailed development roadmap.

## Phase 1 Status

Currently implementing Phase 1: Foundation and Core Infrastructure
- [x] Initialize Rust project with Bevy 0.14
- [x] Set up cargo workspace structure
- [x] Configure development environment
- [x] Create basic plugin architecture (minimal)
- [x] Implement minimal Bevy app with 3D scene
- [x] Set up FPS camera system with WASD controls
- [x] Mouse look controls with cursor grab/release
- [ ] Create simple mesh loading for single files
- [x] Implement basic lighting setup

### Camera Controls
- **WASD**: Move camera forward/backward/left/right
- **Q**: Move up
- **E**: Move down
- **Alt + Q/E**: Move up/down at half speed (for precise positioning)
- **Mouse**: Look around (click to grab cursor, Esc to release)
- **Escape Mode**: When Esc is pressed, all camera controls are disabled until you click again

## License

This project is dual-licensed under MIT OR Apache-2.0.
