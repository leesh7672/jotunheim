; =======================================================================
; asm/x86_64/isr_stubs.asm — NASM, ELF64 SysV
; 64-bit interrupt stubs that call into Rust handlers.
; Stack alignment: we add/sub 8 to keep 16-byte alignment for SysV calls.
; =======================================================================

BITS 64
default rel
section .text align=16

; ---- exported symbols (exact names used by Rust externs) ----
global isr_default_stub
global isr_gp_stub
global isr_pf_stub
global isr_ud_stub
global isr_timer_stub
global isr_df_stub

; ---- Rust handlers we call ----
extern isr_default_rust
extern isr_gp_rust
extern isr_pf_rust
extern isr_ud_rust
extern isr_timer_rust

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

; -----------------------------------------------------------------------
; Default ISR (no CPU error code): vec=0xFF, err=0
; -----------------------------------------------------------------------
isr_default_stub:
    PUSH_CALLER
    sub  rsp, 8
    mov  rdi, 0xFF
    xor  rsi, rsi
    call isr_default_rust
    add  rsp, 8
    POP_CALLER
    iretq

; -----------------------------------------------------------------------
; #GP (13) — CPU pushes error code (8 bytes)
; -----------------------------------------------------------------------
isr_gp_stub:
    PUSH_CALLER
    sub  rsp, 8
    mov  rdi, 13
    mov  rsi, [rsp + 8 + 9*8]   ; +8 for our align shim
    call isr_gp_rust
    add  rsp, 8
    POP_CALLER
    add  rsp, 8                 ; drop error code from CPU
    iretq

; -----------------------------------------------------------------------
; #PF (14) — CPU pushes error code (8 bytes)
; -----------------------------------------------------------------------
isr_pf_stub:
    PUSH_CALLER
    sub  rsp, 8
    mov  rdi, 14
    mov  rsi, [rsp + 8 + 9*8]
    call isr_pf_rust
    add  rsp, 8
    POP_CALLER
    add  rsp, 8
    iretq

; -----------------------------------------------------------------------
; #UD (6) — no error code
; -----------------------------------------------------------------------
isr_ud_stub:
    PUSH_CALLER
    sub  rsp, 8
    mov  rdi, 6
    xor  rsi, rsi
    call isr_ud_rust
    add  rsp, 8
    POP_CALLER
    iretq

; -----------------------------------------------------------------------
; LAPIC Timer (vector 0x40) — no error code
; -----------------------------------------------------------------------
isr_timer_stub:
    PUSH_CALLER
    sub  rsp, 8
    mov  rdi, 0x40
    xor  rsi, rsi
    call isr_timer_rust
    add  rsp, 8
    POP_CALLER
    iretq

; -----------------------------------------------------------------------
; #DF — double fault (must use IST in IDT). Keep it non-returning.
; If you later want a Rust handler, you can add one and call it here.
; -----------------------------------------------------------------------
isr_df_stub:
    cli
.hang_df:
    hlt
    jmp .hang_df
