arch ?= amd64
config ?= release
target ?= x86_64-unknown-none
image := build/micros-$(arch).elf
iso := build/micros-$(arch).iso

linker_script := src/micros_kernel/linker.ld
limine_cfg := src/limine.cfg
assembly_source_files := $(wildcard src/micros_kernel/*.asm)
assembly_object_files := $(patsubst src/micros_kernel/%.asm, build/src/micros_kernel/%.o, $(assembly_source_files))
kernel := target/$(target)/$(config)/libmicros_kernel.a

.PHONY: all clean run iso rust_build

all: $(iso)

limine/limine:
	@cd limine && make limine

clean:
	@rm -rf build
	@rm -rf target
	@rm -rf Cargo.lock

run: $(iso)
	@qemu-system-x86_64 -cdrom $(iso) -d int -no-shutdown -no-reboot

check: $(image)
	@cargo clippy 
	@cargo audit

rust_build:
	@cargo build --target src/$(target).json --release

iso: $(iso)

build/third-party-licenses.html: about.toml about.hbs
	@cargo about generate about.hbs -o build/third-party-licenses.html

$(iso): $(image) $(limine_cfg) LICENSE build/third-party-licenses.html rust_build limine/limine
	@rm -rf build/isofiles
	@mkdir -p build/isofiles/EFI/BOOT
	@cp $(image) build/isofiles/micros.elf
	@cp target/$(target)/$(config)/micros_memory_manager build/isofiles/memory_manager.elf
	@cat $(limine_cfg) | sed "s/kernel_hash/`b2sum build/isofiles/micros.elf | sed 's/  .*//'`/" | sed "s/memory_manager_hash/`b2sum build/isofiles/memory_manager.elf | sed 's/  .*//'`/" > build/isofiles/limine.cfg
	@cp LICENSE build/isofiles/
	@cp build/third-party-licenses.html build/isofiles/
	@cp limine/limine-uefi-cd.bin build/isofiles/
	@cp limine/limine-bios-cd.bin build/isofiles/
	@cp limine/limine-bios.sys build/isofiles/
	@cp limine/BOOTX64.EFI build/isofiles/EFI/BOOT/
	@limine/limine enroll-config build/isofiles/EFI/BOOT/BOOTX64.EFI `b2sum build/isofiles/limine.cfg | sed 's/  .*//'`
	@xorriso -as mkisofs -b limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table --efi-boot limine-uefi-cd.bin -efi-boot-part --efi-boot-image --protective-msdos-label build/isofiles -o $(iso)
	@limine/limine bios-install $(iso)


$(image): $(assembly_object_files) $(linker_script) rust_build
	@ld.lld -n -s -T $(linker_script) -o $(image) $(assembly_object_files) $(kernel)

build/src/micros_kernel/%.o: src/micros_kernel/%.asm
	@mkdir -p $(shell dirname $@)
	@nasm -felf64 $< -o $@

kernellinecount:
	cloc src/micros_kernel src/frame_allocation --exclude-lang=TOML

