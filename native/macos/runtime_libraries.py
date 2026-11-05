"""Copy Mach-O runtime libraries and replace external load paths before signing."""
import os
from pathlib import Path
import re
import shutil
import subprocess


def output(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True)


def resolve_library(name, source, executable, inherited):
    def expand(value):
        return value.replace("@loader_path", str(source.parent)).replace("@executable_path", str(executable.parent))
    if name.startswith("@rpath/"):
        load = output("/usr/bin/otool", "-l", source)
        rpaths = re.findall(r"cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset", load)
        for base in [*map(expand, rpaths), *inherited]:
            candidate = Path(base) / name.removeprefix("@rpath/")
            if candidate.is_file():
                return candidate.resolve()
        raise ValueError(f"Unresolved runtime library: {name} from {source}")
    path = Path(expand(name))
    if not path.is_absolute() or not path.is_file():
        raise ValueError(f"Unresolved runtime library: {name} from {source}")
    return path.resolve()


def library_info(source):
    arch = output("/usr/bin/lipo", "-archs", source).strip()
    load = output("/usr/bin/otool", "-l", source)
    versions = re.findall(r"minos ([\d.]+)", output("/usr/bin/vtool", "-show-build", source))
    if not versions:
        versions = re.findall(r"cmd LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version ([\d.]+)", load)
    if not versions:
        raise ValueError(f"Cannot determine minimum macOS version: {source}")
    minimum = max(tuple(map(int, v.split("."))) for v in versions)
    rpaths = re.findall(r"cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset", load)
    identity = output("/usr/bin/otool", "-D", source).splitlines()[1:]
    names = [line.strip().split(" (compatibility", 1)[0] for line in output("/usr/bin/otool", "-L", source).splitlines()[1:]]
    libraries = [name for name in names if name not in identity and not name.startswith(("/usr/lib/", "/System/Library/"))]
    return arch, minimum, rpaths, libraries


def copy_library(contents, library):
    destination = contents / "Frameworks" / library.name
    shutil.copy2(library, destination)
    destination.chmod(0o755)
    subprocess.run(["/usr/bin/install_name_tool", "-id", f"@rpath/{library.name}", str(destination)], check=True)
    # Homebrew and many standalone runtime SDKs ship notices at the prefix root.
    for notice in library.parent.parent.glob("LICENSE*"):
        notices = contents / "Resources/Runtime Licenses" / library.parent.parent.name
        notices.mkdir(parents=True, exist_ok=True)
        shutil.copy2(notice, notices / notice.name)


def bundle_libraries(contents, origins):
    frameworks = contents / "Frameworks"
    frameworks.mkdir()
    copied = {}
    minimum = (11, 0)
    architecture = None
    queue = [(target, source.resolve(), source.resolve(), []) for target, source in origins.items()]
    while queue:
        target, source, executable, inherited = queue.pop(0)
        arch, required_os, rpaths, libraries = library_info(source)
        if architecture is None:
            architecture = arch
        if arch != architecture:
            raise ValueError(f"Architecture mismatch: {source}: {arch}, expected {architecture}")
        minimum = max(minimum, required_os)
        inherited = [p.replace("@loader_path", str(source.parent)).replace("@executable_path", str(executable.parent)) for p in rpaths] + inherited
        for name in libraries:
            library = resolve_library(name, source, executable, inherited)
            destination = frameworks / library.name
            if library.name in copied and copied[library.name] != library:
                raise ValueError(f"Runtime library filename collision: {library.name}")
            if library.name not in copied:
                copied[library.name] = library
                copy_library(contents, library)
                queue.append((destination, library, executable, inherited))
            relative = os.path.relpath(destination, target.parent)
            subprocess.run(["/usr/bin/install_name_tool", "-change", name, f"@loader_path/{relative}", str(target)], check=True)
        for rpath in rpaths:
            if rpath.startswith("/") and not rpath.startswith(("/usr/lib/", "/System/Library/")):
                subprocess.run(["/usr/bin/install_name_tool", "-delete_rpath", rpath, str(target)], check=True)
    return ".".join(map(str, minimum))
