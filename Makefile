# Makefile for Jotunheim OS (bootloader + kernel + ESP + QEMU run)

# ===== Toolchains / versions =====
RUSTUP          := rustup
TOOLCHAIN       ?= stable

# ===== Paths =====
BOOT_DIR         := jotunboot
KERNEL_DIR       := jotunheimkernel
TARGET_DIR_BOOT  := $(BOOT_DIR)/target/x86_64-unknown-uefi/debug
TARGET_DIR_KRN   := $(KERNEL_DIR)/target/x86_64-unknown-none/debug

BOOT_EFI_NAME    := jotunboot.efi
KERNEL_ELF_NAME  := jotunheim-kernel

BOOT_EFI         := $(TARGET_DIR_BOOT)/$(BOOT_EFI_NAME)
KERNEL_ELF       := $(TARGET_DIR_KRN)/$(KERNEL_ELF_NAME)

ESP              ?= ::
IMG				 ?= $(PWD)/image.img
ESP_EFI_DIR      := $(ESP)/EFI
ESP_BOOT_DIR	 := $(ESP_EFI_DIR)/BOOT
ESP_OS_DIR       := $(ESP)/JOTUNHEIM
ESP_BOOTX64      := $(ESP_BOOT_DIR)/BOOTX64.EFI
ESP_KERNEL       := $(ESP_OS_DIR)/KERNEL.ELF

# ===== QEMU / UEFI firmware =====
QEMU             ?= qemu-system-x86_64
QEMU_MACHINE     ?= q35
QEMU_MEM         ?= 8G
OVMF_CODE        ?= /usr/local/share/edk2-qemu/QEMU_UEFI_CODE-x86_64.fd
QEMU_EXTRA       ?=

# ===== Default target =====
.PHONY: all
all: esp-populate

# ===== Build targets =====
.PHONY: boot
boot: 
	@echo "==> Building bootloader"
	cd $(BOOT_DIR) && cargo build
.PHONY: kernel
kernel: 
	@echo "==> Building kernel"
	cd $(KERNEL_DIR) && cargo build

# ===== ESP population =====
.PHONY: esp-prep
esp-prep: image
	@echo "==> Preparing ESP directories: $(ESP_EFI_DIR) and $(ESP_OS_DIR)"
	mmd -i $(IMG) "$(ESP_EFI_DIR)" "$(ESP_BOOT_DIR)" "$(ESP_OS_DIR)"
	
.PHONY: esp-populate
esp-populate: boot kernel esp-prep
	@echo "==> Copying artifacts to ESP"
	mcopy -i "$(IMG)" -b "$(BOOT_EFI)" "$(ESP_BOOTX64)"
	mcopy -i "$(IMG)" -b "$(KERNEL_ELF)" "$(ESP_KERNEL)"
	@echo "==> ESP ready at: $(ESP)"

.PHONY: image
image: distclean
	@echo "==> Generating image"
	dd if=/dev/zero of="$(IMG)" bs=1M count=4000
	newfs_msdos -F32 "$(IMG)"

# ===== Run in QEMU =====
.PHONY: run
run:
	@echo "==> Launching QEMU"
	$(QEMU) -machine $(QEMU_MACHINE) -m $(QEMU_MEM) -cpu max \
		-drive if=pflash,format=raw,readonly=on,file=$(OVMF_CODE) \
		-drive format=raw,file=$(IMG) \
  		-chardev stdio,id=ch0,signal=off \
  		-serial chardev:ch0 \
  		-chardev socket,id=ch1,host=127.0.0.1,port=1234,server=on,wait=off,telnet=off \
  		-serial chardev:ch1 \
		-display gtk
		$(QEMU_EXTRA) &

# ===== Utilities =====
.PHONY: clean
clean:
	@echo "==> Cleaning cargo targets"
	-cd $(BOOT_DIR) && cargo clean
	-cd $(KERNEL_DIR) && cargo clean

.PHONY: distclean
distclean:
	@echo "==> Removing ESP: $(ESP)"
	rm -rf "$(IMG)"

.PHONY: tree
tree:
	@echo "==> Expected artifacts"
	@echo "  Boot EFI  : $(BOOT_EFI)"
	@echo "  Kernel ELF: $(KERNEL_ELF)"
	@echo "  ESP boot  : $(ESP_BOOTX64)"
	@echo "  ESP kernel: $(ESP_KERNEL)"

.PHONY: help
help:
	@echo "Targets:"
	@echo "  all            - Build bootloader & kernel, and populate ESP"
	@echo "  boot           - Build bootloader only"
	@echo "  kernel         - Build kernel only"
	@echo "  esp-populate   - Copy artifacts into ESP (runs boot+kernel automatically)"
	@echo "  run            - Launch QEMU with the ESP"
	@echo "  clean          - cargo clean both crates"
	@echo "  distclean      - clean + remove ESP directory"
	@echo "Variables (override like VAR=value make run):"
	@echo "  RUSTUP=$(RUSTUP)"
	@echo "  TOOLCHAIN=$(TOOLCHAIN)"
	@echo "  ESP=$(ESP)"
	@echo "  OVMF_CODE=$(OVMF_CODE)"
	@echo "  QEMU=$(QEMU)  QEMU_MACHINE=$(QEMU_MACHINE)  QEMU_MEM=$(QEMU_MEM)  QEMU_EXTRA='$(QEMU_EXTRA)'"
