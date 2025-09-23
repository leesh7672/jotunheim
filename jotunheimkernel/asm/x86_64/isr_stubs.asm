; asm/x86_64/isr_stubs.asm
; NASM 64-bit ISR stubs for Jotunheim
; Matches Rust TrapFrame:
;   r15..rax, vec, err, rip, cs, rflags, rsp, ss
; Same-CPL assumptions:
;   No-error exceptions: stack = [RIP][CS][RFLAGS]
;   With-error exceptions: stack = [ERR][RIP][CS][RFLAGS]
; We synthesize TF.rsp = &RIP slot; TF.ss = current SS.
; We pass one arg to Rust: rdi = &TrapFrame.

[BITS 64]
default rel

section .text

; ---------------- Exported stubs ----------------
global isr_default_stub
global isr_gp_stub
global isr_pf_stub
global isr_df_stub
global isr_ud_stub
global isr_bp_stub
global isr_db_stub
global isr_timer_stub
global isr_spurious_stub

; ---------------- External Rust handlers (all take *mut TrapFrame) ----------
extern isr_default_rust        ; fn(*mut TrapFrame) -> !
extern isr_gp_rust             ; fn(*mut TrapFrame) -> !
extern isr_pf_rust             ; fn(*mut TrapFrame) -> !
extern isr_df_rust             ; fn(*mut TrapFrame) -> !
extern isr_ud_rust             ; fn(*mut TrapFrame) -> !
extern isr_bp_rust             ; fn(*mut TrapFrame) -> ()
extern isr_db_rust             ; fn(*mut TrapFrame) -> ()
extern isr_timer_rust          ; fn() -> ()
extern isr_spurious_rust       ; fn() -> ()

; ---------------- TrapFrame field offsets (bytes) ----------------
%define TF_R15      (0*8)
%define TF_R14      (1*8)
%define TF_R13      (2*8)
%define TF_R12      (3*8)
%define TF_R11      (4*8)
%define TF_R10      (5*8)
%define TF_R9       (6*8)
%define TF_R8       (7*8)
%define TF_RSI      (8*8)
%define TF_RDI      (9*8)
%define TF_RBP     (10*8)
%define TF_RDX     (11*8)
%define TF_RCX     (12*8)
%define TF_RBX     (13*8)
%define TF_RAX     (14*8)
%define TF_VEC     (15*8)
%define TF_ERR     (16*8)
%define TF_RIP     (17*8)
%define TF_CS      (18*8)
%define TF_RFLAGS  (19*8)
%define TF_RSP     (20*8)
%define TF_SS      (21*8)
%define TF_SIZE    (22*8)

; ---------------- Helpers ----------------

; SysV call alignment helper:
; Before CALL: RSP%16 must be 8 (so inside callee it's 16).
%macro CALL_SYSV 1
    mov     rax, rsp
    and     rax, 15
    cmp     rax, 8
    je      %%aligned
    sub     rsp, 8
    call    %1
    add     rsp, 8
    jmp     %%done
%%aligned:
    call    %1
%%done:
%endmacro

; Save GPR snapshot into TF (at [rsp]), not onto the CPU stack.
%macro SAVE_GPRS_TO_TF 0
    mov [rsp + TF_R15], r15
    mov [rsp + TF_R14], r14
    mov [rsp + TF_R13], r13
    mov [rsp + TF_R12], r12
    mov [rsp + TF_R11], r11
    mov [rsp + TF_R10], r10
    mov [rsp + TF_R9 ], r9
    mov [rsp + TF_R8 ], r8
    mov [rsp + TF_RSI], rsi
    mov [rsp + TF_RDI], rdi
    mov [rsp + TF_RBP], rbp
    mov [rsp + TF_RDX], rdx
    mov [rsp + TF_RCX], rcx
    mov [rsp + TF_RBX], rbx
    mov [rsp + TF_RAX], rax
%endmacro

; Restore GPRs from TF
%macro RESTORE_GPRS_FROM_TF 0
    mov  r15, [rsp + TF_R15]
    mov  r14, [rsp + TF_R14]
    mov  r13, [rsp + TF_R13]
    mov  r12, [rsp + TF_R12]
    mov  r11, [rsp + TF_R11]
    mov  r10, [rsp + TF_R10]
    mov   r9, [rsp + TF_R9 ]
    mov   r8, [rsp + TF_R8 ]
    mov  rsi, [rsp + TF_RSI]
    mov  rdi, [rsp + TF_RDI]
    mov  rbp, [rsp + TF_RBP]
    mov  rdx, [rsp + TF_RDX]
    mov  rcx, [rsp + TF_RCX]
    mov  rbx, [rsp + TF_RBX]
    mov  rax, [rsp + TF_RAX]
%endmacro

; ----- NO-ERROR exceptions: entry stack = [RIP][CS][RFLAGS] -----
; We capture r12 = &RIP BEFORE allocating TF space.
%macro BUILD_TF_NO_ERR 1
    mov     r12, rsp              ; r12 = &RIP (top of HW frame)
    sub     rsp, TF_SIZE          ; reserve TrapFrame

    SAVE_GPRS_TO_TF

    ; Copy HW frame from r12
    mov     rax, [r12 + 0]        ; RIP
    mov     [rsp + TF_RIP], rax
    mov     rax, [r12 + 8]        ; CS
    mov     [rsp + TF_CS], rax
    mov     rax, [r12 + 16]       ; RFLAGS
    mov     [rsp + TF_RFLAGS], rax

    ; Synthesize RSP/SS at trap time
    mov     [rsp + TF_RSP], r12   ; interrupted RSP points to RIP slot
    mov     ax, ss
    movzx   eax, ax
    mov     [rsp + TF_SS], rax

    ; Vector / Error
    mov     qword [rsp + TF_VEC], %1
    mov     qword [rsp + TF_ERR], 0
%endmacro

; ----- WITH-ERROR exceptions: entry = [ERR][RIP][CS][RFLAGS] -----
; We capture r12 = &RIP, r13 = &ERR BEFORE allocating TF space.
%macro BUILD_TF_WITH_ERR 1
    lea     r12, [rsp + 8]        ; r12 = &RIP
    mov     r13, rsp              ; r13 = &ERR
    sub     rsp, TF_SIZE

    SAVE_GPRS_TO_TF

    ; Copy HW frame and error
    mov     rax, [r12 + 0]        ; RIP
    mov     [rsp + TF_RIP], rax
    mov     rax, [r12 + 8]        ; CS
    mov     [rsp + TF_CS], rax
    mov     rax, [r12 + 16]       ; RFLAGS
    mov     [rsp + TF_RFLAGS], rax
    mov     rax, [r13 + 0]        ; ERR
    mov     [rsp + TF_ERR], rax

    ; Synthesize RSP/SS
    mov     [rsp + TF_RSP], r12   ; &RIP (skip ERR on return)
    mov     ax, ss
    movzx   eax, ax
    mov     [rsp + TF_SS], rax

    mov     qword [rsp + TF_VEC], %1
%endmacro

; Write back possibly-updated RIP/CS/RFLAGS into HW frame at r12
%macro WRITE_BACK_HW 0
    mov     rax, [rsp + TF_RIP]
    mov     [r12 + 0],  rax
    mov     rax, [rsp + TF_CS]
    mov     [r12 + 8],  rax
    mov     rax, [rsp + TF_RFLAGS]
    mov     [r12 + 16], rax
%endmacro

; =============================================================================
; Stubs
; =============================================================================

; Default catch-all (no error), vec=0
isr_default_stub:
    BUILD_TF_NO_ERR 0
    mov     rdi, rsp                ; &TrapFrame
    CALL_SYSV isr_default_rust
    WRITE_BACK_HW
    RESTORE_GPRS_FROM_TF
    mov     rsp, r12                ; back to HW frame (&RIP)
    iretq

; #BP (3) — no error
isr_bp_stub:
    BUILD_TF_NO_ERR 3
    mov     rdi, rsp
    CALL_SYSV isr_bp_rust
    WRITE_BACK_HW
    RESTORE_GPRS_FROM_TF
    mov     rsp, r12
    iretq

; #DB (1) — no error
isr_db_stub:
    BUILD_TF_NO_ERR 1
    mov     rdi, rsp
    CALL_SYSV isr_db_rust
    WRITE_BACK_HW
    RESTORE_GPRS_FROM_TF
    mov     rsp, r12
    iretq

; #UD (6) — no error
isr_ud_stub:
    BUILD_TF_NO_ERR 6
    mov     rdi, rsp
    CALL_SYSV isr_ud_rust
    WRITE_BACK_HW
    RESTORE_GPRS_FROM_TF
    mov     rsp, r12
    iretq

; #GP (13) — with error
isr_gp_stub:
    BUILD_TF_WITH_ERR 13
    mov     rdi, rsp
    CALL_SYSV isr_gp_rust
    WRITE_BACK_HW
    RESTORE_GPRS_FROM_TF
    mov     rsp, r12                ; skip ERR (now top is RIP)
    iretq

; #PF (14) — with error
isr_pf_stub:
    BUILD_TF_WITH_ERR 14
    mov     rdi, rsp
    CALL_SYSV isr_pf_rust
    WRITE_BACK_HW
    RESTORE_GPRS_FROM_TF
    mov     rsp, r12
    iretq

; #DF (8) — with error (hardware pushes 0)
isr_df_stub:
    BUILD_TF_WITH_ERR 8
    mov     rdi, rsp
    CALL_SYSV isr_df_rust
    WRITE_BACK_HW
    RESTORE_GPRS_FROM_TF
    mov     rsp, r12
    iretq

; LAPIC Timer (no error) — minimal edge (no TF). If you want TF-based preemption,
; convert to BUILD_TF_NO_ERR 0x20 and pass &TrapFrame instead.
isr_timer_stub:
    BUILD_TF_NO_ERR 0x20
    mov     rdi, rsp
    CALL_SYSV isr_timer_rust
    WRITE_BACK_HW
    RESTORE_GPRS_FROM_TF
    mov     rsp, r12
    iretq

; LAPIC Spurious (no error)
isr_spurious_stub:
    CALL_SYSV isr_spurious_rust
    iretq
