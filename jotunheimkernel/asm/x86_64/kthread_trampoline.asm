[bits 64]
default rel
section .text
global kthread_trampoline
extern sched_exit_current_trampoline

; Stack on entry (top -> high):
;   [rsp + 0x00] = arg (usize)
;   [rsp + 0x08] = entry (extern "C" fn(usize) -> !)

kthread_trampoline:
    pop rdi            ; arg
    pop rax            ; entry fn ptr
    sub rsp, 8         ; ensure RSP%16 == 8 before CALL (so callee sees 16)
    sti
    call rax           ; entry(rdi) -> !
    add rsp, 8         ; (won’t run if entry is noreturn)
    jmp  sched_exit_current_trampoline
