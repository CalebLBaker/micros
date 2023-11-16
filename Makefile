arch ?= amd64
config ?= release
target ?= x86_64-unknown-none
image := build/micros-$(arch).bin
iso := build/micros-$(arch).iso

linker_script := image/linker.ld
grub_cfg := image/grub.cfg
assembly_source_files := $(wildcard arch/$(arch)/*.asm)
assembly_object_files := $(patsubst arch/$(arch)/%.asm, build/arch/$(arch)/%.o, $(assembly_source_files))
kernel := target/$(target)/$(config)/libmicros_kernel_amd64.a

.PHONY: all clean run iso build_kernel

all: $(image)

clean:
	@rm -r build
	@rm -r target
	@rm Cargo.lock

run: $(iso)
	@qemu-system-x86_64 -cdrom $(iso) -d int -no-shutdown -no-reboot

check: $(image)
	@cargo clippy 
	@cargo audit

build_kernel:
	@cargo build --target arch/$(arch)/$(target).json --release

iso: $(iso)

build/third-party-licenses.html: about.toml about.hbs
	@cargo about generate about.hbs -o build/third-party-licenses.html

$(iso): $(image) $(grub_cfg) LICENSE build/third-party-licenses.html
	@mkdir -p build/isofiles/boot/grub
	@cp $(image) build/isofiles/boot/micros.bin
	@cp $(grub_cfg) build/isofiles/boot/grub
	@cp LICENSE build/isofiles/
	@cp build/third-party-licenses.html build/isofiles/
	@grub-mkrescue -o $(iso) build/isofiles 2> /dev/null
	@rm -r build/isofiles

$(image): $(assembly_object_files) $(linker_script) build_kernel
	@ld -n -T $(linker_script) -o $(image) $(assembly_object_files) $(kernel)

build/arch/$(arch)/%.o: arch/$(arch)/%.asm
	@mkdir -p $(shell dirname $@)
	@nasm -felf64 $< -o $@

