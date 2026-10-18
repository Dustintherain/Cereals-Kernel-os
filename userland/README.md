# Cerlesse Userland

- 阶段：v0.9 之后（纯内核化改造，见 [docs/pure-kernel.md](../docs/pure-kernel.md) 阶段四）
- 最后更新：2026-10-12
- 状态：✅ 已拆分（独立 workspace + 独立构建入口；`make -C userland images`、`make -C userland test-all`（含 `test-pkg`）与 `make test-kernel-only` 实测通过）

> 本文件顶部保留 阶段 / 最后更新 / 状态 字段，与项目文档维护约定对齐。

Shell · init · libc · coreutils ·（未来的）包管理器 —— **不属于内核**。本目录是它们的
工作区：内核只提供机制（系统调用、进程、内存、VFS、驱动、网络），用户态在这里构建。

## 1. 边界：与内核的唯一接触面

```text
userland/（本 workspace）                      内核仓库（../Cargo.toml）
  libc/  user/{hello,shell,hello-bin,ls,cat,echo,ps}     shared/ boot/ kernel/
        │  make images
        ▼
  ../build/userland/images/
      user_*.elf  dynhello  ld.so  libgreet.so
      manifest.txt        ───────────────▶  kernel/build.rs 按清单复制 + 生成
                                            $OUT_DIR/userland_images.rs
                                            （include! 进 syscall/user.rs）
```

- **内核不认识任何具体程序**：RAMFS 上的路径（`/init`、`/bin/sh`、`/lib/ld.so` …）由
  `manifest.txt` 决定，内核只在 `syscall/user.rs` 里保留一份**契约常量**（清单必须
  覆盖这些路径，缺一条就明确报错，而不是给出难懂的 `ENOENT`）。
- **清单不存在时内核照样工作**：`kernel/build.rs` 生成空清单，内核启动后进入串口
  诊断模式（`user: no userland images embedded; entering diagnostic idle loop`），
  由 `make test-kernel-only` 断言。
- 内核侧不得反向引用本目录：`python3 ../tools/check-boundary.py`（`make check-boundary`）
  会检查内核构建脚本不调用外部命令、内核源码不内嵌用户态字节、内核不依赖用户态包。

`manifest.txt` 格式（每行一条，空格分隔）：

```text
P /init         user_hello.elf      # P = 可运行程序（写进 RAMFS，按路径 exec）
L /lib/ld.so    ld.so               # L = 被装载对象（解释器 / 共享库）
```

## 2. 布局

| 路径 | 内容 |
| ---- | ---- |
| `libc/` | 最小用户态运行时：syscall 封装、`_start`、`print!/println!`、`#[global_allocator]`、`args` |
| `user/hello/` | 第一个用户态进程（RAMFS `/init`）：文件 syscall / 用户堆 / `exit(42)` |
| `user/shell/` | 交互式 Shell（`/bin/sh`）：键盘行缓冲 → `stdin` → 内建命令 + `fork`/`execve`/`wait4` |
| `user/hello-bin/` | 外部程序（`/bin/hello`）：由 Shell `fork` + `execve` 起来，打印 PID 与 `argv` |
| `user/{ls,cat,echo,ps}/` | coreutils：`getdents` / 文件 syscall / `write` / `getprocs` |
| `user/volumes/` | 存储卷枚举（`/bin/volumes`）：`storage_volumes`（CSS-1），打印卷表（`id`/`disk`/`part`/`fs`/`label`/`health`/`mounts`） |
| `user/tcpecho/` | TCP 回显验收（`/bin/tcpecho`）：`connect` 连宿主回声服务，两轮往返分别走 `sendto`/`recvfrom` 与 `write`/`read`（v1.0 socket/TCP 补齐） |
| `user/dynhello/` | 动态链接夹具（`as`+`ld` 现编）：`dynhello` + `libgreet.so` + `ld.so` |
| `user/init/` | v0.9 占位（2 行）；真正的 `/init` 是 `user/hello` 的产物 |
| `tools/check_dyn_fixture.py` | 动态链接夹具结构自检（ET_DYN + `PT_INTERP` + `DT_NEEDED`） |
| `tools/check_user_elf.py` | 用户程序 ELF 段布局校验（基址/页对齐/`W^X`/映像窗口/无重定位）—— `make images` 逐个调用；**阶段五从内核仓库 `tools/` 迁入** |
| `pkgmgr/` | 包管理器主机工具：`ckc.py`（`.ckc` 容器，H-06 P0a）+ `cpm.py`（事务引擎 = CKM-3/P0b）+ `tests/`；**阶段五从内核仓库 `tools/pkgmgr/` 与 `tests/` 迁入**，见 [`pkgmgr/README.md`](pkgmgr/README.md) |

## 3. 构建

```bash
make -C userland images        # 构建 9 个程序（+3 个动态链接对象）+ manifest.txt
make -C userland check         # 同上，并逐个校验 ELF 段布局（基址/页对齐/W^X/窗口/无重定位）
make -C userland dyn           # 只重编动态链接夹具（需要 binutils：as/ld）
make -C userland dyn-hostcheck # 用宿主机 glibc 的 ld.so 复核同一批夹具（缺工具则跳过）
make -C userland test-pkg      # 包管理器验收（.ckc 容器 37 + cpm 事务 63，主机侧，不启 QEMU）
make -C userland test-all      # images + check + dyn-hostcheck + test-pkg（不需要内核源码的那部分）
make -C userland clean
```

需要：`cargo`（目标 `x86_64-unknown-none`）、`binutils`（`as`/`ld`，仅动态链接夹具）。
**不需要内核源码**，也不启动 QEMU。构建参数（`code-model=large`、`user-release` profile、
`user.ld`）都在本 workspace 内：见 [`.cargo/config.toml`](.cargo/config.toml) 与
[`Cargo.toml`](Cargo.toml)。

端到端（在仓库根）：

```bash
make image            # 先出用户态镜像，再编内核（按清单内嵌），最后组装磁盘镜像
make test-user        # 96 项：ELF 装载 / ring3 syscall / 用户堆 / mprotect / 隔离 / Shell
make test-shell test-proc test-programs test-dyn
make test-pkg         # 转发到 userland/pkgmgr（.ckc 容器 + cpm 事务引擎）
make test-orch-userland  # 只跑标记 userland 的编排场景
```

## 4. 内核侧对应的验收

| 目标 | 用户态侧证据 |
| ---- | ---- |
| `make test-user`（96 项） | `/init` 从 RAMFS 装载、`exit(42)`、用户堆、`mprotect`（`PROT_NONE`/`RX` 转换 + 真的执行机器码）、页帧回收、隔离探针 |
| `make test-shell`（11 项） | 键盘 → 行缓冲 → `/bin/sh` → 内建命令 |
| `make test-proc`（8 项 + 4 正则） | `fork`/`execve`/`wait4` + `argv`（`/bin/hello`、`ENOENT` → 127） |
| `make test-programs`（33 项 + 7 正则） | `/bin/{ls,cat,echo,ps,volumes}` 真的被 exec 起来（`/bin/tcpecho` 的目录项一并断言） |
| `make test-dyn`（10 项） | `PT_INTERP` + `libgreet.so` + `RELATIVE`/`JUMP_SLOT` |
| `make test-kernel-only`（7 项） | **反向**证据：没有用户态镜像时内核照样启动并进入诊断模式 |

## 5. 修订记录

| 日期 | 变更 |
| ---- | ---- |
| 2026-10-14 | **v1.0 Socket/TCP 补齐**：`libc` 新增 `socket`/`connect`/`bind`/`sendto`/`recvfrom` 封装 + `AF_INET`/`SOCK_STREAM`/`IPPROTO_TCP`/`SOCKADDR_IN_LEN` + `sockaddr_in` 构造器（`lib.rs` re-export）；新增 `user/tcpecho`（`/bin/tcpecho`，两轮回显分别走 `sendto`/`recvfrom` 与 `write`/`read`）；`make images` 构建 8 → **9 个程序**（manifest/清单同步） |
| 2026-10-12 | **阶段五补充**：`userland/pkgmgr/`（`ckc.py` + `cpm.py` + 两份套件 + `README.md`，从内核仓库 `tools/pkgmgr/` 与 `tests/` 迁入）与 `userland/tools/check_user_elf.py`（从 `tools/` 迁入）落位；新增 `make -C userland test-pkg`/`test-all` —— 用户态侧**不需要内核源码**即可构建与测试 |
| 2026-10-11 | 创建本 workspace：`libc/` 与 `user/` 从内核仓库根移入（保留 git 历史），独立 `Cargo.toml`（含 `user-release` profile）与 `.cargo/config.toml`（`code-model=large` 只对用户态生效）；构建入口 `Makefile`（`images`/`check`/`dyn`/`dyn-hostcheck`）产出 `build/userland/images/manifest.txt`；动态链接夹具与 `dyn-hostcheck` 从内核仓库搬来；新增 `tools/check_dyn_fixture.py` |
| 2026-10-11 | **v1.0 第一步：`mprotect`** —— `libc` 新增 `mprotect`/`PROT_NONE`（`lib.rs` re-export，`syscall.rs` 文档同步）；`user/hello` 新增 `mprotect_selftest`（`PROT_NONE` → 内核拒收该缓冲 `-EFAULT` → 改回 `RW` 内容原样 → 换 `RX` 后真的执行 `mov eax,42; ret`，另覆盖 brk 堆窗口与 6 类拒绝）；`make test-user` 61 → **91 项** |
