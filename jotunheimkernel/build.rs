use std::env;
use std::path::PathBuf;

fn main() {
    // Adjust if your .asm path differs:
    let asm_path = "asm/x86_64/isr_stubs.asm";

    // Assemble with explicit format (elf64). Do NOT ignore errors.
    let mut b = nasm_rs::Build::new();
    b.file(asm_path).flag("-felf64").flag("-w+all").flag("-g");
    b.compile("isr_stubs").expect("NASM assembly failed");

    // Belt-and-suspenders: ensure the linker sees the static lib
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=isr_stubs");

    // Rebuild if the ASM changes
    println!("cargo:rerun-if-changed={}", asm_path);
}
