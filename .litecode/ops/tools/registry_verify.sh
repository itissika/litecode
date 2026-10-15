#!/usr/bin/env bash
# LiteCode ops: verify (and optionally repair) the ops cargo registry against cache tarballs + Cargo.lock checksums.
# Usage: registry_verify.sh [--fix] [--lock /workspace/litecode/Cargo.lock]
# Read-only by default. --fix re-extracts damaged crates (rm dir, tar x from cache, write .cargo-ok {"v":1}).
# Never compiles. Exit 0 = clean (after fix), 1 = problems remain.
echo "LITECODE_OPS tools_version=$(node -e 'console.log(require(process.argv[1]).tools_version)' "$(dirname "$0")/registry.json") tool=ops.registry_verify"
# v20: g++ check. System g++ present -> ok. Else ensure user-space g++ (.toolchain/gxx-root + gxx-bin) from cached debs in .toolchain/debs; --fix re-extracts offline.
TC=/workspace/litecode/.litecode/ops/.toolchain; GXX_RC=0
gxx_ok() { PATH="$TC/gxx-bin:$PATH" sh -c 'd=$(mktemp -d); printf "#include <string>\nint main(){std::string s=\"a\"; s+=\"b\"; return s.size()==2?0:1;}\n" > $d/t.cpp; g++ $d/t.cpp -o $d/t 2>/dev/null && $d/t; r=$?; rm -rf $d; exit $r'; }
if [ -z "$LC_FORCE_USERSPACE_GXX" ] && command -v g++ >/dev/null 2>&1; then echo "gxx: system g++ present ($(g++ --version | head -1))"
else
  if [ "$1" = "--fix" ] && ! gxx_ok; then
    if ls "$TC"/debs/*.deb >/dev/null 2>&1; then
      rm -rf "$TC/gxx-root"; mkdir -p "$TC/gxx-root" "$TC/gxx-bin"; for d in "$TC"/debs/*.deb; do dpkg -x "$d" "$TC/gxx-root"; done
      printf '%s\n' '#!/bin/sh' 'R=/workspace/litecode/.litecode/ops/.toolchain/gxx-root; V=14; T=x86_64-linux-gnu' 'exec "$R/usr/bin/$T-g++-$V" -B"$R/usr/libexec/gcc/$T/$V/" -B/usr/lib/gcc/$T/$V/ -isystem "$R/usr/include/c++/$V" -isystem "$R/usr/include/$T/c++/$V" -L"$R/usr/lib/gcc/$T/$V" "$@"' > "$TC/gxx-bin/g++"
      chmod 755 "$TC/gxx-bin/g++"; ln -sf g++ "$TC/gxx-bin/c++"; echo "gxx: re-extracted user-space g++ from cached debs"
    else echo "gxx: no cached debs in $TC/debs (apt-get download g++ g++-14 g++-14-x86-64-linux-gnu cpp-14 libstdc++-14-dev)"; fi
  fi
  if gxx_ok; then echo "gxx: user-space g++ OK (gxx-bin)"; else echo "GXX_MISSING: no working g++ (run with --fix; needs cached debs)"; GXX_RC=1; fi
fi
python3 - "$@" <<'PY'
import sys,os,tarfile,hashlib,shutil,json,re
args=sys.argv[1:]; fix='--fix' in args
lock='/workspace/litecode/Cargo.lock'
if '--lock' in args: lock=args[args.index('--lock')+1]
reg=os.path.join(os.environ.get('CARGO_HOME','/workspace/litecode/.litecode/ops/.toolchain/cargo'),'registry')
sums={}
txt=open(lock).read()
for blk in txt.split('[[package]]'):
    n=re.search(r'^name = "(.+)"',blk,re.M); v=re.search(r'^version = "(.+)"',blk,re.M); c=re.search(r'^checksum = "(.+)"',blk,re.M)
    if n and v and c: sums[f"{n.group(1)}-{v.group(1)}"]=c.group(1)
st=dict(src_checked=0,src_damaged=0,src_fixed=0,cache_checked=0,cache_bad=0,src_without_cache=[],lock_missing_cache=[])
bad_cache=[]; damaged=[]
def sha(p,b=1<<20):
    h=hashlib.sha256()
    with open(p,'rb') as f:
        for ch in iter(lambda:f.read(b),b''): h.update(ch)
    return h.hexdigest()
for idx in sorted(os.listdir(os.path.join(reg,'cache'))):
    cdir=os.path.join(reg,'cache',idx); sdir=os.path.join(reg,'src',idx)
    for f in sorted(os.listdir(cdir)):
        if not f.endswith('.crate'): continue
        k=f[:-6]
        if k in sums:
            st['cache_checked']+=1
            if sha(os.path.join(cdir,f))!=sums[k]: st['cache_bad']+=1; bad_cache.append(k)
    for k in sums:
        if not os.path.exists(os.path.join(cdir,k+'.crate')): st['lock_missing_cache'].append(k)
    if not os.path.isdir(sdir): continue
    for d in sorted(os.listdir(sdir)):
        p=os.path.join(sdir,d)
        if not os.path.isdir(p): continue
        st['src_checked']+=1
        crate=os.path.join(cdir,d+'.crate')
        if not os.path.exists(crate): st['src_without_cache'].append(d); continue
        if d in bad_cache: damaged.append((idx,d,'bad tarball')); continue
        probs=[]
        try:
            ok=open(os.path.join(p,'.cargo-ok')).read().strip()
            if json.loads(ok)!={"v":1}: probs.append('cargo-ok content')
        except Exception: probs.append('no .cargo-ok')
        with tarfile.open(crate) as t:
            for m in t:
                if not m.isfile(): continue
                rel=m.name.split('/',1)[1] if '/' in m.name else m.name
                fp=os.path.join(p,rel)
                if not os.path.isfile(fp): probs.append('missing '+rel)
                elif os.path.getsize(fp)!=m.size: probs.append('size '+rel)
                elif hashlib.sha256(t.extractfile(m).read()).digest()!=hashlib.sha256(open(fp,'rb').read()).digest(): probs.append('content '+rel)
                if len(probs)>5: break
        if probs: damaged.append((idx,d,'; '.join(probs[:5])))
st['src_damaged']=len(damaged)
for idx,d,why in damaged:
    print(f"DAMAGED {idx}/{d}: {why}")
    if fix and d not in bad_cache:
        sdir=os.path.join(reg,'src',idx); shutil.rmtree(os.path.join(sdir,d))
        with tarfile.open(os.path.join(reg,'cache',idx,d+'.crate')) as t: t.extractall(sdir, **({'filter':'tar'} if hasattr(tarfile,'tar_filter') else {}))
        open(os.path.join(sdir,d,'.cargo-ok'),'w').write('{"v":1}'); st['src_fixed']+=1
for k in bad_cache: print(f"BAD_TARBALL {k} (delete it and run cargo fetch)")
print(json.dumps(st))
sys.exit(0 if (st['src_damaged']==st['src_fixed'] and not bad_cache) else 1)
PY
PYRC=$?
[ $PYRC -ne 0 ] && exit $PYRC
exit $GXX_RC
