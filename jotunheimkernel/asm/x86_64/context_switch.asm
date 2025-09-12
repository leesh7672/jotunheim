; asm/x86_64/context_switch.asm
; rdi = &prev.ctx, rsi = &next.ctx
global __ctx_switch
section .text
__ctx_switch:
    ; save callee-saved + volatile we track
    mov     [rdi+0x00], r15
    mov     [rdi+0x08], r14
    mov     [rdi+0x10], r13
    mov     [rdi+0x18], r12
    mov     [rdi+0x20], r11
    mov     [rdi+0x28], r10
    mov     [rdi+0x30], r9
    mov     [rdi+0x38], r8
    mov     [rdi+0x40], rsi        ; save as seen by caller
    mov     [rdi+0x48], rdi
    mov     [rdi+0x50], rbp
    mov     [rdi+0x58], rbx
    mov     [rdi+0x60], rdx
    mov     [rdi+0x68], rcx
    mov     [rdi+0x70], rax
    ; save rsp/rip/rflags from our call-frame
    lea     rax, [rel .ret_here]
    mov     [rdi+0x80], rax
    pushfq
    pop     rax
    mov     [rdi+0x88], rax
    mov     rax, rsp
    mov     [rdi+0x78], rax

    ; restore next
    mov     r15, [rsi+0x00]
    mov     r14, [rsi+0x08]
    mov     r13, [rsi+0x10]
    mov     r12, [rsi+0x18]
    mov     r11, [rsi+0x20]
    mov     r10, [rsi+0x28]
    mov     r9,  [rsi+0x30]
    mov     r8,  [rsi+0x38]
    mov     rsi, [rsi+0x40]
    mov     rdi, [rsi+0x48]        ; careful: use a temp if you need rsi
    ; Better: use a temp register to load rdi before clobbering rsi:
    ;   mov rax, [rsi+0x48]
    ;   mov rdi, rax
    mov     rbp, [rsi+0x50]
    mov     rbx, [rsi+0x58]
    mov     rdx, [rsi+0x60]
    mov     rcx, [rsi+0x68]
    mov     rax, [rsi+0x70]
    mov     rsp, [rsi+0x78]
    push    qword [rsi+0x88]       ; rflags
    popfq
    jmp     qword [rsi+0x80]
.ret_here:
    ret
