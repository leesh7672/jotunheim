; isr_stubs.asm — complete NASM ISR/IRQ stubs for Jotunheim OS
; SPDX-License-Identifier: JOSSL-1.0
; Copyright (C) 2025 The Jotunheim Project
; 64‑bit, SysV calling convention, ring‑0 only.  
; This version preserves your macro names and fixes context‑switch return:  
; we return on the **target thread’s** stack (TF_RSP), not always the old r12.
;
; Public entry points (labels) you can wire into the IDT:  
;   - isr_de_stub   (Divide Error,    #DE, 0)          
;   - isr_db_stub   (Debug,            #DB, 0)
;   - isr_nmi_stub  (NMI,              NMI, 0)
;   - isr_bp_stub   (Breakpoint,       #BP, 0)
;   - isr_of_stub   (Overflow,         #OF, 0)
;   - isr_br_stub   (BOUND,            #BR, 0)
;   - isr_ud_stub   (Invalid Opcode,   #UD, 0)
;   - isr_nm_stub   (Device NA,        #NM, 0)
;   - isr_df_stub   (Double Fault,     #DF, has error)
;   - isr_ts_stub   (TS,               #TS, has error)
;   - isr_np_stub   (NP,               #NP, has error)
;   - isr_ss_stub   (SS,               #SS, has error)
;   - isr_gp_stub   (GP,               #GP, has error)
;   - isr_pf_stub   (Page Fault,       #PF, has error)
;   - isr_mf_stub   (x87 FP,           #MF, 0)
;   - isr_ac_stub   (Alignment Check,  #AC, has error)
;   - isr_xm_stub   (SIMD FP,          #XM, 0)
;   - isr_virt_stub (Virtualization,   #VE, 0 or vendor)
;   - isr_timer_stub         (LAPIC timer, no error)
;   - isr_spurious_stub      (Spurious IRQ, no error)
;   - isr_apic_error_stub    (LAPIC error, no error)
;   - isr_generic_irq##_stub (template, no error)
;
; For each stub there should be a Rust handler with SysV ABI:  
;   extern "C" fn isr_XXX_rust(tf: &mut TrapFrame);
;
; ───────────────────────────────────────────────────────────────────────────

BITS 64
default rel

section .text

%define PUSHREG(x)      push x
%define POPREG(x)       pop  x

; ────────────────── TrapFrame layout (kept small & cache‑friendly) ─────────
; We save GPRs + essential return frame + error code, in this order:
;   r15..r8, rdi, rsi, rbp, rbx, rdx, rcx, rax   (15 * 8)
;   rip, cs, rflags, rsp                         (4 * 8)
;   err                                          (1 * 8)
; Total: 20 * 8 = 160 bytes.
; NOTE: TF_RSP is the **address of saved RIP** (HW frame base) to iretq from.

%assign TF_R15      0
%assign TF_R14      8
%assign TF_R13     16
%assign TF_R12     24
%assign TF_R11     32
%assign TF_R10     40
%assign TF_R9      48
%assign TF_R8      56
%assign TF_RDI     64
%assign TF_RSI     72
%assign TF_RBP     80
%assign TF_RBX     88
%assign TF_RDX     96
%assign TF_RCX    104
%assign TF_RAX    112
%assign TF_RIP    120
%assign TF_CS     128
%assign TF_RFLAGS 136
%assign TF_RSP    144
%assign TF_ERR    152
%assign TF_SIZE   160

; ───────────────────────── Existing macro names (redefined) ────────────────
; These preserve your naming while fixing semantics.

; Restore general‑purpose registers from TF at [rsp]
%undef RESTORE_GPRS_FROM_TF
%macro RESTORE_GPRS_FROM_TF 0
    mov     r15, [rsp + TF_R15]
    mov     r14, [rsp + TF_R14]
    mov     r13, [rsp + TF_R13]
    mov     r12, [rsp + TF_R12]
    mov     r11, [rsp + TF_R11]
    mov     r10, [rsp + TF_R10]
    mov     r9,  [rsp + TF_R9]
    mov     r8,  [rsp + TF_R8]
    mov     rdi, [rsp + TF_RDI]
    mov     rsi, [rsp + TF_RSI]
    mov     rbp, [rsp + TF_RBP]
    mov     rbx, [rsp + TF_RBX]
    mov     rdx, [rsp + TF_RDX]
    mov     rcx, [rsp + TF_RCX]
    mov     rax, [rsp + TF_RAX]
%endmacro

; Destination HW frame base is in RBX (address of saved RIP).  
; Write RIP/CS/RFLAGS from TF into that HW frame.
%undef WRITE_BACK_HW
%macro WRITE_BACK_HW 0
    mov     rax, [rsp + TF_RIP]
    mov     [rbx + 0],  rax
    mov     rax, [rsp + TF_CS]
    mov     [rbx + 8],  rax
    mov     rax, [rsp + TF_RFLAGS]
    mov     [rbx + 16], rax
%endmacro

; Helper: choose the HW frame we must return from (may differ on a switch)  
; Loads RBX = TF_RSP, which is the &RIP of the target frame.
%macro LOAD_TARGET_HWBASE_RBX 0
    mov     rbx, [rsp + TF_RSP]
%endmacro

; SysV call into Rust: rdi = &TrapFrame (which currently lives at [rsp]).
%undef CALL_SYSV
%macro CALL_SYSV 1
    mov     rdi, rsp
    call    %1
%endmacro

; ───────────────────────── Prologue/Epilogue templates ─────────────────────
; We follow the pattern:
;  - On entry: RSP points to HW frame (maybe +8 if error code was pushed).  
;  - We compute r12 = &RIP on *entry* HW frame (old current thread).        
;  - We build a TF on the stack and set TF_RSP = r12 (initially).           
;  - Call Rust; it may modify TF_RSP to point to another thread’s frame.     
;  - On return: write RIP/CS/RFLAGS to TF_RSP’s HW frame, restore GPRs,     
;    move RSP to TF_RSP, iretq.

; Save all GPRs into a new TF and fill return frame pieces.
%macro PROLOGUE_NOERR 0
    ; r12 := &RIP of current HW frame (no error code)
    mov     r12, rsp                ; r12 -> [RIP,CS,RFLAGS]

    ; Reserve TF and store GPRs (top‑down avoids extra reg temps)
    sub     rsp, TF_SIZE
    mov     [rsp + TF_R15], r15
    mov     [rsp + TF_R14], r14
    mov     [rsp + TF_R13], r13
    mov     [rsp + TF_R12], r12     ; keep original r12 for debugging
    mov     [rsp + TF_R11], r11
    mov     [rsp + TF_R10], r10
    mov     [rsp + TF_R9],  r9
    mov     [rsp + TF_R8],  r8
    mov     [rsp + TF_RDI], rdi
    mov     [rsp + TF_RSI], rsi
    mov     [rsp + TF_RBP], rbp
    mov     [rsp + TF_RBX], rbx
    mov     [rsp + TF_RDX], rdx
    mov     [rsp + TF_RCX], rcx
    mov     [rsp + TF_RAX], rax

    ; Copy the HW return trio from the entry frame into TF
    mov     rax, [r12 + 0]          ; RIP
    mov     [rsp + TF_RIP], rax
    mov     rax, [r12 + 8]          ; CS
    mov     [rsp + TF_CS], rax
    mov     rax, [r12 + 16]         ; RFLAGS
    mov     [rsp + TF_RFLAGS], rax

    ; RSP for returning: start with *current* frame base (&RIP)
    mov     [rsp + TF_RSP], r12
    xor     eax, eax                ; TF_ERR = 0 for IRQs/most traps
    mov     [rsp + TF_ERR], rax
%endmacro

%macro PROLOGUE_ERR 0
    ; Entry stack top contains ERROR CODE at [rsp]; HW frame begins at +8
    lea     r12, [rsp + 8]          ; r12 -> [RIP,CS,RFLAGS]

    ; Reserve TF and store GPRs
    sub     rsp, TF_SIZE
    mov     [rsp + TF_R15], r15
    mov     [rsp + TF_R14], r14
    mov     [rsp + TF_R13], r13
    mov     [rsp + TF_R12], r12
    mov     [rsp + TF_R11], r11
    mov     [rsp + TF_R10], r10
    mov     [rsp + TF_R9],  r9
    mov     [rsp + TF_R8],  r8
    mov     [rsp + TF_RDI], rdi
    mov     [rsp + TF_RSI], rsi
    mov     [rsp + TF_RBP], rbp
    mov     [rsp + TF_RBX], rbx
    mov     [rsp + TF_RDX], rdx
    mov     [rsp + TF_RCX], rcx
    mov     [rsp + TF_RAX], rax

    ; Copy return trio from HW frame into TF
    mov     rax, [r12 + 0]          ; RIP
    mov     [rsp + TF_RIP], rax
    mov     rax, [r12 + 8]          ; CS
    mov     [rsp + TF_CS], rax
    mov     rax, [r12 + 16]         ; RFLAGS
    mov     [rsp + TF_RFLAGS], rax

    ; Save the incoming error code into TF_ERR
    mov     rax, [r12 - 8]          ; error code was at original [rsp]
    mov     [rsp + TF_ERR], rax

    ; Default return frame base = current frame unless Rust changes it
    mov     [rsp + TF_RSP], r12
%endmacro

%macro EPILOGUE_COMMON 0
    ; Pick target frame base (may equal r12, or a different thread)
    LOAD_TARGET_HWBASE_RBX          ; rbx = TF_RSP

    ; Write back RIP/CS/RFLAGS to the target HW frame
    WRITE_BACK_HW

    ; Restore GPRs for the target thread from TF
    RESTORE_GPRS_FROM_TF

    ; Hop to the target HW frame and return
    mov     rsp, rbx                ; rsp = &RIP,CS,RFLAGS
    iretq
%endmacro

; ───────────────────── ISR generator macros (noerr / err) ──────────────────

; Define a no‑error ISR stub that calls a Rust handler symbol
%macro DEF_ISR_NOERR 2
    global %1
    extern %2
%1:
    PROLOGUE_NOERR
    CALL_SYSV %2
    EPILOGUE_COMMON
%endmacro

; Define an error‑code ISR stub that calls a Rust handler symbol
%macro DEF_ISR_ERR 2
    global %1
    extern %2
%1:
    PROLOGUE_ERR
    CALL_SYSV %2
    EPILOGUE_COMMON
%endmacro

; ───────────────────────────── Concrete stubs ───────────────────────────────

; Faults/traps without error codes
DEF_ISR_NOERR isr_de_stub, isr_de_rust
DEF_ISR_NOERR isr_db_stub, isr_db_rust
DEF_ISR_NOERR isr_nmi_stub, isr_nmi_rust
DEF_ISR_NOERR isr_bp_stub, isr_bp_rust
DEF_ISR_NOERR isr_of_stub, isr_of_rust
DEF_ISR_NOERR isr_br_stub, isr_br_rust
DEF_ISR_NOERR isr_ud_stub, isr_ud_rust
DEF_ISR_NOERR isr_nm_stub, isr_nm_rust
DEF_ISR_NOERR isr_mf_stub, isr_mf_rust
DEF_ISR_NOERR isr_xm_stub, isr_xm_rust
DEF_ISR_NOERR isr_virt_stub, isr_virt_rust

; Faults with error codes
DEF_ISR_ERR   isr_df_stub, isr_df_rust
DEF_ISR_ERR   isr_ts_stub, isr_ts_rust
DEF_ISR_ERR   isr_np_stub, isr_np_rust
DEF_ISR_ERR   isr_ss_stub, isr_ss_rust
DEF_ISR_ERR   isr_gp_stub, isr_gp_rust
DEF_ISR_ERR   isr_pf_stub, isr_pf_rust
DEF_ISR_ERR   isr_ac_stub, isr_ac_rust

; IRQs (examples)
DEF_ISR_NOERR isr_timer_stub,      isr_timer_rust
DEF_ISR_NOERR isr_spurious_stub,   isr_spurious_rust
DEF_ISR_NOERR isr_apic_error_stub, isr_apic_error_rust

; Template for more IOAPIC/LAPIC IRQ vectors you wire later:
; DEF_ISR_NOERR isr_irq20_stub, isr_irq20_rust
; DEF_ISR_NOERR isr_irq21_stub, isr_irq21_rust
; ...
