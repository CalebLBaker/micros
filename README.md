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

### Build Dependencies

This list may be incomplete:

* Make
* nasm
* rustc
* cargo
* cargo-about
* lld
* openssl
* sbsigntools
* mtools
* xorriso

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

## Changelog

### v0.1.0 (2024-03-13)

First Pre-release.

Kernel boots from a multiboot2-compliant bootloader and then launches the memory manager.

Memory manager prints "Hello, world" to the screen to give some indication that something has worked.

### v0.2.0 (2024-03-22)

Start in video graphics mode rather than text mode in order to support more modern UEFI firmwares that don't support legacy text mode.

Turn the whole screen white instead of printing "Hello, world" because that's easier when you're not in text mode.

Make release image compatible with UEFI Secure Boot (and propery embed kernel hash in bootloader so that secure boot isn't pointless).

