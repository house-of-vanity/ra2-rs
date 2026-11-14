#!/usr/bin/env python3
"""Optional developer audit: run native RA2 mounting/impact math in a disposable TCG VM.

Requires gcc/binutils for i386 and qemu-system-i386. No Windows/Wine, game launch,
network, disks or extracted assets are used by the VM. Never a runtime dependency.
"""
import argparse
import hashlib
import re
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

EXPECTED_SHA256 = "bc0f6833ece8c47e8625e5b12940d2509540cb393566577504b50b2318351a4f"
SOURCE = Path(__file__).resolve().parent


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, default=SOURCE.parents[2] / "RobotArena2/Robot Arena 2.exe")
    parser.add_argument("--qemu", default=shutil.which("qemu-system-i386"))
    args = parser.parse_args()
    if not args.qemu:
        parser.error("qemu-system-i386 is required for this optional audit; pass --qemu PATH")
    data = args.exe.read_bytes()
    if hashlib.sha256(data).hexdigest() != EXPECTED_SHA256:
        parser.error("unrecognised EXE: addresses in this research probe are version-specific")
    pe = struct.unpack_from("<I", data, 60)[0]
    count = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    with tempfile.TemporaryDirectory(prefix="ra2-native-math-") as folder:
        work = Path(folder)
        sections = []
        linker = ["ENTRY(_start)", "SECTIONS { . = 0x100000;",
                  ".multiboot : { *(.multiboot) } .text : { *(.text*) }",
                  ".rodata : { *(.rodata*) } .data : { *(.data*) } .bss : { *(.bss*) }"]
        for i in range(count):
            header = pe + 24 + optional_size + i * 40
            virtual_size, address, size, offset = struct.unpack_from("<IIII", data, header + 8)
            (work / f"pe-{i}.bin").write_bytes(data[offset:offset + size] + bytes(max(0, virtual_size - size)))
            sections.append(f'.section .pe{i},"awx"\n.incbin "pe-{i}.bin"')
            linker.append(f". = {0x400000 + address:#x}; .pe{i} : {{ *(.pe{i}) }}")
        sections.append('.section .note.GNU-stack,"",@progbits')
        (work / "pe.S").write_text("\n".join(sections) + "\n")
        (work / "probe.ld").write_text("\n".join(linker) + "\n}\n")

        def command(argv):
            subprocess.run(argv, cwd=work, check=True)

        command(["as", "--32", str(SOURCE / "boot.S"), "-o", "boot.o"])
        command(["as", "--32", "pe.S", "-o", "pe.o"])
        command(["gcc", "-m32", "-O1", "-ffreestanding", "-nostdlib", "-fno-pic", "-fno-pie",
                 "-fno-stack-protector", "-c", str(SOURCE / "probe.c"), "-o", "probe.o"])
        command(["ld", "-m", "elf_i386", "--no-warn-rwx-segments", "-T", "probe.ld", "boot.o", "probe.o", "pe.o", "-o", "probe.elf"])
        result = subprocess.run([
            args.qemu, "-accel", "tcg", "-m", "32", "-kernel", "probe.elf", "-nographic",
            "-nodefaults", "-serial", "stdio", "-monitor", "none", "-no-reboot",
            "-device", "isa-debug-exit,iobase=0xf4,iosize=4",
        ], cwd=work, text=True, capture_output=True, timeout=10)
        output = result.stdout
        if result.returncode != 33 or "MATH END" not in output:
            raise RuntimeError(f"native math probe failed ({result.returncode}):\n{output}\n{result.stderr}")
        output = output[output.index("MATH START"):]
        if "bf2cbcb1 3f164d52 bdfc46fc bedc1db5" not in output:
            raise RuntimeError(f"native result differs from recorded fixture:\n{output}")
        for fixture in (
            "3f32b51f 3de6b432 bf32b51e bde6b432",
            "bee20c91 be11e8f3 3f620c91 3d91e8f3",
            "3e16a1cd 3f13ff57 3f1fbf32 bf01361c",
            "BURST LIMITS\n00000000 \nc019999a",
            "BURST LIMITS\n00000000 \n4019999a",
        ):
            if fixture not in output:
                raise RuntimeError(f"native hinge/stroke result differs from fixture:\n{output}")
        # Recorded outputs from the unmodified original 0x45a5a0 arithmetic.
        impacts = re.findall(r"IMPACT [^\n]+\n([0-9a-f]{8})", output)
        expected = ["4131c71d", "4231c71d", "42b1c71d", "42186186", "4231c71d",
                    "43480000", "42a00000", "41f62763", "41029cbc", "41c80000"]
        if impacts != expected:
            raise RuntimeError(f"native impact results differ from fixtures: {impacts}")
        print(f"EXE SHA-256: {EXPECTED_SHA256}\n{output}")


if __name__ == "__main__":
    run()
