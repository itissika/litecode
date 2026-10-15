import json,glob,statistics as st,os,re
R='/workspace/litecode/.tmp/wp154/raw/w4'
def med(x):
    x=[v for v in x if v is not None]; return round(st.median(x),1) if x else None
def f(v): return '–' if v is None else (f'{v:.1f}' if isinstance(v,float) else str(v))
def kind(a):
    if 'code-search-worker' in a: return 'cs-worker'
    if a.startswith('git'): return 'git'
    if 'rust-analyzer' in a: return 'rust-analyzer'
    if '/cargo' in a or a.startswith('cargo'): return 'cargo(RA)'
    if 'npm' in a: return 'npm'
    return a.split(' ')[0].split('/')[-1] or '(no args)'
out=[]; summ=[]
SC=['cold_noindex','cold_noindex_tc','cold_index','cold_index_tc','hot_reconnect','hot_reload']
for c in ['small','big']:
  for s in SC:
    recs=[json.loads(open(p).read().strip().splitlines()[-1]) for p in sorted(glob.glob(f'{R}/{c}/{s}/r*/rec.jsonl'))]
    if not recs: continue
    out.append(f'\n### {c} / {s}  (n={len(recs)})\n')
    cols=['READY(spawn→)','spawn','READY','WS open','hello','session/new sent','t_new (reply)','cs Warm','peak RSS MB']
    hot=s.startswith('hot')
    if hot: cols+=['H: index.html','H: WS open','H: hello','H: session/list sent','H: t_list','H: subscribe']
    rows=[]
    for r in recs:
        q=r['req'][0]
        row=[r['ready']-r['spawn'],r['spawn'],r['ready'],r['ws_open'],r['hello'],q['sent'],r['t_new'],r['cs_warm'],r['peak_rss_kb']/1024]
        if hot:
            h=r['hot']; hq={x['method']:x for x in h['req']}
            row+=[(h.get('reload') or {}).get('index_html'),h['ws_open'],h['hello'],hq.get('session/list',{}).get('sent'),h['t_list'],h.get('t_subscribe')]
        rows.append(row)
    out.append('| run | '+' | '.join(cols)+' |'); out.append('|'+'---|'*(len(cols)+1))
    for i,row in enumerate(rows): out.append(f'| r{i+1} | '+' | '.join(f(v) for v in row)+' |')
    m=[med([row[j] for row in rows]) for j in range(len(cols))]
    out.append('| **median** | '+' | '.join(f(v) for v in m)+' |')
    # child processes
    pk={}
    for r in recs:
        for p in r['proc']: pk.setdefault(kind(p['args']),[]).append(p['first_seen'])
    out.append('\nchild processes (first_seen ms rel. T0, all runs): '+'; '.join(f'{k}: '+', '.join(f(v) for v in vs) for k,vs in pk.items()))
    errs=[x for r in recs for x in r['req']+(r.get('hot') or {}).get('req',[]) if x['error']]
    wd=[r['watchdog'] for r in recs if r['watchdog']]; lo=[r['leftover'] for r in recs if r['leftover']]
    if errs or wd or lo: out.append(f'\n**anomalies:** errors={errs} watchdog={wd} leftover={lo}')
    if s.endswith('_tc'):
        ev=[]
        for ri,r in enumerate(recs):
            e=r['evict']; a=e['after']; b=e['before']
            ev.append(f"r{ri+1}: before {b.get('resident_pages')}/{b.get('total_pages')} pages resident over {b.get('files')} files → after {a.get('resident_pages')} pages; still resident: "+', '.join(f"{os.path.basename(k)} {v[0]}" for k,v in (a.get('still_resident') or {}).items())+f"; verified={e['verified']} own_files_evicted={e['verified_own_files']}")
        out.append('\nmincore: \n'+'\n'.join('- '+x for x in ev))
    summ.append((c,s,m,cols))
# harness
hout=[]
for c in ['small','big']:
  for mde in ['serve_order','decompose']:
    hs=[json.loads(open(p).read()) for p in sorted(glob.glob(f'{R}/harness/{c}_{mde}_r*.json'))]
    hout.append(f'\n### harness {c} / {mde} (n={len(hs)})\n\n| step | source | r1 | r2 | r3 | median ms |\n|---|---|---|---|---|---|')
    for k,stp in enumerate(hs[0]['steps']):
        v=[h['steps'][k]['ms'] for h in hs]
        hout.append(f"| {stp['step']} | `{stp['src']}` | "+' | '.join(f'{x:.2f}' for x in v)+f' | {st.median(v):.2f} |')
    if mde=='serve_order':
        v=[h['steps'][-1]['start_ms']+h['steps'][-1]['ms'] for h in hs]; w=[h['cs_warm_ms'] for h in hs]
        hout.append(f"| _sum to bind end_ | | "+' | '.join(f'{x:.2f}' for x in v)+f' | {st.median(v):.2f} |')
        hout.append(f"| _code_search Warm (bg)_ | `src/engines/code_search_engine.rs:374` | "+' | '.join(f'{x:.0f}' for x in w)+f' | {st.median(w):.0f} |')
    else: hout.append(f"\nsession_count: {[h['session_count'] for h in hs]}")
json.dump([(c,s,m,cols) for c,s,m,cols in summ],open('/tmp/w4summ.json','w'))
open('/tmp/w4_tables.md','w').write('\n'.join(out)); open('/tmp/w4_harness.md','w').write('\n'.join(hout))
