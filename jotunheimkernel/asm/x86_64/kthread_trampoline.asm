[bits 64]
default rel
section .text
global kthread_trampoline
extern sched_exit_current_trampoline   ; Rust shim we’ll add below

; Stack layout expected on entry (highest addr at the top):
;   [rsp+0x00] = arg (usize)
;   [rsp+0x08] = entry fn ptr (extern "C" fn(usize)->!)
; We will pop into regs and CALL entry(arg). If it (incorrectly) returns,
; jump to sched_exit_current_trampoline() to cleanly kill the task.

kthread_trampoline:
    pop rdi            ; arg → rdi (SysV first arg)
    pop rax            ; entry fn ptr → rax
    call rax           ; entry(rdi) ; type says -> !, so should not return
    jmp  sched_exit_current_trampoline
