#!/usr/bin/env python3
"""Generate a local directory-source `vendor/` for the Tauri shell, reusing the
main repo's vendor.py logic (symlink deref + .cargo-checksum.json).

A directory source does NO tar unpack and NO mtime stamping, so it sidesteps the
kernel LSM that blocks `futimens` (cargo registry unpack's mtime step).
All .crate files are already present in the USTC registry cache.
"""
import os, tarfile, hashlib, json, re, shutil

REG = os.path.expanduser("~/.cargo/registry")
H = os.listdir(os.path.join(REG, "src"))[0]
CACHE = os.path.join(REG, "cache", H)
LOCK  = os.path.join(os.path.dirname(os.path.abspath(__file__)), "Cargo.lock")
VENDOR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "vendor")
os.makedirs(VENDOR, exist_ok=True)

lock = open(LOCK).read()
pkgs = []
for blk in re.findall(r'\[\[package\]\]\n(.*?)(?=\n\[\[package\]|\Z)', lock, re.S):
    n = re.search(r'name = "([^"]+)"', blk)
    v = re.search(r'version = "([^"]+)"', blk)
    reg = re.search(r'source = "registry', blk)
    path = re.search(r'path = "', blk)
    if n and v and reg and not path:
        pkgs.append((n.group(1), v.group(1)))
pkgs = sorted(set(pkgs))
print("registry pkgs to vendor:", len(pkgs))


def unpack_crate(cf, dest, top):
    t = tarfile.open(cf, "r:gz")
    p2 = top + "/"
    for m in t.getmembers():
        rel = m.name[len(p2):] if m.name.startswith(p2) else None
        if rel in (None, ""):
            continue
        tgt = os.path.join(dest, rel)
        os.makedirs(os.path.dirname(tgt), exist_ok=True)
        if m.issym():
            link_abs = os.path.normpath(os.path.join(dest, m.linkname))
            try:
                if os.path.exists(link_abs):
                    with open(tgt, "wb") as out:
                        out.write(open(link_abs, "rb").read())
                else:
                    open(tgt, "w").close()
            except Exception:
                open(tgt, "w").close()
        elif m.isdir():
            os.makedirs(tgt, exist_ok=True)
        else:
            with open(tgt, "wb") as out:
                out.write(t.extractfile(m).read())


done, missing = 0, []
for n, v in pkgs:
    cf = os.path.join(CACHE, f"{n}-{v}.crate")
    if not os.path.exists(cf):
        missing.append(f"{n}-{v}")
        continue
    top = f"{n}-{v}"
    dest = os.path.join(VENDOR, top)
    if os.path.exists(os.path.join(dest, "Cargo.toml")):
        done += 1
        continue
    if os.path.isdir(dest):
        shutil.rmtree(dest)
    unpack_crate(cf, dest, top)
    files = {}
    for root, _, fs in os.walk(dest):
        for f in fs:
            fp = os.path.join(root, f)
            rel = os.path.relpath(fp, dest)
            files[rel] = hashlib.sha256(open(fp, "rb").read()).hexdigest()
    meta = {"files": files, "package": hashlib.sha256(open(cf, "rb").read()).hexdigest()}
    with open(os.path.join(dest, ".cargo-checksum.json"), "w") as f:
        json.dump(meta, f)
    done += 1

print(f"vendored/kept: {done}, missing: {len(missing)}")
for m in missing:
    print("  MISSING", m)
print("total vendor dirs:", len([d for d in os.listdir(VENDOR) if os.path.isdir(os.path.join(VENDOR, d))]))
