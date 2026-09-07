# Native protocol facade

Build with `node product/clients/sdk/build.mjs` from the repository root, using the pinned Node on PATH and the desktop's pinned TypeScript installation. `npm --prefix product/clients/sdk test` tests the built ESM package. There are no runtime npm dependencies. Protocol declarations are copied from the Rust-generated contract at build time.

```ts
import { NativeClient } from "@ag1/native-client";
// The embedding host supplies framed JSONL, Tauri or another explicit transport.
const client = new NativeClient({ send, subscribe });
const detach = client.subscribe(view => render(view));
const reply = await client.prompt("Review the project");
// reply.data contains admission/operation information; admission is not completion.
// Observe later Client snapshots or query native command_result using the saved ID.
detach();
client.dispose(); // Detaches this facade; the native task remains owned by the host.
```

`Transport.send(frame)` writes one generated WireMessage. `Transport.subscribe(listener)` returns a detach function. The transport must decode/validate bounded frames and deliver the host's initial Client snapshot before commands. Reconnection obtains a fresh snapshot. Preserve command IDs and query recorded outcomes before retrying an uncertain operation. A framing failure rejects pending promises as unknown; it does not invent reply correlation. Replies require both ID and complete scope equality. Snapshots are cloned so presentation cannot mutate authority. The facade bounds outstanding commands to 256 and owns no credentials, process, inference loop or stdout.

`NativeClient` exposes `snapshot`, `subscribe`, `command`, `prompt`, `steer`, `cancel`, and `dispose`. `createAgentSession(transport)` is a named construction convenience returning this facade, not the old TypeScript AgentSession object. The old `getAgentDir` ambient global and `takeOverStdout` terminal takeover throw `UnsupportedAPI`. Importing `@ag1/native-client/hooks` also throws `UnsupportedAPI`: the pinned advertised source/dist hooks targets do not exist. Active P09 extension hooks are a different API and remain supported at their documented native boundaries.

See [client configuration and modes](../README.md) and [compatibility](../COMPATIBILITY.md). This package has not been published; its private package manifest prevents an accidental publish.
