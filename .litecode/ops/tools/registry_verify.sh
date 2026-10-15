#!/usr/bin/env bash
# LiteCode ops: verify (and optionally repair) the ops cargo registry against cache tarballs + Cargo.lock checksums.
# Usage: registry_verify.sh [--fix] [--quick]   (v21 --quick: stat-only vs cached tarball file lists, prints damaged names, exit 1 if any; no g++ check)
# (old usage) registry_verify.sh [--fix] [--lock /workspace/litecode/Cargo.lock]
# Read-only by default. --fix re-extracts damaged crates (rm dir, tar x from cache, write .cargo-ok {"v":1}).
# Never compiles. Exit 0 = clean (after fix), 1 = problems remain.
echo "LITECODE_OPS tools_version=$(node -e 'console.log(require(process.argv[1]).tools_version)' "$(cd "$(dirname "$0")" && pwd)/registry.json") tool=ops.registry_verify"
# v20: g++ check. System g++ present -> ok. Else ensure user-space g++ (.toolchain/gxx-root + gxx-bin) from cached debs in .toolchain/debs; --fix re-extracts offline.
TC=/workspace/litecode/.litecode/ops/.toolchain; GXX_RC=0
QUICK=0; for a in "$@"; do [ "$a" = "--quick" ] && QUICK=1; done
FIXF=0; for a in "$@"; do [ "$a" = "--fix" ] && FIXF=1; done
gxx_ok() { [ -x "$TC/gxx-bin/g++" ] && sh -c 'd=$(mktemp -d); printf "#include <string>\nint main(){std::string s=\"a\"; s+=\"b\"; return s.size()==2?0:1;}\n" > $d/t.cpp; "$0" $d/t.cpp -o $d/t 2>/dev/null && $d/t; r=$?; rm -rf $d; exit $r' "$TC/gxx-bin/g++"; }
if [ "$QUICK" = 1 ]; then :
elif [ -z "${LC_FORCE_USERSPACE_GXX:-}" ] && command -v g++ >/dev/null 2>&1; then echo "gxx: system g++ present ($(g++ --version | head -1))"
else
  if [ "$FIXF" = 1 ] && ! gxx_ok; then
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
args=sys.argv[1:]; fix='--fix' in args; quick='--quick' in args
lock='/workspace/litecode/Cargo.lock'
if '--lock' in args: lock=args[args.index('--lock')+1]
reg=os.path.join(os.environ.get('CARGO_HOME','/workspace/litecode/.litecode/ops/.toolchain/cargo'),'registry')
sums={}
txt=open(lock).read()
for blk in txt.split('[[package]]'):
    n=re.search(r'^name = "(.+)"',blk,re.M); v=re.search(r'^version = "(.+)"',blk,re.M); c=re.search(r'^checksum = "(.+)"',blk,re.M)
    if n and v and c: sums[f"{n.group(1)}-{v.group(1)}"]=c.group(1)
mpath=os.path.join(reg,'.verify_manifest.json')
try: manifest=json.load(open(mpath))
except Exception: manifest={}
mdirty=False
def expected(idx,d,crate):
    global mdirty
    k=idx+'/'+d
    if k not in manifest:
        with tarfile.open(crate) as t: manifest[k]=[[m.name.split('/',1)[1] if '/' in m.name else m.name,m.size] for m in t if m.isfile()]
        mdirty=True
    return manifest[k]
st=dict(src_checked=0,src_damaged=0,src_fixed=0,cache_checked=0,cache_bad=0,src_without_cache=[],lock_missing_cache=[])
bad_cache=[]; damaged=[]
def sha(p,b=1<<20):
    h=hashlib.sha256()
    with open(p,'rb') as f:
        for ch in iter(lambda:f.read(b),b''): h.update(ch)
    return h.hexdigest()
for idx in sorted(os.listdir(os.path.join(reg,'cache'))):
    if quick: pass
    cdir=os.path.join(reg,'cache',idx); sdir=os.path.join(reg,'src',idx)
    for f in sorted(os.listdir(cdir)):
        if not f.endswith('.crate'): continue
        k=f[:-6]
        if k in sums and not quick:
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
        if quick:  # v21: stat-only vs tarball file list (cached manifest): existence + size of every file; no hashing
            for rel,size in expected(idx,d,crate):
                fp=os.path.join(p,rel)
                try:
                    if os.stat(fp).st_size!=size: probs.append('size '+rel)
                except OSError: probs.append('missing '+rel)
                if len(probs)>5: break
            if not os.path.exists(os.path.join(p,'.cargo-ok')): probs.append('no .cargo-ok')
            if probs: damaged.append((idx,d,'; '.join(probs[:5])))
            continue
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
if mdirty:
    try: json.dump(manifest,open(mpath+'.tmp','w')); os.replace(mpath+'.tmp',mpath)
    except Exception: pass
st['src_damaged']=len(damaged)
for idx,d,why in damaged:
    print(f"DAMAGED {idx}/{d}: {why}")
    if fix and d not in bad_cache:
        sdir=os.path.join(reg,'src',idx); shutil.rmtree(os.path.join(sdir,d))
        with tarfile.open(os.path.join(reg,'cache',idx,d+'.crate')) as t: t.extractall(sdir, **({'filter':'tar'} if hasattr(tarfile,'tar_filter') else {}))
        open(os.path.join(sdir,d,'.cargo-ok'),'w').write('{"v":1}'); st['src_fixed']+=1
if quick and damaged: print('QUICK_DAMAGE '+str(len(damaged))+' crate(s): '+', '.join(d for _,d,_ in damaged[:6]))
for k in bad_cache: print(f"BAD_TARBALL {k} (delete it and run cargo fetch)")
print(json.dumps(st))
sys.exit(0 if (st['src_damaged']==st['src_fixed'] and not bad_cache) else 1)
PY
PYRC=$?
[ $PYRC -ne 0 ] && exit $PYRC
exit $GXX_RC
