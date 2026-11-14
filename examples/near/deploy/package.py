#!/usr/bin/env python3
"""Package trusted Linux x86_64 build artifacts without a server-side toolchain."""

import argparse
import hashlib
import os
import platform
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path


CONTRACTS = ("gnorix_near_wallet.wasm", "gnorix_near_token.wasm")
WASM_HEADER = b"\0asm\x01\0\0\0"
LOADER = "ld-linux-x86-64.so.2"


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def regular_file(path):
    path = path.resolve(strict=True)
    if not path.is_file():
        raise ValueError(f"Expected a regular file: {path}")
    return path


def interpreter(path):
    """Read PT_INTERP directly, keeping the build host's exact dynamic loader."""
    with path.open("rb") as binary:
        header = binary.read(64)
        if len(header) != 64 or header[:6] != b"\x7fELF\x02\x01":
            raise ValueError(f"Expected a little-endian ELF64 executable: {path}")
        fields = struct.unpack("<16sHHIQQQIHHHHHH", header)
        if fields[1] not in (2, 3) or fields[2] != 62 or fields[9] < 56:
            raise ValueError(f"Expected a Linux x86_64 executable: {path}")
        for index in range(fields[10]):
            binary.seek(fields[5] + index * fields[9])
            program = struct.unpack("<IIQQQQQQ", binary.read(56))
            if program[0] != 3:
                continue
            if not 1 < program[5] <= 4096:
                break
            binary.seek(program[2])
            value = binary.read(program[5])
            if value.endswith(b"\0"):
                loader = Path(value[:-1].decode())
                if loader.is_absolute() and loader.name == LOADER:
                    return loader
            break
    raise ValueError(f"No supported dynamic loader in {path}")


def dependencies(binary):
    result = subprocess.run(
        ["ldd", str(binary)],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={**os.environ, "LC_ALL": "C"},
    )
    libraries = {}
    for line in result.stdout.splitlines():
        line = line.strip()
        if not line or line.startswith("linux-vdso.so."):
            continue
        if "=> not found" in line:
            raise ValueError(f"Unresolved library for {binary}: {line}")
        mapped = re.fullmatch(r"(\S+)\s+=>\s+(.+?)\s+\(0x[0-9a-fA-F]+\)", line)
        direct = re.fullmatch(r"(/.+?)\s+\(0x[0-9a-fA-F]+\)", line)
        if mapped:
            name, source = mapped.groups()
        elif direct:
            source = direct.group(1)
            name = Path(source).name
        else:
            raise ValueError(f"Unrecognized ldd output for {binary}: {line}")
        name = Path(name).name
        source = regular_file(Path(source))
        if name in libraries and digest(libraries[name]) != digest(source):
            raise ValueError(f"Different libraries share the name {name}")
        libraries[name] = source
    if not libraries:
        raise ValueError(f"No dynamic dependencies found for {binary}")
    return libraries


def validate_inputs(binary, sandbox, contracts):
    binary = regular_file(binary)
    sandbox = regular_file(sandbox)
    loader = regular_file(interpreter(binary))
    if interpreter(sandbox) != Path("/lib64") / LOADER:
        raise ValueError("The sandbox must use the standard Ubuntu x86_64 loader")
    version = subprocess.run(
        [str(sandbox), "--version"], check=True, capture_output=True, text=True
    ).stdout
    if "(release 2.14.1)" not in version or "sandbox" not in version:
        raise ValueError("Expected the official NEAR sandbox 2.14.1 executable")
    libraries = dependencies(binary)
    if LOADER in libraries and digest(libraries[LOADER]) != digest(loader):
        raise ValueError("The app's interpreter differs from its resolved loader")
    libraries[LOADER] = loader
    sandbox_libraries = dependencies(sandbox)
    for name in CONTRACTS:
        artifact = regular_file(contracts / name)
        with artifact.open("rb") as source:
            if source.read(8) != WASM_HEADER:
                raise ValueError(f"Expected a WebAssembly v1 contract: {artifact}")
    return binary, sandbox, libraries, sandbox_libraries


def copy_file(source, destination, executable=False):
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if executable else 0o644)


def write_release(directory, binary, sandbox, libraries, contracts):
    for name in ("bin", "lib", "contracts"):
        (directory / name).mkdir()
    copy_file(binary, directory / "bin/gnorix-near-demo", executable=True)
    copy_file(sandbox, directory / "bin/near-sandbox", executable=True)
    for name, source in libraries.items():
        copy_file(source, directory / "lib" / name, executable=name == LOADER)
    for name in CONTRACTS:
        copy_file(contracts / name, directory / "contracts" / name)
    launcher = directory / "run"
    launcher.write_text(
        '#!/bin/sh\n'
        'set -eu\n'
        'release_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)\n'
        'export GNORIX_DEMO_CONTRACTS_DIR="$release_dir/contracts"\n'
        'export NEAR_SANDBOX_BIN_PATH="$release_dir/bin/near-sandbox"\n'
        'exec "$release_dir/lib/ld-linux-x86-64.so.2" \\\n'
        '    --library-path "$release_dir/lib" "$release_dir/bin/gnorix-near-demo" "$@"\n'
    )
    launcher.chmod(0o755)
    files = sorted(path for path in directory.rglob("*") if path.is_file())
    (directory / "SHA256SUMS").write_text(
        "".join(f"{digest(path)}  {path.relative_to(directory).as_posix()}\n" for path in files)
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--sandbox", required=True, type=Path)
    parser.add_argument("--contracts", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if sys.platform != "linux" or platform.machine() != "x86_64":
        parser.error("Package this release on Linux x86_64")
    output = args.output.absolute()
    if output.is_symlink() or (output.exists() and (not output.is_dir() or any(output.iterdir()))):
        parser.error("--output must be absent or an empty directory")
    try:
        binary, sandbox, libraries, sandbox_libraries = validate_inputs(
            args.binary, args.sandbox, args.contracts
        )
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=".gnorix-release-", dir=output.parent) as temporary:
            directory = Path(temporary) / "release"
            directory.mkdir()
            write_release(directory, binary, sandbox, libraries, args.contracts)
            os.replace(directory, output)
    except (OSError, ValueError, struct.error, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Packaging failed: {error}\n")
    print(f"Release prepared: {output}")
    print("Sandbox uses Ubuntu host libraries: " + ", ".join(sorted(sandbox_libraries)))


if __name__ == "__main__":
    main()
