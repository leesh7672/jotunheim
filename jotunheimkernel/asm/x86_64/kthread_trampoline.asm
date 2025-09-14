; kthread_trampoline:
; Stack layout expected at first run (top of stack):
;   [0] = arg
;   [1] = entry fn pointer
; RSP -> [arg][entry]
extern sched_exit_current_trampoline
global kthread_trampoline
kthread_trampoline:
    pop rdi            ; rdi = arg
    pop rax            ; rax = entry
    sub rsp, 8         ; keep 16-byte alignment before CALL
    sti
    call rax           ; entry(arg) -> !
    add  rsp, 8        ; (won’t run if entry is noreturn)
    jmp  sched_exit_current_trampoline
