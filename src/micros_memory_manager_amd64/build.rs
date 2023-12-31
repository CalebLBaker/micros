fn main() {
    println!("cargo:rustc-link-arg=--script=src/micros_memory_manager_amd64/linker.ld");
}
