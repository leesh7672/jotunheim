; asm/x86_64/isr_stubs.asm
; NASM 64-bit ISR stubs for Jotunheim
; Builds to an ELF64 .o via build.rs (nasm-rs).
; Conventions:
;   - SysV x86_64 ABI for Rust targets
;   - Caller-saved registers are pushed/popped in stubs
;   - Stack alignment: ensure RSP % 16 == 8 before `call`
;   - Error-code exceptions (#GP, #PF, #DF) read error code from stack
;   - No-error exceptions (e.g., #UD) pass err = 0
;   - #UD also passes RIP and interrupted RSP to Rust

[bits 64]
default rel

section .text

; ---------------- Exported stubs ----------------
global isr_default_stub
global isr_gp_stub
global isr_pf_stub
global isr_df_stub
global isr_ud_stub
global isr_timer_stub
global isr_spurious_stub

; ---------------- External Rust targets ----------------
extern isr_default_rust        ; fn(u64, u64) -> ()
extern isr_gp_rust             ; fn(u64, u64) -> !
extern isr_pf_rust             ; fn(u64, u64, u64) -> !
extern isr_df_rust             ; fn(u64, u64) -> !
extern isr_ud_rust             ; fn(u64, u64, u64, u64) -> !
extern isr_timer_rust          ; fn() -> ()
extern isr_spurious_rust       ; fn() -> ()

; ---------------- Common helpers ----------------
; We save caller-saved registers per SysV: rax, rcx, rdx, rsi, rdi, r8..r11
%macro PUSH_VOLATILES 0
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

%macro POP_VOLATILES 0
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

; Count of pushes above
%define N_SAVED 9

; IRET frame locations relative to current RSP after PUSH_VOLATILES
;  - No-error exceptions: frame is [RIP, CS, RFLAGS, ...] at rsp + N_SAVED*8
;  - Error-code exceptions: CPU pushes err first, then [RIP, CS, RFLAGS, ...]
%define FRAME_NOERR   rsp + N_SAVED*8      ; -> [RIP, CS, RFLAGS, ...]
%define FRAME_ERRTOP  rsp + N_SAVED*8      ; -> error code
%define FRAME_ERR     rsp + N_SAVED*8 + 8  ; -> [RIP, CS, RFLAGS, ...]

; Alignment helpers:
; On same-CPL interrupts:
;   - No-error exceptions/timer/spurious: CPU pushes 3 qwords → entry RSP%16 == 8
;     After 9 pushes (+72), RSP%16 == 0 → subtract 8 so before CALL it's 8.
;   - Error-code exceptions: CPU pushes 4 qwords → entry RSP%16 == 0
;     After 9 pushes (+72), RSP%16 == 8 → already correct, call directly.
%macro CALL_ALIGN_NOERR 1
    sub rsp, 8
    call %1
    add rsp, 8
%endmacro

%macro CALL_ALIGN_ERR 1
    ; already aligned (RSP%16 == 8)
    call %1
%endmacro

; ============================================================================ ;
; Default stub (no error code) — used for catch-all vectors that don't push err
; ============================================================================ ;
isr_default_stub:
    PUSH_VOLATILES
    mov     rdi, 0                ; vec = 0 (unknown/default)
    xor     rsi, rsi              ; err = 0
    CALL_ALIGN_NOERR isr_default_rust
    POP_VOLATILES
    iretq

; ============================================================================ ;
; #GP (vector 13) — HAS error code
; Stack on entry: [err][RIP][CS][RFLAGS][(SS,RSP if CPL change)]
; ============================================================================ ;
isr_gp_stub:
    PUSH_VOLATILES
    mov     rdi, 13
    mov     rsi, [FRAME_ERRTOP + 0]   ; err
    CALL_ALIGN_ERR isr_gp_rust        ; -> !
.hang_gp:
    cli
    hlt
    jmp     .hang_gp

; ============================================================================ ;
; #PF (vector 14) — HAS error code
; Rust signature expects RIP as 3rd arg.
; ============================================================================ ;
isr_pf_stub:
    PUSH_VOLATILES
    mov     rdi, 14
    mov     rsi, [FRAME_ERRTOP + 0]   ; err
    mov     rdx, [FRAME_ERR + 0]      ; rip (first qword of IRET frame)
    CALL_ALIGN_ERR isr_pf_rust         ; -> !
.hang_pf:
    cli
    hlt
    jmp     .hang_pf

; ============================================================================ ;
; #DF (vector 8) — HAS error code (hardware pushes 0)
; ============================================================================ ;
isr_df_stub:
    PUSH_VOLATILES
    mov     rdi, 8
    mov     rsi, [FRAME_ERRTOP + 0]   ; err (0 by hardware)
    CALL_ALIGN_ERR isr_df_rust         ; -> !
.hang_df:
    cli
    hlt
    jmp     .hang_df

; ============================================================================ ;
; #UD (vector 6) — NO error code
; Pass vec, err=0, rip, and interrupted rsp to Rust.
; ============================================================================ ;
isr_ud_stub:
    PUSH_VOLATILES
    mov     rdi, 6                    ; vec
    xor     rsi, rsi                  ; err = 0
    lea     rcx, [FRAME_NOERR]        ; rcx = interrupted RSP (points to [RIP,CS,RFLAGS,...])
    mov     rdx, [rcx + 0]            ; rdx = RIP   (NOT +8; +8 would be CS)
    CALL_ALIGN_NOERR isr_ud_rust      ; -> !
.hang_ud:
    cli
    hlt
    jmp     .hang_ud

; ============================================================================ ;
; LAPIC Timer (vector 0x20) — NO error code
; Must EOI in Rust (or here after the call).
; ============================================================================ ;
isr_timer_stub:
    PUSH_VOLATILES
    CALL_ALIGN_NOERR isr_timer_rust
    POP_VOLATILES
    iretq

; ============================================================================ ;
; LAPIC Spurious — NO error code
; Typically no EOI required, but harmless if Rust does it.
; ============================================================================ ;
isr_spurious_stub:
    PUSH_VOLATILES
    CALL_ALIGN_NOERR isr_spurious_rust
    POP_VOLATILES
    iretq
