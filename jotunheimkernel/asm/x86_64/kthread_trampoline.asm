; SPDX-License-Identifier: JOSSL-1.0
; Copyright (C) 2025 The Jotunheim Project
; kthread_trampoline:
; Stack layout expected at first run (top of stack):
;   [0] = arg
;   [1] = entry fn pointer
; RSP -> [arg][entry]
[BITS 64]
extern sched_exit_current_trampoline
global kthread_trampoline
.text
kthread_trampoline:
    pop rdi
    pop rax
    sub rsp, 8
    call rax
    add rsp, 8
    jmp sched_exit_current_trampoline