# 系统调用 / 用户空间 / ELF（阶段文档）

- 阶段：第六阶段（v0.6 – v0.7）+ 第九阶段（v0.9 运行时前置条件与动态链接）
- 最后更新：2026-10-12
- 状态：🟡 v0.7 已落地：**ELF Loader**（从 RAMFS 上的 `/init` 装载，按段权限逐页映射）+ 文件 syscall
  （`read`/`write`/`open`/`close`，对接 VFS/RAMFS）+ 用户 `RW` 段对 ring3 可写；
  **v0.9 已落地六步**：用户堆（3.5）、`stdin` + 交互式 Shell（3.6）、
  **进程对象 + `fork`/`execve`/`wait4` + `argc`/`argv`**（3.7，外部程序真的能跑了）、
  简化网络栈（drivers.md 1.8）、基础用户程序与目录/进程枚举（3.8）、
  **内核侧最小动态链接**（3.9：`PT_INTERP` + `DT_NEEDED` + `RELATIVE`/`JUMP_SLOT`）；
  **v1.0 第一步已落地**：`mprotect`（3.10：已映射页的保护属性可在运行期改写，`PROT_NONE`
  与 W^X 的「先写后换只执行」因此成立）；
  `make test-user` 96 项 / `make test-shell` 11 项 / `make test-proc` 8 项 + 4 条正则 /
  `make test-programs` 33 项 + 7 条正则 / `make test-dyn` 10 项 / `make test-fs` 21 项 PASS；
  **socket/TCP（3.13）**：`socket`/`bind`/`connect`/`sendto`/`recvfrom`
  （Linux 号 41/42/44/45/49）+ 内核 TCP 客户端最小集 + `/bin/tcpecho` 三轮回声收发，
  `make test-network` 24 项 PASS（见 drivers.md 1.8）；
  仍未做：环境变量（`envp`）、**完整**动态链接（用户态 `ld.so`，见 3.9）、文件映射（`mmap` 加 offset）、
  TCP 服务端（`listen`/`accept`）、UDP 套接字、ICMP
- **阶段三（2026-10-12）**：调用号 / `errno` / 标志位 / 记录布局的**权威定义已移入 `abi/` crate**
  （单一事实源）：本文档的号段表与各记录布局由它校对，内核与 `libc` 都引用同一处；
  防漂移命令是 `make abi-check`（序号/号段/记录尺寸/能力位固化）+ `abi/src/record.rs`
  （编译期断言：改坏即构建失败）。本文档仍是**文字规格**，代码里的数字以 `abi/` 为准

> 本文档顶部保留阶段 / 最后更新 / 状态字段，与项目文档维护约定对齐。
- 依赖：process.md（进程模型）、memory.md（用户地址空间）

## 1. 概述

v0.6 与 v0.7 一起构成系统调用 / 用户空间阶段，但两者有清晰先后：

1. **v0.6 核心**：建立用户态 / 内核态隔离 + 系统调用入口 + 第一批 syscall + 用户态 `hello` 运行。
2. **v0.7 扩展（本版已完成）**：ELF Loader（从文件装载、按段权限映射）、
   VFS 对接的文件语义（`open`/`read`/`write`/`close`）。
3. **v0.9 第一步（已做）**：用户堆 —— `brk`/`mmap`/`munmap` + libc 的全局分配器（见 3.5）。
4. **v0.9 第二步（已做）**：`stdin`（键盘行缓冲 → `read(0)` 整行交付）+ 交互式 Shell `/bin/sh`（见 3.6）。
5. **v0.9 第三步（已做）**：进程对象与 `fork`/`execve`/`wait4` + `argc`/`argv` ——
   Shell 现在能真的跑起外部程序（见 3.7 与 `docs/process.md` 1.4）。
6. **v0.9 第五·六步（已做）**：基础用户程序与目录/进程枚举（见 3.8）、
   **内核侧最小动态链接**（见 3.9：`PT_INTERP` + `DT_NEEDED` + `RELATIVE`/`JUMP_SLOT`）。
7. **v1.0 第一步（已做）**：`mprotect`（见 3.10）—— 已映射页的保护属性可在运行期改写，
   `PROT_NONE` 与 W^X 的「先 `mmap(RW)` 写、再 `mprotect(RX)` 执行」因此成立。
8. **socket/TCP（已做，见 3.13）**：`socket`/`bind`/`connect`/`sendto`/`recvfrom`
   （Linux 号 41/42/44/45/49）+ 内核 TCP 客户端最小集（三次握手/按序收发/FIN 关闭）+
   `/bin/tcpecho`（`sendto`/`recvfrom` 与 `write`/`read` 两轮 + 超 MSS 小缓冲第三轮）。
9. **仍未做**：环境变量（`envp`）、**完整**动态链接（用户态 `ld.so`）、文件映射（`mmap` 加 offset）、
   TCP 服务端（`listen`/`accept`）、UDP 套接字、ICMP。

## 2. v0.6 实际实现（2026-10-10 落地）

### 2.1 入口选型（已固化，不再是草案）

**决定了 `syscall` / `sysret` 快速路径**（`int 0x80` 方案不再保留）。

| 项 | 取值 | 说明 |
| ---- | ---- | ---- |
| 入口指令 | `syscall` → `IA32_LSTAR` = `syscall_entry` | 保存 RIP→`rcx`、RFLAGS→`r11`，**不切栈** |
| 返回指令 | `sysretq` | CS=`STAR[63:48]+16`、SS=`STAR[63:48]+8` |
| syscall 号 | `rax` | 越界（≥ `SYS_MAX`=267）返回 `-ENOSYS` |
| 参数 | `rdi, rsi, rdx, r10, r8` | 与 Linux x86_64 一致，便于以后直接跑现成用户态程序 |
| 返回值 | `rax` | 负数 = `-errno`（如 `-ENOSYS` = -38、`-EFAULT` = -14） |
| 进入内核时的 RFLAGS 掩码 | `IA32_FMASK` = IF\|DF\|TF = `0x700` | 内核在关中断下运行 syscall handler |

`syscall` 不切换栈，因此入口汇编的第一件事是把用户 RSP 存到 `CER_USER_RSP` 并
切到**专用系统调用栈**（`trampoline.rs` 的 16 KiB 静态栈）。

### 2.2 GDT 布局（由 `IA32_STAR` 决定，顺序不可随意调整）

```text
0x00 null | 0x08 内核代码 | 0x10 内核数据 | 0x18 用户数据 | 0x20 用户代码 | 0x28 TSS
```

- `syscall`：CS = STAR[47:32] = `0x08`，SS = CS+8 = `0x10`；
- `sysret`：STAR[63:48] = `0x10` → SS = base+8 = `0x18`，CS = base+16 = `0x20`。

「用户数据段紧邻用户代码段之前」是硬约束；改动顺序会让 `sysret` 直接 #GP。
`trampoline.rs` 里有两条编译期断言把汇编里的立即数 `0x1b`/`0x23` 与 GDT 常量绑定。

### 2.3 CPU 侧使能项

- `EFER.SCE`（使能 `syscall`/`sysret`）；`EFER.NXE`（页表 NX 位，v0.3 起堆页已用 bit63）；
- `CR4.SMEP/SMAP` **清除**：ring0 需要直接读写用户页（`sys_read`/`sys_write` 拷用户缓冲）。
  v0.7 **未**改为 `stac`/`clac` 精细控制（内核仍直接访问用户页），该项列入 3.4 未做项；
- `IA32_STAR` / `IA32_LSTAR` / `IA32_FMASK` 见上表。

### 2.4 用户地址空间（`syscall/address_space.rs`）

> v0.6 形态。“代码页（1..=8 页、只读可执行）”在 v0.7 已被“映像窗口（16 页）按 ELF 段权限逐页映射”
> 取代，见 3.1；本节的页表构造（PML4 下标 1、惰性清 `USER` 位、每级置 `USER`）至今未变。

- 用户映射放在 **PML4 下标 1**（`0x0000_0080_0000_0000` = 512 GiB，规范低半区）。
  内核恒等映射只用下标 0，因此用户子树完全归本模块所有，`Drop` 时可整棵释放；
- 新 PML4 先**逐项拷贝**内核 PML4（内核在新地址空间里照样可见，切 CR3 不崩）；
- 再**惰性清理**拷贝过来的内核子树中的 `USER` 位：只在 `USER` 确实为 1 时才写回，
  并把清位数打到串口；
- 用户子树 PDPT/PD/PT **每一级**都置 `USER`：ring3 要求每级都置位才能到达叶子页；
- 用户代码页 = `PRESENT|USER`（只读可执行，不可写），**按映像大小逐页映射**（1..=8 页，每页 4 KiB）；
  用户栈 4 页 = `PRESENT|WRITABLE|USER|NO_EXEC`（固定排在代码区上限之后，地址不随程序大小漂移）；
- 帧分配任何一步失败都会回滚已分配的帧，不留泄漏（v0.6 是 `new()` 与 `load_program()` 各自回滚；
  v0.7 的入口是 `map_image_page()` —— ELF Loader 调用它逐页映射，失败时由 `Drop` 统一回收）。

> 固件页表只读（实测发现）：OVMF/EDK2 通过 EnablePageTableProtection 把自己建立的
> 页表页标记为**只读**。因此“把整棵内核子树无条件重写一遍”会在启动路径上直接 #PF
> （实测：写 `0x1fc03000` 处的 PDPT 项，`error=P|W`）。这正是上面“惰性清理”的原因：
> `USER` 本来就为 0 的表项“改成 0”没有任何意义，而这个写入会立刻崩溃。

### 2.5 用户态执行期的内核状态收敛

进入 ring3 前必须做两件事（否则会以难以定位的方式炸掉）：

1. **挂起调度器**（`scheduler::suspend`）。用户程序不在调度器任务表里，若 PIT 中断在
   用户态上下文里切到内核任务，那个任务会在「用户页表 + 用户栈」的中间状态上恢复执行。
   挂起后 PIT 仍递增 tick 与打印心跳，只是不再切换；
2. **`TSS.RSP0` 指向专用陷入栈**（`trampoline::trap_stack_top()`）。进入用户态时内核
   调用栈仍然存活（`enter_user` 的调用者还压着返回地址），若沿用同一条栈，用户态期间的
   中断会把帧压到活动的内核栈上，覆盖内核自身的栈帧。返回内核后恢复原 `RSP0`。

两条栈分开：用户态中的中断/异常走陷入栈，`syscall` 走系统调用栈。

### 2.6 异常处理（ring3 不再 panic）

`exception_dispatch` 用 **CS 的 RPL 位**区分异常来源：

- `cs & 3 == 3` → 来自 ring3：打印 `user: FAULT vector/error/rip/cr2` 与
  `user: isolation PASS` 提示，然后 `trampoline::leave()` **终止用户态执行**并回到内核调用者，
  内核继续存活；
- 否则维持原行为（打印完整诊断 + `KERNEL PANIC`）。

### 2.7 分发层与首批 syscall（已实现）

| syscall | 号 | 状态 | 行为 |
| ---- | ---- | ---- | ---- |
| `read` | 0 | ✅ v0.7 / v0.9 / TCP 补齐 | fd 0 → **键盘行缓冲整行交付**（v0.9：行尾统一 `\n`；3 秒无整行 → `-EAGAIN`，见 3.6）；fd ≥ 3 → 从 VFS 读入用户缓冲（要求**可写**，否则 `-EFAULT`）；偏移量随之推进，EOF 返回 0；**套接字 fd（≥ 16）→ `recv`（首段即返），见 3.13** |
| `write` | 1 | ✅ v0.6/v0.7 / TCP 补齐 | `fd=1/2` 写串口；`fd ≥ 3` 写文件（VFS，偏移量随之推进）；缓冲要求可读；**套接字 fd（≥ 16）→ `send`（返回已确认字节数，可能部分发送），见 3.13** |
| `open` | 2 | ✅ v0.7 | `open(path_ptr, path_len, flags)`：路径用**指针 + 长度**（内核已校验区间，不在不可信内存上找 NUL）；`flags` 只接受 `O_RDONLY`/`O_WRONLY`/`O_RDWR`/`O_CREAT`，返回**最小空闲 fd**（≥3） |
| `close` | 3 | ✅ v0.7 / TCP 补齐 | 释放动态 fd（0/1/2 是保留位，关闭它们返回 `-EBADF`）；**套接字 fd 先 FIN 关闭再释放槽位（握手失败也释放），见 3.13** |
| `sleep`（nanosleep） | 35 | ✅ v0.6 | 以 PIT tick（10ms）为单位等待；期间 `sti; hlt` 让 PIT 继续推进 |
| `getpid` | 39 | ✅ v0.6 | 返回当前用户程序 PID（串口打印 `user: getpid -> N`） |
| `exit` | 60 | ✅ v0.6 | 记录退出码并结束用户态执行（不返回） |
| `mmap` | 9 | ✅ v0.9 第一步 | 匿名映射：只收 `MAP_PRIVATE\|MAP_ANONYMOUS`、`fd` 必须 -1、**只传 5 个参数（无 offset）**；first-fit 落在 256 KiB 的 mmap 窗口；新页**保证全零**；`PROT_WRITE\|PROT_EXEC` 拒绝（W^X）；`PROT_NONE` 自 v1.0 起合法（映射存在、帧已分配，但 ring3 不可访问，可由 `mprotect` 启用，见 3.10） |
| `munmap` | 11 | ✅ v0.9 第一步 | 整段归还：页对齐 + 全段已映射，否则 `-EINVAL`；执行时清页表项 **并 `invlpg`**（否则 TLB 会留下幽灵映射） |
| `brk` | 12 | ✅ v0.9 第一步 | 增长/收缩堆顶（窗口 256 KiB）；`addr = 0` 只查询；**Linux 语义：不返回错误**，失败返回原值（`ret < addr` 即失败）；增长全成或全不成 |
| `fork` / `execve` / `wait4` | 57 / 59 / 61 | ✅ v0.9 第三步 | `fork` 复制地址空间与当前用户帧（子进程 `rax=0`、父进程得 PID）；`execve(path, argv)` 换映像并重建 argv 用户栈（**成功不返回**）；`wait4(pid)` 运行并回收直接子进程、返回退出码（非子进程 `-ECHILD`）；详见 3.7 |
| `mprotect` | 10 | ✅ v1.0 第一步 | 改写**已映射**页的保护属性（页对齐、`len > 0`、范围不跨窗口、每页已映射，缺一即 `-EINVAL`）；`PROT_WRITE\|PROT_EXEC` 仍拒绝（W^X）；`PROT_NONE` 清 `USER` 位而保留 `PRESENT`（帧与内容都在，可改回来）；用户栈固定 `RW`+`NX`，不在支持范围 —— 详见 3.10 |
| `socket` / `bind` / `connect` / `sendto` / `recvfrom` | 41 / 49 / 42 / 44 / 45 | ✅ TCP 补齐 | 只认 `AF_INET` + `SOCK_STREAM` + `IPPROTO_TCP`（其余 `-EAFNOSUPPORT`）；地址是 16 字节 `sockaddr_in`（`addr_len != 16` 即 `-EINVAL`）；套接字 fd 独立号段（16..=23）；`sendto`/`recvfrom` 只收 `NULL` 地址（已连接字节流）；详见 3.13 |

分发表按 `SYS_MAX`(=267) 排满：未实现的号落在公共 ENOSYS 桩，
「号合法但未实现」与「号越界」是两种可区分的失败路径。
号段划分：`0..=255` 沿用 Linux 语义，`>= 256` 是 Cerlesse 扩展 —— `256` = `getprocs`、
`257..=260` 由 CKM 提案预留、`261..=266` 由 CSS 使用（已实现 **261 `storage_volumes`**、
**262 `storage_mount`**、**263 `storage_unmount`**、**264 `storage_assign_letter`**、**265 `storage_rescan`**；
只有 **266 `storage_format`** 仍落 ENOSYS 桩 —— 它要 CSS-4 才定义，见 3.11/3.12）。
Linux 号段另有 **socket 系 41/42/44/45/49**（`socket`/`connect`/`sendto`/`recvfrom`/`bind`，
见 3.13；`listen`/`accept` 等服务端号仍为 ENOSYS 桩，连名字都未发布）。

文件 syscall 的两条硬规则（v0.7 新增）：

1. **用户指针先校验再用**：`write`/`open` 要求缓冲可读、`read` 要求缓冲可写，
   按**页**检查（半页在映射里、半页越界也会被拒），失败返回 `-EFAULT` 并留痕
   `user: EFAULT write ptr=0xdead00000000 len=1 (buffer not readable in user space)`；
2. **单次 I/O 上限** 4096 字节（`IO_MAX`）与路径上限 256 字节（`PATH_MAX`），
   避免用户态用巨大 count 让内核长时间不返回。

文件相关错误的来源分别是：`-EBADF`（fd 未打开/模式不符）、`-ENOENT`（路径不存在）、
`-EISDIR`/`-ENOTDIR`（类型不符）、`-EEXIST`（`create` 已存在）、`-ENOSPC`（fd 用尽 /
文件超过 RAMFS 上限）、`-EINVAL`（flags 未知位/非法路径）—— 与 `fs` 层的 errno 一致。

### 2.8 用户程序来源与运行驱动（`syscall/user.rs`，v0.6 形态）

> v0.7 已把这里描述的两条路径升级为「构建期内嵌 **ELF** → RAMFS `/init` → ELF Loader」，
> 见 3.1。本节保留 v0.6 的形态，因为它是 `source=` 字段与自测结构的由来。

v0.6 **没有** ELF Loader（那是 v0.7），用户程序的获取方式是「**构建期摊平 + 内嵌**」：

- `hello`：来自 `user/hello`（Rust + `libc`）。`kernel/build.rs` 在构建内核时用嵌套 cargo
  构建它（large 代码模型 + `user-release` profile），解析 ELF 并摊平成「从 `USER_VIRT_BASE`
  开始的连续映像」，写入 `$OUT_DIR/user_hello.bin` 后由内核 `include_bytes!` 内嵌。
  因此**跑的不是内核里的临时机器码，而是用户态脚手架的产物**（见 2.11）；
- `probe`：位置无关的手写机器码（`global_asm!`，只用立即数与寄存器间接寻址，
  因此整段字节可直接拷到用户代码页执行），读内核映像地址 `0x200_0000` 验证隔离。
  v0.7 仍然保留这条路径（`UserSpace::map_raw`）：隔离自测需要一段**必然越权**的代码，
  把它写成 ELF 只会让探针依赖被测的 ELF Loader。

运行驱动 `run(name, source, program, pid)` 的顺序（每一步都有理由，顺序错了就会崩）：

```text
UserSpace::new() → load_program()
→ TSS.RSP0 = 陷阱栈顶 → CER_USER_SPACE = &space → write_cr3(用户页表根)
→ iretq 到 ring3（enter_user）
→ ... 用户程序跑 syscall / 触发 ring3 异常 ...
→ leave_user 回到内核 → write_cr3(内核根) → 恢复 TSS.RSP0 → CER_USER_SPACE = 0
→ drop(space) 回收全部用户帧与页表帧
```

### 2.9 验收（v0.6）

已通过 `make test-user`（**v0.6 时为 19 项串口断言**，QEMU + OVMF；v0.7 增至 32 项，
v0.9 第一步增至 59 项，见 3.3 / 3.5）：

- [x] 脚手架生效：`user: program=hello source=libc bytes=6584 code_pages=2`
      （比串口断言更硬的是可重复性：`make user` 不启 QEMU 就能单独构建 `libc` + `user/hello`，
      产物 ELF 无重定位、入口 `0x8000000000`）
- [x] ring3 进入可观测：`user: enter name=hello pid=1 entry=0x8000000000`
- [x] syscall 入口/返回可观测：`user: write fd=1 len=22`、`user: getpid -> 1`
- [x] `exit` 回收退出码：`user: exit code=42` → `user: hello PASS`
- [x] `sleep` 可用且计时真实推进：`user: sleep ticks=2` / `user: sleep done ticks=2`
- [x] 用户地址空间隔离：probe 读内核页被拦下，
      `user: FAULT vector=0xe error=0x5` + `user: FAULT cr2=0x2000000` +
      `user: isolation PASS`，且**内核不 panic、继续运行**
- [x] 用户态 `/init` 通过 **ELF Loader** 加载运行 → v0.7 已完成（见 3.1 / 3.3）
- [x] `fork`/`exec`/`wait` **真实语义** → v0.9 第三步已完成（3.7）：前置条件是进程对象
      （每进程地址空间 + 每进程内核栈），v0.6/v0.7 的 `exit` 语义因此从
      「结束当前用户态执行」升级为「标记僵尸进程并回到 `wait4` 的调用方」

### 2.10 v0.6 明确未做的事（避免误读）

- **没有** ELF Loader（用户程序是构建期摊平的内嵌映像，不是从文件加载）→ **v0.7 已做**（3.1）；
  没有用户栈参数传递（`argc/argv`）→ **v0.9 第三步已做**（参数经入口寄存器 `rdi`/`rsi` 传递，见 3.7）；
- **没有**用户堆/`brk`/`mmap`。v0.6 里用户程序连静态数据都不能写（代码页整体只读）；
  v0.7 起 `RW` 段（`.data`/`.bss`）映射为 ring3 可写（实测 `hello: static counter=1`），
  但仍然**没有堆**；
- **没有**进程/线程对象：一次只跑一个用户程序，`exit` 结束该次执行并回到内核
  → **v0.9 第三步已做**（见 3.7：每进程 PCB + 每进程内核栈）；
- **没有** `fork/exec/wait` → **v0.9 第三步已做**（见 3.7）；
  v0.6 没有 `read/open/close` → **v0.7 已做**（见 2.7 / 3.2）；
- **没有**高半区迁移：内核仍在低地址恒等映射内，隔离靠**页表项 `USER` 位**而不是地址空间划分
  （ADR-009 的“高半区”一项推迟，实现方式见 2.4）；
- 用户页表是内核 PML4 的**快照**：内核在用户态运行期间新增 PML4 槽位级的映射不会出现在
  用户页表中（用户态路径只做串口输出与 tick 等待，不触发这类增长）。

### 2.11 用户态脚手架（`libc/` + `user/*`，2026-10-11 落地）

v0.6 的目标之一是「新用户程序不必再手写机器码」。脚手架由三部分组成：

| 组成 | 位置 | 职责 |
| ---- | ---- | ---- |
| 运行时 | `libc/` | syscall 封装（`write`/`getpid`/`nanosleep`/`exit`）、`_start` → 用户 `main` → `exit`、`print!/println!`、`#[panic_handler]` |
| 用户程序 | `user/hello/` | 第一个真实程序：`Cargo.toml` + `build.rs`（传链接脚本）+ `user.ld` + `src/main.rs` |
| 构建链 | `kernel/build.rs` | 构建用户程序 → 校验 ELF → `include_bytes!` 内嵌（v0.7：**不再摊平**，见 3.1） |

构建与装载流水线（v0.6 形态；v0.7 的替换见 3.1）：

```text
user/hello (Rust + libc)
  └─ cargo build -p user-hello --target x86_64-unknown-none --profile user-release
       RUSTFLAGS="-C relocation-model=static -C code-model=large"
  └─ user-hello ELF（链接在 0x8000000000，无重定位）
       └─ kernel/build.rs::flat_image() ⟶ $OUT_DIR/user_hello.bin（第 0 字节即入口）
            └─ include_bytes! ⟶ UserSpace::load_program()（逐页拷贝 + 逐页映射 PRESENT|USER）
                 └─ iretq ⟶ ring3
```

两个「错了必然跑不起来」的约束（`kernel/build.rs` 在构建期直接断言，不留给运行期；
v0.7 又加了「段页对齐」与「映像窗口」，见 3.1）：

1. 映像基址 = `USER_VIRT_BASE`（0x0000_0080_0000_0000）—— 内核不做加载时重定位；
2. 不存在可装载重定位（`SHT_RELA`/`SHT_REL` + `SHF_ALLOC`）—— 同上的直接后果。

链接参数也不是可选的：链接地址 2^39 超出 small 代码模型 ±2GiB 的绝对寻址范围，
必须 `code-model=large`；而 large 模型与 LTO 不兼容（实测 rust-lld 报
`failed to load bitcode of module core-*.o`），因此用户程序用 workspace 的
`user-release` profile（inherits `dev`、`lto = false`、`opt-level = 2`、`strip = true`）。
另外，外层 cargo 会把 `CARGO_ENCODED_RUSTFLAGS` 注入 build script 的环境且其优先级
高于 `RUSTFLAGS`，所以 `kernel/build.rs` 调用嵌套 cargo 时必须显式移除它，
否则会用内核的 rustflags 去编用户程序（直接报 `R_X86_64_32S out of range`）。

用法（新程序的完整约定见 [libc/README.md](../userland/libc/README.md)）：

```rust
#![no_std]
#![no_main]

use libc::{getpid, nanosleep, println};

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("hello from user space"); // 一次宏调用 = 一次 write
    println!("pid={}", getpid());      // core::fmt 可用，不需要堆
    nanosleep(2);                      // 2 个 PIT tick = 20ms
    42                                 // 退出码
}
```

验证方式：`make user`（不启 QEMU，验编译链接 **+ 段布局**）与 `make test-user`（ring3 实测，
断言 `user: program=/init source=elf`）。

## 3. v0.7 实际实现（2026-10-11 落地）

### 3.1 ELF Loader（`kernel/src/syscall/elf.rs` + `address_space.rs`）

v0.6 的「构建期摊平 + 原样拷贝」被替换成真实的加载链：

```text
user/hello (Rust + libc)
  └─ cargo build（large 代码模型 + user-release profile）
  └─ user-hello ELF（链接在 0x8000000000；段页对齐、无重定位）
       └─ kernel/build.rs：校验（基址/页对齐/W^X/映像窗口/无重定位）→ $OUT_DIR/user_hello.elf
            └─ include_bytes! → fs::init：写进 RAMFS 的 /init
                 └─ vfs::read_all("/init")（挂载表路由 + 文件读）
                      └─ elf::load() → UserSpace::map_image_page() 按段权限逐页映射
                           └─ iretq → ring3
```

装载做的事（全部在**内核 CR3 下**、切 CR3 之前完成）：

1. **解析**：只接受 `ELF64 / little-endian / ET_EXEC / EM_X86_64`；每个 `PT_LOAD` 必须在
   用户映像窗口 `[0x0000_0080_0000_0000, +64 KiB)` 内，入口必须落在某个段里；
   （v0.9 动态链接起，`ET_DYN` 在 `load` 入口分流到 `dynamic.rs`，见 3.9 —— 上面的固定地址
   规则仍是 `ET_EXEC` 路径的规则，那条路径一行未改）
2. **映射**：按**页**并集各段权限 —— `p_flags` 的 `W` → `WRITABLE`、非 `X` → `NO_EXEC`；
   页帧在首次映射时分配并清零，同页被多个段覆盖时只并集权限（不重复清零）；
3. **拷贝**：每段 `p_filesz` 字节写进对应页，`p_memsz - p_filesz` 保持 0（这就是 `.bss`）；
4. **入口**：直接用 ELF 头的 `e_entry`（不再要求「映像第 0 字节即入口」）。

用户虚拟地址布局（固定，不随程序大小漂移）：

```text
0x0000_0080_0000_0000 + 0x0000   页 0..15   用户映像窗口（64 KiB，ELF PT_LOAD 按段权限映射）
0x0000_0080_0000_0000 + 0x10000  页 16..19  用户栈（16 KiB，RW、NX）
0x0000_0080_0000_0000 + 0x20000  页 32..95  brk 堆窗口（256 KiB，按需映射，RW、NX；v0.9）
0x0000_0080_0000_0000 + 0x60000  页 96..159 匿名 mmap 窗口（256 KiB，first-fit，RW 或 RX；v0.9）
0x0000_0080_0000_0000 + 0x100000 页 256..287 共享库窗口（128 KiB；v0.9 动态链接：解释器 + `.so`）
```

五个窗口都在同一张 PT 覆盖的 2 MiB 内（共 288 页），因此仍然只需要一张页表；
窗户之间（如页 20..31）没映射 —— ring3 摸到就是 #PF，这是隔离的正常表现。

两个新的构建期/装载期约束（都有实测理由）：

| 约束 | 不满足时 |
| ---- | ---- |
| 每个段**页对齐**（`user.ld` 里 `ALIGN(0x1000)`） | 同页既 `W` 又 `X` → `W^X` 退化，装载器拒绝：`user: ELF rejected: W and X segments share a page` |
| 零初始化静态量用 `.lbss.*` 段名（large 代码模型） | `user.ld` 漏列 `.lbss.*` 时它们成为**孤儿段**被塞在 `.data` 之后，RW 段不再页对齐（实测落在 `0x8000003040`） |

链接脚本还要注意：`.text`/`.rodata`/`.data`/`.bss` 四个段分别 `ALIGN(0x1000)` 之后，
`user/hello` 的 ELF 是 4 个 `PT_LOAD`（`RX` / `R` / `RW` / `RW`（`.bss`））、4 个映像页、2 个可写页。

`make user` 不启 QEMU 就能独立验证这套约定：`tools/check_user_elf.py` 检查基址、段页对齐、
`W^X` 不重叠、映像窗口与「无加载时重定位」。

### 3.2 文件语义对接（已完成）

| 项 | 实现位置 | 说明 |
| ---- | ---- | ---- |
| fd 表 | `kernel/src/fs/file.rs` | 0/1/2 保留（stdin/串口），动态 fd 从 3 开始、取最小空闲槽位；记录 `path` + `offset` + 模式 |
| `read(0)`（stdin） | `syscall/file.rs::read_stdin` | **键盘行缓冲（v0.9）**：交付整行（行尾统一 `\n`）；3 秒没有整行返回 `-EAGAIN`(-11)，见 3.6 |
| `open` | `syscall/file.rs::sys_open` | 路径用「指针 + 长度」；带 `O_CREAT` 时不存在则创建（`EEXIST` 视为成功） |
| `read`/`write` | `syscall/file.rs` | 用户缓冲按页校验（读要求可写、写要求可读）；fd ≥ 3 转 `vfs::read/write` |
| VFS/RAMFS | `kernel/src/fs/` | 见 [filesystem.md](filesystem.md)；挂载表最长前缀路由，`/mnt` 上挂第二个 RAMFS |

### 3.3 验收（v0.7，实测）

`make test-user`（v0.7 时 32 项；v0.9 第一步增至 59 项、第三步起 61 项、v1.0 第一步起 91 项、TCP 补齐后 **96 项**，见 3.5/3.10/3.13）与 `make test-fs`（**21 项**）
串口断言全部 PASS，关键证据：

- [x] **ELF 从文件装载**：`fs: create /init` → `user: program=/init source=elf bytes=15596 segments=4
      image_pages=7 writable_pages=2 entry=0x8000000000`（`source=elf` 与 `segments/writable_pages`
      是「走的是 ELF Loader 且段权限生效」的证据；`bytes`/`image_pages` 随 `/init` 加入堆自测而增长）；
- [x] **文件 syscall 全链路**：`user: open path=/etc/motd flags=0x0 fd=3` →
      `user: read fd=3 want=64 got=23` → 内容由用户态 `write` 打到串口（`user: write fd=1 len=23`）→
      `user: read fd=3 want=64 got=0`（EOF，证明偏移在推进）→ `user: close fd=3`；
- [x] **用户写入落到文件系统**：`user: open path=/tmp/hello.txt flags=0x41 fd=3`（fd 复用 3）→
      `user: write fd=3 len=24` → 内核读回 `fs: readback /tmp/hello.txt len=24` +
      `fs: user data: written by user program`；
- [x] **`RW` 段对 ring3 可写**：`hello: static counter=1`（v0.6 会 #PF —— 代码页整体只读）；
- [x] **syscall 参数校验**：`user: EFAULT write ptr=0xdead00000000 len=1 (buffer not readable in user space)`
      且用户侧收到 `hello: bad write ret=-14`；
- [x] **寄存器保存规则**：一次 `syscall` 往返后除 `rax`/`rcx`/`r11` 外全部寄存器保持进入前的值
      （Linux syscall ABI 语义，见 2.1 与 B-15）；
- [x] 隔离与退出码回归不变：`FAULT vector=0xe error=0x5 cr2=0x2000000` → `user: isolation PASS`，
      `user: exit code=42` → `user: init PASS`；
- [x] 除 `/init` 之外的**多个**用户程序：v0.9 第二步落地 `/bin/sh`（`user/shell`）——
      同样从 VFS 读 ELF 装载，因此「多个程序」不再需要 `argc/argv`（命令来自 stdin，见 3.6）；
- [x] `ls`/`cd` 等**外部**用户程序 → v0.9 第三步已打通：`/bin/hello`（`user/hello-bin`）
      由 Shell `fork` + `execve` 起来，拿到 `argc/argv` 并返回退出码（`make test-proc`）；
      v0.9 第五步起 `ls`/`cat`/`echo`/`ps` 也是真实程序（`/bin/*`，见 3.8）——
      注意 `cat`/`echo` 的**裸命令名**仍走 Shell 内建命令（内建遮蔽同名程序，与真实 shell 一致），
      跑程序本身要写 `/bin/cat` / `/bin/echo`。

### 3.4 仍未做（v0.9 起逐步补齐）

- ~~用户堆~~ → **已做**（3.5：`brk`/`mmap`/`munmap` + libc 全局分配器）；
  ~~`mprotect`（`PROT_NONE`、W^X 转换）~~ → **已做**（3.10）；仍缺**文件映射**（`mmap` 加 offset）；
- ~~`stdin`~~ → **已做**（3.6：键盘行缓冲 → `read(0)` 整行交付）；
- ~~**`fork`/`execve`/`wait4` 与进程对象**~~ → **已做**（3.7）：每进程地址空间 +
  每进程内核栈 + 停在 syscall 里的完整用户帧，Shell 的外部程序（`/bin/hello`）已能跑；
- ~~`argc/argv`~~ → **已做**（3.7：参数经入口寄存器 `rdi`/`rsi` 传递，`libc::args` 读出）；
  环境变量（`envp`）仍未做；
- ~~基础用户程序与目录/进程枚举~~ → **已做**（3.8：`getdents` + `getprocs`，`/bin/{ls,cat,echo,ps}`）；
  其余程序（`cp`/`mv`/`rm`/`mkdir`/`kill`）与 Shell 增强（`cd`、管道、重定向）仍未做；
- `stac`/`clac` 精细控制（内核目前直接读写用户页）、内核高半区迁移；
- ~~动态链接 / `PT_INTERP` / `ET_DYN`~~ → **最小闭环已做**（3.9：`PT_INTERP` + `DT_NEEDED` +
  `RELATIVE`/`GLOB_DAT`/`JUMP_SLOT`）；**完整**动态链接（用户态 `ld.so`、版本符号、TLS、`rpath`）仍未做。

### 3.5 用户堆（v0.9 第一步，2026-10-11 落地）

v0.9 的运行时前置条件按依赖顺序补：**先有可分配的用户内存**（本版），再有 `argc/argv`、
`fork`/`execve`/`wait4` 与进程对象，然后是动态链接与 shell（shell 于 3.6 落地，动态链接的
最小闭环于 3.9 落地）。
理由很直接：glibc、`ld.so`、任何真实程序的第一件事都是要堆。

| 项 | 实现位置 | 说明 |
| ---- | ---- | ---- |
| 地址空间窗口 | `syscall/address_space.rs` | brk 窗口 256 KiB（页 32..95）+ 匿名 mmap 窗口 256 KiB（页 96..159），均**按需映射**：没碰过的页 `page_flags` 返回 `None`，因此指向它的用户指针会被 syscall 层判为 `EFAULT` |
| `brk` / `mmap` / `munmap` | `syscall/memory.rs` | 语义与错误码见 2.7；每个失败路径都留一行 `user: <op> rejected (...)` 或 `user: <op> errno=<n>` |
| 用户态分配器 | `libc/src/heap.rs` | `#[global_allocator]`（bump over `mmap`，64 KiB 块，`dealloc` 不回收）—— `Vec`/`String`/`Box` 从此可用 |
| 自测/验收 | `user/hello/src/main.rs` | 堆自测写在参考用户程序里（因此“堆能不能用”是 **ring3 事实**，不是内核侧自说自话） |

四条有实测理由的性质（`make test-user` 逐条断言）：

1. **新页必须是零**：`hello: heap zeroed=1` / `hello: mmap zeroed=1`。把上一任使用者的内容
   送给新调用方是信息泄漏，所以 `brk`/`mmap` 分配帧时都先清零；
2. **堆页要能当 syscall 缓冲**：用户程序把**堆上**的 26 字节交给 `write` 打到串口
   （`user: write fd=1 len=26` + `hello: heap write ret=26`）——证明 `check_range` 经 `page_flags`
   认可堆/mmap 页，而不是只认映像窗口与栈；
3. **`munmap` 之后真的访问不到**：清页表项的同时 `invlpg`（`paging::flush_tlb`）。
   少了刷新就会出现「页帧已归还、TLB 里还是旧映射」的幽灵访问；实测里释放后的同一地址
   又被下一次 `mmap` 复用（`user: mmap len=65536 ... -> 0x8000060000`），旧映射没有残留；
4. **用户态结束后页帧全部归还**：`UserSpace::Drop` 释放映像/栈/堆/mmap 四类页帧；
   `user: frames free before=119159 after=119159` → `user: heap frame reclamation PASS`
   （内核在 selftest 里直接对比帧分配器的 `free_frames`，不靠日志措辞）。

负例也逐条具体（不是「都失败就行」）：文件映射 / `MAP_SHARED` / `PROT_WRITE|PROT_EXEC` /
未对齐的 `munmap` / 重复 `munmap` → `-EINVAL`(-22)；超过 256 KiB 窗口 → `-ENOMEM`(-12)。

**仍然没有的**（写下来避免误读）：文件映射（`mmap` 无 offset 参数、`fd` 必须 -1）、
回收式分配器（`dealloc` 是空操作：一次运行结束由内核整棵释放；长驻程序的内存占用会单调上升）。
（`mprotect` 已由 v1.0 第一步补上，见 3.10。）

### 3.6 交互式 Shell 与 stdin（v0.9 第二步，2026-10-11 落地）

依赖顺序（与 3.5 一脉相承）：**先有用户堆 → 再有 `stdin` 与第二个用户程序 → 最后才是
进程模型/动态链接**。本版做的是中间那一步：把 v0.4 的键盘输入接到用户态，
并交付一个真正能交互的 Shell（`user/shell`，RAMFS 上的 `/bin/sh`）。

```text
PS/2 键盘 IRQ1 → 行缓冲 + 行编辑（driver/keyboard.rs）
      │ kbd: line ready len=5
      ▼
read(0)（syscall/file.rs::read_stdin）→ 整行（内容 + \n）
      │ user: read fd=0 want=128 got=5
      ▼
/bin/sh：提示符 → 解析 → 内建命令 → 循环
```

| 项 | 实现位置 | 说明 |
| ---- | ---- | ---- |
| 行缓冲 | `driver/keyboard.rs` | 128 字节缓冲 + 行编辑（退格、回显）；`Enter` 把整行标为可取并打印 `kbd: line ready len=N` |
| 回显协议 | 同上 | v0.9 起改为**终端式原样回显**（旧格式 `KB <字符>` 已废弃，`make test-keyboard` 同步改断言）；`KBIRQ ENTRY/EXIT` 不变 |
| `read(0)` | `syscall/file.rs::read_stdin` | 等待上限 `STDIN_IDLE_TICKS` = 300 tick（3 秒）→ `-EAGAIN`(-11)；等待期间 `sti; hlt`（同 `sys_sleep`），取到行后 `cli` 恢复 handler 关中断约定 |
| Shell | `user/shell/src/main.rs` | 提示符 `root@cerlesse C:>`（用户名@主机名 盘符>）；内建 `help`/`echo`/`cat`/`pid`/`exit`；不用堆（映像 5 页） |
| 外部命令 | 同上 `cmd_external` | `fork()` → 子进程 `execve("/bin/hello", argv)` → 父进程 `wait4(pid)` 取退出码；找不到的程序由 `execve` 返回 `-ENOENT`，子进程按惯例 `exit(127)`（见 3.7） |
| 内嵌与装载 | `kernel/build.rs` + `syscall/user.rs` + `fs::init` | 三个用户程序同一套构建/校验/装载路径，`/bin/sh` 只是“另一个普通程序” |

**为什么曾经不能“嵌套运行”外部程序**（保留这段记录，因为它是 v0.9 第三步的设计前提）：
v0.9 第二步时 ring3 入口（当时的 `trampoline::enter_ring3`）假设“一次只有一个用户程序”：
恢复现场用的用户上下文槽与陷入栈都是**全局唯一**的，所以从 `sys_read` 里再进去一次，
子程序的 syscall 会覆盖父程序停在 syscall 里的现场。**结论不是放弃，而是先补前置条件**：
v0.9 第三步把入口改成按进程切换（当前进程下标 + 每进程内核栈 + 每进程地址空间），
`suspend/resume` 才可能成立 —— 这就是 3.7 的做法与 `docs/process.md` 1.4 的实现。

验收（`make test-shell`，11 项，QEMU monitor 注入真实按键；断言的是日志行为不是
“有 Shell 字样”）：

- [x] `user: program=/bin/sh source=elf`：Shell 与 `/init` 一样**从 VFS 装载**；
- [x] `root@cerlesse C:>` + `user: write fd=1 len=17`：提示符按默认加载样式**一次 `write`** 送出；
- [x] `user: read fd=0 want=128 got=5`：注入的 `help`+Enter（4+1 字节）真的经键盘进了 `stdin`；
- [x] `sh: builtins (v0.9 step 3):`：解析与分派生效（帮助文本列出内建命令与外部程序约定）；
- [x] `user: open path=/etc/motd flags=0x0 fd=3` → `motd: hello from ramfs` →
      `sh: cat /etc/motd bytes=23`：`cat` 走真实文件 syscall；
- [x] `sh: bye` + `user: exit code=0` + `user: shell PASS`：退出路径；
- [x] 页帧不泄漏：`user: frames free before=… after=…`（全部归还）。

外部程序的单独验收见 3.7（`make test-proc`）。

**已知限制（故意写清楚）**：

1. `read(0)` 会在 3 秒无整行时返回 `-EAGAIN`，Shell 连续两次即自行退出
   （日志 `sh: no input (2 idle windows), exiting …`）。这是**没有进程模型时的权宜之计**：
   内核是在 syscall 里等用户程序，若 `stdin` 永久阻塞，任何无输入的启动都会停在 Shell 上，
   `user: selftest PASS` 不再出现。注意 `wait4` 现在**真的会阻塞式地跑子进程**（3.7），
   `stdin` 的 `-EAGAIN` 是仅剩的一处“不会被唤醒的等待”；
2. 只有内建命令（`help`/`echo`/`cat`/`pid`/`exit`）+ 外部程序（`/bin/*`）；
   没有 `cd`/管道/重定向。第三步时 `/bin/ls` 还不存在，`ls` 走的是
   「execve 失败 → exit(127)」这条路径；第四步把 `/bin/ls`（连同 `cat`/`echo`/`ps`）
   真的做了出来（见 3.8），负例改用 `foo`（`make test-proc` 已同步）；
3. 内建命令**遮蔽**同名程序（与真实 shell 一致）：裸 `cat`/`echo` 走内建，
   要跑程序本身必须写 `/bin/cat` / `/bin/echo`；`ls`/`ps` 没有内建版本，裸名字就是程序；
4. `argv` 由 Shell 切分后交给 `execve`（见 3.7），但**没有**引号/环境变量/globbing；
5. 回显是“cooked”（内核侧回显），且同一串口通道上还有 `KBIRQ ENTRY/EXIT` 标记 ——
   可读性靠“一次击键一段标记”的约定，将来有终端抽象后再收敛。

### 3.7 进程调用：`fork` / `execve` / `wait4` 与 `argc`/`argv`（v0.9 第三步，2026-10-11 落地）

内核侧的完整设计（PCB 字段、状态机、每进程内核栈、两条踩坑）写在
[docs/process.md](./process.md) 1.4；这里只记 syscall 层能看到的部分。

| 项 | 实现位置 | 说明 |
| ---- | ---- | ---- |
| `fork` | `syscall/process.rs::sys_fork` | 复制地址空间（`UserSpace::clone_for_fork`）+ 克隆当前用户帧（`rax=0`）；进程表满 → `-EAGAIN`；失败**不改动进程表** |
| `execve` | 同上 `sys_exec` | 参数是 `(path_ptr, path_len, argv_ptr, argc)`：路径与 `argv` 都用「指针 + 长度」并**先校验区间**再读；先读 ELF 再建新地址空间，**失败时当前映像不受影响**；成功后换 CR3 + `iretq` 到新入口（**不返回**） |
| `wait4` | 同上 `sys_wait` | 只认**直接子进程**（`find_by_pid(want, Some(my_pid))`），否则 `-ECHILD`；子进程从未跑过→现在跑（“延迟调度”）；已是僵尸→直接取退出码；两种路径都 `reap()` 回收地址空间 |
| `argv` 用户栈 | `syscall/process.rs::build_user_frame` | 从用户栈顶向下放参数字符串与指针数组（上限 `ARG_MAX_BYTES` = 1 KiB / 8 项 / 单项 256 字节），入口寄存器 `rdi = argc` / `rsi = argv` |
| 用户态包装 | `libc/src/syscall.rs` + `libc/src/args.rs` | `fork()` / `execve(path, argv: &[&str])` / `wait4(pid)` 与 `argc()` / `arg(i)` / `arg_str(i)` |
| 外部程序 | `user/hello-bin` → `/bin/hello` | 打印自己的 `pid`/`argc`/`argv` 后 `exit(1)` —— 它不关心自己是被谁跑起来的 |

**PID 不能写死在验收里**：`syscall::user` 手测里 `/bin/sh` 固定是 PID 3，但 `fork` 出的子进程号
取决于此前消耗了几个 PID（调度器自测会先取几个），所以 `make test-proc` 用**反向引用**
断言 fork / execve / wait4 三处的 PID 是同一个值（`--expect-re` 新增自 `tests/test_keyboard.py`）。

验收（`make test-proc`：注入 `hello world` / `ls` / `exit` 三个命令）：

- [x] `user: fork parent=3 -> child pid=N image_pages=7`：子进程拿到**父进程映像的副本**（7 页 = Shell 的映像）；
- [x] `user: run child pid=N name=fork-child`：子进程在父进程 `wait4` 时被恢复（单 CPU 的延迟调度）；
- [x] `user: exec pid=N path=/bin/hello segments=4 image_pages=6 entry=0x8000000000`：映像被**换掉**且 PID 不变（页数随 libc/程序体积变动，2026-10-12 实测 6 页）；
- [x] `hello-bin: argc=2` + `argv[0]=hello` + `argv[1]=world`：内核写的 `argv` 用户栈被用户程序读出来（`libc::args`）；
- [x] `user: exec /bin/ls -> ENOENT` + `sh: ls: exit code=127`：**路径不存在**也是完整链路（fork 成功 → execve 失败 → 子进程 `exit(127)` → 父进程拿到 127）；
- [x] `sh: hello: exit code=1` + `user: reaped pid=N exit=1`：退出码沿 `wait4` 回到父进程；
- [x] `user: frames free before=N after=N`：父进程 + 两个子进程的地址空间**全部归还**（无页帧泄漏）。

### 3.8 基础用户程序与目录/进程枚举（v0.9 第五步，2026-10-11 落地）

前三步把「用户程序」与「进程」做通了，但 `/bin` 里只有一个 `hello` —— 真正的命令行
程序一个也没有。这一步补上四个，并为此加了两个 syscall（都是**只读内核表**的调用）。

| syscall | 号 | 实现位置 | 用户态使用者 | 说明 |
| ---- | ---- | ---- | ---- | ---- |
| `getdents(path_ptr, path_len, buf_ptr, buf_len)` | 78 | `syscall/dir.rs::sys_getdents` | `user/ls` → `/bin/ls` | 把 `vfs::list()` 的结果写成**定长 88 字节记录**（`kind`/`size`/`name_len`/`name`）到用户缓冲；返回写入的记录数，装不下就**截断**（每条都完整）；缓冲 < 1 条记录 → `-EINVAL` |
| `getprocs(buf_ptr, buf_len)` | **256** | `syscall/process.rs::sys_getprocs` | `user/ps` → `/bin/ps` | 把进程表快照写成**定长 40 字节记录**（`pid`/`ppid`/`state`/`comm`）；按**槽位顺序**枚举，不承诺按 PID 排序 |

两个号的位置是有意的：`getdents = 78` 沿用 Linux 同名号（但**记录格式不同** —— 本内核的 VFS
没有 inode 号/硬链接，不编造 `linux_dirent64` 里的 `ino`/`off`/`d_type`）；而 Linux x86_64
**没有**「列举进程」的调用（它的 `ps` 读 `/proc`），所以 `getprocs` 放在 **256** ——
`syscall_table.rs` 明确把 `0..=255` 留给 Linux 语义、`>= 256` 作为 Cerlesse 扩展号段
（`SYS_MAX` 随之 256 → 257）。解析偏移只有一处权威定义（阶段三起：`abi::dirent` / `abi::procinfo`），
`libc` 提供 `dirent_name`/`dirent_is_dir`/`dirent_size` 与 `procinfo_pid`/`ppid`/`state`/`name`
让用户程序不必自己算偏移。

四个程序各自的「为什么值得单独存在」：

| 程序 | RAMFS 路径 | 用到的内核能力 | 输出（可与串口逐条对齐） |
| ---- | ---- | ---- | ---- |
| `user/ls` | `/bin/ls` | `getdents` | `ls: / entries=6` + 每条 `ls: entry name=… kind=file|dir size=…` |
| `user/ps` | `/bin/ps` | `getprocs` | 表头 + 每条 `ps: pid=… ppid=… state=RUNNING comm=…` + `ps: N process(es)` |
| `user/cat` | `/bin/cat` | `open`/`read`/`write`/`close` | 文件内容 + `cat: /etc/motd bytes=23` |
| `user/echo` | `/bin/echo` | `argc`/`argv` + 一次 `write` | 参数拼成一行（一次系统调用，与内建 `echo` 的每参数一次 `write` 不同） |

`ps` 需要一个「这个进程现在是什么程序」的字段：`Process::comm: [u8; 16]`（定长，
Linux `task_struct.comm` 的形状）—— `create` 用调试名填充，`execve` 成功后改写为
程序名（`/bin/ls` → `ls`）。之前只有 `name: &'static str`（写死的 `"exec"`），
要把它改成程序名就得泄漏一个 `String`，所以用定长字段而不是指针。

验收（`make test-programs`：注入 `ls` / `ls /bin` / `ps` / `/bin/echo …` / `/bin/cat /etc/motd`，
17 项 + 5 条正则）：

- [x] `user: getdents path=/ entries=6 returned=6` + `ls: / entries=6`：`ls` 读的是 **VFS 的目录项**
      （v0.7 起 `list()` 只有内核自测用过）；`ls: entry name=bin kind=dir size=0` 证明元数据也被读出来了；
      （`/` 在第五步时是 4 条，3.9 又加了 `/lib` → 5 条，CKM-2 又加了内核的 `/ckm`（覆盖区）→ **6** 条；
      `fs: ls / entries=5` 那条是内核自测的窗口，在 `/ckm` 建立之前，因此不变）
- [x] `ls: /bin entries=9`：`/bin` 里恰好是这次构建写进去的 9 个程序
      （`sh`/`hello`/`ls`/`cat`/`echo`/`ps`/`dynhello`/`volumes`/`tcpecho`，后三个见 3.9、3.11 与 3.12）—— 同时证明 `fs::init` 的程序清单是完整的；
- [x] `user: getprocs count=2` + `ps: pid=3 ppid=0 state=RUNNING comm=sh` + `ps: pid=N ppid=3 … comm=ps`：
      `ps` 读的是**内核进程表**：表里恰好是 Shell 与 `ps` 自己，`ppid` 指回 Shell（`fork` 建出的父子关系）；
- [x] `user: exec pid=N path=/bin/{ls,ps,echo} segments=4 image_pages=5 entry=0x8000000000`（`/bin/cat` 为 **6 页**）：
      四个程序都是**真的**被 `execve` 起来的（不是内建命令代劳）；
- [x] `cat: /etc/motd bytes=23` + `motd: hello from ramfs`：`/bin/cat` 走完整文件 syscall；
- [x] `echo program works`：`argv` 经内核写入新进程用户栈后被程序原样打印；
- [x] `sh: {ls,ps,/bin/echo,/bin/cat}: exit code=0`：四个程序的退出码都沿 `wait4` 回到 Shell。

### 3.9 内核侧最小动态链接（v0.9 第六步，2026-10-11 落地）

v0.9 之前装载器只接受 `ET_EXEC`：ELF 里的地址就是运行地址，内核不做任何加载时重定位
（`tools/check_user_elf.py` 的第 7 条就是「没有 `SHT_REL/RELA + SHF_ALLOC`」）。这一步把
**动态链接的最小闭环**在 ring3 里跑通 —— 目标不是「能跑 glibc」，而是让一条真实存在的
动态链接路径（`PT_INTERP` + `DT_NEEDED` + PLT/GOT 重定位）有可断言的证据。

| 环节 | 实现位置 | 说明 |
| ---- | ---- | ---- |
| `ET_DYN` 分流 | `syscall/elf.rs::load` 入口 | `e_type == 3` → `syscall/dynamic.rs::load`；`ET_EXEC` 的静态程序路径**一行未改** |
| `PT_INTERP` | `syscall/dynamic.rs` | 取出解释器路径（本夹具是 `/lib/ld.so`）→ 从 VFS 读入 → 当作普通共享对象装载 |
| `DT_NEEDED` | 同上 | 每个依赖名拼成 `/lib/<name>` → VFS 读入 → 装载；支持递归，`MAX_OBJECTS = 8` 挡住 `DT_NEEDED` 环 |
| `.dynamic` 解析 | 同上 | `DT_STRTAB`/`DT_SYMTAB`/`DT_SYMENT`/`DT_RELA`/`DT_RELASZ`/`DT_RELAENT`/`DT_JMPREL`/`DT_PLTRELSZ`/`DT_PLTREL`/`DT_HASH`；`DT_PLTREL == DT_REL` → **明确拒绝**（避免「看起来跑通了但重定位没做」） |
| 共享库窗口 | `syscall/address_space.rs` | `USER_DYN_BASE = USER_VIRT_BASE + 1 MiB`、32 页（128 KiB）：`map_dyn_page`（幂等、按需分配、新页全零、同一套 W^X 页权限）依次串排解释器与每个 `.so`；`fork` 复制整个窗口（`UserSpace::clone`） |
| 重定位 | `syscall/dynamic.rs` | `R_X86_64_RELATIVE`（`base + addend`）与 `GLOB_DAT`/`JUMP_SLOT`/`R_X86_64_64`（在**所有对象**的 `.dynsym` 里按名字线性扫描，命中已定义符号 → 定义对象基址 + `st_value` + addend）；未知类型跳过并留日志，解析不到符号 → `-ENOEXEC` |
| 写入路径 | `UserSpace::phys_of` | 装载发生在内核 CR3、切用户 CR3 之前，所以段数据与重定位都经「用户虚拟地址 → 物理帧」反查后写（内核恒等映射物理内存） |

地址布局（`address_space.rs` 头部注释同步更新）：

```text
0x8000000000            映像窗口（主程序 ET_DYN PIE 与静态 ET_EXEC 同址；64 KiB）
0x8000020000            brk 堆窗口（256 KiB）
0x8000060000            匿名 mmap 窗口（256 KiB）
0x8000100000            **共享库窗口**（32 页 = 128 KiB；解释器 + DT_NEEDED 依次串排）
```

主程序仍在映像窗口：`e_entry`、`argv` 栈、`/init` 等既有路径全部不变，静态与动态程序共用
同一条装载入口（只在 `e_type` 上分流），装载基址都是 `USER_VIRT_BASE`（链接基址 0 + 基址修正）。

**边界（刻意，与 `docs/architecture.md` 第 5 节一致）**：链接由**内核**做。`PT_INTERP` 指向的
解释器会被装载和重定位，但**不被进入** —— 真正的用户态 `ld.so` 需要一个能 `mmap` 文件的内核
（本版 `mmap` 只支持匿名页），留到后续版本。不支持：版本节（`.gnu.version*`）、
`DT_RPATH/RUNPATH`/`LD_LIBRARY_PATH`（固定 `/lib/<name>`）、TLS、`INIT/FINI`、`DT_REL`
（x86_64 上均为 `RELA`）、惰性绑定（所有 `JUMP_SLOT` 在装载期填充）。

夹具（`user/dynhello/`，由 binutils `as` + `ld` 在构建期现编，`kernel/build.rs::build_dynamic_fixtures`
产出到 `$OUT_DIR/dyn/` 并内嵌）：`dynhello.s` 是带 `PT_INTERP`/`DT_NEEDED`/PLT 的 PIE 主程序，
`libgreet.s` 是导出 `greet` 的共享库，`ldso.s` 是解释器占位。启动时三个工件作为**普通文件**
落进 RAMFS：`/bin/dynhello`、`/lib/ld.so`、`/lib/libgreet.so`（`fs::init` 新增 `/lib`；
程序清单 7 → 8，`PROGRAM_IMAGES` 与 `LIBRARY_IMAGES` 分开 —— 后者是「被装载的对象」，
不是「被直接运行的程序」，这一步与真实系统里「解释器与 `.so` 就是文件系统里的文件」一致）。

验收（`make test-dyn`，10 项串口断言）：

- [x] `dyn: PT_INTERP=/lib/ld.so` + `dyn: NEEDED libgreet.so -> /lib/libgreet.so`：两条路径都是内核
      从 ELF 里读出来的（不是测试脚本写死的字符串）；
- [x] `dyn: load <main> base=0x8000000000 segments=4 pages=5` 与 `dyn: load /lib/libgreet.so base=0x8000103000`：
      主程序在映像窗口、库在共享库窗口，`base` 由装载器选定（对象链接基址是 0）；
- [x] `dyn: reloc R_X86_64_JUMP_SLOT sym=greet -> obj=/lib/libgreet.so` +
      `dyn: link PASS objects=3 relocs=2 entry=0x8000001020`：3 个对象、2 条重定位真的应用了
      （另一条是 `R_X86_64_RELATIVE`）；
- [x] `main: dynamic link ok`（主程序自己的 `.rodata`）与 `greet: hello from libgreet.so`
      （共享库的 `.rodata`，在共享库窗口里）—— **两条代码路径都在 ring3 里跑了**，且库里的 `write`
      缓冲通过了内核的用户缓冲校验（`page_flags` 认这个窗口）；
- [x] `user: exit code=42`：退出码 = 共享库 `greet()` 的返回值；
      `user: dynamic link PASS (PT_INTERP + libgreet.so + RELATIVE/JUMP_SLOT)`；
      `user: frames free before=… after=…` —— 连共享库窗口一起无页帧泄漏。
- [x] **独立复核**：`make dyn-hostcheck` 用**宿主机的 glibc `ld.so`** 跑同一批 `.s`（`as` + `ld` 重编）——
      两套完全不同的装载器都要跑出相同的两行输出与 `exit(42)`，才说明 ELF 结构 / 机器码 /
      重定位语义是对的（与 `mkfs` + mtools 同一个思路；环境无 binutils/glibc 时跳过）。

### 3.10 `mprotect`（v1.0 第一步，2026-10-11 落地）

v0.9 收官时「号已定义但未实现」的调用只剩 `mprotect`（`syscall_table::UNIMPLEMENTED` 里
唯一一项，串口上表现为 `syscall: ENOSYS nr=10 name=mprotect`）。它同时是两件事的前提：
`mmap(PROT_NONE)` 的映射要能再启用，以及 W^X 下的「先写、再换只执行」。本步把它补上后
**这张表一度空了** —— 随后 CSS-1（3.11）在扩展号段放入 261/265，CSS-3（3.12）再放入 262/263/264，
如今这张桩表只剩 **266 `storage_format`**（CSS-4 才定义）。

| 项 | 实现位置 | 说明 |
| ---- | ---- | ---- |
| ABI 校验 | `syscall/memory.rs::sys_mprotect` | 页对齐、`len > 0`、未知保护位、`PROT_WRITE\|PROT_EXEC`（W^X）——每类拒绝都留一行 `user: mprotect rejected (<reason>)` |
| 页权限改写 | `syscall/address_space.rs::mprotect` | 范围必须落在**同一个窗口**（映像 / brk 堆 / 匿名 mmap / 共享库）且每页都已映射，否则 `-EINVAL`（与 `munmap` 的「整段已映射」同一约定；Linux 这里用 `ENOMEM`） |
| `PROT_NONE` 的表示 | `prot_to_flags` + `accessible` | 清 `USER` 而保留 `PRESENT`：ring3 访问 → #PF（`error=P\|U`），页帧与内容保留，内核也别想把它当用户缓冲（`page_flags` 返回 `None` → `-EFAULT`） |
| TLB | `paging::flush_tlb` | 每次改权限都 `invlpg`：不刷就可能继续用「旧的、更宽的权限」访问同一页（与 `munmap` 的幽灵映射同一个坑） |
| 用户栈 | 不支持（`-EINVAL`） | 它的保护属性（4 页 `RW` + `NX`）由 `UserSpace::new` 一次性定死；拒绝会留串口记录，不静默忽略 |

验收（`make test-user` 新增 30 项断言，总数 61 → **91**）：

- [x] `user: mprotect addr=0x8000070000 len=4096 prot=0x0 -> pages=1`：内核逐页重写页表项，
      返回被改写的页数（地址是确定的：堆自测的 mmap 已归还，全局分配器的 64 KiB 块占掉前 16 页）；
- [x] **双向证据**：`PROT_NONE` 之后既 `user: EFAULT write ptr=0x8000070000 len=1`
      （内核按页校验不再认它）也 `hello: mprotect none write ret=-14`；
- [x] `hello: mprotect data kept=1`：改回 `RW` 后内容原样（`mprotect` 不搬家、不清零）；
- [x] `hello: mprotect exec ret=42`：改成 `RX` 后**从这页真的执行**了 `mov eax,42; ret`
      ——「先写后换只执行」不再是纸面说法（在此之前 `mprotect` 落在 ENOSYS 桩上，这条路不通）；
- [x] 六类拒绝各自留痕：未页对齐 / `len=0` / 未知保护位 / `W^X`（都是 `-22`，但日志里能区分
      是哪一条）、未映射的页 `-22`、用户栈 `-22`；
- [x] **brk 堆窗口同样在支持范围内**（逐页 `heap_flags` 的意义）：`user: mprotect addr=0x8000020000 len=4096 prot=0x0 -> pages=1`
      → `hello: mprotect heap none write ret=-14` → 改回 `RW` 后 `hello: mprotect heap data kept=1`
      （堆里第一页的第一个字节是堆自测写入的模式）；
- [x] `hello: mprotect munmap ret=0`：`mprotect` 过的页帧仍在，`munmap` 照样能整段归还
      （页帧泄漏检查照旧通过）。

已知边界（写下来避免误读）：只支持上面四个窗口，**不支持** `MAP_FIXED` 式的固定地址放置
（仍无 `mmap` 的 offset/固定地址能力）、不支持把一整段匿名映射拆成不同权限的分段操作
（一次调用只针对一段、不跨窗口）。

### 3.11 存储枚举（CSS-1，2026-10-11 落地）

CSS-1（见 [storage.md](./storage.md)）在内核里立起「设备 → 分区 → 卷」的只读识别层，
用户态通过两个新号读它 —— 与 `getprocs`/`getdents` 同一套做法：**定长记录 + 一次调用写满**，
记录布局的权威定义在 `abi::volume`（阶段三起；`storage/volume.rs` 是写记录的一方），`libc` 只提供解析器。

| syscall | 号 | 实现位置 | 用户态使用者 | 说明 |
| ---- | ---- | ---- | ---- | ---- |
| `storage_volumes(buf_ptr, buf_len, out_count_ptr)` | **261** | `syscall/storage.rs::sys_storage_volumes` | `user/volumes` → `/bin/volumes` | 把卷表快照写成**定长 64 字节记录**到用户缓冲，返回**实际写入条数**；`out_count_ptr != 0` 时同刻写回**总卷数**（LE `u64`，先问大小再开缓冲）；`buf_len < 64` → `-EINVAL`；**只读查询，无需能力位** |
| `storage_rescan(disk_index)` | **265** | `syscall/storage.rs::sys_storage_rescan` | （CSS-5 管理程序） | 重扫某设备的（`0xFFFF_FFFF` = 全部）分区与文件系统，更新卷表并打印证据行；设备不存在 → `-ENODEV`；受控动作，CKM 落地后需 `CAP_STORAGE_ADMIN`（判定点 8） |

两条要点：

- **确定性顺序**（storage.md 6.5）：`disk_id` 升序 → `part_index` 升序 → `id` 字节序升序；
  因此两次 `storage_volumes` 调用逐字节相同（验收项 3）。
- **只读面无需能力位**：`storage_volumes` 像 `ckm_component_state` 的只读面一样不查能力位；
  受控的重扫才走 CKM（CSS-1 还没有 CKM，先只提供机制，权限在 CSS-3 接上）。

记录布局（64 字节，`storage/volume.rs`）：`id`（稳定标识，如 `fs:<sha>`/`pt:<disk>-<part>-<lba>`/
`raw:<disk>`）/ `disk` / `part` / `fs`（文件系统种类）/ `label` / `ro`（只读原因）/
`health`（0=ok/`degraded`/`unsupported`）/ `mounts`；`libc` 的 `volume_id()`/`volume_fs()`/`volume_health()`/`volume_mounts()` 等解析器
按固定偏移取值，让 `/bin/volumes` 不必自己算偏移（与 `dirent_*`/`procinfo_*` 同理）。

验收（`make test-storage` 26 项 + `make test-storage-mbr` 13 项 + `make test-programs`
的 `/bin/volumes` 断言 23 项 + 6 正则 + 内核解析自测 11 例 / 45 断言）：

- [x] `user: storage_volumes count=1 total=1`：`/bin/volumes` 真的把卷表读出来（GPT 两分区镜像里
      识别出 FAT 卷并列出其 `id`/`disk`/`part`/`fs`/`label`/`health`）；
- [x] 枚举确定性：同一镜像两次枚举的 `storage: vol …` 行逐字节相同；
- [x] 无分区表的镜像（`fat.img`/`disk.img` 形态）仍被识别为**整盘一卷**（`table=none partitions=1`）；
- [x] 主表头损坏 → 回退备用表头；两表 CRC 都不对 → `health=degraded`；未识别文件系统 → `health=unsupported`
      但**仍然列出**（可见但不挂载）。

### 3.12 卷管理（CSS-3，2026-10-12 落地）

CSS-3（见 [storage.md](./storage.md) 第 4/9 节）把 262/263/264 从 ENOSYS 桩上拿下，让"盘符"与
"把卷挂到路径"真的能用。三个号都是**受控动作**，走 CKM 判定点 8（`CAP_STORAGE_ADMIN`）；
参数校验（长度/保留位/指针/UTF-8）**先于**能力判定，因此 `EINVAL` 不会退化成 `EPERM`。

| syscall | 号 | 失败语义 | 行为 |
| ---- | ---- | ---- | ---- |
| `storage_mount(vol_id_ptr, vol_id_len, point_ptr, point_len, flags)` | **262** | `ENOENT` 卷不存在 / `ENOTSUP` 没后端 / `EEXIST` 挂载点重复 | 把卷挂到路径；`flags` bit0 = 只读挂载（→ `ro-mount`），其余位 `EINVAL`；已挂载的卷再加点只加**别名**（不重开设备） |
| `storage_unmount(point_ptr, point_len, 0, 0, 0)` | **263** | `ENOENT` 不是挂载点 / `EINVAL` 卸根 | 卸掉别名；卷仍在卷表（`mount_count` 减一） |
| `storage_assign_letter(vol_id_ptr, vol_id_len, letter, 0, 0)` | **264** | `EEXIST` 字母被占 / `EINVAL` 非法字母 | 分配/取消盘符（`letter = 0` 回到缺省规则）；映射写进 CKM 状态区 `storage/letters`，重启后不变 |

- 实现位置 `syscall/storage.rs`；底层落点 `fs::mount_volume_by_id`/`fs::unmount_point`（262/263）与
  `storage::registry::assign_letter` → `storage/letters::assign`（264）；
- **用户态**：`libc` 新增 `storage_mount`/`storage_unmount`/`storage_assign_letter` 包装，
  以及 `libc::path`（`is_windows`/`resolve`/`resolve_str`）把 `C:\x` 查卷表改写为 POSIX 路径；
  `/bin/volumes` 扩为 `list`/`mount`/`unmount`/`letter`/`demo` 子命令；
- 验收（`make test-storage` + `make test-programs` + `make test-ckm`）：
  - [x] `volumes demo`（Shell，uid 1000）：`storage_mount` → 用 `C:\KERNEL.ELF`（libc 重写）与
        `/volumes/<label>/KERNEL.ELF` **两个入口**读同一段字节 → `storage_unmount` → 再开返回 `ENOENT`；
  - [x] 盘符：`storage: letters table n=… first=C`、卷记录的 `letter=` 与表一致；自定义 `Z` 写盘
        （`ckm: store put key=storage/letters …`）；冲突拒绝 `storage: letters conflict rejected letter=D`；
  - [x] 权限：`/bin/hello`（uid 2000）三次存储受控动作均 `-1` + `ckm: policy op=storage_mount … need=0x10`
        （判定点 8 的拒绝三件套）。

### 3.13 Socket 与 TCP（2026-10-14 落地）

v0.9 的网络只到 UDP（`network/{tcp,socket}.rs` 是占位，见 drivers.md 1.8）；
这里补的是 **socket 可用的最小 TCP** —— 用户态 `socket` 系调用 + 内核 TCP 客户端最小集
（三次握手/按序收发/FIN 关闭，见 drivers.md 1.8 的 wire 部分）。号沿用 Linux x86_64：

| syscall | 号 | 签名 | 失败语义 |
| ---- | ---- | ---- | ---- |
| `socket(domain, ty, protocol)` | **41** | 只认 `AF_INET`(2) + `SOCK_STREAM`(1) + `IPPROTO_TCP`(6)/0 | 其余组合 `-EAFNOSUPPORT`；表满（8 个）`-ENOSPC` |
| `bind(fd, addr_ptr, 16)` | **49** | `sockaddr_in`，地址只接受全 0（bind-any），端口非 0 | `addr_len != 16` / 端口 0 / 端口被占 / 重复 bind → `-EINVAL` |
| `connect(fd, addr_ptr, 16)` | **42** | 三次握手，阻塞到建立或失败；未 `bind` 时从临时段自动分配端口 | 对端 RST → `-ECONNREFUSED`；重传耗尽 → `-ETIMEDOUT` |
| `sendto(fd, buf, len, 0, 0)` | **44** | 已连接套接字上发送（按 MSS 切块、每块等累积 ACK；返回已确认字节数，可能少于 `len`） | 未连接 `-ENOTCONN`；带地址 `-EINVAL`；超时 `-ETIMEDOUT`；对端 RST `-ECONNREFUSED`（对端半关闭仍可单向发送） |
| `recvfrom(fd, buf, len, 0, 0)` | **45** | 首段即返（`0` = 对端有序关闭）；地址回填未实现，传 0 | 未连接 `-ENOTCONN`；带地址 `-EINVAL`；超时 `-ETIMEDOUT` |

- 地址布局的权威定义在 `abi::sock`（16 字节：family LE + port BE + addr + 补 0），
  `libc::sockaddr_in` 是调用侧构造器；新增 errno（`abi::errno`，Linux 取值）：
  `-EAFNOSUPPORT`(97)/`-ENOTCONN`(107)/`-ETIMEDOUT`(110)/`-ECONNREFUSED`(111)；
- fd 号段：套接字 16..=23（`SOCK_FD_BASE`，`network::socket` 表），文件 0..=15 ——
  `read`/`write`/`close` 按号段分发到同一套后端（`read` = `recv` 首段即返，
  `write` = `send`（返回已确认字节数，可能部分发送），`close` 先 FIN 关闭再释放槽位）；
- **用户态**：`libc` 新增 `socket`/`bind`/`connect`/`sendto`/`recvfrom` + `sockaddr_in` 构造器
  （`lib.rs` re-export），新增 `/bin/tcpecho`（`user/tcpecho`）：`connect(10.0.2.2:18080)` 后做
  **三轮**收发 —— 第一轮 `sendto`/`recvfrom`，第二轮 `write`/`read`（同一连接，
  覆盖 fd 路由），第三轮 3000 字节（> MSS，切三段）用 512 字节小缓冲读
  （验证未交付字节不被 ACK 吞掉）；逐字节比对，成功 `tcpecho: PASS` + `exit(0)`；
- 内核自测（`syscall/user.rs`）：有网卡时跑 `/bin/tcpecho` 并要求 `exit(0)`
  （`user: tcpecho PASS`），无网卡时 `user: tcpecho SKIP`（不是失败）；
  套接字表随 `fs::file::reset` 一起在每次进入用户态前清空（全局表模型，见 `network/socket.rs`）；
- 验收（`make test-network` 13 → **24 项**，编排 `network` 场景同步）：
  `net: tcp connect 10.0.2.2:18080 ESTABLISHED` → `send len=12 acked` →
  `recv len=12 match=PASS` → `close FIN-ACK PASS` → `tcp PASS`（内核自测），
  再 `tcpecho: round 1 sendto/recvfrom PASS` / `round 2 write/read PASS` /
  `round 3 large payload PASS len=3000` / `tcpecho: PASS` / `user: tcpecho PASS`
  （用户态 syscall 路径 + 超 MSS 分段与小缓冲读）；
- 明确不做（见 drivers.md 1.8）：服务端（`listen`/`accept`）、UDP 套接字、ICMP、
  窗口通告与流控（恒 65535）、SACK/时间戳/窗口扩大、乱序重组、RTO 估算。

## 4. 调用链示例

实际调用链（v0.7 + v0.9 用户堆 / 动态链接 + v1.0 `mprotect`）：

```text
用户态 syscall 指令
  → syscall_entry（保存用户 RSP + rdi/rsi/rdx/r10/r8/r9，切系统调用栈，压 rcx/r11/rax）
  → syscall_dispatch（kernel/src/syscall/mod.rs）
  → syscall_table::parse（号 → handler）
  → file::sys_read/sys_write/sys_open/sys_close
       └─ fs::file（fd 表）→ fs::vfs（挂载表路由）→ fs::ramfs
  → memory::sys_mmap / sys_munmap / sys_brk（用户堆：地址空间窗口 + 页帧分配）
  → memory::sys_mprotect（已映射页改权限位 + `invlpg`；`PROT_NONE` = 清 USER）
  → process::sys_exit / sys_getpid / sys_sleep / sys_fork / sys_exec / sys_wait / sys_getprocs
       ├─ wait4 → trampoline::run（切到子进程的 CR3 / 内核栈 / 用户帧）→ 子进程 exit → 回收
       └─ getprocs → 进程表快照 → 用户缓冲（`user/ps`）
  → dir::sys_getdents
       └─ fs::vfs::list → 定长 88 字节记录 → 用户缓冲（`user/ls`）
  → elf::load 按 e_type 分流（v0.9 动态链接，3.9）
       ├─ ET_EXEC → 固定地址逐段装载（v0.7 起）
       └─ ET_DYN  → dynamic::load
            ├─ PT_INTERP=/lib/ld.so → VFS 读入 → 装进共享库窗口
            ├─ 每个 DT_NEEDED → /lib/<name> → VFS 读入 → 装进共享库窗口
            └─ R_X86_64_RELATIVE / GLOB_DAT / JUMP_SLOT / 64 → 写 GOT/数据槽
                 → 入口 = 装载基址 + e_entry（`/bin/dynhello`）
  → sysretq 回到用户态（恢复 rdi/rsi/rdx/r10/r8/r9；exit 与 ring3 异常走 leave_user 回内核）
```

v0.6 遗留的「`write` 把串口写死」已按 ADR-004 收口：fd 1/2 继续走串口（它是控制台设备），
fd ≥ 3 一律经 VFS 抽象路由，syscall 层不认识具体后端。

## 5. 非目标 / 后续

- **完整** ELF 动态链接：非第一版目标；v0.9 第六步落地的是**内核侧最小闭环**（3.9）——
  用户态 `ld.so`、版本符号、TLS、`rpath`/`LD_LIBRARY_PATH` 均不在第一版内。
- 完整 POSIX：非第一版目标。
- 完整内存保护细节（`stac/clac` 精细控制、内核高半区迁移）：在 v0.7 起记录，必要时追加 ADR。

## 6. 修订记录

| 日期 | 变更 |
| ---- | ---- |
| 2026-10-06 | 第一阶段创建占位 |
| 2026-10-07 | 补记路线细化：明确 v0.6/v0.7 的分工、syscall 列表拆分、libc 封装位置与验收占位 |
| 2026-10-08 | 本次 docs 迭代统一文档顶部元信息与修订记录 |
| 2026-10-10 | **v0.6 核心落地**：填入真实入口选型（`syscall`/`sysret`）、寄存器/错误码约定、GDT 布局约束、用户地址空间构造、ring3 异常终止语义、首批 4 个 syscall 与 `make test-user` 验收结果；补记实测坑（固件页表只读 → 惰性清 `USER` 位）与 v0.6 明确未做项 |
| 2026-10-11 | **v0.6 用户态脚手架可用**：新增 2.11（`libc/` API、`user.ld` 链接约束、large 代码模型/`user-release` profile、构建期摊平与检查项）；2.8 改为「用户程序来源」并区分 `source=libc` 与 `source=asm`；2.4 更新为多代码页映射；验收项增至 19 项 |
| 2026-10-11 | **v0.9 第一步：用户堆** —— 新增 `syscall/memory.rs`（`brk`/`mmap`/`munmap`，含 W^X、`MAP_*` 白名单、5 参数无 offset 的取舍说明与逐条错误码）+ `address_space.rs` 的堆窗口（256 KiB）与匿名 mmap 窗口（256 KiB，first-fit，含 `invlpg`）；`libc` 新增 `heap`（`#[global_allocator]`，bump over mmap）与 `brk/mmap/munmap` 封装；`user/hello` 加入堆自测（零页、模式读回、堆缓冲 `write`、6 个负例、`Vec`/`format!`）；`user.rs` 新增页帧泄漏检查（`frames free before/after` → `heap frame reclamation PASS`）；`make test-user` 断言由 32 项增至 **59 项**（`orchestrate.json` 的 `user` 场景同步），全量回归（`test-fs` 19 项 / `test-driver` 22 项 / `test-fat` 21 项 / `make go-test`）PASS |
| 2026-10-11 | **v0.9 第二步：交互式 Shell 与 stdin** —— `driver/keyboard.rs` 加 128 字节行缓冲 + 行编辑，回显协议由 `KB <字符>` 改为终端式原样回显（`Enter` 打印 `kbd: line ready len=N`，`make test-keyboard` 断言同步）；`syscall/file.rs` 新增 `read_stdin`（`read(0)` 整行交付、3 秒超时返回 `-EAGAIN`）；`libc` 新增 `EAGAIN` 与 `fork`/`execve`/`wait4` 包装；新增 `user/shell`（提示符 `root@cerlesse C:>`、内建 `help`/`echo`/`cat`/`pid`/`exit`、外部命令真的调 `fork()`），`kernel/build.rs`/`fs::init`/`user.rs` 按“两个用户程序”改造（`/init` + `/bin/sh`）；`syscall/mod.rs` 对已命名未实现的号打印 `syscall: ENOSYS nr=… name=…`；新增 `make test-shell`（12 项）与编排 `shell` 场景，`make test-user` 增至 61 项（含非交互路径的 Shell 步骤），`test-fs` 根目录条目数改为 4；新增 3.6，更新 3.2/3.3/3.4 |
| 2026-10-11 | **v0.9 第三步：`fork`/`execve`/`wait4` + `argc`/`argv`（外部程序真的能跑了）** —— 新增 3.7；`syscall/process.rs` 实现三号（fork 复制地址空间与当前用户帧、execve 换映像并重建 argv 用户栈、wait4 延迟调度 + 回收），`process/process.rs` 换成真正的 PCB（每进程 64 KiB 内核栈 + 栈底哨兵、`Ready/Running/Zombie`、`return_to`），`trampoline` 改为按进程切换（CR3/TSS.RSP0/栈顶/当前进程）；`libc` 去掉「已定号未实现」的说法，新增 `libc/src/args.rs`（`rdi`/`rsi` 入口 ABI）；新增 `user/hello-bin` → `/bin/hello` 作为外部程序；Shell 的 `cmd_external` 改走 `fork`+`execve`+`wait4`（`ls` 现在是“execve 失败 → exit(127)”）；`tests/test_keyboard.py` 新增 `--expect-re`（正则 + 反向引用，用于断言 PID 一致性）；新增 `make test-proc`（8 项 + 4 条正则）并加入 `test-all`，编排场景 11 → **12**（新增 `proc`）；`make test-shell` 12 → **11 项**（去掉两条 ENOSYS 依赖证据，改为断言外部程序路径）；`make test-user` 的 `shell PASS` 断言同步 |
| 2026-10-11 | **v0.7 落地**：第 3 节从规划改为实现记录 —— ELF Loader（解析/按段权限逐页映射/入口取 `e_entry`）、用户地址布局（映像窗口 64 KiB + 栈 4 页）、段页对齐与 `.lbss` 孤儿段的实测坑、`make user` 的独立 ELF 检查（`tools/check_user_elf.py`）；2.7 填入 `read/open/write/close`（含 fd 表与两条硬规则）；2.10 标注 v0.7 已做项；4 节更新调用链；3.4 列出仍未做项（用户堆/`argc/argv`/`fork` 系/动态链接） |
| 2026-10-11 | **v0.9 第五步：基础用户程序（`ls`/`cat`/`echo`/`ps`）与目录/进程枚举** —— 新增 3.8；新增 `syscall/dir.rs`（`getdents = 78`，定长 88 字节记录、VFS `list()` 转发、截断语义与 `-EINVAL` 边界）与 `syscall/process.rs::sys_getprocs`（**扩展号段 256**，定长 40 字节记录）；`syscall_table.rs` 的 `SYS_MAX` 256 → 257，并写明「`0..=255` = Linux 语义、`>= 256` = Cerlesse 扩展」；`process/process.rs` 新增定长 `comm: [u8; 16]`（`create` 填充、`execve` 改写为程序名），顶层程序运行前置为 `RUNNING`；`libc` 新增 `getdents`/`getprocs` 与 `dirent_*`/`procinfo_*` 解析器；新增 `user/{ls,cat,echo,ps}` 四个程序（`kernel/build.rs`/`workspace`/`fs::init` 清单 3 → 7）；Shell 的帮助文本补上四个程序并说明「内建遮蔽同名程序」；新增 `make test-programs`（17 项 + 5 条正则）与编排 `programs` 场景（场景数 **13 → 14**）；`make test-proc` 的负例由 `ls` 改为 `foo`（`/bin/ls` 已存在，不能再当“找不到的程序”） |
| 2026-10-11 | **v0.9 第六步：内核侧最小动态链接** —— 新增 3.9；`syscall/elf.rs::load` 按 `e_type` 分流（`ET_DYN` → 新增 `syscall/dynamic.rs`，`ET_EXEC` 路径不变）；`address_space.rs` 新增共享库窗口（`USER_DYN_BASE` = 基址 + 1 MiB、32 页）与 `map_dyn_page`/`dyn_page_count`/`phys_of`（内核 CR3 下写重定位），`fork` 复制窗口、`page_flags` 认可该窗口、`Drop` 整树释放；`dynamic.rs` 解析 `PT_INTERP`/`PT_DYNAMIC`/`DT_NEEDED`（`/lib/<name>` 递归装载，上限 8 个对象）并应用 `R_X86_64_RELATIVE`/`GLOB_DAT`/`JUMP_SLOT`/`R_X86_64_64`（按名字线性扫描 `.dynsym`；`DT_PLTREL=REL` 明确拒绝）；新增夹具 `user/dynhello/`（`as`+`ld` 现编，`kernel/build.rs` 内嵌）与 RAMFS `/lib`（`fs::init` 新增 `libraries` 参数，程序清单 7 → 8）；新增 `make test-dyn`（10 项）与 `make dyn-hostcheck`（宿主 glibc 独立复核），`make test-fs` 20 → 21 项、`test-programs` 的 `/` 与 `/bin` 条目数同步 5/7；3.8 的条目数断言与 3.1 的地址布局/`ET_EXEC` 说明同步更新 |
| 2026-10-11 | **v1.0 第一步：`mprotect`（运行时改保护属性）** —— 新增 3.10；`syscall/memory.rs` 新增 `sys_mprotect`（页对齐/`len>0`/未知位/`W^X` 四类 ABI 拒绝，每类留一行 `user: mprotect rejected (<reason>)`）+ `mmap(PROT_NONE)` 由拒绝改为合法；`address_space.rs` 新增 `mprotect`（范围须在同一窗口、每页已映射，否则 `-EINVAL`；`PROT_NONE` = 清 `USER` 留 `PRESENT`）、`UserWindow`、`prot_to_flags`、`accessible`（`PRESENT`+`USER` 齐备才算可访问）与 `set_page_flags`（改权限即 `invlpg`），堆窗口新增逐页 `heap_flags`（`brk` 增长/收缩与 `fork` 同步，`page_flags` 据此判断堆页）；`syscall_table` 注册 10 号、`UNIMPLEMENTED` 因此**空表**（`mod.rs` 的打印注释同步）；`libc` 新增 `mprotect`/`PROT_NONE`（`lib.rs` re-export）；`user/hello` 新增 `mprotect_selftest`（`PROT_NONE` → 内核拒收 `-EFAULT` → 内容原样 → `RX` 后真的执行机器码 → 6 个负例 → `munmap` 照常归还）；`make test-user` 61 → **91 项** PASS（含 brk 堆窗口的 `PROT_NONE` 往返）；头部状态与 2.7 表、1、3.4/3.5 的「未做」清单同步 |
| 2026-10-11 | **CSS-1：存储枚举（261 `storage_volumes` / 265 `storage_rescan`）** —— 新增 3.11；`syscall/storage.rs` 实现两个号（定长 64 字节卷记录、确定性枚举、`storage_volumes` 无需能力位、`rescan` 设备不存在 `-ENODEV`）；`syscall_table.rs` 的 `SYS_MAX` 257 → **267**，并在 CKM 预留的 257–260 之后开出 CSS 号段 261–266（261/265 已实现，262/263/264/266 落 `UNIMPLEMENTED` 桩，串口可区分「未实现」与「号越界」）；`libc` 新增 `storage_volumes`/`storage_rescan` + `volume_*` 解析器，新增用户程序 `user/volumes` → `/bin/volumes`；头部号段表、1 节分发表说明、3.4 与 3.10 的「空表」表述同步 |
| 2026-10-12 | **阶段三：系统调用 ABI 的单一事实源（`abi/` crate）** —— 新增 `abi/`（`no_std`、零依赖、`forbid(unsafe_code)`）：`syscall.rs`（号 + `SYS_MAX` + 号段规则 + `NAMED`/`UNIMPLEMENTED`）、`errno.rs`、`flags.rs`（`O_*`/`PROT_*`/`MAP_*`/fd 编号）、`limits.rs`（`PATH_MAX`/`IO_MAX`/`ARG_*`）、`dirent.rs`/`procinfo.rs`/`volume.rs`/`ckm.rs`（记录布局、能力位、op 编号、受保护前缀）与 `record.rs`（编译期断言）；内核 18 个文件（`syscall/*` 9 + `fs/file.rs` + `security/capability.rs` + `ckm/{mod,audit,state,policy}.rs` + `storage/volume.rs` 等）与 `libc/src/syscall.rs` 的 45 个常量改为引用它（**对外名字与类型不变**，行为零变化）；新增 `make abi-check` 并入 `make check`。相关段落（3.8 的解析偏移、3.11 的记录布局）已改为指向 `abi` |
| 2026-10-14 | **Socket/TCP 补齐（41/42/44/45/49 + 内核 TCP 客户端最小集 + `/bin/tcpecho`）** —— 新增 3.13；`abi/` 新增 `sock.rs`（`AF_INET`/`SOCK_STREAM`/`IPPROTO_TCP` + `sockaddr_in` 布局）与 `NAMED` 26 → **31**（`SYS_MAX` 不变，号落在 Linux 段内），`errno.rs` 新增 `-EAFNOSUPPORT`(97)/`-ENOTCONN`(107)/`-ETIMEDOUT`(110)/`-ECONNREFUSED`(111)；`network/tcp.rs` 重写为真实实现（三次握手 + 按 MSS 切块发送 + 累积 ACK + 重传 + 按序接收 + RST/FIN + `echo` 验收函数），`network/socket.rs` 重写为套接字表（fd 16..=23，`create`/`bind`/`connect`/`send`/`recv`/`close`），`network/arp.rs` 新增无缓存 `resolve` 自由函数（`Stack::arp_resolve` 改为调它，只留缓存查/填）；新增 `syscall/net.rs`（五个 handler + `read`/`write`/`close` 的套接字 fd 路由），`syscall_table.rs` 注册五号，`syscall/file.rs` 按号段分发，`syscall/user.rs` 有网卡时跑 `/bin/tcpecho`（`REQUIRED_PROGRAM_PATHS` 8 → 9，无网卡 `SKIP`）；`libc` 新增封装 + `sockaddr_in` 构造器（`lib.rs` re-export），新增 `user/tcpecho`（manifest/清单 8 → 9，`/bin` 条目数 8 → **9**）；`tests/test_boot.py` 新增 `--tcp-echo-port`（宿主回声服务），`make test-network` 13 → **23 项**，编排 `network` 场景同步；回归 `test-user`（96）/`test-programs`（33+7）/`test-driver`（23）/`test-fs`（21）/`check-boundary`/`go-test` 全 PASS；头部状态与 1、2.7、3.4 的计数同步 |
| 2026-10-12 | **CSS-3：卷管理（262 `storage_mount` / 263 `storage_unmount` / 264 `storage_assign_letter`）** —— 新增 3.12；`syscall/storage.rs` 实现三个号（参数校验先于能力判定；对应 `fs::mount_volume_by_id`/`fs::unmount_point` 与 `storage::registry::assign_letter`）；`libc` 新增三个包装与 `path` 模块（`C:\x` → POSIX）、`/bin/volumes` 扩为 `list`/`mount`/`unmount`/`letter`/`demo`；头部号段表改为「只余 266 `storage_format` 落 ENOSYS 桩」，3.10 的桩表描述同步；验收在 `make test-storage`（盘符断言）/`test-programs`（`volumes demo` 双入口）/`test-ckm`（零能力拒绝） |
