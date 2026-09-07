"""Inventory an explicitly selected legacy bundle read-only into new private evidence."""
from __future__ import annotations
import argparse
import collections
import json
import os
from pathlib import Path
from common import ROOT, GateError, sha256, verify_original
from runtime_gate import preservation
from phase_gate import verify_dependencies
from p13_prepare_runtime import private_json


def inventory(workspace, source, selected):
    workspace=workspace.resolve(strict=True);source=source.resolve(strict=True)
    files=[];journals={};counts=collections.Counter();custom=collections.Counter()
    paths=sorted(source.rglob('*'))
    for path in paths:
        if path.is_symlink():raise GateError('Legacy source links require explicit closure mapping')
        if path.is_dir():continue
        if not path.is_file():raise GateError('Legacy source contains non-file state')
        relative=path.relative_to(source);parts=relative.parts
        size=path.stat().st_size
        if size>256*1024*1024:raise GateError('Legacy file exceeds importer bound')
        row={'path':relative.as_posix(),'sha256':sha256(path),'bytes':size,'kind':'extra','session':None,'origin':None}
        if len(parts)==1 and path.suffix=='.jsonl':
            with path.open() as stream:
                header=json.loads(stream.readline(4*1024*1024))
                if header.get('type')!='session' or Path(header.get('cwd','/')).resolve()!=workspace:
                    raise GateError('Source root contains a journal from another workspace')
                session=header['id'];journals[session]={'path':relative.as_posix(),'sha256':row['sha256'],'timestamp':header.get('timestamp')}
                for line in stream:
                    if line.strip():
                        value=json.loads(line);counts[value.get('type','unknown')]+=1
                        if value.get('type')=='custom':custom[value.get('customType','unknown')]+=1
            row.update(kind='journal',session=session)
        elif len(parts)>=3 and parts[0]=='artifacts':
            row.update(session=parts[1],origin=parts[1])
            if path.name=='cursor-key-v1':row['kind']='cursor_key'
            elif len(parts)==4 and parts[2]=='records' and path.suffix=='.json':
                value=json.loads(path.read_text())
                if value.get('sessionId')!=parts[1]:raise GateError('Forked artifact origin needs explicit closure mapping')
                row['kind']='artifact'
            elif len(parts)==4 and parts[2]=='blobs':row['kind']='blob'
            elif path.name=='leases.json':row['kind']='leases'
            else:row['kind']='metadata'
        files.append(row)
    if selected not in journals:raise GateError('Selected session is absent from this workspace bundle')
    if any(row['session'] and row['session'] not in journals for row in files):raise GateError('Missing journal for artifact custody')
    if not files or len(files)>100000 or sum(f['bytes'] for f in files)>4*1024**3:raise GateError('Legacy bundle exceeds importer bounds')
    for row in files:
        if sha256(source/row['path'])!=row['sha256']:raise GateError('Legacy source changed during inventory')
    return {'version':1,'frozen':True,'files':files}, {'workspace':str(workspace),'source':str(source),'selected_session':selected,
        'journals':journals,'file_count':len(files),'bytes':sum(f['bytes'] for f in files),'record_types':dict(counts),
        'custom_types':dict(custom),'file_kinds':dict(collections.Counter(f['kind'] for f in files)),
        'original_data_writes':0,'authority_registered':False,'model_actions':0}


def run(workspace,source,selected,output):
    verify_dependencies('P13');pins=verify_original();before=preservation();os.umask(0o077)
    output=output.absolute()
    if output.exists() or not output.parent.resolve().is_relative_to((ROOT/'.local').resolve()):raise GateError('Use a new private output directory under .local')
    manifest,report=inventory(workspace,source,selected)
    output.mkdir(mode=0o700)
    reference=private_json(output/'manifest.json',manifest)
    report.update(version=1,status='prepared',fixture=False,manifest=reference)
    private_json(output/'inventory.json',report)
    if pins!=verify_original() or before!=preservation():raise GateError('Preservation changed during inventory')
    print(json.dumps({'status':'prepared','fixture':False,'selected_session':selected,'journals':len(report['journals']),
        'files':report['file_count'],'bytes':report['bytes'],'evidence':str(output),'phase_acceptance':False}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('workspace',type=Path);parser.add_argument('source',type=Path);parser.add_argument('session')
    parser.add_argument('new_private_output',type=Path);args=parser.parse_args()
    try:run(args.workspace,args.source,args.session,args.new_private_output)
    except (GateError,OSError,ValueError) as error:parser.exit(1,'Legacy preparation did not pass: '+str(error)+'\n')
