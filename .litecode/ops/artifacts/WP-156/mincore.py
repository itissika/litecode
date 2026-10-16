#!/usr/bin/env python3
"""Page-cache eviction / residency check without root (WP-154 shared).
usage: python3 mincore.py check|evict [--verify] PATH...   (dirs are walked recursively)
evict: fsync (flush dirty pages) then posix_fadvise(DONTNEED), then re-read residency via mincore.
--verify: exit 1 if any page is still resident after the command.
Output: JSON {"files": N, "resident_pages": R, "total_pages": T, "still_resident": {path: [res, pages]}}"""
import ctypes, json, os, sys
libc = ctypes.CDLL("libc.so.6", use_errno=True)
libc.mmap.restype = ctypes.c_void_p
libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_long]
libc.munmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
libc.mincore.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
PG = os.sysconf("SC_PAGE_SIZE")
def resident(path):
    size = os.path.getsize(path)
    if size == 0: return [0, 0]
    fd = os.open(path, os.O_RDONLY)
    try:
        addr = libc.mmap(None, size, 1, 1, fd, 0)  # PROT_READ, MAP_SHARED: does not fault pages in
        if addr in (None, ctypes.c_void_p(-1).value): return [-1, 0]
        pages = (size + PG - 1) // PG
        vec = (ctypes.c_ubyte * pages)()
        r = libc.mincore(addr, size, vec)
        libc.munmap(addr, size)
        return [sum(v & 1 for v in vec) if r == 0 else -1, pages]
    finally: os.close(fd)
def evict(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        try: os.fsync(fd)          # dirty pages cannot be dropped until written back
        except OSError: pass
        os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
    finally: os.close(fd)
def walk(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _, names in os.walk(p):
                for n in names:
                    f = os.path.join(root, n)
                    if os.path.isfile(f) and not os.path.islink(f): yield f
        elif os.path.isfile(p): yield p
if __name__ == "__main__":
    args = sys.argv[1:]; cmd = args.pop(0); verify = "--verify" in args
    args = [a for a in args if a != "--verify"]
    files = list(walk(args)); res = tot = 0; left = {}
    for f in files:
        try:
            if cmd == "evict": evict(f)
            r, n = resident(f)
        except OSError: continue
        res += max(r, 0); tot += n
        if r: left[f] = [r, n]
    print(json.dumps({"files": len(files), "resident_pages": res, "total_pages": tot, "still_resident": left}))
    sys.exit(1 if verify and res else 0)
