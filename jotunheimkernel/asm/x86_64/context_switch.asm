; __ctx_switch(&prev, &next)
; Save: r15 r14 r13 r12 rbp rbx rsp  and store resume RIP/RFLAGS
; Restore: next's rflags (incl. IF), callee-saved, rsp, then jmp next.rip

[bits 64]
default rel
global __ctx_switch
section .text

%define OFF_R15     0x00
%define OFF_R14     0x08
%define OFF_R13     0x10
%define OFF_R12     0x18
%define OFF_RBP     0x20
%define OFF_RBX     0x28
%define OFF_RSP     0x30
%define OFF_RIP     0x38
%define OFF_RFLAGS  0x40

__ctx_switch:
    ; rdi = &prev, rsi = &next
    push rbp
    mov  rbp, rsp

    ; Save callee-saved of PREV using rdi as base
    mov  [rdi+OFF_R15], r15
    mov  [rdi+OFF_R14], r14
    mov  [rdi+OFF_R13], r13
    mov  [rdi+OFF_R12], r12
    mov  [rdi+OFF_RBP], rbp
    mov  [rdi+OFF_RBX], rbx       ; save original RBX before clobber
    mov  [rdi+OFF_RSP], rsp

    ; Save where PREV should resume and its RFLAGS
    lea  rax, [rel .resume]
    mov  [rdi+OFF_RIP], rax
    pushfq
    pop  qword [rdi+OFF_RFLAGS]

    ; Load NEXT context
    ; (Use rsi as base; leave rdi untouched)
    mov  rsp, [rsi+OFF_RSP]

    ; Restore NEXT RFLAGS (incl. IF) first
    push qword [rsi+OFF_RFLAGS]
    popfq

    ; Restore callee-saved
    mov  r15, [rsi+OFF_R15]
    mov  r14, [rsi+OFF_R14]
    mov  r13, [rsi+OFF_R13]
    mov  r12, [rsi+OFF_R12]
    mov  rbp, [rsi+OFF_RBP]
    mov  rbx, [rsi+OFF_RBX]

    ; Jump to NEXT RIP on its stack
    jmp  qword [rsi+OFF_RIP]

.resume:
    pop  rbp
    ret
