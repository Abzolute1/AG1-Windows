"""Owned MCP discovery peer; no external I/O or packages.

Wire shapes are pinned against rmcp 3.3.0. This deliberately independent peer is
not an SDK conformance suite. All files/processes stay in the test's temp scope.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

mode, revision = sys.argv[1:]
assert set(os.environ) <= {"LC_CTYPE"}, "parent environment must not be inherited"
transcript = Path("events.jsonl")


def record(method):
    with transcript.open("a") as output:
        output.write(json.dumps(method) + "\n")


def send(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()


if mode == "descendant":
    child = subprocess.Popen([sys.executable, "-c", "import os,signal,time; os.setsid(); signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(60)"])
    Path("descendant.pid").write_text(str(child.pid))

if mode == "stderr":
    sys.stderr.write("synthetic-mcp-secret\x1b[31m" * 100_000)
    sys.stderr.flush()

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    record(method or "client_response")
    if method == "notifications/initialized":
        assert revision == "2025-11-25"
        continue
    if method == "notifications/cancelled":
        continue
    params = request["params"]
    if revision == "2026-07-28":
        assert params["_meta"]["io.modelcontextprotocol/protocolVersion"] == revision
        assert params["_meta"]["io.modelcontextprotocol/clientCapabilities"] == {}
    else:
        assert "_meta" not in params
    if mode == "hang":
        time.sleep(60)
    if mode == "oversized":
        sys.stdout.write("x" * (1024 * 1024 + 1))
        sys.stdout.flush()
        time.sleep(60)
    if mode == "truncated":
        sys.stdout.write('{"jsonrpc":"2.0","id":')
        sys.stdout.flush()
        sys.exit(0)
    if mode == "error":
        send({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"synthetic-mcp-secret"}})
        continue
    capabilities = {} if mode == "no_capabilities" else {"tools":{}, "resources":{}, "prompts":{}}
    if method == "initialize":
        assert revision == "2025-11-25"
        result = {"protocolVersion":"2099-01-01" if mode == "wrong_revision" else revision,
                  "capabilities":capabilities,"serverInfo":{"name":"owned-fixture","version":"1"}}
    elif method == "server/discover":
        assert revision == "2026-07-28"
        result = {"resultType":"complete", "supportedVersions":["2099-01-01" if mode == "wrong_revision" else revision],
                  "capabilities":capabilities,"ttlMs":0,"cacheScope":"private",
                  "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"owned-fixture","version":"1"}}}
    elif method == "tools/list":
        if mode == "slow_pages":
            time.sleep(0.09)
        if mode == "changed":
            send({"jsonrpc":"2.0","method":"notifications/tools/list_changed"})
        if mode == "flood":
            for _ in range(129):
                send({"jsonrpc":"2.0","method":"notifications/message","params":{"data":"ignored"}})
        if mode == "server_request":
            send({"jsonrpc":"2.0","id":"peer-request","method":"sampling/createMessage","params":{}})
            reply = json.loads(sys.stdin.readline())
            assert reply["id"] == "peer-request" and reply["error"]["code"] == -32601
            record("sampling_refused")
        if mode == "duplicate_response":
            send({"jsonrpc":"2.0","id":"ag1-mcp-1","result":{}})
        if mode == "continuation":
            send({"jsonrpc":"2.0","id":request["id"],"result":{"resultType":"input_required","requestState":"opaque-private-state"}})
            continue
        result = {"tools":[{"name":"second" if params.get("cursor") else "first",
                            "inputSchema":{"type":"object","properties":{}}}]}
        if not params.get("cursor"):
            result["nextCursor"] = "page-2"
    elif method == "resources/list":
        result = {"resources":[{"name":"source","uri":"file:///not-an-ag1-grant"}]}
        if mode == "catalog_pages":
            result["resources"][0]["uri"] += "-second" if params.get("cursor") else "-first"
            if not params.get("cursor"):
                result["nextCursor"] = "second"
        if mode == "catalog_cursor":
            result["resources"] = []
            result["nextCursor"] = "repeated"
        if mode == "catalog_changed":
            send({"jsonrpc":"2.0","method":"notifications/resources/list_changed"})
        if mode == "catalog_huge":
            result["resources"] = [{"name":str(i),"uri":"fixture://"+str(i)} for i in range(1025)]
    elif method == "resources/templates/list":
        result = {"resourceTemplates":[{"name":"by-id","uriTemplate":"fixture://{id}"}]}
    elif method == "prompts/list":
        result = {"prompts":[{"name":"review","arguments":[{"name":"topic","required":True}]}]}
        if mode == "catalog_duplicate":
            result["prompts"][0]["arguments"] *= 2
    else:
        raise AssertionError("Discovery must never invoke a tool, read a resource or get a prompt")
    if mode == "selection_scale" and method == "tools/list":
        page = int(params.get("cursor", "0"))
        result = {"tools":[{"name":f"tool-{i:04d}", "description":f"Owned capability {i}",
                            "inputSchema":{"type":"object", "properties":{"count":{"type":"integer"}}, "required":["count"]},
                            "promptGuidelines":["untrusted instruction must not enter registry guidelines"]}
                           for i in range(page*40,(page+1)*40)]}
        if page == 24:
            result["tools"][-1]["inputSchema"]["properties"]["count"] = {"enum":[1,2]}
        else:
            result["nextCursor"] = str(page+1)
    if mode == "manager_late_change" and method == "prompts/list":
        send({"jsonrpc":"2.0","method":"notifications/resources/list_changed"})
    if mode == "manager_budget" and method in ("resources/list", "resources/templates/list", "prompts/list"):
        key = {"resources/list":"resources", "resources/templates/list":"resourceTemplates", "prompts/list":"prompts"}[method]
        page = int(params.get("cursor", "0"))
        result[key] = [{"name":f"entry-{page}-{i}", "uri":f"fixture://{page}/{i}",
                        "uriTemplate":f"fixture://{page}/{i}/{{id}}", "padding":"x"*60_000} for i in range(5)]
        if page < 19:
            result["nextCursor"] = str(page+1)
    send({"jsonrpc":"2.0","id":"foreign" if mode == "bad_id" else request["id"],"result":result})
