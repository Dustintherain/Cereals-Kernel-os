# Cerlesse OS — 构建入口（v0.1 实现）
# 详见 DEVELOPMENT.md 第 5、7 节。
#
# 纯内核化（见 docs/pure-kernel.md 阶段四）：本 Makefile 只管**内核侧**
#   - 引导/内核构建、镜像组装、QEMU 测试；
#   - 「内核不依赖用户态」由 image-kernel / test-kernel-only 与 `make check-boundary`
#     （tools/check-boundary.py）保证。
# 用户态是**独立 workspace**：`make userland-images` 转发到 `make -C userland images`，
# 产物是 $(USERLAND_IMAGES)/manifest.txt —— 内核构建脚本按它内嵌；清单不存在时
# 内核照样构建与启动（进入串口诊断模式）。

CARGO ?= cargo
QEMU  ?= qemu-system-x86_64
PYTHON ?= python3
GO    ?= go

BUILD := build
OVMF_CODE ?= /usr/share/OVMF/OVMF_CODE.fd
OVMF_VARS ?= /usr/share/OVMF/OVMF_VARS.fd

BOOT_EFI := target/x86_64-unknown-uefi/debug/boot.efi
KERNEL_ELF := target/x86_64-unknown-none/debug/kernel

# 用户态（独立 workspace）：本 Makefile 不碰它的 rustflags / profile ——
# large 代码模型、user-release profile、user.ld 都在 userland/ 里管，
# 两边各自一个 `.cargo/config.toml`（见 userland/.cargo/config.toml）。
USERLAND        := userland
USERLAND_IMAGES := $(BUILD)/userland/images

.PHONY: help build boot kernel kernel-nouserland image image-kernel image-exception image-pagefault fat-image storage-images run test test-kernel-only test-exception test-memory test-pagefault test-keyboard test-shell test-proc test-programs test-dyn dyn-hostcheck test-scheduler test-ipc test-fs test-user test-ckm test-ckm-store test-driver test-ahci test-fat test-storage-fixtures test-storage test-storage-mbr test-network test-all user userland-images test-pkg test-all-with-userland abi-check go-test test-orch test-orch-userland check-boundary check clean

help:
	@echo "targets:"
	@echo "  build  - build bootloader + kernel + disk image"
	@echo "  run    - boot the image in QEMU (serial on stdio)"
	@echo "  test   - integration test: assert 'Kernel started!' on serial"
	@echo "  test-exception - v0.2: trigger #DE, assert exception diagnostics"
	@echo "  test-memory - v0.3/v0.4: assert frame/heap/paging/context-switch self-tests"
	@echo "  test-pagefault - v0.3: touch unmapped address, assert #PF diagnostics"
	@echo "  test-keyboard - v0.4/v0.9: inject keys via QEMU monitor, assert IRQ1 markers + line buffer + PIT heartbeat"
	@echo "  test-shell - v0.9: inject keys into the interactive shell (help / cat /etc/motd / exit) via QEMU monitor"
	@echo "  test-proc - v0.9: fork/execve/wait4 through the shell: external program + argv + missing program + reap"
	@echo "  test-programs - v0.9: run /bin/{ls,cat,echo,ps} from the shell (getdents + getprocs + file syscalls)"
	@echo "  test-dyn - v0.9: dynamic linking loop (PT_INTERP + libgreet.so + RELATIVE/JUMP_SLOT)"
	@echo "  dyn-hostcheck - v0.9: forwards to `make -C userland dyn-hostcheck` (host glibc ld.so cross-check)"
	@echo "  test-scheduler - v0.5: assert >=3 kernel tasks rotate under PIT time slices"
	@echo "  test-ipc - v0.5: assert pipe create/write/read summary logs + selftest"
	@echo "  test-fs - v0.7: assert VFS mount routing + RAMFS read/write/ls + negative errno cases"
	@echo "  test-user - v0.7/v0.9/v1.0: assert ELF loaded from RAMFS + ring3 file syscalls + user heap + mprotect + isolation + shell"
	@echo "  test-ckm - v1.0: CKM-1 + CKM-2 — capability policy (EPERM vs EACCES) + audit ring + component state + overlay + persistent state area"
	@echo "  test-ckm-store - CKM-2: write -> restart -> read back (dual-slot A/B state area on a private disk copy)"
	@echo "  test-driver - v0.8: assert PCI enumeration + driver probe + virtio-blk sector read/write"
	@echo "  test-ahci - v1.0: assert the AHCI/SATA driver on the chipset controller (00:1f.2) + block selftest on a SATA disk"
	@echo "  fat-image - v0.8: build the FAT test volume (tools/mkfs/mkfs.py from tests/fatroot/data)"
	@echo "  test-fat - v0.8: assert FAT16/32 on disk mounted in VFS: read/ls/LFN/multi-cluster/EROFS"
	@echo "  storage-images - CSS-1: build GPT/MBR fixtures (mkfs.py; ext4 payload from host mkfs.ext4)"
	@echo "  test-storage-fixtures - CSS-1: host-side fixture gate (our 2nd parser + partx/mtools/dumpe2fs)"
	@echo "  test-storage - CSS-1/CSS-2: GPT family + VFS mount collapse (vol_id on mounts, alias mount, umount, volcheck)"
	@echo "  test-storage-mbr - CSS-1/CSS-2: MBR family — primary table + EBR chain (logical 5/6) + broken chain"
	@echo "  test-network - v0.9: assert virtio-net + Ethernet/ARP/IPv4/UDP (ARP gateway + DHCP UDP round-trip)"
	@echo "  userland-images - build the userland (independent workspace): programs + dyn fixtures + manifest"
	@echo "  user   - alias of userland-images"
	@echo "  image-kernel - kernel-only disk image (no userland images embedded)"
	@echo "  test-kernel-only - pure-kernel acceptance: boot + self-tests + diagnostic idle loop without userland"
	@echo "  check-boundary - assert the kernel does not depend on userland (tools/check-boundary.py)"
	@echo "  abi-check - phase 3: assert the syscall ABI has a single source of truth (abi crate, host-side)"
	@echo "  check   - host-side gate: abi-check + check-boundary + go-test"
	@echo "  test-pkg - H-06: 转发到 userland/pkgmgr 的包管理器验收（.ckc 容器 + cpm 事务引擎，主机侧）"
	@echo "  test-all-with-userland - 内核 test-all + 用户态集成测试（需要 userland/ 或 CERLESSE_USERLAND_DIR）"
	@echo "  test-orch-userland - 用户态集成场景并行编排（tests/orchestrate.json 里 tags 含 userland 的 6 个场景）"
	@echo "  test-all - run every integration test"

	@echo "  go-test  - v0.4 H-04/H-05: Go unit tests for tools-go (host side)"
	@echo "  test-orch - v0.4 H-04: 内核场景并行编排（tests/orchestrate.json 里 tags 不含 userland 的 13 个场景）"
	@echo "  clean  - remove build artifacts"

build: image

boot:
	$(CARGO) build -p boot --target x86_64-unknown-uefi

kernel:
	$(CARGO) build -p kernel --target x86_64-unknown-none

# 完整镜像。**顺序很重要**：内核构建脚本按 manifest.txt 内嵌用户态镜像，
# 所以必须是「先出镜像 → 再编内核 → 最后组装」；在内核之前就编好的话，
# 内核会（在污染后的缓存里）以「无用户态」的状态被构建出来。
# 内核对用户态的依赖仅此一步，而且是**构建输入文件**而非源码。
image: boot userland-images
	@$(MAKE) --no-print-directory kernel
	$(PYTHON) tools/image-builder/build_image.py \
		--boot $(BOOT_EFI) \
		--kernel $(KERNEL_ELF) \
		--out $(BUILD)/disk.img

# 纯内核镜像：显式置空 CERLESSE_USERLAND_DIR → 内核构建脚本生成空清单，
# 于是内核必须在「没有用户态」的前提下自己构建出来。
kernel-nouserland:
	CERLESSE_USERLAND_DIR= $(CARGO) build -p kernel --target x86_64-unknown-none

image-kernel: boot kernel-nouserland
	$(PYTHON) tools/image-builder/build_image.py \
		--boot $(BOOT_EFI) \
		--kernel $(KERNEL_ELF) \
		--out $(BUILD)/disk-kernel.img

run: image
	cp $(OVMF_VARS) $(BUILD)/OVMF_VARS.fd
	$(QEMU) -machine q35 -m 512M \
		-drive if=pflash,format=raw,readonly=on,file=$(OVMF_CODE) \
		-drive if=pflash,format=raw,file=$(BUILD)/OVMF_VARS.fd \
		-drive if=none,id=blk0,format=raw,file=$(BUILD)/disk.img \
		-device virtio-blk-pci,drive=blk0,disable-modern=on \
		-serial stdio -display none -no-reboot

# 纯内核验收（docs/pure-kernel.md 验收标准 1/2/7）：没有任何用户态镜像时，
# 内核照样启动，跑完文件系统/驱动/调度/IPC 自测，然后进入可诊断的空闲状态。
test-kernel-only: image-kernel
	$(PYTHON) tests/test_boot.py \
		--disk $(BUILD)/disk-kernel.img \
		--serial-log $(BUILD)/serial-kernel-only.log \
		--expect "Kernel started!" \
		--expect "fs: mount point=/ fs=ramfs" \
		--expect "fs: disk selftest PASS" \
		--expect "sched: round-robin wrap PASS" \
		--expect "pipe: selftest PASS" \
		--expect "user: no userland images embedded; entering diagnostic idle loop" \
		--expect "user: kernel-only boot PASS"

test: image
	$(PYTHON) tests/test_boot.py

image-exception: boot
	$(CARGO) build -p kernel --target x86_64-unknown-none --features exception-test
	$(PYTHON) tools/image-builder/build_image.py \
		--boot $(BOOT_EFI) \
		--kernel $(KERNEL_ELF) \
		--out $(BUILD)/disk-exception.img

image-pagefault: boot
	$(CARGO) build -p kernel --target x86_64-unknown-none --features pagefault-test
	$(PYTHON) tools/image-builder/build_image.py \
		--boot $(BOOT_EFI) \
		--kernel $(KERNEL_ELF) \
		--out $(BUILD)/disk-pagefault.img

test-exception: image-exception
	$(PYTHON) tests/test_boot.py \
		--disk $(BUILD)/disk-exception.img \
		--serial-log $(BUILD)/serial-exception.log \
		--expect "Kernel started!" \
		--expect "EXCEPTION: divide error"

# v0.3/v0.4 内核自测验收：帧分配器 + 内核堆 + 页表 Mapper + 上下文切换原语
test-memory: image
	$(PYTHON) tests/test_boot.py \
		--serial-log $(BUILD)/serial-memory.log \
		--expect "Kernel started!" \
		--expect "heap: Box/Vec/String PASS" \
		--expect "frame: stress PASS" \
		--expect "paging: map/unmap PASS" \
		--expect "context: switch PASS"

# v0.3 验收：页错误诊断（feature 门控注入未映射地址访问）
test-pagefault: image-pagefault
	$(PYTHON) tests/test_boot.py \
		--disk $(BUILD)/disk-pagefault.img \
		--serial-log $(BUILD)/serial-pagefault.log \
		--expect "Kernel started!" \
		--expect "EXCEPTION: page fault" \
		--expect "CR2=0x500000000000" \
		--expect "KERNEL PANIC: unhandled exception"

# 全量回归：先跑主机侧边界门禁（内核不得依赖用户态），再跑全部 QEMU 场景 ——
# 包含 test-kernel-only（没有用户态镜像时内核照样启动 + 进入可诊断状态）。
test-all: abi-check check-boundary test test-kernel-only test-exception test-memory test-pagefault test-keyboard test-scheduler test-ipc test-fs test-ckm-store test-driver test-ahci test-fat test-storage-fixtures test-storage test-storage-mbr test-network

# 交叉仓库：内核 test-all + 用户态集成测试（需要 userland/ 或 CERLESSE_USERLAND_DIR）。
# 这些目标断言 `/init`/`/bin/*`/`libc` 的输出，因此**不属于内核 test-all**（见 docs/pure-kernel.md 阶段五）。
test-all-with-userland: userland-images test-all test-user test-shell test-proc test-programs test-dyn dyn-hostcheck test-ckm test-pkg

# v0.5 验收：≥3 个内核任务在 PIT 时间片驱动下轮转（Round Robin）
test-scheduler: image
	$(PYTHON) tests/test_boot.py \
		--serial-log $(BUILD)/serial-sched.log \
		--expect "Kernel started!" \
		--expect "sched: spawn pid=1 name=task_a" \
		--expect "task_a: entered" \
		--expect "task_b: entered" \
		--expect "task_c: entered" \
		--expect "sched: switch pid=3 -> 0" \
		--expect "sched: round-robin wrap PASS"

# v0.7 验收：VFS（挂载点路由 / RAMFS 读写 / 目录列举 / 负例 errno）。
#
# 断言里的数字都是「内核真的做过这件事」的证据而不是日志措辞：
#   /init 的字节数来自内嵌 ELF（断言只写前缀，避免用户程序一改就失效），
#   /etc/motd=23 字节并打印内容，/mnt 走的是**另一个**后端（ramfs-scratch）。
test-fs: image
	$(PYTHON) tests/test_boot.py \
		--serial-log $(BUILD)/serial-fs.log \
		--expect "Kernel started!" \
		--expect "fs: mount point=/ fs=ramfs" \
		--expect "fs: create /init" \
		--expect "fs: write /init bytes=" \
		--expect "fs: mkdir /etc" \
		--expect "fs: create /etc/motd" \
		--expect "fs: write /etc/motd bytes=23" \
		--expect "fs: mkdir /tmp" \
		--expect "fs: stat /etc/motd kind=file size=23 perms=-rw-r--r-- vol=-" \
		--expect "fs: read /etc/motd len=23 data=motd: hello from ramfs" \
		--expect "fs: mount point=/mnt fs=ramfs-scratch" \
		--expect "fs: route /mnt/note.txt fs=ramfs-scratch" \
		--expect "fs: readback /mnt/note.txt len=7 data=v0.7-ok" \
		--expect "fs: ls / entries=5" \
		--expect "fs: ls / name=init kind=file size=" \
		--expect "fs: ls / name=bin kind=dir size=0 perms=drwxr-xr-x" \
		--expect "fs: ls / name=lib kind=dir size=0 perms=drwxr-xr-x" \
		--expect "fs: ls / name=etc kind=dir size=0 perms=drwxr-xr-x" \
		--expect "fs: ls / name=tmp kind=dir size=0 perms=drwxr-xr-x" \
		--expect "fs: negative cases PASS" \
		--expect "fs: selftest PASS"

# v0.7 验收：syscall（syscall/sysret + EFER/STAR/LSTAR/FMASK）+ 用户态 ring3 + 地址空间隔离
# + **ELF Loader 从 RAMFS 加载 /init** + 文件 syscall（open/read/close/write）。
# v0.9 第一步追加**用户堆**（brk/mmap/munmap + `#[global_allocator]`）：
#   - 地址是**可算的**，所以直接断言：brk 起点 0x8000020000（增长两页到 0x8000022000）、
#     mmap 窗口首次分配落在 0x8000060000（first-fit）；
#   - `hello: heap zeroed=1` / `hello: mmap zeroed=1`：匿名内存不得泄漏上一任内容；
#   - `user: write fd=1 len=26` + `hello: heap write ret=26`：把**堆上**的缓冲交给内核写串口，
#     证明 `write` 的按页缓冲校验（`page_flags`）认可堆页而不只是映像/栈；
#   - 每个负例都要**具体**的 errno：文件映射/`MAP_SHARED`/W^X/未对齐 munmap = -22，超窗口 = -12；
#   - `hello: vec len=200 sum=20100`：`Vec`/`format!` 真的走通了全局分配器 → 内核 mmap；
#   - `user: heap frame reclamation PASS`：两次运行的地址空间（含堆页）整棵归还，不成帧泄漏。
#
# 两个程序的来源不同，断言里保留了这个区别：
#   /init —— `user/hello` 编出的 **ELF**：内核从 RAMFS 读它，由 elf.rs 按段权限装载，
#            write/getpid/open/read/close/nanosleep/exit 全链路往返，exit code=42；
#   probe —— 内嵌位置无关机器码（map_raw），直接读内核映像地址（0x2000000），
#            必须被 ring3 #PF 拦下（error=P|U）。
# v0.9 第二步追加**交互式 Shell**（`/bin/sh` = `user/shell`）：命令来自键盘行缓冲（stdin, fd 0），
#   没人按键时它会两次 `-EAGAIN` 后自行退出 —— 因此这条非交互路径断言的是
#   `user: program=/bin/sh source=elf` 与 `user: shell PASS`（交互路径见 `make test-shell`）。
# 硬证据（不是日志措辞）：
#   - `source=elf` + `segments=4 image_pages=7 writable_pages=2`：ELF 装载真的发生了，
#     且 RW 段（.data/.bss）确实被映射成可写；
#   - `user: read fd=3 want=64 got=23` 紧跟 `got=0`：fd 偏移真的在推进（EOF 语义）；
#   - `fs: readback /tmp/hello.txt ... written by user program`：用户在 ring3 写的内容
#     被内核从 RAMFS 读回来（跨态数据往返）；
#   - `hello: static counter=1`：RW 段对 ring3 可写（v0.6 会 #PF）；
#   - `user: EFAULT write ptr=0xdead00000000` / `bad write ret=-14`：用户指针被校验；
#   - `FAULT cr2=0x2000000 error=0x5`：越权读内核页被拦下（error 变了说明触发的不是权限违例）。
#
# v1.0 第一步追加**`mprotect`**（运行时改保护属性，61 → 91 项）：断言的是「改完之后内核/硬件真的变了」，
# 而不是「调用返回 0」：
#   - `user: mprotect addr=0x8000070000 len=4096 prot=0x0 -> pages=1`：内核逐页改页表项
#     （地址确定：堆自测的 mmap 已归还，全局分配器的 64 KiB 块占掉了前 16 页）；
#   - `user: EFAULT write ptr=0x8000070000 len=1` + `hello: mprotect none write ret=-14`：
#     `PROT_NONE` 的页**既**对 ring3 不可访问，**也**不再被当成合法用户缓冲 —— 这是「权限位
#     真的改了」的双向证据；
#   - `hello: mprotect data kept=1`：改回 `RW` 后内容原样（`mprotect` 不搬家、不清零）；
#   - `hello: mprotect exec ret=42`：换成 `RX` 后**从这页真的执行**了 `mov eax,42; ret`
#     —— W^X 的「先写、再换只执行」（在此之前 `mprotect` 落在 ENOSYS 桩上，这条路不通）；
#   - 6 类拒绝各自的 errno 与串口原因：未页对齐 / `len=0` / 未知保护位 / `W^X`（都是 -22，
#     但日志里能区分是哪一条）、未映射的页 -22、用户栈 -22（栈的保护属性由内核定死）；
#   - `user: mprotect addr=0x8000020000 …` + `hello: mprotect heap data kept=1`：brk 堆窗口同样在支持
#     范围内（逐页 `heap_flags` 的意义），`munmap` 与页帧泄漏检查照旧通过。
test-user: image
	$(PYTHON) tests/test_boot.py \
		--serial-log $(BUILD)/serial-user.log \
		--expect "Kernel started!" \
		--expect "syscall: syscall/sysret entry armed" \
		--expect "sched: suspended for user-mode run" \
		--expect "user: selftest begin" \
		--expect "user: program=/init source=elf" \
		--expect "segments=4 image_pages=7 writable_pages=2" \
		--expect "user: enter name=init pid=1 entry=0x8000000000" \
		--expect "hello from user space" \
		--expect "user: write fd=1 len=22" \
		--expect "user: getpid -> 1" \
		--expect "user: open path=/etc/motd flags=0x0 fd=3" \
		--expect "user: read fd=3 want=64 got=23" \
		--expect "motd: hello from ramfs" \
		--expect "user: read fd=3 want=64 got=0" \
		--expect "user: close fd=3" \
		--expect "user: open path=/tmp/hello.txt flags=0x41 fd=3" \
		--expect "user: write fd=3 len=24" \
		--expect "hello: static counter=1" \
		--expect "user: EFAULT write ptr=0xdead00000000 len=1" \
		--expect "hello: bad write ret=-14" \
		--expect "user: brk query -> 0x8000020000" \
		--expect "user: brk addr=0x8000022000 -> 0x8000022000 pages=2" \
		--expect "user: program=/bin/sh source=elf" \
		--expect "user: shell PASS (keyboard line buffer -> stdin -> builtins + external program)" \
		--expect "hello: brk grown=0x8000022000" \
		--expect "hello: heap zeroed=1" \
		--expect "hello: heap pattern=1" \
		--expect "user: brk rejected addr=0x8000120000" \
		--expect "hello: brk over ret=0x8000022000 refused=1" \
		--expect "user: mmap len=8192 prot=0x3 flags=0x22 -> 0x8000060000 pages=2" \
		--expect "hello: mmap zeroed=1" \
		--expect "hello: mmap pattern=1" \
		--expect "user: write fd=1 len=26" \
		--expect "heap buffer through write" \
		--expect "hello: heap write ret=26" \
		--expect "user: munmap addr=0x8000060000 len=8192" \
		--expect "hello: munmap ret=0" \
		--expect "user: mmap rejected (file-backed mmap is not supported" \
		--expect "hello: mmap file-backed ret=-22" \
		--expect "hello: mmap shared ret=-22" \
		--expect "user: mmap errno=12 len=266240" \
		--expect "hello: mmap too big ret=-12" \
		--expect "hello: mmap wx ret=-22" \
		--expect "hello: munmap misaligned ret=-22" \
		--expect "hello: munmap twice ret=-22" \
		--expect "hello: vec len=200 sum=20100" \
		--expect "hello: heap string bytes=200" \
		--expect "hello: heap PASS=1" \
		--expect "user: mprotect addr=0x8000070000 len=4096 prot=0x0 -> pages=1" \
		--expect "hello: mprotect none ret=0" \
		--expect "user: EFAULT write ptr=0x8000070000 len=1" \
		--expect "hello: mprotect none write ret=-14" \
		--expect "user: mprotect addr=0x8000070000 len=4096 prot=0x3 -> pages=1" \
		--expect "hello: mprotect rw ret=0" \
		--expect "hello: mprotect data kept=1" \
		--expect "user: mprotect addr=0x8000070000 len=4096 prot=0x5 -> pages=1" \
		--expect "hello: mprotect rx ret=0" \
		--expect "hello: mprotect exec ret=42" \
		--expect "user: mprotect rejected (addr is not page-aligned)" \
		--expect "hello: mprotect misaligned ret=-22" \
		--expect "user: mprotect rejected (len=0)" \
		--expect "hello: mprotect len0 ret=-22" \
		--expect "user: mprotect rejected (unknown PROT bits)" \
		--expect "hello: mprotect unknown prot ret=-22" \
		--expect "user: mprotect rejected (PROT_WRITE|PROT_EXEC is not allowed (W^X))" \
		--expect "hello: mprotect wx ret=-22" \
		--expect "user: mprotect errno=22 addr=0x8000071000 len=4096" \
		--expect "hello: mprotect unmapped ret=-22" \
		--expect "hello: mprotect stack ret=-22" \
		--expect "user: mprotect addr=0x8000020000 len=4096 prot=0x0 -> pages=1" \
		--expect "hello: mprotect heap none ret=0" \
		--expect "user: EFAULT write ptr=0x8000020000 len=1" \
		--expect "hello: mprotect heap none write ret=-14" \
		--expect "user: mprotect addr=0x8000020000 len=4096 prot=0x3 -> pages=1" \
		--expect "hello: mprotect heap back ret=0" \
		--expect "hello: mprotect heap data kept=1" \
		--expect "hello: mprotect munmap ret=0" \
		--expect "hello: mprotect PASS=1" \
		--expect "user: sleep ticks=2" \
		--expect "user: sleep done ticks=2" \
		--expect "user: exit code=42" \
		--expect "user: init PASS" \
		--expect "hello: ckm selftest begin" \
		--expect "hello: ckm policy_query(state_write) -> -1 need=0x2" \
		--expect "hello: ckm component_state -> 3 count=3" \
		--expect "hello: ckm open /etc/init.flag fd=3" \
		--expect "hello: ckm selftest ok=1" \
		--expect "fs: readback /tmp/hello.txt len=24" \
		--expect "fs: user data: written by user program" \
		--expect "user: enter name=probe pid=2" \
		--expect "user: FAULT vector=0xe error=0x5" \
		--expect "user: FAULT cr2=0x2000000" \
		--expect "user: isolation PASS" \
		--expect "user: probe PASS" \
		--expect "user: heap frame reclamation PASS" \
		--expect "user: selftest PASS"

# v1.0 验收（CKM-1）：**内核级权限控制**——与「日志里有 CKM 字样」无关，
# 断言的是四件机制真的成立（`docs/ckm.md` 第 9 节 CKM-1 的验收出口）：
#
#   1. **能力判定在内核**：内核自测（`ckm: selftest PASS (checks=N)`，46 项）逐项对照
#      能力矩阵，判定行 `ckm: policy op=… result=allow|deny need=…` 在每次判定后必打；
#   2. **缺能力 → `-EPERM`、不属于你 → `-EACCES`**：`/init`（uid 0、只有 `CAP_FS_SYSTEM_WRITE`）
#      能写受保护前缀 `/etc/**`（`fs_write` → 0），但**不能**改扩展状态树（`state_write` → `-1`，
#      `need=0x2`）、**不能**读审计环（`audit_read` → `-1`，`need=0x8`）、不能重扫设备
#      （`storage_rescan` → `-1`，`need=0x10`）—— 「不给 CKM 管理能力」因此是可断言的；
#   3. **记录布局是 ABI**：`/init` 用 **libc 的解析器**读了 48 字节审计记录与 64 字节组件记录
#      （`component_state -> 3 count=3` + 按确定性顺序列出三个组件），偏移读错立刻对不上；
#   4. **审计环可核查**：拒绝必产生审计事件（`ckm: audit seq=N op=… result=deny`），
#      容量 ≥64 且回绕后仍可读（`ckm: audit ring wrapped capacity=64`）。
#
# uid 2000 的「默认拒绝」证据（`-1` EPERM vs `-13` EACCES）由 `test-proc` 断言：
# 那条路径要 `/bin/hello` 真的被 Shell `execve` 起来（键盘注入），不在本目标的观察窗里。
# `--expect-re` 用来绑定「同一个值在多条证据行里一致」（自测项数不算常数）：
# `checks=\d+` 与「两次查询的组件序列逐字节相同」都是 CKM-1 的验收项。
#
# CKM-2 追加的两段机制（`docs/ckm.md` 9 节 CKM-2 行）：
#
#   1. **覆盖层**（同一次启动内可断言）：注册 `demo`（priority=10，前缀 `/etc`）后
#      **新 `open`** 读到覆盖内容（`changed=1 bytes=25` ≠ 基线的 23 字节）；同优先级 +
#      前缀重叠 → `op=overlay_conflict result=deny`；更高优先级 `high` 真的遮蔽 `demo`；
#      逐条撤销 → 先回 `demo` 的视图、再**回到基线**（`docs/ckm.md` 6.4 第 2 条）；
#      收尾时覆盖表必须为空（用户进程看到的是基线）。
#   2. **可写持久化**（**跨重启**，所以交给 `tests/test_ckm_store.py`：清零一份独占镜像
#      → 启动写入（`state=fresh`，槽 A/B 交替）→ 同一份镜像再启动 → `state=loaded`
#      读回逐字节相同且**不重写**）。本目标的第一次启动只断言与「FAT 之外」无关的
#      确定性证据（`verify … ok=1`），因为 `build/disk.img` 里可能已经留有上次 `make run`
#      写入的状态区（`--persist` 才关掉 `snapshot=on`，其它场景都是临时覆盖层）。
test-ckm: image fat-image
	$(PYTHON) tests/test_boot.py \
		--serial-log $(BUILD)/serial-ckm.log \
		--expect "Kernel started!" \
		--expect "ckm: init ABI_VERSION=1 admin_uid=1000 protected=4 audit_cap=64" \
		--expect-re "ckm: selftest PASS \\(checks=[0-9]+\\)" \
		--expect "ckm: component id=init transition=deny from=3 to=2" \
		--expect "ckm: audit ring wrapped capacity=64" \
		--expect "ckm: verify id=kmod result=deny reason=kind" \
		--expect "ckm: verify id=bad id result=deny reason=id" \
		--expect "ckm: verify id=dupe result=deny reason=target" \
		--expect "ckm: policy op=state_write pid=1 uid=0 result=deny need=0x2" \
		--expect "ckm: policy op=audit_read pid=1 uid=0 result=deny need=0x8" \
		--expect "ckm: policy op=storage_rescan pid=1 uid=0 result=deny need=0x10" \
		--expect-re "ckm: audit seq=[0-9]+ op=fs_write result=allow pid=1 uid=0" \
		--expect "hello: ckm selftest begin" \
		--expect "hello: ckm policy_query(policy_query) -> 0 need=0x0" \
		--expect "hello: ckm policy_query(fs_write,etc) -> 0 need=0x0" \
		--expect "hello: ckm policy_query(state_write) -> -1 need=0x2" \
		--expect "hello: ckm audit_read -> -1" \
		--expect "hello: ckm component_state -> 3 count=3" \
		--expect "hello: ckm component[0] id=hello kind=userland state=active" \
		--expect "hello: ckm component[1] id=shell kind=userland state=active" \
		--expect "hello: ckm component[2] id=init kind=system-component state=active" \
		--expect "hello: ckm open /etc/init.flag fd=3" \
		--expect "hello: ckm wrote 30 bytes to /etc/init.flag" \
		--expect "hello: ckm selftest ok=1" \
		--expect "fs: overlay selftest begin" \
		--expect "fs: overlay area /ckm/overlay ready id=demo,high" \
		--expect "ckm: policy op=overlay_register pid=0 uid=0 result=allow need=0x0 detail=0x0" \
		--expect "fs: overlay table[0] id=demo kind=3 priority=10 prefix=/etc" \
		--expect "ckm: overlay path=/etc/motd -> /ckm/overlay/demo/motd id=demo priority=10" \
		--expect "fs: overlay open /etc/motd changed=1 bytes=26" \
		--expect "ckm: policy op=overlay_conflict pid=0 uid=0 result=deny need=0x0 detail=0x0" \
		--expect "fs: overlay conflict rejected (dupe vs demo, same priority)" \
		--expect "ckm: overlay register id=high kind=3 priority=20 prefix=/etc/motd slot=1 shadowed=demo" \
		--expect "ckm: overlay path=/etc/motd -> /ckm/overlay/high id=high priority=20" \
		--expect "fs: overlay priority wins (high=20 > demo=10) bytes=28" \
		--expect "ckm: overlay unregister id=high removed=1 remaining=1 result=allow" \
		--expect "ckm: overlay unregister id=demo removed=1 remaining=0 result=allow" \
		--expect "fs: overlay unregister restored baseline bytes=23" \
		--expect "fs: overlay negative cases PASS (kind/prefix/duplicate)" \
		--expect "fs: overlay selftest PASS" \
		--expect "ckm: store verify key=selftest value=cerlesse-ckm-store-v1 ok=1" \
		--expect "ckm: store verify key=msg value=persist-ok ok=1" \
		--expect-re "ckm: store selftest PASS \\(checks=[0-9]+\\)" \
		--expect "storage: letters load n=0 source=none" \
		--expect "storage: letters table n=1 present=1 first=C" \
		--expect "storage: letters persistence probe skipped (system volume only)" \
		--expect "storage: letters selftest PASS (checks=4)"
	$(PYTHON) tests/test_ckm_store.py --disk2 $(BUILD)/fat.img --serial-log-prefix $(BUILD)/serial-ckm-store

# CKM-2 持久化验收（单跑：只跑「写 → 重启 → 读回」这一段，迭代与编排器场景用）。
# 它**必须**独占一份 `build/disk.img` 的副本（`test_boot.py --persist` 会真的落盘）。
test-ckm-store: image fat-image
	$(PYTHON) tests/test_ckm_store.py \
		--disk2 $(BUILD)/fat.img \
		--serial-log-prefix $(BUILD)/serial-ckm-store

# v0.8 验收：PCI 枚举 → 驱动注册框架 → virtio-blk 挂接 → 块设备读写。
#
# 断言的都是「硬件事实」而不是日志措辞：
#   - `vendor=0x1af4 device_id=0x1001`：QEMU 在 `disable-modern=on` 下暴露的 **legacy**
#     virtio-blk（modern 是 0x1042，被驱动明确拒绝）；
#   - `bar0=io:`：legacy 接口的标志 —— 寄存器在 **I/O 窗口** 里（modern 只有 MMIO）；
#   - `probe PASS (devices=7 matched=2 attached=2 failed=0)`：枚举到了 **7** 个 function
#     （B-18 修好后 00:1f.2/00:1f.3 这两个「多功能设备的后继 function」才看得见），
#     其中 2 个有驱动认领且挂接成功：virtio-blk 与 **ahci**（本场景没插 SATA 盘，
#     所以 AHCI 那行的读数是 `drives=0` —— 有控制器、没盘是常态，不算失败）；
#   - `capacity=131072 sectors (64 MiB)`：容量是设备报的，与 tools/image-builder 的镜像尺寸互相印证；
#   - `boot sector sig=0x55aa fstype=FAT16`：从磁盘真的读回了本次构建的 FAT 引导扇区；
#   - `chunked read PASS`：4608 字节的读跨过驱动内部 8 扇区搬运边界，与两次独立读逐字节一致；
#   - `write/readback PASS (lba=131071 pattern=0xa5)`：最后一个扇区写回读（随后还原原值）。
test-driver: image
	$(PYTHON) tests/test_boot.py \
		--serial-log $(BUILD)/serial-driver.log \
		--expect "Kernel started!" \
		--expect "pci: enumerated devices=" \
		--expect "vendor=0x1af4 device_id=0x1001" \
		--expect "pci: 00:03.0 bar0=io:" \
		--expect "pci: virtio-blk (legacy) at " \
		--expect "driver: registered index=0 name=virtio-blk-pci" \
		--expect "driver: match " \
		--expect "-> virtio-blk-pci" \
		--expect "virtio-blk: host features=" \
		--expect "virtio-blk: queue0 size=" \
		--expect "virtio-blk: capacity=131072 sectors (64 MiB)" \
		--expect "virtio-blk: registered as block device 0" \
		--expect "driver: attached " \
		--expect "driver: probe PASS (devices=" \
		--expect "matched=2 attached=2 failed=0)" \
		--expect "ahci: controller 00:1f.2 drives=0 failures=0" \
		--expect "block: registered devices=1" \
		--expect "block: device=virtio-blk0 sector_size=512 sectors=131072" \
		--expect "block: boot sector sig=0x55aa fstype=FAT16" \
		--expect "block: chunked read PASS" \
		--expect "block: write/readback PASS (lba=131071 pattern=0xa5)" \
		--expect "block: negative cases PASS" \
		--expect "block: selftest PASS"

# v1.0 驱动兼容性验收：AHCI/SATA —— **真实机器上的磁盘控制器**。
#
# q35 的芯片组本身就带一个 ICH9 AHCI 控制器（00:1f.2 / 8086:2922 /
# class 0x01 subclass 0x06 prog_if 0x01），所以这里不加控制器，只把一块盘插到
# 它的 2 号端口上（QEMU 里 q35 的 SATA 端口叫 ide.0..ide.5；2 号端口就是固件
# 引导日志里那个 `Pci(0x1F,0x2)/Sata(0x2,...)`）。
#
# 断言的都是「第二条块设备路径」上的硬件事实，而不是日志措辞：
#   - `pci: 00:1f.2 vendor=0x8086 device_id=0x2922 class=0x1 subclass=0x6 prog_if=0x1`：
#     这是 00:1f.0（ISA 桥）的 **function 2**。修 B-18（头部类型读的是 0x0C —— cache
#     line size，恒为 0 —— 而不是 0x0E）之前，多功能设备的 function 1..7 从来没被
#     枚举过，内核「看不见」这台机器上唯一真实存在的磁盘控制器；
#   - `ahci: port 2 det=3 sig=0x101 (SATA disk)` + `model=QEMU HARDDISK capacity=...`：
#     型号与容量来自 IDENTIFY DEVICE（0xEC）的返回，即设备本身，不是常量；
#   - `block: device=ahci0 ...` 起的一整套 `block::selftest`：与 virtio-blk **同一套**
#     断言（引导扇区 / 跨 4 KiB 搬运边界的分块读 / 末扇区写回读后还原 / 负例）——
#     「SATA 这条路也能读写」因此不是另写一份测试，而是同一批事实在另一条路径上成立；
#   - `fs: mount point=/mnt/fat fs=fat32 ...` + `fs: disk selftest PASS`：同一张 FAT32
#     镜像经 SATA 读出来挂进 VFS，文件内容按公式逐字节比对也成立。
test-ahci: image fat-image
	$(PYTHON) tests/test_boot.py \
		--ahci \
		--ahci-disk $(BUILD)/fat.img \
		--serial-log $(BUILD)/serial-ahci.log \
		--expect "Kernel started!" \
		--expect "pci: 00:1f.2 vendor=0x8086 device_id=0x2922 class=0x1 subclass=0x6 prog_if=0x1" \
		--expect "pci: 00:1f.2 bar5=mmio:" \
		--expect "driver: registered index=2 name=ahci-pci" \
		--expect "driver: match 00:1f.2 -> ahci-pci" \
		--expect "ahci: controller 00:1f.2 abar=" \
		--expect "ahci: version=0x10000" \
		--expect "ahci: port 2 det=3 sig=0x101 (SATA disk)" \
		--expect "ahci: port 2 model=QEMU HARDDISK capacity=131072 sectors (64 MiB) lba48=1" \
		--expect "ahci: registered as block device 1 name=ahci0" \
		--expect "ahci: controller 00:1f.2 drives=1 failures=0" \
		--expect "driver: attached 00:1f.2 -> ahci-pci" \
		--expect "driver: probe PASS (devices=7 matched=2 attached=2 failed=0)" \
		--expect "block: registered devices=2" \
		--expect "ahci: running the block selftest on the SATA disk (block index=1)" \
		--expect "block: device=ahci0 sector_size=512 sectors=131072 bytes=67108864" \
		--expect "block: boot sector sig=0x55aa fstype=FAT32 oem=CERLESSE" \
		--expect "block: chunked read PASS" \
		--expect "block: write/readback PASS (lba=131071 pattern=0xa5)" \
		--expect "block: negative cases PASS" \
		--expect "block: selftest PASS" \
		--expect "storage: disk index=1 dev=ahci0 sectors=131072 table=none partitions=1" \
		--expect "fs: mount point=/mnt/fat fs=fat32 sectors=131072" \
		--expect "fs: disk selftest PASS"

# v0.9 验收：简化网络栈（virtio-net → Ethernet → ARP → IPv4 → UDP）+ TCP/socket 补齐。
#
# QEMU 加一块 legacy virtio-net-pci（`-netdev user` = slirp），断言的是网络上
# **回来的真实内容**而不是日志措辞：
#   - `net: device=virtio-net0 mac=`：驱动从设备配置空间读到了 MAC 并注册成功；
#   - `net: arp 10.0.2.2 -> 52:55:0a:00:02:02 (reply)`：ARP 请求（广播）→ 应答（单播）
#     往返，且 MAC 正是 slirp 网关的固定地址；
#   - `net: dhcp offer yiaddr=10.0.2.15 ...`：DHCP DISCOVER（UDP 68→67 广播）→
#     OFFER（UDP 67→68）往返，报文里的 yiaddr / server-id / mask / router 都被解析出来
#     —— 这是 Ethernet → IPv4 → UDP 收发两条路径都通了的证据；
#   - `net: tcp ... ESTABLISHED` + `send/recv/close`：内核经三次握手连上宿主回声服务
#     （`--tcp-echo-port` 在测试侧起，QEMU 把客户机对 10.0.2.2:18080 的连接转给它），
#     发 12 字节、逐字节收回比对、FIN 完整关闭 —— TCP 是「真握手 + 真字节」；
#   - `tcpecho: PASS` + `user: tcpecho PASS`：`/bin/tcpecho` 走用户态 socket syscall
#     （`socket/connect/sendto/recvfrom` + `write/read` 两轮）与同一回声服务收发成功 ——
#     syscall 路径与内核自测是同一套 TCP 代码（`network::tcp`），两条证据互相印证。
test-network: image
	$(PYTHON) tests/test_boot.py \
		--nic \
		--tcp-echo-port 18080 \
		--serial-log $(BUILD)/serial-network.log \
		--expect "Kernel started!" \
		--expect "pci: enumerated devices=" \
		--expect "vendor=0x1af4 device_id=0x1000" \
		--expect "driver: registered index=1 name=virtio-net-pci" \
		--expect "-> virtio-net-pci" \
		--expect "virtio-net: mac=" \
		--expect "net: device=virtio-net0 mac=" \
		--expect "net: stack mac=" \
		--expect "net: arp 10.0.2.2 -> 52:55:0a:00:02:02 (reply)" \
		--expect "net: arp PASS" \
		--expect "net: dhcp offer yiaddr=10.0.2.15" \
		--expect "net: dhcp PASS" \
		--expect "net: tcp connect 10.0.2.2:18080 ESTABLISHED" \
		--expect "net: tcp send len=12 acked" \
		--expect "net: tcp recv len=12 match=PASS" \
		--expect "net: tcp close FIN-ACK PASS" \
		--expect "net: tcp PASS" \
		--expect "net: selftest PASS" \
		--expect "tcpecho: connected 10.0.2.2:18080" \
		--expect "tcpecho: round 1 sendto/recvfrom PASS" \
		--expect "tcpecho: round 2 write/read PASS" \
		--expect "tcpecho: PASS" \
		--expect "user: tcpecho PASS"

# v0.8 H-03 验收：用 tools/mkfs/mkfs.py 生成「带已知文件集」的 FAT32 测试盘（64 MiB）。
#
# 内容取自 tests/fatroot/data（见该目录的 README：每个文件对应一类断言）。
# 正确性由 **mtools 独立复核**：`mdir -i build/fat.img ::` / `mtype ... ::hello.txt`。
fat-image:
	$(PYTHON) tools/mkfs/mkfs.py --out $(BUILD)/fat.img --from tests/fatroot/data

# v0.8 验收：磁盘上的文件系统（FAT 后端挂进 VFS 后，syscall 层一行未改就能 open/read）。
#
# 断言的都是「磁盘上的事实」而不是日志措辞：
#   - `fs: mount point=/disk fs=fat16 sectors=129024 cluster_bytes=1024 clusters=64244`：
#     启动盘（mtools 写的 ESP）被 **FAT16 解析器** 认出来（类型由簇数判定，不靠扩展名）；
#     这里的 `sectors=129024`（= 63 MiB）而不是设备的 131072（= 64 MiB）：CKM-2 起镜像尾部
#     1 MiB 是**状态区**，FAT 只覆盖前面 63 MiB（`tools/image-builder/build_image.py`）；
#   - `fs: read /disk/KERNEL.ELF magic=0x7f454c46`：从磁盘读回**内核自己的映像**，
#     前 4 字节是 ELF 魔数 —— VFS → FAT → 块设备 → virtio 的完整往返；
#   - `BOOTX64.EFI magic=0x4d5a`（`MZ`）：固件要用的 PE 引导文件在同一张盘上读到；
#   - `fs: mount point=/mnt/fat fs=fat32 sectors=131072 ... clusters=128992`：
#     `mkfs.py` 生成的测试盘，且挂在 `/mnt`（RAMFS）**下面** → 顺带验证最长前缀路由；
#   - `fs: content check PASS ... len=3000`：跨簇文件逐字节对公式（6 个 512 字节簇）；
#   - `fs: offset read PASS ... offset=1024`：任意偏移读（fd 的 read 路径形态）；
#   - `empty file PASS len=0`：首簇为 0 的空文件不会去解引用簇 0；
#   - `long file name PASS`：带空格的长文件名（LFN 目录项 + 校验和）；
#   - `fs: disk negative cases PASS (ENOENT/ENOTDIR/EISDIR/EROFS)`：错误各自具体。
test-fat: image fat-image
	$(PYTHON) tests/test_boot.py \
		--disk $(BUILD)/disk.img \
		--disk2 $(BUILD)/fat.img \
		--serial-log $(BUILD)/serial-fat.log \
		--expect "Kernel started!" \
		--expect "fs: mount point=/disk fs=fat16 sectors=129024 cluster_bytes=1024 clusters=64244" \
		--expect "fs: disk selftest begin" \
		--expect "fs: ls /disk entries=2" \
		--expect "fs: ls /disk name=EFI kind=dir size=0 perms=drwxr-xr-x" \
		--expect "fs: ls /disk name=KERNEL.ELF kind=file size=" \
		--expect "fs: read /disk/KERNEL.ELF magic=0x7f454c46 size=" \
		--expect "fs: read /disk/EFI/BOOT/BOOTX64.EFI magic=0x4d5a route=fat16" \
		--expect "fs: mount point=/mnt/fat fs=fat32 sectors=131072 cluster_bytes=512 clusters=128992" \
		--expect "fs: second volume at /mnt/fat fs=fat32" \
		--expect "fs: ls /mnt/fat entries=4" \
		--expect "fs: read /mnt/fat/hello.txt len=17 data=hello from FAT32" \
		--expect "fs: content check PASS /mnt/fat/dir/nested.bin len=3000 (formula verified byte by byte across 6 clusters)" \
		--expect "fs: offset read PASS /mnt/fat/dir/nested.bin offset=1024 len=8 data=0x" \
		--expect "(matches the formula)" \
		--expect "fs: empty file PASS /mnt/fat/empty.txt len=0" \
		--expect "fs: long file name PASS /mnt/fat/a long file name.txt (LFN entry)" \
		--expect "fs: nested directory PASS /mnt/fat/dir/deep/marker.txt" \
		--expect "fs: route /mnt/note.txt fs=ramfs-scratch /mnt/fat/hello.txt fs=fat32" \
		--expect "fs: disk negative cases PASS (ENOENT/ENOTDIR/EISDIR/EROFS)" \
		--expect "fs: disk selftest PASS"

# ---- CSS-1 存储识别验收（docs/storage.md 第 9 节）------------------------------------------------
#
# 夹具分三组，**可独立复核程度不同**（tools/README.md 的工具表）：
#   - FAT 载荷（`mkfs.py` 自己写）→ mtools（`mdir -i img@@offset`）；
#   - GPT/MBR 分区表（`mkfs.py` 自己写）→ `partx -s`（第三方实现，实测能枚举 GPT）；
#   - ext4 载荷（宿主 `mkfs.ext4` 生成）→ `dumpe2fs`。
#
# 脏变体（`--corrupt`）在**宿主侧**由 `tests/test_storage.py` 用另一套解析器断言「它真的是坏的」，
# 引导到内核时要断言的则是**内核看到的结果** —— 两者不是一回事：实测 OVMF 会在设备扫描阶段
# **自愈 GPT**（主表头坏、备表头好时把备份写回主位置），所以带坏主表头的盘进内核时已经好了
#（`docs/storage.md` 12.9 记了这条兼容性事实）。“用备表头”这条内核路径因此由内存盘自测覆盖
#（`storage: parse selftest`，不经过固件）。
STORAGE        := $(BUILD)/storage
STORAGE_FIXTURES := $(STORAGE)/ext4-root.img $(STORAGE)/gpt.img $(STORAGE)/gpt-degraded.img \
	$(STORAGE)/gpt-primary-bad.img $(STORAGE)/gpt-invalid.img $(STORAGE)/mbr.img $(STORAGE)/mbr-broken.img

storage-images:
	@mkdir -p $(STORAGE)
	@command -v mkfs.ext4 >/dev/null 2>&1 || { \
		echo "storage-images: 需要 mkfs.ext4（e2fsprogs）生成 ext4 载荷"; exit 1; }
	mkfs.ext4 -q -F -b 1024 -L cerlesse-ext4-long -U 11111111-2222-3333-4444-555555555555 \
		$(STORAGE)/ext4-root.img 8M
	$(PYTHON) tools/mkfs/mkfs.py --out $(STORAGE)/gpt.img --table gpt --from tests/fatroot/data \
		--part efi-esp:8M:fat16:efi --part root:8M:ext4:linux --part probe-exfat:4M:exfat:ms-data \
		--part probe-iso:1M:iso9660:unknown --part gpt-blank:1M \
		--part clone-a:2M:fat12:ms-data:serial=0x20261234 \
		--part clone-b:2M:fat12:ms-data:serial=0x20261234 \
		--payload root=$(STORAGE)/ext4-root.img
	$(PYTHON) tools/mkfs/mkfs.py --out $(STORAGE)/gpt-degraded.img --table gpt \
		--part dg-esp:4M:fat16:efi --part dg-exfat:1M:exfat:ms-data --corrupt gpt-entries-crc
	$(PYTHON) tools/mkfs/mkfs.py --out $(STORAGE)/gpt-primary-bad.img --table gpt \
		--part bk-esp:4M:fat16:efi --part bk-exfat:1M:exfat:ms-data --corrupt gpt-primary-crc
	$(PYTHON) tools/mkfs/mkfs.py --out $(STORAGE)/gpt-invalid.img --table gpt \
		--part iv-esp:4M:fat16:efi --part iv-exfat:1M:exfat:ms-data --corrupt gpt-both-headers
	$(PYTHON) tools/mkfs/mkfs.py --out $(STORAGE)/mbr.img --table mbr --from tests/fatroot/data \
		--part efi-esp:4M:fat12:fat12 --part ext:auto:extended \
		--logical probe-ntfs:2M:ntfs --logical mbr-blank:1M
	$(PYTHON) tools/mkfs/mkfs.py --out $(STORAGE)/mbr-broken.img --table mbr \
		--part ext:auto:extended --logical a1:1M:ntfs --logical a2:1M:exfat --corrupt ebr-broken

# CSS-1 宿主侧夹具门禁（不启 QEMU）：用**第二套解析器**（zlib + 直读字节）断言每个夹具
# 就是它声称的东西，并与 `partx`（GPT/MBR）、`mtools`（FAT 载荷）、`dumpe2fs`（ext4）互查。
test-storage-fixtures: storage-images
	$(PYTHON) tests/test_storage.py

# CSS-1 验收（GPT 系）：卷表的身份/健康/冲突 + 每个分区的文件系统探测。
#
#   - `disk index=1 ... sectors=57344 table=GPT partitions=7`：GPT 表被完整解出来；
#   - `disk index=2 ... health=degraded` + `gpt entries CRC mismatch`：表项 CRC 不过仍然解析，
#     但降级（**原因**也要打出来，不只是一个 degraded）；
#   - `disk index=4 ... table=invalid partitions=1 health=degraded`：主/备表头都坏 → 整盘一卷，
#     **不**回退去把保护表当 MBR 解析；
#   - `vol id=fs:66602620 ... fs=fat16 label=EFI-ESP ... health=ok`：`vol_id` 用 FAT 序列号（可复现），
#     卷标来自 BPB 并被规范化为 `[A-Za-z0-9._-]`；
#   - `fs=111111112222`（ext4 UUID 前 6 字节）/ `label=cerlesse-ex`（16 字符卷标被截到 11 位）：
#     非 FAT 卷被认出来但没有后端 → `health=unsupported`（不是 `ok`，也不是漏掉）；
#   - `pt:1-4-43008`：没有 UUID 的分区用「磁盘-分区-起始 LBA」当身份；
#   - `vol conflict id=fs:34122620 at=1:6,1:7 (all marked degraded)`：克隆卷必须**报出来**，
#     且冲突之后才打卷证据行（否则会先打一行自相矛盾的 health=ok）；
#   - **CSS-3**：`letter=` 字段（盘符）—— 系统卷（启动盘）拿 `C`，其余按 6.5 的枚举顺序
#     拿 `D`/`E`/…；**克隆卷的第二个卷不带字母**（同一 `vol_id` 不能有两个入口）；
#     `storage: letter vol=raw:4 letter=Z custom=1` + `ckm: store put key=storage/letters`：
#     自定义字母进了 CKM 状态区（跨重启不变，见 `make test-ckm-store`）。
test-storage: image storage-images
	$(PYTHON) tests/test_boot.py \
		--disk $(BUILD)/disk.img \
		--disk2 $(STORAGE)/gpt.img \
		--extra-disk $(STORAGE)/gpt-degraded.img \
		--extra-disk $(STORAGE)/gpt-primary-bad.img \
		--extra-disk $(STORAGE)/gpt-invalid.img \
		--serial-log $(BUILD)/serial-storage.log \
		--expect "storage: init MAX_VOLUMES=16 MAX_MOUNTS_PER_VOL=2 max_devices=8" \
		--expect "storage: disk index=0 dev=virtio-blk0 sectors=131072 table=none partitions=1" \
		--expect "storage: disk index=1 dev=virtio-blk1 sectors=57344 table=GPT partitions=7" \
		--expect "storage: part disk=1 index=1 table=GPT start_lba=2048 sectors=16384 type=efi" \
		--expect "storage: part disk=1 index=2 table=GPT start_lba=18432 sectors=16384 type=linux-fs" \
		--expect "storage: part disk=1 index=4 table=GPT start_lba=43008 sectors=2048 type=unknown" \
		--expect "storage: part disk=1 index=5 table=GPT start_lba=45056 sectors=2048 type=unknown" \
		--expect "storage: part disk=1 index=7 table=GPT start_lba=51200 sectors=4096 type=ms-data" \
		--expect "storage: gpt entries CRC mismatch on disk=2" \
		--expect "storage: disk index=2 dev=virtio-blk2 sectors=14336 table=GPT partitions=2 health=degraded" \
		--expect "storage: disk index=3 dev=virtio-blk3 sectors=14336 table=GPT partitions=2" \
		--expect "storage: disk index=4 dev=virtio-blk4 sectors=14336 table=invalid partitions=1 health=degraded" \
		--expect "storage: vol conflict id=fs:34122620 at=1:6,1:7 (all marked degraded)" \
		--expect "storage: letters table n=12 present=12 first=C" \
		--expect "storage: vol id=fs:66602620 disk=1 part=1 fs=fat16 label=EFI-ESP letter=D mounts=0 ro=ro-backend health=ok" \
		--expect "storage: vol id=fs:111111112222 disk=1 part=2 fs=ext4 label=cerlesse-ex letter=E mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=fs:bc022620 disk=1 part=3 fs=exfat label=- letter=F mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=pt:1-4-43008 disk=1 part=4 fs=iso9660 label=PROBE-ISO letter=G mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=pt:1-5-45056 disk=1 part=5 fs=unsupported label=- letter=H mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=fs:34122620 disk=1 part=6 fs=fat12 label=CLONE-A letter=I mounts=0 ro=ro-backend health=degraded" \
		--expect "storage: vol id=fs:34122620 disk=1 part=7 fs=fat12 label=CLONE-B letter=- mounts=0 ro=ro-backend health=degraded" \
		--expect "storage: vol id=fs:93622620 disk=2 part=1 fs=fat16 label=DG-ESP letter=J mounts=0 ro=ro-backend health=degraded" \
		--expect "storage: vol id=fs:b43d2620 disk=2 part=2 fs=exfat label=- letter=K mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=fs:8f6c2620 disk=3 part=1 fs=fat16 label=BK-ESP letter=L mounts=0 ro=ro-backend health=ok" \
		--expect "storage: vol id=fs:475e2620 disk=3 part=2 fs=exfat label=- letter=M mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=raw:4 disk=4 part=0 fs=unsupported label=- letter=N mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: parse selftest PASS (cases=11 checks=45)" \
		--expect "storage: selftest PASS (checks=137)" \
		--expect "storage: letters load n=0 source=none" \
		--expect "ckm: store put key=storage/letters len=7 seq=4 slot=B" \
		--expect "storage: letters saved custom=1" \
		--expect "storage: letter vol=raw:4 letter=Z custom=1" \
		--expect "storage: letters persisted probe=Z vol=raw:4" \
		--expect "storage: letters conflict rejected letter=D" \
		--expect-re "storage: letters cleared probe=Y vol=(fs:[0-9a-f]+) fallback=C" \
		--expect "storage: letters selftest PASS (checks=36)" \
		--expect "fs: storage mount selftest begin" \
		--expect "fs: alias point=/volumes/NO_NAME offset=4096 bytes=64 (same bytes as /disk/KERNEL.ELF)" \
		--expect "fs: after unmount path=/volumes/NO_NAME/KERNEL.ELF errno=2" \
		--expect "fs: mount negative cases PASS (ENOENT/EINVAL/EEXIST)" \
		--expect "fs: storage mount selftest PASS" \
		--expect-re "storage: mount point=/disk vol=(fs:[0-9a-f]+) ro=ro-backend.*?storage: volcheck id=\1 backend=fat16 caps=ro-backend case=insensitive label=NO_NAME bytes=66060288 record=ok.*?storage: mount point=/volumes/NO_NAME vol=\1 ro=ro-backend.*?fs: route path=/volumes/NO_NAME/KERNEL.ELF point=/volumes/NO_NAME vol=\1 fs=fat16 ro=ro-backend.*?fs: stat path=/volumes/NO_NAME/KERNEL.ELF kind=file size=[0-9]+ vol=\1.*?storage: unmount point=/volumes/NO_NAME vol=\1 mounts=1.*?fs: after unmount" \
		--expect-re "fs: route path=/volumes/NO_NAME/KERNEL.ELF point=/volumes/NO_NAME vol=(fs:[0-9a-f]+) fs=fat16 ro=ro-backend.*?fs: stat path=/volumes/NO_NAME/KERNEL.ELF kind=file size=[0-9]+ vol=\1"

# CSS-1 验收（MBR 系）：4 个主表项 + 扩展链（EBR）里的逻辑分区 5/6，以及断链的处理。
#
#   - `index=5 / index=6`：逻辑分区号从 5 起算（主分区 1..4），起始 LBA 是**绝对**的；
#   - `type=fat` / `type=ntfs/exfat` / `type=linux-fs`：MBR 类型字节 → 短名；
#   - `table=MBR partitions=2 health=degraded`（坏链夹具）：下一层 EBR 指到盘外时**停下来并降级**，
#     不越界读、不转圈（超深链的上限由 parse selftest 覆盖 —— 那种盘固件自己就不肯引导）；
#   - `pt:1-6-14338`：扩展分区里的空白逻辑分区按位置拿身份。
test-storage-mbr: image storage-images
	$(PYTHON) tests/test_boot.py \
		--disk $(BUILD)/disk.img \
		--disk2 $(STORAGE)/mbr.img \
		--extra-disk $(STORAGE)/mbr-broken.img \
		--serial-log $(BUILD)/serial-storage-mbr.log \
		--expect "storage: disk index=1 dev=virtio-blk1 sectors=20480 table=MBR partitions=3" \
		--expect "storage: part disk=1 index=1 table=MBR start_lba=2048 sectors=8192 type=fat" \
		--expect "storage: part disk=1 index=5 table=MBR start_lba=10241 sectors=4096 type=ntfs/exfat" \
		--expect "storage: part disk=1 index=6 table=MBR start_lba=14338 sectors=2048 type=linux-fs" \
		--expect "storage: disk index=2 dev=virtio-blk2 sectors=10240 table=MBR partitions=2 health=degraded" \
		--expect "storage: part disk=2 index=5 table=MBR start_lba=2049 sectors=2048 type=ntfs/exfat" \
		--expect "storage: vol id=fs:66602620 disk=1 part=1 fs=fat12 label=EFI-ESP letter=D mounts=0 ro=ro-backend health=ok" \
		--expect "storage: vol id=fs:21302620 disk=1 part=5 fs=ntfs label=- letter=E mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=pt:1-6-14338 disk=1 part=6 fs=unsupported label=- letter=F mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=fs:23482620 disk=2 part=5 fs=ntfs label=- letter=G mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: vol id=fs:99192620 disk=2 part=6 fs=exfat label=- letter=H mounts=0 ro=ro-backend health=unsupported" \
		--expect "storage: parse selftest PASS (cases=11 checks=45)" \
		--expect "storage: selftest PASS (checks=88)" \
		--expect "storage: letters selftest PASS (checks=22)" \
		--expect "fs: storage mount selftest PASS" \
		--expect-re "storage: mount point=/disk vol=(fs:[0-9a-f]+) ro=ro-backend.*?storage: unmount point=/volumes/NO_NAME vol=\1 mounts=1"

# CSS-3 拒绝面：uid 2000 的 `/bin/hello`（零能力）请求存储受控动作 → 三条都是
# `-1` + `need=0x10`（判定点 8 的“三件套”：返回值 + 审计 + 串口证据）。
# 与它成对的是 `/bin/volumes demo`（Shell，uid 1000）三次**成功**（`make test-programs`）——
# 同一组 syscall、同一个卷，唯一差别是主体。
#
# v0.5 验收：IPC（pipe 骨架）创建/写入/读取串口摘要日志 + 自测，随 Round Robin 巡回后执行
test-ipc: image
	$(PYTHON) tests/test_boot.py \
		--serial-log $(BUILD)/serial-ipc.log \
		--expect "Kernel started!" \
		--expect "sched: round-robin wrap PASS" \
		--expect "pipe: create id=1 cap=64" \
		--expect "pipe: write id=1 len=10" \
		--expect "pipe: read id=1 len=10" \
		--expect "pipe: selftest PASS"

# v0.4 验收：PIT 心跳递增 + PS/2 键盘 IRQ1 扫描码回显（经 QEMU monitor 注入真实按键）
# v0.9 起回显协议改了（字符原样回显，见 driver/keyboard.rs 模块文档的表），
# 因此这里断言的是「IRQ 真的进了、进了行缓冲、行真的组好了、PIT 还在走」，
# 而不是旧的 `KB <字符>` 那一行。
#
# 按键顺序是刻意选的：`a b Backspace c Enter` → 交付行必须是 **3 字节**（`ac` + `\n`）。
# 若退格没生效就是 `abc\n` = 4 字节 —— 于是 `len=3` 同时证明了「行缓冲收到了字符」
# 与「行编辑真的删掉了一个字符」，而不只是「有个标记出现了」。
test-keyboard: image
	$(PYTHON) tests/test_keyboard.py \
		--disk $(BUILD)/disk.img \
		--serial-log $(BUILD)/serial-kb.log \
		--key a \
		--key b \
		--key backspace \
		--key c \
		--key ret \
		--expect "KBIRQ enabled" \
		--expect "KBIRQ ENTRY" \
		--expect "kbd: line ready len=3" \
		--expect "KBIRQ EXIT" \
		--expect "IRQ0_heartbeat tick="

# v0.9 验收（交互式 Shell 第一步）：键盘行缓冲 → stdin(0) → 解析 → 内建命令。
#
# 硬断言（不是「日志里有 Shell 字样」）：
#   - `user: program=/bin/sh source=elf`：Shell 与 /init 一样是**从 VFS 装载的 ELF**；
#   - `user: read fd=0 want=128 got=5`：注入的 `help` + Enter 真的经键盘进了 stdin（5 = 4 + 行尾 \n）；
#   - `user: write fd=1 len=17` + `root@cerlesse C:>`：提示符按默认加载样式输出（17 字节），一次 write；
#   - `user: open path=/etc/motd ...` + `user: write fd=1 len=23`：`cat` 走真实文件 syscall；
#   - `syscall: ENOSYS nr=57 name=fork`：外部命令（`ls`）真的调了 fork，内核还没有进程模型；
#   - `sh: bye` + `user: shell PASS`：`exit` 正常结束。
test-shell: image
	$(PYTHON) tests/test_keyboard.py \
		--disk $(BUILD)/disk.img \
		--serial-log $(BUILD)/serial-shell.log \
		--ready-marker "user: program=/bin/sh" \
		--key h --key e --key l --key p --key ret \
		--key c --key a --key t --key spc --key slash \
		--key e --key t --key c --key slash --key m --key o --key t --key d --key ret \
		--key e --key x --key i --key t --key ret \
		--expect "user: program=/bin/sh source=elf" \
		--expect "user: read fd=0 want=128 got=5" \
		--expect "root@cerlesse C:>" \
		--expect "user: write fd=1 len=17" \
		--expect "sh: builtins (v0.9 step 3):" \
		--expect "sh:   /bin/hello      - external program" \
		--expect "user: open path=/etc/motd flags=0x0 fd=3" \
		--expect "motd: hello from ramfs" \
		--expect "sh: cat /etc/motd bytes=23" \
		--expect "sh: bye" \
		--expect "user: shell PASS"

# v0.9 第三步验收：**进程模型**（fork / execve / wait4）—— 从 Shell 交互触发，
# 断言的是「内核真的把另一个程序跑起来了」，而不是日志措辞：
#   - `user: fork parent=3 -> child pid=N image_pages=7`：地址空间被复制（7 个映像页 = Shell 的映像）；
#   - `user: run child pid=N name=fork-child`：子进程在父进程 wait4 时被恢复（单 CPU 的延迟调度）；
#   - `user: exec pid=N path=/bin/hello ... segments=4 image_pages=6`：映像被换掉，**PID 不变**；
#     （页数 6 而不是 5：CKM-1 给 `/bin/hello` 加了「默认拒绝」的自证代码，映像因此长了一页）
#   - `hello-bin: argc=2` / `argv[0]=hello` / `argv[1]=world`：内核把 argv 写进了新进程的用户栈；
#   - `user: exec /bin/foo -> ENOENT` + `sh: foo: exit code=127`：
#     **找不到的程序**也是完整路径（fork 成功 → exec 失败 → 子进程 exit(127) → 父进程拿到退出码）；
#     （v0.9 第五步起不能再用 `ls` 当负例 —— `/bin/ls` 已经存在，这里改用 `foo`）
#   - `user: frames free before=… after=…`：父进程 + 所有子进程的帧**全部归还**（没有地址空间泄漏）。
#
# 注意 PID 用**反向引用**断言而不是常数：syscall 手测里 `/bin/sh` 固定是 PID 3，但 fork
# 出的子进程号取决于此前消耗了多少个 PID（调度器自测会先取几个），写常数会让测试随
# 启动顺序变化而碎。`\1` 把 fork / execve / wait4 三处的 PID 绑定为同一个值。
test-proc: image
	$(PYTHON) tests/test_keyboard.py \
		--disk $(BUILD)/disk.img \
		--serial-log $(BUILD)/serial-proc.log \
		--ready-marker "user: program=/bin/sh" \
		--key h --key e --key l --key l --key o --key spc \
		--key w --key o --key r --key l --key d --key ret \
		--key f --key o --key o --key ret \
		--key e --key x --key i --key t --key ret \
		--expect "user: program=/bin/sh source=elf" \
		--expect "hello-bin: Hello from an external program!" \
		--expect "sh: exec /bin/foo failed errno=-2" \
		--expect "sh: hello: exit code=1" \
		--expect "hello-bin: ckm policy_query(fs_write,etc) -> -1 need=0x4" \
		--expect "hello-bin: ckm policy_query(audit_read) -> -1 need=0x8" \
		--expect "hello-bin: ckm policy_query(policy_query) -> 0" \
		--expect "hello-bin: ckm open /etc/hello-bin.flag -> -1 (EPERM expected)" \
		--expect "hello-bin: ckm open /tmp/hello-bin.flag -> -13 (EACCES expected)" \
		--expect "hello-bin: ckm open /etc/motd ro -> 3" \
		--expect "hello-bin: ckm read /etc/motd n=23" \
		--expect "hello-bin: ckm audit_read -> -1" \
		--expect "hello-bin: css storage_mount vol=fs:" \
		--expect "hello-bin: css storage_unmount -> -1 (EPERM expected)" \
		--expect "hello-bin: css storage_assign_letter Q -> -1 (EPERM expected)" \
		--expect "hello-bin: ckm errno EPERM=-1 EACCES=-13" \
		--expect "sh: foo: exit code=127" \
		--expect "sh: bye" \
		--expect "user: heap frame reclamation PASS" \
		--expect "user: selftest PASS" \
		--expect-re "user: fork parent=3 -> child pid=([0-9]+) image_pages=7.*?user: run child pid=\1 name=fork-child.*?sh: child pid=\1 exec /bin/hello \(argc=2\).*?user: exec pid=\1 path=/bin/hello segments=4 image_pages=6 entry=0x8000000000.*?hello-bin: pid=\1.*?user: reaped pid=\1 exit=1" \
		--expect-re "hello-bin: argc=2.*?hello-bin: argv\[0\]=hello.*?hello-bin: argv\[1\]=world" \
		--expect-re "user: run child pid=([0-9]+) name=fork-child.*?user: exec /bin/foo -> ENOENT \(no such program\).*?user: reaped pid=\1 exit=127" \
		--expect-re "user: frames free before=([0-9]+) after=\1"

# v0.9 第五步验收（CSS-1 追加 `/bin/volumes`）：**基础用户程序**
# （`/bin/ls`、`/bin/ps`、`/bin/cat`、`/bin/echo`、`/bin/volumes`）——
# 全部从 Shell 交互触发，走 `fork` + `execve` + `wait4`，断言的是程序**自己产出的内容**
# 与「它们各自用到的内核能力真的通了」：
#   - `user: getdents path=/ entries=6 returned=6` + `ls: / entries=6`：
#     `ls` 读的是 **VFS 的目录项**（v0.7 起只有内核自测用过 `list()`）；
#     根目录是 6 项而不是 5：CKM-2 起内核多建了一个 `/ckm`（覆盖区 `/ckm/overlay`
#     的家，`docs/ckm.md` 6.1）；`fs: ls / entries=5` 那条是 `fs::selftest`
#     的窗口（在 `/ckm` 建立**之前**），两者不矛盾，分别在断自己的时点；
#   - `ls: /bin entries=9`：`/bin` 里恰好是这次构建写进去的 9 个程序
#     （sh / hello / ls / cat / echo / ps / volumes / dynhello / tcpecho）—— 同时证明 `fs::init` 的清单是完整的；
#   - `user: storage_volumes count=1 total=1` + `volumes: fs:… disk=0 part=0 fs=fat16 …`：
#     `volumes` 读的是**内核卷表**（CSS-1 新增的 `storage_volumes`，号 261）；单盘启动
#     恰好 1 个卷（启动盘的 ESP），且 libc 按 64 字节记录解析出的字段与内核一致 ——
#     这条断言的作用是「记录布局这个 ABI 真的被消费者读对了」，而不只是内核自验；
#   - `user: getprocs count=2` + `ps: pid=3 ppid=0 state=RUNNING comm=sh`：
#     `ps` 读的是**内核进程表**（本轮新增的 `getprocs`）：表里恰好是 Shell 与 ps 自己，
#     且 `ppid` 指回 Shell —— 这正是 `fork` 建出来的父子关系；
#   - `user: exec pid=N path=/bin/{ls,ps,echo,cat}`：四个程序都是真的被 `execve` 起来的；
#   - **CSS-3**：`volumes` 多了一列 `letter=`（记录偏移 24，系统卷 = `C`）；
#     `volumes demo` 把 `C:` 那个卷挂到 `/volumes/<卷标>`（`storage_mount`，受控动作，
#     Shell 是 uid 1000 → 允许），然后用**两个入口**读同一段字节
#     （`C:\KERNEL.ELF` 由 libc 重写成 POSIX 路径，`docs/storage.md` 4.2），
#     再卸载并断言路径真的消失 —— 「Windows 式路径不是内核语法」这条因此有实测证据；
#     与之成对的是 `hello-bin: css storage_mount … -> -1`（uid 2000，零能力，见 test-ckm）；
#   - `cat: /etc/motd bytes=23`：`/bin/cat` 走 open/read/write/close 读到内核提供的文件；
#   - `echo program works` + `sh: /bin/echo: exit code=0`：`argv` 经内核写入新进程用户栈，
#     程序原样打印（`/bin/echo` 是程序，裸 `echo` 是内建命令 —— 见 user/shell 的帮助）。
#
# 注意 `cat`/`echo` 用的是**绝对路径**：内建命令会遮蔽裸命令名（与真实 shell 一致），
# 因此要跑程序本身必须写 `/bin/<name>`（而 `ls`/`ps` 没有内建版本，裸名字就是程序）。
test-programs: image
	$(PYTHON) tests/test_keyboard.py \
		--disk $(BUILD)/disk.img \
		--serial-log $(BUILD)/serial-programs.log \
		--ready-marker "user: program=/bin/sh" \
		--key l --key s --key ret \
		--key l --key s --key spc --key slash --key b --key i --key n --key ret \
		--key p --key s --key ret \
		--key v --key o --key l --key u --key m --key e --key s --key ret \
		--key v --key o --key l --key u --key m --key e --key s --key spc \
		--key d --key e --key m --key o --key ret \
		--key slash --key b --key i --key n --key slash --key e --key c --key h --key o \
		--key spc --key e --key c --key h --key o --key spc --key p --key r --key o \
		--key g --key r --key a --key m --key spc --key w --key o --key r --key k --key s \
		--key ret \
		--key slash --key b --key i --key n --key slash --key c --key a --key t \
		--key spc --key slash --key e --key t --key c --key slash --key m --key o \
		--key t --key d --key ret \
		--expect "user: program=/bin/sh source=elf" \
		--expect "user: getdents path=/ entries=6 returned=6" \
		--expect "ls: / entries=6" \
		--expect "ls: entry name=init kind=file size=" \
		--expect "ls: entry name=bin kind=dir size=0" \
		--expect "ls: entry name=ckm kind=dir size=0" \
		--expect "ls: /bin entries=9" \
		--expect "ls: entry name=tcpecho kind=file size=" \
		--expect "ls: entry name=ls kind=file size=" \
		--expect "ls: entry name=volumes kind=file size=" \
		--expect "user: getprocs count=2" \
		--expect "ps: pid ppid state comm" \
		--expect "ps: pid=3 ppid=0 state=RUNNING comm=sh" \
		--expect "ps: 2 process(es)" \
		--expect "user: storage_volumes count=1 total=1" \
		--expect "volumes: id disk part fs label label_trunc letter ro health bytes free_known free mounts" \
		--expect "disk=0 part=0 fs=fat16 label=NO_NAME label_trunc=0 letter=C ro=ro-backend health=ok bytes=66060288 free_known=0 free=0 mounts=1" \
		--expect "volumes: 1 volume(s) (total=1)" \
		--expect "user: storage_mount vol=fs:" \
		--expect "volumes: demo storage_mount -> 0" \
		--expect "volumes: demo win=C:\\KERNEL.ELF linux=/volumes/NO_NAME/KERNEL.ELF" \
		--expect "volumes: demo read win_bytes=16 linux_bytes=16 equal=1 first=0x7f454c46" \
		--expect "volumes: demo storage_unmount -> 0" \
		--expect "volumes: demo after unmount open -> -2" \
		--expect-re "ckm: policy op=storage_mount pid=([0-9]+) uid=1000 result=allow need=0x0 detail=0x0" \
		--expect "ckm: policy op=storage_unmount pid=" \
		--expect "storage: unmount point=/volumes/NO_NAME vol=fs:" \
		--expect "cat: /etc/motd bytes=23" \
		--expect "echo program works" \
		--expect "sh: ls: exit code=0" \
		--expect "sh: ps: exit code=0" \
		--expect "sh: volumes: exit code=0" \
		--expect "sh: /bin/echo: exit code=0" \
		--expect "sh: /bin/cat: exit code=0" \
		--expect-re "user: exec pid=([0-9]+) path=/bin/ls segments=[0-9]+ image_pages=[0-9]+ entry=0x8000000000" \
		--expect-re "user: exec pid=([0-9]+) path=/bin/ps segments=[0-9]+ image_pages=[0-9]+ entry=0x8000000000" \
		--expect-re "user: exec pid=([0-9]+) path=/bin/echo segments=[0-9]+ image_pages=[0-9]+ entry=0x8000000000.*?echo program works" \
		--expect-re "user: exec pid=([0-9]+) path=/bin/cat segments=[0-9]+ image_pages=[0-9]+ entry=0x8000000000.*?motd: hello from ramfs.*?cat: /etc/motd bytes=23" \
		--expect-re "user: getprocs count=2.*?ps: pid=([0-9]+) ppid=3 state=RUNNING comm=ps" \
		--expect-re "user: exec pid=([0-9]+) path=/bin/volumes segments=[0-9]+ image_pages=[0-9]+ entry=0x8000000000.*?volumes: fs:[0-9a-f]+ disk=0 part=0"

# v0.9 动态链接验收：**内核侧最小动态链接器**（`kernel/src/syscall/dynamic.rs`）。
#
# 夹具（`user/dynhello/*.s`，由 `kernel/build.rs` 用 as/ld 现编）里放了三样东西，
# 断言逐条对应它们真的发生了：
#   - `dyn: PT_INTERP=/lib/ld.so`：内核从主程序的 PT_INTERP 里读出了解释器路径；
#   - `dyn: load <main> base=0x8000000000`：ET_DYN（PIE）被装载在映像窗口（同静态程序基址）；
#   - `dyn: NEEDED libgreet.so -> /lib/libgreet.so`：DT_NEEDED 被解析成 VFS 路径；
#   - `dyn: load /lib/libgreet.so base=0x8000103000`：共享库被装进了**共享库窗口**；
#   - `dyn: reloc R_X86_64_JUMP_SLOT sym=greet -> obj=/lib/libgreet.so`：跨对象符号被解析，
#     GOT 槽被填成共享库里的地址（另外一条 R_X86_64_RELATIVE 也在 relocs=2 里）；
#   - `main: dynamic link ok` + `greet: hello from libgreet.so`：**ring3 里两条路径都真跑了**
#     （主程序自己的 .rodata 与共享库的 .rodata 各发一次 write）；
#   - `user: exit code=42`：退出码 = 共享库 `greet()` 的返回值；
#   - `user: dynamic link PASS` + `frames free before=… after=…`：地址空间（含共享库窗口）无泄漏。
test-dyn: image
	$(PYTHON) tests/test_boot.py \
		--serial-log $(BUILD)/serial-dyn.log \
		--expect "dyn: PT_INTERP=/lib/ld.so" \
		--expect "dyn: load <main> base=0x8000000000" \
		--expect "dyn: NEEDED libgreet.so -> /lib/libgreet.so" \
		--expect "dyn: load /lib/libgreet.so base=" \
		--expect "dyn: reloc R_X86_64_JUMP_SLOT sym=greet -> obj=/lib/libgreet.so" \
		--expect "dyn: link PASS objects=3 relocs=2 entry=0x8000001020" \
		--expect "main: dynamic link ok" \
		--expect "greet: hello from libgreet.so" \
		--expect "user: exit code=42" \
		--expect "user: dynamic link PASS"

# v0.9 动态链接的**独立复核**（不是自己验自己）已随夹具搬到用户态 workspace：
# 同一份 `userland/user/dynhello/*.s` 交给宿主机 glibc 的 ld.so 重新链接并执行。
# 目标名保留（旧文档/习惯仍然有效）；内核仓库本身不再需要 binutils。
dyn-hostcheck:
	$(MAKE) -C $(USERLAND) dyn-hostcheck

# 用户态（独立 workspace，见 userland/）：构建程序 + 动态链接夹具，产出
# $(USERLAND_IMAGES)/manifest.txt。**不需要内核源码**，也不启动 QEMU；
# 段布局校验（userland/tools/check_user_elf.py）在那边对每个程序跑 —— 用户态的构建
# 因此**不需要内核仓库的任何工具**（阶段五：两侧分别构建）。
userland-images:
	@if [ -d $(USERLAND) ]; then \
		$(MAKE) -C $(USERLAND) images; \
	else \
		echo "userland-images: 没有 $(USERLAND)/ 目录（纯内核 checkout）—— 跳过（镜像将不含用户态）"; \
	fi

# 兼容 v0.9 的旧名字（当时 `make user` 在内核 workspace 里构建用户程序）。
user: userland-images

# H-06 包管理器验收（**主机侧、不启 QEMU、纯标准库**）—— **已随阶段五迁到用户态**。
#
# `ckc.py`（P0a 容器与打包规则）与 `cpm.py`（P0b = CKM-3 事务引擎）连同两份套件
# 现在住在 `userland/pkgmgr/`（`make -C userland test-pkg`）：安装 / 卸载 / 依赖 / 事务
# 属于用户态管理服务，内核只保留 `.ckc` 的**格式规范**（docs/package-manager.md）
# 与安装期的重新校验（`cpm` 用 `ckc.manifest_from_data`，内核不认识包）。
#
# 因此这里是**交叉仓库转发**：`CERLESSE_USERLAND_DIR` 指向的目录不存在时**明确报错**，
# 而不是静默跳过 —— 与 `image-userland` / `test-all-with-userland` 同一约定。
CERLESSE_USERLAND_DIR ?= $(USERLAND)

test-pkg:
	@if [ -d "$(CERLESSE_USERLAND_DIR)" ]; then \
		$(MAKE) -C $(CERLESSE_USERLAND_DIR) test-pkg; \
	else \
		echo "test-pkg: 找不到用户态目录 '$(CERLESSE_USERLAND_DIR)'。"; \
		echo "          包管理器（.ckc + cpm）自阶段五起住在 userland/pkgmgr/。"; \
		echo "          请设置 CERLESSE_USERLAND_DIR=<path>，或改用内核侧的 make test-all。"; \
		exit 1; \
	fi

# 边界门禁（docs/pure-kernel.md 阶段二）：内核不得依赖用户态 ——
# 不得嵌套调用 cargo/as/ld、不得内嵌用户态字节、不得依赖用户态包。
check-boundary:
	$(PYTHON) tools/check-boundary.py
	$(PYTHON) tools/check-boundary.py --selftest

# 阶段三（ABI 单一事实源，docs/pure-kernel.md）：`abi` crate 的宿主自测 ——
# 号唯一/号段不重叠/记录尺寸自洽/CKM 与存储字面量逐条固化。
# 另有编译期断言（abi/src/record.rs）：任何一侧把数字改坏，**构建**就失败。
abi-check:
	$(CARGO) test -p abi

# 主机侧门禁：ABI 单一事实源 + 边界 + Go 单元测试
check: abi-check check-boundary go-test

# v0.4 H-04/H-05 主机侧：Go 单元测试（串口分析 + 并行编排器）
go-test:
	cd tools-go && $(GO) vet ./... && $(GO) test ./...

# v0.4 H-04 验收：Go 编排器并行跑内核组 13 个集成测试场景（`-exclude-tag userland`）+ 串口日志二次复核。
# 磁盘镜像先构建（默认/异常注入/页错误 + FAT 测试盘），随后场景并行执行；
# 每个场景独占 OVMF_VARS 副本、串口日志与 monitor socket（见 tests/orchestrate.json）。
# 内核场景并行编排：`-exclude-tag userland` 把断言 `/init`、`/bin/*`、`libc` 输出的
# 场景（user/ckm/shell/proc/programs/dyn）挡在外面，因此在纯内核 checkout 里也能全绿。
# 标签定义在 `tests/orchestrate.json` 的场景 `tags` 里（词表见 tools-go 的 KnownTags）。
test-orch: image image-exception image-pagefault fat-image storage-images
	cd tools-go && $(GO) run ./cmd/testorch -config ../tests/orchestrate.json -root .. -exclude-tag userland

# 交叉仓库：用户态集成场景（`user`/`ckm`/`shell`/`proc`/`programs`/`dyn`）并行编排。
# 它需要完整镜像（内嵌用户态）与内核侧的 QEMU 驱动，因此留在内核仓库里跑，
# 但**不属于内核 `test-all`**（阶段五要的就是这条边界）。
test-orch-userland: image image-exception image-pagefault fat-image storage-images
	cd tools-go && $(GO) run ./cmd/testorch -config ../tests/orchestrate.json -root .. -tag userland

clean:
	rm -rf $(BUILD)
