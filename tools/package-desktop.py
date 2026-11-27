"""Package the built ra2-rs executable for Finder or the Linux desktop."""
import argparse
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess


NAME = "ra2-rs"
ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "assets" / "icons"


def macos_bundle(binary):
    bundle = binary.parent / f"{NAME}.app"
    contents = bundle / "Contents"
    executables = contents / "MacOS"
    resources = contents / "Resources"
    version = subprocess.check_output([binary, "--version"], text=True).strip().split()[1]
    release = version.split("-", 1)[0].split("+", 1)[0]
    candidate = re.fullmatch(r"[0-9.]+-rc([1-9][0-9]*)(?:\+.*)?", version)
    build = f"{release}fc{candidate[1]}" if candidate else release
    executables.mkdir(parents=True, exist_ok=True)
    resources.mkdir(parents=True, exist_ok=True)
    shutil.copy2(binary, executables / NAME)
    shutil.copyfile(ICONS / f"{NAME}.icns", resources / f"{NAME}.icns")
    with (contents / "Info.plist").open("wb") as output:
        plistlib.dump({
            "CFBundleExecutable": NAME,
            "CFBundleIconFile": f"{NAME}.icns",
            "CFBundleIdentifier": "cy.hexor.ra2-rs",
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleName": NAME,
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": release,
            "CFBundleVersion": build,
            "NSHighResolutionCapable": True,
            "NSLocalNetworkUsageDescription": "Find nearby multiplayer games and connect to other players.",
        }, output)
    subprocess.run(["codesign", "--force", "--sign", "-", bundle], check=True)
    return bundle


def linux_launcher(binary):
    data = Path(os.environ.get("XDG_DATA_HOME", ""))
    if not data.is_absolute():
        data = Path.home() / ".local/share"
    desktop = data / "applications" / f"{NAME}.desktop"
    desktop.parent.mkdir(parents=True, exist_ok=True)
    theme = data / "icons/hicolor"
    for size in (48, 256):
        icon = theme / f"{size}x{size}/apps" / f"{NAME}.png"
        icon.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ICONS / f"{NAME}-{size}.png", icon)
    os.utime(theme, None)
    # The desktop entry reader unescapes the string before parsing the quoted Exec argument.
    quoted = str(binary).translate(str.maketrans({
        "\\": "\\\\", '"': '\\"', "$": "\\$", "`": "\\`", "%": "%%",
    }))
    argument = quoted.translate(str.maketrans({
        "\\": "\\\\", "\n": "\\n", "\r": "\\r", "\t": "\\t",
    }))
    desktop.write_text(
        f"[Desktop Entry]\nType=Application\nName={NAME}\n"
        f'Exec="{argument}"\nIcon={NAME}\nStartupWMClass={NAME}\n'
        "Categories=Game;\nTerminal=false\n",
        encoding="utf-8",
    )
    return desktop


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=["macos", "linux"])
    parser.add_argument("binary", nargs="?", type=Path, default=ROOT / "target/release" / NAME)
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error(f"build ra2-rs with cargo build --release --locked or pass its executable path: {binary}")
    print(macos_bundle(binary) if args.platform == "macos" else linux_launcher(binary))
