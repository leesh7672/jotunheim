; =======================================================================
; asm/x86_64/isr_stubs.asm — NASM, ELF64 SysV, 64-bit interrupt stubs
; Calls Rust handlers:
;   void isr_default_rust(uint64_t vec, uint64_t err);
;   void isr_pf_rust(uint64_t vec, uint64_t err);
;   void isr_gp_rust(uint64_t vec, uint64_t err);
;   void isr_ud_rust(uint64_t vec, uint64_t err);
; =======================================================================

BITS 64
default rel
section .text align=16

; ---- exported symbols (names must match Rust externs) ----
global isr_default_stub
global isr_gp_stub
global isr_pf_stub
global isr_ud_stub
global isr_df_stub

extern isr_default_rust
extern isr_pf_rust
extern isr_gp_rust
extern isr_ud_rust

%macro PUSH_CALLER 0
    push rax
    push rcx
    push rdx
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
%endmacro

%macro POP_CALLER 0
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rax
%endmacro

; Default ISR (no CPU error code): vec=0xFF, err=0
isr_default_stub:
    PUSH_CALLER
    mov  rdi, 0xFF
    xor  rsi, rsi
    call isr_default_rust
    POP_CALLER
    iretq

; #GP (13) — CPU pushes error code
isr_gp_stub:
    PUSH_CALLER
    mov  rdi, 13
    mov  rsi, [rsp + 9*8]
    call isr_gp_rust
    POP_CALLER
    add  rsp, 8
    iretq

; #PF (14) — CPU pushes error code
isr_pf_stub:
    PUSH_CALLER
    mov  rdi, 14
    mov  rsi, [rsp + 9*8]
    call isr_pf_rust
    POP_CALLER
    add  rsp, 8
    iretq

; #UD (6) — no error code
isr_ud_stub:
    PUSH_CALLER
    mov  rdi, 6
    xor  rsi, rsi
    call isr_ud_rust
    POP_CALLER
    iretq

; #DF — diverging (IDT must use IST1)
isr_df_stub:
    cli
.hang:
    hlt
    jmp .hang
