arch ?= amd64
config ?= release
target ?= x86_64-unknown-none
image := build/micros-$(arch).elf
iso := build/micros-$(arch).iso

linker_script := micros_kernel_common/linker.ld
grub_cfg := grub.cfg
assembly_source_files := $(wildcard arch/$(arch)/*.asm)
assembly_object_files := $(patsubst arch/$(arch)/%.asm, build/arch/$(arch)/%.o, $(assembly_source_files))
kernel := target/$(target)/$(config)/libmicros_kernel_amd64.a

.PHONY: all clean run iso rust_build

all: $(iso)

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
	@cargo build --target arch/$(arch)/$(target).json --release

iso: $(iso)

build/third-party-licenses.html: about.toml about.hbs
	@cargo about generate about.hbs -o build/third-party-licenses.html

$(iso): $(image) $(grub_cfg) LICENSE build/third-party-licenses.html rust_build
	@rm -rf build/isofiles
	@mkdir -p build/isofiles/boot/grub
	@cp $(image) build/isofiles/boot/micros.elf
	@cp target/$(target)/$(config)/micros_memory_manager_$(arch) build/isofiles/boot/memory_manager.elf
	@cp $(grub_cfg) build/isofiles/boot/grub
	@cp LICENSE build/isofiles/
	@cp build/third-party-licenses.html build/isofiles/
	@grub-mkrescue -o $(iso) build/isofiles 2> /dev/null

$(image): $(assembly_object_files) $(linker_script) rust_build
	@ld.lld -n -s -T $(linker_script) -o $(image) $(assembly_object_files) $(kernel)

build/arch/$(arch)/%.o: arch/$(arch)/%.asm
	@mkdir -p $(shell dirname $@)
	@nasm -felf64 $< -o $@

kernellinecount:
	cloc micros_kernel_common arch/amd64/micros_kernel_amd64 arch/amd64/boot.asm arch/amd64/long_mode_init.asm --exclude-lang=TOML

