"""Explicit wire cases grounded in the pinned SDK handlers; no service access.

The separate Node oracle executes these frames through the reference adapters.
This source is not itself parity evidence.
"""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

def build():
    cases = []
    for api in ['anthropic-messages','openai-completions','mistral-conversations',
                'openai-responses','azure-openai-responses','openai-codex-responses',
                'google-generative-ai','google-gemini-cli','google-vertex','bedrock-converse-stream']:
        if api == 'anthropic-messages':
            frames = [{'type':'message_start','message':{'id':'response-1','type':'message','role':'assistant','model':'fixture','content':[], 'usage':{'input_tokens':100,'output_tokens':0,'cache_read_input_tokens':20,'cache_creation_input_tokens':5}}}]
            for index,kind,body in [(0,'thinking',{'thinking':''}),(1,'text',{'text':''}),(2,'tool_use',{'id':'call_1','name':'read','input':{}})]:
                frames.append({'type':'content_block_start','index':index,'content_block':{'type':kind,**body}})
                field = {'thinking':('thinking_delta','thinking',['plan']), 'text':('text_delta','text',['Hello ','🌙']), 'tool_use':('input_json_delta','partial_json',['{"path":','"a.txt"}'])}[kind]
                frames += [{'type':'content_block_delta','index':index,'delta':{'type':field[0],field[1]:text}} for text in field[2]]
                if kind == 'thinking': frames.append({'type':'content_block_delta','index':index,'delta':{'type':'signature_delta','signature':'sig'}})
                frames.append({'type':'content_block_stop','index':index})
            frames += [{'type':'message_delta','delta':{'stop_reason':'tool_use','stop_sequence':None},'usage':{'output_tokens':10}}, {'type':'message_stop'}]
            error={'type':'error','error':{'type':'overloaded_error','message':'fixture provider failure'}}
        elif api in ['openai-completions','mistral-conversations']:
            def chunk(delta,reason=None,usage=None):
                result={'id':'response-1','object':'chat.completion.chunk','created':1,'model':'fixture','choices':[{'index':0,'delta':delta,'finish_reason':reason}]}
                if usage is not None:result['usage']=usage
                return result
            tool_id='call00001' if api=='mistral-conversations' else 'call_1'
            thinking={'content':[{'type':'thinking','thinking':[{'type':'text','text':'plan'}]}]} if api=='mistral-conversations' else {'reasoning_content':'plan'}
            frames=[chunk(thinking),chunk({'content':'Hello '}),chunk({'content':'🌙'}),chunk({'tool_calls':[{'index':0,'id':tool_id,'type':'function','function':{'name':'read','arguments':'{"path":'}}]}),chunk({'tool_calls':[{'index':0,'id':tool_id,'type':'function','function':{'name':'read' if api=='mistral-conversations' else '', 'arguments':'"a.txt"}'}}]}),chunk({},'tool_calls',{'prompt_tokens':120,'completion_tokens':10,'total_tokens':130,'prompt_tokens_details':{'cached_tokens':20}})]
            error={'error':{'type':'server_error','message':'fixture provider failure'}}
        elif 'responses' in api:
            items=[{'id':'rs_1','type':'reasoning','summary':[{'type':'summary_text','text':'plan'}],'encrypted_content':'encrypted-fixture'}, {'id':'msg_1','type':'message','role':'assistant','phase':'final_answer','status':'completed','content':[{'type':'output_text','text':'Hello 🌙','annotations':[]}]}, {'id':'fc_1','call_id':'call_1','type':'function_call','name':'read','arguments':'{"path":"a.txt"}','status':'completed'}]
            frames=[{'type':'response.created','response':{'id':'response-1','status':'in_progress'}}]
            for index,item in enumerate(items):
                start=dict(item)
                if index==0:start['summary']=[]
                if index==1:start['content']=[]
                if index==2:start['arguments']=''
                frames.append({'type':'response.output_item.added','output_index':index,'item':start})
                if index==0:frames.append({'type':'response.reasoning_summary_part.added','output_index':0,'item_id':'rs_1','summary_index':0,'part':{'type':'summary_text','text':''}})
                for text in (['plan'] if index==0 else ['Hello ','🌙'] if index==1 else ['{"path":','"a.txt"}']):
                    frames.append({'type':['response.reasoning_summary_text.delta','response.output_text.delta','response.function_call_arguments.delta'][index],'output_index':index,'item_id':item['id'],'summary_index':0,'content_index':0,'delta':text})
                frames.append({'type':'response.output_item.done','output_index':index,'item':item})
            frames.append({'type':'response.completed','response':{'id':'response-1','status':'completed','output':items,'usage':{'input_tokens':120,'output_tokens':10,'total_tokens':130,'input_tokens_details':{'cached_tokens':20}}}})
            error={'type':'response.failed','response':{'error':{'code':'server_error','message':'fixture provider failure'}}}
        elif api.startswith('google-'):
            frames=[{'responseId':'response-1','candidates':[{'index':0,'content':{'role':'model','parts':[part]}}]} for part in [{'thought':True,'text':'plan','thoughtSignature':'c2ln'},{'text':'Hello '},{'text':'🌙','thoughtSignature':'dGV4dA=='},{'functionCall':{'id':'call_1','name':'read','args':{'path':'a.txt'}},'thoughtSignature':'dG9vbA=='}]]
            frames.append({'candidates':[{'index':0,'finishReason':'STOP'}],'usageMetadata':{'promptTokenCount':120,'candidatesTokenCount':7,'thoughtsTokenCount':3,'cachedContentTokenCount':20,'totalTokenCount':130}})
            if api=='google-gemini-cli':frames=[{'response':f} for f in frames]
            error={'error':{'code':500,'message':'fixture provider failure','status':'INTERNAL'}}
        else:
            frames=[{'messageStart':{'role':'assistant'}},{'contentBlockDelta':{'contentBlockIndex':0,'delta':{'reasoningContent':{'text':'plan','signature':'sig'}}}},{'contentBlockStop':{'contentBlockIndex':0}},{'contentBlockDelta':{'contentBlockIndex':1,'delta':{'text':'Hello '}}},{'contentBlockDelta':{'contentBlockIndex':1,'delta':{'text':'🌙'}}},{'contentBlockStop':{'contentBlockIndex':1}},{'contentBlockStart':{'contentBlockIndex':2,'start':{'toolUse':{'toolUseId':'call_1','name':'read'}}}},{'contentBlockDelta':{'contentBlockIndex':2,'delta':{'toolUse':{'input':'{"path":'}}}},{'contentBlockDelta':{'contentBlockIndex':2,'delta':{'toolUse':{'input':'"a.txt"}'}}}},{'contentBlockStop':{'contentBlockIndex':2}},{'messageStop':{'stopReason':'tool_use'}},{'metadata':{'usage':{'inputTokens':100,'outputTokens':10,'cacheReadInputTokens':20,'cacheWriteInputTokens':5,'totalTokens':135}}}]
            error={'modelStreamErrorException':{'message':'fixture provider failure'}}
        cases.append({'api':api,'frames':frames,'error':error,'expected_text':'Hello 🌙','expected_thinking':'plan','expected_tool':{'name':'read','arguments':{'path':'a.txt'}}})
    return {'version':1,'source_commit':'e30f0b4b238ee5618928bcd17b4571eb046e7038','scope':'Synthetic wire inputs based on pinned handlers; oracle output is captured separately.','cases':cases}

if __name__ == '__main__':
    path=ROOT/'fixtures/providers/wire.json'
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(build(),indent=2,ensure_ascii=False)+'\n')
    print(json.dumps({'provider_groups':10,'network_actions':0}))
