; asm/x86_64/ap_trampoline.asm
; Position-independent AP trampoline: 16 -> 32 -> 64-bit, no .data, no 16-bit relocations.

[bits 16]

global _ap_tramp_start
global _ap_tramp_end
global _ap_tramp_apboot_ptr32
global _ap_tramp_apboot_ptr64

section .text
_ap_tramp_start:

start16:
    cli

    ; --- get 16-bit IP base in BX (offset of start16 within segment) ---
    call .getip16
.getip16:
    pop bx
    sub bx, (.getip16 - start16)

    ; --- load GDTR via offset from BX (no relocations) ---
    mov si, bx
    add si, (gdt_desc - start16)
    lgdt [si]

    ; --- enter 32-bit protected mode ---
    mov eax, cr0
    or  eax, 1                 ; PE=1
    mov cr0, eax

    ; Build far pointer to pmode with selector 0x08 (32-bit code)
    mov ax, cs
    movzx eax, ax
    shl eax, 4                 ; EAX = CS<<4
    add eax, (pmode - start16) ; EAX = linear addr of pmode

    mov si, bx
    add si, (pmode_far_ptr - start16)
    mov [si], eax              ; dd offset
    mov word [si+4], 0x0008    ; dw selector = 32-bit code

    jmp far [si]               ; 16:32 far jump

[bits 32]
pmode:
    ; set flat data segments (0x10)
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax

    ; --- get 32-bit IP base in EBX (position independent) ---
    call .getip32
.getip32:
    pop ebx
    sub ebx, (.getip32 - pmode)

    ; enable PAE
    mov eax, cr4
    bts eax, 5                 ; PAE
    mov cr4, eax

    ; load CR3 (LOW 32 BITS) from ApBoot via IP-relative patch point
    lea esi, [ebx + (_ap_tramp_apboot_ptr32 - pmode)]
    mov esi, [esi]             ; ESI = ApBoot* (PHYS low 32)
    mov eax, [esi + 8]         ; ApBoot.cr3 low 32
    mov cr3, eax

    ; enable IA-32e (EFER.LME)
    mov ecx, 0xC0000080        ; IA32_EFER
    rdmsr
    bts eax, 8                 ; LME
    wrmsr

    ; enable paging (CR0.PG)
    mov eax, cr0
    bts eax, 31
    mov cr0, eax

    ; far jump to 64-bit code selector 0x18
    push dword 0x0018          ; 64-bit code selector
    lea eax, [ebx + (lm64 - pmode)]
    push eax
    retf

[bits 64]
align 16
lm64:
    ; load ApBoot* (64-bit) via RIP-relative patch point
    lea rdx, [rel _ap_tramp_apboot_ptr64]
    mov rax, [rdx]             ; rax = ApBoot* (PHYSICAL)

    ; set stack and entry
    mov rsp, [rax + 24]        ; ApBoot.stack_top
    mov rcx, [rax + 48]        ; ApBoot.entry64

    ; signal ready
    mov dword [rax + 0], 1     ; ApBoot.ready_flag = 1

    jmp rcx                    ; -> ap_entry()

; ---------- tiny flat GDT ----------
align 8
gdt64:
    dq 0
    dq 0x00CF9A000000FFFF      ; 0x08: 32-bit code (L=0, D=1)
    dq 0x00CF92000000FFFF      ; 0x10: data
    dq 0x00AF9A000000FFFF      ; 0x18: 64-bit code (L=1)

gdt_desc:
    dw gdt64_end - gdt64 - 1
    dd gdt64
gdt64_end:

; runtime far pointer (dd offset, dw selector), lives in .text
pmode_far_ptr:
    dd 0
    dw 0

; patch points BSP fills before SIPIs (stay in .text)
align 8
_ap_tramp_apboot_ptr32: dd 0    ; PHYS ApBoot (low 32)
align 8
_ap_tramp_apboot_ptr64: dq 0    ; PHYS ApBoot (64)

_ap_tramp_end:
