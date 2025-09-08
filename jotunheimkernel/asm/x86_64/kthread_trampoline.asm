[bits 64]
default rel
section .text
global kthread_trampoline
extern sched_exit_current_trampoline

; Stack on entry (top -> high):
;   [rsp + 0x00] = arg (usize)
;   [rsp + 0x08] = entry (extern "C" fn(usize) -> !)

kthread_trampoline:
    pop rdi                 ; rdi = arg          (RSP += 8, now %16 == 8)
    pop rax
    call rax                ; entry(arg), should not return
    add rsp, 8              ; balance the unread entry slot if it returns
    jmp  sched_exit_current_trampoline
