#!/usr/bin/env python3
"""Conservative candidate ledger; never declares a full product bundle cleared."""
from functools import lru_cache
from pathlib import Path
import re
import shutil
from common import ROOT, DYAD_PIN, GateError, read_json, sha256, write_json

DYAD = ROOT / 'vendor/dyad'


@lru_cache(maxsize=None)
def inspect(path: Path):
    text = path.read_text(errors='replace')
    imports = set(re.findall(r'(?:from\s*|import\s*\(|require\s*\(|import\s*|@import\s*|@plugin\s*)[\'\"]([^\'\"]+)[\'\"]', text))
    assets = re.findall(r'url\([\'\"]?([^\)\'\"]+)', text)
    dynamic = bool(re.search(r'import\(\s*[^\s\'\"]', text))
    return imports, assets, dynamic


def resolve_import(origin: Path, spec: str):
    stem = (DYAD / 'src' / spec[2:] if spec.startswith('@/') else origin.parent / spec).resolve()
    candidates = [stem, *[Path(str(stem)+suffix) for suffix in ['.ts','.tsx','.js','.css']], stem/'index.ts', stem/'index.tsx']
    found = next((p.resolve() for p in candidates if p.is_file()), None)
    if found is not None and not found.is_relative_to(DYAD):
        raise GateError(f'Import escapes inspection checkout: {spec}')
    return found


def closure(start: Path):
    queue=[start.resolve()]; seen=set(); restricted=[]; external=set(); unresolved=[]; assets=[]
    while queue:
        path=queue.pop().resolve()
        rel=path.relative_to(DYAD).as_posix()
        if rel in seen: continue
        seen.add(rel)
        if rel.startswith('src/pro/'):
            restricted.append(rel); continue
        if path.suffix not in ['.ts','.tsx','.js','.css']: continue
        imports, urls, dynamic = inspect(path)
        for spec in imports:
            if spec.startswith(('@/', '.')):
                target=resolve_import(path,spec)
                if target is None: unresolved.append({'source':rel,'specifier':spec})
                else: queue.append(target)
            else:
                external.add('/'.join(spec.split('/')[:2]) if spec.startswith('@') else spec.split('/')[0])
        assets.extend({'source':rel,'url':url} for url in urls)
        if dynamic: unresolved.append({'source':rel,'specifier':'computed import requires compiler/manual review'})
    return {'source_files':[{'path':p,'sha256':sha256(DYAD/p)} for p in sorted(seen)],
            'restricted_dependencies':sorted(restricted),'external_packages':sorted(external),
            'unresolved':unresolved,'asset_urls':assets}


def main():
    destination=ROOT/'fixtures/provenance'; destination.mkdir(parents=True,exist_ok=True)
    entries=['src/styles/globals.css','src/lib/utils.ts']
    entries += ['src/components/ui/'+name+'.tsx' for name in ['button','badge','card','input','separator','skeleton','dialog','dropdown-menu','popover','tooltip','select','sidebar']]
    entries += ['src/components/preview_panel/'+name+'.tsx' for name in ['FileTree','FileEditor','FileDiffEditor','PreviewPanel','PreviewToolbar','Console','Problems','PreviewLoadingScreen','PreviewErrorBanner','PreviewIframe']]
    entries += ['src/components/chat/'+name+'.tsx' for name in ['MessagesList','ChatInput','ChatHeader','ChatTabs','TerminalPanel','DyadSearchReplace']]
    entries += ['src/pages/chat.tsx','src/pages/home.tsx','src/pages/settings.tsx','src/components/ModelPicker.tsx','src/components/ProviderSettings.tsx','src/contexts/ThemeContext.tsx','src/app/TitleBar.tsx']
    rows=[]
    for entry in entries:
        path=DYAD/entry
        if not path.is_file(): raise GateError(f'Missing Dyad candidate {entry}')
        deps=closure(path)
        rows.append({'source':entry,'upstream':'https://github.com/dyad-sh/dyad','commit':DYAD_PIN,
                     'sha256':sha256(path),'license':'Apache-2.0 source outside src/pro; dependencies separately governed',
                     'destination':'product/apps/desktop/src/'+entry.removeprefix('src/'),
                     'notices':['fixtures/provenance/DYAD-LICENSE','fixtures/provenance/DYAD-NOTICE'],
                     'changes':'Not copied. Planned decoupling of host/domain/asset dependencies.',
                     'disposition':'excluded-as-is: restricted closure' if deps['restricted_dependencies'] else 'candidate: P04 compiler/bundle/asset review required',
                     'closure':deps})
    for name in ['LICENSE','NOTICE']: shutil.copyfile(DYAD/name,destination/('DYAD-'+name))
    shutil.copyfile(ROOT/'reference/ag1-executable/LICENSE',destination/'AG1-LICENSE')
    packages=read_json(destination/'package-evidence.json')
    for package in packages:
        for file in package['license_files']:
            if sha256(ROOT/file['path']) != file['sha256']: raise GateError('License text changed')
    write_json(destination/'dyad-extraction-ledger.json',{'version':1,
               'method':'Normalized conservative static graph, including type/barrel imports and CSS URLs. Not compiler/bundle clearance. No product code copied.',
               'entries':rows})
    print(f'{len(rows)} candidate roots; {sum(bool(r["closure"]["restricted_dependencies"]) for r in rows)} restricted dependency closures; {len(packages)} exact license packages.')


if __name__ == '__main__': main()
