; SPDX-License-Identifier: JOSSL-1.0
; Copyright (C) 2025 The Jotunheim Project
[BITS 64]
default rel
section .text
global far_jump
global disable_flags

far_jump:
    ; Far jump to the same CPL with CS=0x8
    jmp  far [rel .ptr]      ; loads CS and RIP from memory (m16:64)
align 8
.ptr:
    dq .after                ; 64-bit RIP
    dw 0x0008                ; selector
.after:
    ret