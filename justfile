# Justfile for noos OS project with direct QEMU runner

# Default recipe - show help
default:
    @just --list

# Build the kernel
build:
    cargo build

# Build bootable image
build-image: build
    cargo bootimage

# Run in QEMU (with default args)
run: build-image
    qemu-system-x86_64 \
        -drive format=raw,file=target/x86_64-blog_os/debug/bootimage-noos.bin \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
        -serial stdio \
        -display none

# Run with graphics (useful for debugging)
run-display: build-image
    qemu-system-x86_64 \
        -drive format=raw,file=target/x86_64-blog_os/debug/bootimage-noos.bin

# Run with custom QEMU args - usage: just run-custom "-m 512 -smp 2"
run-custom ARGS: build-image
    qemu-system-x86_64 \
        -drive format=raw,file=target/x86_64-blog_os/debug/bootimage-noos.bin \
        {{ARGS}}

# Run tests in QEMU
test: build-image
    qemu-system-x86_64 \
        -drive format=raw,file=target/x86_64-blog_os/debug/bootimage-noos.bin \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
        -serial stdio \
        -display none

# Clean build artifacts
clean:
    cargo clean

# Check code without building
check:
    cargo check

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt -- --check

# Run clippy linter
clippy:
    cargo clippy
