fn main() {
    println!("cargo:rustc-link-arg=--script=src/micros_memory_manager/linker.ld");
}
