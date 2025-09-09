[bits 64]
default rel
section .text; rdi = &prev.ctx, rsi = &next.ctx
__ctx_switch:
    ; save all
    mov [rdi+0x00], r15
    mov [rdi+0x08], r14
    mov [rdi+0x10], r13
    mov [rdi+0x18], r12
    mov [rdi+0x20], r11
    mov [rdi+0x28], r10
    mov [rdi+0x30], r9
    mov [rdi+0x38], r8
    mov [rdi+0x40], rdi
    mov [rdi+0x48], rsi
    mov [rdi+0x50], rbp
    mov [rdi+0x58], rbx
    mov [rdi+0x60], rdx
    mov [rdi+0x68], rcx
    mov [rdi+0x70], rax
    ; rsp/rip/rflags filled by ISR copy
    ; restore next (reverse order for caller-saves first is fine)
    mov rax, [rsi+0x70]
    mov rcx, [rsi+0x68]
    mov rdx, [rsi+0x60]
    mov rbx, [rsi+0x58]
    mov rbp, [rsi+0x50]
    mov rsi, [rsi+0x48]
    mov rdi, [rsi+0x40]
    mov r8,  [rsi+0x38]
    mov r9,  [rsi+0x30]
    mov r10, [rsi+0x28]
    mov r11, [rsi+0x20]
    mov r12, [rsi+0x18]
    mov r13, [rsi+0x10]
    mov r14, [rsi+0x08]
    mov r15, [rsi+0x00]
    mov rsp, [rsi+0x90]       ; rsp offset per your struct
    mov rax, [rsi+0x80]       ; rip
    jmp rax
