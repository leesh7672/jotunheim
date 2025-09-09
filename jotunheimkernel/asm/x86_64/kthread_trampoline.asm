[bits 64]
default rel
section .text
global kthread_trampoline
extern sched_exit_current_trampoline

; Stack on entry (top -> high):
;   [rsp + 0x00] = arg (usize)
;   [rsp + 0x08] = entry (extern "C" fn(usize) -> !)
; --- kthread_trampoline (fix: enforce 16B ABI alignment before call) ---
kthread_trampoline:
    pop rdi            ; arg
    pop rax            ; entry fn ptr
    call rax           ; entry(rdi) -> !
    jmp  sched_exit_current_trampoline
