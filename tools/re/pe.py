"""Shared helpers for offline analysis of PathOfExileSteam.exe (run with Tools/pyenv)."""
import pefile

DEFAULT_EXE = "GameFiles/PathOfExileSteam.exe"


class Image:
    def __init__(self, path=DEFAULT_EXE):
        self.pe = pefile.PE(path, fast_load=True)
        self.base = self.pe.OPTIONAL_HEADER.ImageBase
        self.data = self.pe.get_memory_mapped_image()
        self.sections = {s.Name.rstrip(b"\0").decode(): s for s in self.pe.sections}

    def section(self, name):
        s = self.sections[name]
        start = s.VirtualAddress
        return start, self.data[start : start + s.Misc_VirtualSize]

    def va(self, rva):
        return self.base + rva

    def rva(self, va):
        return va - self.base

    def read(self, va, n):
        r = self.rva(va)
        return self.data[r : r + n]

    def u32(self, va):
        return int.from_bytes(self.read(va, 4), "little")

    def u64(self, va):
        return int.from_bytes(self.read(va, 8), "little")
