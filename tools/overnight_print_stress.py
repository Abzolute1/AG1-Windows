"""P14-guided eight-hour investigation through AG1's existing print-mode gate.

All original source/verifier/recall/context failures remain fatal. This is a
separate mode investigation, not ordinary-chat or formal P14 acceptance.
"""
import argparse
import json
from pathlib import Path
from overnight_stress import Investigation,ROOT
from release_gate import ref
from faults import retain

class PrintInvestigation(Investigation):
    def __init__(self,args):
        self.print_verifier=None
        super().__init__(args)
        self.bound.append(ref(Path(__file__).resolve()))
        retain(self.out/'mode-scope.json',{
            'mode':'Native session print mode for CLI repair prompts; native WebKit trial retains ordinary chat mode',
            'reason':'Ordinary-chat run 7 stopped on false completion with no edit/verifier calls. This run exercises the existing bounded native completion gate and exact canonical-verifier receipts.',
            'prompt_addendum':'The intended verification command is: `node <unchanged original verifier>`',
            'criteria':'Original verifier sources, required current-contract recall, source mutation scope, context bound, real eight-hour clock, owned fault recovery, memory budget and claim review remain unchanged and strict.',
            'prior_failure_retained':str(ROOT/'.local/p14-long-horizon/overnight-7/duration-result.json'),
            'release_acceptance':False,
        })

    def open(self):
        result=super().open()
        execute=self.client.execute
        def gated(command,*args,**kwargs):
            if command.get('kind')=='prompt':
                source=command.get('input',{})
                if source.get('kind')!='inline' or not self.print_verifier:
                    raise RuntimeError('Print investigation only converts its declared inline repair prompt')
                text=source['text']+'\n\nThe intended verification command is: `node '+self.print_verifier+'`'
                command={'kind':'session','action':'print','args':{'input':{'kind':'inline','text':text}}}
            return execute(command,*args,**kwargs)
        self.client.execute=gated
        return result

    def task(self,index,through_renderer=False):
        self.print_verifier=self.specs[index%len(self.specs)]['verification']
        try:return super().task(index,through_renderer)
        finally:self.print_verifier=None

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output',type=Path)
    parser.add_argument('--seed-source',type=Path,required=True)
    parser.add_argument('--package-report',type=Path,default=ROOT/'implementation/evidence/P14/package.json')
    parser.add_argument('--binaries',type=Path,required=True)
    parser.add_argument('--heap-library',type=Path,required=True)
    parser.add_argument('--hours',type=int,choices=[8],default=8)
    PrintInvestigation(parser.parse_args()).run()
