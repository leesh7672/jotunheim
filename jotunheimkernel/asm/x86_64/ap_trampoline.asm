; asm/x86_64/ap_trampoline.asm
; Position-independent AP trampoline: 16 -> 32 -> 64, no .data, no 16-bit relocs.

[bits 16]
global _ap_tramp_start
global _ap_tramp_end
global _ap_tramp_apboot_ptr32
global _ap_tramp_apboot_ptr64

section .text
_ap_tramp_start:
[bits 16]
section .text
_ap_tramp_start:

start16:
    cli

    ; get 16-bit IP-relative base in BX (offset from segment base)
    call .getip16
.getip16:
    pop bx
    sub bx, (.getip16 - start16)

    ; SI -> scratch gdtr buffer (6 bytes) in .text (PIE)
    mov si, bx
    add si, (gdt_desc - start16)

    ; limit = sizeof(gdt64)-1
    mov ax, (gdt64_end - gdt64 - 1)
    mov [si+0], ax

    ; base = CS<<4 + (gdt64 - start16)
    mov ax, cs
    movzx eax, ax
    shl eax, 4
    add eax, (gdt64 - start16)
    mov [si+2], eax

    lgdt [si]

    ; enter protected mode
    mov eax, cr0
    or  eax, 1                  ; CR0.PE
    mov cr0, eax

    ; build far pointer to 32-bit pmode: (offset, selector=0x08)
    mov si, bx
    add si, (pmode_far_ptr - start16)

    mov ax, cs
    movzx eax, ax
    shl eax, 4
    add eax, (pmode - start16)
    mov [si], eax               ; dd offset
    mov word [si+4], 0x0008     ; dw selector (32-bit code)

    jmp dword far [si]

[bits 32]
pmode:
    ; flat data segments (0x10)
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax

    ; >>> TEMP 32-bit STACK (identity-mapped by BSP) <<<
    mov esp, 0x9000 + 0x1000   ; top of the 0x9000..0x9FFF page

    ; --- get 32-bit IP base in EBX (PIE) ---
    call .getip32
.getip32:
    pop ebx
    sub ebx, (.getip32 - pmode)

    ; enable PAE
    mov eax, cr4
    bts eax, 5
    mov cr4, eax

    ; load CR3 (low 32) from ApBoot...
    lea esi, [ebx + (_ap_tramp_apboot_ptr32 - pmode)]
    mov esi, [esi]
    mov eax, [esi + 8]         ; ApBoot.cr3 (low 32)
    mov cr3, eax

    ; IA32_EFER.LME=1
    mov ecx, 0xC0000080
    rdmsr
    bts eax, 8
    wrmsr

    ; paging on
    mov eax, cr0
    bts eax, 31
    mov cr0, eax

    ; far jump into 64-bit CS
    push dword 0x0018
    lea eax, [ebx + (lm64 - pmode)]
    push eax
    retf
    
[bits 64]
lm64:
    lea rdx, [rel _ap_tramp_apboot_ptr64]
    mov rax, [rdx]            ; rax = ApBoot* (PHYSICAL)

    ; correct offsets for #[repr(C)] ApBoot:
    ; stack_top @ +0x20, entry64 @ +0x28, hhdm @ +0x30

    mov rsp, [rax + 0x20]     ; ApBoot.stack_top
    mov rcx, [rax + 0x28]     ; ApBoot.entry64
    
    pop rdi
    jmp rcx                   ; -> ap_entry()

; ---------- tiny flat GDT ----------
align 8
gdt64:
    dq 0
    dq 0x00CF9A000000FFFF     ; 0x08: 32-bit code
    dq 0x00CF92000000FFFF     ; 0x10: data
    dq 0x00AF9A000000FFFF     ; 0x18: 64-bit code
gdt64_end:

; 6-byte scratch GDTR we fill at runtime (limit[0..1], base[2..5])
gdt_desc:
    times 6 db 0

; runtime far pointer buffer for 16->32 (dd offset, dw selector)
pmode_far_ptr:
    dd 0
    dw 0


; BSP patches these (PHYSICAL ApBoot pointer)
align 8
_ap_tramp_apboot_ptr32: dd 0
align 8
_ap_tramp_apboot_ptr64: dq 0

_ap_tramp_end:
