[bits 64]
default rel
section .text
global __ctx_switch

%define OFF_R15     0x00
%define OFF_R14     0x08
%define OFF_R13     0x10
%define OFF_R12     0x18
%define OFF_RBP     0x20
%define OFF_RBX     0x28
%define OFF_RSP     0x30
%define OFF_RIP     0x38
%define OFF_RFLAGS  0x40

; rdi = &prev_ctx, rsi = &next_ctx (SysV)
__ctx_switch:
    ; ---- save prev (callee-saved + rsp/rip/rflags) ----
    mov     [rdi+OFF_R15], r15
    mov     [rdi+OFF_R14], r14
    mov     [rdi+OFF_R13], r13
    mov     [rdi+OFF_R12], r12
    mov     [rdi+OFF_RBP], rbp
    mov     [rdi+OFF_RBX], rbx
    mov     [rdi+OFF_RSP], rsp
    lea     rax, [rel .resume]
    mov     [rdi+OFF_RIP], rax
    pushfq
    pop     rax
    mov     [rdi+OFF_RFLAGS], rax

    ; ---- restore next ----
    mov     rdx, rsi                 ; base = &next_ctx
    mov     rsp, [rdx+OFF_RSP]

    mov     r15, [rdx+OFF_R15]
    mov     r14, [rdx+OFF_R14]
    mov     r13, [rdx+OFF_R13]
    mov     r12, [rdx+OFF_R12]
    mov     rbp, [rdx+OFF_RBP]
    mov     rbx, [rdx+OFF_RBX]

    ; restore next RFLAGS (sets IF etc. for the next context)
    push    qword [rdx+OFF_RFLAGS]
    popfq

    jmp     qword [rdx+OFF_RIP]

.resume:
    ret
