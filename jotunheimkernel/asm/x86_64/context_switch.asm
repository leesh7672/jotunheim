; rdi = &prev.ctx, rsi = &next.ctx
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

__ctx_switch:
    push    rbp
    mov     rbp, rsp

    ; Save prev GP regs into *rdi
    mov     [rdi+OFF_R15], r15
    mov     [rdi+OFF_R14], r14
    mov     [rdi+OFF_R13], r13
    mov     [rdi+OFF_R12], r12
    mov     [rdi+OFF_R11], r11
    mov     [rdi+OFF_R10], r10
    mov     [rdi+OFF_R9],  r9
    mov     [rdi+OFF_R8],  r8
    mov     [rdi+OFF_RDI], rdi            ; safe: rdi still holds &prev
    mov     [rdi+OFF_RSI], rsi
    mov     [rdi+OFF_RBP], rbp
    mov     [rdi+OFF_RBX], rbx
    mov     [rdi+OFF_RDX], rdx
    mov     [rdi+OFF_RCX], rcx
    mov     [rdi+OFF_RAX], rax
    mov     [rdi+OFF_RSP], rsp
    pushfq
    pop     qword [rdi+OFF_RFLAGS]
    ; Save resume RIP (= next instruction after jmp) for prev
    lea     rax, [rel .resume]
    mov     [rdi+OFF_RIP], rax

    ; Switch to next
    mov     rbx, rsi                      ; rbx = &next.ctx

    ; Optionally normalize flags (avoid DF surprises)
    cld

    mov     rsp, [rbx+OFF_RSP]
    ; (we keep current RFLAGS; if you truly want per-thread flags here,
    ; you'd need push [rbx+OFF_RFLAGS] / popfq, but that's rarely needed)

    mov     r15, [rbx+OFF_R15]
    mov     r14, [rbx+OFF_R14]
    mov     r13, [rbx+OFF_R13]
    mov     r12, [rbx+OFF_R12]
    mov     r11, [rbx+OFF_R11]
    mov     r10, [rbx+OFF_R10]
    mov     r9,  [rbx+OFF_R9]
    mov     r8,  [rbx+OFF_R8]
    mov     rbp, [rbx+OFF_RBP]
    mov     rdx, [rbx+OFF_RDX]
    mov     rcx, [rbx+OFF_RCX]
    mov     rax, [rbx+OFF_RAX]
    mov     rdi, [rbx+OFF_RDI]           ; restore arg regs last
    mov     rsi, [rbx+OFF_RSI]

    sti
    jmp     qword [rbx+OFF_RIP]

.resume:
    pop     rbp
    ret
