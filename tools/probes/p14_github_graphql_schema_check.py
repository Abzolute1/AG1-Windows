#!/usr/bin/env python3
"""Validate fixed GitHub templates against a downloaded official public schema.

Usage: python p14_github_graphql_schema_check.py schema.docs.graphql
Requires graphql-core==3.2.6 in an isolated environment (no product dependency).
Download: https://docs.github.com/public/fpt/schema.docs.graphql
This checks static server-schema compatibility, not account permissions or effects.
"""
import hashlib
import json
from pathlib import Path
import re
import sys

from graphql import build_schema, parse, validate

root = Path(__file__).resolve().parents[2]
source = Path(sys.argv[1]).read_bytes()
schema = build_schema(source.decode())
results = []
for filename in ("graph_reads.rs", "graph_mutations.rs", "operations.rs"):
    text = (root / "product/crates/ag1-platform/src/github" / filename).read_text()
    for match in re.finditer(r'"((?:query|mutation)\([^"\n]+)"', text):
        query = json.loads('"' + match[1] + '"')
        errors = validate(schema, parse(query))
        results.append({"file": filename, "line": text[:match.start()].count("\n") + 1,
                        "query_sha256": hashlib.sha256(query.encode()).hexdigest(),
                        "errors": [str(error) for error in errors]})
assert results, "No query templates found"
print(json.dumps({"schema_sha256": hashlib.sha256(source).hexdigest(),
                  "source": "https://docs.github.com/public/fpt/schema.docs.graphql",
                  "templates": results}, indent=2))
sys.exit(any(row["errors"] for row in results))
