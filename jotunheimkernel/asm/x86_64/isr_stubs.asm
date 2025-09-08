; asm/x86_64/isr_stubs.asm
; NASM 64-bit ISR stubs for Jotunheim
; Builds to an ELF64 .o via build.rs (nasm-rs).
; Each stub calls a Rust symbol: isr_*_rust(vec:u64, err:u64).
; - Faults that push an error code: #GP, #PF, #DF (DF pushes 0 by hardware).
; - Others (e.g., #UD, timer) have no error code -> we pass 0.
; - Ensure 16-byte stack alignment before calling into Rust.

[bits 64]
default rel

section .text

global isr_default_stub
global isr_gp_stub
global isr_pf_stub
global isr_df_stub
global isr_ud_stub
global isr_timer_stub

extern isr_default_rust
extern isr_gp_rust
extern isr_pf_rust
extern isr_df_rust
extern isr_ud_rust
extern isr_timer_rust

; ---------------- Common helpers ----------------

%macro PUSH_VOLATILES 0
    ; Save caller-saved regs per SysV ABI
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

; Align stack to 16 bytes before `call`:
; We’ve pushed 9 regs (9*8 = 72). The CPU also pushed RIP, CS, RFLAGS,
; and maybe SS,RSP if CPL changed; alignment isn’t guaranteed. Force it by
; subtracting 8 so that after `call` (pushes 8) we end up 16-aligned.
%macro ALIGN_BEFORE_CALL 0
    sub rsp, 8
%endmacro
%macro UNALIGN_AFTER_CALL 0
    add rsp, 8
%endmacro

; ---------------- Default stub (no error code) ----------------
; Used for “catch-all” vectors that don’t push an error code
isr_default_stub:
    PUSH_VOLATILES
    mov rdi, 0                 ; vec (0 means “default/unknown”)
    mov rsi, 0                 ; err = 0
    ALIGN_BEFORE_CALL
    call isr_default_rust
    UNALIGN_AFTER_CALL
    POP_VOLATILES
    iretq

; ---------------- #GP (vector 13) — has error code ----------------
; Stack on entry (top -> bottom): error, RIP, CS, RFLAGS, [SS,RSP if from lower CPL]
isr_gp_stub:
    PUSH_VOLATILES
    mov rdi, 13                ; vec
    mov rsi, [rsp + 9*8 + 0x00] ; error code sits above our 9 pushes
    ALIGN_BEFORE_CALL
    call isr_gp_rust           ; does not return
    UNALIGN_AFTER_CALL
.hang_gp:
    cli
    hlt
    jmp .hang_gp

; ---------------- #PF (vector 14) — has error code ----------------
isr_pf_stub:
    PUSH_VOLATILES
    mov     rdi, 14                  ; vec

    mov     rsi, [rsp + 9*8 + 0x00]  ; err
    mov     rdx, [rsp + 9*8 + 0x08]  ; rip

    ALIGN_BEFORE_CALL
    call    isr_pf_rust             ; does not return
    UNALIGN_AFTER_CALL

.hang_pf:
    cli
    hlt
    jmp .hang_pf

; ---------------- #DF (vector 8) — hardware pushes error code = 0 ----------------
isr_df_stub:
    PUSH_VOLATILES
    mov rdi, 8
    mov rsi, [rsp + 9*8 + 0x00] ; value will be 0 by hardware, but read it regardless
    ALIGN_BEFORE_CALL
    call isr_df_rust           ; does not return
    UNALIGN_AFTER_CALL
.hang_df:
    cli
    hlt
    jmp .hang_df

; ---------------- #UD (vector 6) — no error code ----------------
isr_ud_stub:
    PUSH_VOLATILES
    mov rdi, 6
    mov rsi, 0
    ALIGN_BEFORE_CALL
    call isr_ud_rust           ; does not return
    UNALIGN_AFTER_CALL
.hang_ud:
    cli
    hlt
    jmp .hang_ud

; ---------------- LAPIC Timer (vector 0x20) — no error code ----------------
; Must return with EOI handled in Rust or after the call.
isr_timer_stub:
    PUSH_VOLATILES
    mov rdi, 0x20              ; vec = TIMER_VECTOR
    mov rsi, 0                 ; err = 0
    ALIGN_BEFORE_CALL
    call isr_timer_rust        ; returns
    UNALIGN_AFTER_CALL
    POP_VOLATILES
    iretq
