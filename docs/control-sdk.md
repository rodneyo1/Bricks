# Local agent and control SDK contract

Design v1, 2026-10-23. The separate per-user agent and named-pipe SDK now implement the device/settings/setup/diagnostic subset, versioned framing, bounded events and atomic snapshots. Slint uses that SDK. See [implementation, tighter request limits and test coverage](single-pc-agent-and-recovery.md). Pairing, live input/sharing/capture, media and external OS cleanup supervision remain unimplemented; the broader command groups below remain a target contract, not enabled capabilities. The WinForms launcher remains a separate development tool.

## Components and authority

Use three new components initially; avoid a generic plugin or RPC framework:

| Component | Responsibility |
| --- | --- |
| `bricks-agent` | Persistent per-user session runtime; owns device storage, settings, identity/trust adapters, admission, session lifecycle and cleanup |
| `bricks-control` | Shared versioned command/event types, bounded codec and asynchronous SDK client; used by UI and CLI |
| `bricks-ui` | Slint views and presentation state; requests changes and displays confirmed agent events |

Reuse the existing Rust config/core/input/transport modules. Keep their remote QUIC protocol distinct from this local control protocol. Existing development CLIs can remain during migration, then use the same control SDK for product operations.

Run the agent in the interactive Windows user's session. Headless means the main control window is optional; the agent can still require a hidden message window and native capture threads. A Session 0 Windows service is not the input agent. A later supervisor/service has a separate, narrowly scoped purpose.

The agent owns session truth and permissions. Frontends cannot declare a peer trusted, choose an epoch, clear held-state faults, or claim successful injection. Capture starts only after authorization and explicit capture activation. Initially allow one live role/session per agent; reject a conflicting operation with Busy rather than silently switching ownership. Stop and revoke must remain available while other work is pending.

Slint and CLI use the control SDK for infrequent operations. Input capture, forwarding, ordering, injection and future video/audio run directly in their native Rust workers. No command per mouse sample, frame-by-frame UI property updates, per-event JSON, or process launch per input. A native viewer can run beside the session workers without sending real-time data through the shell.

## Local endpoint and framing

Windows v1 uses a session-scoped named pipe. Restrict access to the intended user's SID/session, reject remote pipe clients, verify the connecting token/session, and limit concurrent local clients. Do not rely on an obscure pipe name as authentication. There is no localhost HTTP/WebSocket listener, router change or extra network dependency for the UI.

Use a 4-byte little-endian payload length followed by a UTF-8 JSON message, maximum **64 KiB**. Validate the length before allocation and enforce count/string limits after decode. Messages use shared typed Rust enums; reject unknown commands, malformed data and incompatible major versions. JSON is acceptable here because this channel is outside the real-time path. Never serialize arbitrary Rust memory layout.

Initial engineering bounds: at most 4 local SDK clients, 16 outstanding requests per client, 64 saved computers, and 128 queued event messages per subscriber. Names are at most 64 characters; addresses use the shared endpoint validator. These are control-plane memory limits, not performance results. Do not let an event subscriber stall the input worker or accumulate an unbounded backlog.

## Handshake, snapshot and events

On connection, exchange supported API versions. An accepted handshake returns a fresh agent-instance ID, selected version, full state snapshot, and the snapshot's event sequence. Subscribe atomically with snapshot creation so no state change can be lost between reading state and listening. The snapshot includes:

- Capabilities with a reason for unavailable features.
- Saved computers and their separate trust, availability and session states.
- Local sharing/capture state, current controller and pending approvals.
- Active operations, cleanup state, and configuration schema/revision.

Every event carries the agent-instance ID and a strictly increasing sequence. A restarted agent uses a new instance ID. The SDK discards old-instance events, detects sequence gaps, and requests a new snapshot. UI state is marked unavailable until reconciled. A new snapshot supersedes events at or before its sequence.

If a subscriber queue overflows, disconnect that subscriber and require snapshot reconciliation; never silently omit state changes while leaving the UI believing it is current. The agent retains authoritative active/pending state. A full control queue must not block input or make stop/revoke wait behind filesystem work.

Request IDs correlate responses on one SDK connection. State-changing asynchronous requests return **Accepted(operation_id)** or **Rejected(error)** promptly. Accepted means work was admitted, not that a connection/sharing/cleanup succeeded. Completion arrives as a typed operation/domain event. A broken pipe or timeout can leave the outcome unknown: reconcile before retrying a non-idempotent operation. Stop/disconnect/cancel are idempotent; repeating them must not create a new session.

## Commands and events

| Command group | Operations | Confirmation / feedback |
| --- | --- | --- |
| Read state | GetSnapshot, GetSettings | Snapshot/config with current revision |
| Saved computers | AddDevice, EditDevice, RemoveDevice | DeviceSaved / DeviceRemoved; no implicit trust grant |
| Pairing | BeginPairing, SubmitPairing, CancelPairing, ApprovePairing, DeclinePairing | OfferCreated / ApprovalRequired / Paired / PairingFailed / PairingCancelled |
| Permissions | RevokePeer | Admissions invalidated, session stop/cleanup events, then revocation outcome |
| Sharing | StartSharing, StopSharing | SharingReady / SharingStopped, plus cleanup state where applicable |
| Session | Connect, CancelOperation, Disconnect | Connecting / ApprovalPending / SessionActive / SessionEnded / OperationFailed |
| Capture safety | SuspendCapture, ResumeCapture, RetryRelease | CaptureSuspended / CaptureActive / CleanupCompleted / CleanupFailed |
| Settings | UpdateSettings(expected_revision, typed_patch) | SettingsSaved / ValidationFailed / RevisionConflict |
| Advanced diagnostics | RunDiagnostic, CancelOperation, ExportDiagnostic | DiagnosticStarted / DiagnosticCompleted / DiagnosticFailed; separate from live-session capability |

Command names describe semantics; freeze exact enum payloads and conformance vectors in the first implementation. Requests identify devices/approvals/operations by agent-issued IDs. An approval applies to the specific pending request and verified identity, never merely a display name or address.

The initial capability set distinguishes device management, development fingerprint pairing, secure-code pairing, discovery, live input, native viewer/video, diagnostics and tray/background sharing. Report implemented support only. Unsupported commands fail before a listener, capture or ownership side effect starts. A functioning synthetic probe does not set live_input or video to true.

Short-code pairing is unavailable until a reviewed PAKE binds the identities and transcript with expiry/attempt limits. Pairing material goes only to the initiating authorized flow, is cleared on completion/expiry, and is never persisted in snapshots, event history, logs or report exports. Normal trust is saved by the agent after verified completion and required approval. Development fingerprint import remains explicitly separate.

## State and cleanup semantics

Track local agent connectivity, pairing, sharing, sessions, capture and cleanup separately. Do not squeeze them into one Connected boolean. In particular:

```text
Session: idle -> connecting -> awaiting_approval -> active
                                  |                 |
                                  +-> stopping <----+
                                           |
                                  ended / cleanup_failed

Capture: off -> active -> suspended -> off
Cleanup: not_needed / pending / confirmed / failed / unknown_local / unknown_remote
```

Additional transitions cover rejection, cancellation and transport failure; cancellation is legal before active. Resolve cancel-versus-completion races in the agent. A frontend cannot turn a late operation success into a new connection after the operation was terminally cancelled. Keep stop/revoke serialized with admission/injection, using the existing ownership invariants.

For a disconnect or stop, stop new admissions/capture as appropriate, invalidate session ownership, attempt release of owned state, and emit the actual outcome. Failed releases remain tracked for retry. Confirmed means the backend's checked release submissions succeeded; it does not prove game consumption. A disconnected client cannot assume remote releases succeeded without feedback. The host can confirm its own cleanup while the client shows unknown_remote.

An empty new process does not prove the OS is neutral after an agent crash. After an unclean restart, surface unknown_local and block new control until the backend/supervisor can establish safe state. Do not guess ownership or release unrelated local keys. Implementing and externally verifying that crash-recovery mechanism remains an M3 gate.

For the first GUI iteration, orderly application exit requests stop/disconnect and waits for the outcome. It does not kill a potentially injecting process; a cleanup failure remains visible with retry available. Unexpected loss of a UI-owned control connection suspends that session and attempts cleanup. Explicit CLI-owned headless operation needs no GUI; the CLI keeps its own control lease alive, and losing that owner follows the same cleanup policy. Background GUI sharing after close is a later opt-in capability with a visible local stop indicator. Neither UI lifetime nor a destructor alone covers an agent process crash; external cleanup supervision remains a separate acceptance task.

## Errors, configuration and observability

Return stable error codes with a short user message, operation ID and appropriate retry/recovery action. Initial codes: AgentUnavailable, IncompatibleVersion, UnsupportedCapability, PermissionDenied, Busy, InvalidSettings, RevisionConflict, PeerNotPaired, IdentityChanged, PairingExpired, ApprovalDeclined, TransportTimeout, NoRoute, CleanupFailed and Cancelled. Remote release uncertainty is a state, not a fabricated success. Report NoRoute/firewall causes only when observed; otherwise retain a timeout/unknown diagnosis.

Use the existing strict configuration schema and immutable session settings. Validate the complete effective configuration before committing an update; compare its revision, save atomically, and keep the last valid configuration on failure. Identify settings that apply only on reconnect. The SDK transports typed supported fields, not arbitrary executable paths or shell commands. Slint view preferences cannot override authorization or recovery bounds.

Emit control-state events immediately. Optional aggregate statistics are coalesced to at most 5 updates/second and kept separate from input timestamps. UI activity history is capped at 100 entries and contains no key events, pairing secrets or private keys. Detailed reports/traces are opt-in and redacted under the existing policy. All numerical bounds here are proposed implementation defaults, not benchmark claims.

## Implementation checks

First implement shared types/reducer, then framed pipe/SDK, then the Slint views. Verify rejected/malformed/oversized messages, token/session rejection, version mismatch, partial pipe reads, response correlation, atomic snapshot/subscription, event gaps/restart, bounded queues, duplicate commands, cancellation races, settings conflict/rollback, revocation during idle/active control, cleanup failure and unknown remote release. Use a fake clock and recording injector before a real OS backend.

Test UI feedback against these exact states without injecting OS keys. Later tests must externally observe real OS release and retain the existing two-PC/physical latency gates. No new runtime, SDK, secure pairing, installer or media backend is claimed by this design document. See [frontend layouts and event wording](frontend-design.md).
