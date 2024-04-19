arch ?= amd64
config ?= release
target ?= x86_64-unknown-none
iso := build/micros-$(arch).iso

# Keys used for signing actual binary releases (private key is stored securely outside of the repository)
# key := /home/caleb/keys/db.key
# cert := release/micros.crt

# Keys generated for convenience in development and testing
key := build/tmp.key
cert := build/tmp.crt

isodir := build/isofiles
image := $(isodir)/micros.elf

linker_script := src/micros_kernel/linker.ld
limine_cfg := src/limine.cfg
assembly_source_files := $(wildcard src/micros_kernel/*.asm)
assembly_object_files := $(patsubst src/micros_kernel/%.asm, build/src/micros_kernel/%.o, $(assembly_source_files))
kernel := target/$(target)/$(config)/libmicros_kernel.a

.PHONY: all clean run iso rust_build

all: $(iso)

release: release/micros-$(arch).iso

release/micros-$(arch).iso: $(iso)
	cp $(iso) $@

build/tmp.crt: build
	openssl req -newkey rsa:4096 -nodes -keyout build/tmp.key -new -x509 -sha256 -days 36500 -subj "/CN=Temporary Dev Micros Key/" -out $@

build/tmp.key: build/tmp.crt

limine/limine:
	$(MAKE) -C limine limine

clean:
	rm -rf build
	rm -rf target
	rm Cargo.lock

run: $(iso)
	qemu-system-x86_64 -cdrom $(iso) -d int -no-shutdown -no-reboot

check: $(image) rust_build
	cargo clippy 
	cargo audit

rust_build:
	cargo build --target src/$(target).json --release

iso: $(iso)

$(isodir)/third-party-licenses.html: about.toml about.hbs $(isodir)
	cargo about generate about.hbs -o $@

$(isodir)/EFI/BOOT:
	mkdir -p $@

$(isodir): $(isodir)/EFI/BOOT

build: $(isodir)

$(isodir)/memory_manager.elf: rust_build
	cp target/$(target)/$(config)/micros_memory_manager $@

$(isodir)/limine.cfg: $(limine_cfg) $(image) $(isodir)/memory_manager.elf
	cat $(limine_cfg) | sed "s/kernel_hash/`b2sum $(image) | sed 's/  .*//'`/" | sed "s/memory_manager_hash/`b2sum $(isodir)/memory_manager.elf | sed 's/  .*//'`/" > $@

$(isodir)/LICENSE: LICENSE
	cp LICENSE $(isodir)/

$(isodir)/limine-bios-cd.bin: $(isodir)
	cp limine/limine-bios-cd.bin $(isodir)/

$(isodir)/limine-bios.sys: $(isodir)
	cp limine/limine-bios.sys $(isodir)/

build/BOOTX64.EFI: $(isodir)/limine.cfg limine/limine
	cp limine/BOOTX64.EFI build/
	limine/limine enroll-config $@ `b2sum $(isodir)/limine.cfg | sed 's/  .*//'`

$(isodir)/EFI/BOOT/BOOTX64.EFI: $(isodir)/EFI/BOOT $(key) $(cert) build/BOOTX64.EFI
	sbsign --key $(key) --cert $(cert) --output $@ build/BOOTX64.EFI

$(isodir)/limine-uefi-cd.bin: $(isodir)/EFI/BOOT/BOOTX64.EFI $(image) $(isodir)/memory_manager.elf $(isodir)/limine.cfg
	dd if=/dev/zero of=$@ bs=512 count=2880 2>/dev/null
	mformat -i $@ -f 1440 -N 12345678 ::
	mcopy -D o -s -m -i $@ $(isodir)/EFI ::
	mcopy -D o -m -i $@ $(image) ::
	mcopy -D o -m -i $@ $(isodir)/memory_manager.elf ::
	mcopy -D o -m -i $@ $(isodir)/limine.cfg ::


$(iso): $(image) $(isodir)/limine.cfg $(isodir)/LICENSE $(isodir)/third-party-licenses.html $(isodir)/memory_manager.elf $(isodir)/limine-uefi-cd.bin $(isodir)/limine-bios-cd.bin $(isodir)/limine-bios.sys $(isodir)/EFI/BOOT/BOOTX64.EFI limine/limine
	xorriso -as mkisofs -b limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table --efi-boot limine-uefi-cd.bin -efi-boot-part --efi-boot-image --protective-msdos-label $(isodir) -o $@
	limine/limine bios-install $@


$(image): $(assembly_object_files) $(linker_script) rust_build $(isodir)
	ld.lld -n -s -T $(linker_script) -o $(image) $(assembly_object_files) $(kernel)

build/src/micros_kernel/%.o: src/micros_kernel/%.asm
	mkdir -p $(shell dirname $@)
	nasm -felf64 $< -o $@

kernellinecount:
	cloc src/micros_kernel src/frame_allocation --exclude-lang=TOML

