arch ?= amd64
config ?= release
target ?= x86_64-unknown-none
image := build/micros-$(arch).bin
iso := build/micros-$(arch).iso

linker_script := image/linker.ld
grub_cfg := image/grub.cfg
assembly_source_files := $(wildcard bootstrap/src/arch/$(arch)/*.asm)
assembly_object_files := $(patsubst bootstrap/src/arch/$(arch)/%.asm, build/bootstrap/arch/$(arch)/%.o, $(assembly_source_files))
kernel_source_files := $(wildcard kernel/src/*.rs) $(wildcard kernel/src/arch/*.rs) $(wildcard kernel/src/arch/amd64/*.rs)
display_source_files := $(wildcard display-driver/src/arch/$(arch)/*.rs)
kernel := target/$(target)/$(config)/libkernel.a

.PHONY: all clean run iso

all: $(image)

clean:
	@rm -r build
	@rm -r target
	@rm Cargo.lock

run: $(iso)
	@qemu-system-x86_64 -cdrom $(iso) -d int -no-shutdown -no-reboot

$(kernel): $(kernel_source_files) $(display_source_files) Cargo.toml kernel/Cargo.toml display-daemon/Cargo.toml
	@cargo build --target kernel/arch/$(arch)/$(target).json --release

iso: $(iso)

$(iso): $(image) $(grub_cfg)
	@mkdir -p build/isofiles/boot/grub
	@cp $(image) build/isofiles/boot/micros.bin
	@cp $(grub_cfg) build/isofiles/boot/grub
	@grub-mkrescue -o $(iso) build/isofiles 2> /dev/null
	@rm -r build/isofiles

$(image): $(assembly_object_files) $(linker_script) $(kernel)
	@ld -n -T $(linker_script) -o $(image) $(assembly_object_files) $(kernel)

build/bootstrap/arch/$(arch)/%.o: bootstrap/src/arch/$(arch)/%.asm
	@mkdir -p $(shell dirname $@)
	@nasm -felf64 $< -o $@

