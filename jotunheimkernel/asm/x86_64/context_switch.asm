; rdi = &prev.ctx, rsi = &next.ctx
global __ctx_switch
section .text
__ctx_switch:
    ; -------- save prev --------
    mov     [rdi+0x00], r15
    mov     [rdi+0x08], r14
    mov     [rdi+0x10], r13
    mov     [rdi+0x18], r12
    mov     [rdi+0x20], r11
    mov     [rdi+0x28], r10
    mov     [rdi+0x30], r9
    mov     [rdi+0x38], r8
    mov     [rdi+0x40], rsi
    mov     [rdi+0x48], rdi
    mov     [rdi+0x50], rbp
    mov     [rdi+0x58], rbx
    mov     [rdi+0x60], rdx
    mov     [rdi+0x68], rcx
    mov     [rdi+0x70], rax
    lea     rax, [rel .ret_here]
    mov     [rdi+0x80], rax         ; RIP
    pushfq
    pop     rax
    mov     [rdi+0x88], rax         ; RFLAGS
    mov     rax, rsp
    mov     [rdi+0x78], rax         ; RSP

    ; -------- restore next --------
    mov     rax, rsi                ; RAX holds BASE = &next.ctx (stable)
    mov     r15, [rax+0x00]
    mov     r14, [rax+0x08]
    mov     r13, [rax+0x10]
    mov     r12, [rax+0x18]
    mov     r11, [rax+0x20]
    mov     r10, [rax+0x28]
    mov     r9,  [rax+0x30]
    mov     r8,  [rax+0x38]
    mov     rsi, [rax+0x40]
    mov     rdi, [rax+0x48]
    mov     rbp, [rax+0x50]
    mov     rbx, [rax+0x58]
    mov     rcx, [rax+0x68]
    mov     rdx, [rax+0x80]         ; next RIP -> RDX
    mov     r10, [rax+0x88]         ; next RFLAGS -> R10
    mov     rsp, [rax+0x78]         ; *** correct: +0x78 ***
    mov     rdx, rdx                ; (no-op; just to emphasize RDX holds target)
    mov     rax, [rax+0x70]         ; restore RAX last
    push    r10
    popfq
    jmp     rdx

.ret_here:
    ret
