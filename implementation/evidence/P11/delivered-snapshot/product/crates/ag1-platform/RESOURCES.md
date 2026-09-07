# Native resource foundations (P09 in progress)

`ag1_core::resources` supplies bounded YAML frontmatter, skill metadata/activation/XML formatting, prompt argument substitution, package source parsing, theme validation, precedence/collisions, and project extension trust selection. `resource_fs` discovers and captures files through registered P03 workspace descriptors. `resources::ResourceLoader` assembles the selected package/local/auto resources and context files, rechecks the captured files, and publishes one complete generation. The TS desktop remains the execution authority; this document is not a P09 completion claim.

## Captured behavior

The executable reference probes in `tools/probes/tool_*resource*`, `tool_theme_reference.mjs`, `tool_package_source_reference.mjs`, and `tool_extension_trust_reference.mjs` retain the actual pinned SDK's outputs. Native resource tests compare these outputs, including:

- User-first standalone skill loading versus project-first full resource loading; canonical file deduplication, warning-before-collision ordering, project package identity precedence, manifest filters and automatic `.ag1` / `.agents` discovery.
- 79 helper cases; 11 discovery/helper cases in one filesystem fixture; 23 themes; 44 package-pattern sets over 15 paths; 46 package sources and six npm command configurations; 12 package settings changes; 14 extension trust cases. Individual fixture rows cover different layers; these counts do not claim every P01 child case is complete.
- Prompt substitution's actual four sequential JavaScript replacement passes, including replacement-string dollar syntax and UTF-16 truncation behavior.
- Theme color tokens, variable references/cycles, optional diff fallbacks, and the reference's permissive pairwise hex parsing. Theme data is separate from terminal rendering.
- Project, user and package source information; global context then ancestor context in root-to-cwd order; project `SYSTEM.md`/`APPEND_SYSTEM.md` before user files.

Package pattern evaluation uses bounded brace expansion, glob segments and extended-glob alternatives/repetition. It includes source-order exact `+`/`-` overrides and skill-parent matching. Matching only selects from discovered files; it cannot authorize an outside root. Bounds and unsupported/malformed inputs return explicit errors. Matching arbitrary third-party minimatch syntax beyond the captured cases is not a compatibility claim.

## Capability, trust and publication boundaries

Resource roots are registered explicitly by the host. Canonicalization observes alias metadata; content is opened through a registered no-follow workspace descriptor, with raw hash and before/after identity checks. Aliases outside registered roots cannot supply content. Directory cycles are skipped with diagnostics. Optional ancestor discovery only visits host-admitted roots; it never registers the filesystem root merely to emulate an unrestricted SDK walk.

Default capture marks extension entries as untrusted data. The host captures the explicit trust option, environment override, and **user** trusted-project settings for `resources::trust::select`; project settings cannot grant trust. The compatibility process must separately admit the selected entry bytes under its active generation. A manifest, resource name, or `SourceInfo` is never an execution grant.

Reload is compare-and-swap on the caller's generation and swaps only after building and revalidating the complete candidate. Cancelled, stale, malformed-settings and changed-file failures preserve the prior snapshot. Capture revalidation includes canonical alias identity and the content of observed settings/manifests. This is a validated filesystem observation, not an atomic snapshot of every directory on the machine: a newly created file after discovery belongs to a later reload.

`extend_resources` accepts host-admitted skill, prompt and theme contributions under the same generation fence. It refreshes only the supplied resource kinds and preserves the other captured bytes, context and system prompts. Repeated normalized paths keep their position and replace contribution metadata; a full reload clears these dynamic contributions. Code entries cannot be added through this API. Nine captured reference states verify this behavior. Native capture deliberately retains prior auto/package source metadata that the SDK loses during an extension update. Contribution metadata is provenance data, not a trust grant; the compatibility host must bind it to the contributing extension. Missing-skill diagnostics survive updates to other resource kinds.

`ResourceLoader::discover_extensions` connects an already admitted extension host to that publication boundary. It checks the caller's scope, resource generation and held workspace identity before invoking `resources_discover`, accepts only skill/prompt/theme paths, derives source labels from admitted extension entries and then calls `extend_resources`. Helper metadata cannot grant trust or code loading. Stale scope/generation and malformed or ungranted results leave the prior resource snapshot intact. A handler exceeding its result bound contributes no partial paths; the helper reports its error and continues with later handlers. Native integration tests cover startup/reload, exact SDK source metadata for all three resource kinds, failed updates and an unrelated workspace. P10 still owns session lifecycle events and system-prompt rebuilding.

`skill_creator::SkillCreator` is a host-registered command, not an automatically active model tool. Its private draft and one-use approval bind the workspace, scope/epoch, registry, arguments, target and service incarnation. `Workspace::create_directory_document_checked` publishes an entire new skill directory with `RENAME_NOREPLACE` under the P03 workspace/Git claim. A competing directory cannot be merged or overwritten; cancellation before publication removes owned staging contents. Empty ancestor directories may remain after an interrupted scaffold. Published-but-unconfirmed durability is reported as uncertain. P10/P12 must call this through the durable executor/controller and perform the subsequent reload; helper creation does not itself verify a task or start an extension.

## Explicit corrections and limits

The helper preserves ordinary reference bytes but quotes descriptions when unquoted YAML would alter their value or add fields. Project trust also follows canonical entry provenance, closing the captured user-directory symlink bypass. YAML alias cycles are rejected before materialization; unsupported complex mapping keys and invalid metadata types do not become executable or mislabeled resources. Invalid settings fail a reload explicitly rather than silently exposing a partial catalog. Errors use native typed categories where the JS parser's wording differs.

Limits include 64 registered roots; 32 directory levels; 20,000 scanned entries; 4,096 discovered/captured resources; 1 MiB per document; 32 MiB aggregate captured input and accounted retained catalog; 256 KiB YAML input, depth 32, 50,000 events, 20,000 materialized nodes and 4 MiB materialized strings; 4 MiB template/skill prompt output; and bounded pattern expansions/work. These are explicit native resource limits, not claims about the original SDK. The retained estimate includes strings, containers and theme values; allocator/process overhead is additional.

Incremental extension updates retain at most 32 MiB of accounted source captures (bytes plus path/metadata storage), separately from the 32 MiB parsed catalog. Resolved and contributed path sets each have a 4 MiB accounted limit. The old generation remains alive while the bounded candidate is built, so peak memory includes both generations and temporary parsing data.

`package_settings::PackageSettings` preserves unknown settings/filter fields and normalizes local source references against the selected settings base. Its generated write must pass the ordinary file policy and one-use approval; it compares the JSON capture revision with the prepared file revision, then P03 CAS checks again at publication. Equivalent source identities produce no-op additions/removals. A stale settings edit cannot overwrite a newer external change.

## Remaining integration

The selected skill tests now replay the pinned fixture tree, including invalid YAML,
multiline descriptions, invocation flags and collision metadata. YAML parse warnings
retain a line/column location; parser-specific wording is not a byte-for-byte SDK
contract. Additional cases capture skill activation and XML formatting, isolated
home-path expansion, disabled discovery with explicit additions, negative settings
filters, and user/project skill/prompt/theme precedence. Explicit extension paths
precede discovered entries; additional data-resource paths continue to follow them.
The optional helper reports duplicate tool/flag owners while retaining both
colliding commands under suffixed names.

The loader accepts host-pinned installation records and reports missing or mismatched remote packages without implicitly executing package commands. `package_execution::PackageService` executes approved install/remove/update requests through the core lifecycle manager and P05 durable executor. Native package inspection, update selection and bounded remote queries are implemented; startup missing-package decisions, live progress and settings/reload composition still need controller wiring. Native resource override callbacks and the optional JavaScript process/tool/inference adapters are implemented and have fixture coverage. Connecting resource contributions and custom provider registration to the composition controller, remaining module/package entry cases, and the full P09 deterministic driver/gate remain in progress. No unimplemented fixture may be discarded to declare the phase complete.

## Package execution

`managePackage` is an explicitly registered trusted-host command, inactive by default. Preparation binds source, action, scope, registry, workspace/session epoch, service incarnation and package generation. The interactive policy requests approval; the parent may explicitly admit the command under its existing autonomous policy. A native P05 intent is acknowledged before execution, and each command's separate stdout/stderr evidence is persisted before its result. Completed operations replay without another process launch. Unacknowledged command evidence leaves the intent uncertain and retains bounded owned stdout/stderr and operation metadata files for reconciliation.

The host supplies dedicated writable package roots, read roots, npm/Git executable bindings and an explicit environment. Executable identity is checked and a held descriptor supplies the executable path; working directories and spool directories are pinned P03 capabilities. The adapter serializes its package manager, uses no shell interpolation, rejects option-like names/refs and credential-bearing HTTP Git URLs, and checks cancellation/deadlines during command I/O. Npm global commands receive the explicitly admitted prefix; global-root answers must match its Linux `lib/node_modules` directory. User/global npm configuration and Git global configuration are disabled in the command environment. No environment is inherited implicitly.

Approved package scripts run with the host account's privileges. These controls are **not an OS sandbox**, and scripts or their descendants can access files and services available to that account. Registering an overly broad writable package root also grants native removal within that root. Native mkdir, new metadata files and bounded tree removal use P03 descriptor operations; Git/npm remain external mutations in the owned checkout/cache, with interruption reported as an unknown effect. Empty Git ancestors are pruned without recursively deleting newly added contents. Temporary npm install/update/remove consistently use the temporary prefix.

Limits are 16 writable roots, 64 initial read roots, two configured command bindings, 32 commands per operation, 128 arguments per manager command, 128 progress records, 64 KiB captured command response, 4 MiB retained stdout for other commands and 8 MiB stderr per command. Exceeding output limits stops the owned child and marks the retained evidence as truncated. Captures have a 10-second timeout and other commands a five-minute limit, each shortened by the operation deadline. Progress is currently retained in the operation result; live controller delivery remains integration work.

The core manager matches 24 captured SDK lifecycle cases through a deterministic port. Separate native tests execute only a compiled owned fixture command: actual npm/Git packages are never installed. They cover project/user/temporary destinations, Git checkout/removal, separate output artifacts, replay, failure, quota, active cancellation, timeout, stale generations, replaced binaries, outside symlink rejection and failed evidence acknowledgement. These tests do not yet cover every inventoried package child case or constitute the P09 gate.

## Package inspection and update checks

Twelve captured SDK selection cases cover user-first explicit updates, project-first
update checks, first-entry identity deduplication, pinned project entries suppressing
user copies, and source-prefix suggestions. Twenty-two additional cases compare npm
version and Git upstream/HEAD results and exact query-port calls. These use the actual
SDK methods with fixture transport responses, not a second implementation of the oracle.

`PackageService::needs_install` reads only admitted installed paths. Unpinned installed
packages need no registry lookup; pinned versions use the SDK's exact string comparison,
including range strings. `update_check_queries` resolves the already granted npm prefix
without starting npm. Resource reload withholds a mismatched pinned package's resources
and reports that installation is required. The normal approved P05 install operation can
then settle the requested pin; inspection itself never grants execution.

`PackageService::installation_record` returns a host observation only when an
admitted package directory exists and its configured npm pin matches. Local/Git
observations do not run npm validation or commands. `ResourceLoader::reload_installed`
uses those records under its generation fence, captures the resources again and
publishes the installation map and snapshot together. Failed or cancelled reloads
retain both the prior snapshot and prior map. A fixture integration test installs
through P05, discovers the installed prompt, then verifies rollback after malformed
settings and mismatched package identity; inspection and reload launch no commands.


The core checker holds at most four active checks and returns results in configuration
order. `package_queries::QueryPort` supplies registered manifest reads and native HTTP
through `WebHttp`, with a host-selected registry origin, no ambient credentials, the
existing redirect/DNS rules and a ten-second network bound. `package_query_git::NativeGit`
admits exact installation roots and held executable identities, accepts only the selected
rev-parse/ls-remote commands, and uses the existing owned-process supervisor. Repository
configuration and remote helpers remain trusted host-account code, not sandboxed code.

Query results require original evidence acknowledgement first. `StoredEvidence` uses a
session/operation-bound P05 context store and binary-preserving envelopes; its aggregate
limits are 8,192 records and 64 MiB of encoded envelopes. Manifest/HTTP bodies are bounded
by the existing one-MiB/one-million-byte readers; Git stdout and stderr each have a 64-KiB
capture limit. Owned spool files remain until acknowledgement, including interrupted or
failed capture paths. A cancelled future drops its owned process/network requests.

Ordinary registry/Git failures yield no update, matching the SDK. Native owner cancellation,
expired operation deadlines, stale capabilities, resource limits and failed evidence
acknowledgement propagate explicitly. A per-request network timeout is an ordinary failed
check if the owner deadline is still live. Malformed registry version values retain the
SDK's strict JavaScript comparison behavior. Embedded HTTP credentials in Git sources and
npm URL/alias specs are rejected before package execution. Native tests use only owned
loopback HTTP servers and compiled fixture commands, and verify P05 bytes and session ACLs.


## Native resource overrides

`ResourceLoader::with_overrides` installs an explicitly named Rust embedding
callback before the first reload. The callback receives a typed `View` of the
catalog, context files and system/append prompts. It provides the native equivalent
of the SDK resource override functions; resource settings and extension RPC cannot
construct this callback capability. Six captured SDK states cover injected skills,
system replacement, combined overrides and a subsequent prompt-only refresh.

A partial refresh passes the changed resource kinds and rejects changes to other
resource groups or context/system fields. Unchanged effective groups remain intact;
changed groups start from freshly captured input, so transformations do not append
themselves repeatedly. Unknown combined host diagnostics remain until explicitly
replaced or fully reloaded. Native provenance retention follows the correction
already documented above.

Each overridden snapshot retains a separate original view plus before/after hashes
and the registered override identity. A partial refresh updates the original view
only for its changed groups. Derived metadata cannot mark an extension as trusted:
all resulting code entries remain data until separate path/hash/trust admission.
Captured sources are revalidated after the callback. Callback errors, excess output,
changed sources and forbidden partial changes leave the previous generation intact.

Original/effective parsed catalogs each have a 32 MiB accounted limit, and each
context view has a 32 MiB text/path limit. These are additional to retained raw
resource captures. Building an overridden generation temporarily retains both old
and candidate views and callback inputs; no unbounded history chain is kept. Native
embedding callbacks are trusted code and can allocate their own working memory;
output bounds are enforced before publication.

Explicit package source resolution is available through
`ResourceLoader::resolve_sources`. It shares manifest discovery and package
filtering with configured packages, returns scoped resource paths and diagnostics,
and does not publish a catalog. All roots still pass through `ResourceFs` grants;
remote packages need host-pinned installation records. Discovery never implicitly
installs or refreshes a package. P10 can request an approved `PackageService`
operation and then resolve the settled installation. Observed manifests are
revalidated before returning, and extension entries still require capture and
trust admission before helper execution.
A mismatched pinned npm version is withheld from discovery until an approved
installation settles. This intentionally replaces the SDK's implicit installation
during `resolve`; the native installer separately checks the configured pin,
executes the approved operation and supplies its updated installation record.
