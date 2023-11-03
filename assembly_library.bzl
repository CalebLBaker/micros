def assembly_library(name):
  native.genrule(
    name = name,
    srcs = [name + ".asm"],
    outs = [name + ".o"],
    cmd = "nasm -felf64 $< -o $@",
    visibility = ["//:__pkg__"],
  )

