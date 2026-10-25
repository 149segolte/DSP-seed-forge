# DSP Seed Forge

A GPU-powered seed searcher for Dyson Sphere Program, starting with a CLI scaffold.
Seed searching and automated game-algorithm porting will be added in later steps.

```sh
cargo run -- --help
cargo run -- --version
```

Running without arguments also prints help. Unknown arguments report an error.
Help and version output do not initialize Vulkan or load shaders.

The working Vulkan compute demo is available through `--gpu-demo`: Rust
source is compiled to SPIR-V by `cargo-gpu`; an `ash` host dispatches it and
verifies the result on the CPU. There is no window or graphics pipeline.

## Prerequisites

- Stable Rust via rustup for the host (edition 2024).
- `cargo-gpu` 0.11.0, already installed on this machine.
- Vulkan development libraries for linking, plus a Vulkan 1.1+ compute-capable driver.
  On macOS, install the [Vulkan SDK](https://vulkan.lunarg.com/sdk/home#mac)
  including MoltenVK and source its `setup-env.sh` in your terminal before running.
  On Linux/Windows, install the Vulkan loader and your GPU vendor's Vulkan driver.

The host uses ash's `linked` entry point. The build script locates Vulkan through
`pkg-config`, falling back to `VULKAN_SDK` (`lib` on Unix, `Lib`/`Lib32` on Windows).
Changing `VULKAN_SDK` triggers a rebuild. This is linking at build time; with the
usual SDK/Homebrew packages, the executable still depends on the shared Vulkan
loader at runtime.

For Homebrew on macOS:

```sh
brew install pkgconf vulkan-loader molten-vk
export VK_DRIVER_FILES="$(brew --prefix molten-vk)/etc/vulkan/icd.d/MoltenVK_icd.json"
```

Keep that export in the shell used to run the example. It tells the Vulkan loader
where to find the MoltenVK driver; linking the loader alone does not select a driver.

The first shader build downloads the matching Rust nightly and compiler backend
through cargo-gpu. This needs network access and may take several minutes.
The shader pins `spirv-std = "=0.11.0"`; cargo-gpu selects the corresponding
backend/toolchain. The shader is excluded from the host workspace and has its own
lockfile, so the host can keep using stable Rust.

## Build and run

From this directory:

```sh
cargo shader
cargo run -- --gpu-demo
# Optional: choose the number of elements.
cargo run --release -- --gpu-demo --count 65536
```

`cargo shader` is an alias for:

```sh
cargo gpu build --shader-crate shader --output-dir target/shaders \
  --target spirv-unknown-vulkan1.1 --auto-install-rust-toolchain
```

Rebuild the shader after changing its source. Host builds intentionally do not
compile shaders, so `cargo check` and `cargo clippy` work independently.
SPIR-V is loaded at runtime from `target/shaders/manifest.json`; keep the shader
crate and generated output in place when running this development example.

The default dispatch processes 1,003 elements in 16 workgroups of 64 threads.
Each invocation performs `value = value * 2 + 1` with wrapping `u32` arithmetic.
The last workgroup exercises the bounds check. A successful run prints your
device name and:

```text
Verified 1003 values across 16 workgroups: [1, 3, 5, 7, 9, 11, 13, 15]
```

## Code

- `shader/src/lib.rs`: `no_std` compute entry point, global invocation ID, and
  a writable storage buffer at descriptor set 0, binding 0.
- `src/main.rs`: CLI help, version, and argument parsing. `--count` requires
  `--gpu-demo` and accepts positive 32-bit integers.
- `src/gpu_demo.rs`: Vulkan instance/device selection, storage-buffer allocation,
  descriptors, compute pipeline, dispatch, host readback, and result verification.
- `.cargo/config.toml`: shader build command.
- `build.rs`: Vulkan library discovery for the host linker.

The host enables portability enumeration/subset when advertised (for MoltenVK).
It uses host-visible, host-coherent memory for simplicity, a shader-to-host memory
barrier, and a fence before reading results. Production workloads may benefit
from device-local buffers and staging transfers. Resource cleanup also runs when
initialization or dispatch fails.

Keep `threads(64)` in the shader and `WORKGROUP_SIZE` in the GPU demo in sync when
changing the workgroup size. Counts must be positive and fit the selected GPU's
storage-buffer and dispatch limits; oversized inputs report an error.

## Checks

```sh
cargo fmt --check
cargo fmt --manifest-path shader/Cargo.toml --check
cargo clippy --all-targets -- -D warnings
cargo run -- --help
cargo run -- --version
cargo shader
cargo run -- --gpu-demo --count 1
cargo run -- --gpu-demo --count 64
cargo run -- --gpu-demo --count 1003
```

The last three checks need a working Vulkan runtime. On macOS, source the SDK's
environment script or use the Homebrew driver export above. An "Unable to find a
Vulkan driver" error means the loader could not initialize a driver; check the
manifest path and that the process can access Metal (including sandbox permissions).
If linking fails with "library not found for -lvulkan", check `pkg-config --libs
vulkan` or set `VULKAN_SDK`. If shaders are missing, run `cargo shader`.

## References

- [Rust GPU guide introduction](https://rust-gpu.github.io/rust-gpu/book/introduction.html)
- [Writing shader crates](https://rust-gpu.github.io/rust-gpu/book/writing-shader-crates.html)
- [cargo-gpu](https://github.com/Rust-GPU/rust-gpu/tree/main/crates/cargo-gpu)
- [ash API](https://docs.rs/ash/latest/ash/)

This follows the guide's `no_std` shader/`spirv` attribute model, using the
installed cargo-gpu CLI to manage shader compilation. The host's `build.rs` only
configures Vulkan linking.

## License

Licensed under the [Mozilla Public License 2.0](LICENSE).
Rust source files carry SPDX license identifiers, and both Cargo packages
declare `license = "MPL-2.0"`.
