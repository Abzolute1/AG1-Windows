"""Extract pinned research regex data; no code evaluation or original-tree writes."""
import argparse
import hashlib
import json
import pathlib
import re
ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'reference/ag1-executable/packages/coding-agent/src/core/tools/research-output.ts'
TARGET = ROOT / 'product/crates/ag1-core/resources/research-rules.json'

def literal(text, start):
    assert text[start] == '/'
    i = start + 1
    bracket = False
    while i < len(text):
        if text[i] == '\\':
            i += 2
            continue
        if text[i] == '[': bracket = True
        if text[i] == ']': bracket = False
        if text[i] == '/' and not bracket:
            end = i + 1
            while end < len(text) and text[end] in 'gimsuy': end += 1
            return text[start + 1:i], text[i + 1:end], end
        i += 1
    raise ValueError('unterminated source regex')

def capture():
    source = SOURCE.read_text()
    values = {}
    names = ['CURRENT_EVENTS_COVERAGE_SLOTS', 'DOCS_COVERAGE_SLOTS', 'PRICING_COVERAGE_SLOTS', 'RELEASE_COVERAGE_SLOTS', 'ERROR_COVERAGE_SLOTS', 'RESEARCH_PLACEHOLDER_PATTERNS', 'RESEARCH_NOISE_LINE_PATTERNS', 'RESEARCH_CODE_LINE_PATTERNS', 'RESEARCH_DOCS_CHROME_LINE_PATTERNS', 'RESEARCH_LOW_VALUE_DOCS_PREVIEW_PATTERNS']
    for name in names:
        start = source.index('const ' + name)
        start = source.index('= [', start) + 3
        body = source[start:source.index('\n];', start)]
        rows = []
        if 'slot:' in body or 'reason:' in body:
            marker = 'slot' if 'slot:' in body else 'reason'
            names_found = re.findall(marker + r': "([^"]+)"', body)
            for pattern in re.finditer(r'pattern:\s*/', body):
                regex, flags, _ = literal(body, pattern.end() - 1)
                rows.append({'pattern': regex, 'flags': flags})
            assert len(rows) == len(names_found)
            for row, value in zip(rows, names_found): row[marker] = value
        else:
            pos = 0
            while True:
                slash = body.find('/', pos)
                if slash < 0: break
                regex, flags, pos = literal(body, slash)
                rows.append({'pattern': regex, 'flags': flags})
        assert rows
        values[name] = rows
    return {'source_commit': 'e30f0b4b238ee5618928bcd17b4571eb046e7038', 'source_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(), 'rules': values}

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--capture', action='store_true')
    args = parser.parse_args()
    value = capture()
    if args.capture:
        with TARGET.open('x') as stream: stream.write(json.dumps(value, indent=2) + '\n')
    else:
        assert value == json.loads(TARGET.read_text())
    print(json.dumps({'status': 'passed', 'research_rule_groups': len(value['rules']), 'research_rules': sum(map(len, value['rules'].values()))}))
