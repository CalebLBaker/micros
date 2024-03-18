arch ?= amd64
config ?= release
target ?= x86_64-unknown-none
iso := build/micros-$(arch).iso
key := /home/caleb/private/db.key
cert := db.crt
isodir := build/isofiles
image := $(isodir)/micros.elf

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
	@rm Cargo.lock

run: $(iso)
	@qemu-system-x86_64 -cdrom $(iso) -d int -no-shutdown -no-reboot

check: $(image) rust_build
	@cargo clippy 
	@cargo audit

rust_build:
	@cargo build --target src/$(target).json --release

iso: $(iso)

$(isodir)/third-party-licenses.html: about.toml about.hbs $(isodir)
	@cargo about generate about.hbs -o $@

$(isodir)/EFI/BOOT:
	@mkdir -p $@

$(isodir): $(isodir)/EFI/BOOT

$(isodir)/memory_manager.elf: rust_build
	@cp target/$(target)/$(config)/micros_memory_manager $@

$(isodir)/limine.cfg: $(limine_cfg) $(image) $(isodir)/memory_manager.elf
	@cat $(limine_cfg) | sed "s/kernel_hash/`b2sum $(image) | sed 's/  .*//'`/" | sed "s/memory_manager_hash/`b2sum $(isodir)/memory_manager.elf | sed 's/  .*//'`/" > $@

$(isodir)/LICENSE: LICENSE
	@cp LICENSE $(isodir)/

$(isodir)/limine-uefi-cd.bin: $(isodir)
	@cp limine/limine-uefi-cd.bin $(isodir)/

$(isodir)/limine-bios-cd.bin: $(isodir)
	@cp limine/limine-bios-cd.bin $(isodir)/

$(isodir)/limine-bios.sys: $(isodir)
	@cp limine/limine-bios.sys $(isodir)/

$(isodir)/EFI/BOOT/BOOTX64.EFI: $(isodir)/EFI/BOOT $(key) $(cert) limine/limine $(isodir)/limine.cfg
	@sbsign --key $(key) --cert $(cert) --output $@ limine/BOOTX64.EFI
	@limine/limine enroll-config $@ `b2sum $(isodir)/limine.cfg | sed 's/  .*//'`

$(iso): $(image) $(isodir)/limine.cfg $(isodir)/LICENSE $(isodir)/third-party-licenses.html $(isodir)/memory_manager.elf $(isodir)/limine-uefi-cd.bin $(isodir)/limine-bios-cd.bin $(isodir)/limine-bios.sys $(isodir)/EFI/BOOT/BOOTX64.EFI limine/limine
	@xorriso -as mkisofs -b limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table --efi-boot limine-uefi-cd.bin -efi-boot-part --efi-boot-image --protective-msdos-label $(isodir) -o $@
	@limine/limine bios-install $@


$(image): $(assembly_object_files) $(linker_script) rust_build $(isodir)
	@ld.lld -n -s -T $(linker_script) -o $(image) $(assembly_object_files) $(kernel)

build/src/micros_kernel/%.o: src/micros_kernel/%.asm
	@mkdir -p $(shell dirname $@)
	@nasm -felf64 $< -o $@

kernellinecount:
	cloc src/micros_kernel src/frame_allocation --exclude-lang=TOML

