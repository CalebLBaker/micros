genrule(
  name = "kernel",
  srcs = glob(["src/**/*.rs"]) + [
    "Cargo.toml",
    "arch/amd64/x86_64-unknown-none.json",
  ],
  outs = ["x86_64-unknown-none/release/libkernel.a"],
  cmd = "cd $(location Cargo.toml) && cargo build --target kernel/arch/x86_64-unknown-none.json --release --target-dir $(RULEDIR)",
  visibility = ["//:__pkg__"],
)
