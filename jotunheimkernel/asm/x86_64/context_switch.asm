[bits 64]
default rel
section .text
global __ctx_switch

; void __ctx_switch(struct CpuContext* prev, const struct CpuContext* next)
; rdi = prev, rsi = next
__ctx_switch:
    ; save callee-saved
    mov [rdi + 0x00], r15
    mov [rdi + 0x08], r14
    mov [rdi + 0x10], r13
    mov [rdi + 0x18], r12
    mov [rdi + 0x20], rbx
    mov [rdi + 0x28], rbp
    mov [rdi + 0x30], rsp        ; save rsp
    lea rax, [rel .resume]
    mov [rdi + 0x38], rax        ; save rip

    ; restore next
    mov r15, [rsi + 0x00]
    mov r14, [rsi + 0x08]
    mov r13, [rsi + 0x10]
    mov r12, [rsi + 0x18]
    mov rbx, [rsi + 0x20]
    mov rbp, [rsi + 0x28]
    mov rsp, [rsi + 0x30]        ; restore rsp
    mov rax, [rsi + 0x38]        ; next rip
    jmp rax
.resume:
    ret
