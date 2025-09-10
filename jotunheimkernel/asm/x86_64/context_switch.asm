[bits 64]
default rel
section .text
global __ctx_switch

__ctx_switch:
    ; rdi=&prev.ctx, rsi=&next.ctx
; asm/x86_64/context_switch.asm
[bits 64]
default rel
global __ctx_switch
section .text

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
; rdi = &prev.ctx, rsi = &next.ctx
__ctx_switch:
    ; ---- save prev ----
    mov     [rdi+0x00], r15
    mov     [rdi+0x08], r14
    mov     [rdi+0x10], r13
    mov     [rdi+0x18], r12
    mov     [rdi+0x20], r11
    mov     [rdi+0x28], r10
    mov     [rdi+0x30], r9
    mov     [rdi+0x38], r8
    mov     [rdi+0x40], rdi
    mov     [rdi+0x48], rsi
    mov     [rdi+0x50], rbp
    mov     [rdi+0x58], rbx
    mov     [rdi+0x60], rdx
    mov     [rdi+0x68], rcx
    mov     [rdi+0x70], rax
    mov     [rdi+0x78], rsp
    lea     rax, [rel .resume]
    mov     [rdi+0x80], rax
    pushfq
    pop     rax
    mov     [rdi+0x88], rax

    ; ---- restore next ----
    mov     rdx, rsi                 ; base = &next.ctx (DON'T use rbx)
    mov     rsp, [rdx+0x78]

    mov     r15, [rdx+0x00]
    mov     r14, [rdx+0x08]
    mov     r13, [rdx+0x10]
    mov     r12, [rdx+0x18]
    mov     r11, [rdx+0x20]
    mov     r10, [rdx+0x28]
    mov     r9,  [rdx+0x30]
    mov     r8,  [rdx+0x38]
    mov     rbp, [rdx+0x50]
    ; rdx is our base, so restore RCX/RAX after we've used them as scratch if you want
    mov     rcx, [rdx+0x68]
    mov     rax, [rdx+0x70]
    mov     rdi, [rdx+0x40]
    mov     rsi, [rdx+0x48]
    mov     rbx, [rdx+0x58]         ; restore RBX LAST (no longer need base as RBX)
    sti
    jmp     qword [rdx+0x80]

.resume:
    ret
