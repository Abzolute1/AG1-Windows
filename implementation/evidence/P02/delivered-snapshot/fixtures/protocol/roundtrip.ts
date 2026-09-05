// Generated Rust codec fixtures.
import type { WireMessage } from '../../product/apps/desktop/src/generated/protocol.js';
export const fixtures = [
  {
    "type": "handshake",
    "data": {
      "version": {
        "major": 1,
        "minor": 0,
        "schema": "7486efa2f87f5daa2ca1823c385bbc3860d6d022e846553426116db9312a6af7"
      },
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "fake": true,
      "capabilities": [
        "fixture_prompt",
        "cancel",
        "approve",
        "command_result",
        "artifact_read",
        "snapshot_subscribe"
      ]
    }
  },
  {
    "type": "command",
    "data": {
      "version": {
        "major": 1,
        "minor": 0,
        "schema": "7486efa2f87f5daa2ca1823c385bbc3860d6d022e846553426116db9312a6af7"
      },
      "id": "request-1",
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "expected_revision": "0",
      "command": {
        "kind": "prompt",
        "input": {
          "kind": "inline",
          "text": "fixture only"
        }
      }
    }
  },
  {
    "type": "command",
    "data": {
      "version": {
        "major": 1,
        "minor": 0,
        "schema": "7486efa2f87f5daa2ca1823c385bbc3860d6d022e846553426116db9312a6af7"
      },
      "id": "request-1",
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "expected_revision": "0",
      "command": {
        "kind": "prompt",
        "input": {
          "kind": "inline",
          "text": "fixture only"
        }
      },
      "extension": null
    }
  },
  {
    "type": "command",
    "data": {
      "version": {
        "major": 1,
        "minor": 0,
        "schema": "7486efa2f87f5daa2ca1823c385bbc3860d6d022e846553426116db9312a6af7"
      },
      "id": "request-1",
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "expected_revision": "0",
      "command": {
        "kind": "prompt",
        "input": {
          "kind": "inline",
          "text": "fixture only"
        }
      },
      "extension": {
        "decimal": "18446744073709551615",
        "future": [
          null,
          {
            "flag": true
          }
        ]
      }
    }
  },
  {
    "type": "subscription",
    "data": {
      "id": "fixture-sub-1",
      "snapshot": {
        "fake": true,
        "scope": {
          "workspace": "fixture-workspace",
          "session": "fixture-session",
          "instance": "fixture-codegen",
          "epoch": "1"
        },
        "branch": "fixture-branch",
        "revision": "0",
        "watermark": "0",
        "queued": [],
        "last_terminal": null
      }
    }
  },
  {
    "type": "snapshot",
    "data": {
      "fake": true,
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "branch": "fixture-branch",
      "revision": "0",
      "watermark": "0",
      "queued": [],
      "last_terminal": null
    }
  },
  {
    "type": "result",
    "data": {
      "id": "request-1",
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "watermark": "1",
      "result": {
        "kind": "accepted",
        "operation": "fixture-op-2"
      }
    }
  },
  {
    "type": "snapshot",
    "data": {
      "fake": true,
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "branch": "fixture-branch",
      "revision": "2",
      "watermark": "2",
      "queued": [],
      "last_terminal": {
        "operation": "fixture-op-2",
        "status": "completed"
      }
    }
  },
  {
    "type": "delivery",
    "data": {
      "kind": "events",
      "events": [
        {
          "scope": {
            "workspace": "fixture-workspace",
            "session": "fixture-session",
            "instance": "fixture-codegen",
            "epoch": "1"
          },
          "sequence": "1",
          "revision": "1",
          "operation": {
            "operation": "fixture-op-2",
            "status": "queued"
          }
        },
        {
          "scope": {
            "workspace": "fixture-workspace",
            "session": "fixture-session",
            "instance": "fixture-codegen",
            "epoch": "1"
          },
          "sequence": "2",
          "revision": "2",
          "operation": {
            "operation": "fixture-op-2",
            "status": "completed"
          }
        }
      ],
      "cursor": {
        "scope": {
          "workspace": "fixture-workspace",
          "session": "fixture-session",
          "instance": "fixture-codegen",
          "epoch": "1"
        },
        "after": "2"
      },
      "more": false
    }
  },
  {
    "type": "cursor",
    "data": {
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "after": "9007199254740993"
    }
  },
  {
    "type": "cursor",
    "data": {
      "scope": {
        "workspace": "fixture-workspace",
        "session": "fixture-session",
        "instance": "fixture-codegen",
        "epoch": "1"
      },
      "after": "18446744073709551615"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "incompatible_version"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "malformed",
      "detail": "fixture"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "oversize",
      "limit": 65536,
      "use_artifact": true
    }
  },
  {
    "type": "error",
    "data": {
      "code": "queue_full",
      "queue": "session",
      "capacity": 256
    }
  },
  {
    "type": "error",
    "data": {
      "code": "unavailable",
      "family": "model"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "stale_context"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "command_conflict"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "invalid_approval"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "not_found"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "resync_required"
    }
  },
  {
    "type": "error",
    "data": {
      "code": "capacity",
      "resource": "subscriber"
    }
  }
] satisfies WireMessage[];
