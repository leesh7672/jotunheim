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
global isr_bp_stub
global isr_timer_stub
global isr_spurious_stub
global isr_db_stub

; ---------------- External Rust targets ----------------
extern isr_default_rust        ; fn(u64, u64) -> ()
extern isr_gp_rust             ; fn(u64, u64) -> !
extern isr_pf_rust             ; fn(u64, u64, u64) -> !
extern isr_df_rust             ; fn(u64, u64) -> !
extern isr_ud_rust             ; fn(u64, u64, u64, u64) -> !
extern isr_timer_rust          ; fn() -> ()
extern isr_spurious_rust       ; fn() -> ()
extern isr_bp_rust       ; fn() -> ()
extern isr_db_rust

extern preempt

%macro PUSH_VOLATILES 0
    push r15
    push r14
    push r13
    push r12
    push r11
    push r10
    push r9
    push r8
    push rsi
    push rdi
    push rbp
    push rdx
    push rcx
    push rbx
    push rax
%endmacro

%macro POP_VOLATILES 0
    pop rax
    pop rbx
    pop rcx
    pop rdx
    pop rbp
    pop rdi
    pop rsi
    pop r8
    pop r9
    pop r10
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15
%endmacro

%macro CALL_ALIGN 1
    mov     rbx, rsp
    and     rbx, 15            ; rbx = rsp % 16
    sub     rsp, rbx           ; make %rsp 16-aligned for 'call'
    call    %1
    add     rsp, rbx           ; restore
%endmacro


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


; Count of pushes above
%define N_SAVED 15

; IRET frame locations relative to current RSP after PUSH_VOLATILES
;  - No-error exceptions: frame is [RIP, CS, RFLAGS, ...] at rsp + N_SAVED*8
;  - Error-code exceptions: CPU pushes err first, then [RIP, CS, RFLAGS, ...]
%define FRAME   rsp + N_SAVED*8      ; -> [RIP, CS, RFLAGS, ...]
%define FRAME_ERRTOP  rsp + N_SAVED*8      ; -> error code
%define FRAME_ERR     rsp + N_SAVED*8 + 8  ; -> [RIP, CS, RFLAGS, ...]

; Alignment helpers:
; On same-CPL interrupts:
;   - No-error exceptions/timer/spurious: CPU pushes 3 qwords → entry RSP%16 == 8
;     After 9 pushes (+72), RSP%16 == 0 → subtract 8 so before CALL it's 8.
;   - Error-code exceptions: CPU pushes 4 qwords → entry RSP%16 == 0
;     After 9 pushes (+72), RSP%16 == 8 → already correct, call directly.
%macro CALL_ALIGN 1
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
    CALL_ALIGN isr_default_rust
    POP_VOLATILES
    iretq
    
isr_bp_stub:
    ; RSP -> [ RIP | CS | RFLAGS ]
    lea     r11, [rsp]              ; HBASE = RSP (points at RIP)
    sub     rsp, TF_SIZE            ; reserve TrapFrame

    ; Save GPRs
    mov [rsp + TF_R15], r15
    mov [rsp + TF_R14], r14
    mov [rsp + TF_R13], r13
    mov [rsp + TF_R12], r12
    mov [rsp + TF_R11], r11         ; we can store HBASE as "r11" value; fine
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

    ; vec/err
    mov qword [rsp + TF_VEC], 3
    mov qword [rsp + TF_ERR], 0

    ; Copy HW frame (RIP/CS/RFLAGS) from HBASE
    mov rax, [r11 + 0]              ; RIP
    mov [rsp + TF_RIP], rax
    mov rax, [r11 + 8]              ; CS
    mov [rsp + TF_CS], rax
    mov rax, [r11 + 16]             ; RFLAGS
    mov [rsp + TF_RFLAGS], rax

    ; Synthesize SS, RSP in TrapFrame
    lea rax, [r11 + 0]              ; "return frame" base (RSP at exception)
    mov [rsp + TF_RSP], rax
    mov ax, ss
    movzx eax, ax
    mov [rsp + TF_SS], rax

    ; Call Rust
    mov rdi, rsp                    ; &TrapFrame
    call isr_bp_rust

    ; Restore GPRs
    mov r15, [rsp + TF_R15]
    mov r14, [rsp + TF_R14]
    mov r13, [rsp + TF_R13]
    mov r12, [rsp + TF_R12]
    mov r11, [rsp + TF_R11]
    mov r10, [rsp + TF_R10]
    mov  r9, [rsp + TF_R9 ]
    mov  r8, [rsp + TF_R8 ]
    mov rsi, [rsp + TF_RSI]
    mov rdi, [rsp + TF_RDI]
    mov rbp, [rsp + TF_RBP]
    mov rdx, [rsp + TF_RDX]
    mov rcx, [rsp + TF_RCX]
    mov rbx, [rsp + TF_RBX]
    mov rax, [rsp + TF_RAX]

    ; Write adjusted RIP/CS/RFLAGS back to HW frame at HBASE (= saved TF_RSP)
    mov rdx, [rsp + TF_RSP]         ; rdx = HBASE
    mov rax, [rsp + TF_RIP]
    mov [rdx + 0],  rax
    mov rax, [rsp + TF_CS]
    mov [rdx + 8],  rax
    mov rax, [rsp + TF_RFLAGS]
    mov [rdx + 16], rax

    ; Return to HW frame
    mov rsp, rdx
    iretq

isr_db_stub:
    ; RSP -> [ RIP | CS | RFLAGS ]
    lea     r11, [rsp]              ; HBASE = RSP (points at RIP)
    sub     rsp, TF_SIZE            ; reserve TrapFrame

    ; Save GPRs
    mov [rsp + TF_R15], r15
    mov [rsp + TF_R14], r14
    mov [rsp + TF_R13], r13
    mov [rsp + TF_R12], r12
    mov [rsp + TF_R11], r11         ; we can store HBASE as "r11" value; fine
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

    ; vec/err
    mov qword [rsp + TF_VEC], 3
    mov qword [rsp + TF_ERR], 0

    ; Copy HW frame (RIP/CS/RFLAGS) from HBASE
    mov rax, [r11 + 0]              ; RIP
    mov [rsp + TF_RIP], rax
    mov rax, [r11 + 8]              ; CS
    mov [rsp + TF_CS], rax
    mov rax, [r11 + 16]             ; RFLAGS
    mov [rsp + TF_RFLAGS], rax

    ; Synthesize SS, RSP in TrapFrame
    lea rax, [r11 + 0]              ; "return frame" base (RSP at exception)
    mov [rsp + TF_RSP], rax
    mov ax, ss
    movzx eax, ax
    mov [rsp + TF_SS], rax

    ; Call Rust
    mov rdi, rsp                    ; &TrapFrame
    call isr_db_rust

    ; Restore GPRs
    mov r15, [rsp + TF_R15]
    mov r14, [rsp + TF_R14]
    mov r13, [rsp + TF_R13]
    mov r12, [rsp + TF_R12]
    mov r11, [rsp + TF_R11]
    mov r10, [rsp + TF_R10]
    mov  r9, [rsp + TF_R9 ]
    mov  r8, [rsp + TF_R8 ]
    mov rsi, [rsp + TF_RSI]
    mov rdi, [rsp + TF_RDI]
    mov rbp, [rsp + TF_RBP]
    mov rdx, [rsp + TF_RDX]
    mov rcx, [rsp + TF_RCX]
    mov rbx, [rsp + TF_RBX]
    mov rax, [rsp + TF_RAX]

    ; Write adjusted RIP/CS/RFLAGS back to HW frame at HBASE (= saved TF_RSP)
    mov rdx, [rsp + TF_RSP]         ; rdx = HBASE
    mov rax, [rsp + TF_RIP]
    mov [rdx + 0],  rax
    mov rax, [rsp + TF_CS]
    mov [rdx + 8],  rax
    mov rax, [rsp + TF_RFLAGS]
    mov [rdx + 16], rax

    ; Return to HW frame
    mov rsp, rdx
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
    push    qword 3            ; vec = 3 (#BP)
    push    qword 0            ; err = 0 (synthetic)
    lea     rdi, [rsp]         ; arg0 = &TrapFrame
    CALL_ALIGN isr_bp_rust     ; Rust may tweak tf->rflags (TF) or RIP, etc.
    add     rsp, 16            ; pop our synthetic (vec, err)
    POP_VOLATILES
    iretq

; ============================================================================ ;
; #UD (vector 6) — NO error code
; Pass vec, err=0, rip, and interrupted rsp to Rust.
; ============================================================================ ;
isr_ud_stub:
    PUSH_VOLATILES
    mov rdi, rsp            
    mov     rdi, 8
    mov     rsi, [FRAME_ERRTOP + 0]   ; err (0 by hardware)
    CALL_ALIGN_ERR isr_df_rust         ; -> !
.hang_ud:
    cli
    hlt
    jmp     .hang_ud

; ============================================================================ ;
; LAPIC Timer (vector 0x20) — NO error code
; Must EOI in Rust (or here after the call).
; ============================================================================ ;
; extern isr_timer_rust          ; fn(hbase: u64) -> *const PreemptPack

isr_timer_stub:
    PUSH_VOLATILES
    sub     rsp, 8
    call    isr_timer_rust
    add     rsp, 8
    POP_VOLATILES
    iretq


; ============================================================================ ;
; LAPIC Spurious — NO error code
; Typically no EOI required, but harmless if Rust does it.
; ============================================================================ ;
isr_spurious_stub:
    PUSH_VOLATILES
    CALL_ALIGN isr_spurious_rust
    POP_VOLATILES
    iretq
