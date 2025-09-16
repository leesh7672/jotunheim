// build.rs — force ELF64 so extern/relocs work
use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=asm/x86_64/isr_stubs.asm");
    println!("cargo:rerun-if-changed=asm/x86_64/context_switch.asm");
    println!("cargo:rerun-if-changed=asm/x86_64/kthread-trampoline.asm");
    println!("cargo:rerun-if-changed=asm/x86_64/ap_trampoline.asm");

    let target = env::var("TARGET").unwrap_or_default();
    if !target.starts_with("x86_64-") {
        println!("cargo:warning=Skipping ASM for non-x86_64 target: {target}");
        return;
    }

    let mut build = nasm_rs::Build::new();

    // ---- IMPORTANT: force ELF64 (don’t rely on defaults) ----
    // Either of these work; use ONE (prefer the first):
    // 1) If your nasm-rs has .format():
    // build.format("elf64");
    // 2) Flag form (works on all versions):
    build.flag("-f").flag("elf64");
    // ---------------------------------------------------------

    build.include("asm/x86_64");

    if env::var("PROFILE").as_deref() == Ok("debug") {
        build.debug(true);
        build.flag("-w+all");
    }

    build
        .file("asm/x86_64/isr_stubs.asm")
        .file("asm/x86_64/context_switch.asm")
        .file("asm/x86_64/kthread_trampoline.asm")
        .file("asm/x86_64/ap_trampoline.asm");

    if let Err(e) = build.compile("arch_x86_64_asm") {
        panic!("NASM build failed: {e}");
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=arch_x86_64_asm");
}
