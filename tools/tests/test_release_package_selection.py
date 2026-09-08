import copy,hashlib,io,json,shutil,sys,tarfile,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from common import ROOT,GateError
from release_package import validate_candidate

class CandidateSelection(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.digest='a'*64
        destination=self.root/'product/release';destination.mkdir(parents=True)
        shutil.copyfile(ROOT/'product/release/install.py',destination/'install.py')
        payloads={'bin/ag1-cli':b'synthetic fixture bytes, never executed','sbom.cdx.json':b'{"fixture":true}\n'}
        manifest={'version':1,'platform':'linux-x86_64','input_sha256':self.digest,'files':[{'path':name,'sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data),'mode':0o755 if name.startswith('bin/') else 0o644} for name,data in payloads.items()]}
        encoded=json.dumps(manifest).encode();payloads['package-manifest.json']=encoded
        archive=self.root/'fixture.tar.gz'
        with tarfile.open(archive,'w:gz') as tar:
            for name,data in payloads.items():
                info=tarfile.TarInfo(name);info.size=len(data);info.mode=0o755 if name.startswith('bin/') else 0o644;tar.addfile(info,io.BytesIO(data))
        (self.root/'manifest.json').write_bytes(encoded);(self.root/'sbom.json').write_bytes(payloads['sbom.cdx.json']);(self.root/'build.log').write_text('synthetic unit fixture\n')
        def ref(name):return {'path':name,'sha256':hashlib.sha256((self.root/name).read_bytes()).hexdigest()}
        self.report={'status':'built','input_sha256':self.digest,'platform':'linux-x86_64','archive':ref('fixture.tar.gz'),'extensions_archive':ref('fixture.tar.gz'),'manifest':ref('manifest.json'),'sbom':ref('sbom.json'),'commands':[{'argv':['synthetic-fixture'],'exit_code':0,'log':'build.log'}],'artifacts':[ref(n) for n in ('fixture.tar.gz','manifest.json','sbom.json','build.log')]}
    def select(self,report=None,digest=None):
        return validate_candidate(report or self.report,digest or self.digest,self.root,self.root/'installation')
    def test_exact_archive_is_installed_without_rebuilding(self):
        before=(self.root/'fixture.tar.gz').read_bytes();installed=self.select()
        self.assertEqual((installed/'bin/ag1-cli').read_bytes(),b'synthetic fixture bytes, never executed')
        self.assertEqual((self.root/'fixture.tar.gz').read_bytes(),before)
    def test_stale_source_and_unbound_logs_are_rejected(self):
        for field,value in (('input_sha256','b'*64),('commands',[{'argv':['fixture'],'exit_code':0,'log':'missing'}])):
            report=copy.deepcopy(self.report);report[field]=value
            with self.assertRaises(GateError):self.select(report)
    def test_manifest_must_bind_the_same_input_digest(self):
        report=copy.deepcopy(self.report);report['input_sha256']='b'*64
        with self.assertRaises(GateError):self.select(report,'b'*64)
    def test_archive_tampering_is_not_hidden_by_a_rebuild(self):
        with (self.root/'fixture.tar.gz').open('ab') as f:f.write(b'tampered')
        with self.assertRaises(GateError):self.select()

if __name__=='__main__':unittest.main()
