#!/usr/bin/env python3
"""QEMU 启动集成测试（v0.1 实现，v0.2 支持多断言与异常注入镜像）。

启动 QEMU（OVMF + 磁盘镜像），收集串口输出，断言所有 --expect 字符串出现。
超时后杀掉 QEMU，以断言结果决定成败。

并可通过 ``--expect-re`` 追加**正则**断言（CSS-2 起；实现与 `test_keyboard.py` 一致）：
在整份日志上做 ``re.search``（``re.DOTALL``），因此可以用反向引用把同一份日志里
**先捕获的值**与后面的输出对齐。存储验收需要它，因为启动盘的卷序列号（因此 `vol_id`）
取决于镜像构建时间 —— 断言应当绑定**一致性**（同一个卷在多条证据行里是同一个 id），
而不是某个常数：

    --expect-re "storage: mount point=/disk vol=(fs:[0-9a-f]+).*?storage: unmount .*? vol=\\1 mounts="

用法：
  python3 tests/test_boot.py                          # 默认：disk.img，断言 "Kernel started!"
  python3 tests/test_boot.py --disk build/disk-exception.img \
      --expect "Kernel started!" --expect "EXCEPTION: divide error"
"""

import argparse
import re
import shutil
import socket
import subprocess
import sys
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "build"
OVMF_CODE = Path("/usr/share/OVMF/OVMF_CODE.fd")
OVMF_VARS_SRC = Path("/usr/share/OVMF/OVMF_VARS.fd")
TIMEOUT_SECONDS = 40


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Cerlesse QEMU serial-log test")
    parser.add_argument("--disk", default=str(BUILD / "disk.img"), help="磁盘镜像路径")
    parser.add_argument(
        "--disk2",
        default=None,
        help="第二块 virtio-blk 盘（v0.8 test-fat：mkfs 生成的 FAT 测试盘，内核挂到 /mnt/fat）",
    )
    parser.add_argument(
        "--extra-disk",
        action="append",
        default=[],
        help="再追加 virtio-blk 盘（可重复；CSS-1 的 GPT/MBR 夹具紧跟 --disk2 之后注册，"
        "因此块设备号从 2 开始按给出顺序递增）",
    )
    parser.add_argument(
        "--expect",
        action="append",
        default=None,
        help="串口必须出现的字符串（可重复）；默认断言 'Kernel started!'",
    )
    parser.add_argument(
        "--expect-re",
        action="append",
        default=None,
        help="串口日志必须匹配的正则（可重复；整份日志上 re.search，DOTALL；可用 \\1 反向引用前面的捕获组）",
    )
    parser.add_argument("--serial-log", default=None, help="串口日志输出路径")
    parser.add_argument(
        "--ovmf-vars",
        default=None,
        help="OVMF_VARS 副本路径（并行编排时按场景隔离，默认 build/OVMF_VARS.fd）",
    )
    parser.add_argument(
        "--nic",
        action="store_true",
        help="v0.9 test-network：加一块 legacy virtio-net-pci 网卡（QEMU user/slirp 网络）",
    )
    parser.add_argument(
        "--tcp-echo-port",
        type=int,
        default=None,
        help="TCP 验收的宿主侧一半：在宿主 0.0.0.0 上起回声服务（发什么回什么），"
        "内核经 slirp 网关 10.0.2.2 连它（端口须与 kernel/src/network/mod.rs::TCP_ECHO_PORT 一致）",
    )
    parser.add_argument(
        "--ahci",
        action="store_true",
        help="v1.0 test-ahci：把一块盘挂到 q35 内建的 ICH9 AHCI 控制器（00:1f.2）的 2 号端口上；"
        "内核的 SATA 驱动（kernel/src/driver/ahci.rs）会认领它不是 virtio-blk",
    )
    parser.add_argument(
        "--ahci-disk",
        default=str(BUILD / "fat.img"),
        help="挂到 AHCI 端口上的盘（默认 build/fat.img：同一张 FAT32 镜像因此可以由 virtio 与 SATA "
        "两条完全不同的驱动路径读，文件内容互相印证）",
    )
    parser.add_argument(
        "--persist",
        action="store_true",
        help="blk0 **不加** `snapshot=on`：写真的落盘（CKM-2 的持久化自测需要它，"
        "见 tests/test_ckm_store.py）。只能配一块**独占的镜像副本**用 —— 并行编排的"
        "场景共享 build/disk.img，在那里开写会互相踩。",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=TIMEOUT_SECONDS,
        help="QEMU 观察窗口秒数（默认 40）",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    disk = Path(args.disk)
    disk2 = Path(args.disk2) if args.disk2 else None
    extra = [Path(p) for p in args.extra_disk]
    ahci_disk = Path(args.ahci_disk) if args.ahci else None
    expects = [s.encode() for s in (args.expect or ["Kernel started!"])]
    regexes = args.expect_re or []
    serial_log = Path(args.serial_log) if args.serial_log else BUILD / "serial.log"
    timeout = args.timeout

    if shutil.which("qemu-system-x86_64") is None:
        print("FAIL: missing tool qemu-system-x86_64")
        return 2
    if not disk.is_file():
        print(f"FAIL: disk image not found: {disk} (run `make image` first)")
        return 2
    if disk2 is not None and not disk2.is_file():
        print(f"FAIL: second disk image not found: {disk2} (run `make fat-image` first)")
        return 2
    for path in extra:
        if not path.is_file():
            print(f"FAIL: extra disk image not found: {path} (run `make storage-images` first)")
            return 2
    if ahci_disk is not None and not ahci_disk.is_file():
        print(f"FAIL: AHCI disk image not found: {ahci_disk} (run `make fat-image` first)")
        return 2

    BUILD.mkdir(parents=True, exist_ok=True)
    vars_copy = Path(args.ovmf_vars) if args.ovmf_vars else BUILD / "OVMF_VARS.fd"
    vars_copy.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(OVMF_VARS_SRC, vars_copy)

    cmd = [
        "qemu-system-x86_64",
        "-machine", "q35",
        "-m", "512M",
        "-drive", f"if=pflash,format=raw,readonly=on,file={OVMF_CODE}",
        "-drive", f"if=pflash,format=raw,file={vars_copy}",
        # snapshot=on：磁盘以临时写时复制方式打开，不占用镜像写锁，
        # 多个并行场景可共享同一磁盘镜像（H-04 编排器），且不污染原始镜像。
        #
        # v0.8：启动盘挂在 **virtio-blk（legacy）** 上，因为内核里的块驱动就驱动它
        #（kernel/src/driver/virtio_blk.rs）——「镜像能启动」与「驱动能读写这块盘」
        # 因此是同一台设备上的同一条路径。disable-modern=on 强制设备暴露 legacy
        # 接口（vendor 1af4 / device 0x1001 / BAR0 是 I/O 窗口）。
        "-drive",
        f"if=none,id=blk0,format=raw,file={disk}{'' if args.persist else ',snapshot=on'}",
        "-device", "virtio-blk-pci,drive=blk0,disable-modern=on",
        "-serial", "stdio",
        "-display", "none",
        "-no-reboot",
    ]

    if disk2 is not None:
        # 第二块盘：排在 blk0 之后 → PCI 上就是下一个 function，块设备号 = 1。
        # 内核按「探测顺序」注册设备，因此 blk0 仍是 `/disk`（启动盘）、blk1 是 `/mnt/fat`。
        cmd[cmd.index("-serial"):cmd.index("-serial")] = [
            "-drive", f"if=none,id=blk1,format=raw,file={disk2},snapshot=on",
            "-device", "virtio-blk-pci,drive=blk1,disable-modern=on",
        ]

    # CSS-1：从 blk2 起的盘按给出顺序追加（块设备号 = 2、3、……），用于 GPT/MBR 夹具。
    for index, path in enumerate(extra, start=2):
        cmd[cmd.index("-serial"):cmd.index("-serial")] = [
            "-drive", f"if=none,id=blk{index},format=raw,file={path},snapshot=on",
            "-device", f"virtio-blk-pci,drive=blk{index},disable-modern=on",
        ]

    if ahci_disk is not None:
        # v1.0：q35 的内建芯片组本身就带一个 ICH9 AHCI 控制器（00:1f.2 / 8086:2922 /
        # class 0x01 subclass 0x06 prog_if 0x01），因此不需要再加控制器，只要把一块盘
        # 插到它的端口上。q35 的 SATA 端口在 QEMU 里叫 `ide.0`..`ide.5`（不是 `sata.N`），
        # 2 号端口与固件引导日志里的 `Pci(0x1F,0x2)/Sata(0x2,...)` 是同一个端口。
        # 盘仍然用 snapshot=on：测试不污染共享的 fat.img。
        cmd[cmd.index("-serial"):cmd.index("-serial")] = [
            "-drive", f"if=none,id=ahci0,format=raw,file={ahci_disk},snapshot=on",
            "-device", "ide-hd,drive=ahci0,bus=ide.2",
        ]

    if args.nic:
        # v0.9：网络验收用 `-netdev user`（slirp）—— 它自带 DHCP 服务器与虚拟网关，
        # 因此 ARP + DHCP 的 UDP 往返不需要主机侧跑任何服务。disable-modern=on
        # 强制 legacy 接口（vendor 1af4 / device 0x1000），内核的 virtio-net 驱动只认它。
        cmd[cmd.index("-serial"):cmd.index("-serial")] = [
            "-netdev", "user,id=n0",
            "-device", "virtio-net-pci,netdev=n0,disable-modern=on",
        ]

    # TCP 回声服务（若要求）：QEMU 起之前先在宿主监听，客户机的两次连接
    # （内核自测 + /bin/tcpecho）都会打到它；QEMU 结束即停。
    echo_stop = threading.Event()
    echo_thread = None
    if args.tcp_echo_port is not None:
        echo_thread = threading.Thread(
            target=_run_echo_server, args=(args.tcp_echo_port, echo_stop), daemon=True
        )
        echo_thread.start()

    print(f"running: {' '.join(cmd)}")
    timed_out = False
    try:
        result = subprocess.run(
            cmd, capture_output=True, timeout=timeout, check=False
        )
        output = result.stdout + result.stderr
    except subprocess.TimeoutExpired as expired:
        timed_out = True
        output = (expired.stdout or b"") + (expired.stderr or b"")
    finally:
        echo_stop.set()
        if echo_thread is not None:
            echo_thread.join(timeout=5)

    serial_log.write_bytes(output)

    missing = [exp for exp in expects if exp not in output]

    # 正则断言：在整份日志上 re.search（DOTALL），既能跨行，也能用反向引用
    text = output.decode("utf-8", errors="replace")
    missing_re: list[str] = []
    for pattern in regexes:
        try:
            if re.search(pattern, text, re.DOTALL) is None:
                missing_re.append(pattern)
        except re.error as err:  # 正则本身写错也是测试失败，不能静默通过
            missing_re.append(f"{pattern} (invalid regex: {err})")

    if not missing and not missing_re:
        print(
            f"PASS: all {len(expects)} expectation(s) and {len(regexes)} regex(es) "
            f"found on serial (log: {serial_log})"
        )
        return 0

    for exp in missing:
        print(f"FAIL: {exp!r} not found on serial (log: {serial_log})")
    for pattern in missing_re:
        print(f"FAIL: /{pattern}/ did not match serial (log: {serial_log})")
    if timed_out:
        print(f"  (qemu was killed after {timeout}s)")
    tail = output[-4000:]
    print("--- serial tail ---")
    sys.stdout.buffer.write(tail)
    sys.stdout.buffer.flush()
    print()
    return 1


def _run_echo_server(port: int, stop: threading.Event) -> None:
    """TCP 回声服务（test-network 的宿主侧一半：发什么回什么，直到对端关闭）。

    内核侧是两次独立连接（`net: tcp` 自测 + `/bin/tcpecho`），因此这里循环 accept
    直到 QEMU 结束；单个连接内把收到的一字不差送回去（分段回声由内核侧拼合）。
    """
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        srv.bind(("0.0.0.0", port))
    except OSError as err:
        print(f"FAIL: tcp echo server cannot bind 0.0.0.0:{port}: {err}")
        return
    srv.listen(8)
    srv.settimeout(0.5)
    while not stop.is_set():
        try:
            conn, _ = srv.accept()
        except socket.timeout:
            continue
        except OSError:
            break
        try:
            conn.settimeout(5.0)
            while True:
                data = conn.recv(4096)
                if not data:
                    break
                conn.sendall(data)
        except OSError:
            pass
        finally:
            try:
                conn.close()
            except OSError:
                pass
    srv.close()


if __name__ == "__main__":
    sys.exit(main())
