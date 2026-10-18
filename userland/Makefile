# Cerlesse Userland —— 构建入口（见 docs/pure-kernel.md 阶段四）
#
# 本 Makefile 属于**用户态 workspace**，与仓库根的 Makefile（内核）互不依赖：
#   - 输入：本目录下的 libc/ 与 user/*（Rust + 汇编夹具）；
#   - 输出：$(IMAGES) —— 程序 ELF + 动态链接夹具 + manifest.txt；
#   - 内核侧只按 manifest.txt 消费这些产物（`make userland-images` → 内核 `make image`）。
#
# 独立可验证的两条：
#   - `make images` 不需要内核源码，也不启动 QEMU；
#   - `make dyn-hostcheck` 用**宿主机 glibc 的 ld.so** 复核同一批夹具（不是自己验自己）。
#
# v0.9 之前这些工作由 `kernel/build.rs` 用嵌套 cargo + as/ld 在做，因此内核
# 「删掉 libc/、user/ 就编不出来」；现在内核构建脚本不再调用任何外部命令，
# 边界由 `python3 ../tools/check-boundary.py` 强制。

CARGO  ?= cargo
PYTHON ?= python3
AS     ?= as
LD     ?= ld

ROOT    := $(abspath ..)
BUILD   := $(ROOT)/build/userland
TARGET  := $(BUILD)/target
IMAGES  := $(BUILD)/images
TRIPLE  := x86_64-unknown-none
PROFILE := user-release
OUT     := $(TARGET)/$(TRIPLE)/$(PROFILE)

PROGRAMS := user-hello user-shell user-hello-bin user-ls user-cat user-echo user-ps user-volumes user-tcpecho
CARGO_PROGRAMS := $(addprefix -p ,$(PROGRAMS))

# manifest.txt 是「用户态 → 内核」的**唯一接口**：每行 `<P|L> <RAMFS 路径> <文件名>`，
#   P = 可运行程序（内核写进 RAMFS 后按路径 exec）；L = 被装载对象（解释器 / 共享库）。
MANIFEST := $(IMAGES)/manifest.txt

SOURCES := $(shell find libc user -name '*.rs' -o -name 'Cargo.toml' -o -name 'user.ld' -o -name '*.s')

.PHONY: help images check dyn dyn-hostcheck test-pkg test-ckc test-cpm test-all clean

help:
	@echo "targets:"
	@echo "  images        - 构建 $(words $(PROGRAMS)) 个用户程序 + 动态链接夹具，产出 images/manifest.txt"
	@echo "  check         - 校验每个 ELF 的段布局（基址/页对齐/W^X/映像窗口/无重定位）"
	@echo "  dyn           - 用 binutils（as/ld）现编动态链接夹具：dynhello + libgreet.so + ld.so"
	@echo "  dyn-hostcheck - 用宿主机 glibc 的 ld.so 跑同一批夹具（binutils + glibc；缺则跳过）"
	@echo "  test-pkg      - 包管理器验收（主机侧、不启 QEMU）：.ckc 容器（test-ckc）+ cpm 事务引擎（test-cpm）"
	@echo "  test-ckc      - .ckc 容器格式与打包规则（阶段五从内核仓库迁来）"
	@echo "  test-cpm      - cpm 事务/回执/回滚/恢复（P0b = CKM-3）"
	@echo "  test-all      - images + check + dyn-hostcheck + test-pkg（不需要内核源码的那部分）"
	@echo "  clean         - 删除 $(BUILD)"

images: $(MANIFEST)

$(MANIFEST): $(SOURCES) Makefile Cargo.toml
	@mkdir -p $(IMAGES)
	$(CARGO) build $(CARGO_PROGRAMS) --target $(TRIPLE) --profile $(PROFILE) \
		--offline --target-dir $(TARGET)
	@for p in $(PROGRAMS); do \
		$(PYTHON) $(CURDIR)/tools/check_user_elf.py $(OUT)/$$p || exit 1; \
	done
	@cp $(OUT)/user-hello     $(IMAGES)/user_hello.elf
	@cp $(OUT)/user-shell     $(IMAGES)/user_shell.elf
	@cp $(OUT)/user-hello-bin $(IMAGES)/user_hello_bin.elf
	@cp $(OUT)/user-ls        $(IMAGES)/user_ls.elf
	@cp $(OUT)/user-cat       $(IMAGES)/user_cat.elf
	@cp $(OUT)/user-echo      $(IMAGES)/user_echo.elf
	@cp $(OUT)/user-ps        $(IMAGES)/user_ps.elf
	@cp $(OUT)/user-volumes   $(IMAGES)/user_volumes.elf
	@cp $(OUT)/user-tcpecho   $(IMAGES)/user_tcpecho.elf
	@$(MAKE) --no-print-directory dyn
	@$(PYTHON) $(CURDIR)/tools/check_dyn_fixture.py $(IMAGES)/dynhello
	@printf '%s\n' \
		'P /init user_hello.elf' \
		'P /bin/sh user_shell.elf' \
		'P /bin/hello user_hello_bin.elf' \
		'P /bin/ls user_ls.elf' \
		'P /bin/cat user_cat.elf' \
		'P /bin/echo user_echo.elf' \
		'P /bin/ps user_ps.elf' \
		'P /bin/volumes user_volumes.elf' \
		'P /bin/tcpecho user_tcpecho.elf' \
		'P /bin/dynhello dynhello' \
		'L /lib/ld.so ld.so' \
		'L /lib/libgreet.so libgreet.so' > $(MANIFEST)
	@echo "userland: images ready ($(words $(PROGRAMS)) programs + 3 dyn objects) -> $(IMAGES)"

check: images
	@echo "userland: ELF 段布局检查 PASS（$(words $(PROGRAMS)) 个程序）"

# 动态链接夹具：裸目标 x86_64-unknown-none 不支持 cdylib，因此这一组由 as/ld 现编。
# `dynhello` 是 ET_DYN PIE（`--dynamic-linker=/lib/ld.so` + `-lgreet`）—— 路径写死在
# PT_INTERP 里，与内核 `syscall/dynamic.rs` 的查找规则一致（内核按名字去 VFS 找）。
dyn:
	@mkdir -p $(IMAGES)
	@command -v $(AS) >/dev/null 2>&1 && command -v $(LD) >/dev/null 2>&1 || { \
		echo "需要 binutils（as/ld）来汇编动态链接夹具：请安装 binutils 或设置 AS=/path/to/as LD=/path/to/ld"; exit 1; }
	$(AS) -o $(IMAGES)/libgreet.o user/dynhello/libgreet.s
	$(LD) -shared -soname libgreet.so -o $(IMAGES)/libgreet.so $(IMAGES)/libgreet.o
	$(AS) -o $(IMAGES)/ldso.o user/dynhello/ldso.s
	$(LD) -shared -o $(IMAGES)/ld.so $(IMAGES)/ldso.o
	$(AS) -o $(IMAGES)/dynhello.o user/dynhello/dynhello.s
	$(LD) -pie --dynamic-linker=/lib/ld.so -L$(IMAGES) -lgreet \
		-o $(IMAGES)/dynhello $(IMAGES)/dynhello.o

# 独立复核：同一份 .s 交给宿主机真实的动态链接器（glibc 的 ld.so）重新链接并执行。
# 两个完全不同的装载器（内核 vs glibc）都要跑出相同的两行输出与 exit(42)，
# 才说明 ELF 结构 / 机器码 / 重定位语义是对的。缺 binutils/glibc 时**跳过**（不当作失败）。
dyn-hostcheck: dyn
	@LD_SO=$$(ls /lib64/ld-linux-x86-64.so.2 /lib/ld-linux-x86-64.so.2 2>/dev/null | head -1); \
	test -n "$$LD_SO" || { echo "dyn-hostcheck SKIP (no glibc ld.so)"; exit 0; }; \
	echo "dyn-hostcheck: host ld.so = $$LD_SO"; \
	$(AS) -o $(IMAGES)/dynhello.o user/dynhello/dynhello.s && \
	$(LD) -pie --dynamic-linker=$$LD_SO -L$(IMAGES) -lgreet \
		-o $(BUILD)/dynhello-host $(IMAGES)/dynhello.o && \
	LD_LIBRARY_PATH=$(IMAGES) $(BUILD)/dynhello-host > $(BUILD)/dynhost-out.txt; \
	status=$$?; cat $(BUILD)/dynhost-out.txt; \
	grep -q "main: dynamic link ok" $(BUILD)/dynhost-out.txt && \
	grep -q "greet: hello from libgreet.so" $(BUILD)/dynhost-out.txt && \
	test $$status -eq 42 && echo "dyn-hostcheck PASS (glibc ld.so, exit=42)" || { echo "dyn-hostcheck FAIL (status=$$status)"; exit 1; }

# 包管理器（纯内核化阶段五从内核仓库 `tools/pkgmgr/` 迁来）：`ckc.py`（.ckc 容器）与
# `cpm.py`（事务引擎）连同两份套件住在 `pkgmgr/`。主机侧、纯标准库、不启 QEMU，
# 因此**不需要内核源码** —— 这是本 workspace 可独立测试的那一半。
#
# 需要内核镜像的那一半（`test-user`/`test-shell`/`test-proc`/`test-programs`/`test-dyn`）
# 仍由内核仓库的 Makefile 驱动：它们跑 QEMU，而 QEMU 驱动与镜像组装属于内核仓库。
test-pkg: test-ckc test-cpm

test-ckc:
	$(PYTHON) pkgmgr/tests/test_ckc.py

test-cpm:
	$(PYTHON) pkgmgr/tests/test_cpm.py

# 本 workspace 的「不需要对方源码」的测试集合。
test-all: images check dyn-hostcheck test-pkg

clean:
	rm -rf $(BUILD)
