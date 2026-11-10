# Development identity and control ownership

Implemented in `bricks-transport::identity`, `auth`, and `ownership`. This is a development provisioning path. The [recording-only QUIC probe](quic-probe.md) now connects live endpoints to these libraries. OS input networking and the simple pairing UI remain pending.

## Storage and trust

`os-store:bricks-host` resolves to `%LOCALAPPDATA%\Bricks\identities\bricks-host`; the client reference resolves similarly. Reference names permit only ASCII letters, digits, and hyphens (maximum 64 characters). Paths are selected locally, never from a remote packet.

Each identity has an `identity.json` containing the public DER certificate and a Windows current-user DPAPI-protected PKCS#8 key blob. Plaintext PKCS#8 temporary buffers use zeroizing storage. DPAPI output is wiped and freed with its documented allocator. Credential reload also validates that the private key matches the certificate. Initialization refuses to overwrite an existing identity. Public certificate exports also refuse to overwrite a destination.

This protects private-key storage under the Windows account boundary. It does not protect against software running as that same user, filesystem rollback, account/profile loss, or a compromised endpoint. Use the per-user application-data location with its normal permissions. No private-key export command exists. Certificate rotation/recovery and signed trust metadata are not implemented.

Peer records live under `peers/<sha256>.json`. Import requires the exact lowercase SHA-256 fingerprint of the public certificate, verified through an independent trusted path. Existing trust is not silently replaced; revoke and explicitly import again to change permissions. Public trust files contain a separate `allow_control` flag. Trusting a host for connection does not automatically permit it to control this PC.

## Development commands

Build with Cargo first; the compiled executables below do not require Cargo on PATH. On the host:

```powershell
.\target\debug\bricks-host.exe identity init os-store:bricks-host
.\target\debug\bricks-host.exe identity show os-store:bricks-host
.\target\debug\bricks-host.exe identity export os-store:bricks-host .\host-public.der
```

On the client, use `bricks-client.exe` and `os-store:bricks-client`, exporting `client-public.der`. Transfer only these public DER certificates between the PCs. Verify each displayed fingerprint independently before import. Commands below use placeholders that must be replaced with the actual 64 lowercase hex digits:

```powershell
# Host: explicitly permit the verified client to request control.
.\target\debug\bricks-host.exe identity trust os-store:bricks-host .\client-public.der CLIENT_SHA256 --allow-control

# Client: trust the host without granting it reverse control.
.\target\debug\bricks-client.exe identity trust os-store:bricks-client .\host-public.der HOST_SHA256

# Host: revoke the client's saved trust.
.\target\debug\bricks-host.exe identity revoke os-store:bricks-host CLIENT_SHA256
```

`identity show`/`export` validate the protected identity before returning public information. `init` should run under the Windows user that will run Bricks. Missing DPAPI profile access is an error, not grounds to store the key unprotected. No real per-user Bricks identity has been provisioned by this implementation task; automated identity tests use temporary stores and remove them afterward.

## Session admission

`auth::client_for_peer` and `server_for_peer` load protected local credentials and explicitly trusted peer certificates. TLS 1.3, client authentication, name validation on the client, application protocol `bricks-input/1`, and disabled early data remain mandatory.

`ControlledSession::admit` additionally requires a completed authenticated server connection, exact matching current client trust, persisted control permission, and explicit host approval. It generates a fresh cryptographic session epoch. A shared host `Store` permits one active control lease. The eventual host actor must own that shared store; independent processes/stores are not coordinated by a cross-process lock yet.

The wrapper checks the lease before applying input and rejects old epochs through the core state machine. Explicit stop invalidates the lease and returns cleanup errors. Drop makes a best-effort cleanup attempt; abrupt process termination still needs the planned external supervisor.

## Revocation scheduling

In-process `Store::revoke` invalidates associated leases and removes saved trust. A separate process's revocation is detected by `refresh_leases`, which must run off the input hot path. The QUIC recording probe now refreshes trust through a blocking worker and polls authorization on a 20 ms control tick, including while idle. Tests verify recorded-key release without another input event. Real OS injection actors, their timing bounds, and an external crash watchdog remain M2/M3 integration work.

Reimporting a certificate does not revive revoked sessions. Existing in-flight work may complete around a concurrent revocation check; admission, revocation, and injection must be serialized by the future host actor. Cleanup failures remain tracked by the core and can be retried. Errors removing trust must be surfaced: a failed filesystem operation is not a successful persistent revocation.

## Verification

The Windows tests exercise DPAPI protect/reload, refusal to overwrite an identity, certificate/key mismatch rejection, incorrect fingerprints, trust without control permission, lease exclusivity, persistent revocation detection, and an end-to-end in-memory path from protected credentials through mutual TLS to authorized input state. The idle-revocation test verifies a pressed fixture key is released when authorization is polled. Automated tests do not inject real OS keys.

DPAPI tests require access to the normal Windows user profile. They failed in the restricted execution profile and passed with normal profile access; the complete suite then passed all 29 tests. This is functional verification, not a security audit or a two-PC result.
