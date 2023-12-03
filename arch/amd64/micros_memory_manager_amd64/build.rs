fn main() {
    println!("cargo:rustc-link-arg=--script=arch/amd64/micros_memory_manager_amd64/linker.ld");
}
