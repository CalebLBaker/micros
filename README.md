# micros

Minimal microkernel operating system

This is a hobby project and there is no guarantee that it will be maintained or that it will function correctly

Micros currently only targets the AMD64 (x86\_64) architecture.

## Building/Installing

### Pre-built Releases

Latest release is in the repository at `release/micros-amd64.iso`.

### Creating a bootable flashdrive

```bash
sudo dd if=release/micros-amd64.iso of=/dev/<your-flash-drive>
```

### UEFI Secure Boot Support

The bootloader in the release image is signed with the key that corresponds to the cert at `release/micros.crt`.

If you add `release/micros.crt` as a DB key in your devices firmware then the release image will be able to boot in UEFI Secure Boot on your computer.

Alternatively, you may build an image signed with a key that you've already added in your firmware by editing `Makefile` to set the `key` and `cert` variables to point at your key and certificate and then running `make`.
The newly built image will be at `build/micros-amd64.iso`.

### Installing Build Dependencies

#### Arch Linux

```bash
sudo pacman -S core-devel nasm rustup lld mtools libisoburn openssl sbsigntools
rustup component add rust-src --toolchain nightly-x86_64-unknown-linux-gnu
cargo install cargo-about
cd $(MICROS_REPO_ROOT)
git submodule init
git submodule update
```

##### Dependencies only needed for supply chain auditing
```bash
cargo install cargo-audit
```

##### Dependencies only needed for running in an emulaator
```bash
sudo pacman -S qemu-desktop
```

#### Debian

```bash
sudo apt install curl nasm lld mtools sbsigntools xorriso
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup component add rust-src --toolchain nightly-x86_64-unknown-linux-gnu
cargo install cargo-about
```

##### Dependencies only needed for supply chain auditing
```bash
cargo install cargo-audit
```

##### Dependencies only needed for running in an emulaator
```bash
sudo apt install qemu-system-x86
```

### Build Instructions

Run `make` from the root of the repository and then a bootable ISO file will be at `build/micros-amd64.iso`.

## Usage

There's nothing in micros to use. The OS doesn't yet support any kind of user interaction or even true userspace processes yet (The memory manager is technically a user space process, but it's a special kind of user space process that gets to be more priviledged than the rest).

## Component Interfaces

### Bootloader <-> Kernel

* The kernel expects to be booted by a multiboot2-compliant bootloader.

* The kernel expects there to be a boot module whose associated string contains the text "memory\_manager".
  This boot module should be the memory manager executable in ELF file format.

### Kernel <-> Memory Manager

* The kernel will invoke the memory manager's main entry function and pass in two parameters.

    - The first parameter is a pointer to a `Amd64FrameAllocator` structure as described in the `amd64` module of the `src/frame_allocation` crate. This will contain all of the memory frames that are not in use at the time the memory manager is launched.

    - The second parameter is a pointer to the multiboot2 boot information structure.

* The memory manager will be launched in user mode but will have all of the devices physical memory identity mapped into its address space.

## Repository branches

* master contains the code that was used to build the latest release and is only updated when a new release occurs.

* staging is the branch that new releases are built from. It is typically updated shortly before new release.

* develop is the branch where active development occurs.

## Steps to 1.0

I'm not sure what all I'll require before calling something a 1.0 release, but it will be at least the following:

* True user space processes that don't require special privileges

* Multiprocessing

* A file system

* User interaction (IO)

* A working libc

