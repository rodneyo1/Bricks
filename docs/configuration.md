# Configuration and extensibility

Status: the initial typed parser, profile resolution, validation, effective-config JSON, and SHA-256 fingerprint are implemented in `bricks-config`. Development host/client CLIs inspect settings, provision identities, and start an authenticated recording-only QUIC probe using the configured endpoint and identity reference. OS input networking, configuration migrations, per-field origin reporting, arbitrary field overrides, and the product UI remain pending. See [identity and ownership](identity-and-ownership.md) and [QUIC probe](quic-probe.md).

## Principles

Use configuration to compose tested modules and select sensible policies. Ordinary users install, pair, and connect through the UI. The UI and CLI share one typed configuration model; saving settings produces the same validated TOML. Do not require users to understand transport internals to start a session.

Each component has a stable backend ID, typed options, and declared capabilities. Examples: `windows_raw_input`, `windows_send_input`, `quic`, and later `linux_evdev`, `linux_uinput`, or a vendor-specific encoder. A new backend implements the interface and registers its configuration schema. Configuration cannot invent an implementation or make an unsupported combination work.

Use compiled modules first. Runtime-loaded DLL plugins introduce ABI, signing, and lifecycle complexity and are deferred. Keep protocol capabilities independent of local backend names: peers negotiate “relative pointer” rather than requiring identical OS implementations.

## Files and precedence

Store user settings beneath the OS per-user application-data directory. Keep private keys in OS-protected storage; configuration contains identity references only. Peer fingerprints and permissions belong in a separate trust store. Benchmark exports exclude secrets and identify peers with redacted stable IDs.

Resolve settings in this order: compiled defaults, user base configuration, one explicitly selected named profile, and explicitly supplied CLI overrides. A CLI override must map to a known typed field. Do not implicitly read arbitrary environment variables. Organization-managed policy, if introduced later, can constrain allowed settings but requires its own documented precedence.

Profiles override supported fields; they cannot include other files, inherit from other profiles, or execute commands. Ship a `gaming` profile as the initial default and add alternatives only when tested. Benchmarks export the complete resolved configuration, its hash, schema version, backend versions, and CLI overrides. A diagnostic command equivalent to `bricks config show --resolved --redact` should explain where each value came from.

## Validation and lifecycle

- Require `schema_version`; reject unsupported future versions and misspelled/unknown keys with a field path and a useful correction.
- Validate types, ranges, maximum queue sizes, retry/deadline relationships, bind addresses, and file references before session start.
- Reject unavailable backends, incompatible capabilities, and unsafe settings combinations before capturing any input. An `auto` backend resolves from a published preference order and reports the actual backend. Benchmark profiles specify exact backends.
- Migrate known old schemas with a backup and a readable change summary. Never silently reinterpret an old setting.
- Build one immutable resolved session configuration. Backend changes, authentication changes, and recovery-policy changes apply on reconnect. Status verbosity and aggregate diagnostics may update live; input tracing requires explicit activation.
- Reject malformed settings while retaining the last valid configuration. Never keep running with a partly applied configuration.
- UI hiding advanced options does not bypass validation. Security invariants, parser bounds, event deduplication, and owned-key cleanup cannot be disabled by a performance profile.

Profiles may adjust measured tradeoffs such as motion aggregation, packet history, retry policy, video rate, or presentation mode. They must never add random key timing. A changed option that invalidates a benchmark condition produces a different benchmark configuration, not an undocumented optimization.

## Component contracts

| Module | Configurable behavior | Fixed contract |
| --- | --- | --- |
| Capture | Backend, selected devices, escape chord | Physical events, monotonic capture time, explicit ownership |
| Forwarding | Bounded aggregation and queue caps | No fabricated events; ordering barriers preserved |
| Transport | Registered backend, endpoint, payload cap | Authenticated peer and bounded messages |
| Recovery | ACK history, retry/deadline bounds, snapshots | Deduplication, bounded memory, neutral-state resync |
| Injection | Backend and supported input capabilities | Checked results, exactly-once application within epoch |
| Media | Capture/codec backends, FPS, rate, presentation | Capability negotiation and bounded frame queues |
| Diagnostics | Aggregate metrics, explicit trace mode, retention | Never block input; count trace drops; redact secrets |

“Exactly once” here describes deduplicated application within a healthy epoch, not a guarantee of delivery or completion during outages. Lost history must be reported explicitly.

## Examples

See [host.toml](examples/host.toml) and [client.toml](examples/client.toml). Both parse and validate with the development binaries. The implemented subset covers endpoints, selected Windows backends, identity references, forwarding aggregation, snapshot/watchdog values, recovery history/reorder caps, retry/deadline clamps, and diagnostics. The [single-PC recovery check](recovery-selftest.md) uses the new limits and records resolved settings/hash. These recovery additions are not yet wired into the live QUIC probe; snapshots remain reserved. Loopback endpoints keep these examples locally scoped; the future connection UI selects a private adapter/address for two-PC use after pairing. Validation does not open a listener, retrieve a key, or establish a session.

Currently only `quic`, `windows_raw_input`, and `windows_send_input` are recognized application backend IDs. Their live startup is pending M1/M2. The UDP reference is a separate loopback-only benchmark command, not an application backend that can bypass authentication. Profiles currently override only `motion_aggregation_us`; adding fields requires an explicit typed schema change. The initial escape chord is fixed at `Ctrl+Alt+Shift+F12`; broader chord configuration follows when the capture adapter exists.

Keep host authorization separate from selecting `role = "host"`. A host does not accept control until sharing is enabled and a paired peer receives an ownership grant. Future media fields can be added under a versioned module section without changing the input core.

The [desktop preview](desktop-ui.md) embeds these same host/client examples, substitutes the selected numeric endpoint/port, and invokes the strict Rust CLI before a recording-only connection. Its small versioned UI preferences file stores role, addresses, port and public fingerprints separately; it cannot override trust or turn on unauthenticated input. Complete UI configuration editing, migrations and arbitrary profile overrides remain pending.

For the selected Slint product frontend, the persistent agent owns configuration and saved-device state; UI and CLI request changes through the [same control SDK](control-sdk.md). Keep view-only preferences separate from authorization/session truth. Validate complete typed updates against the current configuration revision, commit atomically, and reject stale/invalid updates without partially applying them. This supersedes subprocess/temp-TOML orchestration as the product control design; the current diagnostic launcher is not migrated by this design step.

The [Slint desktop](slint-desktop.md) persists schema-1 `desktop.json` through the separate per-user agent and typed `bricks-control` IPC SDK: numeric endpoints, display name and default new-entry port. The controller validates complete updates and revisions, stages/syncs/replaces the file, and prevents concurrent writers. This does not edit engine TOML or grant trust. Engine options remain in the strict configuration until live OS session integration. The network lab has a separate bounded typed `Options` structure for rates, counts, seeds, impairments and cumulative-feedback batching; it does not add arbitrary plugin names or unregistered backends.
