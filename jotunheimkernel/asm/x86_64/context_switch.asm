; void __ctx_switch(CpuContext* prev, const CpuContext* next);
; SysV: rdi = &prev, rsi = &next

[BITS 64]
global __ctx_switch
default rel
section .text

__ctx_switch:
    ; ---------------- save prev ----------------
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
    mov     [rdi+0x50], rbx
    mov     [rdi+0x58], rbp
    mov     [rdi+0x60], rdx
    mov     [rdi+0x68], rcx
    mov     [rdi+0x70], rax
    mov     [rdi+0x78], rsp
    lea     rax, [rel .ret_here]
    mov     [rdi+0x80], rax            ; save return RIP
    pushfq
    pop     qword [rdi+0x88]           ; save RFLAGS

    ; ---------------- restore next ----------------
    ; Keep BASE = &next in RDX (stable) until RIP is pushed.
    mov     rdx, rsi

    ; Restore GPRs that don't destroy BASE
    mov     r15, [rdx+0x00]
    mov     r14, [rdx+0x08]
    mov     r13, [rdx+0x10]
    mov     r12, [rdx+0x18]
    mov     r11, [rdx+0x20]
    mov     r10, [rdx+0x28]
    mov     r9,  [rdx+0x30]
    mov     r8,  [rdx+0x38]
    mov     rbx, [rdx+0x50]
    mov     rbp, [rdx+0x58]
    mov     rcx, [rdx+0x68]
    mov     rax, [rdx+0x70]

    ; Switch to next stack before flags/ret
    mov     rsp, [rdx+0x78]

    ; Restore FLAGS exactly as saved
    push    qword [rdx+0x88]
    popfq
    ; Push next RIP on next stack, then set arg regs and RET
    mov     rdi, [rdx+0x40]
    mov     rsi, [rdx+0x48]
    push    qword [rdx+0x80]           ; may #PF if next.ctx+0x80 unmapped
    mov     rdx, [rdx+0x60]            ; now safe to restore RDX (BASE no longer needed)
    sti
    ret

.ret_here:
    ret
