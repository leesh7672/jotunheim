# Makefile — Jotunheim OS (bootloader + kernel + ESP + QEMU) [FreeBSD bmake]

# ===== Toolchains / versions =====
RUSTUP           ?= rustup
TOOLCHAIN        ?= stable
PROFILE          ?= debug                # debug|release
FEATURES         ?=
CARGO            ?= cargo

# ===== Paths =====
BOOT_DIR         := jotunboot
KERNEL_DIR       := jotunheimkernel
TARGET_DIR_BOOT  := ${BOOT_DIR}/target/x86_64-unknown-uefi/${PROFILE}
TARGET_DIR_KRN   := ${KERNEL_DIR}/target/x86_64-unknown-none/${PROFILE}

BOOT_EFI_NAME    := jotunboot.efi
KERNEL_ELF_NAME  := jotunheim-kernel

BOOT_EFI         := ${TARGET_DIR_BOOT}/${BOOT_EFI_NAME}
KERNEL_ELF       := ${TARGET_DIR_KRN}/${KERNEL_ELF_NAME}

ESP              ?= ::
IMG              ?= ${PWD}/image-${PROFILE}.img
ESP_EFI_DIR      := ${ESP}/EFI
ESP_BOOT_DIR     := ${ESP_EFI_DIR}/BOOT
ESP_OS_DIR       := ${ESP}/JOTUNHEIM
ESP_BOOTX64      := ${ESP_BOOT_DIR}/BOOTX64.EFI
ESP_KERNEL       := ${ESP_OS_DIR}/KERNEL.ELF

# ===== QEMU / UEFI firmware =====
QEMU             ?= qemu-system-x86_64
QEMU_MACHINE     ?= q35
QEMU_MEM         ?= 8G
CPU_FLAGS        ?= max
OVMF_CODE        ?= /usr/local/share/edk2-qemu/QEMU_UEFI_CODE-x86_64.fd
QEMU_EXTRA       ?=

# ===== Derived (BSD make conditionals) =====
CARGO_FLAGS      :=
.if ${PROFILE} == "release"
CARGO_FLAGS      += --release
.endif
.if !empty(FEATURES)
CARGO_FLAGS      += --features ${FEATURES}
.endif

# ===== Default target =====
.PHONY: all
all: esp-populate

# ===== Preflight =====
.PHONY: check-tools
check-tools:
	@command -v ${QEMU} >/dev/null || { echo "Missing ${QEMU}"; exit 1; }
	@command -v mmd >/dev/null || { echo "Missing mtools (mmd)"; exit 1; }
	@command -v mcopy >/dev/null || { echo "Missing mtools (mcopy)"; exit 1; }
	@command -v newfs_msdos >/dev/null || { echo "Missing newfs_msdos"; exit 1; }
	@test -r "${OVMF_CODE}" || { echo "OVMF_CODE not found: ${OVMF_CODE}"; exit 1; }

# ===== Build targets =====
.PHONY: boot
boot: ${BOOT_EFI}

${BOOT_EFI}:
	@echo "==> Building bootloader (${PROFILE})"
	cd ${BOOT_DIR} && ${RUSTUP} run ${TOOLCHAIN} ${CARGO} build ${CARGO_FLAGS}
	@test -r "${BOOT_EFI}" || { echo "Boot EFI not found: ${BOOT_EFI}"; exit 1; }

.PHONY: kernel
kernel: ${KERNEL_ELF}

${KERNEL_ELF}:
	@echo "==> Building kernel (${PROFILE})"
	cd ${KERNEL_DIR} && ${RUSTUP} run ${TOOLCHAIN} ${CARGO} build ${CARGO_FLAGS}
	@test -r "${KERNEL_ELF}" || { echo "Kernel ELF not found: ${KERNEL_ELF}"; exit 1; }

# ===== Image / ESP =====
.PHONY: image
image: ${IMG}

${IMG}:
	@echo "==> Generating FAT32 image: $@"
	@rm -f "$@"
	@dd if=/dev/zero of="$@" bs=1G count=0 seek=4 status=none
	@newfs_msdos -F32 -L JOTUN-ESP "$@"

.PHONY: esp-prep
esp-prep: check-tools image
	@echo "==> Preparing ESP directories"
	@mmd   -i "${IMG}" -D o ::/EFI           || true
	@mmd   -i "${IMG}" -D o "${ESP_BOOT_DIR}" || true
	@mmd   -i "${IMG}" -D o "${ESP_OS_DIR}"   || true

.PHONY: esp-populate
esp-populate: boot kernel esp-prep
	@echo "==> Copying artifacts to ESP"
	@mcopy -i "${IMG}" -b -o "${BOOT_EFI}"   "${ESP_BOOTX64}"
	@mcopy -i "${IMG}" -b -o "${KERNEL_ELF}" "${ESP_KERNEL}"
	@echo "==> ESP ready: ${IMG}"

# ===== Run in QEMU =====
.PHONY: run
run: check-tools esp-populate
	@echo "==> Launching QEMU (${PROFILE})"
	${QEMU} \
	  -machine ${QEMU_MACHINE} -m ${QEMU_MEM} -cpu ${CPU_FLAGS} \
	  -drive if=pflash,format=raw,readonly=on,file="${OVMF_CODE}" \
	  -drive format=raw,file="${IMG}" \
	  -chardev stdio,id=ch0,signal=off \
	  -serial chardev:ch0 \
	  -display gtk \
	  ${QEMU_EXTRA}

.PHONY: run-debug
run-debug: check-tools esp-populate
	@echo "==> Launching QEMU with RSP on tcp:1234"
	${QEMU} \
	  -machine ${QEMU_MACHINE} -m ${QEMU_MEM} -cpu ${CPU_FLAGS} \
	  -drive if=pflash,format=raw,readonly=on,file="${OVMF_CODE}" \
	  -drive format=raw,file="${IMG}" \
	  -chardev stdio,id=ch0,signal=off \
	  -serial chardev:ch0 \
	  -chardev socket,id=ch1,host=127.0.0.1,port=1234,server=on,wait=off,telnet=off \
	  -serial chardev:ch1 \
	  -display gtk \
	  ${QEMU_EXTRA}

.PHONY: run-headless
run-headless: check-tools esp-populate
	@echo "==> Launching QEMU (headless)"
	${QEMU} \
	  -machine ${QEMU_MACHINE} -m ${QEMU_MEM} -cpu ${CPU_FLAGS} \
	  -drive if=pflash,format=raw,readonly=on,file="${OVMF_CODE}" \
	  -drive format=raw,file="${IMG}" \
	  -chardev stdio,id=ch0,signal=off \
	  -serial chardev:ch0 \
	  -nographic \
	  ${QEMU_EXTRA}

# ===== Utilities =====
.PHONY: size
size: boot kernel
	@echo "==> Artifact sizes"
	@ls -lh "${BOOT_EFI}" "${KERNEL_ELF}" 2>/dev/null || true
	@du -h  "${IMG}" 2>/dev/null || true

.PHONY: clean
clean:
	@echo "==> Cleaning cargo targets"
	-@cd ${BOOT_DIR}   && ${CARGO} clean
	-@cd ${KERNEL_DIR} && ${CARGO} clean

.PHONY: distclean
distclean: clean
	@echo "==> Removing image: ${IMG}"
	@rm -f "${IMG}"

.PHONY: tree
tree:
	@echo "==> Expected artifacts"
	@printf "  Boot EFI  : %s\n" "${BOOT_EFI}"
	@printf "  Kernel ELF: %s\n" "${KERNEL_ELF}"
	@printf "  ESP boot  : %s\n" "${ESP_BOOTX64}"
	@printf "  ESP kernel: %s\n" "${ESP_KERNEL}"

.PHONY: help
help:
	@echo "Targets:"
	@echo "  all, boot, kernel, image, esp-prep, esp-populate, run, run-debug, run-headless,"
	@echo "  size, clean, distclean, tree, check-tools"
	@echo ""
	@echo "Vars: TOOLCHAIN=${TOOLCHAIN} PROFILE=${PROFILE} FEATURES='${FEATURES}'"
	@echo "      IMG=${IMG}"
	@echo "      OVMF_CODE=${OVMF_CODE}"
	@echo "      QEMU=${QEMU} QEMU_MACHINE=${QEMU_MACHINE} QEMU_MEM=${QEMU_MEM} CPU_FLAGS=${CPU_FLAGS}"
	@echo "      QEMU_EXTRA='${QEMU_EXTRA}'"
