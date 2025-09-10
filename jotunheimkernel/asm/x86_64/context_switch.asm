; asm/x86_64/context_switch_full.asm
[bits 64]
default rel
section .text
global __ctx_switch

; CpuContext offsets (must match Rust struct above)
%define OFF_R15     0x00
%define OFF_R14     0x08
%define OFF_R13     0x10
%define OFF_R12     0x18
%define OFF_R11     0x20
%define OFF_R10     0x28
%define OFF_R9      0x30
%define OFF_R8      0x38
%define OFF_RDI     0x40
%define OFF_RSI     0x48
%define OFF_RBP     0x50
%define OFF_RBX     0x58
%define OFF_RDX     0x60
%define OFF_RCX     0x68
%define OFF_RAX     0x70
%define OFF_RSP     0x78
%define OFF_RIP     0x80
%define OFF_RFLAGS  0x88

; Stack layout on entry:
;   [rsp+0x00]  return address (from call)
;   [rsp+0x08]  prev_ctx (pushed by wrapper)
;   [rsp+0x10]  next_ctx (pushed by wrapper)

; We will FIRST push *all* GPRs + RFLAGS to capture the true pre-call state.
; After pushes:
;   rsp' = rsp - 128
;   At rsp' + offsets:
;     +0x00 RFLAGS
;     +0x08 R15, +0x10 R14, +0x18 R13, +0x20 R12,
;     +0x28 R11, +0x30 R10, +0x38 R9,  +0x40 R8,
;     +0x48 RDI, +0x50 RSI, +0x58 RBP, +0x60 RBX,
;     +0x68 RDX, +0x70 RCX, +0x78 RAX,
;     +0x80 RET, +0x88 PREV, +0x90 NEXT

__ctx_switch:
    ; Load prev/next pointers from above our capture frame
    push    rax
    push    rcx
    push    rdx
    push    rbx
    push    rbp
    push    rsi
    push    rdi
    push    r8
    push    r9
    push    r10
    push    r11
    push    r12
    push    r13
    push    r14
    push    r15
    pushfq

    mov     rdx, [rsp + 0x88]       ; prev_ctx*
    mov     rcx, [rsp + 0x90]       ; next_ctx*

    ; Save into prev_ctx from the captured frame
    mov     rax, [rsp + 0x08]       ; r15
    mov     [rdx + OFF_R15], rax
    mov     rax, [rsp + 0x10]       ; r14
    mov     [rdx + OFF_R14], rax
    mov     rax, [rsp + 0x18]       ; r13
    mov     [rdx + OFF_R13], rax
    mov     rax, [rsp + 0x20]       ; r12
    mov     [rdx + OFF_R12], rax
    mov     rax, [rsp + 0x28]       ; r11
    mov     [rdx + OFF_R11], rax
    mov     rax, [rsp + 0x30]       ; r10
    mov     [rdx + OFF_R10], rax
    mov     rax, [rsp + 0x38]       ; r9
    mov     [rdx + OFF_R9],  rax
    mov     rax, [rsp + 0x40]       ; r8
    mov     [rdx + OFF_R8],  rax
    mov     rax, [rsp + 0x48]       ; rdi
    mov     [rdx + OFF_RDI], rax
    mov     rax, [rsp + 0x50]       ; rsi
    mov     [rdx + OFF_RSI], rax
    mov     rax, [rsp + 0x58]       ; rbp
    mov     [rdx + OFF_RBP], rax
    mov     rax, [rsp + 0x60]       ; rbx
    mov     [rdx + OFF_RBX], rax
    mov     rax, [rsp + 0x68]       ; rdx
    mov     [rdx + OFF_RDX], rax
    mov     rax, [rsp + 0x70]       ; rcx
    mov     [rdx + OFF_RCX], rax
    mov     rax, [rsp + 0x78]       ; rax
    mov     [rdx + OFF_RAX], rax
    lea     rax, [rsp + 0x80]       ; original RSP before our pushes
    mov     [rdx + OFF_RSP], rax
    lea     rax, [rel .resume]
    mov     [rdx + OFF_RIP], rax
    mov     rax, [rsp + 0x00]       ; RFLAGS
    mov     [rdx + OFF_RFLAGS], rax

    ; --- restore the next task's state ---
    ; Set its stack pointer first
    mov     rsp, [rcx + OFF_RSP]

    ; Restore GPRs (any order that doesn't clobber RCX until last read)
    mov     r15, [rcx + OFF_R15]
    mov     r14, [rcx + OFF_R14]
    mov     r13, [rcx + OFF_R13]
    mov     r12, [rcx + OFF_R12]
    mov     r11, [rcx + OFF_R11]
    mov     r10, [rcx + OFF_R10]
    mov     r9,  [rcx + OFF_R9]
    mov     r8,  [rcx + OFF_R8]
    mov     rdi, [rcx + OFF_RDI]
    mov     rsi, [rcx + OFF_RSI]
    mov     rbp, [rcx + OFF_RBP]
    mov     rbx, [rcx + OFF_RBX]
    mov     rdx, [rcx + OFF_RDX]
    mov     rax, [rcx + OFF_RAX]

    ; Prepare RIP and RFLAGS before clobbering RCX
    mov     r8,  [rcx + OFF_RIP]     ; temp keep RIP in r8
    push    qword [rcx + OFF_RFLAGS]
    popfq
    mov     rcx, [rcx + OFF_RCX]

    jmp     r8                       ; never falls through

.resume:
    ; We are back in the previously-saved task after some *other* task switched to us.
    ; Return to the Rust wrapper; it will `add rsp,16` to drop (prev,next).
    ret
