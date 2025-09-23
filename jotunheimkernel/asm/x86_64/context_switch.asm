; void __ctx_switch(CpuContext* prev, const CpuContext* next);
; SysV: rdi = &prev, rsi = &next

[BITS 64]
global __ctx_switch
global __first_switch
default rel
section .text


__ctx_switch:
    ; ------------ save prev ------------
    mov     [rdi+0x00], r15
    mov     [rdi+0x08], r14
    mov     [rdi+0x10], r13
    mov     [rdi+0x18], r12
    mov     [rdi+0x20], rbx
    mov     [rdi+0x28], rbp
    mov     [rdi+0x30], rsp
    lea     rax, [rel .ret_here]
    mov     [rdi+0x38], rax        ; rip
    pushfq
    pop     qword [rdi+0x40]       ; rflags

    ; ------------ restore next ----------
    mov     r15, [rdi+0x00]
    mov     r14, [rdi+0x08]
    mov     r13, [rdi+0x10]
    mov     r12, [rdi+0x18]
    mov     rbx, [rdi+0x20]
    mov     rbp, [rdi+0x28]

    mov     rsp, [rdi+0x30]
    push    qword [rdi+0x40]
    popfq
    push    qword [rdi+0x38]
    ret

.ret_here:
    ret 

__first_switch:
    mov     rdi, rdi              ; BASE = next

    ; restore callee-saved first (no base clobber)
    mov     r15, [rdi+0x00]
    mov     r14, [rdi+0x08]
    mov     r13, [rdi+0x10]
    mov     r12, [rdi+0x18]
    mov     rbx, [rdi+0x20]
    mov     rbp, [rdi+0x28]

    ; stack, flags, rip
    mov     rsp, [rdi+0x30]
    push    qword [rdi+0x40]      ; rflags
    popfq
    push    qword [rdi+0x38]      ; rip
    ret             