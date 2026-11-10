# Installation and connection

Product requirement: a nontechnical user can install Bricks on two Windows PCs and establish a first LAN session without using a terminal, editing files, or entering port numbers. The following is intended behavior, not an existing installer.

The first [Slint desktop app](slint-desktop.md) is now runnable by double-click and supports saved computers, settings and local setup/checks. It does not yet establish a live remote-control session. The built portable folder is `dist\BricksDesktop`; the workspace Start Bricks shortcut points to it.

The selected product interface is a simple Slint frontend backed by a persistent Rust agent and shared local control SDK. Use [frontend layouts and event feedback](frontend-design.md) and the [control contract](control-sdk.md) for the next implementation. Three main screens suffice; diagnostics live under Advanced, and there are no animations or decorative progress indicators.

Implemented development tool: the [WinForms diagnostic launcher](desktop-ui.md) launches by double-click, runs the current checks, opens the foreground input fixture, and guides public-identity verification and recording-only host/client connection tests. It uses manual file/fingerprint pairing; it is not the product frontend, short-code flow, signed installer, live desktop control or M5 usability acceptance described below.

## Normal flow

1. Install the same Bricks application on both PCs using a signed Windows installer. The default install is per-user. The client needs no driver or routine elevation.
2. On the host, choose **Share this PC**. Explain that an approved peer can operate the current desktop. Sharing starts only after this action.
3. On the client, choose **Connect**. Show hosts discovered on the same permitted local network and saved paired hosts. Offer **Enter address** when discovery is unavailable.
4. For first pairing, the host displays a short-lived code; the client enters it. Both screens show matching device identities, and the host approves the request. Save trust after successful pairing.
5. Show a clear **Connect** action and an obvious local disconnect/escape action. Later connections use the saved host with no repeated pairing, unless trust was revoked or its identity changed.

Default host policy requires session approval. An optional **Allow this paired device to reconnect** permission enables unattended reconnect with a visible host indicator and local revoke/stop control. A saved host is not an unrestricted authorization grant.

## Pairing security

The short code must bind the verified identity exchange. Do not send a reusable code or token over unauthenticated plaintext, or treat a six-digit number as a TLS key. Select and review a maintained PAKE implementation, with expiry, attempt limits, and transcript/identity binding, before shipping short-code pairing. If this is not ready, the development build uses explicit out-of-band fingerprint verification; do not present that build as the completed simple setup flow.

Provisional UX defaults: code expires after two minutes; at most five failed attempts per offer; each offer is single-use and cancelled after success. Discovery names/addresses are untrusted hints and never grant control. Identity changes stop automatic connection and show a clear re-pair action.

## Discovery and firewall

Use local discovery only while sharing, scoped to private interfaces; avoid advertising on public adapters. Discovery failure must not block direct address or tunnel-address entry. Keep the selected port automatic for ordinary use, stable for saved hosts, and advanced-configurable for conflicts.

Windows firewall changes may require one-time elevation. Explain that request at the moment sharing is enabled; prefer an application-specific inbound rule limited to the chosen private/tunnel interface and required traffic. No “disable your firewall” advice, automatic public exposure, router port forwarding, or UPnP in the initial release. Removing the application removes its owned firewall rules.

## Networks outside the home/LAN

The first release uses an existing WireGuard/Tailscale route. Offer a guided **Connect over your private network** path, detect an existing installation where practical, and let the user enter/select the host's tunnel address. Never install a third-party service or change its account without an explicit action. Show whether the observed path is direct, relayed, or unknown.

This adds a prerequisite for Internet use; do not call the first release universally one-click remote access. Built-in NAT traversal and a managed rendezvous/relay service are a later product decision with separate hosting, authentication, and privacy requirements.

## Connected experience and recovery

Show host name, controlling peer, capture state, route status, and an accessible disconnect action. Keep detailed timings in an expandable diagnostics panel. On a brief interruption, release remote state and attempt a fresh authenticated session with a new epoch; never replay buffered input. Restore control only under the saved peer's session permission and explicit client capture state.

Make errors actionable: host not sharing, pairing expired, approval declined, identity changed, no route, firewall blocked, incompatible version, or game not accepting input. Do not collapse these into “connection failed.” Provide a redacted diagnostic export and **Test input** fixture before suggesting OS privilege changes.

Hybrid mode must disclose that video uses a separately installed Sunshine/Moonlight stack and verify single input ownership. Full Bricks mode is the eventual single-app gaming experience; the early input-only build must state its mode clearly.

## Packaging and maintenance

One installer with share/connect roles, Start-menu entry, clean uninstall, optional tray presence, and opt-in start at login. No kernel driver in the first package. Run the agent in the interactive user session; a later service supervisor, if needed, is separate and minimally privileged.

Plan signed binaries and signed update metadata, version compatibility checks, rollback, and updates between sessions. Avoid forced update restarts during play. Do not declare signing available until certificates and release infrastructure exist. Support a portable developer build, but use the installer for usability acceptance.

## Acceptance targets

Provisional usability goals: first LAN pairing within three minutes after launching the two installed apps; a saved-host connection within ten seconds on a healthy LAN; zero mandatory terminal commands or TOML edits. Measure with at least five first-time users on clean Windows profiles and record success rate, elapsed time, prompts, and assistance required. Test blocked discovery, declined elevation, expired codes, wrong identities, reboot, and uninstall. Pairing security and control approval remain mandatory even if they cost an extra click.
