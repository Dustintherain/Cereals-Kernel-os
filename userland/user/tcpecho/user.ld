/* Cerlesse 用户程序链接脚本（/bin/tcpecho 用；与 user/hello-bin/user.ld 逐条相同）
 *
 * 四条约束，改错任一条都会在链接期或 ring3 里立刻炸：
 *
 * 1. 装载地址 = 0x0000_0080_0000_0000（PML4 下标 1，与内核
 *    `kernel/src/syscall/address_space.rs::USER_VIRT_BASE` 必须一致）。
 *    内核不做加载时重定位，链接地址必须等于运行地址；
 * 2. 每个段 **页对齐**（`ALIGN(0x1000)`）：v0.7 的 ELF Loader 按页设置权限
 *    （`p_flags` → `NO_EXEC`/`WRITABLE`），同页既 W 又 X 会让 W^X 退化，
 *    装载器会直接拒绝（`user: ELF rejected: W and X segments share a page`）；
 * 3. `.text.start` 排在映像最前（`_start` 就在这个段里，见 libc 的 `global_asm!`）；
 * 4. 只保留可装载段：`.eh_frame`/`.comment` 之类丢掉，避免把映像撑大。
 *    `.data`/`.bss` 现在**允许存在**：v0.7 的 ELF Loader 把 `RW` 段映射成
 *    ring3 可写（v0.6 里写静态数据会 #PF）。
 *
 * 注意：链接地址在高半区之外（> 2GiB）但离 0 很远，默认 small 代码模型会
 * 产生 R_X86_64_32S 重定位而链接失败，因此用户程序必须用 large 代码模型构建：
 *   RUSTFLAGS="-C relocation-model=static -C code-model=large"
 * 该标志由 `kernel/build.rs`（内核自动重建用户程序）与 `Makefile` 的
 * `user` 目标成对维护，改一处要同步另一处。
 *
 * 另外 large 代码模型与 LTO 不兼容（实测 rust-lld: failed to load bitcode），
 * 所以用户程序用 workspace 的 `user-release` profile（inherits = "dev"，
 * lto = false，opt-level = 2），不要用 `--release`。
 */
ENTRY(_start)

SECTIONS
{
    . = 0x8000000000;

    /* `R E` 段：入口在最前 */
    .text : ALIGN(0x1000) {
        KEEP(*(.text.start))
        *(.text .text.*)
        *(.ltext .ltext.*)
    }

    /* `R` 段：只读数据（不可执行） */
    .rodata : ALIGN(0x1000) {
        *(.rodata .rodata.*)
        *(.lrodata .lrodata.*)
    }

    /* `RW` 段：可写数据（不可执行） */
    .data : ALIGN(0x1000) {
        *(.data .data.*)
        *(.ldata .ldata.*)
        *(.data.rel.ro .data.rel.ro.*)
        *(.got .got.*)
    }

    /* `RW` 段的 NOBITS 尾巴：静态变量的零初始化部分。
     *
     * `.lbss.*` 必须显式列上：链接在 2^39（large 代码模型）时，rustc 把零初始化的
     * 静态量放进按名字命名的 `.lbss.<symbol>` 段（而不是 `.bss`）。漏掉它们的结果不是
     * 链接报错，而是它们变成**孤儿段**被 lld 塞在 `.data` 之后（实测落在
     * 0x8000003040），于是 RW 段不再页对齐、ELF Loader 直接拒绝。 */
    .bss : ALIGN(0x1000) {
        *(COMMON)
        *(.bss .bss.*)
        *(.lbss .lbss.*)
    }

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.comment)
        *(.note .note.*)
    }
}
